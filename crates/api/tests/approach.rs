//! Owned world operations: the reach gate refuses an op whose target no
//! route reaches and no approach script serves, and the stall watch ends an
//! accepted approach that stops moving (`interact/approach.rs`).

use api::interact::{
    op_reach, ActionSpec, ApproachSample, ApproachVerdict, ApproachWatch, Driver, Interactions,
    OpReach, OpTarget, Resolution, SendReason, SendResult, APPROACH_STALL_TICKS,
};
use api::snapshot::{
    ActorView, ChatLineView, GameSnapshot, LocLayer, LocView, LocalPlayerView, NpcView, PlayerView,
    SceneView, WorldTile,
};
use client::client::{Client, ClientConfig, ClientRevision};
use client::dash3d::CollisionFlag;
use client::io::ServerProt;

const BASE: i32 = 3200;
const SIZE: i32 = 40;

/// A solid wall tile: blocks walking and sight.
const SOLID: i32 = CollisionFlag::WALK_SCENERY | CollisionFlag::VIS_SCENERY;
/// A counter tile: blocks walking, not sight.
const COUNTER: i32 = CollisionFlag::WALK_SCENERY;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile {
        x: BASE + x,
        z: BASE + z,
        level: 0,
    }
}

/// An open scene with a full-height wall column at scene x = 20.
fn walled_scene(wall: i32) -> SceneView {
    let mut flags = vec![0; (SIZE * SIZE) as usize];
    for z in 0..SIZE {
        flags[(20 * SIZE + z) as usize] = wall;
    }
    SceneView {
        available: true,
        base_x: BASE,
        base_z: BASE,
        level: 0,
        width: SIZE,
        height: SIZE,
        collision_flags: flags,
    }
}

fn open_scene() -> SceneView {
    SceneView {
        available: true,
        base_x: BASE,
        base_z: BASE,
        level: 0,
        width: SIZE,
        height: SIZE,
        collision_flags: vec![0; (SIZE * SIZE) as usize],
    }
}

fn actor(at: WorldTile) -> ActorView {
    ActorView {
        name: None,
        actions: Vec::new(),
        tile: at,
        distance: 0,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: 0,
        health: 10,
        total_health: 10,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    }
}

fn player_at(at: WorldTile) -> LocalPlayerView {
    LocalPlayerView {
        player: PlayerView {
            index: 3,
            actor: actor(at),
            network: at,
            combat_level: 3,
            skill_level: 3,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    }
}

fn npc_at(at: WorldTile) -> NpcView {
    NpcView {
        index: 7,
        r#type: Some(9),
        name: Some("Hans".into()),
        actions: vec![Some("Talk-to".into())],
        tile: at,
        distance: 0,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: 0,
        health: 7,
        total_health: 7,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: at,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn loc_at(at: WorldTile) -> LocView {
    LocView {
        typecode: 1 << 14,
        info: 0,
        id: 1,
        name: Some("Chest".into()),
        description: None,
        actions: vec![Some("Open".into())],
        tile: at,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: true,
        block_range: true,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }
}

/// An attached, in-game snapshot (a real client rebuild) with seeded world.
fn snapshot(
    scene: SceneView,
    player: WorldTile,
    npcs: Vec<NpcView>,
    locs: Vec<LocView>,
    tick: u32,
) -> (GameSnapshot, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    let mut c = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        },
        ClientRevision::R274,
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = BASE;
    c.map_build_base_z = BASE;
    c.stream = Some(stream);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    snap.seed_scene(scene);
    snap.seed_local_player(player_at(player));
    snap.seed_npcs(npcs);
    snap.seed_locs(locs);
    snap.seed_tick(tick);
    (snap, listener)
}

#[derive(Default)]
struct Recorder {
    menus: Vec<(i32, i32, i32, i32, i32)>,
    actions: Vec<i32>,
    moves: usize,
    native_rows: usize,
}

impl Driver for Recorder {
    fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
        self.menus.push((slot, action, a, b, c));
    }
    fn do_action(&mut self, slot: i32) -> bool {
        self.actions.push(slot);
        true
    }
    fn dispatch_native_menu(
        &mut self,
        _snapshot: &GameSnapshot,
        _target: api::interact::NativeMenuTarget<'_>,
    ) -> Result<(), SendReason> {
        self.native_rows += 1;
        Ok(())
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        self.moves += 1;
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (BASE, BASE)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn send_packet(&mut self, _packet: &api::prot::Send) -> Result<(), api::prot::PacketError> {
        Ok(())
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

impl Recorder {
    fn wrote_nothing(&self) -> bool {
        self.menus.is_empty() && self.actions.is_empty() && self.moves == 0 && self.native_rows == 0
    }
    fn sent_ops(&self) -> usize {
        self.actions.len() + self.native_rows
    }
}

fn talk() -> ActionSpec {
    ActionSpec::Label("Talk-to".into())
}

// ---- reach gate ------------------------------------------------------------

/// The Hans shape: a solid wall column between the player and the NPC.
/// The client's route search fails; the old path sent the op alone and the
/// engine stalled silently. The owned path refuses it with a typed reason
/// and writes nothing.
#[test]
fn op_across_a_solid_wall_is_refused_and_nothing_is_written() {
    let npc = npc_at(tile(25, 10));
    let (snap, _l) = snapshot(
        walled_scene(SOLID),
        tile(15, 10),
        vec![npc.clone()],
        vec![],
        10,
    );
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::Unreachable
    );
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        let result = ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk());
        assert!(
            matches!(
                result,
                SendResult::Refused {
                    reason: SendReason::Unreachable,
                    ..
                }
            ),
            "{result:?}"
        );
        assert_eq!(ix.last_reach(), Some(OpReach::Unreachable));
        assert!(ix.take_approach_watch().is_none());
    }
    assert!(rec.wrote_nothing(), "no op, no menu, no route");
}

/// A target beyond the approach range behind a counter (sight is open) is
/// also refused: no approach script runs past ten tiles.
#[test]
fn op_beyond_approach_range_with_no_route_is_refused() {
    let npc = npc_at(tile(35, 10));
    let (snap, _l) = snapshot(walled_scene(COUNTER), tile(15, 10), vec![npc], vec![], 10);
    let mut rec = Recorder::default();
    let mut ix = Interactions::new(&snap, &mut rec);
    assert!(matches!(
        ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk()),
        SendResult::Refused {
            reason: SendReason::Unreachable,
            ..
        }
    ));
    assert!(rec.wrote_nothing());
}

/// Bank tellers stand behind a counter: no stand is reachable, but sight is
/// open and the target is within ten tiles, so an approach script
/// (`[apnpc1,_bank_teller]`) may serve it. The op is sent as before and
/// watched: a script that serves it acts at once with an outcome, so standing
/// still with none for the whole bound is still a stall.
#[test]
fn op_behind_a_counter_in_sight_and_range_is_sent_and_watched() {
    let npc = npc_at(tile(25, 10));
    let (snap, _l) = snapshot(walled_scene(COUNTER), tile(15, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::ApproachScript
    );
    let mut rec = Recorder::default();
    {
        let mut ix = Interactions::new(&snap, &mut rec);
        assert!(matches!(
            ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk()),
            SendResult::Sent { .. }
        ));
        assert!(ix.take_approach_watch().is_some());
    }
    assert_eq!(rec.sent_ops(), 1);
}

/// A target beyond range from where the player stands, behind a counter, is
/// still sent when walking brings it into sight within range: the engine
/// walks and tests the approach again each cycle. Refusing it would be a
/// false `Unreachable`.
#[test]
fn op_from_beyond_range_is_sent_when_walking_brings_it_in_approach_range() {
    let npc = npc_at(tile(25, 10));
    let (snap, _l) = snapshot(walled_scene(COUNTER), tile(2, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::ApproachScript
    );
    let mut rec = Recorder::default();
    let mut ix = Interactions::new(&snap, &mut rec);
    assert!(matches!(
        ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk()),
        SendResult::Sent { .. }
    ));
    assert!(ix.take_approach_watch().is_some());
}

/// A reachable target far from the player is sent as before and watched.
#[test]
fn reachable_target_out_of_reach_is_sent_and_watched() {
    let npc = npc_at(tile(10, 10));
    let (snap, _l) = snapshot(open_scene(), tile(2, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::Routable
    );
    let mut rec = Recorder::default();
    let mut ix = Interactions::new(&snap, &mut rec);
    assert!(matches!(
        ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk()),
        SendResult::Sent { .. }
    ));
    let watch = ix.take_approach_watch().expect("watch armed");
    assert_eq!(watch.still_ticks(), 0);
    assert_eq!(rec.sent_ops(), 1);
}

/// An NPC the player already stands beside is sent and not watched.
#[test]
fn target_in_reach_is_sent_unwatched() {
    let npc = npc_at(tile(10, 10));
    let (snap, _l) = snapshot(open_scene(), tile(9, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::InReach
    );
    let mut rec = Recorder::default();
    let mut ix = Interactions::new(&snap, &mut rec);
    assert!(matches!(
        ix.interact(OpTarget::Npc(&snap.npcs()[0]), talk()),
        SendResult::Sent { .. }
    ));
    assert!(ix.take_approach_watch().is_none());
}

/// A loc behind a solid wall is refused through the same gate.
#[test]
fn loc_across_a_solid_wall_is_refused() {
    let loc = loc_at(tile(25, 10));
    let (snap, _l) = snapshot(walled_scene(SOLID), tile(15, 10), vec![], vec![loc], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Loc(&snap.locs()[0])),
        OpReach::Unreachable
    );
    let mut rec = Recorder::default();
    let mut ix = Interactions::new(&snap, &mut rec);
    assert!(matches!(
        ix.interact(
            OpTarget::Loc(&snap.locs()[0]),
            ActionSpec::Label("Open".into())
        ),
        SendResult::Refused {
            reason: SendReason::Unreachable,
            ..
        }
    ));
    assert!(rec.wrote_nothing());
}

/// The client blocks the scene's outermost ring of tiles whatever the server
/// holds there, so a target whose stands touch it is never proved unreachable.
#[test]
fn a_target_on_the_scene_boundary_ring_is_never_refused() {
    let npc = npc_at(tile(0, 10));
    let (snap, _l) = snapshot(walled_scene(SOLID), tile(25, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::Unknown
    );
}

/// An unmodelled scene (no player, or no scene) gives no verdict: the op is
/// sent exactly as before.
#[test]
fn an_unavailable_scene_never_refuses() {
    let npc = npc_at(tile(25, 10));
    let mut scene = walled_scene(SOLID);
    scene.available = false;
    let (snap, _l) = snapshot(scene, tile(15, 10), vec![npc], vec![], 10);
    assert_eq!(
        op_reach(&snap, &OpTarget::Npc(&snap.npcs()[0])),
        OpReach::Unknown
    );
}

// ---- stall watch -----------------------------------------------------------

fn sample(tick: u32, at: WorldTile) -> ApproachSample {
    ApproachSample {
        tick,
        player: at,
        chat_sequence: 0,
        refused: false,
        modal_open: false,
        animation: -1,
        in_reach: Some(Some(false)),
    }
}

fn armed(tick: u32, at: WorldTile) -> ApproachWatch {
    let npc = npc_at(tile(10, 10));
    let (snap, _l) = snapshot(open_scene(), at, vec![npc], vec![], tick);
    ApproachWatch::arm(&snap, &OpTarget::Npc(&snap.npcs()[0])).expect("watch")
}

/// The detector bound is the engine-derived five consecutive zero-step
/// observed ticks, ending in the typed stall at the fifth.
#[test]
fn stationary_player_with_no_outcome_stalls_at_the_bound() {
    let at = tile(2, 10);
    let mut watch = armed(100, at);
    for n in 1..APPROACH_STALL_TICKS {
        assert_eq!(
            watch.observe_sample(sample(100 + n, at)),
            ApproachVerdict::Pending,
            "tick {n}"
        );
    }
    assert_eq!(
        watch.observe_sample(sample(100 + APPROACH_STALL_TICKS, at)),
        ApproachVerdict::Stalled {
            ticks: APPROACH_STALL_TICKS
        }
    );
}

/// A player that moves is never flagged, however long the approach, and a
/// step resets the still count.
#[test]
fn moving_player_is_not_flagged() {
    let mut watch = armed(100, tile(2, 10));
    for n in 1..=40u32 {
        let verdict = watch.observe_sample(sample(100 + n, tile(2 + (n as i32 % 6), 10)));
        assert_eq!(verdict, ApproachVerdict::Pending, "tick {n}");
    }
    // Four still ticks, one step, four still ticks: the step reset the count.
    let at = tile(7, 10);
    let mut watch = armed(100, at);
    for n in 1..=4 {
        assert_eq!(
            watch.observe_sample(sample(100 + n, at)),
            ApproachVerdict::Pending
        );
    }
    assert_eq!(
        watch.observe_sample(sample(105, tile(8, 10))),
        ApproachVerdict::Pending
    );
    assert_eq!(watch.still_ticks(), 0);
    for n in 6..=9 {
        assert_eq!(
            watch.observe_sample(sample(100 + n, tile(8, 10))),
            ApproachVerdict::Pending
        );
    }
}

/// Polls between server ticks never count. A drain that applied several
/// ticks is a coverage gap: its intermediate tiles were never observed, so
/// it counts nothing and restarts the streak, however many ticks it spans.
#[test]
fn observed_server_ticks_count_not_polls_and_batches_count_nothing() {
    let at = tile(2, 10);
    let mut watch = armed(100, at);
    for _ in 0..50 {
        assert_eq!(
            watch.observe_sample(sample(100, at)),
            ApproachVerdict::Pending,
            "a poll without a new tick"
        );
    }
    assert_eq!(watch.still_ticks(), 0);
    // Single-packet observations accumulate.
    for n in 1..=3 {
        assert_eq!(
            watch.observe_sample(sample(100 + n, at)),
            ApproachVerdict::Pending
        );
    }
    assert_eq!(watch.still_ticks(), 3);
    // A batched drain of far more than the bound: not a stall, and the
    // streak restarts.
    assert_eq!(
        watch.observe_sample(sample(103 + 2 * APPROACH_STALL_TICKS, at)),
        ApproachVerdict::Pending
    );
    assert_eq!(watch.still_ticks(), 0);
    // Only later one-packet observations count again, up to the bound.
    let base = 103 + 2 * APPROACH_STALL_TICKS;
    for n in 1..APPROACH_STALL_TICKS {
        assert_eq!(
            watch.observe_sample(sample(base + n, at)),
            ApproachVerdict::Pending,
            "tick {n}"
        );
    }
    assert_eq!(
        watch.observe_sample(sample(base + APPROACH_STALL_TICKS, at)),
        ApproachVerdict::Stalled {
            ticks: APPROACH_STALL_TICKS
        }
    );
}

/// A counter that runs backwards (a session change) counts nothing.
#[test]
fn a_tick_counter_that_goes_backwards_counts_nothing() {
    let at = tile(2, 10);
    let mut watch = armed(100, at);
    assert_eq!(
        watch.observe_sample(sample(90, at)),
        ApproachVerdict::Pending
    );
    assert_eq!(watch.still_ticks(), 0);
}

/// Any outcome ends the watch without a stall.
#[test]
fn outcomes_resolve_the_watch() {
    let at = tile(2, 10);

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.chat_sequence = 5;
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::Output)
    );

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.modal_open = true;
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::Output)
    );

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.animation = 806;
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::Output)
    );

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.refused = true;
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::Refused)
    );

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.in_reach = None;
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::TargetGone)
    );

    let mut watch = armed(100, at);
    let mut s = sample(101, at);
    s.in_reach = Some(Some(true));
    assert_eq!(
        watch.observe_sample(s),
        ApproachVerdict::Resolved(Resolution::InReach)
    );
}

/// The watch reads the real snapshot: the same bound, driven through
/// `observe` with a seeded tile and tick, and a refusal line resolves it.
#[test]
fn observe_reads_tick_tile_and_chat_from_the_snapshot() {
    let at = tile(2, 10);
    let npc = npc_at(tile(10, 10));
    let (snap, _l) = snapshot(open_scene(), at, vec![npc.clone()], vec![], 100);
    let mut watch = ApproachWatch::arm(&snap, &OpTarget::Npc(&snap.npcs()[0])).unwrap();
    let mut verdict = ApproachVerdict::Pending;
    let (mut later, _later_listener) = snapshot(open_scene(), at, vec![npc.clone()], vec![], 100);
    for n in 1..=APPROACH_STALL_TICKS {
        later.seed_tick(100 + n);
        verdict = watch.observe(&later);
    }
    assert_eq!(
        verdict,
        ApproachVerdict::Stalled {
            ticks: APPROACH_STALL_TICKS
        }
    );

    let mut watch = ApproachWatch::arm(&snap, &OpTarget::Npc(&snap.npcs()[0])).unwrap();
    let (mut later, _l) = snapshot(open_scene(), at, vec![npc], vec![], 101);
    later.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "I can't reach that!".into(),
        sequence: 4,
    }]);
    assert_eq!(
        watch.observe(&later),
        ApproachVerdict::Resolved(Resolution::Refused)
    );
}

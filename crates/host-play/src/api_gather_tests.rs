use super::*;
use api::selected::{ClientRevision, EntityId, FamilyPreparation};
use api::snapshot::{
    ActorView, ItemContainer, ItemView, LocLayer, LocView, LocalPlayerView, PlayerView, StatView,
    WorldStateView, WorldTile,
};
use client::client::{Client, ClientConfig, ClientPlayer};
use client::config::{Cache, ObjType};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SLOT: &str = "gather-api";
const LOG_ID: i32 = 1511;
const AXE_ID: i32 = 1351;
const CLIENT_SOURCE: &str = r#"
export const apiVersion = 2;
export function tick(api) {
  const page = api.snapshot.gather;
  globalThis.__lastPage = page;
  if (!globalThis.__started) {
    globalThis.__started = true;
    globalThis.__run = api.gather.run({ skill: 'Woodcutting' });
    globalThis.__run.then(value => { globalThis.__runResult = value; });
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
    globalThis.__rs2b0t_run_override({ energyMin: 77 });
  }
  if (page && page.phase === 'running' && page.status && page.status.phase !== 'paused' && !globalThis.__liveHeldSent) {
    globalThis.__liveHeldSent = true;
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
  }
  if (globalThis.__stop && page && !globalThis.__stopSent) {
    globalThis.__stopSent = true;
    globalThis.__stopResult = api.gather.stop();
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
  }
  if (globalThis.__stopSent && page === null && !globalThis.__afterStopSent) {
    globalThis.__afterStopSent = true;
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
  }
}
"#;
const QUIET_SOURCE: &str = r#"
export const apiVersion = 2;
export function tick(_api) {}
"#;

fn selected_tree() -> (Arc<api::game_data::SelectedGameData>, i32, WorldTile) {
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("R289 selected data");
    let worker_data = Arc::clone(&selected);
    let catalog = FamilyPreparation::run(move |worker| worker_data.prepare_gathering(worker))
        .expect("start gathering catalog preparation")
        .join()
        .expect("gathering catalog worker")
        .expect("selected gathering catalog");
    let region = api::gather_methods::SceneRegionInput {
        min_x: i32::MIN / 2,
        min_z: i32::MIN / 2,
        max_x: i32::MAX / 2,
        max_z: i32::MAX / 2,
        level: 0,
    };
    for method in catalog.methods_for_resource("normal") {
        let Some(tree_id) = api::gather_methods::known_rows(&method.targets)
            .iter()
            .find_map(|target| {
                (target.class == api::gather_methods::TargetClass::Resource)
                    .then_some(target.entity)
                    .and_then(|entity| match entity {
                        EntityId::Loc(id) => Some(id),
                        _ => None,
                    })
            })
        else {
            continue;
        };
        let mut spots = catalog.spots(method, &region).expect("tree placements");
        if let Some(spot) = spots.find(|spot| spot.entity == EntityId::Loc(tree_id)) {
            return (selected, tree_id, spot.origin);
        }
    }
    panic!("R289 selected catalog has no complete normal-tree placement");
}

fn gather_snapshot(tree_id: i32, tile: WorldTile) -> GameSnapshot {
    let item_def = |id, name: &str| api::ItemDefView {
        id,
        name: Some(name.into()),
        stackable: false,
        members: false,
        base_value: 0,
        noted: false,
        certificate_link: -1,
        certificate_template: -1,
    };
    let logs = (0..28)
        .map(|slot| ItemView {
            def: item_def(LOG_ID, "Logs"),
            container: ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot,
            count: 1,
            actions: vec![Some("Drop".into())],
            component_id: -1,
        })
        .collect();
    let axe = ItemView {
        def: item_def(AXE_ID, "Bronze axe"),
        container: ItemContainer::Equipment,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![Some("Remove".into())],
        component_id: -1,
    };
    let actor = ActorView {
        name: Some("gather-host-fixture".into()),
        actions: Vec::new(),
        tile,
        distance: 0,
        animation: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health: 10,
        total_health: 10,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_npcs(Vec::new());
    snapshot.seed_world(WorldStateView {
        map_base_x: tile.x.saturating_sub(52),
        map_base_z: tile.z.saturating_sub(52),
        level: tile.level,
        members: true,
        multi_combat: false,
        player_count: 1,
        npc_count: 0,
        cycle: 1,
    });
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor,
            combat_level: 3,
            skill_level: 1,
        },
        energy: 100,
        weight: 0,
    });
    snapshot.seed_stats(vec![StatView {
        index: 8,
        name: "woodcutting".into(),
        effective: 1,
        base: 1,
        xp: 0,
        used: true,
    }]);
    snapshot.seed_inventory(logs, 28);
    snapshot.seed_equipment(vec![axe]);
    snapshot.seed_locs(vec![LocView {
        typecode: 10,
        info: 0,
        id: tree_id,
        name: Some("Tree".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    snapshot
}

fn attached_client(tile: WorldTile) -> (Client, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let stream = client::io::ClientStream::connect(&address.ip().to_string(), address.port())
        .expect("attached loopback stream");
    let mut cache = Cache::default();
    cache.objs.resize((LOG_ID + 1) as usize, ObjType::default());
    cache.objs[LOG_ID as usize].id = LOG_ID;
    cache.objs[LOG_ID as usize].name = "Logs".into();
    cache.objs[LOG_ID as usize].iop[4] = Some("Drop".into());
    if cache.objs.len() <= AXE_ID as usize {
        cache.objs.resize((AXE_ID + 1) as usize, ObjType::default());
    }
    cache.objs[AXE_ID as usize].id = AXE_ID;
    cache.objs[AXE_ID as usize].name = "Bronze axe".into();
    let config = ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    };
    let mut client = Client::from_shared_with_revision(
        config,
        Arc::new(cache),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        ClientRevision::R289,
    );
    client.stream = Some(stream);
    client.ingame = true;
    client.scene_state = 2;
    client.minusedlevel = tile.level;
    client.map_build_base_x = tile.x.saturating_sub(5);
    client.map_build_base_z = tile.z.saturating_sub(5);
    client.local_player = Some(ClientPlayer::at(5, 5));
    client.out.random = Some(client::io::Isaac::new(&[1, 2, 3, 4]));
    (client, listener)
}

fn start_source(
    scripts: &ScriptWall,
    selected: Arc<api::game_data::SelectedGameData>,
    source: &str,
) {
    let cell = script_slot_or_insert(scripts, SLOT);
    cell.lock()
        .unwrap()
        .start_load_with_loadouts_and_game_data(
            source.into(),
            script::LoadShape::NativeTick,
            vec![],
            &[],
            Some(selected),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
        .expect("v2 isolate starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match cell.lock().unwrap().poll_start() {
            script::StartPoll::Settled(script::StartOutcome::Ready) => return,
            script::StartPoll::Settled(outcome) => panic!("v2 start failed: {outcome:?}"),
            script::StartPoll::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            script::StartPoll::Pending => panic!("v2 setup did not settle"),
            script::StartPoll::NotOwed => panic!("v2 start was not owed"),
        }
    }
}

fn start_script(scripts: &ScriptWall, selected: Arc<api::game_data::SelectedGameData>) {
    start_source(scripts, selected, CLIENT_SOURCE);
}

fn start_quiet_script(scripts: &ScriptWall, selected: Arc<api::game_data::SelectedGameData>) {
    start_source(scripts, selected, QUIET_SOURCE);
}

#[allow(clippy::too_many_arguments)]
fn observe_frame(
    client: &mut Client,
    tick: u64,
    here: WorldTile,
    snapshot: &GameSnapshot,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    cache: &Arc<Cache>,
    names: &Arc<api::obj_names::ObjNames>,
    policy: &mut host::ScriptRunPolicy,
) -> Vec<u8> {
    let checkpoint =
        api::interact::Driver::packet_checkpoint(client).expect("R289 packet checkpoint");
    script_observe_cached(
        client,
        SLOT,
        true,
        true,
        tick,
        Some((here.x, here.z, here.level)),
        None,
        None,
        Some(snapshot),
        None,
        Some(names.as_ref()),
        scripts,
        cheats,
        navs,
        &None,
        false,
        false,
        None,
        None,
        Some(Arc::clone(cache)),
        Some(Arc::clone(names)),
        Some(policy),
    );
    let mut packets = Vec::new();
    assert!(api::interact::Driver::trace_packets(
        client,
        *checkpoint,
        &mut |opcode| packets.push(opcode)
    ));
    packets
}

fn probe(scripts: &ScriptWall, expression: &str) -> serde_json::Value {
    script_slot(scripts, SLOT)
        .expect("script slot")
        .lock()
        .unwrap()
        .probe(expression)
        .expect("Load probe")
}

fn force_watchdog_sampling(slot: &mut script::SlotScript, here: WorldTile) {
    let now = Instant::now();
    slot.feed_watchdog(
        now,
        Some((here.x, here.z, here.level)),
        &[],
        false,
        true,
        &[],
    );
    assert_eq!(
        slot.feed_watchdog(
            now + script::watchdog::WEDGE,
            Some((here.x, here.z, here.level)),
            &[],
            false,
            true,
            &[],
        ),
        script::WatchdogAction::RequestAnchor
    );
    assert!(slot.watchdog().holds_script_actions());
}

fn finish_watchdog_hold(slot: &mut script::SlotScript, here: WorldTile) {
    let anchor = WorldTile {
        x: here.x + 5,
        z: here.z,
        level: here.level,
    };
    assert!(matches!(
        slot.feed_watchdog(
            Instant::now(),
            Some((here.x, here.z, here.level)),
            &[],
            false,
            true,
            &[script::shim::InteractReq::RecoveryAnchor {
                x: anchor.x,
                z: anchor.z,
                level: anchor.level,
            }],
        ),
        script::WatchdogAction::ArmWalk { .. }
    ));
    slot.feed_watchdog(
        Instant::now(),
        Some((anchor.x, anchor.z, anchor.level)),
        &[],
        false,
        true,
        &[],
    );
    assert!(!slot.watchdog().holds_script_actions());
}

#[test]
fn gather_seat_owns_foreground_through_recovery_hold_stop_and_resume_dispatch() {
    let (selected, tree_id, here) = selected_tree();
    let snapshot = gather_snapshot(tree_id, here);
    let (mut client, _listener) = attached_client(here);
    let cache = Arc::clone(&client.cache);
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut policy = host::ScriptRunPolicy::default();
    start_script(&scripts, selected);
    {
        let cell = script_slot(&scripts, SLOT).unwrap();
        force_watchdog_sampling(&mut cell.lock().unwrap(), here);
    }

    let held_op = client::io::ClientProt289::OPHELD5.id as u8;
    let packets = observe_frame(
        &mut client,
        1,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert!(
        packets.is_empty(),
        "recovery hold must drop the same-batch Held row"
    );
    let cell = script_slot(&scripts, SLOT).unwrap();
    assert!(
        cell.lock().unwrap().api_owns_foreground(),
        "gather-run is admitted while the Load script is recovery-held"
    );
    assert!(
        format!("{policy:?}").contains("Floor(77)"),
        "run-policy control passes the recovery hold and is applied: {policy:?}"
    );
    finish_watchdog_hold(&mut cell.lock().unwrap(), here);

    let mut native_drop_batch_seen = false;
    let mut live_held_queued = false;
    let mut tick = 2;
    for _ in 0..48 {
        let packets = observe_frame(
            &mut client,
            tick,
            here,
            &snapshot,
            &scripts,
            &cheats,
            &navs,
            &cache,
            &names,
            &mut policy,
        );
        if packets.iter().filter(|opcode| **opcode == held_op).count() == 5 {
            native_drop_batch_seen = true;
        }
        live_held_queued |= probe(&scripts, "globalThis.__liveHeldSent === true") == true;
        if native_drop_batch_seen && live_held_queued {
            break;
        }
        tick += 1;
    }
    assert!(
        native_drop_batch_seen,
        "the live Gatherer seat dispatches its five-item Drop batch"
    );
    assert!(
        live_held_queued,
        "the v2 script queued its Held row while the seat was live"
    );

    let cell = script_slot(&scripts, SLOT).unwrap();
    cell.lock()
        .unwrap()
        .probe("globalThis.__stop = true")
        .expect("request API stop");
    let stop_packets = observe_frame(
        &mut client,
        tick,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert!(
        stop_packets.is_empty(),
        "gather-stop and its same-batch Held row are drained before game dispatch"
    );
    assert!(
        !cell.lock().unwrap().api_owns_foreground(),
        "Stop tears down the foreground owner"
    );

    tick += 1;
    let resumed_packets = observe_frame(
        &mut client,
        tick,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert_eq!(
        resumed_packets
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        1,
        "after teardown the script's Held row dispatches again"
    );
    assert!(probe(&scripts, "globalThis.__afterStopSent === true") == true);
    let result = probe(&scripts, "globalThis.__runResult");
    assert_eq!(
        result["kind"], "done",
        "run resolves with the unchanged machine envelope: {result}"
    );
    assert_eq!(
        result["value"]["end"], "stopped",
        "host terminal is the nested GatherEnd: {result}"
    );
    assert!(probe(&scripts, "globalThis.__lastPage === null") == true);
}

#[test]
fn api_foreground_edges_drop_paused_game_rows_but_restore_unowned_rows() {
    let (selected, tree_id, here) = selected_tree();
    let snapshot = gather_snapshot(tree_id, here);
    let (mut client, _listener) = attached_client(here);
    let cache = Arc::clone(&client.cache);
    let names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut policy = host::ScriptRunPolicy::default();
    start_quiet_script(&scripts, selected);

    let cell = script_slot(&scripts, SLOT).expect("Load slot");
    let held = || script::shim::InteractReq::Held {
        name: "Logs".into(),
        action: "Drop".into(),
        slot: None,
    };
    let held_op = client::io::ClientProt289::OPHELD5.id as u8;

    cell.lock().unwrap().pause();
    cell.lock().unwrap().restore_interacts(vec![held()]);
    let paused_unowned = observe_frame(
        &mut client,
        1,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert_eq!(
        paused_unowned
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        0,
        "Pause keeps an unowned game row queued"
    );
    assert_eq!(cell.lock().unwrap().state(), script::RunState::Paused);
    cell.lock().unwrap().resume();
    let resumed_unowned = observe_frame(
        &mut client,
        2,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert_eq!(
        resumed_unowned
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        1,
        "unowned paused rows retain their order and dispatch on Resume"
    );

    cell.lock().unwrap().pause();
    cell.lock().unwrap().restore_interacts(vec![
        script::shim::InteractReq::GatherRun {
            request_id: 7,
            settings: Arc::new(serde_json::Map::new()),
        },
        held(),
    ]);
    let paused_run_edge = observe_frame(
        &mut client,
        3,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert!(
        !paused_run_edge.contains(&held_op),
        "same-batch game row is dropped when paused control drain admits Gather"
    );
    assert!(cell.lock().unwrap().api_owns_foreground());

    cell.lock().unwrap().restore_interacts(vec![
        script::shim::InteractReq::GatherStop { request_id: 7 },
        held(),
    ]);
    let paused_stop_edge = observe_frame(
        &mut client,
        4,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert!(
        !paused_stop_edge.contains(&held_op),
        "same-batch game row is dropped when paused control drain stops Gather"
    );
    assert!(!cell.lock().unwrap().api_owns_foreground());

    cell.lock().unwrap().resume();
    let resumed_after_stop = observe_frame(
        &mut client,
        5,
        here,
        &snapshot,
        &scripts,
        &cheats,
        &navs,
        &cache,
        &names,
        &mut policy,
    );
    assert!(
        !resumed_after_stop.contains(&held_op),
        "a row dropped on the Stop edge is never replayed after Resume"
    );
    cell.lock().unwrap().stop();
}

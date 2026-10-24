use super::*;
use api::selected::{ClientRevision, EntityId, FamilyPreparation};
use api::snapshot::{
    ActorView, ItemContainer, ItemView, LocLayer, LocView, LocalPlayerView, PlayerView, StatView,
    WorldStateView, WorldTile,
};
use client::client::{Client, ClientConfig, ClientPlayer};
use client::config::{Cache, ObjType};
use script::native::AllowWalkDanger;
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
    globalThis.__run = api.gather.run({ skill: 'Woodcutting', disposition: 'Power' });
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

fn gather_snapshot(tree_id: i32, tile: WorldTile, client: &Client, log_slots: i32) -> GameSnapshot {
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
    let logs = (0..log_slots)
        .map(|slot| ItemView {
            def: item_def(LOG_ID, "Logs"),
            container: ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot,
            count: 1,
            actions: vec![None, None, None, None, Some("Drop".into())],
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
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    snapshot.seed_ingame(2);
    snapshot.seed_npcs(Vec::new());
    snapshot.seed_world(WorldStateView {
        map_base_x: tile.x.saturating_sub(5),
        map_base_z: tile.z.saturating_sub(5),
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
            network: actor.tile,
            actor,
            combat_level: 3,
            skill_level: 1,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
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
    {
        let mut slot = cell.lock().unwrap();
        if slot.work_scope().is_none() {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            slot.bind_incarnation(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        let start = crate::script_runtime::js_surface_start(
            crate::script_runtime::js_start_card(
                source.into(),
                script::LoadShape::NativeTick,
                vec![],
            ),
            None,
            Some(selected),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
            Arc::from(Vec::<api::loadout::Loadout>::new()),
        );
        slot.start_surface(start).expect("v2 isolate starts");
    }
    let deadline = Instant::now() + Duration::from_secs(60);
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
    service_arm: Option<&SlotArm>,
) -> Vec<u8> {
    let checkpoint =
        api::interact::Driver::packet_checkpoint(client).expect("R289 packet checkpoint");
    script_observe_cached_with_channels(
        client,
        SLOT,
        true,
        true,
        false,
        tick,
        Some((here.x, here.z, here.level)),
        None,
        None,
        Some(snapshot),
        None,
        None,
        Some(names.as_ref()),
        scripts,
        cheats,
        navs,
        &None,
        false,
        false,
        false,
        None,
        None,
        Some(Arc::clone(cache)),
        Some(Arc::clone(names)),
        None,
        script_channels::BrokerWorld::Unavailable,
        Some(policy),
        None,
        service_arm,
    );
    crate::script_runtime::pump_route_work(navs, SLOT);
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
    let now = Instant::now() - script::watchdog::WEDGE;
    slot.note_game_packet_tick(now);
    slot.feed_watchdog(
        now,
        Some((here.x, here.z, here.level)),
        &[],
        false,
        true,
        &[script::shim::InteractReq::NoteProgress],
    );
    let until = now + script::watchdog::WEDGE;
    let mut packet_tick = now;
    while packet_tick < until {
        packet_tick = (packet_tick + Duration::from_secs(5)).min(until);
        slot.note_game_packet_tick(packet_tick);
    }
    assert_eq!(
        slot.feed_watchdog(
            until,
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
        x: here.x + script::watchdog::ANCHOR_NEAR + 1,
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
    let mut rig = GatherReconnectRig::with_source(CLIENT_SOURCE);
    rig.queue_start();
    force_watchdog_sampling(&mut rig.slot().lock().unwrap(), rig.tile);

    let held_op = client::io::ClientProt289::OPHELD5.id as u8;
    let packets = rig.frame();
    assert!(
        packets.is_empty(),
        "recovery hold must drop the same-batch Held row"
    );
    assert!(
        !rig.slot().lock().unwrap().api_owns_foreground(),
        "the typed start remains held with its batch until recovery releases it"
    );
    finish_watchdog_hold(&mut rig.slot().lock().unwrap(), rig.tile);

    let mut native_drop_batch_seen = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let packets = rig.frame();
        native_drop_batch_seen |= packets.iter().filter(|opcode| **opcode == held_op).count() == 5;
        if native_drop_batch_seen && rig.probe("globalThis.__liveHeldSent === true") == true {
            break;
        }
    }
    assert!(rig.slot().lock().unwrap().api_owns_foreground());
    assert!(
        native_drop_batch_seen,
        "the live Gatherer seat dispatches its five-item Drop batch"
    );
    assert!(
        rig.probe("globalThis.__liveHeldSent === true") == true,
        "the v2 script queued its Held row while the seat was live"
    );

    rig.probe("globalThis.__stop = true");
    rig.isolate_tick_without_host_drain();
    let stop_packets = rig.frame();
    assert!(
        stop_packets.is_empty(),
        "typed Stop and its same-batch Held row are drained before game dispatch"
    );
    assert!(
        !rig.slot().lock().unwrap().api_owns_foreground(),
        "Stop tears down the foreground owner"
    );

    rig.isolate_tick_without_host_drain();
    assert!(rig.probe("globalThis.__afterStopSent === true") == true);
    let resumed_packets = rig.frame();
    assert_eq!(
        resumed_packets
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        1,
        "after teardown the script's Held row dispatches again"
    );
    let result = rig.probe("globalThis.__runResult");
    assert_eq!(
        result["kind"], "done",
        "run resolves with the unchanged machine envelope: {result}"
    );
    assert_eq!(
        result["value"]["end"], "stopped",
        "host terminal is the nested GatherEnd: {result}"
    );
    assert!(rig.probe("globalThis.__lastPage === null") == true);
}

#[test]
fn live_gather_seat_delivers_broadcast_posts_and_host_rows_but_drops_game_rows() {
    use script::shim::InteractReq;
    use script_channels::{BrokerWorld, ChannelBroker};

    let mut rig = GatherReconnectRig::with_empty_inventory(
        r#"export const apiVersion = 2;
           export function tick(api) {
             globalThis.__api = api;
             globalThis.__page = api.snapshot.gather;
             if (globalThis.__launch && !globalThis.__run) {
               globalThis.__run = api.gather.run({ skill: 'Woodcutting', disposition: 'Power' });
               globalThis.__run.then(value => { globalThis.__runResult = value; });
             }
           }"#,
    );
    rig.queue_start();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(owned);
    let _token = rig.wait_for_start_page();

    let channel = "rs2b0t:kq:v1:gather-api,bob,carol,dave";
    let mut receiver = GatherReconnectRig::with_source(
        r#"export const apiVersion = 2;
           globalThis.__received = [];
           const channel = new BroadcastChannel('rs2b0t:kq:v1:gather-api,bob,carol,dave');
           channel.onmessage = e => __received.push(e.data);
           export function tick(_api) {}"#,
    );
    receiver.isolate_tick_without_host_drain();
    rig.scripts
        .lock()
        .unwrap()
        .insert("bob".into(), receiver.slot());
    let broker = ChannelBroker::default();
    let sender = broker.slot(SLOT);
    let peers = ["bob", "carol", "dave"].map(|name| broker.slot(name));
    let (_, receiver_rows, _) = receiver.slot().lock().unwrap().drain_host_interacts();
    let receiver_generation = receiver.slot().lock().unwrap().runtime_generation();
    let receiver_channel_id = receiver_rows
        .iter()
        .find_map(|row| match &row.req {
            InteractReq::ChannelOpen { channel_id, .. } => Some(*channel_id),
            _ => None,
        })
        .expect("real receiver opens its channel");
    peers[0].pump(
        receiver_generation,
        BrokerWorld::Local,
        true,
        receiver_rows.into_iter().map(|queued| queued.req).collect(),
    );
    for (id, peer) in (11..).zip(&peers[1..]) {
        peer.pump(
            1,
            BrokerWorld::Local,
            true,
            vec![InteractReq::ChannelOpen {
                channel_id: id,
                name: channel.into(),
            }],
        );
    }
    rig.probe(&format!(
        "globalThis.__channel = new BroadcastChannel('{channel}'); \
         __channel.postMessage({{ proof: 'during-seat' }}); \
         __api.request({{ op: 'held', name: 'Logs', action: 'Drop' }}); true"
    ));
    rig.isolate_tick_without_host_drain();
    rig.slot()
        .lock()
        .unwrap()
        .restore_interacts(vec![InteractReq::SetCameraYaw { yaw: 777 }]);
    // Use the production observation and broker path, not a second drain/filter.
    let checkpoint = api::interact::Driver::packet_checkpoint(&rig.client).unwrap();
    script_observe_cached_with_channels(
        &mut rig.client,
        SLOT,
        true,
        true,
        true,
        rig.tick,
        Some((rig.tile.x, rig.tile.z, rig.tile.level)),
        None,
        None,
        Some(&rig.snapshot),
        None,
        None,
        Some(rig.names.as_ref()),
        &rig.scripts,
        &rig.cheats,
        &rig.navs,
        &None,
        false,
        false,
        false,
        None,
        None,
        Some(Arc::clone(&rig.cache)),
        Some(Arc::clone(&rig.names)),
        Some(&sender),
        BrokerWorld::Local,
        Some(&mut rig.policy),
        None,
        None,
    );
    rig.tick += 1;
    let mut packets = Vec::new();
    assert!(api::interact::Driver::trace_packets(
        &rig.client,
        *checkpoint,
        &mut |opcode| packets.push(opcode)
    ));
    assert!(
        !packets.contains(&(client::io::ClientProt289::OPHELD5.id as u8)),
        "the script's game row must be dropped: {packets:?}"
    );
    receiver.isolate_tick_without_host_drain();
    assert_eq!(
        receiver.probe("globalThis.__received"),
        serde_json::json!([{ "proof": "during-seat" }]),
        "the BroadcastChannel post must be delivered while the seat is live"
    );
    assert_eq!(rig.client.orbit_camera_yaw, 777);
    // A reply reaches the real sending isolate only if its open/post reached
    // the broker during ownership; it also proves membership stayed in sync.
    rig.probe("globalThis.__received = []; __channel.onmessage = e => __received.push(e.data)");
    let reply = peers[0].pump(
        receiver_generation,
        BrokerWorld::Local,
        true,
        vec![InteractReq::ChannelPost {
            channel_id: receiver_channel_id,
            name: channel.into(),
            data: script::channel::encode(&serde_json::json!({ "reply": "during-seat" })).unwrap(),
        }],
    );
    assert!(
        reply.iter().any(|delivery| delivery.account == SLOT),
        "the channel opened during a live seat must receive a broker delivery"
    );
    deliver_channel_events(&rig.scripts, reply);
    rig.isolate_tick_without_host_drain();
    assert_eq!(
        rig.probe("globalThis.__received"),
        serde_json::json!([{ "reply": "during-seat" }])
    );
    rig.probe("__channel.close()");
    rig.isolate_tick_without_host_drain();
    let (_, rows, owned) = rig.slot().lock().unwrap().drain_host_interacts();
    assert!(owned);
    sender.pump(
        rig.slot().lock().unwrap().runtime_generation(),
        BrokerWorld::Local,
        true,
        rows.into_iter().map(|queued| queued.req).collect(),
    );
    let after_close = peers[0].pump(
        receiver_generation,
        BrokerWorld::Local,
        true,
        vec![InteractReq::ChannelPost {
            channel_id: receiver_channel_id,
            name: channel.into(),
            data: script::channel::encode(&serde_json::json!("after-close")).unwrap(),
        }],
    );
    assert!(
        !after_close.iter().any(|delivery| delivery.account == SLOT),
        "close must reach the broker during ownership"
    );
    rig.slot().lock().unwrap().stop();
}

#[test]
fn paused_gather_seat_keeps_host_rows_queued_until_resume_in_every_frame_state() {
    use script::shim::InteractReq;

    for owned in [false, true] {
        for (up, hold, has_snapshot) in [
            (false, false, false),
            (true, true, true),
            (true, false, false),
            (true, false, true),
        ] {
            let source = if owned {
                RECONNECT_SOURCE
            } else {
                QUIET_SOURCE
            };
            let mut rig = GatherReconnectRig::with_source(source);
            if owned {
                rig.queue_start();
                let (rows, seat_owns) = rig.drain_host();
                assert!(rows.is_empty());
                assert!(seat_owns, "the real typed Gather start owns the host seat");
            } else {
                rig.isolate_tick_without_host_drain();
                rig.frame();
            }
            let cell = rig.slot();
            cell.lock().unwrap().pause();
            assert_eq!(cell.lock().unwrap().api_owns_foreground(), owned);

            let rows = vec![
                InteractReq::InspectRoute {
                    x: rig.tile.x + 1,
                    z: rig.tile.z,
                    level: rig.tile.level,
                    from_x: rig.tile.x,
                    from_z: rig.tile.z,
                    from_level: rig.tile.level,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    avoid: vec![],
                    cross: vec![],
                    request_id: 9,
                },
                InteractReq::InspectAck {
                    seq: 1,
                    generation: cell.lock().unwrap().work_epoch(),
                },
                InteractReq::SetCameraYaw { yaw: 777 },
            ];
            cell.lock().unwrap().restore_interacts(rows);
            let before_yaw = rig.client.orbit_camera_yaw;
            script_observe_cached(
                &mut rig.client,
                SLOT,
                up,
                true,
                rig.tick,
                Some((rig.tile.x, rig.tile.z, rig.tile.level)),
                None,
                None,
                has_snapshot.then_some(&rig.snapshot),
                None,
                Some(rig.names.as_ref()),
                &rig.scripts,
                &rig.cheats,
                &rig.navs,
                &None,
                hold,
                false,
                None,
                None,
                Some(Arc::clone(&rig.cache)),
                Some(Arc::clone(&rig.names)),
                Some(&mut rig.policy),
            );
            assert_eq!(cell.lock().unwrap().state(), script::RunState::Paused);
            assert_eq!(rig.client.orbit_camera_yaw, before_yaw);

            cell.lock().unwrap().resume();
            rig.frame();
            assert_eq!(
                rig.client.orbit_camera_yaw, 777,
                "the host row left queued during Pause dispatches after Resume"
            );
            cell.lock().unwrap().stop();
        }
    }
}

#[test]
fn api_foreground_edges_drop_owned_rows_and_restore_unowned_paused_rows() {
    let mut unowned = GatherReconnectRig::with_source(QUIET_SOURCE);
    unowned.isolate_tick_without_host_drain();
    unowned.frame();
    let cell = unowned.slot();
    let held = || script::shim::InteractReq::Held {
        name: "Logs".into(),
        action: "Drop".into(),
        slot: None,
        target_item_id: None,
    };
    let held_op = client::io::ClientProt289::OPHELD5.id as u8;

    cell.lock().unwrap().pause();
    cell.lock().unwrap().restore_interacts(vec![held()]);
    let paused_unowned = unowned.frame();
    assert!(
        !paused_unowned.contains(&held_op),
        "Pause holds an unowned game row without dispatching it"
    );
    assert_eq!(cell.lock().unwrap().state(), script::RunState::Paused);
    cell.lock().unwrap().resume();
    let resumed_unowned = unowned.frame();
    assert_eq!(
        resumed_unowned
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        1,
        "the unowned row dispatches once after Resume"
    );
    cell.lock().unwrap().stop();

    let mut rig = GatherReconnectRig::with_log_slots(CLIENT_SOURCE, 1, None);
    rig.queue_start();
    let start_edge = rig.frame();
    assert!(
        !start_edge.contains(&held_op),
        "a game row sharing the typed start batch is dropped by the admitted seat"
    );
    assert!(rig.slot().lock().unwrap().api_owns_foreground());

    let stop_deadline = Instant::now() + Duration::from_secs(10);
    let token = loop {
        let page = rig.probe("globalThis.__lastPage && globalThis.__lastPage.token");
        if let Some(token) = page.as_u64() {
            break token;
        }
        rig.frame();
        assert!(
            Instant::now() < stop_deadline,
            "typed Gather start did not publish its page"
        );
    };
    // The page may publish its token before the compiled family enters
    // Running. Emit and drain the live Held row while the seat still owns
    // foreground; otherwise a late page could first emit it after Stop.
    let live_deadline = Instant::now() + Duration::from_secs(10);
    while rig.probe("globalThis.__liveHeldSent === true") != true {
        let in_seat = rig.frame();
        assert!(!in_seat.contains(&held_op), "the live row is seat-filtered");
        assert!(
            Instant::now() < live_deadline,
            "Gather did not reach Running"
        );
    }
    let in_seat = rig.frame();
    assert!(!in_seat.contains(&held_op), "the live row is seat-filtered");
    assert!(rig.slot().lock().unwrap().api_owns_foreground());
    rig.probe("globalThis.__stop = true");
    rig.isolate_tick_without_host_drain();
    let stop_edge = rig.frame();
    assert!(
        !stop_edge.contains(&held_op),
        "a game row sharing typed Stop is dropped before host dispatch"
    );
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());

    rig.isolate_tick_without_host_drain();
    assert!(rig.probe("globalThis.__afterStopSent === true") == true);
    let resumed_after_stop = rig.frame();
    assert_eq!(
        resumed_after_stop
            .iter()
            .filter(|opcode| **opcode == held_op)
            .count(),
        1,
        "a later unowned row dispatches after Stop"
    );
    let result_deadline = Instant::now() + Duration::from_secs(10);
    while rig.probe("globalThis.__runResult").is_null() {
        rig.frame();
        assert!(
            Instant::now() < result_deadline,
            "typed Stop did not settle Gather"
        );
    }
    let result = rig.probe("globalThis.__runResult");
    assert_eq!(result["kind"], "done");
    assert_eq!(result["value"]["end"], "stopped");
    assert_eq!(result["value"]["token"], token);
    assert!(rig.probe("globalThis.__lastPage === null") == true);
}

const RECONNECT_SOURCE: &str = r#"
export const apiVersion = 2;
globalThis.__settleCount = 0;
export function tick(api) {
  const page = api.snapshot.gather;
  globalThis.__page = page;
  if (globalThis.__launch && !globalThis.__run) {
    globalThis.__run = api.gather.run({ skill: 'Woodcutting', disposition: 'Power' });
    globalThis.__run.then(value => {
      globalThis.__settleCount++;
      globalThis.__runResult = value;
    });
  }
  if (globalThis.__pair && !globalThis.__run) {
    globalThis.__run = api.gather.run({ skill: 'Woodcutting', disposition: 'Power' });
    globalThis.__run.then(value => {
      globalThis.__settleCount++;
      globalThis.__runResult = value;
    });
    api.gather.stop();
  }
  if (globalThis.__stop && page && !globalThis.__stopSent) {
    globalThis.__stopSent = true;
    api.gather.stop();
  }
}
"#;

struct GatherReconnectRig {
    tile: WorldTile,
    snapshot: GameSnapshot,
    client: Client,
    _listener: TcpListener,
    cache: Arc<Cache>,
    names: Arc<api::obj_names::ObjNames>,
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    policy: host::ScriptRunPolicy,
    tick: u64,
    external_session: api::identity::SessionKey,
    service_arm: Option<Arc<SlotArm>>,
    walk_danger: Option<&'static dyn script::native::WalkDangerQuery>,
}

impl GatherReconnectRig {
    fn new() -> Self {
        Self::with_empty_inventory(RECONNECT_SOURCE)
    }

    fn with_source(source: &str) -> Self {
        Self::with_log_slots(source, 28, None)
    }

    fn with_empty_inventory(source: &str) -> Self {
        Self::with_log_slots(source, 0, None)
    }

    fn with_real_session_gate(source: &str) -> Self {
        let arm = SlotArm::new(7, true);
        arm.complete_login_for_test();
        let session = arm.session_key().expect("owner began a session");
        arm.session_gate()
            .observe(session, &api::lifecycle::SessionEventOut::SceneReady)
            .expect("owner scene is ready");
        Self::with_log_slots(source, 0, Some(arm))
    }

    fn with_log_slots(source: &str, log_slots: i32, service_arm: Option<Arc<SlotArm>>) -> Self {
        let (selected, tree_id, tile) = selected_tree();
        let (client, listener) = attached_client(tile);
        let snapshot = gather_snapshot(tree_id, tile, &client, log_slots);
        let cache = Arc::clone(&client.cache);
        let names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
        if let Some(arm) = &service_arm {
            let incarnation = arm
                .session_key()
                .expect("owner began a session")
                .slot()
                .incarnation()
                .get();
            let mut slot = script::SlotScript::new();
            slot.bind_incarnation(incarnation);
            slot.bind_surface_factory(Arc::new(
                script::load::surface::JsSurfaceFactory::default(),
            ));
            scripts
                .lock()
                .unwrap()
                .insert(SLOT.to_string(), Arc::new(Mutex::new(slot)));
        }
        start_source(&scripts, selected, source);
        let external_session = api::identity::SessionKey::first(
            script_slot(&scripts, SLOT)
                .expect("bound Load slot")
                .lock()
                .unwrap()
                .slot_key_for_test()
                .expect("host-assigned incarnation"),
        );
        Self {
            tile,
            snapshot,
            client,
            _listener: listener,
            cache,
            names,
            scripts,
            cheats,
            navs,
            policy: host::ScriptRunPolicy::default(),
            external_session,
            service_arm,
            tick: 1,
            walk_danger: None,
        }
    }

    fn slot(&self) -> Arc<Mutex<script::SlotScript>> {
        script_slot(&self.scripts, SLOT).expect("reconnect Load slot")
    }

    fn probe(&self, expression: &str) -> serde_json::Value {
        probe(&self.scripts, expression)
    }

    /// Tick the real isolate after a host keyframe but leave its typed seat
    /// batch queued for explicit reconnect/admission ordering. Do not set the
    /// host-side hold bit: the seat must still advance terminal state.
    fn isolate_tick_without_host_drain(&mut self) -> (bool, bool) {
        let tick = self.tick;
        self.tick += 1;
        let tile = self.tile;
        let snapshot = &self.snapshot;
        let names = Arc::clone(&self.names);
        let cell = self.slot();
        let mut slot = cell.lock().unwrap();
        let pages = with_script_snapshot_input(
            tick,
            Some((tile.x, tile.z, tile.level)),
            true,
            None,
            Some(snapshot),
            Some(names.as_ref()),
            None,
            None,
            false,
            false,
            false,
            0,
            false,
            0,
            false,
            None,
            PostedWalkOutcome::default(),
            PostedInspect::default(),
            |input, native| {
                script::load::surface::post_snapshot_delta_pages_for_test(
                    &mut slot, input, native, false,
                )
            },
        );
        let mut context = script::ScriptCtx {
            driver: &mut self.client,
            tick,
            here: Some((tile.x, tile.z, tile.level)),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: Some(names.as_ref()),
            compiled: script::CompiledTick {
                walk_danger: self.walk_danger,
                ..Default::default()
            },
        };
        slot.on_game_tick(&mut context);
        drop(slot);
        let _ = self.probe("undefined");
        pages
    }

    fn frame(&mut self) -> Vec<u8> {
        let packets = observe_frame(
            &mut self.client,
            self.tick,
            self.tile,
            &self.snapshot,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &self.cache,
            &self.names,
            &mut self.policy,
            self.service_arm.as_deref(),
        );
        self.tick += 1;
        let _ = self.probe("undefined");
        packets
    }

    /// Exercise the host's typed-seat drain; only ordinary host rows escape it.
    fn drain_host(&self) -> (Vec<script::shim::InteractReq>, bool) {
        let cell = self.slot();
        let mut slot = cell.lock().unwrap();
        let (policy, requests, owned) = drain_observed_host_interacts(&mut slot);
        assert!(policy.is_none(), "fixture emits no run-policy update");
        (
            requests.into_iter().map(|queued| queued.req).collect(),
            owned,
        )
    }

    fn reconnect(&mut self) {
        let cell = self.slot();
        let mut slot = cell.lock().unwrap();
        assert!(
            slot.reconnect_session_work(),
            "active Load work survives an unexpected reconnect"
        );
        self.external_session = self.external_session.next().expect("host session epoch");
        slot.on_is_up(true);
        slot.align_seat_session(Some(self.external_session));
        assert_eq!(slot.state(), script::RunState::Running);
    }

    fn native_run(&self) -> api::selected::RunKey {
        self.slot()
            .lock()
            .unwrap()
            .native_status()
            .expect("installed Gatherer status")
            .run
    }

    fn expect_running_page(&mut self, token: u64) {
        let expression = format!(
            "!!globalThis.__page && globalThis.__page.phase === 'running' && globalThis.__page.token === {token}"
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            self.frame();
            if self.probe(&expression) == true
                && self.slot().lock().unwrap().native_status().is_some()
            {
                return;
            }
        }
        panic!(
            "Gatherer page for token {token} did not install: {:?}",
            self.probe("globalThis.__page")
        );
    }

    fn queue_start(&mut self) {
        let _ = self.probe("globalThis.__launch = true");
        self.isolate_tick_without_host_drain();
    }

    fn wait_for_start_page(&mut self) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame();
            let token = self.probe("globalThis.__page && globalThis.__page.token");
            if let Some(token) = token.as_u64() {
                assert!(
                    self.slot().lock().unwrap().api_owns_foreground(),
                    "the public Gather page belongs to the admitted host seat"
                );
                return token;
            }
            assert!(
                Instant::now() < deadline,
                "typed Gather start did not publish its host page: page={:?}, result={:?}, native={:?}, logs={:?}",
                self.probe("globalThis.__page"),
                self.probe("globalThis.__runResult"),
                self.slot().lock().unwrap().native_status(),
                self.slot().lock().unwrap().drain_logs(),
            );
        }
    }

    fn start_through_host(&mut self) -> u64 {
        self.queue_start();
        let (rows, owned) = self.drain_host();
        assert!(
            rows.is_empty(),
            "typed Gather controls do not leak as host rows"
        );
        assert!(owned, "the host admits the isolate's typed Gather start");
        self.wait_for_start_page()
    }

    fn settled_result(&self, token: u64) -> serde_json::Value {
        let result = self.settled_result_from_wire();
        assert_eq!(result["value"]["token"], token, "terminal token: {result}");
        result
    }

    fn settled_result_from_wire(&self) -> serde_json::Value {
        assert_eq!(self.probe("globalThis.__settleCount"), 1);
        let result = self.probe("globalThis.__runResult");
        assert_eq!(result["kind"], "done", "Gather waiter settled: {result}");
        assert_eq!(result["value"]["end"], "stopped", "terminal: {result}");
        assert!(
            result["value"]["token"].as_u64().is_some(),
            "terminal retains its public request token: {result}"
        );
        result
    }

    fn stop_and_settle(&mut self, token: u64) {
        let _ = self.probe("globalThis.__stop = true");
        for _ in 0..32 {
            self.frame();
            if self.probe("globalThis.__settleCount") == 1 {
                self.settled_result(token);
                return;
            }
        }
        panic!(
            "Gather stop did not settle waiter: {:?}",
            self.probe("globalThis.__page")
        );
    }
}

#[test]
fn gather_reconnect_before_start_drain_replays_pending_typed_start_once() {
    let mut rig = GatherReconnectRig::new();
    rig.queue_start();
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());

    rig.reconnect();
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    rig.isolate_tick_without_host_drain();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(
        owned,
        "the reconnect-carried typed start is admitted by the host"
    );

    let token = rig.wait_for_start_page();
    rig.expect_running_page(token);
    assert_eq!(rig.probe("globalThis.__settleCount"), 0);
    rig.stop_and_settle(token);
    assert_eq!(rig.probe("globalThis.__settleCount"), 1);
}

#[test]
fn gather_reconnect_during_preparation_keeps_one_waiter_and_seat() {
    let mut rig = GatherReconnectRig::new();
    rig.queue_start();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(owned);
    {
        let slot = rig.slot();
        let slot = slot.lock().unwrap();
        assert!(slot.api_owns_foreground());
        assert!(
            slot.native_status().is_none(),
            "the typed start is admitted before Gatherer preparation installs"
        );
    }

    rig.reconnect();
    {
        let slot = rig.slot();
        let slot = slot.lock().unwrap();
        assert!(slot.api_owns_foreground(), "preparation survives reconnect");
        assert!(slot.native_status().is_none());
    }
    rig.isolate_tick_without_host_drain();
    let token = rig
        .probe("globalThis.__page && globalThis.__page.token")
        .as_u64()
        .expect("reconnect keyframe exposes the admitted Gather token");
    assert_eq!(rig.probe("globalThis.__page.phase"), "preparing");
    assert!(rig.slot().lock().unwrap().api_owns_foreground());
    rig.expect_running_page(token);
    assert_eq!(rig.probe("globalThis.__settleCount"), 0);
    assert_eq!(
        rig.native_run().session,
        rig.slot().lock().unwrap().work_epoch(),
        "preparation installs into the boundary epoch, not its stale admission epoch"
    );
    rig.stop_and_settle(token);
}

#[test]
fn gather_reconnect_after_install_rekeys_same_token_and_keeps_waiter_pending() {
    let mut rig = GatherReconnectRig::new();
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    assert_eq!(rig.probe("globalThis.__page.token"), token);
    let before = rig.native_run();

    rig.reconnect();
    let after = rig.native_run();
    assert_eq!((before.slot, before.run), (after.slot, after.run));
    assert_ne!(
        before.session, after.session,
        "installed run rekeys on reconnect"
    );
    assert!(rig.slot().lock().unwrap().api_owns_foreground());

    rig.isolate_tick_without_host_drain();
    assert_eq!(rig.probe("globalThis.__page.token"), token);
    assert_eq!(rig.probe("globalThis.__settleCount"), 0);
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(
        owned,
        "the acknowledged Gather session stays foreground-owned"
    );
    assert_eq!(rig.native_run().run, before.run);
    rig.stop_and_settle(token);
}

#[test]
fn load_observer_rearms_kept_seat_from_real_owner_gate_after_reconnect() {
    let mut rig = GatherReconnectRig::with_real_session_gate(RECONNECT_SOURCE);
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    let before = rig.native_run();

    let cell = rig.slot();
    {
        let mut slot = cell.lock().unwrap();
        // A down observation can precede the owner's boundary notification.
        // Its local work epoch is not the newly minted F0 session epoch.
        slot.on_is_up(false);
        assert!(slot.reconnect_session_work());
        slot.on_is_up(true);
        // Do not align_seat_session here: the observer must get the
        // replacement execution fence from the real owner on its next tick.
    }
    let arm = rig.service_arm.as_ref().expect("real slot owner");
    let old_session = arm.session_key().expect("first owner session");
    arm.observe_session_exit(api::lifecycle::SessionEndCause::ConnectionLost, true);
    arm.complete_login_for_test();
    let next_session = arm.session_key().expect("replacement owner session");
    assert_ne!(old_session, next_session);
    arm.session_gate()
        .observe(next_session, &api::lifecycle::SessionEventOut::SceneReady)
        .expect("replacement scene is ready");

    rig.frame();
    assert!(rig.slot().lock().unwrap().api_owns_foreground());
    assert_eq!(rig.native_run().run, before.run);
    assert_eq!(rig.native_run().session_key(), Some(next_session));
    rig.stop_and_settle(token);
    rig.probe(
        "globalThis.__launch=false; globalThis.__run=null; globalThis.__stop=false; globalThis.__stopSent=false; globalThis.__settleCount=0; true",
    );
    let next_token = rig.start_through_host();
    assert_ne!(next_token, token, "the new Start has its own request id");
    rig.expect_running_page(next_token);
    rig.stop_and_settle(next_token);
    assert!(
        !rig
            .slot()
            .lock()
            .unwrap()
            .drain_logs()
            .iter()
            .any(|log| log.contains("seat batch without an execution fence")
                || log.contains("dropped malformed seat batch run stamp")),
        "the Load observer must rearm the execution fence and the isolate's Start stamp after reconnect"
    );
}

#[test]
fn manual_movement_takes_over_a_gather_session_with_no_walk_in_flight() {
    let mut rig = GatherReconnectRig::new();
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    assert!(
        !rig.slot().lock().unwrap().live_walking_operation(),
        "the Gatherer works in place: no walking operation"
    );
    assert!(
        rig.navs
            .lock()
            .unwrap()
            .get(SLOT)
            .is_none_or(|bot| !bot.has_route()
                && bot.pending_route.is_none()
                && !bot.native_walk.as_ref().is_some_and(|owner| owner.live())),
        "no host walk is in flight"
    );
    let manual = crate::SlotFrameInput {
        manual_move_intent: Some(crate::ManualMoveIntent::Minimap),
        ..Default::default()
    };
    assert!(
        crate::script_runtime::take_manual_walk_ownership(
            &rig.scripts,
            &rig.navs,
            SLOT,
            manual,
            true,
            rig.tick,
            true,
        ),
        "manual movement takes over the session's foreground"
    );
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Paused,
        "Pause script on manual movement applies"
    );
    rig.slot().lock().unwrap().resume();
    for _ in 0..32 {
        rig.frame();
        if rig.probe("globalThis.__settleCount") == 1 {
            break;
        }
    }
    let result = rig.probe("globalThis.__runResult");
    assert_eq!(result["kind"], "done", "{result}");
    assert_eq!(result["value"]["end"], "blocked", "{result}");
    assert_eq!(result["value"]["token"], token, "{result}");
    assert_eq!(
        result["value"]["failure"]["code"], "manual-movement",
        "{result}"
    );
}

#[test]
fn gather_reconnect_terminal_before_boundary_settles_once_across_keyframes() {
    let mut rig = GatherReconnectRig::new();
    let token = rig.start_through_host();
    rig.expect_running_page(token);

    let _ = rig.probe("globalThis.__stop = true");
    rig.isolate_tick_without_host_drain();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(!owned, "the real typed Stop releases the host seat");
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    assert_eq!(rig.probe("globalThis.__settleCount"), 0);

    rig.isolate_tick_without_host_drain();
    rig.settled_result(token);
    for _ in 0..2 {
        rig.reconnect();
        rig.isolate_tick_without_host_drain();
        rig.settled_result(token);
    }
}

#[test]
fn gather_reconnect_lost_stop_reemits_stop_without_restarting_seat() {
    let mut rig = GatherReconnectRig::new();
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    let before = rig.native_run();

    let _ = rig.probe("globalThis.__stop = true");
    rig.isolate_tick_without_host_drain();
    assert!(
        rig.slot().lock().unwrap().api_owns_foreground(),
        "an unconsumed Stop cannot release the host seat before its batch is dispatched"
    );
    rig.reconnect();
    assert_eq!(rig.native_run().session, before.session.wrapping_add(1));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (rows, owned) = rig.drain_host();
        assert!(rows.is_empty(), "the typed Stop has no ordinary host row");
        if !owned {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the carried Stop did not release the original Gatherer seat"
        );
        rig.isolate_tick_without_host_drain();
        let _ = rig.probe("true");
        std::thread::yield_now();
    }
    assert!(
        rig.slot().lock().unwrap().native_status().is_none(),
        "Stop removes the original Gatherer without admitting a replacement"
    );
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    rig.isolate_tick_without_host_drain();
    rig.settled_result(token);
}

#[test]
fn guardian_no_navigation_terminal_drains_after_guardian_release() {
    let mut rig = GatherReconnectRig::new();
    rig.walk_danger = None;
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    assert!(rig.slot().lock().unwrap().api_owns_foreground());

    // No data only blocks a needed walk, not work at the current stand.
    // Move the observed player away while keeping the target tree in place.
    rig.tile.x += 4;
    let mut player = rig.snapshot.local_player().unwrap().clone();
    player.player.actor.tile = rig.tile;
    player.player.network = rig.tile;
    rig.snapshot.seed_local_player(player);

    let _ = rig.probe("globalThis.__stop = true");
    rig.slot().lock().unwrap().seat_guardian_hold(true);
    for tick in 0..40 {
        rig.isolate_tick_without_host_drain();
        if tick % 2 == 0 {
            let _ = rig.drain_host();
        } else {
            rig.slot().lock().unwrap().retain_held_seat_batches();
        }
    }

    let result = rig.probe("globalThis.__runResult");
    assert_eq!(result["value"]["end"], "blocked", "{result}");
    assert_eq!(
        result["value"]["failure"]["code"], "resource-unavailable",
        "{result}"
    );
    assert!(
        result["value"]["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("no navigation data")),
        "{result}"
    );
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    assert!(
        rig.slot().lock().unwrap().seat_batch_backlog() > 0,
        "the held Stop must exercise the retained-batch drain"
    );

    rig.slot().lock().unwrap().seat_guardian_hold(false);
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(!owned);
    let cell = rig.slot();
    let slot = cell.lock().unwrap();
    assert_eq!(
        slot.seat_batch_backlog(),
        0,
        "state={:?}, seat={:?}",
        slot.state(),
        slot.seat_probe()
    );
    drop(slot);
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    assert_eq!(rig.probe("globalThis.__settleCount"), 1);
    assert_eq!(rig.probe("globalThis.__runResult"), result);
}

#[test]
fn guardian_hold_keeps_isolate_seat_batch_backlog_bounded() {
    let mut rig = GatherReconnectRig::new();
    rig.walk_danger = Some(&AllowWalkDanger);
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    let _ = rig.probe("globalThis.__stop = true");
    rig.slot().lock().unwrap().seat_guardian_hold(true);
    for tick in 0..40 {
        rig.isolate_tick_without_host_drain();
        if tick % 2 == 0 {
            let _ = rig.drain_host();
        } else {
            // Paused guardian frames do not drain host interactions.
            rig.slot().lock().unwrap().retain_held_seat_batches();
        }
        let pending = rig.slot().lock().unwrap().seat_batch_backlog();
        assert!(
            pending <= 2,
            "guardian hold must not retain one seat batch per tick: tick={tick}, pending={pending}"
        );
    }
    rig.slot().lock().unwrap().seat_guardian_hold(false);
    assert!(
        rig.slot().lock().unwrap().api_owns_foreground(),
        "the active-seat fixture must remain live throughout the forty held frames"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let _ = rig.drain_host();
        if !rig.slot().lock().unwrap().api_owns_foreground() {
            break;
        }
        assert!(Instant::now() < deadline, "held Stop did not dispatch");
        rig.isolate_tick_without_host_drain();
        let _ = rig.probe("true");
    }
    assert_eq!(rig.slot().lock().unwrap().seat_batch_backlog(), 0);
}

#[test]
fn paused_only_guardian_frames_keep_seat_backlog_bounded() {
    let mut rig = GatherReconnectRig::new();
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    let _ = rig.probe("globalThis.__stop = true");
    rig.slot().lock().unwrap().seat_guardian_hold(true);
    let mut max_pending = 0;
    for _ in 0..40 {
        rig.isolate_tick_without_host_drain();
        rig.slot().lock().unwrap().retain_held_seat_batches();
        max_pending = max_pending.max(rig.slot().lock().unwrap().seat_batch_backlog());
    }
    assert!(
        max_pending <= 2,
        "paused-only guardian frames must retain bounded batches: {max_pending}"
    );
    rig.slot().lock().unwrap().seat_guardian_hold(false);
}

#[test]
fn gather_reconnect_lost_start_and_stop_replays_pair_then_one_terminal() {
    let mut rig = GatherReconnectRig::new();
    let _ = rig.probe("globalThis.__pair = true");
    rig.isolate_tick_without_host_drain();
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());

    rig.reconnect();
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    rig.isolate_tick_without_host_drain();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(
        !owned,
        "the same-batch typed start and release leave no live seat"
    );
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());

    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.probe("globalThis.__settleCount") == 0 {
        rig.frame();
        assert!(
            Instant::now() < deadline,
            "the replayed typed start and release did not settle"
        );
    }
    let result = rig.settled_result_from_wire();
    let token = result["value"]["token"]
        .as_u64()
        .expect("stopped terminal carries its public token");
    for _ in 0..2 {
        rig.reconnect();
        rig.isolate_tick_without_host_drain();
        assert_eq!(rig.settled_result(token), result);
    }
}

#[test]
fn recreated_load_isolates_never_inherit_old_seat_pages_or_authority() {
    for boundary in ["stop", "replace", "watchdog", "remove"] {
        for terminal_before_boundary in [false, true] {
            let mut rig = GatherReconnectRig::new();
            let token = rig.start_through_host();
            rig.expect_running_page(token);
            let old_run = rig.native_run();
            let selected = rig.slot().lock().unwrap().compiled_game_data().unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            let old_action = loop {
                let cell = rig.slot();
                let mut slot = cell.lock().unwrap();
                slot.on_game_tick(&mut script::ScriptCtx {
                    driver: &mut rig.client,
                    tick: rig.tick,
                    here: Some((rig.tile.x, rig.tile.z, rig.tile.level)),
                    walk: None,
                    walk_with: None,
                    inv: None,
                    snapshot: Some(&rig.snapshot),
                    obj_names: Some(rig.names.as_ref()),
                    compiled: script::CompiledTick {
                        selected: Some(selected.as_ref()),
                        ..Default::default()
                    },
                });
                rig.tick += 1;
                if let Some(action) = slot.take_native_action() {
                    break action;
                }
                assert!(Instant::now() < deadline, "native action was not queued");
                std::thread::yield_now();
            };
            let authority = old_action.authority();
            assert!(authority.live());
            if terminal_before_boundary {
                let _ = rig.probe("globalThis.__stop = true");
                rig.isolate_tick_without_host_drain();
                let (rows, owned) = rig.drain_host();
                assert!(rows.is_empty());
                assert!(!owned);
                assert!(!rig.slot().lock().unwrap().api_owns_foreground());
                assert_eq!(rig.isolate_tick_without_host_drain(), (false, true));
                rig.settled_result(token);
            }
            match boundary {
                "stop" => {
                    rig.slot().lock().unwrap().stop();
                    let deadline = Instant::now() + Duration::from_secs(60);
                    loop {
                        let cell = rig.slot();
                        let mut slot = cell.lock().unwrap();
                        slot.poll_start();
                        if slot.state() == script::RunState::Idle {
                            break;
                        }
                        assert!(Instant::now() < deadline, "operator Stop did not reap");
                        std::thread::yield_now();
                    }
                    start_source(&rig.scripts, Arc::clone(&selected), RECONNECT_SOURCE);
                }
                "replace" => {
                    rig.slot().lock().unwrap().stop();
                    start_source(&rig.scripts, Arc::clone(&selected), RECONNECT_SOURCE);
                }
                "watchdog" => {
                    rig.slot()
                        .lock()
                        .unwrap()
                        .restart_from_identity(Instant::now())
                        .unwrap();
                    let deadline = Instant::now() + Duration::from_secs(60);
                    loop {
                        let cell = rig.slot();
                        let mut slot = cell.lock().unwrap();
                        slot.poll_start();
                        if slot.state() == script::RunState::Running && slot.surface_active() {
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "watchdog recreate did not settle"
                        );
                        std::thread::yield_now();
                    }
                }
                "remove" => {
                    rig.slot().lock().unwrap().stop();
                    rig.scripts.lock().unwrap().remove(SLOT);
                    start_source(&rig.scripts, Arc::clone(&selected), RECONNECT_SOURCE);
                }
                _ => unreachable!(),
            }
            assert!(!authority.live(), "{boundary} must revoke old native work");
            assert!(!rig.slot().lock().unwrap().api_owns_foreground());
            assert!(!rig.slot().lock().unwrap().has_native_actions());
            assert_eq!(
                rig.isolate_tick_without_host_drain(),
                (false, false),
                "{boundary} must clear both live and terminal wire pages"
            );
            assert!(rig.probe("globalThis.__page === null") == true);
            assert_eq!(rig.probe("globalThis.__settleCount"), 0);
            // Request tokens are isolate-local and may be reused after recreate.
            let next_token = rig.start_through_host();
            rig.expect_running_page(next_token);
            assert_ne!(
                rig.native_run(),
                old_run,
                "{boundary} creates a fresh RunKey"
            );
            assert_eq!(
                rig.probe("globalThis.__settleCount"),
                0,
                "a new waiter must not consume the old terminal"
            );
            rig.stop_and_settle(next_token);
        }
    }
}

#[test]
fn progress_pending_across_reconnect_completes_once_then_admits_next_session() {
    let mut rig = GatherReconnectRig::with_source(
        r#"export const apiVersion = 2;
export function tick(api) { globalThis.__api = api; }"#,
    );
    rig.snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    rig.isolate_tick_without_host_drain();
    rig.probe(
        "globalThis.__firstSettles=0; __api.questProgress({quest:'cook'}).then(out=>{globalThis.__first=out; __firstSettles++}); true",
    );
    rig.isolate_tick_without_host_drain();

    assert!(!rig.slot().lock().unwrap().api_owns_foreground());

    rig.reconnect();
    assert!(!rig.slot().lock().unwrap().api_owns_foreground());
    rig.isolate_tick_without_host_drain();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(owned, "the host admits the carried typed progress request");
    let first_deadline = Instant::now() + Duration::from_secs(60);
    while rig.probe("globalThis.__first || null").is_null() {
        rig.frame();
        assert!(
            Instant::now() < first_deadline,
            "carried progress waiter did not receive its host terminal"
        );
    }
    let first = rig.probe("globalThis.__first");
    assert_eq!(first["kind"], "done");
    assert_eq!(first["value"]["end"], "done");
    assert_eq!(first["value"]["token"], 1);
    assert_eq!(first["value"]["row"]["stage"]["value"], "cook:2");
    assert_eq!(rig.probe("globalThis.__firstSettles"), 1);

    // Progress is a Session: a second root cannot supersede it while live.
    // Once the first terminal is credited, the next request is admitted.
    rig.probe(
        "globalThis.__secondSettles=0; __api.questProgress({quest:'cook'}).then(out=>{globalThis.__second=out; __secondSettles++}); true",
    );
    rig.isolate_tick_without_host_drain();
    let (rows, owned) = rig.drain_host();
    assert!(rows.is_empty());
    assert!(owned, "the host admits the next typed progress request");

    let deadline = Instant::now() + Duration::from_secs(60);
    while rig.probe("globalThis.__second || null").is_null() {
        rig.frame();
        assert!(Instant::now() < deadline, "second progress waiter stranded");
        std::thread::yield_now();
    }
    let terminal = rig.probe("globalThis.__second");
    assert_eq!(terminal["kind"], "done");
    assert_eq!(terminal["value"]["end"], "done");
    assert_eq!(terminal["value"]["token"], 2);
    assert_eq!(terminal["value"]["row"]["stage"]["value"], "cook:2");
    assert_eq!(rig.probe("globalThis.__firstSettles"), 1);
    assert_eq!(rig.probe("globalThis.__secondSettles"), 1);

    rig.reconnect();
    rig.isolate_tick_without_host_drain();
    assert_eq!(rig.probe("globalThis.__second"), terminal);
    assert_eq!(rig.probe("globalThis.__secondSettles"), 1);
}

const WALK_PERMISSION_SOURCE: &str = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__api = api;
  const page = api.snapshot.gather;
  globalThis.__page = page;
  if (globalThis.__launch && !globalThis.__run) {
    const settings = Object.assign(
      { skill: 'Woodcutting', disposition: 'Power' },
      globalThis.__settings || {}
    );
    globalThis.__run = api.gather.run(settings);
  }
}
"#;

fn active_gather_permission_rig(settings: serde_json::Value) -> GatherReconnectRig {
    let mut rig = GatherReconnectRig::with_source(WALK_PERMISSION_SOURCE);
    rig.probe(&format!("globalThis.__settings = {settings}; true"));
    let token = rig.start_through_host();
    rig.expect_running_page(token);
    rig
}

fn danger_route_world() -> NavWorld {
    let width = 12;
    let height = 1;
    let flags = vec![0u32; width * height * 4];
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let collision = nav_planner::collision::WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = nav_planner::transport::TransportGraph::default();
    graph.zones = Some(
        nav_planner::zones::ZoneTable::from_parts(
            vec![nav_planner::zones::Zone::npc(
                WorldTile {
                    x: 6,
                    z: 0,
                    level: 0,
                },
                0,
                nav_planner::zones::ZoneClass::Always,
                u16::MAX,
                0,
            )],
            vec![nav_planner::zones::ZoneKind::new(
                "danger",
                "Danger zone",
                1,
                1,
                false,
                false,
            )],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            collision.origin,
            collision.width as u32,
            collision.height as u32,
            &graph.wilderness,
        )
        .expect("valid synthetic danger zone"),
    );
    NavWorld::from_parts(collision, graph, Vec::new())
}

fn wilderness_route_world() -> NavWorld {
    let flags = vec![0u32; 5 * 12];
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let graph = nav_planner::transport::TransportGraph {
        wilderness: nav_planner::transport::WildernessRules {
            zones: vec![nav_planner::transport::WildernessZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 6399,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            }],
            divisor: 8,
            offset: 1,
        },
        ..nav_planner::transport::TransportGraph::default()
    };
    NavWorld::from_parts(
        nav_planner::collision::WorldCollision {
            origin: WorldTile {
                x: 3099,
                z: 3518,
                level: 0,
            },
            width: 5,
            height: 12,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    )
}

fn route_exists(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    options: nav_planner::router::FindOptions,
) -> bool {
    nav_planner::router::find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        options,
        &nav_planner::WorldState::default(),
        &[],
    )
    .is_ok()
}

#[test]
fn load_gather_inspect_route_keeps_load_script_options() {
    let mut rig = active_gather_permission_rig(serde_json::json!({
        "allowDangerZones": true,
        "allowWilderness": true,
        "allowTeleports": true,
    }));
    rig.navs.lock().unwrap().get_mut(SLOT).unwrap().walk_globals =
        Some(Arc::new(Mutex::new(crate::WalkGlobals::fail_closed())));

    let _ = rig.drain_host();
    assert_eq!(
        rig.probe("globalThis.__api.request({op:'inspect-route', from:{x:3200,z:3200,level:0}, to:{x:3210,z:3210,level:0}, allow_bank_fetch:true, request_id:0}); true"),
        serde_json::Value::Bool(true)
    );
    rig.isolate_tick_without_host_drain();
    let (requests, owned) = rig.drain_host();
    assert!(owned, "the Load Gather session owns the foreground");
    let (allow_teleports, allow_wilderness, allow_bank_fetch) = requests
        .iter()
        .find_map(|request| match request {
            script::shim::InteractReq::InspectRoute {
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                ..
            } => Some((*allow_teleports, *allow_wilderness, *allow_bank_fetch)),
            _ => None,
        })
        .expect("the Load script's inspect-route row survives while Gather owns foreground");
    let options = crate::walk_permissions::compiled_options(
        &rig.navs,
        SLOT,
        nav_planner::router::FindOptions {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            ..nav_planner::router::FindOptions::default()
        },
    );
    assert_eq!(
        (
            options.allow_teleports,
            options.allow_wilderness,
            options.allow_bank_fetch
        ),
        (false, false, true),
        "Load script bools must not inherit the session card's Gather permissions"
    );
    rig.slot().lock().unwrap().stop();
}

#[test]
fn load_gather_watchdog_arm_walk_keeps_load_script_options() {
    let mut rig = active_gather_permission_rig(serde_json::json!({
        "allowDangerZones": true,
        "allowWilderness": true,
        "allowTeleports": true,
    }));
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut(SLOT).unwrap();
        bot.walk_globals = Some(Arc::new(Mutex::new(crate::WalkGlobals::fail_closed())));
        assert_eq!(bot.native_permissions, None);
    }
    assert_eq!(
        rig.slot().lock().unwrap().api_gather_walk_permissions(),
        Some(script::native::WalkPermissions {
            allow_teleports: true,
            allow_wilderness: true,
            allow_danger_zones: true,
        })
    );

    let world = Some(Arc::new(wilderness_route_world()));
    crate::script_runtime::apply_watchdog_nav_action(
        script::WatchdogAction::ArmWalk {
            x: 3100,
            z: 3525,
            level: 0,
        },
        &mut rig.client,
        Some(&rig.snapshot),
        Some((3100, 3519, 0)),
        &rig.navs,
        &world,
        Some(&nav_planner::WorldState::empty()),
        None,
        SLOT,
    );

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        crate::script_runtime::pump_route_work(&rig.navs, SLOT);
        if !rig.navs.lock().unwrap()[SLOT].route_work_pending() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watchdog ArmWalk route work did not settle"
        );
        std::thread::yield_now();
    }
    let navs = rig.navs.lock().unwrap();
    let bot = &navs[SLOT];
    assert!(
        bot.walk_outcome_failed,
        "the non-inheriting arm cannot cross"
    );
    assert!(!bot.has_route());
    assert!(bot.requested_route.is_none());
    drop(navs);
    rig.slot().lock().unwrap().stop();
}

#[test]
fn load_api_gather_permissions_admit_danger_and_wilderness_routes() {
    let defaults = active_gather_permission_rig(serde_json::json!({}));
    let allowed = active_gather_permission_rig(serde_json::json!({
        "allowDangerZones": true,
        "allowWilderness": true,
    }));
    for rig in [&defaults, &allowed] {
        let mut navs = rig.navs.lock().unwrap();
        navs.get_mut(SLOT).unwrap().walk_globals =
            Some(Arc::new(Mutex::new(crate::WalkGlobals::fail_closed())));
    }
    let default_options = crate::walk_permissions::native_admission(
        &defaults.navs,
        SLOT,
        script::native::WalkOptions::default(),
        defaults
            .slot()
            .lock()
            .unwrap()
            .api_gather_walk_permissions(),
    )
    .0;
    let allowed_options = crate::walk_permissions::native_admission(
        &allowed.navs,
        SLOT,
        script::native::WalkOptions::default(),
        allowed.slot().lock().unwrap().api_gather_walk_permissions(),
    )
    .0;

    let danger_world = danger_route_world();
    let danger_from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let danger_to = WorldTile {
        x: 11,
        z: 0,
        level: 0,
    };
    assert!(!route_exists(
        &danger_world,
        danger_from,
        danger_to,
        default_options,
    ));
    assert!(route_exists(
        &danger_world,
        danger_from,
        danger_to,
        allowed_options,
    ));

    let wilderness_world = wilderness_route_world();
    let wilderness_from = WorldTile {
        x: 3100,
        z: 3519,
        level: 0,
    };
    let wilderness_to = WorldTile {
        x: 3100,
        z: 3525,
        level: 0,
    };
    assert!(!route_exists(
        &wilderness_world,
        wilderness_from,
        wilderness_to,
        default_options,
    ));
    assert!(route_exists(
        &wilderness_world,
        wilderness_from,
        wilderness_to,
        allowed_options,
    ));

    defaults.slot().lock().unwrap().stop();
    allowed.slot().lock().unwrap().stop();
}

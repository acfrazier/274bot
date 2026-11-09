use super::*;
use api::gather_methods::{known_rows, GatherSkill, TargetClass, TickArmKind};
use api::selected::EntityId;
use api::snapshot::{
    AnimationUpdateView, GameSnapshot, ItemContainer, LocLayer, LocView, SceneView, StatView,
    WorldStateView, WorldTile,
};
use std::sync::Arc;

#[test]
fn ordered_real_gatherer_knife_arm_uses_packet_clock_after_relog() {
    let Some(mut rig) = Rig::with_game_data() else {
        return;
    };
    if api::test_support::local_asset_path(
        "",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/data/game-data/289/gathering.json"
        ),
        "revision 289 gathering catalog for the real Gatherer relog",
    )
    .is_none()
    {
        return;
    }
    const BEFORE_RELOG_TICK: u64 = 50_000;
    const BEFORE_RELOG_PACKET_TICK: u64 = 50_000;
    const RESET_NATIVE_TICK: u64 = 50_001;
    const RESET_PACKET_TICK: u64 = 1;
    const RAW_GENERATION_BEFORE_RELOG: u64 = 700_000;

    rig.client.map_build_base_x = 3200;
    rig.client.map_build_base_z = 3328;
    rig.client.local_player = Some(client::dash3d::ClientPlayer::at(84, 40));
    let player = rig.client.local_player.as_mut().unwrap();
    player.entity.x = 84 * 128 + 64;
    player.entity.z = 40 * 128 + 64;
    rig.client.gens.player_info = RAW_GENERATION_BEFORE_RELOG;
    rig.client.gens.player += 1;
    rig.snapshot.rebuild(&rig.client);

    let selected = Arc::clone(&rig.selected);
    let mut settings = script::native::SettingsBag::new();
    settings.insert("skill".into(), serde_json::json!("Mining"));
    settings.insert("miningResources".into(), serde_json::json!(["iron"]));
    settings.insert("location".into(), serde_json::json!("Start"));
    settings.insert("radius".into(), serde_json::json!(12));
    settings.insert("disposition".into(), serde_json::json!("Power"));
    settings.insert("tickMethod".into(), serde_json::json!("Knife"));

    let slot = script_slot_or_insert(&rig.scripts, "so3-wire");
    {
        let mut slot = slot.lock().unwrap();
        slot.bind_world_members(api::selected::Truth::True);
        slot.start_compiled(
            "so3-wire",
            script::CompiledId("Gatherer"),
            Arc::new(settings),
            Arc::clone(&selected),
            Arc::default(),
            None,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match slot.poll_start() {
                script::StartPoll::Settled(script::StartOutcome::Ready) => break,
                script::StartPoll::Pending => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Gatherer preparation timed out"
                    );
                    std::thread::yield_now();
                }
                other => panic!("Gatherer start failed: {other:?}"),
            }
        }
    }

    let gathering = selected
        .try_gathering()
        .expect("compiled Gatherer preparation loads the selected gathering catalog");
    let method = gathering
        .method("mining.iron")
        .expect("selected iron mining method");
    assert_eq!(method.skill, GatherSkill::Mining);
    assert_eq!(method.tick_arm.kind, TickArmKind::Continuing);
    let resource = known_rows(&method.targets)
        .iter()
        .find(|target| target.class == TargetClass::Resource)
        .expect("selected iron method has a real resource target");
    let EntityId::Loc(loc_id) = resource.entity else {
        panic!(
            "selected iron resource target must be a loc: {:?}",
            resource.entity
        );
    };
    let (loc_op, action) = gathering
        .op(method)
        .unwrap()
        .expect("selected iron method has an interaction option");
    assert!(loc_op > 0);
    let tool_id = known_rows(&method.tools)
        .iter()
        .find(|tool| {
            method
                .tick_arm
                .for_tool(tool.item)
                .is_some_and(|facts| !facts.no_gain)
        })
        .expect("selected iron method has a timed tool")
        .item;
    let skill_animation = method
        .tick_arm
        .for_tool(tool_id)
        .expect("selected tool timing facts")
        .skill_anim;

    let ordered = selected.server_ordered_catalog().unwrap();
    let knife_id = ordered.knife().knife().id();
    let logs_id = ordered.knife().logs()[0].id();

    let local_x = 85;
    let local_z = 40;
    let target_tile = WorldTile {
        x: 3200 + local_x,
        z: 3328 + local_z,
        level: 0,
    };
    {
        let cache = Arc::get_mut(&mut rig.client.cache).unwrap();
        let loc_index = usize::try_from(loc_id).unwrap();
        if cache.locs.len() <= loc_index {
            cache.locs.resize_with(loc_index + 1, Default::default);
        }
        cache.locs[loc_index].width = 1;
        cache.locs[loc_index].length = 1;
    }

    let mut equipment_tool = inventory_item(tool_id, 0, "Selected mining tool");
    equipment_tool.container = ItemContainer::Equipment;
    equipment_tool.actions = vec![Some("Remove".into())];
    equipment_tool.component_id = -1;
    let inventory = vec![
        inventory_item(knife_id, 0, "Knife"),
        inventory_item(logs_id, 1, "Logs"),
    ];
    let target_name = gathering
        .alias(resource.entity)
        .unwrap_or("iron rock")
        .to_owned();
    let loc = LocView {
        typecode: (loc_id << 14) | (local_z << 7) | local_x,
        info: 0,
        id: loc_id,
        name: Some(target_name),
        description: None,
        actions: vec![Some(action.into())],
        tile: target_tile,
        distance: 1,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: true,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    };
    let player_template = rig
        .snapshot
        .local_player()
        .expect("loopback client has a local player")
        .clone();
    let seed_runtime_facts = |snapshot: &mut GameSnapshot, serial: u32, animation: i32| {
        let mut player = player_template.clone();
        player.animation_update = Some(AnimationUpdateView {
            serial,
            sequence: animation,
        });
        player.player.actor.animation = animation;
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3328,
            level: 0,
            members: true,
            multi_combat: false,
            player_count: 1,
            npc_count: 0,
            cycle: 0,
        });
        snapshot.seed_scene(SceneView {
            available: true,
            base_x: 3200,
            base_z: 3328,
            level: 0,
            width: 104,
            height: 104,
            collision_flags: vec![0; 104 * 104],
        });
        snapshot.seed_local_player(player);
        snapshot.seed_locs(vec![loc.clone()]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_stats(vec![
            StatView {
                index: 0,
                name: "attack".into(),
                effective: 99,
                base: 99,
                xp: 0,
                used: true,
            },
            StatView {
                index: 14,
                name: "mining".into(),
                effective: 99,
                base: 99,
                xp: 0,
                used: true,
            },
        ]);
        snapshot.seed_inventory(inventory.clone(), 28);
        snapshot.seed_inventory_packet_generations(Some(1), None);
        snapshot.seed_equipment(vec![equipment_tool.clone()]);
    };

    seed_runtime_facts(&mut rig.snapshot, 1, -1);
    let _hook_reset = TickHookReset;
    let hook = crate::tick_engine_test_support::TickEngineTestHook::new("so3-wire");
    hook.bind_client(&mut rig.client, "so3-wire", Arc::clone(&rig.input));
    rig.pump_three_clocks(
        BEFORE_RELOG_TICK,
        BEFORE_RELOG_PACKET_TICK as u32,
        RAW_GENERATION_BEFORE_RELOG,
    );

    rig.client.gens.session += 1;
    rig.client.gens.player_info = RAW_GENERATION_BEFORE_RELOG;
    rig.snapshot.reset_session(rig.client.gens);
    rig.client.gens.player_info += 1;
    rig.client.gens.player += 1;
    rig.snapshot.rebuild(&rig.client);
    seed_runtime_facts(&mut rig.snapshot, 2, skill_animation);
    hook.bind_client(&mut rig.client, "so3-wire", Arc::clone(&rig.input));
    hook.arm_measurement(logs_id, &[knife_id, logs_id]);

    let mut observed_dispatch = None;
    for offset in 0..8_u64 {
        let native_tick = RESET_NATIVE_TICK + offset;
        let packet_tick = RESET_PACKET_TICK + offset;
        rig.pump_three_clocks(
            native_tick,
            packet_tick as u32,
            RAW_GENERATION_BEFORE_RELOG + packet_tick,
        );
        if !hook.trace().dispatches.is_empty() {
            observed_dispatch = Some((native_tick, packet_tick));
            break;
        }
    }
    let (native_tick, packet_tick) = observed_dispatch.unwrap_or_else(|| {
        panic!(
            "real compiled Gatherer must emit and dispatch KnifeArm after relog: status={:?}, packets={:?}",
            slot.lock().unwrap().native_status(),
            rig.decoded()
        )
    });
    let trace = hook.trace();
    assert_eq!(
        trace.dispatches.len(),
        1,
        "real Gatherer dispatches: {trace:?}"
    );
    let dispatch = &trace.dispatches[0];
    let receipt = dispatch.receipt;
    assert!(
        dispatch
            .request_debug
            .as_deref()
            .is_some_and(|request| request.contains("KnifeArm")),
        "captured request must be the Gatherer's real typed KnifeArm: {dispatch:?}"
    );
    assert_eq!(
        receipt.row_count, 3,
        "continuing KnifeArm rows: {receipt:?}"
    );
    assert!(receipt.terminal.is_none(), "KnifeArm dispatch: {receipt:?}");
    assert!(
        receipt.dispatched_prefix > 0,
        "KnifeArm prefix: {receipt:?}"
    );
    assert_eq!(
        receipt.dispatched_prefix, receipt.row_count,
        "the complete KnifeArm prefix must reach the encrypted driver: {receipt:?}"
    );
    assert_eq!(
        receipt.dispatch_tick,
        RAW_GENERATION_BEFORE_RELOG + packet_tick,
        "packet-clock receipt: {receipt:?}"
    );
    assert_eq!(
        dispatch.game_tick, native_tick,
        "native completion clock: {dispatch:?}"
    );
    assert_eq!(rig.snapshot.tick(), packet_tick as u32);
    let input_observation = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(
        input_observation.observed_tick(),
        Some((RAW_GENERATION_BEFORE_RELOG + packet_tick) as u32)
    );
    assert_eq!(
        input_observation.session_generation(),
        rig.client.gens.session
    );
    assert_eq!(
        trace
            .wire
            .iter()
            .filter(|packet| packet.request_id == Some(receipt.request_id))
            .count(),
        usize::from(receipt.wire_events),
        "encrypted Client packets for the real KnifeArm dispatch: {trace:?}"
    );

    slot.lock().unwrap().stop();
}

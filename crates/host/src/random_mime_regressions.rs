use super::{FakeDriver, Guardian, MiniMenuAction, ProfileSettings};
use api::snapshot::{
    ActorView, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
    LocalPlayerView, PlayerView, SceneView, SideTabView, StatView, WorldStateView, WorldTile,
};
use script::native::HostEffect;
use script::shim::InteractReq;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIME_REWARD_ROOT: i32 = 356;
const MIME_REWARD_CONTINUE: i32 = 358;
const INVENTORY_TAB_INDEX: i32 = 3;
const INVENTORY_INTERFACE_ID: i32 = 3_213;
const YEWTREE_ID: i32 = 1_309;

fn inventory_tab(root_component_id: i32) -> SideTabView {
    SideTabView {
        index: INVENTORY_TAB_INDEX,
        root_component_id,
        available: root_component_id != -1,
        active: false,
        visible: false,
        widgets: vec![],
    }
}

fn mime_gatherer_snapshot() -> (GameSnapshot, WorldTile) {
    let target = WorldTile {
        x: 3_085,
        z: 3_468,
        level: 0,
    };
    let here = WorldTile {
        x: target.x + 3,
        z: target.z + 1,
        ..target
    };
    let base_x = target.x - 52;
    let base_z = target.z - 52;
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: base_x,
        map_base_z: base_z,
        level: 0,
        members: true,
        ..Default::default()
    });
    snapshot.seed_scene(SceneView {
        available: true,
        base_x,
        base_z,
        level: 0,
        width: 104,
        height: 104,
        collision_flags: vec![0; 104 * 104],
    });
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor: ActorView {
                name: Some("Mime regression".into()),
                actions: vec![],
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
                health: 10,
                total_health: 10,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            network: here,
            combat_level: 3,
            skill_level: 3,
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
        base: 60,
        effective: 60,
        xp: 0,
        used: true,
    }]);
    snapshot.seed_equipment(vec![ItemView {
        def: api::ItemDefView {
            id: 1_351,
            name: Some("Bronze axe".into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Equipment,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![Some("Remove".into())],
        component_id: -1,
    }]);
    snapshot.seed_side_tabs(vec![inventory_tab(INVENTORY_INTERFACE_ID)], -1);
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_locs(vec![LocView {
        typecode: 0,
        info: 0,
        id: YEWTREE_ID,
        name: Some("Yew".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile: target,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 3,
        length: 3,
        footprint_width: 3,
        footprint_length: 3,
        block_walk: true,
        block_range: true,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    snapshot.seed_npcs(vec![]);
    (snapshot, here)
}

fn tick_gatherer(
    slot: &mut script::SlotScript,
    driver: &mut FakeDriver,
    tick: u64,
    snapshot: &GameSnapshot,
    here: WorldTile,
    inv: Option<&[(i32, i32)]>,
) {
    slot.on_game_tick(&mut script::ScriptCtx {
        driver,
        tick,
        here: Some((here.x, here.z, here.level)),
        walk: None,
        walk_with: None,
        inv,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: script::CompiledTick::default(),
    });
}

fn drain_interactions(slot: &mut script::SlotScript) -> Vec<InteractReq> {
    let mut interactions = Vec::new();
    while let Some(action) = slot.take_native_action() {
        if let HostEffect::Interaction(request) = action.effect {
            interactions.push(request);
        }
    }
    interactions
}

fn is_yew_chop(request: &InteractReq) -> bool {
    matches!(
        request,
        InteractReq::Loc {
            id: Some(YEWTREE_ID),
            action,
            ..
        } if action == "Chop down"
    )
}

#[test]
fn mime_unlock_page_is_continued_before_inventory_readiness() {
    let (mut snapshot, here) = mime_gatherer_snapshot();
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let mut settings = script::native::SettingsBag::new();
    settings.insert("skill".into(), serde_json::json!("Woodcutting"));
    settings.insert("woodcuttingResources".into(), serde_json::json!(["yew"]));
    settings.insert("location".into(), serde_json::json!("Start"));
    settings.insert("radius".into(), serde_json::json!(12));
    settings.insert("disposition".into(), serde_json::json!("Power"));

    let mut slot = script::SlotScript::new();
    slot.bind_incarnation(4_270);
    slot.start_compiled(
        "mime-regression",
        script::CompiledId("Gatherer"),
        Arc::new(settings),
        selected,
        Arc::default(),
    )
    .unwrap();
    let start_deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match slot.poll_start() {
            script::StartPoll::Settled(outcome) => {
                assert_eq!(outcome, script::StartOutcome::Ready);
                break;
            }
            script::StartPoll::Pending => {
                assert!(
                    Instant::now() < start_deadline,
                    "Gatherer preparation timed out"
                );
                std::thread::yield_now();
            }
            script::StartPoll::NotOwed => panic!("Gatherer start was not owed"),
        }
    }

    let mut driver = FakeDriver::default();
    let mut tick = 1;
    let mut gathering = false;
    for _ in 0..8 {
        tick_gatherer(&mut slot, &mut driver, tick, &snapshot, here, Some(&[]));
        let requests = drain_interactions(&mut slot);
        gathering |= requests.iter().any(is_yew_chop);
        tick += 1;
        if gathering {
            break;
        }
    }
    assert!(
        gathering,
        "the hosted Gatherer must start its selected Yew resource"
    );

    // The hosted random-event path holds the slot while the real Guardian
    // answers the Mime's emote page, then releases it after the return.
    let mut client = super::new_client();
    super::ingame_scene(&mut client);
    client.side_icon[INVENTORY_TAB_INDEX as usize] = -1;
    super::plant_player(&mut client, "Mime regression", 31 * 64, 74 * 64);
    super::plant_npc(&mut client, 0, "Mime", -1, None);
    client.npc[0]
        .as_mut()
        .expect("planted mime")
        .entity
        .primary_anim = 860;
    let mut guardian = Guardian::new();
    let mut guardian_driver = FakeDriver::default();
    let guardian_settings = ProfileSettings::default();
    let mut guardian_snapshot = GameSnapshot::new();
    super::tick_at(&mut client, &mut guardian_snapshot);
    let status = guardian.tick(
        &mut guardian_driver,
        &guardian_snapshot,
        &guardian_settings,
        0,
        None,
    );
    assert!(
        status.hold,
        "Guardian holds the MIME stage while the script is paused"
    );

    client.chat_modal_id = 6_543;
    super::tick_at(&mut client, &mut guardian_snapshot);
    let status = guardian.tick(
        &mut guardian_driver,
        &guardian_snapshot,
        &guardian_settings,
        1,
        None,
    );
    assert!(
        status.hold,
        "Guardian keeps holding while the MIME emote page is handled"
    );
    assert_eq!(
        guardian_driver.menus,
        vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6_546)]
    );
    assert_eq!(guardian_driver.actions, vec![0]);

    client.npc[0] = None;
    super::plant_player(&mut client, "Mime regression", here.x, here.z);
    client.side_icon[INVENTORY_TAB_INDEX as usize] = -1;
    client.chat_modal_id = MIME_REWARD_ROOT;
    super::tick_at(&mut client, &mut guardian_snapshot);
    let status = guardian.tick(
        &mut guardian_driver,
        &guardian_snapshot,
        &guardian_settings,
        2,
        None,
    );
    assert_eq!(
        status.kind, None,
        "the return tile is outside the MIME event"
    );
    assert!(
        !status.hold,
        "Guardian releases the slot on the returned gathering tile"
    );

    // The MIME entry point nulls sidebar tabs. Return teleport reaches this
    // emote mesbox before mapzoneexit can run initalltabs, so inventory is
    // still absent. The R289 chat root/continue component are 356/358.
    snapshot.seed_side_tabs(vec![inventory_tab(-1)], -1);
    snapshot.seed_chat_modal(
        MIME_REWARD_ROOT,
        vec!["You can now use the Climb Rope emote!".into()],
    );
    snapshot.seed_chat_options(vec![], MIME_REWARD_CONTINUE);
    snapshot.seed_inventory(vec![], 0);
    let mut continued = false;
    for _ in 0..4 {
        tick_gatherer(&mut slot, &mut driver, tick, &snapshot, here, None);
        let requests = drain_interactions(&mut slot);
        continued |= requests
            .iter()
            .any(|request| matches!(request, InteractReq::ContinueDialog { .. }));
        tick += 1;
        if continued {
            break;
        }
    }
    assert!(
        continued,
        "Gatherer must continue the MIME reward page before requiring inventory readiness"
    );
    client.chat_modal_id = -1;
    client.side_icon[INVENTORY_TAB_INDEX as usize] = INVENTORY_INTERFACE_ID;
    super::tick_at(&mut client, &mut guardian_snapshot);
    let status = guardian.tick(
        &mut guardian_driver,
        &guardian_snapshot,
        &guardian_settings,
        3,
        None,
    );
    assert!(
        !status.hold,
        "restoring the sidebar does not reacquire the MIME hold"
    );

    // Continuing the mesbox lets the queued mapzoneexit run initalltabs,
    // restoring the inventory sidebar interface before Gatherer reads it.
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_side_tabs(vec![inventory_tab(INVENTORY_INTERFACE_ID)], -1);
    snapshot.seed_inventory(vec![], 28);
    let mut resumed = false;
    for _ in 0..8 {
        tick_gatherer(&mut slot, &mut driver, tick, &snapshot, here, Some(&[]));
        let requests = drain_interactions(&mut slot);
        resumed |= requests.iter().any(is_yew_chop);
        tick += 1;
        if resumed {
            break;
        }
    }
    assert!(
        resumed,
        "Gatherer must resume chopping after the unlock page closes"
    );
    slot.stop();
}

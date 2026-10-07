//! Gatherer lifecycle regressions from the G1 independent review.
use api::interact::Driver;
use api::prot::Out;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{
    ActorView, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocalPlayerView,
    PlayerView, StatView, WorldStateView, WorldTile,
};
use api::ItemDefView;
use script::native::{HostEffect, InteractionReceipt, NativePhase, SettingsBag};
use script::shim::InteractReq;
use script::slot::{StartOutcome, StartPoll};
use script::{CompiledTick, ScriptCtx, SlotScript};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct OutSink;
impl Out for OutSink {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}

#[derive(Default)]
struct Rec {
    out: OutSink,
}
impl Driver for Rec {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
        true
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
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

fn def(id: i32, name: &str) -> ItemDefView {
    ItemDefView {
        id,
        name: Some(name.into()),
        stackable: false,
        members: false,
        base_value: 0,
        noted: false,
        certificate_link: -1,
        certificate_template: -1,
    }
}

fn log(slot: i32) -> ItemView {
    ItemView {
        def: def(1511, "Logs"),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count: 1,
        actions: vec![Some("Drop".into())],
        component_id: -1,
    }
}

fn snapshot(slots: &[i32]) -> GameSnapshot {
    let here = WorldTile {
        x: 3190,
        z: 3245,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    let mut client = client::client::Client::new(client::client::ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: String::new(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = here.x - 52;
    client.map_build_base_z = here.z - 52;
    client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    snapshot.rebuild(&client);
    snapshot.seed_ingame(2);
    snapshot.seed_npcs(Vec::new());
    snapshot.seed_world(WorldStateView {
        map_base_x: here.x - 52,
        map_base_z: here.z - 52,
        level: 0,
        members: true,
        multi_combat: false,
        player_count: 1,
        npc_count: 0,
        cycle: 1,
    });
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            network: here,
            actor: ActorView {
                name: Some("alice".into()),
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
                spot_animation_stamp: -1,
                health: 10,
                total_health: 10,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            combat_level: 3,
            skill_level: 1,
            headicons: 0,
            weapon: None,
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
    snapshot.seed_inventory(slots.iter().map(|&slot| log(slot)).collect(), 28);
    snapshot.seed_equipment(vec![ItemView {
        def: def(1351, "Bronze axe"),
        container: ItemContainer::Equipment,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![Some("Remove".into())],
        component_id: -1,
    }]);
    snapshot.seed_locs(Vec::new());
    snapshot
}

fn fixture_catalog(
    selected: &Arc<api::game_data::SelectedGameData>,
) -> Arc<api::gather_methods::GatherCatalog> {
    api::gather_methods::cached(selected).unwrap_or_else(|| {
        let selected = Arc::clone(selected);
        api::selected::FamilyPreparation::run(move |worker| selected.prepare_gathering(worker))
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
    })
}

fn depleted_snapshot(selected: &Arc<api::game_data::SelectedGameData>) -> GameSnapshot {
    use api::gather_methods::{SceneRegionInput, TargetClass};
    use api::selected::{EntityId, Knowledge};
    use api::snapshot::{LocLayer, LocView};
    let catalog = fixture_catalog(selected);
    let region = SceneRegionInput {
        min_x: 3190 - 12,
        min_z: 3245 - 12,
        max_x: 3190 + 12,
        max_z: 3245 + 12,
        level: 0,
    };
    let mut locs = Vec::new();
    for method in catalog.methods_for_resource("normal") {
        let Knowledge::Known(targets) = &method.targets else {
            continue;
        };
        let Some(depleted) = targets.iter().find_map(|target| {
            (target.class == TargetClass::Depleted && matches!(target.respawn, Knowledge::Known(_)))
                .then_some(target.entity)
                .and_then(|entity| match entity {
                    EntityId::Loc(id) => Some(id),
                    _ => None,
                })
        }) else {
            continue;
        };
        for spot in catalog.spots(method, &region).unwrap() {
            locs.push(LocView {
                typecode: 10,
                info: 0,
                id: depleted,
                name: Some("Tree stump".into()),
                description: None,
                actions: Vec::new(),
                tile: spot.origin,
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
            });
        }
    }
    let mut snapshot = snapshot(&[]);
    snapshot.seed_locs(locs);
    snapshot
}

fn mining_snapshot(
    selected: &Arc<api::game_data::SelectedGameData>,
    method_id: &str,
    depleted: bool,
) -> GameSnapshot {
    use api::gather_methods::{known_rows, SceneRegionInput, TargetClass};
    use api::selected::{EntityId, Truth};
    let catalog = fixture_catalog(selected);
    let method = catalog.method(method_id).unwrap();
    let region = SceneRegionInput {
        min_x: 0,
        min_z: 0,
        max_x: 20000,
        max_z: 20000,
        level: 0,
    };
    let spot = catalog
        .spots(method, &region)
        .unwrap()
        .find(|spot| catalog.access(method, spot).unwrap() == Truth::True)
        .unwrap();
    let mut frame = snapshot(&[]);
    let mut player = frame.local_player().unwrap().clone();
    // The arrival contract needs an observed scene, not only seeded loc rows.
    let mut client = client::client::Client::new(client::client::ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: String::new(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = spot.origin.x - 52;
    client.map_build_base_z = spot.origin.z - 52;
    client.minusedlevel = spot.origin.level;
    client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    frame.rebuild(&client);
    frame.seed_inventory(Vec::new(), 28);
    player.player.actor.tile = WorldTile {
        x: spot.origin.x + 1,
        ..spot.origin
    };
    player.player.network = player.player.actor.tile;
    frame.seed_local_player(player);
    frame.seed_world(WorldStateView {
        map_base_x: spot.origin.x - 52,
        map_base_z: spot.origin.z - 52,
        level: spot.origin.level,
        members: true,
        ..WorldStateView::default()
    });
    frame.seed_stats(vec![StatView {
        index: 14,
        name: "mining".into(),
        effective: 85,
        base: 85,
        xp: 0,
        used: true,
    }]);
    let mut pick = log(0);
    pick.def = def(1275, "Rune pickaxe");
    pick.container = ItemContainer::Equipment;
    frame.seed_equipment(vec![pick]);
    let mut loc = depleted_snapshot(selected).locs()[0].clone();
    let entity = if depleted {
        known_rows(&method.targets)
            .iter()
            .find(|target| target.class == TargetClass::Depleted)
            .expect("mining method has depletion")
            .entity
    } else {
        spot.entity
    };
    let EntityId::Loc(id) = entity else {
        panic!("loc method")
    };
    loc.id = id;
    loc.tile = spot.origin;
    loc.name = Some("Rocks".into());
    loc.actions = vec![Some("Mine".into())];
    frame.seed_locs(vec![loc]);
    frame
}

#[test]
fn resource_wait_deadline_does_not_slide_with_unchanged_observations() {
    let selected = selected();
    let mut slot = started(4260, &selected);
    let frame = depleted_snapshot(&selected);
    tick(&mut slot, &frame, 1);
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    for now in 2..250 {
        tick(&mut slot, &frame, now);
    }
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "resource-unavailable"
    );
    assert!(!slot.has_native_actions());
    slot.stop();
}
#[test]
fn runite_resource_wait_ends_at_eight_minutes_before_watchdog_wedge() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["runite"]));
    let mut slot = started_with(4265, &selected, bag);
    let frame = mining_snapshot(&selected, "mining.runite", true);
    let admission = 10_000;
    for now in admission..admission + 5 {
        tick(&mut slot, &frame, now);
    }

    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    let deadline = wait_until(&slot);
    assert_eq!(deadline, admission + 800);
    assert!(deadline < admission + 1_000);
    slot.pause();
    for now in admission + 5..admission + 50 {
        tick(&mut slot, &frame, now);
        assert!(!slot.has_native_actions());
    }
    slot.resume();
    tick(&mut slot, &frame, admission + 50);
    assert_eq!(
        wait_until(&slot),
        deadline,
        "Resume must not slide the wait"
    );
    tick(&mut slot, &frame, deadline - 1);
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    tick(&mut slot, &frame, deadline);
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "resource-unavailable"
    );
    assert!(!slot.has_native_actions());
    slot.stop();
}

#[test]
fn depleted_auto_groups_share_the_original_gameplay_wait_cap() {
    use api::gather_methods::SceneRegionInput;
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["mithril"]));
    bag.insert("location".into(), serde_json::json!("Auto"));
    let mut slot = started_with(4267, &selected, bag);
    let mut frame = mining_snapshot(&selected, "mining.mithril", true);
    let here = WorldTile {
        x: 3035,
        z: 9771,
        level: 0,
    };
    let mut player = frame.local_player().unwrap().clone();
    player.player.actor.tile = here;
    player.player.network = here;
    frame.seed_local_player(player);
    frame.seed_world(WorldStateView {
        map_base_x: here.x - 52,
        map_base_z: here.z - 52,
        members: true,
        ..WorldStateView::default()
    });
    let catalog = api::gather_methods::cached(&selected).unwrap();
    let method = catalog.method("mining.mithril").unwrap();
    let region = SceneRegionInput {
        min_x: here.x - 45,
        min_z: here.z - 45,
        max_x: here.x + 45,
        max_z: here.z + 45,
        level: 0,
    };
    let template = frame.locs()[0].clone();
    let locs: Vec<_> = catalog
        .spots(method, &region)
        .unwrap()
        .map(|spot| {
            let mut loc = template.clone();
            loc.tile = spot.origin;
            loc
        })
        .collect();
    for (x, z) in [(3036, 9772), (3049, 9736)] {
        assert!(locs
            .iter()
            .any(|loc| { loc.tile.x.abs_diff(x).max(loc.tile.z.abs_diff(z)) <= 12 }));
    }
    frame.seed_locs(locs);

    let admission = 10_000;
    let mut first_exhausted = None;
    let mut first_deadline = None;
    let mut saw_second_wait = false;
    for now in admission..admission + 800 {
        tick(&mut slot, &frame, now);
        assert!(!slot.has_native_actions(), "both groups are loaded");
        if slot.native_status().unwrap().phase == NativePhase::Waiting {
            let first = *first_exhausted.get_or_insert(now);
            let deadline = wait_until(&slot);
            let original = *first_deadline.get_or_insert(deadline);
            if deadline != original {
                saw_second_wait = true;
                assert!(
                    deadline <= first + 800,
                    "widening must not restart the gameplay wait cap"
                );
            }
        }
    }
    assert!(
        saw_second_wait,
        "the runner must exhaust both loaded groups"
    );
    tick(&mut slot, &frame, admission + 800);
    assert_ne!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    let mut left_depleted_groups = false;
    for now in admission + 801..admission + 900 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            let HostEffect::Walk(request) = action.effect else {
                panic!("depleted groups must not emit gathering interactions");
            };
            assert!(
                request
                    .target
                    .x
                    .abs_diff(here.x)
                    .max(request.target.z.abs_diff(here.z))
                    > 45,
                "widening must approach a third group"
            );
            left_depleted_groups = true;
        }
        let status = slot.native_status().unwrap();
        if status.phase == NativePhase::Blocked {
            assert_eq!(
                status.failure.as_ref().unwrap().code.as_ref(),
                "resource-unavailable"
            );
            left_depleted_groups = true;
        }
        if left_depleted_groups {
            break;
        }
        assert_ne!(status.phase, NativePhase::Waiting);
    }
    assert!(
        left_depleted_groups,
        "the capped wait must leave in-place alternation"
    );
    slot.stop();
}

#[test]
fn recreated_gatherer_keeps_resource_wait_fresh_at_large_native_tick() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["runite"]));
    let mut slot = started_with(4266, &selected, bag);
    let frame = mining_snapshot(&selected, "mining.runite", true);
    let admission = 50_000;
    for now in admission..admission + 5 {
        tick(&mut slot, &frame, now);
    }
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    assert!(wait_until(&slot) > admission);

    slot.restart_from_identity(Instant::now()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while slot.state() == script::RunState::Starting {
        assert!(Instant::now() < deadline, "compiled restart stalled");
        slot.observe_lifecycle();
        std::thread::yield_now();
    }
    for now in admission + 5..admission + 10 {
        tick(&mut slot, &frame, now);
    }

    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    assert_eq!(wait_until(&slot), admission + 5 + 800);
    slot.stop();
}

#[test]
fn compiled_hold_at_idle_boundary_defers_dispatch_until_released() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["copper"]));
    let mut slot = started_with(4267, &selected, bag);
    let frame = mining_snapshot(&selected, "mining.copper", false);

    tick_with_hold(&mut slot, &frame, 1, true);
    assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    assert!(
        !slot.has_native_actions(),
        "a held idle boundary must not dispatch an action"
    );

    tick_with_hold(&mut slot, &frame, 2, false);
    assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    let action = slot
        .take_native_action()
        .expect("releasing hold resumes mining without Retry");
    assert!(matches!(
        action.effect,
        HostEffect::Interaction(InteractReq::Loc { .. })
    ));
    slot.stop();
}

fn wait_until(slot: &SlotScript) -> u64 {
    let status = slot.native_status().unwrap();
    let area = status
        .fields
        .iter()
        .find(|field| field.key == "area")
        .and_then(|field| match &field.value {
            script::native::StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
        .expect("Gatherer status includes the selected work area");
    let (_, deadline) = area
        .split_once("wait_until: ")
        .expect("waiting status includes its deadline");
    deadline.parse().expect("wait deadline is a native tick")
}

#[test]
fn pause_resume_before_escape_dispatch_reissues_walk_without_hazard_click() {
    use api::gather_methods::{known_rows, TargetClass};
    use api::selected::EntityId;

    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["copper"]));
    let mut slot = started_with(4268, &selected, bag);
    let mut frame = mining_snapshot(&selected, "mining.copper", false);
    tick(&mut slot, &frame, 1);
    let action = slot.take_native_action().expect("initial mine");
    let authority = action.authority();
    slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 1,
                sequence: 1,
            },
            accepted: true,
            chat_since: 0,
        },
    );

    let catalog = api::gather_methods::cached(&selected).unwrap();
    let method = catalog.method("mining.copper").unwrap();
    let EntityId::Loc(hazard) = known_rows(&method.targets)
        .iter()
        .find(|target| target.class == TargetClass::Hazard)
        .expect("copper has a gas hazard")
        .entity
    else {
        panic!("gas target is a loc")
    };
    let mut gas = frame.locs()[0].clone();
    gas.id = hazard;
    frame.seed_locs(vec![gas]);

    let mut queued_walk = false;
    for now in 2..8 {
        tick(&mut slot, &frame, now);
        if slot.has_native_actions() {
            queued_walk = true;
            break;
        }
    }
    assert!(queued_walk, "hazard observation begins an escape walk");

    slot.pause();
    assert!(
        !slot.has_native_actions(),
        "Pause revokes the undrained escape walk"
    );
    slot.resume();
    let mut resumed_walk = None;
    for now in 10..20 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            match action.effect {
                HostEffect::Walk(request) => resumed_walk = Some(request),
                HostEffect::Interaction(InteractReq::Loc { .. }) => {
                    panic!("resume must not click the hazard")
                }
                _ => panic!("resume must only reissue the escape walk"),
            }
        }
        if resumed_walk.is_some() {
            break;
        }
    }
    let request = resumed_walk.expect("Resume reissues the cancelled escape walk");
    let hazard_tile = frame.locs()[0].tile;
    let player_tile = frame.local_player().unwrap().player.actor.tile;
    let distance = |tile: WorldTile| {
        tile.x
            .abs_diff(hazard_tile.x)
            .max(tile.z.abs_diff(hazard_tile.z))
    };
    assert!(
        distance(request.target) > distance(player_tile),
        "reissued escape walk must move farther from the hazard"
    );
    slot.stop();
}

fn tick_with_hold(slot: &mut SlotScript, snapshot: &GameSnapshot, tick: u64, hold: bool) {
    slot.on_game_tick(&mut ScriptCtx {
        driver: &mut Rec::default(),
        tick,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: CompiledTick {
            hold,
            ..CompiledTick::default()
        },
    });
}

fn tick(slot: &mut SlotScript, snapshot: &GameSnapshot, tick: u64) {
    tick_with_hold(slot, snapshot, tick, false);
}

#[test]
fn incidental_gems_are_dropped_and_held_tool_is_preserved() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["copper"]));
    let mut slot = started_with(4261, &selected, bag);
    let catalog = api::gather_methods::cached(&selected).unwrap();
    let gems = catalog.incidental_gem_ids();
    assert_eq!(gems.len(), 4);
    let mut frame = mining_snapshot(&selected, "mining.copper", false);
    let mut rows: Vec<_> = (0..27)
        .map(|index| {
            let mut row = log(index);
            row.def = def(gems[index as usize % gems.len()], "Uncut gem");
            row
        })
        .collect();
    let mut pick = log(27);
    pick.def = def(1265, "Bronze pickaxe");
    rows.push(pick);
    frame.seed_inventory(rows.clone(), 28);
    let mut dropped = Vec::new();
    for now in 1..30 {
        tick(&mut slot, &frame, now);
        let sent = drain(&mut slot, now);
        assert!(sent.iter().all(|slot| *slot != 27));
        rows.retain(|row| !sent.contains(&row.slot));
        dropped.extend(sent);
        frame.seed_inventory(rows.clone(), 28);
        if dropped.len() == 27 {
            break;
        }
    }
    assert_eq!(dropped.len(), 27);
    assert_eq!(
        rows.iter().map(|row| row.def.id).collect::<Vec<_>>(),
        [1265]
    );
    slot.stop();
}

#[test]
fn gas_replacement_cancels_the_queued_mine_with_a_walk_when_no_other_target_is_live() {
    use api::gather_methods::{known_rows, TargetClass};
    use api::selected::EntityId;
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["copper"]));
    let mut slot = started_with(4262, &selected, bag);
    let mut frame = mining_snapshot(&selected, "mining.copper", false);
    tick(&mut slot, &frame, 1);
    let action = slot.take_native_action().expect("initial mine");
    assert!(matches!(
        action.effect,
        HostEffect::Interaction(InteractReq::Loc { .. })
    ));
    let authority = action.authority();
    slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 1,
                sequence: 1,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    let catalog = api::gather_methods::cached(&selected).unwrap();
    let method = catalog.method("mining.copper").unwrap();
    let EntityId::Loc(hazard) = known_rows(&method.targets)
        .iter()
        .find(|target| target.class == TargetClass::Hazard)
        .unwrap()
        .entity
    else {
        panic!("gas loc")
    };
    let mut gas = frame.locs()[0].clone();
    gas.id = hazard;
    frame.seed_locs(vec![gas]);
    let mut escaped = false;
    for now in 2..6 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            match action.effect {
                HostEffect::Walk(request) => {
                    assert_ne!(
                        request.target,
                        frame.local_player().unwrap().player.actor.tile
                    );
                    escaped = true;
                }
                _ => panic!("hazard must never be clicked"),
            }
        }
        if escaped {
            break;
        }
    }
    assert!(
        escaped,
        "a walk must cancel the server's queued mining operation"
    );
    slot.stop();
}

fn observed_npc(kind: usize, tile: WorldTile) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index: 42,
        r#type: Some(kind),
        name: None,
        actions: vec![Some("Net".into())],
        tile,
        distance: 2,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 0,
        total_health: 0,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn assert_npc_hazard_escape(slot: &mut SlotScript, frame: &GameSnapshot) {
    let mut walked = false;
    for now in 6..12 {
        tick(slot, frame, now);
        while let Some(action) = slot.take_native_action() {
            let HostEffect::Walk(request) = action.effect else {
                panic!("hazard replacement must cancel gathering, never click");
            };
            let hazard = frame.npcs()[0].tile;
            let distance =
                |tile: WorldTile| tile.x.abs_diff(hazard.x).max(tile.z.abs_diff(hazard.z));
            assert!(
                distance(request.target)
                    > distance(frame.local_player().unwrap().player.actor.tile),
                "the escape must increase distance, not merely move sideways"
            );
            walked = true;
        }
        if walked {
            break;
        }
    }
    assert!(walked, "hazard replacement must produce an escape walk");
}

#[test]
fn observed_ent_replacing_the_only_live_tree_cancels_chopping_with_a_walk() {
    let selected = selected();
    let mut slot = started(4264, &selected);
    let mut frame = depleted_snapshot(&selected);
    let mut locs = frame.locs().to_vec();
    locs[0].id = 1276;
    let tile = locs[0].tile;
    frame.seed_locs(locs);
    let mut player = frame.local_player().unwrap().clone();
    player.player.actor.tile = tile;
    player.player.network = tile;
    frame.seed_local_player(player);
    tick(&mut slot, &frame, 1);
    let action = slot.take_native_action().expect("initial chop");
    assert!(matches!(
        action.effect,
        HostEffect::Interaction(InteractReq::Loc { .. })
    ));
    let authority = action.authority();
    slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 1,
                sequence: 1,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    let mut locs = frame.locs().to_vec();
    locs.remove(0);
    frame.seed_locs(locs);
    frame.seed_npcs(vec![observed_npc(444, tile)]);
    assert_npc_hazard_escape(&mut slot, &frame);
    slot.stop();
}

#[test]
fn observed_fishing_spot_uses_actor_approach_instead_of_routing_to_water() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Fishing"));
    bag.insert(
        "fishingMethod".into(),
        serde_json::json!("fishing.saltfish.op1"),
    );
    let mut slot = started_with(4263, &selected, bag);
    let tile = WorldTile {
        x: 3267,
        z: 3147,
        level: 0,
    };
    let mut frame = snapshot(&[]);
    let mut player = frame.local_player().unwrap().clone();
    player.player.actor.tile = WorldTile { z: 3149, ..tile };
    player.player.network = player.player.actor.tile;
    frame.seed_local_player(player);
    frame.seed_world(WorldStateView {
        map_base_x: tile.x - 52,
        map_base_z: tile.z - 52,
        members: true,
        ..WorldStateView::default()
    });
    frame.seed_stats(vec![StatView {
        index: 10,
        name: "fishing".into(),
        effective: 1,
        base: 1,
        xp: 0,
        used: true,
    }]);
    let mut net = log(0);
    net.def = def(303, "Small fishing net");
    frame.seed_inventory(vec![net], 28);
    frame.seed_equipment(Vec::new());
    frame.seed_npcs(vec![observed_npc(330, tile)]);
    let mut observed_op = false;
    for now in 1..6 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            assert!(
                matches!(
                    action.effect,
                    HostEffect::Interaction(InteractReq::Npc {
                        index: Some(42),
                        ..
                    })
                ),
                "observed fishing targets must not route onto their water tile"
            );
            observed_op = true;
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: now,
                        sequence: 1,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
        }
        if observed_op {
            break;
        }
    }
    assert!(
        observed_op,
        "the nonadjacent observed spot must be approached by its NPC op"
    );
    frame.seed_npcs(vec![observed_npc(404, tile)]);
    assert_npc_hazard_escape(&mut slot, &frame);
    slot.stop();
}

/// Drain the tick's outbox like the host: every drop is written and accepted.
fn drain(slot: &mut SlotScript, tick: u64) -> Vec<i32> {
    let mut sent = Vec::new();
    while let Some(action) = slot.take_native_action() {
        if let HostEffect::Interaction(InteractReq::Held {
            slot: Some(index), ..
        }) = &action.effect
        {
            sent.push(*index);
        }
        let authority = action.authority();
        slot.complete_native_interaction(
            &authority,
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
    }
    sent
}

fn started(incarnation: u64, selected: &Arc<api::game_data::SelectedGameData>) -> SlotScript {
    started_with(incarnation, selected, SettingsBag::new())
}

fn started_with(
    incarnation: u64,
    selected: &Arc<api::game_data::SelectedGameData>,
    bag: SettingsBag,
) -> SlotScript {
    started_with_banks(incarnation, selected, bag, Arc::default())
}

fn started_with_banks(
    incarnation: u64,
    selected: &Arc<api::game_data::SelectedGameData>,
    mut bag: SettingsBag,
    banks: Arc<api::named_banks::NamedBankFacts>,
) -> SlotScript {
    bag.entry("disposition")
        .or_insert_with(|| serde_json::json!("Power"));
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_compiled(
        "alice",
        script::CompiledId("Gatherer"),
        Arc::new(bag),
        Arc::clone(selected),
        banks,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(outcome) => {
                assert_eq!(outcome, StartOutcome::Ready);
                return slot;
            }
            StartPoll::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            StartPoll::NotOwed => panic!("no start owed"),
        }
    }
}

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()
}

#[test]
fn gather_progress_must_not_hide_a_later_idle_stall() {
    use api::gather_methods::{known_rows, TargetClass};
    use api::selected::EntityId;

    let selected = selected();
    let mut slot = started(4290, &selected);
    let mut frame = depleted_snapshot(&selected);
    let catalog = api::gather_methods::cached(&selected).unwrap();
    let method = catalog.methods_for_resource("normal").next().unwrap();
    let EntityId::Loc(id) = known_rows(&method.targets)
        .iter()
        .find(|row| row.class == TargetClass::Resource)
        .unwrap()
        .entity
    else {
        panic!("normal woodcutting has loc resources");
    };
    let mut target = frame.locs()[0].clone();
    target.id = id;
    target.name = Some("Tree".into());
    target.actions = vec![Some("Chop down".into())];
    let mut player = frame.local_player().unwrap().clone();
    player.player.actor.tile = WorldTile {
        x: target.tile.x + 1,
        ..target.tile
    };
    player.player.network = player.player.actor.tile;
    player.player.actor.animation = -1;
    frame.seed_local_player(player);
    frame.seed_locs(vec![target]);

    let mut chops = Vec::new();
    let mut gain_seeded = false;
    for now in 1..80 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            if matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::Loc { action, .. })
                    if action == "Chop down"
            ) {
                chops.push(now);
            }
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: now,
                        sequence: now,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
        }
        if !chops.is_empty() && !gain_seeded {
            frame.seed_inventory(vec![log(0)], 28);
            let mut stats = frame.stats().to_vec();
            stats.iter_mut().find(|row| row.index == 8).unwrap().xp = 25;
            frame.seed_stats(stats);
            gain_seeded = true;
        }
    }
    let status = slot.native_status().unwrap();
    for (key, expected) in [("yielded", 1), ("xp", 25)] {
        assert_eq!(
            status
                .fields
                .iter()
                .find(|field| field.key == key)
                .unwrap()
                .value,
            script::native::StatusValue::Integer(expected),
            "the first yield remains counted after the idle attempt ends"
        );
    }
    slot.stop();
    assert!(
        gain_seeded,
        "must reach the real GatherRun and observe its first product/XP gain"
    );
    assert!(
        chops.len() >= 2,
        "after an observed gain, an unchanged idle target must be retried instead of waiting forever; chops={chops:?}"
    );
}

/// A level-up's unlock page is a new modal, not a failed close of the first page.
#[test]
fn chained_level_up_pages_receive_separate_continues() {
    let mut slot = started(4250, &selected());
    let mut snapshot = snapshot(&[]);
    snapshot.seed_chat_modal(100, vec!["Your Mining level is now 31.".into()]);
    snapshot.seed_chat_options(Vec::new(), 101);
    let mut continues = 0;
    for t in 1..25 {
        tick(&mut slot, &snapshot, t);
        while let Some(action) = slot.take_native_action() {
            assert!(matches!(
                action.effect,
                HostEffect::Interaction(InteractReq::ContinueDialog { component_id: None })
            ));
            continues += 1;
            let authority = action.authority();
            slot.complete_native_interaction(
                &authority,
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: t,
                        sequence: t,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            if continues == 1 {
                snapshot
                    .seed_chat_modal(200, vec!["You can now mine with Adamant Pickaxes.".into()]);
                snapshot.seed_chat_options(Vec::new(), 201);
            } else {
                snapshot.seed_chat_modal(-1, Vec::new());
                snapshot.seed_chat_options(Vec::new(), -1);
            }
        }
        if continues == 2 {
            tick(&mut slot, &snapshot, t + 1);
            break;
        }
    }
    assert_eq!(
        continues, 2,
        "each distinct page needs its own acknowledged continue"
    );
    assert!(slot.native_status().unwrap().failure.is_none());
    slot.stop();
}

#[test]
fn unchanged_dialog_page_still_fails_after_eight_ticks() {
    let mut slot = started(4251, &selected());
    let mut snapshot = snapshot(&[]);
    snapshot.seed_chat_modal(100, vec!["Your Mining level is now 31.".into()]);
    snapshot.seed_chat_options(Vec::new(), 101);
    for t in 1..20 {
        tick(&mut slot, &snapshot, t);
        drain(&mut slot, t);
        if slot.native_status().unwrap().phase == NativePhase::Blocked {
            break;
        }
    }
    assert_eq!(
        slot.native_status()
            .unwrap()
            .failure
            .as_ref()
            .unwrap()
            .code
            .as_ref(),
        "oneop-failed"
    );
    slot.stop();
}

/// An unsettled drop remains retryable after newer receipts evict its receipt.
#[test]
fn unsettled_drop_is_replanned_or_blocked_after_receipt_eviction() {
    let mut slot = started(4242, &selected());
    // Slot 0 is the drop the engine refused: it never empties.
    let mut present: Vec<i32> = (0..28).collect();
    let mut t = 1;
    let mut first = Vec::new();
    while first.is_empty() && t < 20 {
        tick(&mut slot, &snapshot(&present), t);
        first = drain(&mut slot, t);
        t += 1;
    }
    assert_eq!(first, [0, 1, 2, 3, 4], "first batch");
    let mut resent_zero = false;
    for step in 0..60 {
        // Every other sent slot lands on the next tick; slot 0 never does.
        present.retain(|slot| *slot == 0 || !first.contains(slot));
        tick(&mut slot, &snapshot(&present), t);
        let sent = drain(&mut slot, t);
        if !sent.is_empty() || step % 10 == 0 {
            let status = slot.native_status().unwrap();
            eprintln!(
                "tick {t} present={} sent {sent:?} phase={:?} failure={:?}",
                present.len(),
                status.phase,
                status.failure
            );
        }
        resent_zero |= sent.contains(&0);
        if slot.native_status().unwrap().phase == NativePhase::Blocked {
            break;
        }
        first = sent;
        t += 1;
    }
    let status = slot.native_status().unwrap();
    assert!(
        resent_zero || status.phase == NativePhase::Blocked,
        "slot 0 was never re-planned and the batch never blocked: phase={:?}",
        status.phase
    );
    slot.stop();
}

/// F2: a full pack holding nothing droppable begins and clears a DropBatch every tick.
#[test]
fn full_pack_without_products_blocks_instead_of_looping() {
    let mut slot = started(4243, &selected());
    let mut snapshot = snapshot(&[]);
    let bones = (0..28)
        .map(|slot| ItemView {
            def: def(526, "Bones"),
            ..log(slot)
        })
        .collect();
    snapshot.seed_inventory(bones, 28);
    let mut sent = 0;
    for t in 1..200 {
        tick(&mut slot, &snapshot, t);
        sent += drain(&mut slot, t).len();
        if slot.native_status().unwrap().phase == NativePhase::Blocked {
            break;
        }
    }
    let status = slot.native_status().unwrap();
    eprintln!(
        "after 200 ticks: phase={:?} failure={:?} sent={sent} fields={:?}",
        status.phase, status.failure, status.fields
    );
    assert_eq!(
        status.phase,
        NativePhase::Blocked,
        "a full pack with nothing droppable must block, not loop"
    );
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "inventory-blocked"
    );
    assert_eq!(sent, 0, "non-products must never be dropped");
    slot.stop();
}

#[test]
fn unchanged_resource_wait_does_not_allocate() {
    let selected = selected();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["runite"]));
    let mut slot = started_with(4244, &selected, bag);
    let snapshot = mining_snapshot(&selected, "mining.runite", true);
    for t in 1..30 {
        tick(&mut slot, &snapshot, t);
        drain(&mut slot, t);
    }
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    let info = allocation_counter::measure(|| {
        // Repeated polls of the same observation stay within the eight-minute
        // wait. Expiry is a state transition, not an unchanged waiting poll.
        for poll in 0..1000 {
            tick(&mut slot, &snapshot, 30 + poll / 2);
        }
    });
    eprintln!(
        "resource wait: 1000 unchanged polls allocations={} bytes={}",
        info.count_total, info.bytes_total
    );
    assert_eq!(
        info.count_total, 0,
        "unchanged resource wait must not allocate"
    );
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    slot.stop();
}

/// F3: a RestartRequired edit is applied in place at the next idle boundary.
#[test]
fn restart_required_edit_is_not_applied_in_place() {
    let selected = selected();
    let mut slot = started(4245, &selected);
    let snapshot = depleted_snapshot(&selected);
    tick(&mut slot, &snapshot, 1);
    let run = slot.native_run().unwrap();
    let mut bag = SettingsBag::new();
    bag.insert("skill".into(), serde_json::json!("Mining"));
    bag.insert("miningResources".into(), serde_json::json!(["copper"]));
    let prepared_selected = Arc::clone(&selected);
    let config = api::selected::FamilyPreparation::run(move |families| {
        script::slot::prepare_config(
            families,
            script::CompiledId("Gatherer"),
            2,
            Arc::new(bag),
            prepared_selected,
            Arc::default(),
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    eprintln!("delivery={:?}", slot.configure_compiled(config, run));
    for t in 2..6 {
        tick(&mut slot, &snapshot, t);
        drain(&mut slot, t);
    }
    let status = slot.native_status().unwrap();
    let skill = status.fields.iter().find(|field| field.key == "skill");
    eprintln!(
        "active={} pending={:?} skill={skill:?}",
        status.active_settings, status.pending_settings
    );
    assert_eq!(status.active_settings, 1);
    assert_eq!(status.pending_settings, Some(2));
    assert!(
        matches!(
            skill.map(|field| &field.value),
            Some(script::native::StatusValue::Text(value)) if value.as_ref() == "Woodcutting"
        ),
        "the old skill must stay active until the slot restarts"
    );
    slot.stop();
}

#[test]
fn partial_drop_progress_survives_pause_and_stop() {
    for pause in [true, false] {
        let mut slot = started(4250 + u64::from(pause), &selected());
        let present: Vec<i32> = (0..28).collect();
        let mut sent = Vec::new();
        let mut t = 1;
        while sent.is_empty() && t < 20 {
            tick(&mut slot, &snapshot(&present), t);
            sent = drain(&mut slot, t);
            t += 1;
        }
        assert_eq!(sent, [0, 1, 2, 3, 4]);
        let remaining: Vec<_> = present.into_iter().filter(|s| !sent.contains(s)).collect();
        tick(&mut slot, &snapshot(&remaining), t);
        let status = slot.native_status().unwrap();
        let dropped = status
            .fields
            .iter()
            .find(|field| field.key == "dropped")
            .unwrap();
        assert_eq!(dropped.value, script::native::StatusValue::Integer(5));
        if pause {
            slot.pause();
            let status = slot.native_status().unwrap();
            let dropped = status
                .fields
                .iter()
                .find(|field| field.key == "dropped")
                .unwrap();
            assert_eq!(dropped.value, script::native::StatusValue::Integer(5));
        } else {
            slot.stop();
        }
    }
}

#[test]
fn bronze_tool_remains_carried_below_catalog_wield_level() {
    let selected = selected();
    let mut slot = started(4252, &selected);
    let mut snapshot = depleted_snapshot(&selected);
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1351, "Bronze axe"),
            actions: vec![Some("Wield".into()), Some("Drop".into())],
            ..log(0)
        }],
        28,
    );
    for t in 1..5 {
        tick(&mut slot, &snapshot, t);
        while let Some(action) = slot.take_native_action() {
            assert!(
                !matches!(
                    action.effect,
                    HostEffect::Interaction(InteractReq::Held { .. })
                ),
                "Attack 0 must not attempt to wield the Attack-1 bronze axe"
            );
        }
    }
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    slot.stop();
}

#[test]
fn late_drop_settlement_counts_while_modal_budget_defers_resend() {
    let mut slot = started(4253, &selected());
    let all: Vec<i32> = (0..28).collect();
    let mut snapshot = snapshot(&all);
    let mut sent = Vec::new();
    let mut t = 1;
    while sent.is_empty() && t < 20 {
        tick(&mut slot, &snapshot, t);
        sent = drain(&mut slot, t);
        t += 1;
    }
    assert_eq!(sent, [0, 1, 2, 3, 4]);
    tick(&mut slot, &snapshot, t);
    assert!(drain(&mut slot, t).is_empty());
    snapshot.seed_chat_modal(123, vec!["Your inventory is too full.".into()]);
    tick(&mut slot, &snapshot, t + 1);
    assert_eq!(drain(&mut slot, t + 1), [0, 1, 2, 3]);
    snapshot.seed_inventory(all.into_iter().filter(|s| *s != 4).map(log).collect(), 28);
    tick(&mut slot, &snapshot, t + 2);
    let status = slot.native_status().unwrap();
    let dropped = status
        .fields
        .iter()
        .find(|field| field.key == "dropped")
        .unwrap();
    assert_eq!(dropped.value, script::native::StatusValue::Integer(1));
    slot.stop();
}

#[test]
fn restart_required_edit_supersedes_an_older_boundary_edit() {
    let selected = selected();
    let mut slot = started(4254, &selected);
    let snapshot = depleted_snapshot(&selected);
    tick(&mut slot, &snapshot, 1);
    let original_area = slot
        .native_status()
        .unwrap()
        .fields
        .iter()
        .find(|field| field.key == "area")
        .unwrap()
        .value
        .clone();
    let run = slot.native_run().unwrap();
    for (revision, bag) in [
        (
            2,
            [("radius".into(), serde_json::json!(6))]
                .into_iter()
                .collect(),
        ),
        (
            3,
            [("skill".into(), serde_json::json!("Mining"))]
                .into_iter()
                .collect(),
        ),
    ] {
        let selected = Arc::clone(&selected);
        let config = api::selected::FamilyPreparation::run(move |families| {
            script::slot::prepare_config(
                families,
                script::CompiledId("Gatherer"),
                revision,
                Arc::new(bag),
                selected,
                Arc::default(),
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        slot.configure_compiled(config, run);
    }
    for t in 2..6 {
        tick(&mut slot, &snapshot, t);
        drain(&mut slot, t);
    }
    let status = slot.native_status().unwrap();
    assert_eq!(
        status.active_settings, 1,
        "superseded boundary edit must not activate"
    );
    assert_eq!(status.pending_settings, Some(3));
    let area = status
        .fields
        .iter()
        .find(|field| field.key == "area")
        .unwrap();
    assert_eq!(
        area.value, original_area,
        "the active work area must remain unchanged"
    );
    slot.stop();
}

#[test]
fn a_missing_tool_enters_bank_selection_instead_of_blocking_before_stock_is_read() {
    let selected = selected();
    let mut slot = started(4300, &selected);
    let mut frame = depleted_snapshot(&selected);
    frame.seed_inventory(Vec::new(), 28);
    frame.seed_equipment(Vec::new());
    for now in 1..20 {
        tick(&mut slot, &frame, now);
        while let Some(action) = slot.take_native_action() {
            assert!(!matches!(
                action.effect,
                HostEffect::Interaction(InteractReq::Loc { .. })
            ));
        }
        if slot
            .native_status()
            .is_some_and(|status| status.phase == NativePhase::Blocked)
        {
            break;
        }
    }
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "bank-unavailable"
    );
    slot.stop();
}

#[test]
fn power_to_bank_edit_waits_for_the_live_drop_batch_and_the_next_full_pack() {
    let selected = selected();
    let mut slot = started(4301, &selected);
    let mut present: Vec<i32> = (0..28).collect();
    let mut now = 1;
    let sent = loop {
        tick(&mut slot, &snapshot(&present), now);
        let sent = drain(&mut slot, now);
        if !sent.is_empty() {
            break sent;
        }
        now += 1;
        assert!(now < 20);
    };
    let preparation_selected = Arc::clone(&selected);
    let next = api::selected::FamilyPreparation::run(move |families| {
        script::slot::prepare_config(
            families,
            script::CompiledId("Gatherer"),
            2,
            Arc::new(
                [("disposition".into(), serde_json::json!("Bank"))]
                    .into_iter()
                    .collect(),
            ),
            preparation_selected,
            Arc::default(),
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    slot.configure_compiled(next, slot.native_run().unwrap());
    assert_eq!(slot.native_status().unwrap().active_settings, 1);
    present.retain(|row| !sent.contains(row));
    while !present.is_empty() {
        now += 1;
        tick(&mut slot, &snapshot(&present), now);
        let status = slot.native_status().unwrap();
        assert_eq!(
            status.active_settings, 1,
            "a boundary edit must not change a live batch"
        );
        let sent = drain(&mut slot, now);
        present.retain(|row| !sent.contains(row));
        assert!(now < 80);
    }
    for _ in 0..5 {
        now += 1;
        tick(&mut slot, &depleted_snapshot(&selected), now);
        while let Some(action) = slot.take_native_action() {
            assert!(
                !matches!(action.effect, HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drop")
            );
        }
    }
    // A newly full pack selects banking; no drop from the pending disposition
    // can escape while the previous batch is still settling.
    for _ in 0..20 {
        now += 1;
        tick(&mut slot, &snapshot(&(0..28).collect::<Vec<_>>()), now);
        while let Some(action) = slot.take_native_action() {
            assert!(
                !matches!(action.effect, HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drop")
            );
        }
        if slot
            .native_status()
            .is_some_and(|status| status.phase == NativePhase::Blocked)
        {
            break;
        }
    }
    let status = slot.native_status().unwrap();
    assert_eq!(status.active_settings, 2);
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "bank-unavailable"
    );
    slot.stop();
}

fn next_trip_effect(
    slot: &mut SlotScript,
    frame: &GameSnapshot,
    now: &mut u64,
) -> (script::native::HostAuthority, u64, HostEffect) {
    for _ in 0..40 {
        *now += 1;
        tick(slot, frame, *now);
        if let Some(action) = slot.take_native_action() {
            return (action.authority(), action.request_id.get(), action.effect);
        }
        assert_ne!(
            slot.native_status().unwrap().phase,
            NativePhase::Blocked,
            "{:?}",
            slot.native_status()
        );
    }
    panic!("trip did not advance: {:?}", slot.native_status());
}

fn accept_trip_operation(
    slot: &mut SlotScript,
    authority: &script::native::HostAuthority,
    id: u64,
    now: u64,
) {
    slot.complete_native_interaction(
        authority,
        InteractionReceipt {
            request_id: id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            accepted: true,
            chat_since: 0,
        },
    );
}

fn trip_position(frame: &mut GameSnapshot, here: WorldTile) {
    let mut player = frame.local_player().unwrap().clone();
    player.player.actor.tile = here;
    player.player.network = here;
    frame.seed_local_player(player);
    let mut world = *frame.world();
    world.level = here.level;
    world.map_base_x = here.x - 52;
    world.map_base_z = here.z - 52;
    frame.seed_world(world);
}

fn assert_resource_return(
    selected: &Arc<api::game_data::SelectedGameData>,
    request: &script::native::WalkRequest,
    anchor: WorldTile,
) {
    assert_eq!(request.arrival, nav::arrival::ArrivalKind::Area);
    assert_eq!(request.radius, 1);
    assert_eq!(request.target.level, anchor.level);
    let region = api::gather_methods::SceneRegionInput {
        min_x: anchor.x - 12,
        min_z: anchor.z - 12,
        max_x: anchor.x + 12,
        max_z: anchor.z + 12,
        level: anchor.level,
    };
    let catalog = fixture_catalog(selected);
    assert!(
        catalog.methods_for_resource("normal").any(|method| catalog
            .spots(method, &region)
            .unwrap()
            .any(|spot| spot.origin == request.target)),
        "Return must target a resource in the retained work area: {:?}",
        request.target
    );
}

#[derive(Clone, Copy)]
enum RuneStock {
    Full,
    Partial,
    Empty,
}

fn run_supply_trip_scenario(incarnation: u64, rune_stock: RuneStock) {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use script::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
    use script::native::{WalkEnd, WalkReceipt};

    const RUNE_COSTS: [(i32, i32); 3] = [(554, 1), (556, 3), (563, 1)];
    const PARTIAL_BANK_RUNES: [(i32, i32); 3] = [(554, 3), (556, 9), (563, 3)];

    let selected = selected();
    let mut frame = snapshot(&[]);
    let anchor = frame.local_player().unwrap().player.actor.tile;
    let bank_tile = WorldTile {
        x: anchor.x + 20,
        z: anchor.z,
        level: 1,
    };
    let spell = selected
        .teleports()
        .iter()
        .find(|spell| {
            spell.available()
                && spell.runes.len() == RUNE_COSTS.len()
                && RUNE_COSTS.iter().all(|(id, count)| {
                    spell
                        .runes
                        .iter()
                        .any(|rune| rune.id == *id && rune.count == *count)
                })
                && spell
                    .runes
                    .iter()
                    .all(|rune| rune.id >= 0 && rune.count > 0 && !rune.name.is_empty())
        })
        .unwrap()
        .clone();
    let bank = NamedBank::new("fixture-bank", bank_tile);
    let facts = Arc::new(NamedBankFacts::from_banks(vec![bank]));
    let mut settings = SettingsBag::new();
    settings.insert("disposition".into(), serde_json::json!("Bank"));
    settings.insert("reserveTeleport".into(), serde_json::json!(spell.name));
    settings.insert("reserveCasts".into(), serde_json::json!(5));
    let mut slot = started_with_banks(incarnation, &selected, settings.clone(), Arc::clone(&facts));
    frame = depleted_snapshot(&selected);
    let work_locs = frame.locs().to_vec();
    let mut held: Vec<_> = spell
        .runes
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, rune)| ItemView {
            def: def(rune.id, &rune.name),
            count: rune.count,
            ..log(index as i32 + 1)
        })
        .collect();
    frame.seed_inventory(held.clone(), 28);
    frame.seed_equipment(Vec::new());
    let mut stats = frame.stats().to_vec();
    stats.push(StatView {
        index: 0,
        name: "attack".into(),
        effective: 1,
        base: 1,
        xp: 0,
        used: true,
    });
    frame.seed_stats(stats);
    let axe = ItemView {
        def: def(1351, "Bronze axe"),
        actions: vec![Some("Wield".into()), Some("Drop".into())],
        ..log(0)
    };
    let mut stock = axe.clone();
    stock.container = ItemContainer::Bank;
    stock.actions = vec![
        Some("Withdraw-1".into()),
        Some("Withdraw-5".into()),
        Some("Withdraw-10".into()),
        None,
        Some("Withdraw-X".into()),
    ];
    let mut bank_stock = vec![stock];
    for (index, rune) in spell.runes.iter().enumerate() {
        let count = match rune_stock {
            RuneStock::Full => 10_000,
            RuneStock::Partial => PARTIAL_BANK_RUNES
                .iter()
                .find(|(id, _)| *id == rune.id)
                .map(|(_, count)| *count)
                .expect("the partial rune stock names every reserve rune"),
            RuneStock::Empty => 0,
        };
        if count == 0 {
            continue;
        }
        bank_stock.push(ItemView {
            def: def(rune.id, &rune.name),
            count,
            container: ItemContainer::Bank,
            actions: vec![
                Some("Withdraw-1".into()),
                Some("Withdraw-5".into()),
                Some("Withdraw-10".into()),
                None,
                Some("Withdraw-X".into()),
            ],
            ..log(index as i32 + 1)
        });
    }
    let mut now = 0;
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(matches!(effect, HostEffect::BankPick(_)));
    slot.complete_native_bank_pick(
        &authority,
        BankPickReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            selected: SelectedBank {
                bank_index: 0,
                access_tile: bank_tile,
                kind: PickKind::Reachable,
                access: Some(Arc::new(BankStandAccess {
                    bank,
                    stand_tile: bank_tile,
                    kind: AccessKind::Booth,
                    stand_op: 2,
                    name: Some(Arc::from("Bank booth")),
                    choose: None,
                })),
            },
        },
    );
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("selection must approach bank")
    };
    assert_eq!(request.target, bank_tile);
    trip_position(&mut frame, bank_tile);
    let mut booth = work_locs[0].clone();
    booth.id = 2213;
    booth.tile = bank_tile;
    booth.name = Some("Bank booth".into());
    booth.actions = vec![None, Some("Use-quickly".into())];
    frame.seed_locs(vec![booth]);
    slot.complete_native_walk(
        &authority,
        WalkReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(matches!(
        effect,
        HostEffect::Interaction(InteractReq::OpenStand { .. })
    ));
    accept_trip_operation(&mut slot, &authority, request_id, now);
    frame.seed_bank_observation(1, 1, None, Vec::new());
    for _ in 0..3 {
        now += 1;
        tick(&mut slot, &frame, now);
        assert!(!slot.has_native_actions(), "unread bank is not empty stock");
        assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    }
    frame.seed_bank_observation(1, 1, Some(bank_stock.clone()), held.clone());
    // The trip's own open started this session in Item mode (`bank.rs2:18-19`
    // resets `%bankcert`), so the first withdraw needs no Item press
    // (TICK-FIX #11).
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(matches!(
        effect,
        HostEffect::Interaction(InteractReq::WithdrawX {
            bank_item_id: 1351,
            lands_as_id: 1351,
            count: 1,
            ..
        })
    ));
    accept_trip_operation(&mut slot, &authority, request_id, now);
    now += 1;
    tick(&mut slot, &frame, now);
    assert!(
        !slot.has_native_actions(),
        "accepted withdrawal is not an observed tool"
    );
    held.push(axe.clone());
    bank_stock.remove(0);
    frame.seed_inventory(held.clone(), 28);
    frame.seed_bank_observation(1, 1, Some(bank_stock.clone()), held.clone());
    let first = &spell.runes[0];
    if matches!(rune_stock, RuneStock::Empty) {
        for _ in 0..8 {
            now += 1;
            tick(&mut slot, &frame, now);
            assert!(
                slot.take_native_action().is_none(),
                "zero-rune stock must block after the tool withdrawal"
            );
            if slot
                .native_status()
                .is_some_and(|status| status.phase == NativePhase::Blocked)
            {
                break;
            }
        }
        let status = slot.native_status().unwrap();
        assert_eq!(status.phase, NativePhase::Blocked);
        let failure = status.failure.as_ref().unwrap();
        assert_eq!(failure.code.as_ref(), "supply-missing");
        assert!(failure.message.contains(first.name.as_str()));
        slot.stop();
        return;
    }
    let first_withdrawal = match rune_stock {
        RuneStock::Full => first.count * 5,
        RuneStock::Partial => PARTIAL_BANK_RUNES
            .iter()
            .find(|(id, _)| *id == first.id)
            .map(|(_, count)| *count)
            .unwrap(),
        RuneStock::Empty => unreachable!(),
    };
    let (old_authority, old_request, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(
        matches!(effect, HostEffect::Interaction(InteractReq::WithdrawX {
        bank_item_id, lands_as_id, count, ..
    }) if bank_item_id == first.id && lands_as_id == first.id && count == first_withdrawal)
    );
    accept_trip_operation(&mut slot, &old_authority, old_request, now);
    let first_bank_row = bank_stock
        .iter_mut()
        .find(|item| item.def.id == first.id)
        .unwrap();
    first_bank_row.count -= first_withdrawal;
    held.push(ItemView {
        def: def(first.id, &first.name),
        count: first_withdrawal,
        ..log(1)
    });
    frame.seed_inventory(held.clone(), 28);
    frame.seed_bank_observation(1, 1, Some(bank_stock.clone()), held.clone());
    slot.pause();
    assert!(!old_authority.live());
    let pending = api::selected::FamilyPreparation::run({
        let selected = Arc::clone(&selected);
        move |families| {
            settings.insert("reserveCasts".into(), serde_json::json!(1));
            script::slot::prepare_config(
                families,
                script::CompiledId("Gatherer"),
                2,
                Arc::new(settings),
                selected,
                facts,
            )
        }
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    slot.configure_compiled(pending, slot.native_run().unwrap());
    slot.resume();
    // The resumed trip is still in the session its own open started, which
    // nothing switched to Note, so it presses no Item (TICK-FIX #11).
    // The latched stock-limited plan must finish after this one-cast edit.
    for rune in spell.runes.iter().skip(1) {
        let expected_withdrawal = rune.count
            * match rune_stock {
                RuneStock::Full => 4,
                RuneStock::Partial => 3,
                RuneStock::Empty => unreachable!(),
            };
        let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
        assert!(
            matches!(effect, HostEffect::Interaction(InteractReq::WithdrawX {
            bank_item_id, lands_as_id, count, ..
        }) if bank_item_id == rune.id && lands_as_id == rune.id && count == expected_withdrawal)
        );
        assert_eq!(slot.native_status().unwrap().active_settings, 1);
        assert_eq!(slot.native_status().unwrap().pending_settings, Some(2));
        accept_trip_operation(&mut slot, &authority, request_id, now);
        held.iter_mut()
            .find(|item| item.def.id == rune.id)
            .unwrap()
            .count += expected_withdrawal;
        bank_stock
            .iter_mut()
            .find(|item| item.def.id == rune.id)
            .unwrap()
            .count -= expected_withdrawal;
        frame.seed_inventory(held.clone(), 28);
        frame.seed_bank_observation(1, 1, Some(bank_stock.clone()), held.clone());
    }
    for rune in &spell.runes {
        let expected = match rune_stock {
            RuneStock::Full => rune.count * 5,
            RuneStock::Partial if rune.id == first.id => rune.count * 3,
            RuneStock::Partial => rune.count * 4,
            RuneStock::Empty => unreachable!(),
        };
        assert_eq!(
            held.iter()
                .find(|item| item.def.id == rune.id)
                .unwrap()
                .count,
            expected
        );
    }
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(matches!(
        effect,
        HostEffect::Interaction(InteractReq::Close)
    ));
    accept_trip_operation(&mut slot, &authority, request_id, now);
    frame.seed_bank_observation(-1, now, None, Vec::new());
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(
        matches!(effect, HostEffect::Interaction(InteractReq::Held { ref action, .. }) if action == "Wield")
    );
    accept_trip_operation(&mut slot, &authority, request_id, now);
    now += 1;
    tick(&mut slot, &frame, now);
    assert!(
        !slot.has_native_actions(),
        "return must wait for observed equipment"
    );
    let mut worn = axe;
    worn.container = ItemContainer::Equipment;
    frame.seed_equipment(vec![worn]);
    held.retain(|item| item.def.id != 1351);
    frame.seed_inventory(held, 28);
    let (old_authority, old_request, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("wear settlement must return")
    };
    assert_resource_return(&selected, &request, anchor);
    let return_target = request.target;
    slot.pause();
    assert!(!old_authority.live());
    slot.resume();
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("off-plane Resume must retain return step")
    };
    assert_resource_return(&selected, &request, anchor);
    assert_eq!(request.target, return_target);
    assert_eq!(request.target.level, 0);
    slot.complete_native_walk(
        &old_authority,
        WalkReceipt {
            request_id: old_request,
            evidence: EvidenceStamp {
                run: old_authority.run(),
                tick: now,
                sequence: now,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
    slot.reconnect_session_work();
    slot.on_is_up(true);
    assert!(!authority.live());
    let (fresh_authority, fresh_request, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("reconnect must retain return step")
    };
    assert_resource_return(&selected, &request, anchor);
    assert_eq!(request.target, return_target);
    assert_ne!(fresh_authority.run().session, authority.run().session);
    let _ = request_id;
    trip_position(&mut frame, request.target);
    let mut locs = work_locs;
    locs[0].id = 1276;
    locs[0].name = Some("Tree".into());
    locs[0].actions = vec![Some("Chop down".into())];
    let fresh_tree = locs[0].tile;
    frame.seed_locs(locs);
    slot.complete_native_walk(
        &fresh_authority,
        WalkReceipt {
            request_id: fresh_request,
            evidence: EvidenceStamp {
                run: fresh_authority.run(),
                tick: now,
                sequence: now,
            },
            end: WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
    let (authority, request_id, mut effect) = next_trip_effect(&mut slot, &frame, &mut now);
    if let HostEffect::Walk(request) = effect {
        assert_eq!(request.target, fresh_tree);
        assert_eq!(request.radius, 1);
        trip_position(
            &mut frame,
            WorldTile {
                x: fresh_tree.x - 1,
                ..fresh_tree
            },
        );
        slot.complete_native_walk(
            &authority,
            WalkReceipt {
                request_id,
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: now,
                    sequence: now,
                },
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
        );
        effect = next_trip_effect(&mut slot, &frame, &mut now).2;
    }
    assert!(
        matches!(effect, HostEffect::Interaction(InteractReq::Loc { .. })),
        "only observed return permits gathering: {:?}",
        slot.native_status()
    );
    slot.stop();
}

#[test]
fn supply_trip_waits_for_loaded_stock_equipment_and_return_and_survives_off_plane_interrupts() {
    for (incarnation, rune_stock) in [
        (4302, RuneStock::Full),
        (4303, RuneStock::Partial),
        (4304, RuneStock::Empty),
    ] {
        run_supply_trip_scenario(incarnation, rune_stock);
    }
}
fn lifecycle_hitpoints(frame: &mut GameSnapshot, effective: i32, base: i32) {
    let mut stats = frame.stats().to_vec();
    if let Some(hitpoints) = stats.iter_mut().find(|stat| stat.index == 3) {
        hitpoints.name = "hitpoints".into();
        hitpoints.effective = effective;
        hitpoints.base = base;
    } else {
        stats.push(StatView {
            index: 3,
            name: "hitpoints".into(),
            effective,
            base,
            xp: 0,
            used: true,
        });
    }
    frame.seed_stats(stats);
}

fn lifecycle_frame_copy(source: &GameSnapshot) -> GameSnapshot {
    let mut frame = snapshot(&[]);
    frame.seed_world(*source.world());
    frame.seed_local_player(source.local_player().unwrap().clone());
    frame.seed_inventory(source.inventory().to_vec(), source.inventory_size());
    frame.seed_equipment(source.equipment().to_vec());
    frame.seed_stats(source.stats().to_vec());
    frame.seed_locs(source.locs().to_vec());
    frame.seed_npcs(source.npcs().to_vec());
    frame.seed_chat_lines(source.chat_lines().to_vec());
    frame
}

fn lifecycle_death_frame(
    baseline: &GameSnapshot,
    sequence: i32,
    tile: WorldTile,
    hp_effective: i32,
    hp_base: i32,
) -> GameSnapshot {
    let mut frame = lifecycle_frame_copy(baseline);
    let mut player = frame
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.actor.tile = tile;
    player.player.network = tile;
    frame.seed_local_player(player);
    lifecycle_hitpoints(&mut frame, hp_effective, hp_base);
    frame.seed_chat_lines(vec![api::snapshot::ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence,
    }]);
    frame
}

fn lifecycle_status_integer(slot: &SlotScript, key: &str) -> i64 {
    let status = slot.native_status().expect("compiled status is published");
    let field = status
        .fields
        .iter()
        .find(|field| field.key == key)
        .unwrap_or_else(|| panic!("missing status field {key}"));
    match &field.value {
        script::native::StatusValue::Integer(value) => *value,
        other => panic!("status field {key} is not an integer: {other:?}"),
    }
}

fn lifecycle_status_text(slot: &SlotScript, key: &str) -> String {
    let status = slot.native_status().expect("compiled status is published");
    let field = status
        .fields
        .iter()
        .find(|field| field.key == key)
        .unwrap_or_else(|| panic!("missing status field {key}"));
    match &field.value {
        script::native::StatusValue::Text(value) => value.to_string(),
        other => panic!("status field {key} is not text: {other:?}"),
    }
}

fn recreate_gatherer(slot: &mut SlotScript) {
    slot.restart_from_identity(Instant::now())
        .expect("watchdog recreation should retain the active run");
    let deadline = Instant::now() + Duration::from_secs(20);
    while slot.state() == script::RunState::Starting {
        assert!(Instant::now() < deadline, "compiled recreation stalled");
        slot.observe_lifecycle();
        std::thread::yield_now();
    }
    assert_eq!(slot.state(), script::RunState::Running);
}

fn complete_lifecycle_walk(
    slot: &mut SlotScript,
    authority: &script::native::HostAuthority,
    request_id: u64,
    now: u64,
    end: script::native::WalkEnd,
) {
    slot.complete_native_walk(
        authority,
        script::native::WalkReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            end,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
}

fn lifecycle_bank(
    tile: WorldTile,
) -> (
    api::named_banks::NamedBank,
    Arc<api::named_banks::NamedBankFacts>,
) {
    let bank = api::named_banks::NamedBank::new("fixture-bank", tile);
    let facts = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
    (bank, facts)
}

fn accept_lifecycle_bank_pick(
    slot: &mut SlotScript,
    authority: &script::native::HostAuthority,
    request_id: u64,
    now: u64,
    bank: api::named_banks::NamedBank,
    bank_tile: WorldTile,
) {
    use script::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};

    slot.complete_native_bank_pick(
        authority,
        BankPickReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            selected: SelectedBank {
                bank_index: 0,
                access_tile: bank_tile,
                kind: PickKind::Reachable,
                access: Some(Arc::new(BankStandAccess {
                    bank,
                    stand_tile: bank_tile,
                    kind: AccessKind::Booth,
                    stand_op: 2,
                    name: Some(Arc::from("Bank booth")),
                    choose: None,
                })),
            },
        },
    );
}

const LUMBRIDGE_RESPAWN_EDGE: WorldTile = WorldTile {
    x: 3224,
    z: 3215,
    level: 0,
};

#[test]
fn same_frame_death_observation_uses_default_recovery_inside_the_same_scene() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let initial_world = *initial.world();
    // Deliberately omit deathPolicy/maxDeaths: G4a defaults to Recover/2.
    let mut slot = started(4400, &selected);
    tick(&mut slot, &initial, 1);

    // The death line, restored base HP and Lumbridge respawn tile all arrive
    // together. The tile is on the edge of the inclusive ±3 square.
    let death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    assert_eq!(death.world().map_base_x, initial_world.map_base_x);
    assert_eq!(death.world().map_base_z, initial_world.map_base_z);
    tick(&mut slot, &death, 2);
    assert_ne!(
        slot.native_status().unwrap().phase,
        NativePhase::Blocked,
        "the death latch frame already proves respawn"
    );
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);

    for now in 3..=5 {
        tick(&mut slot, &death, now);
        assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
    }
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 2);
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Working);
    assert_ne!(anchor, LUMBRIDGE_RESPAWN_EDGE);
    slot.stop();
}

#[test]
fn recreated_runner_latches_gap_death_after_empty_or_initialized_watermark() {
    for (incarnation, baseline_sequence) in [(4401, None), (4402, Some(7))] {
        let selected = selected();
        let mut initial = depleted_snapshot(&selected);
        lifecycle_hitpoints(&mut initial, 10, 10);
        initial.seed_chat_lines(
            baseline_sequence
                .map(|sequence| {
                    vec![api::snapshot::ChatLineView {
                        type_: 0,
                        username: None,
                        text: "Welcome to RuneScape".into(),
                        sequence,
                    }]
                })
                .unwrap_or_default(),
        );
        let mut slot = started(incarnation, &selected);
        tick(&mut slot, &initial, 1);
        assert_eq!(lifecycle_status_integer(&slot, "deaths"), 0);

        // No instance examines the new death line. In particular, the empty
        // ring above retains Some(0), so sequence 1 is new after recreation.
        recreate_gatherer(&mut slot);
        let death_sequence = baseline_sequence.map_or(1, |sequence| sequence + 1);
        let death = lifecycle_death_frame(&initial, death_sequence, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
        tick(&mut slot, &death, 2);
        assert_ne!(
            slot.native_status().unwrap().phase,
            NativePhase::Blocked,
            "a death delivered in the recreation gap must start recovery"
        );
        assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
        assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);
        slot.stop();
    }
}

#[test]
fn death_recovery_reads_loaded_stock_instead_of_skipping_a_needed_supply_trip() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let bank_tile = WorldTile {
        x: anchor.x + 20,
        z: anchor.z,
        level: anchor.level,
    };
    let (bank, banks) = lifecycle_bank(bank_tile);
    let mut slot = started_with_banks(4403, &selected, SettingsBag::new(), banks);
    let mut now = 1;
    tick(&mut slot, &initial, now);

    // Death removed the only usable axe. The death frame is still a valid
    // respawn observation; recovery must reach its shared supply sequence.
    let mut death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    death.seed_inventory(Vec::new(), 28);
    death.seed_equipment(Vec::new());
    now += 1;
    tick(&mut slot, &death, now);
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &death, &mut now);
    assert!(
        matches!(effect, HostEffect::BankPick(_)),
        "a missing post-death tool must select a bank before recovery can continue"
    );
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 3);
    accept_lifecycle_bank_pick(&mut slot, &authority, request_id, now, bank, bank_tile);

    let work_locs = initial.locs().to_vec();
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &death, &mut now);
    assert!(matches!(effect, HostEffect::Walk(_)));
    trip_position(&mut death, bank_tile);
    let mut booth = work_locs[0].clone();
    booth.id = 2213;
    booth.tile = bank_tile;
    booth.name = Some("Bank booth".into());
    booth.actions = vec![None, Some("Use-quickly".into())];
    death.seed_locs(vec![booth]);
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request_id,
        now,
        script::native::WalkEnd::Arrived,
    );
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &death, &mut now);
    assert!(matches!(
        effect,
        HostEffect::Interaction(InteractReq::OpenStand { .. })
    ));
    accept_trip_operation(&mut slot, &authority, request_id, now);

    // An unopened/unread bank is not an empty-stock observation.
    death.seed_bank_observation(1, 1, None, Vec::new());
    for _ in 0..3 {
        now += 1;
        tick(&mut slot, &death, now);
        assert!(!slot.has_native_actions());
        assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    }

    death.seed_bank_observation(1, 1, Some(Vec::new()), Vec::new());
    for _ in 0..10 {
        now += 1;
        tick(&mut slot, &death, now);
        assert!(
            slot.take_native_action().is_none(),
            "an empty loaded bank must not produce a withdrawal"
        );
        if slot.native_status().unwrap().phase == NativePhase::Blocked {
            break;
        }
    }
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "supply-missing");
    assert!(failure.message.to_ascii_lowercase().contains("axe"));
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 3);
    slot.stop();
}

#[test]
fn death_return_and_recreation_resume_retained_live_steps() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let mut slot = started(4404, &selected);
    let mut now = 1;
    tick(&mut slot, &initial, now);
    let death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &death, now);

    let (old_authority, _, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("recovery step 5 must return to resources in the retained area")
    };
    assert_resource_return(&selected, &request, anchor);
    let return_target = request.target;
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 5);

    // Recreate while the return walk is pending. The old death line remains
    // in the ring but is already behind the retained watermark.
    recreate_gatherer(&mut slot);
    assert!(!old_authority.live());
    let (fresh_authority, fresh_request, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("Pending step 5 recreation must re-enter the return step")
    };
    assert_resource_return(&selected, &request, anchor);
    assert_eq!(request.target, return_target);
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 5);

    complete_lifecycle_walk(
        &mut slot,
        &fresh_authority,
        fresh_request,
        now,
        script::native::WalkEnd::Arrived,
    );
    let mut returned = lifecycle_frame_copy(&death);
    trip_position(&mut returned, return_target);
    // The return arrives in a rebuilt build area: its arrival is read on the
    // next tick, once that area's loc changes have landed.
    for _ in 0..2 {
        now += 1;
        tick(&mut slot, &returned, now);
    }
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 6);

    // Proving is retained too; the old death line cannot start another run.
    recreate_gatherer(&mut slot);
    now += 1;
    tick(&mut slot, &returned, now);
    assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 6);
    assert_eq!(lifecycle_status_integer(&slot, "recoveries"), 0);

    let mut product_only = lifecycle_frame_copy(&returned);
    product_only.seed_inventory(vec![log(0)], 28);
    now += 1;
    tick(&mut slot, &product_only, now);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 6);
    assert_eq!(lifecycle_status_integer(&slot, "recoveries"), 0);

    let mut proof = product_only;
    let mut stats = proof.stats().to_vec();
    let woodcutting = stats
        .iter_mut()
        .find(|stat| stat.name.eq_ignore_ascii_case("woodcutting"))
        .unwrap();
    woodcutting.xp = woodcutting.xp.saturating_add(50);
    proof.seed_stats(stats);
    now += 1;
    tick(&mut slot, &proof, now);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 0);
    assert_eq!(lifecycle_status_integer(&slot, "recoveries"), 1);
    assert_eq!(
        lifecycle_status_text(&slot, "last_event"),
        "recovered after death 1"
    );
    slot.stop();
}

#[test]
fn death_return_refused_stops_terminally_with_reason() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let mut slot = started(4413, &selected);
    let mut now = 1;
    tick(&mut slot, &initial, now);
    let death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &death, now);

    let (authority, request_id, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("recovery step 5 must return to resources in the retained area")
    };
    assert_resource_return(&selected, &request, anchor);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 5);
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request_id,
        now,
        script::native::WalkEnd::Refused,
    );

    now += 1;
    tick(&mut slot, &death, now);
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "return-failed");
    assert!(
        failure.message.contains("Refused")
            && failure
                .message
                .contains("arrival at the resource observation stand must be observed"),
        "terminal status retains the return refusal reason: {}",
        failure.message
    );
    assert_eq!(slot.state(), script::RunState::Idle);
    assert!(slot.native_run().is_none());
    assert!(!slot.has_native_actions());
    assert_eq!(
        slot.lifecycle_receipt().map(|receipt| receipt.reason),
        Some(failure.message.to_string())
    );
    assert!(slot.restart_from_identity(Instant::now()).is_err());
}

#[test]
fn default_max_deaths_allows_two_recoveries_with_a_haul_and_blocks_the_third_death() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    // Keep both G4a settings at their defaults: Recover, maxDeaths = 2.
    let mut slot = started(4405, &selected);
    let mut now = 1;
    tick(&mut slot, &initial, now);
    let mut first_death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &first_death, now);
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &first_death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("recovery must return to resources")
    };
    assert_resource_return(&selected, &request, anchor);
    trip_position(&mut first_death, request.target);
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request_id,
        now,
        script::native::WalkEnd::Arrived,
    );
    // The return arrives in a rebuilt build area: its arrival is read on the
    // next tick, once that area's loc changes have landed.
    for _ in 0..2 {
        now += 1;
        tick(&mut slot, &first_death, now);
    }
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 6);

    let mut product_and_xp = lifecycle_frame_copy(&first_death);
    product_and_xp.seed_inventory(vec![log(0)], 28);
    let mut stats = product_and_xp.stats().to_vec();
    let woodcutting = stats
        .iter_mut()
        .find(|stat| stat.name.eq_ignore_ascii_case("woodcutting"))
        .unwrap();
    woodcutting.xp += 50;
    product_and_xp.seed_stats(stats);
    now += 1;
    tick(&mut slot, &product_and_xp, now);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 0);
    assert_eq!(lifecycle_status_integer(&slot, "recoveries"), 1);

    // Complete one observed disposal before the next death.
    let mut full_pack = lifecycle_frame_copy(&product_and_xp);
    let mut present: Vec<i32> = (0..28).collect();
    full_pack.seed_inventory(present.iter().copied().map(log).collect(), 28);
    let mut hauled = false;
    for _ in 0..12 {
        now += 1;
        full_pack.seed_inventory(present.iter().copied().map(log).collect(), 28);
        tick(&mut slot, &full_pack, now);
        let sent = drain(&mut slot, now);
        present.retain(|slot| !sent.contains(slot));
        if present.is_empty() {
            now += 1;
            full_pack.seed_inventory(Vec::new(), 28);
            tick(&mut slot, &full_pack, now);
            assert!(drain(&mut slot, now).is_empty());
            hauled = true;
            break;
        }
    }
    assert!(hauled, "the first post-recovery disposal must complete");
    assert_eq!(lifecycle_status_integer(&slot, "dropped"), 28);

    let second_death = lifecycle_death_frame(&full_pack, 2, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &second_death, now);
    assert_ne!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 2);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);

    let third_death = lifecycle_death_frame(&second_death, 3, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &third_death, now);
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "max-deaths");
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 3);
    slot.stop();
}

#[test]
fn stop_death_policy_terminates_the_recovery_run() {
    let selected = selected();
    let mut settings = SettingsBag::new();
    settings.insert("deathPolicy".into(), serde_json::json!("Stop"));
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let mut slot = started_with(4406, &selected, settings);
    tick(&mut slot, &initial, 1);
    let death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    tick(&mut slot, &death, 2);
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "died");
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);

    assert_eq!(slot.state(), script::RunState::Idle);
    assert!(slot.native_run().is_none());
    for now in 3..=6 {
        tick(&mut slot, &death, now);
    }
    assert_eq!(slot.native_status().as_deref(), Some(status.as_ref()));
    assert!(!slot.want_run);
    slot.stop();
}

#[test]
fn random_hold_release_revalidates_lost_or_broken_pickaxe_before_gathering() {
    for (incarnation, broken_head) in [(4407, false), (4408, true)] {
        let selected = selected();
        let frame = mining_snapshot(&selected, "mining.coal", false);
        let anchor = frame.local_player().unwrap().player.actor.tile;
        let bank_tile = WorldTile {
            x: anchor.x + 20,
            z: anchor.z,
            level: anchor.level,
        };
        let (bank, banks) = lifecycle_bank(bank_tile);
        let mut settings = SettingsBag::new();
        settings.insert("skill".into(), serde_json::json!("Mining"));
        settings.insert("miningResources".into(), serde_json::json!(["coal"]));
        settings.insert("disposition".into(), serde_json::json!("Bank"));
        let mut slot = started_with_banks(incarnation, &selected, settings, banks);

        tick(&mut slot, &frame, 1);
        assert!(
            slot.has_native_actions(),
            "live mining should enqueue its op"
        );
        tick_with_hold(&mut slot, &frame, 2, true);
        assert!(
            !slot.has_native_actions(),
            "an owned random hold revokes queued gathering before dispatch"
        );

        let mut released = lifecycle_frame_copy(&frame);
        released.seed_equipment(Vec::new());
        if broken_head {
            // This is an observed replacement object from the blast event, not
            // a supported pickaxe id in the selected catalogue.
            let mut broken = log(0);
            broken.def = def(i32::MAX, "Broken pickaxe");
            released.seed_inventory(vec![broken], 28);
        } else {
            // Lost-head event: the handle has been reattached but no head is
            // left in the player's inventory.
            released.seed_inventory(Vec::new(), 28);
        }
        let mut now = 2;
        let (authority, request_id, effect) = next_trip_effect(&mut slot, &released, &mut now);
        assert!(
            matches!(effect, HostEffect::BankPick(_)),
            "release must revalidate and enter the shared supply trip"
        );
        assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 0);
        accept_lifecycle_bank_pick(&mut slot, &authority, request_id, now, bank, bank_tile);
        slot.stop();
    }
}

#[test]
fn food_is_eaten_at_the_boundary_then_refilled_by_the_shared_supply_trip() {
    let selected = selected();
    let mut frame = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut frame, 6, 10);
    frame.seed_chat_lines(Vec::new());
    let anchor = frame.local_player().unwrap().player.actor.tile;
    let bank_tile = WorldTile {
        x: anchor.x + 20,
        z: anchor.z,
        level: anchor.level,
    };
    let (bank, banks) = lifecycle_bank(bank_tile);
    let mut settings = SettingsBag::new();
    settings.insert("food".into(), serde_json::json!("Lobster"));
    settings.insert("foodTarget".into(), serde_json::json!(1));
    settings.insert("eatBelow".into(), serde_json::json!(5));
    let mut lobster = log(0);
    lobster.def = def(379, "Lobster");
    lobster.actions = vec![Some("Eat".into()), Some("Drop".into())];
    frame.seed_inventory(vec![lobster], 28);
    let mut slot = started_with_banks(4409, &selected, settings, banks);

    tick(&mut slot, &frame, 1);
    assert!(
        !slot.has_native_actions(),
        "HP above eatBelow does not consume food"
    );
    lifecycle_hitpoints(&mut frame, 5, 10);
    tick(&mut slot, &frame, 2);
    let eat = slot
        .take_native_action()
        .expect("HP at eatBelow should start an Eat operation");
    assert!(matches!(
        eat.effect,
        HostEffect::Interaction(InteractReq::Held {
            ref name,
            ref action,
            ..
        }) if name == "Lobster" && action == "Eat"
    ));
    let authority = eat.authority();
    accept_trip_operation(&mut slot, &authority, eat.request_id.get(), 2);

    // An accepted packet alone is not an observed meal, and must not be
    // reissued while the same food and HP remain visible.
    tick(&mut slot, &frame, 3);
    assert!(!slot.has_native_actions());
    frame.seed_inventory(Vec::new(), 28);
    lifecycle_hitpoints(&mut frame, 7, 10);
    let mut now = 3;
    tick(&mut slot, &frame, 4);
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(
        matches!(effect, HostEffect::BankPick(_)),
        "the observed empty food slot must re-enter the supply trip"
    );
    accept_lifecycle_bank_pick(&mut slot, &authority, request_id, now, bank, bank_tile);
    slot.stop();
}

#[test]
fn recreated_pending_respawn_still_requires_position_and_restored_hitpoints() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let outside = WorldTile {
        x: 3225,
        z: 3218,
        level: 0,
    };
    let mut slot = started(4410, &selected);
    tick(&mut slot, &initial, 1);
    let mut death = lifecycle_death_frame(&initial, 1, outside, 10, 10);
    tick(&mut slot, &death, 2);
    recreate_gatherer(&mut slot);
    for now in 3..=8 {
        tick(&mut slot, &death, now);
        assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);
        assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
        assert!(!slot.has_native_actions());
    }
    trip_position(&mut death, LUMBRIDGE_RESPAWN_EDGE);
    lifecycle_hitpoints(&mut death, 9, 10);
    tick(&mut slot, &death, 9);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);
    lifecycle_hitpoints(&mut death, 10, 10);
    for now in 10..=13 {
        tick(&mut slot, &death, now);
    }
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 2);
    slot.stop();
}

#[test]
fn death_in_proving_recreation_gap_blocks_as_a_second_death() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let mut slot = started(4411, &selected);
    tick(&mut slot, &initial, 1);
    let mut death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    let mut now = 2;
    tick(&mut slot, &death, now);
    let (authority, request, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(return_request) = effect else {
        panic!("recovery must return to resources")
    };
    assert_resource_return(&selected, &return_request, anchor);
    trip_position(&mut death, return_request.target);
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request,
        now,
        script::native::WalkEnd::Arrived,
    );
    // The return arrives in a rebuilt build area: its arrival is read on the
    // next tick, once that area's loc changes have landed.
    for _ in 0..2 {
        now += 1;
        tick(&mut slot, &death, now);
    }
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 6);
    recreate_gatherer(&mut slot);
    let second = lifecycle_death_frame(&death, 2, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    now += 1;
    tick(&mut slot, &second, now);
    let status = slot.native_status().unwrap();
    let failure = status.failure.as_ref().unwrap();
    assert_eq!(failure.code.as_ref(), "died-again");
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 2);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 1);
    assert!(!slot.has_native_actions());
    slot.stop();
}

#[test]
fn recovery_return_reconnect_on_another_plane_keeps_the_retained_resource_area() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let anchor = initial.local_player().unwrap().player.actor.tile;
    let mut slot = started(4412, &selected);
    tick(&mut slot, &initial, 1);
    let mut death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    let mut now = 2;
    tick(&mut slot, &death, now);
    let (old, _, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("recovery must return to resources")
    };
    assert_resource_return(&selected, &request, anchor);
    slot.pause();
    trip_position(
        &mut death,
        WorldTile {
            level: 1,
            ..LUMBRIDGE_RESPAWN_EDGE
        },
    );
    slot.resume();
    slot.reconnect_session_work();
    slot.on_is_up(true);
    assert!(!old.live());
    let (_, _, effect) = next_trip_effect(&mut slot, &death, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("reconnect must re-enter the retained return, not gather or supply");
    };
    assert_resource_return(&selected, &request, anchor);
    assert_eq!(lifecycle_status_integer(&slot, "recovery_step"), 5);
    assert_eq!(lifecycle_status_integer(&slot, "deaths"), 1);
    assert!(slot.native_status().unwrap().failure.is_none());
    slot.stop();
}

#[test]
fn refused_death_return_stops_and_cannot_be_recreated() {
    let selected = selected();
    let mut initial = depleted_snapshot(&selected);
    lifecycle_hitpoints(&mut initial, 10, 10);
    initial.seed_chat_lines(Vec::new());
    let mut slot = started(4410, &selected);
    tick(&mut slot, &initial, 1);
    let death = lifecycle_death_frame(&initial, 1, LUMBRIDGE_RESPAWN_EDGE, 10, 10);
    let mut now = 2;
    tick(&mut slot, &death, now);
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &death, &mut now);
    assert!(matches!(effect, HostEffect::Walk(_)));
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request_id,
        now,
        script::native::WalkEnd::Refused,
    );
    tick(&mut slot, &death, now + 1);
    assert_eq!(slot.state(), script::RunState::Idle);
    assert!(!slot.want_run);
    assert!(slot.native_run().is_none());
    assert!(!authority.live());
    assert!(!slot.has_native_actions());
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "return-failed"
    );
    assert!(slot.restart_from_identity(Instant::now()).is_err());
    tick(&mut slot, &death, now + 2);
    assert_eq!(slot.native_status().as_deref(), Some(status.as_ref()));
}

fn site_willow_bag(site: &str) -> SettingsBag {
    let mut bag = SettingsBag::new();
    bag.insert("woodcuttingResources".into(), serde_json::json!(["willow"]));
    bag.insert("location".into(), serde_json::json!("Site"));
    bag.insert("site".into(), serde_json::json!(site));
    bag
}

fn site_willow_frame() -> GameSnapshot {
    let mut frame = snapshot(&[]);
    frame.seed_stats(vec![StatView {
        index: 8,
        name: "woodcutting".into(),
        effective: 30,
        base: 30,
        xp: 0,
        used: true,
    }]);
    frame
}

fn site_work_anchor(slot: &SlotScript) -> WorldTile {
    let area = lifecycle_status_text(slot, "area");
    let coordinates = area
        .strip_prefix("site (")
        .unwrap()
        .split_once(')')
        .unwrap()
        .0;
    let parts: Vec<i32> = coordinates.split(',').map(|v| v.parse().unwrap()).collect();
    WorldTile {
        x: parts[0],
        z: parts[1],
        level: parts[2],
    }
}

#[test]
fn site_first_walk_resume_and_changed_site_fresh_start_keep_selected_resource_anchor() {
    let selected = selected();
    let mut slot = started_with(4420, &selected, site_willow_bag("woodcutting.draynor"));
    let mut frame = site_willow_frame();
    let mut now = 0;
    let (_, _, first) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(
        matches!(first, HostEffect::Walk(_)),
        "Site starts outside the work area"
    );
    let anchor = site_work_anchor(&slot);
    let catalog = fixture_catalog(&selected);
    let region = api::gather_methods::SceneRegionInput {
        min_x: anchor.x,
        max_x: anchor.x,
        min_z: anchor.z,
        max_z: anchor.z,
        level: anchor.level,
    };
    assert!(catalog.methods_for_resource("willow").any(|method| catalog
        .spots(method, &region)
        .unwrap()
        .any(|spot| spot.origin == anchor)));
    slot.pause();
    trip_position(
        &mut frame,
        WorldTile {
            x: 2809,
            z: 3441,
            level: 1,
        },
    );
    slot.resume();
    now += 1;
    tick(&mut slot, &frame, now);
    assert_eq!(
        site_work_anchor(&slot),
        anchor,
        "Resume cannot relocate a Site"
    );

    let run = slot.native_run().unwrap();
    let worker_selected = selected.clone();
    let changed = site_willow_bag("woodcutting.market");
    let config = api::selected::FamilyPreparation::run(move |families| {
        script::slot::prepare_config(
            families,
            script::CompiledId("Gatherer"),
            2,
            Arc::new(changed),
            worker_selected,
            Arc::default(),
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    slot.configure_compiled(config, run);
    now += 1;
    tick(&mut slot, &frame, now);
    assert_eq!(slot.native_status().unwrap().active_settings, 1);
    assert_eq!(slot.native_status().unwrap().pending_settings, Some(2));
    assert_eq!(site_work_anchor(&slot), anchor);
    slot.stop();

    // A fresh Start on the SAME slot must clear the previous Site anchor
    // through the production Stop path (`RetainedMemory` is dropped on
    // Stop), not merely by constructing a new slot.
    slot.start_compiled(
        "alice",
        script::CompiledId("Gatherer"),
        Arc::new(site_willow_bag("woodcutting.market")),
        Arc::clone(&selected),
        Arc::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(outcome) => {
                assert_eq!(outcome, StartOutcome::Ready);
                break;
            }
            StartPoll::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            StartPoll::NotOwed => panic!("no start owed"),
        }
    }
    let mut now = 0;
    next_trip_effect(&mut slot, &site_willow_frame(), &mut now);
    assert_ne!(
        site_work_anchor(&slot),
        anchor,
        "fresh Start clears the previous Site anchor"
    );
    slot.stop();
}

#[test]
fn site_refused_initial_walk_stops_with_nav_detail() {
    let selected = selected();
    let mut slot = started_with(4422, &selected, site_willow_bag("woodcutting.draynor"));
    let frame = site_willow_frame();
    let mut now = 0;
    let (authority, request, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    assert!(matches!(effect, HostEffect::Walk(_)));
    slot.complete_native_walk(
        &authority,
        script::native::WalkReceipt {
            request_id: request,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: now,
                sequence: now,
            },
            end: script::native::WalkEnd::Refused,
            blocked: None,
            detail: Some(Arc::from("no route to the selected site")),
            refusal: None,
            assessment: None,
            escape: None,
        },
    );
    now += 1;
    tick(&mut slot, &frame, now);
    assert_eq!(slot.state(), script::RunState::Idle);
    assert!(!slot.want_run);
    let status = slot.native_status().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    assert!(status
        .failure
        .as_ref()
        .unwrap()
        .message
        .contains("no route to the selected site"));
    assert!(!slot.has_native_actions());
}

#[test]
fn exhausted_site_stops_instead_of_widening() {
    use api::gather_methods::{known_rows, TargetClass};
    use api::selected::EntityId;
    let selected = selected();
    let mut slot = started_with(4423, &selected, site_willow_bag("woodcutting.draynor"));
    let mut frame = site_willow_frame();
    let mut now = 0;
    let (authority, request_id, effect) = next_trip_effect(&mut slot, &frame, &mut now);
    let HostEffect::Walk(request) = effect else {
        panic!("initial Site approach")
    };
    let anchor = site_work_anchor(&slot);
    trip_position(&mut frame, request.target);
    let region = api::gather_methods::SceneRegionInput {
        min_x: anchor.x - 12,
        max_x: anchor.x + 12,
        min_z: anchor.z - 12,
        max_z: anchor.z + 12,
        level: anchor.level,
    };
    let catalog = fixture_catalog(&selected);
    let mut locs = Vec::new();
    let template = depleted_snapshot(&selected).locs()[0].clone();
    for method in catalog.methods_for_resource("willow") {
        let EntityId::Loc(id) = known_rows(&method.targets)
            .iter()
            .find(|target| target.class == TargetClass::Depleted)
            .unwrap()
            .entity
        else {
            panic!("willow stump")
        };
        for spot in catalog.spots(method, &region).unwrap() {
            let mut loc = template.clone();
            loc.id = id;
            loc.tile = spot.origin;
            locs.push(loc);
        }
    }
    assert!(!locs.is_empty());
    frame.seed_locs(locs);
    complete_lifecycle_walk(
        &mut slot,
        &authority,
        request_id,
        now,
        script::native::WalkEnd::Arrived,
    );
    for tick_now in now + 1..now + 900 {
        tick(&mut slot, &frame, tick_now);
        assert!(
            !slot.has_native_actions(),
            "Site cannot search or walk to another camp"
        );
        if slot.state() == script::RunState::Idle {
            break;
        }
    }
    assert_eq!(slot.state(), script::RunState::Idle);
    let status = slot.native_status().unwrap();
    assert_eq!(
        status.failure.as_ref().unwrap().code.as_ref(),
        "resource-unavailable"
    );
    assert_eq!(site_work_anchor(&slot), anchor);
}

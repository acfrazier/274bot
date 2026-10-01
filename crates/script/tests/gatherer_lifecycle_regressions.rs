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
            actor: ActorView {
                name: Some("alice".into()),
                actions: Vec::new(),
                tile: here,
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
            },
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

fn depleted_snapshot(selected: &api::game_data::SelectedGameData) -> GameSnapshot {
    use api::gather_methods::{SceneRegionInput, TargetClass};
    use api::selected::{EntityId, Knowledge};
    use api::snapshot::{LocLayer, LocView};
    let catalog = api::gather_methods::cached(selected).expect("slot holds the catalog");
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
    selected: &api::game_data::SelectedGameData,
    method_id: &str,
    depleted: bool,
) -> GameSnapshot {
    use api::gather_methods::{known_rows, SceneRegionInput, TargetClass};
    use api::selected::{EntityId, Truth};
    let catalog = api::gather_methods::cached(selected).expect("slot holds the catalog");
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
    player.player.actor.tile = WorldTile {
        x: spot.origin.x + 1,
        ..spot.origin
    };
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
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
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
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_compiled(
        "alice",
        script::CompiledId("Gatherer"),
        Arc::new(bag),
        Arc::clone(selected),
        Arc::default(),
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
                HostEffect::Interaction(InteractReq::ContinueDialog)
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

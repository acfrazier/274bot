use super::*;
use crate::native::tickcycle::{AnimClass, AnimInstruction, CycleSignals, CycleStep, TickPolicy};
use crate::quester::families::tests::{
    def, local_player, with_tick, with_tick_bank, with_tick_reach,
};
use api::interact::server_ordered::{RowDispatch, ServerOrderedDispatch};
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, SceneView,
    StatView, WorldStateView, WorldTile,
};

fn fixture() -> (Gatherer, GameSnapshot, TargetPlan) {
    fixture_with(&[])
}

fn fixture_with(extra: &[(&str, serde_json::Value)]) -> (Gatherer, GameSnapshot, TargetPlan) {
    let extra: Vec<(String, serde_json::Value)> = extra
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    let config = api::selected::FamilyPreparation::run(move |families| {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut settings = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Mining")),
            ("miningResources", serde_json::json!(["copper"])),
            ("tickMethod", serde_json::json!("Knife")),
            ("food", serde_json::json!("Lobster")),
        ] {
            settings.insert(key.into(), value);
        }
        for (key, value) in extra {
            settings.insert(key, value);
        }
        crate::slot::prepare_config(
            families,
            crate::CompiledId("Gatherer"),
            1,
            Arc::new(settings),
            selected,
            Arc::default(),
            None,
        )
    })
    .unwrap()
    .join()
    .unwrap()
    .unwrap();
    let prepared = Arc::clone(config.get::<Arc<Prepared>>().unwrap());
    let method_index = prepared.methods[0];
    let method = &prepared.catalog.methods()[method_index];
    let entity = known_rows(&method.targets)
        .iter()
        .find(|row| row.class == api::gather_methods::TargetClass::Resource)
        .unwrap()
        .entity;
    let target = TargetPlan {
        entity,
        tile: WorldTile {
            x: 3201,
            z: 3202,
            level: 0,
        },
        op: "Mine".into(),
        alias: "Copper rocks".into(),
        products: Default::default(),
        products_len: 0,
        skill_stat: 14,
        method_index: method_index as u16,
        npc_index: -1,
    };
    let run = RunKey {
        slot: 1,
        run: 1,
        session: 1,
    };
    let mut gatherer = Gatherer::new(run, config, prepared, GatherRetained::default());
    gatherer.tool = ToolState {
        id: 1265,
        worn: false,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1265, "Bronze pickaxe"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    snapshot.seed_equipment(vec![]);
    snapshot.seed_stats(vec![StatView {
        index: 14,
        name: "mining".into(),
        base: 99,
        effective: 99,
        xp: 0,
        used: true,
    }]);
    (gatherer, snapshot, target)
}

fn seed_access_scene(
    snapshot: &mut GameSnapshot,
    target: &TargetPlan,
    here: WorldTile,
    wall_x: Option<i32>,
) -> api::query::ReachQueryView {
    let api::selected::EntityId::Loc(id) = target.entity else {
        panic!("copper replacement is a loc")
    };
    const SIDE: i32 = 41;
    let base_x = here.x - SIDE / 2;
    let base_z = here.z - SIDE / 2;
    let mut scene = SceneView {
        available: true,
        base_x,
        base_z,
        level: here.level,
        width: SIDE,
        height: SIDE,
        collision_flags: vec![0; (SIDE * SIDE) as usize],
    };
    let index = |tile: WorldTile| ((tile.x - base_x) * SIDE + tile.z - base_z) as usize;
    let blocked = client::dash3d::CollisionFlag::SQ_BLOCKED;
    scene.collision_flags[index(target.tile)] |= blocked;
    if let Some(x) = wall_x {
        for z in base_z..base_z + SIDE {
            scene.collision_flags[index(WorldTile {
                x,
                z,
                level: here.level,
            })] |= blocked;
        }
    }
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: base_x,
        map_base_z: base_z,
        level: here.level,
        members: true,
        ..WorldStateView::default()
    });
    snapshot.seed_scene(scene);
    snapshot.seed_local_player(local_player(here));
    snapshot.seed_locs(vec![LocView {
        typecode: 0,
        info: 0,
        id,
        name: Some(target.alias.to_string()),
        description: None,
        actions: vec![Some(target.op.to_string())],
        tile: target.tile,
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
    }]);
    snapshot.seed_npcs(vec![]);
    let scene = snapshot.scene();
    let flood = api::query::SceneQuery::new(scene, Some(here))
        .flood_reach()
        .expect("live collision makes a posted reach flood");
    api::query::pack_reach_query(scene, Some(&flood))
}

fn seed_timing_stock(gatherer: &Gatherer, snapshot: &mut GameSnapshot) {
    let kit = gatherer.prepared.supply.tick_kit.as_ref().unwrap();
    snapshot.seed_inventory(
        [gatherer.tool.id, kit.knife.id, kit.logs.id]
            .into_iter()
            .enumerate()
            .map(|(slot, id)| ItemView {
                def: def(id, "Fixture item"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: 1,
                actions: vec![],
                component_id: 3214,
            })
            .collect(),
        28,
    );
    // KnifeArm references require an observed packet generation.
    snapshot.seed_inventory_packet_generations(Some(1), None);
}

fn proven_idle_cycle(gatherer: &Gatherer, target: &TargetPlan) -> (TickCycle, TickPolicy) {
    let method = &gatherer.prepared.catalog.methods()[usize::from(target.method_index)];
    let spec = GatherTiming::spec(method, gatherer.tool.id).unwrap();
    let mut cycle = TickCycle::new(spec, 100, 7, 0);
    let mut policy = TickPolicy::default();
    let first = CycleSignals {
        tick: 101,
        anim: Some(AnimInstruction {
            serial: 8,
            class: AnimClass::Skill,
        }),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: true,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(cycle.drive(&first, &mut policy), CycleStep::Batch);
    cycle.note_emit(101, CycleStep::Batch, 1);
    let dispatch = ServerOrderedDispatch {
        request_id: 1,
        dispatch_tick: 101,
        row_count: 3,
        dispatched_prefix: 3,
        rows: [
            RowDispatch::HostQueued { wire_events: 1 },
            RowDispatch::HostQueued { wire_events: 1 },
            RowDispatch::HostQueued { wire_events: 2 },
            RowDispatch::NotAttempted,
            RowDispatch::NotAttempted,
        ],
        wire_events: 4,
        terminal: None,
    };
    let success = CycleSignals {
        tick: 102,
        anim: Some(AnimInstruction {
            serial: 9,
            class: AnimClass::Null,
        }),
        success_line: true,
        target_ready: false,
        receipt: Some((101, &dispatch)),
        ..first
    };
    assert_eq!(cycle.drive(&success, &mut policy), CycleStep::Wait);
    (cycle, policy)
}

#[test]
fn parked_depleting_idle_cycle_walks_before_remote_replacement_batch() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::HostEffect;

    for (here, target_tile) in [
        (
            WorldTile {
                x: 3200,
                z: 3201,
                level: 0,
            },
            WorldTile {
                x: 3201,
                z: 3202,
                level: 0,
            },
        ),
        (
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 3201,
                z: 3203,
                level: 0,
            },
        ),
    ] {
        let (mut gatherer, mut snapshot, mut target) = fixture();
        target.tile = target_tile;
        let api::selected::EntityId::Loc(loc_id) = target.entity else {
            panic!("copper replacement is a loc")
        };
        seed_timing_stock(&gatherer, &mut snapshot);
        let reach = seed_access_scene(&mut snapshot, &target, here, None);
        let (cycle, policy) = proven_idle_cycle(&gatherer, &target);
        gatherer.tick_policy = policy;
        gatherer.parked_cycle = Some(cycle);
        let selected = SelectedTarget {
            plan: target.clone(),
            class: PlacementClass::Live,
            observation_goals: None,
        };
        let mut ledger = None;
        with_tick_reach(&snapshot, &reach, &mut ledger, 103, |tick| {
            let request = selected
                .approach(tick.cx.snapshot(), tick.cx.evidence(), tick.walk_danger)
                .unwrap()
                .expect("a remote replacement requires a walk");
            let frame = tick.frame.snapshot.unwrap();
            let loc = frame
                .locs()
                .iter()
                .find(|loc| loc.id == loc_id && loc.tile == target.tile)
                .unwrap();
            assert_eq!(
                api::query::loc_approach::can_operate_from(loc, frame.scene(), here),
                Some(false)
            );
            assert_eq!(request.target, target.tile);
            gatherer.start_target(selected, tick);
        });
        assert!(
            matches!(gatherer.active, Active::Walk(_)),
            "a remote replacement must finish its normal walk before gathering"
        );
        assert!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(|action| { !matches!(action.effect, HostEffect::ServerOrdered(_)) }),
            "no replacement batch is emitted before the player reaches an exact stand"
        );
        assert!(
            gatherer.parked_cycle.is_some(),
            "the normal walk keeps ownership of the parked cycle"
        );
    }
}

#[test]
fn parked_depleting_idle_cycle_arms_a_local_operable_replacement() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::HostEffect;
    use crate::surface::ServerOrderedPayload;
    use api::interact::server_ordered::ServerOrderedRequest;

    let (mut gatherer, mut snapshot, target) = fixture();
    let api::selected::EntityId::Loc(loc_id) = target.entity else {
        panic!("copper replacement is a loc")
    };
    let here = WorldTile {
        x: 3200,
        z: 3202,
        level: 0,
    };
    seed_timing_stock(&gatherer, &mut snapshot);
    let reach = seed_access_scene(&mut snapshot, &target, here, None);
    let (cycle, policy) = proven_idle_cycle(&gatherer, &target);
    gatherer.tick_policy = policy;
    gatherer.parked_cycle = Some(cycle);
    let selected = SelectedTarget {
        plan: target.clone(),
        class: PlacementClass::Live,
        observation_goals: None,
    };
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 103, |tick| {
        assert!(
            selected
                .approach(tick.cx.snapshot(), tick.cx.evidence(), tick.walk_danger)
                .unwrap()
                .is_none(),
            "a locally operable replacement needs no walk"
        );
        let frame = tick.frame.snapshot.unwrap();
        let loc = frame
            .locs()
            .iter()
            .find(|loc| loc.id == loc_id && loc.tile == target.tile)
            .unwrap();
        assert_eq!(
            api::query::loc_approach::can_operate_from(loc, frame.scene(), here),
            Some(true)
        );
        gatherer.start_target(selected, tick);
    });
    assert!(matches!(gatherer.active, Active::Gather(_)));
    let action = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .expect("local replacement batch emitted immediately");
    let HostEffect::ServerOrdered(envelope) = &action.effect else {
        panic!("local replacement uses its checked server-ordered batch")
    };
    assert!(matches!(
        envelope.payload(),
        ServerOrderedPayload::Content(ServerOrderedRequest::KnifeArm { .. })
    ));
}

#[test]
fn parked_depleting_idle_cycle_keeps_walk_for_long_or_unreachable_access() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::HostEffect;

    for (here, target_tile, wall_x) in [
        (
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 3201,
                z: 3205,
                level: 0,
            },
            None,
        ),
        (
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            Some(3201),
        ),
    ] {
        let (mut gatherer, mut snapshot, mut target) = fixture();
        target.tile = target_tile;
        seed_timing_stock(&gatherer, &mut snapshot);
        let reach = seed_access_scene(&mut snapshot, &target, here, wall_x);
        let (cycle, policy) = proven_idle_cycle(&gatherer, &target);
        gatherer.tick_policy = policy;
        gatherer.parked_cycle = Some(cycle);
        let selected = SelectedTarget {
            plan: target,
            class: PlacementClass::Live,
            observation_goals: None,
        };
        let mut ledger = None;
        with_tick_reach(&snapshot, &reach, &mut ledger, 103, |tick| {
            gatherer.start_target(selected, tick);
        });
        assert!(
            matches!(gatherer.active, Active::Walk(_)),
            "a long or flood-unreachable loc must retain the normal walk"
        );
        assert!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(|action| { !matches!(action.effect, HostEffect::ServerOrdered(_)) }),
            "no replacement batch is emitted while the normal approach walk remains active"
        );
        assert!(
            gatherer.parked_cycle.is_some(),
            "the normal walk keeps the checked cycle parked"
        );
    }
}

#[test]
fn not_due_kit_shortage_comes_from_built_plan_and_is_published_without_a_trip() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut gatherer, snapshot, target) = fixture();
    let mut bank = api::bank_memory::BankMemory::default();
    bank.observe(
        &[],
        64,
        api::bank_memory::ObservedAt {
            unix_secs: 1,
            tick: 1,
            bank_session: 1,
        },
    );
    with_tick_bank(&snapshot, Some(&bank), &mut None, 1, |tick| {
        let stock = tick.cx.snapshot().stock();
        let stats = tick.cx.snapshot().stats().unwrap().value;
        assert!(
            !SupplyPlan::due(&gatherer.prepared, stats, stock),
            "a known incomplete kit is not a trip"
        );
        assert!(SupplyPlan::tick_kit_missing(&gatherer.prepared, stock));
        assert!(gatherer.timing_for_target(&target, &tick.cx).is_none());
    });
    gatherer.refresh_tick_status();
    assert_eq!(
        gatherer.tick_method_status().as_ref(),
        "Knife (off: no-arm-supply)"
    );
    assert_eq!(gatherer.trip, TripStep::Idle);
    assert!(gatherer.failure.is_none());
}

#[test]
fn cancellation_hold_and_bank_trip_drop_cycle_but_keep_run_policy() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut gatherer, _, target) = fixture();
    gatherer
        .tick_policy
        .disable(crate::native::tickcycle::OffReason::MembersOnlyArm);
    let policy = gatherer.tick_policy;
    let method = &gatherer.prepared.catalog.methods()[usize::from(target.method_index)];
    let cycle = TickCycle::new(GatherTiming::spec(method, 1265).unwrap(), 100, 7, 0);
    gatherer.parked_cycle = Some(cycle);
    gatherer.cancel_active();
    assert!(gatherer.parked_cycle.is_none());
    assert_eq!(gatherer.tick_policy, policy);
    gatherer.parked_cycle = Some(cycle);
    gatherer.interrupt(Interrupt::Hold(true));
    assert!(gatherer.parked_cycle.is_none());
    assert_eq!(gatherer.tick_policy, policy);
    gatherer.parked_cycle = Some(cycle);
    gatherer.start_trip(TripStep::Deposit);
    assert!(gatherer.parked_cycle.is_none());
    assert_eq!(gatherer.tick_policy, policy);
}

#[test]
fn eat_pauses_parked_arm_and_exposes_pause_counter_before_publish() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut gatherer, mut snapshot, target) = fixture();
    let method = &gatherer.prepared.catalog.methods()[usize::from(target.method_index)];
    let mut cycle = TickCycle::new(GatherTiming::spec(method, 1265).unwrap(), 100, 7, 0);
    let signals = CycleSignals {
        tick: 101,
        anim: Some(AnimInstruction {
            serial: 8,
            class: AnimClass::Skill,
        }),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: true,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    let mut policy = TickPolicy::default();
    assert_eq!(cycle.drive(&signals, &mut policy), CycleStep::Batch);
    cycle.note_emit(101, CycleStep::Batch, 1);
    gatherer.tick_policy = policy;
    gatherer.parked_cycle = Some(cycle);
    let food = gatherer.prepared.supply.food.as_ref().unwrap();
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(food.id, &food.name),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "hitpoints".into(),
        base: 20,
        effective: 5,
        xp: 0,
        used: true,
    }]);
    let eat = OneOpArgs::eat(&food.name, food.id, 1);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 102, |tick| {
        gatherer.fence.observe(102);
        assert!(gatherer.begin_survival_click(eat, tick));
    });
    gatherer.dirty = false;
    gatherer.refresh_tick_status();
    assert!(
        gatherer.dirty,
        "pause is visible before publish's early return"
    );
    assert_eq!(gatherer.status_data(None).tick_stats.paused, 1);
    assert_eq!(gatherer.last_event.as_ref(), "tick: paused (eat)");
    assert!(matches!(gatherer.active, Active::Tend(_)));
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "eat never shares its window with an arm"
    );
}

#[test]
fn eating_after_an_ended_idle_cycle_adds_no_verdict_and_the_next_gather_restarts_natively() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::{HostEffect, InteractionReceipt};
    use crate::quester::families::tests::local_player;
    use api::interact::server_ordered::{RowDispatch, ServerOrderedDispatch};
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::AnimationUpdateView;
    let (mut gatherer, mut snapshot, target) = fixture();
    let method = &gatherer.prepared.catalog.methods()[usize::from(target.method_index)];
    let spec = GatherTiming::spec(method, 1265).unwrap();
    let skill_anim = method.tick_arm.for_tool(1265).unwrap().skill_anim;
    let mut cycle = TickCycle::new(spec, 100, 7, 0);
    let mut policy = TickPolicy::default();
    let first = CycleSignals {
        tick: 101,
        anim: Some(AnimInstruction {
            serial: 8,
            class: AnimClass::Skill,
        }),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: true,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(cycle.drive(&first, &mut policy), CycleStep::Batch);
    cycle.note_emit(101, CycleStep::Batch, 1);
    let dispatch = ServerOrderedDispatch {
        request_id: 1,
        dispatch_tick: 101,
        row_count: 3,
        dispatched_prefix: 3,
        rows: [
            RowDispatch::HostQueued { wire_events: 1 },
            RowDispatch::HostQueued { wire_events: 1 },
            RowDispatch::HostQueued { wire_events: 2 },
            RowDispatch::NotAttempted,
            RowDispatch::NotAttempted,
        ],
        wire_events: 4,
        terminal: None,
    };
    let delayed = CycleSignals {
        tick: 102,
        anim: Some(AnimInstruction {
            serial: 9,
            class: AnimClass::Null,
        }),
        success_line: true,
        target_ready: false,
        receipt: Some((101, &dispatch)),
        ..first
    };
    assert_eq!(cycle.drive(&delayed, &mut policy), CycleStep::Wait);
    assert_eq!(policy.stats().delayed, 1);
    let ended = policy.diagnostic();
    gatherer.tick_policy = policy;
    gatherer.parked_cycle = Some(cycle);
    gatherer.refresh_tick_status();
    let food = gatherer.prepared.supply.food.as_ref().unwrap();
    let food_id = food.id;
    let food_name = food.name.clone();
    let mut inventory: Vec<_> = [1265, 946, 1511, food_id]
        .into_iter()
        .enumerate()
        .map(|(slot, id)| ItemView {
            def: def(id, "Fixture item"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: slot as i32,
            count: 1,
            actions: vec![],
            component_id: 3214,
        })
        .collect();
    snapshot.seed_inventory(inventory.clone(), 28);
    snapshot.seed_inventory_packet_generations(Some(1), None);
    let mining = StatView {
        index: 14,
        name: "mining".into(),
        base: 99,
        effective: 99,
        xp: 0,
        used: true,
    };
    snapshot.seed_stats(vec![
        mining.clone(),
        StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 20,
            effective: 5,
            xp: 0,
            used: true,
        },
    ]);
    let mut player = local_player(target.tile);
    player.animation_update = Some(AnimationUpdateView {
        serial: 9,
        sequence: -1,
    });
    snapshot.seed_local_player(player);
    let EntityId::Loc(resource) = target.entity else {
        panic!("copper fixture")
    };
    snapshot.seed_locs(vec![LocView {
        typecode: 0,
        info: 0,
        id: resource,
        name: None,
        description: None,
        actions: vec![],
        tile: target.tile,
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
    }]);
    let eat = OneOpArgs::eat(&food_name, food_id, 1);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 102, |tick| {
        gatherer.fence.observe(102);
        assert!(gatherer.begin_survival_click(eat, tick));
    });
    gatherer.refresh_tick_status();
    assert_eq!(gatherer.tick_policy.diagnostic(), ended);
    assert_eq!(gatherer.tick_policy.stats().paused, 0);
    assert_eq!(gatherer.last_event.as_ref(), "eating at boundary");
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 103,
                sequence: 103,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    snapshot.seed_stats(vec![
        mining,
        StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 20,
            effective: 20,
            xp: 0,
            used: true,
        },
    ]);
    // The accepted bite's observation includes its consumed item. HP alone
    // cannot settle Eat because regeneration can raise it independently.
    inventory.retain(|row| row.def.id != food_id);
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_inventory_packet_generations(Some(2), None);
    let handle = with_tick(&snapshot, &mut ledger, 103, |tick| {
        let Active::Tend(eat) = &gatherer.active else {
            panic!("eat owner")
        };
        assert!(matches!(
            tick.actions.poll(eat, &mut tick.cx),
            std::task::Poll::Ready(Ok(true))
        ));
        gatherer.active = Active::None;
        let timing = gatherer.timing_for_target(&target, &tick.cx);
        tick.actions
            .begin::<GatherRun>(
                GatherRunArgs {
                    target,
                    catalog: Arc::clone(&gatherer.prepared.catalog),
                    stall_ticks: DEFAULT_STALL_TICKS,
                    quest_owned: false,
                    timing,
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(
        matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::Interaction(_)
        ),
        "the first gather emission after eating is a plain click"
    );
    let mut player = snapshot.local_player().unwrap().clone();
    player.animation_update = Some(AnimationUpdateView {
        serial: 10,
        sequence: skill_anim,
    });
    snapshot.seed_local_player(player);
    with_tick(&snapshot, &mut ledger, 104, |tick| {
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
    });
    assert!(
        matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::ServerOrdered(_)
        ),
        "the arm waits for the next fresh native roll"
    );
}

#[test]
fn per_target_applicability_rechecks_the_method_and_changed_tool_before_the_next_cycle() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut gatherer, mut snapshot, mut target) = fixture();
    snapshot.seed_inventory(
        [1265, 946, 1511]
            .into_iter()
            .enumerate()
            .map(|(slot, id)| ItemView {
                def: def(id, "Fixture item"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: 1,
                actions: vec![],
                component_id: 3214,
            })
            .collect(),
        28,
    );
    for (method_id, tool, expected) in [
        ("woodcutting.normal", 1349, Some(OffReason::NotInRelease)),
        (
            "fishing.category_633.op1",
            3157,
            Some(OffReason::NoDelayVariable),
        ),
        ("mining.iron", 1275, Some(OffReason::NoGain)),
        ("mining.iron", 1273, None),
        ("mining.iron.get_ore_normal", 1275, Some(OffReason::NoGain)),
        ("mining.iron.get_ore_normal", 1273, None),
        ("woodcutting.oak", 1349, None),
    ] {
        let index = gatherer
            .prepared
            .catalog
            .methods()
            .iter()
            .position(|method| method.id.0.as_ref() == method_id)
            .unwrap();
        let method = &gatherer.prepared.catalog.methods()[index];
        target.method_index = index as u16;
        target.entity = known_rows(&method.targets)
            .iter()
            .find(|row| row.class == api::gather_methods::TargetClass::Resource)
            .unwrap()
            .entity;
        gatherer.tool = ToolState {
            id: tool,
            worn: false,
        };
        with_tick(&snapshot, &mut None, 1, |tick| {
            assert_eq!(
                gatherer.timing_for_target(&target, &tick.cx).is_some(),
                expected.is_none(),
                "{method_id} tool {tool}"
            );
        });
        assert_eq!(
            gatherer.tick_applicability, expected,
            "{method_id} tool {tool}"
        );
    }
}

/// "Iron" holds a fast-handler method (regular rocks) and a normal-handler method (Haunted Mine veins). A cycle
/// parked after one method's target must never time the other method's: delayed against immediate yield decides
/// how the cycle reads a success, so the next target starts a cycle from its own method's facts.
#[test]
fn a_parked_cycle_serves_only_a_target_of_the_same_method_facts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut gatherer, mut snapshot, fast) =
        fixture_with(&[("miningResources", serde_json::json!(["iron"]))]);
    gatherer.tool = ToolState {
        id: 1273,
        worn: false,
    };
    seed_timing_stock(&gatherer, &mut snapshot);
    let catalog = Arc::clone(&gatherer.prepared.catalog);
    assert_eq!(
        catalog.methods()[usize::from(fast.method_index)]
            .id
            .0
            .as_ref(),
        "mining.iron",
        "the fixture target is regular iron"
    );
    let normal_index = gatherer
        .prepared
        .methods
        .iter()
        .copied()
        .find(|&index| catalog.methods()[index].id.0.as_ref() == "mining.iron.get_ore_normal")
        .expect("the iron option carries the Haunted Mine normal-handler method");
    let mut normal = fast.clone();
    normal.method_index = normal_index as u16;
    normal.entity = known_rows(&catalog.methods()[normal_index].targets)
        .iter()
        .find(|row| row.class == api::gather_methods::TargetClass::Resource)
        .unwrap()
        .entity;
    let (fast_cycle, fast_policy) = proven_idle_cycle(&gatherer, &fast);
    let (normal_cycle, _) = proven_idle_cycle(&gatherer, &normal);
    assert!(!fast_cycle.spec().delay_drop && normal_cycle.spec().delay_drop);

    gatherer.tick_policy = fast_policy;
    gatherer.parked_cycle = Some(normal_cycle);
    with_tick(&snapshot, &mut None, 1, |tick| {
        let timing = gatherer
            .timing_for_target(&fast, &tick.cx)
            .expect("regular iron arms with the knife");
        assert_eq!(
            timing.cycle.spec(),
            fast_cycle.spec(),
            "the Haunted Mine cycle is dropped, not continued, for a regular rock"
        );
        assert_ne!(timing.cycle, normal_cycle);
    });
    assert!(gatherer.parked_cycle.is_none());

    gatherer.parked_cycle = Some(fast_cycle);
    with_tick(&snapshot, &mut None, 2, |tick| {
        let timing = gatherer
            .timing_for_target(&fast, &tick.cx)
            .expect("regular iron arms with the knife");
        assert_eq!(
            timing.cycle, fast_cycle,
            "a cycle of the same method keeps going across targets"
        );
    });
}

/// The post-death recovery bite follows the shared tier rule
/// (`policy::eats_food`) with the user's `eatBelow` as the caller floor: no
/// bite at full Hitpoints even when `eatBelow` reaches the maximum, none in the
/// caution tier when the lobster's 12-point heal does not fit the missing
/// Hitpoints (37/40 with `eatBelow` 38), a bite when it fits (28/40), and in
/// the critical tier a bite whatever it overheals (1/40).
#[test]
fn recovery_bite_follows_the_shared_tier_rule_with_eat_below_as_the_floor() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (eat_below, hp, eats) in [
        (40, 40, false),
        (38, 37, false),
        (38, 28, true),
        (38, 1, true),
        (38, 39, false),
    ] {
        let (mut gatherer, mut snapshot, _) =
            fixture_with(&[("eatBelow", serde_json::json!(eat_below))]);
        let food = gatherer.prepared.supply.food.as_ref().unwrap();
        let food_name = food.name.clone();
        snapshot.seed_inventory(
            vec![ItemView {
                def: def(food.id, &food_name),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 40,
            effective: hp,
            xp: 0,
            used: true,
        }]);
        let mut ledger = None;
        let began = with_tick(&snapshot, &mut ledger, 102, |tick| {
            gatherer.fence.observe(102);
            gatherer.begin_recovery_eat(tick)
        });
        assert_eq!(
            began, eats,
            "{hp}/40 with eatBelow {eat_below}: the recovery bite"
        );
        assert_eq!(matches!(gatherer.active, Active::Tend(_)), eats);
    }
}

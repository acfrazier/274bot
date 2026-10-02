use super::*;
use crate::native::{ledger, HostEffect, NativeOutput, NativeTick, RetainedMemory, ScriptStatus};
use api::obj_names::ItemDefView;
use api::quest_progress::{EvidenceStamp, ProgressFlag, QuestProgress};
use api::selected::{ClientRevision, FactKey, Knowledge, RunKey, Truth};
use api::snapshot::{
    GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
    SnapshotView,
};
use std::time::Instant;

struct Output;
impl NativeOutput for Output {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
    fn settings_applied(&mut self, _: u64) {}
}
pub(crate) fn with_tick<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output(snapshot, ledger, tick, &mut Output, f)
}
pub(crate) fn with_tick_output<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    output: &mut dyn NativeOutput,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(snapshot, None, ledger, tick, output, f)
}

pub(crate) fn with_tick_reach<R>(
    snapshot: &GameSnapshot,
    reach: &api::query::ReachQueryView,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(snapshot, Some(reach), ledger, tick, &mut Output, f)
}

fn with_tick_output_reach<R>(
    snapshot: &GameSnapshot,
    reach: Option<&api::query::ReachQueryView>,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    output: &mut dyn NativeOutput,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let pin = data.selected_pin().unwrap();
    let evidence = api::quest_progress::EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    };
    let mut retained = RetainedMemory::default();
    let mut budget = ledger::TickBudget::default();
    budget.observe(tick);
    let mut actions = NativeActions { _private: () };
    let mut native = NativeTick {
        actions: &mut actions,
        cx: crate::native::ActionContext {
            evidence,
            observed_walk_outcome_seq: 0,
            pin: &pin,
            snapshot: SnapshotView::new(Some(snapshot), evidence).with_reach(reach),
            retained: &mut retained,
            action_id: 0,
            active_now: Duration::from_millis(tick * 600),
            wall_now: Instant::now(),
            ledger,
            budget: &mut budget,
            eligible: true,
        },
        output,
        pairs: None,
        #[cfg(feature = "load")]
        frame: crate::native::HostFrame {
            here: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: crate::CompiledTick {
                selected: Some(&data),
                reach: None,
                hold: false,
                interacts: Some(Vec::new()),
            },
        },
    };
    f(&mut native)
}
fn ready() -> GameSnapshot {
    let mut s = GameSnapshot::new();
    s.seed_ingame(2);
    s.seed_inventory(vec![], 28);
    s
}
fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}
pub(crate) fn def(id: i32, name: &str) -> ItemDefView {
    ItemDefView {
        id,
        name: Some(name.into()),
        stackable: false,
        members: false,
        base_value: 1,
        noted: false,
        certificate_link: -1,
        certificate_template: -1,
    }
}
fn ground() -> GroundItemView {
    GroundItemView {
        def: def(1944, "Egg"),
        count: 1,
        actions: vec![Some("Take".into())],
        tile: tile(3229, 3302),
        distance: 5,
    }
}
fn loc(id: i32, name: &str, op: &str) -> LocView {
    LocView {
        id,
        name: Some(name.into()),
        actions: vec![Some(op.into())],
        tile: tile(3167, 3308),
        distance: 3,
        typecode: 0,
        info: 0,
        description: None,
        layer: LocLayer::GroundDecoration,
        shape: 0,
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
    }
}
pub(crate) fn local_player(tile: WorldTile) -> api::snapshot::LocalPlayerView {
    api::snapshot::LocalPlayerView {
        player: api::snapshot::PlayerView {
            index: 0,
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
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
            },
            combat_level: 3,
            skill_level: 0,
        },
        energy: 100,
        weight: 0,
    }
}
pub(crate) fn post_user_input_walk_receipt(
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> u64 {
    let (request_id, run) = {
        let ledger = ledger.as_ref().expect("walk request must be queued");
        let action = ledger
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::Walk(_)))
            .expect("walk request must be queued");
        (action.request_id.get(), action.run())
    };
    ledger.as_mut().expect("ledger must remain present").walk = Some(crate::native::WalkReceipt {
        request_id,
        evidence: EvidenceStamp {
            run,
            tick,
            sequence: tick,
        },
        end: crate::native::WalkEnd::UserInput,
        blocked: None,
        detail: None,
    });
    request_id
}

fn assert_manual_movement(result: Poll<Result<StepOutcome, ActionError>>) {
    assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
}
fn wall_door_reach_view() -> api::query::ReachQueryView {
    let mut reachable = vec![0u32];
    reachable[0] = 1 << 4;
    let mut exact_rank = vec![u16::MAX; 9];
    exact_rank[4] = 0;
    api::query::ReachQueryView {
        available: true,
        base_x: 4,
        base_z: 4,
        level: 0,
        width: 3,
        height: 3,
        walkable: vec![0b1_1111_1111],
        reachable,
        reachable_adj: vec![1 << 4],
        adjacent_rank: exact_rank.clone(),
        exact_rank,
        step: vec![0; 9],
        canlight: Vec::new(),
    }
}
fn reach_args(kind: reach::ReachKind, wait: bool) -> reach::ReachArgs {
    reach::ReachArgs {
        kind,
        op: Arc::from("Take"),
        anchor: Some(tile(3227, 3300)),
        radius: 2,
        wait_if_missing: wait,
    }
}
fn egg(wait: bool) -> reach::ReachArgs {
    reach_args(
        reach::ReachKind::Ground {
            id: 1944,
            obj: Arc::from("Egg"),
        },
        wait,
    )
}
fn emitted(ledger: &Option<Box<ledger::Ledger>>) -> &InteractReq {
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Interaction(req) => req,
        _ => panic!("unexpected walk"),
    }
}
#[test]
fn reach_takes_the_observed_stack_not_the_anchor() {
    let mut s = ready();
    s.seed_ground_items(vec![ground()]);
    let mut ledger = None;
    let _handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Obj {
            x: 3229,
            z: 3302,
            ..
        }
    ));
}
#[test]
fn missing_drop_waits_without_clicking_then_takes_a_spawn() {
    let mut s = ready();
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(with_tick(&s, &mut ledger, 60, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    s.seed_ground_items(vec![ground()]);
    assert!(with_tick(&s, &mut ledger, 61, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Obj {
            x: 3229,
            z: 3302,
            ..
        }
    ));
}
#[test]
fn unready_ground_is_not_evidence_of_a_take() {
    let mut s = GameSnapshot::new();
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    assert!(with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    s.seed_ingame(2);
    assert!(with_tick(&s, &mut ledger, 3, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}
#[test]
fn reach_picks_the_observed_loc_not_the_anchor() {
    let mut s = ready();
    s.seed_locs(vec![loc(1551, "Wheat", "Pick")]);
    let mut ledger = None;
    let mut args = reach_args(
        reach::ReachKind::Loc {
            id: Some(1551),
            name: Some(Arc::from("wheat")),
        },
        false,
    );
    args.op = Arc::from("Pick");
    let _handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
    });
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Loc {
            x: 3167,
            z: 3308,
            id: Some(1551),
            ..
        }
    ));
}

#[test]
fn reach_walks_through_an_open_door_instead_of_closing_it() {
    let mut s = ready();
    s.seed_local_player(api::snapshot::LocalPlayerView {
        player: api::snapshot::PlayerView {
            index: 0,
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
                tile: tile(3077, 3426),
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
            skill_level: 0,
        },
        energy: 100,
        weight: 0,
    });
    let mut wheel = loc(2644, "Spinning wheel", "Spin");
    wheel.tile = tile(3081, 3430);
    wheel.distance = 4;
    let mut leaf = loc(1531, "Door", "Close");
    leaf.tile = tile(3076, 3426);
    leaf.distance = 1;
    let mut other_door = loc(1530, "Door", "Open");
    other_door.tile = tile(3077, 3431);
    other_door.distance = 5;
    s.seed_locs(vec![wheel.clone(), leaf, other_door]);
    let mut args = reach_args(
        reach::ReachKind::Loc {
            id: Some(2644),
            name: None,
        },
        false,
    );
    args.op = Arc::from("Spin");
    args.anchor = Some(wheel.tile);
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
    });
    s.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "I can't reach that!".into(),
        type_: 0,
        username: None,
    }]);
    assert!(with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Walk(request) => {
            assert_eq!(request.target, wheel.tile);
            assert_eq!(request.radius, 1);
        }
        HostEffect::Interaction(request) => panic!("open door recovery must walk, got {request:?}"),
    }
    assert!(
        !ledger.as_ref().unwrap().outbox.iter().any(|entry| matches!(
            &entry.effect,
            HostEffect::Interaction(InteractReq::Loc { action, .. })
                if action.eq_ignore_ascii_case("close")
        ))
    );
}
#[test]
fn west_straight_wall_door_opens_from_engine_reachable_side_without_walk() {
    let mut s = ready();
    s.seed_local_player(local_player(tile(5, 5)));
    let mut wheel = loc(2644, "Spinning wheel", "Spin");
    wheel.tile = tile(8, 5);
    let mut door = loc(1530, "Door", "Open");
    door.tile = tile(6, 5);
    door.distance = 1;
    door.layer = LocLayer::Wall;
    door.shape = 0;
    door.angle = 0;
    s.seed_locs(vec![wheel.clone(), door]);

    let mut args = reach_args(
        reach::ReachKind::Loc {
            id: Some(2644),
            name: None,
        },
        false,
    );
    args.op = Arc::from("Spin");
    args.anchor = Some(wheel.tile);
    let mut ledger = None;
    let reach = wall_door_reach_view();
    let handle = with_tick_reach(&s, &reach, &mut ledger, 1, |t| {
        t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
    });
    s.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "I can't reach that!".into(),
        type_: 0,
        username: None,
    }]);

    assert!(with_tick_reach(&s, &reach, &mut ledger, 2, |t| t
        .actions
        .poll(&handle, &mut t.cx))
    .is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(InteractReq::Loc {
            x: 6,
            z: 5,
            action,
            id: Some(1530),
            ..
        }) if action.eq_ignore_ascii_case("open")
    ));
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert!(!outbox
        .iter()
        .any(|entry| matches!(&entry.effect, HostEffect::Walk(_))));
    assert!(!outbox.iter().any(|entry| matches!(
        &entry.effect,
        HostEffect::Interaction(InteractReq::Loc { action, .. })
            if action.eq_ignore_ascii_case("close")
    )));
}

#[test]
fn non_straight_wall_door_keeps_the_approach_walk_and_propagates_failure() {
    let mut s = ready();
    s.seed_local_player(local_player(tile(5, 5)));
    let mut wheel = loc(2644, "Spinning wheel", "Spin");
    wheel.tile = tile(8, 5);
    let mut door = loc(1530, "Door", "Open");
    door.tile = tile(6, 5);
    door.distance = 1;
    door.layer = LocLayer::Wall;
    door.shape = 9;
    door.angle = 0;
    s.seed_locs(vec![wheel.clone(), door.clone()]);

    let mut args = reach_args(
        reach::ReachKind::Loc {
            id: Some(2644),
            name: None,
        },
        false,
    );
    args.op = Arc::from("Spin");
    args.anchor = Some(wheel.tile);
    let mut ledger = None;
    let reach = wall_door_reach_view();
    let handle = with_tick_reach(&s, &reach, &mut ledger, 1, |t| {
        t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
    });
    s.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "I can't reach that!".into(),
        type_: 0,
        username: None,
    }]);

    assert!(with_tick_reach(&s, &reach, &mut ledger, 2, |t| t
        .actions
        .poll(&handle, &mut t.cx))
    .is_pending());
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(request) if request.target == door.tile && request.radius == 1
    ));

    let request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 3,
            sequence: 3,
        },
        end: crate::native::WalkEnd::Failed,
        blocked: None,
        detail: None,
    });
    assert!(matches!(
        with_tick_reach(&s, &reach, &mut ledger, 3, |t| t
            .actions
            .poll(&handle, &mut t.cx)),
        Poll::Ready(Ok(false))
    ));
    assert!(
        !ledger.as_ref().unwrap().outbox.iter().any(|entry| matches!(
            &entry.effect,
            HostEffect::Interaction(InteractReq::Loc { action, .. })
                if action.eq_ignore_ascii_case("open") || action.eq_ignore_ascii_case("close")
        ))
    );
}
#[test]
fn closed_door_recovery_walks_to_an_operable_side_before_opening() {
    let mut s = ready();
    let mut wheel = loc(2644, "Spinning wheel", "Spin");
    wheel.tile = tile(3081, 3430);
    let mut door = loc(1530, "Door", "Open");
    door.tile = tile(3076, 3427);
    door.distance = 1;
    s.seed_locs(vec![wheel.clone(), door.clone()]);
    let mut args = reach_args(
        reach::ReachKind::Loc {
            id: Some(2644),
            name: None,
        },
        false,
    );
    args.op = Arc::from("Spin");
    args.anchor = Some(wheel.tile);
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
    });
    s.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "I can't reach that!".into(),
        type_: 0,
        username: None,
    }]);
    assert!(with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Walk(request) => {
            assert_eq!(request.target, door.tile);
            assert_eq!(request.radius, 1);
        }
        HostEffect::Interaction(request) => {
            panic!("door must be approached before Open, got {request:?}")
        }
    }
}

#[test]
fn vanished_clicked_loc_fails_immediately_without_retargeting_a_replacement() {
    for kind in [
        reach::ReachKind::Loc {
            id: Some(1551),
            name: Some(Arc::from("Wheat")),
        },
        reach::ReachKind::Name {
            name: Arc::from("Wheat"),
        },
    ] {
        let mut s = ready();
        let original = loc(1551, "Wheat", "Pick");
        s.seed_locs(vec![original.clone()]);
        let mut args = reach_args(kind, false);
        args.op = Arc::from("Pick");
        args.radius = 4;
        let mut ledger = None;
        let handle = with_tick(&s, &mut ledger, 1, |t| {
            t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
        });
        let mut replacement = original;
        replacement.tile.x += 1;
        s.seed_locs(vec![replacement]);
        assert!(matches!(
            with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)),
            Poll::Ready(Ok(false))
        ));
    }
}
fn with_step<R>(t: &mut NativeTick<'_>, f: impl FnOnce(&mut StepContext<'_, '_>) -> R) -> R {
    let quests = api::quest_facts::QuestCatalog::empty();
    let required_after = t.cx.evidence();
    let bank = crate::quester::bank_memo::BankMemo::default();
    f(&mut StepContext {
        tick: t,
        quests: &quests,
        progress: &[],
        required_after,
        bank: &bank,
    })
}
#[test]
fn interact_false_is_failure_not_success() {
    let s = ready();
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(false), &mut t.cx)
            .unwrap()
    });
    // Reach's observed-missing result is Ok(false), not an action error.
    let mut run = InteractRun {
        kind: egg(false).kind,
        op: Arc::from("Take"),
        tile: None,
        radius: 2,
        wait_if_missing: false,
        deadline: None,
        missing_deadline: None,
        waiting: None,
        settle_duration: Duration::from_secs(20),
        walk: None,
        reach: Some(handle),
        started: true,
    };
    assert!(matches!(
        with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))),
        Poll::Ready(Err(_))
    ));
}
#[test]
fn interact_spawn_wait_is_bounded_independently_of_the_click_deadline() {
    let s = ready();
    let mut ledger = None;
    let plan = InteractPlan {
        kind: egg(true).kind,
        op: Arc::from("Take"),
        tile: None,
        radius: 2,
        wait_if_missing: true,
        settle_ms: Some(20_000),
        ambiguous: false,
    };
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(with_tick(&s, &mut ledger, 100, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(matches!(
        with_tick(&s, &mut ledger, 203, |t| with_step(t, |cx| run.poll(cx))),
        Poll::Ready(Err(ActionError::Unavailable(reason))) if reason.contains("Egg")
    ));
}

#[test]
fn missing_spawn_recovers_when_the_observed_stack_respawns() {
    let mut s = ready();
    let mut ledger = None;
    let plan = InteractPlan {
        kind: egg(true).kind,
        op: Arc::from("Take"),
        tile: None,
        radius: 2,
        wait_if_missing: true,
        settle_ms: Some(20_000),
        ambiguous: false,
    };
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(with_tick(&s, &mut ledger, 100, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    s.seed_ground_items(vec![ground()]);
    assert!(with_tick(&s, &mut ledger, 101, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(run.waiting_for().is_none());
    assert!(
        matches!(emitted(&ledger), InteractReq::Obj {name: Some(name), action, ..} if name == "Egg" && action == "Take")
    );
    s.seed_inventory(
        vec![ItemView {
            def: def(1944, "Egg"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 0,
        }],
        28,
    );
    assert!(matches!(
        with_tick(&s, &mut ledger, 102, |t| with_step(t, |cx| run.poll(cx))),
        Poll::Ready(Ok(_))
    ));
}
#[test]
fn use_on_waits_for_visibility_and_uses_resolved_inventory_identity() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::empty();
    let path = FactKey::new("synthetic");
    let progress = test_progress();
    let areas = Default::default();
    let recipes = Default::default();
    let compile = CompileContext {
        path: &path,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        areas: &areas,
        recipes: &recipes,
        bank: None,
        bank_items: &[],
        loadouts: &crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([])),
    };
    let plan = compile_use_on(&serde_json::json!({ "item": "grain", "target": {"loc": "hopper_lumbridge"}, "radius": 8, "settle_ms": 20000 }), &compile).unwrap();
    let id = resolve_obj(&compile, "grain").unwrap();
    let loc_id = resolve_loc(&compile, "hopper_lumbridge").unwrap();
    let mut s = ready();
    s.seed_inventory(
        vec![ItemView {
            def: def(id, "Grain"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 7,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    let mut ledger = None;
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
    s.seed_locs(vec![loc(loc_id, "Hopper", "Use")]);
    assert!(with_tick(&s, &mut ledger, 3, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(
        matches!(emitted(&ledger), InteractReq::UseOn { name, x: 3167, z: 3308, source_item_id: Some(source), source_item_slot: Some(7), .. } if name == "Grain" && *source == id)
    );
}

#[test]
fn acquire_waits_for_its_inner_settle_using_the_recipe_step_chat_mark() {
    use api::snapshot::ChatLineView;
    let line = |sequence| ChatLineView {
        sequence,
        text: "You put the grain in the hopper.".into(),
        type_: 0,
        username: None,
    };
    let mut s = ready();
    s.seed_chat_lines(vec![line(2)]);
    let plan = AcquirePlan {
        recipe: Arc::from("flour"),
        steps: vec![CompiledAcquireStep {
            advances: false,
            skip_if: Arc::new(AnyPlan { items: vec![] }),
            settle: Arc::new(Message {
                needles: vec!["grain in the hopper".into()],
            }),
            plan: Arc::new(WaitPlan {
                until: Arc::new(AllPlan { items: vec![] }),
                max_ticks: 2,
            }),
        }],
    };
    let mut ledger = None;
    let mut run = with_tick(&s, &mut ledger, 5000, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&s, &mut ledger, 5001, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(with_tick(&s, &mut ledger, 5002, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    s.seed_chat_lines(vec![line(3), line(2)]);
    assert!(matches!(
        with_tick(&s, &mut ledger, 5003, |t| with_step(t, |cx| run.poll(cx))),
        Poll::Ready(Ok(_))
    ));
}

#[test]
fn a_disappearing_stack_is_not_success_until_held_count_grows() {
    let mut s = ready();
    s.seed_ground_items(vec![ground()]);
    let mut ledger = None;
    let handle = with_tick(&s, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    s.seed_ground_items(vec![]);
    assert!(with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)).is_pending());
    s.seed_inventory(
        vec![ItemView {
            def: def(1944, "Egg"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    assert!(matches!(
        with_tick(&s, &mut ledger, 3, |t| t.actions.poll(&handle, &mut t.cx)),
        Poll::Ready(Ok(true))
    ));
}

fn flour_acquire_plan() -> Arc<dyn StepPlan> {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let document = crate::quester::compile::decode_cook().unwrap();
    let path =
        crate::quester::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    path.sequences
        .iter()
        .flat_map(|sequence| &sequence.steps)
        .find(|step| step.id.0.as_ref() == "flour")
        .unwrap()
        .plan
        .clone()
}

#[test]
fn flour_acquire_resumes_at_the_bin_after_observed_grinding() {
    let line = |sequence, text: &str| ChatLineView {
        sequence,
        text: text.into(),
        type_: 0,
        username: None,
    };
    let grind = "You operate the hopper. The grain slides down the chute.";
    let consumed = "You fill a pot with the last of the flour in the bin.";
    for (lines, expected) in [
        (vec![line(2, grind)], tile(3166, 3306)),
        (vec![line(4, consumed), line(2, grind)], tile(3158, 3300)),
        (
            vec![
                line(2, grind),
                line(4, consumed),
                line(6, grind),
                line(8, "Ordinary chat"),
            ],
            tile(3166, 3306),
        ),
    ] {
        let mut s = ready();
        s.seed_inventory(
            vec![ItemView {
                def: def(1931, "Pot"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        s.seed_locs(vec![]);
        s.seed_chat_lines(lines);
        // Recovery's `near` skip predicates require observed position too.
        s.seed_local_player(api::snapshot::LocalPlayerView {
            player: api::snapshot::PlayerView {
                index: 0,
                actor: api::snapshot::ActorView {
                    name: None,
                    actions: vec![],
                    tile: tile(3209, 3215),
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
                skill_level: 0,
            },
            energy: 100,
            weight: 0,
        });
        let mut ledger = None;
        let plan = flour_acquire_plan();
        let mut run = with_tick(&s, &mut ledger, 5000, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&s, &mut ledger, 5001, |t| with_step(t, |cx| run.poll(cx))).is_pending());
        match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
            HostEffect::Walk(request) => assert_eq!(request.target, expected),
            _ => panic!("recovery must return to the bin, not harvest or grind another grain"),
        }
    }
}

fn test_progress() -> crate::quester::progress::CompiledProgress {
    crate::quester::progress::CompiledProgress {
        binding: FactKey::new("journal:cook"),
        role: None,
        colour_not_started: FactKey::new("cook:0"),
        colour_in_progress: FactKey::new("cook:1"),
        colour_complete: FactKey::new("cook:2"),
        stage_keys: Arc::from(vec![
            FactKey::new("cook:0"),
            FactKey::new("cook:1"),
            FactKey::new("cook:2"),
        ]),
        rules: Arc::from([]),
        flags: Arc::from(vec![
            crate::quester::progress::CompiledProgressFlagRule {
                flag: FactKey::new("feather"),
                all: Arc::from([]),
                any: Arc::from([Arc::<str>::from("feather")]),
                count: None,
            },
            crate::quester::progress::CompiledProgressFlagRule {
                flag: FactKey::new("crystals"),
                all: Arc::from([]),
                any: Arc::from([Arc::<str>::from("crystals")]),
                count: Some(crate::quester::progress::CountCapture { max_digits: 9 }),
            },
        ]),
        monotonic: false,
    }
}

fn compile_context_test<R>(f: impl FnOnce(&CompileContext<'_>) -> R) -> R {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = FactKey::new("cook");
    let progress = test_progress();
    f(&CompileContext {
        path: &path,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        areas: &Default::default(),
        recipes: &Default::default(),
        bank: None,
        bank_items: &[],
        loadouts: &crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([])),
    })
}

#[test]
fn resolved_npc_alias_matches_type_and_sends_display_and_observed_index() {
    compile_context_test(|cx| {
        let row = cx.selected.npc_by_config("king_bolren").unwrap();
        let mut s = ready();
        s.seed_npcs(vec![api::snapshot::NpcView {
            index: 42,
            r#type: Some(row.id as usize),
            name: row.display.clone(),
            actions: vec![Some("Talk-to".into())],
            tile: tile(2542, 3170),
            distance: 1,
            animation: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: tile(2542, 3170),
            x: 0,
            z: 0,
            yaw: 0,
        }]);
        let present = compile_npc_present(&serde_json::json!({"npc":"king_bolren"}), cx).unwrap();
        let mut ledger = None;
        with_tick(&s, &mut ledger, 1, |t| {
            assert_eq!(
                present.evaluate(&PredicateContext {
                    cx: &t.cx,
                    quests: cx.quests,
                    progress: &[],
                    required_after: t.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                    bank: &crate::quester::bank_memo::BankMemo::default(),
                }),
                Truth::True
            );
        });
        for (compile, args) in [
            (
                compile_talk as super::super::compile::CompileStep,
                serde_json::json!({"npc":"king_bolren"}),
            ),
            (
                compile_interact as super::super::compile::CompileStep,
                serde_json::json!({"target":{"npc":"king_bolren"},"op":"Talk-to"}),
            ),
        ] {
            let plan = compile(&args, cx).unwrap();
            let mut ledger = None;
            let mut run = with_tick(&s, &mut ledger, 1, |t| {
                with_step(t, |cx| plan.begin(cx).unwrap())
            });
            let _ = with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx)));
            assert!(
                matches!(emitted(&ledger), InteractReq::Npc {name, index:Some(42), ..} if name == "King Bolren")
            );
        }
    });
}

#[test]
fn dialogue_approaches_a_distant_npc_before_talking() {
    compile_context_test(|cx| {
        let row = cx.selected.npc_by_config("king_bolren").unwrap();
        let npc = |distance| api::snapshot::NpcView {
            index: 42,
            r#type: Some(row.id as usize),
            name: row.display.clone(),
            actions: vec![Some("Talk-to".into())],
            tile: tile(2542, 3170),
            distance,
            animation: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: tile(2542, 3170),
            x: 0,
            z: 0,
            yaw: 0,
        };
        let mut far = ready();
        far.seed_npcs(vec![npc(8)]);
        let plan = compile_talk(&serde_json::json!({"npc":"king_bolren"}), cx).unwrap();
        let mut ledger = None;
        let mut run = with_tick(&far, &mut ledger, 1, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&far, &mut ledger, 2, |t| {
            with_step(t, |cx| run.poll(cx))
        })
        .is_pending());
        match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
            HostEffect::Walk(request) => {
                assert_eq!(request.target, tile(2542, 3170));
                assert_eq!(request.radius, 1);
            }
            HostEffect::Interaction(_) => panic!("talked before approaching the NPC"),
        }

        let mut near = ready();
        near.seed_npcs(vec![npc(1)]);
        assert!(with_tick(&near, &mut ledger, 3, |t| {
            with_step(t, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(emitted(&ledger), InteractReq::Npc {name, index:Some(42), ..} if name == "King Bolren")
        );
    });
}

#[test]
fn use_on_approaches_a_distant_npc_before_using_the_item() {
    compile_context_test(|cx| {
        let row = cx.selected.npc_by_config("sheepunsheered").unwrap();
        let npc = |distance| api::snapshot::NpcView {
            index: 42,
            r#type: Some(row.id as usize),
            name: row.display.clone(),
            actions: vec![Some("Shear".into())],
            tile: tile(3200, 3276),
            distance,
            animation: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: tile(3200, 3276),
            x: 0,
            z: 0,
            yaw: 0,
        };
        let mut far = ready();
        far.seed_npcs(vec![npc(4)]);
        let shears = resolve_obj(cx, "shears").unwrap();
        far.seed_inventory(
            vec![ItemView {
                def: def(shears, "Shears"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 7,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        let plan = compile_use_on(
            &serde_json::json!({
                "item": "shears",
                "target": {"npc": "sheepunsheered"},
                "radius": 8
            }),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&far, &mut ledger, 1, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&far, &mut ledger, 2, |t| {
            with_step(t, |cx| run.poll(cx))
        })
        .is_pending());
        match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
            HostEffect::Walk(request) => {
                assert_eq!(request.target, tile(3200, 3276));
                assert_eq!(request.radius, 1);
            }
            HostEffect::Interaction(_) => panic!("used the item before approaching the NPC"),
        }
    });
}

#[test]
fn use_on_reports_fresh_server_escape_without_waiting_for_product_timeout() {
    use api::snapshot::ChatLineView;

    for reply_in_ack_frame in [false, true] {
        compile_context_test(|cx| {
            let row = cx.selected.npc_by_config("sheepunsheered").unwrap();
            let mut snapshot = ready();
            snapshot.seed_npcs(vec![api::snapshot::NpcView {
                index: 42,
                r#type: Some(row.id as usize),
                name: row.display.clone(),
                actions: vec![Some("Shear".into())],
                tile: tile(3200, 3276),
                distance: 1,
                animation: -1,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                health: 1,
                total_health: 1,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
                level: 1,
                size: 1,
                network: tile(3200, 3276),
                x: 0,
                z: 0,
                yaw: 0,
            }]);
            let shears = resolve_obj(cx, "shears").unwrap();
            snapshot.seed_inventory(
                vec![ItemView {
                    def: def(shears, "Shears"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 7,
                    count: 1,
                    actions: vec![],
                    component_id: 3214,
                }],
                28,
            );
            let message = |sequence, type_, username| ChatLineView {
                sequence,
                type_,
                username,
                text: "The sheep manages to get away from you!".into(),
            };
            snapshot.seed_chat_lines(vec![message(5, 0, None)]);
            let plan = compile_use_on(
            &serde_json::json!({
                "item": "shears",
                "target": {"npc": "sheepunsheered"},
                "radius": 8,
                "product": "wool",
                "no_product": {"Fact": {"kind": "message", "version": 1, "args": {"any": ["The sheep manages to get away from you!"]}}},
                "settle_ms": 240_000
            }),
            cx,
        )
        .unwrap();
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |t| {
                with_step(t, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |t| {
                with_step(t, |cx| run.poll(cx))
            })
            .is_pending());
            let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
            // A previous attempt's server line arrives while this click is queued,
            // before the host confirms it was sent. It must not belong to this run.
            snapshot.seed_chat_lines(vec![message(6, 0, None)]);
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 3,
                        sequence: 3,
                    },
                    accepted: true,
                    chat_since: 6,
                },
            );
            if reply_in_ack_frame {
                snapshot.seed_chat_lines(vec![message(8, 0, None)]);
                assert!(matches!(
                with_tick(&snapshot, &mut ledger, 3, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                }),
                Poll::Ready(Err(ActionError::Failed(_)))
            ), "feedback already present when acceptance is polled must not wait for the timeout");
                return;
            }
            assert!(
                with_tick(&snapshot, &mut ledger, 3, |t| {
                    with_step(t, |cx| run.poll(cx))
                })
                .is_pending(),
                "old server feedback must not fail a new attempt"
            );
            snapshot.seed_chat_lines(vec![message(7, 2, Some("other player".into()))]);
            assert!(
                with_tick(&snapshot, &mut ledger, 4, |t| {
                    with_step(t, |cx| run.poll(cx))
                })
                .is_pending(),
                "player chat is not authoritative action feedback"
            );
            snapshot.seed_chat_lines(vec![message(8, 0, None)]);
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 5, |t| {
                    with_step(t, |cx| run.poll(cx))
                }),
                Poll::Ready(Err(ActionError::Failed(_)))
            ));
        });
    }
}

#[test]
fn use_on_zero_wool_survives_interleaved_escape_rounds_without_step_failure() {
    compile_context_test(|cx| {
        let shears = ItemView {
            def: def(resolve_obj(cx, "shears").unwrap(), "Shears"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        };
        let wool = ItemView {
            def: def(resolve_obj(cx, "wool").unwrap(), "Wool"),
            slot: 1,
            count: 0,
            ..shears.clone()
        };
        let mut snapshot = ready();
        snapshot.seed_inventory(vec![shears.clone()], 28);
        // The target is held to isolate round settlement from movement. This
        // drives the same compiled UseOn machine used by the sheep Path.
        let plan = compile_use_on(
            &serde_json::json!({
                "item": "shears", "target": {"item": "shears"},
                "until": {"obj": "wool", "qty": 20}, "settle_ms": 240_000,
                "no_product": {"Fact": {"kind": "message", "version": 1, "args": {
                    "any": ["The sheep manages to get away from you!"]
                }}}
            }),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        let mut tick = 2;
        let mut count = 0;
        for round in 0..27 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
            tick += 1;
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick,
                        sequence: tick,
                    },
                    accepted: true,
                    chat_since: snapshot
                        .chat_lines()
                        .first()
                        .map_or(0, |line| line.sequence),
                },
            );
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            tick += 1;
            if round % 4 == 0 {
                snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
                    sequence: tick as i32,
                    type_: 0,
                    username: None,
                    text: "The sheep manages to get away from you!".into(),
                }]);
            } else {
                count += 1;
                snapshot.seed_inventory(
                    vec![
                        shears.clone(),
                        ItemView {
                            count,
                            ..wool.clone()
                        },
                    ],
                    28,
                );
            }
            let result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            if count == 20 {
                assert!(matches!(result, Poll::Ready(Ok(_))));
            } else {
                assert!(
                    result.is_pending(),
                    "round {round} must stay in the bounded until loop, not park"
                );
            }
            tick += 1;
        }
        assert_eq!(count, 20);
    });
}

#[test]
fn use_on_negative_feedback_is_authored_and_not_a_sheep_special_case() {
    compile_context_test(|cx| {
        let mut snapshot = ready();
        snapshot.seed_inventory(
            vec![ItemView {
                def: def(resolve_obj(cx, "shears").unwrap(), "Shears"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        let plan = compile_use_on(&serde_json::json!({
            "item": "shears", "target": {"item": "shears"}, "product": "wool",
            "no_product": {"Fact": {"kind": "message", "version": 1, "args": {"any": ["Nothing is produced."]}}}
        }), cx).unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
        ledger.as_mut().unwrap().complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 3,
                    sequence: 3,
                },
                accepted: true,
                chat_since: snapshot
                    .chat_lines()
                    .first()
                    .map_or(0, |line| line.sequence),
            },
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            sequence: 4,
            type_: 0,
            username: None,
            text: "The sheep manages to get away from you!".into(),
        }]);
        assert!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "unconfigured sheep feedback cannot settle a generic step"
        );
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            sequence: 5,
            type_: 0,
            username: None,
            text: "Nothing is produced.".into(),
        }]);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    });
}

fn wool_menu_client() -> client::client::Client {
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-no-production-cache".into(),
        members: true,
        lowmem: false,
    });
    let cache = Arc::get_mut(&mut client.cache).unwrap();
    cache.objs.resize(1738, client::config::ObjType::default());
    cache.objs[1737] = client::config::ObjType {
        id: 1737,
        name: "Wool".into(),
        ..Default::default()
    };
    client.set_iface(
        2100,
        IfType {
            id: 2100,
            layer_id: 2100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2110, 2120]),
            ..Default::default()
        },
    );
    client.set_iface(
        2110,
        IfType {
            id: 2110,
            layer_id: 2100,
            r#type: ComponentType::TYPE_MODEL,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        2110,
        IfTypeMut {
            model1_type: 4,
            model1_id: 1737,
            ..Default::default()
        },
    );
    client.set_iface(
        2120,
        IfType {
            id: 2120,
            layer_id: 2100,
            r#type: ComponentType::TYPE_TEXT,
            button_text: "Make X".into(),
            ..Default::default()
        },
    );
    client.set_iface_mut(
        2120,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
    client.chat_modal_id = 2100;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    client
}

#[test]
fn make_selects_the_input_menu_row_and_settles_on_the_output() {
    use crate::native_production::{MakeMachine, MakeRequest};
    use client::io::ServerProt;
    let mut client = wool_menu_client();

    let mut snapshot = ready();
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1737, "Wool"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 20,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<MakeMachine>(
                MakeRequest {
                    product_id: 1759,
                    menu_id: 1737,
                    qty: 20,
                    make_x: true,
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::IfButton { component_id: 2120 }
    ));
    let before = ledger.as_ref().unwrap().outbox.len();
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), before);
    client.dialog_input_open = true;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::AnswerCount { value: 20 }
    ));
    client.dialog_input_open = false;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1737, "Wool"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 20,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1759, "Ball of wool"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 20,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    client.chat_modal_id = -1;
    client.bump_gens(ServerProt::IF_CLOSE);
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 7, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(crate::native_production::MakeReceipt { held: 20 }))
    ));
}

fn counted_sheep_progress(
    selected: &api::game_data::SelectedGameData,
    evidence: EvidenceStamp,
    count: u32,
) -> QuestProgress {
    QuestProgress {
        quest: FactKey::new("sheep"),
        stage: Knowledge::Known(FactKey::new("sheep:1")),
        complete: Truth::False,
        signals: Arc::from([]),
        flags: Arc::from([ProgressFlag {
            flag: FactKey::new("sheep:balls_to_go"),
            truth: Truth::True,
            count: Some(count),
        }]),
        evidence,
        binding: FactKey::new("journal:sheep"),
        role: None,
        rule: Knowledge::Known(FactKey::new("sheep:1")),
        pin: selected.selected_pin().unwrap(),
    }
}

fn with_sheep_step<R>(
    tick: &mut NativeTick<'_>,
    remaining: u32,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let evidence = tick.cx.evidence();
    let progress = [counted_sheep_progress(&selected, evidence, remaining)];
    let bank = crate::quester::bank_memo::BankMemo::default();
    f(&mut StepContext {
        tick,
        quests: &quests,
        progress: &progress,
        required_after: evidence,
        bank: &bank,
    })
}

#[test]
fn sheep_partial_hand_in_use_on_stops_at_only_the_missing_raw_count() {
    compile_context_test(|cx| {
        let mut document: crate::quester::path::PathDocument =
            serde_json::from_str(crate::quester::compile::SHEEP_JSON).unwrap();
        let shear = document.roles[0].sequences[1]
            .steps
            .iter_mut()
            .find(|step| step.id.0.as_ref() == "shear")
            .unwrap();
        // Keep the released quantity; isolate production from NPC movement.
        shear.args["target"] = serde_json::json!({"item": "shears"});
        shear.args.as_object_mut().unwrap().remove("anchor");
        let path =
            crate::quester::compile::compile_uncached_for_test(&document, cx.selected, cx.quests)
                .unwrap();
        let plan = &path.sequences[1]
            .steps
            .iter()
            .find(|step| step.id.0.as_ref() == "shear")
            .unwrap()
            .plan;
        let item = |id, name, slot, count| ItemView {
            def: def(id, name),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count,
            actions: vec![],
            component_id: 3214,
        };
        let mut snapshot = ready();
        snapshot.seed_inventory(
            vec![
                item(1735, "Shears", 0, 1),
                item(1737, "Wool", 1, 4),
                item(1759, "Ball of wool", 2, 3),
            ],
            28,
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_sheep_step(tick, 8, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_sheep_step(tick, 8, |cx| run.poll(cx))
        })
        .is_pending());
        let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
        ledger.as_mut().unwrap().complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 3,
                    sequence: 3,
                },
                accepted: true,
                chat_since: snapshot
                    .chat_lines()
                    .first()
                    .map_or(0, |line| line.sequence),
            },
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_sheep_step(tick, 8, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_inventory(
            vec![
                item(1735, "Shears", 0, 1),
                item(1737, "Wool", 1, 5),
                item(1759, "Ball of wool", 2, 3),
            ],
            28,
        );
        assert!(
            matches!(
                with_tick(&snapshot, &mut ledger, 4, |tick| {
                    with_sheep_step(tick, 8, |cx| run.poll(cx))
                }),
                Poll::Ready(Ok(_))
            ),
            "12 handed in and 3 held needs only 5 raw wool"
        );
    });
}

#[test]
fn sheep_product_progress_selects_shear_spin_then_hand_in() {
    compile_context_test(|cx| {
        let document = serde_json::from_str(crate::quester::compile::SHEEP_JSON).unwrap();
        let path =
            crate::quester::compile::compile_uncached_for_test(&document, cx.selected, cx.quests)
                .unwrap();
        let mut bank = crate::quester::bank_memo::BankMemo::default();
        bank.update(&crate::native_bank::BankReceipt {
            counts: vec![],
            complete: true,
        });
        for (id, name, count, expected) in [
            (1737, "Wool", 19, "shear"),
            (1737, "Wool", 20, "spin"),
            (1759, "Ball of wool", 20, "hand-in"),
        ] {
            let mut snapshot = ready();
            snapshot.seed_inventory(
                vec![
                    ItemView {
                        def: def(id, name),
                        container: ItemContainer::Inventory,
                        action_family: ItemActionFamily::Held,
                        slot: 0,
                        count,
                        actions: vec![],
                        component_id: 3214,
                    },
                    ItemView {
                        def: def(1735, "Shears"),
                        container: ItemContainer::Inventory,
                        action_family: ItemActionFamily::Held,
                        slot: 1,
                        count: 1,
                        actions: vec![],
                        component_id: 3214,
                    },
                ],
                28,
            );
            with_tick(&snapshot, &mut None, 1, |tick| {
                let progress = [counted_sheep_progress(cx.selected, tick.cx.evidence(), 20)];
                let context = PredicateContext {
                    cx: &tick.cx,
                    quests: cx.quests,
                    progress: &progress,
                    required_after: tick.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                    bank: &bank,
                };
                let crate::quester::select::SelectionDecision::Selected(selection) =
                    crate::quester::select::select(&path, 1, &context)
                else {
                    panic!("expected a known product-progress step");
                };
                assert_eq!(selection.step.id.0.as_ref(), expected);
            });
        }
    });
}

#[test]
fn wait_observes_until_and_expires_at_the_authored_bound() {
    compile_context_test(|cx| {
        let args = serde_json::json!({"until":{"Fact":{"kind":"has_item","version":1,"args":{"obj":"egg"}}},"max_ticks":3});
        let plan = compile_wait(&args, cx).unwrap();
        let mut s = ready();
        let mut ledger = None;
        let mut run = with_tick(&s, &mut ledger, 1, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        for tick in [2, 3] {
            assert!(
                with_tick(&s, &mut ledger, tick, |t| with_step(t, |cx| run.poll(cx))).is_pending()
            );
        }
        assert!(matches!(
            with_tick(&s, &mut ledger, 4, |t| with_step(t, |cx| run.poll(cx))),
            Poll::Ready(Err(_))
        ));
        let mut run = with_tick(&s, &mut ledger, 5, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        s.seed_inventory(
            vec![ItemView {
                def: def(1944, "Egg"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        assert!(matches!(
            with_tick(&s, &mut ledger, 6, |t| with_step(t, |cx| run.poll(cx))),
            Poll::Ready(Ok(_))
        ));
        for args in [
            serde_json::json!({}),
            serde_json::json!({"until":{"All":[]},"max_ticks":0}),
            serde_json::json!({"until":{"All":[]},"max_ticks":"three"}),
        ] {
            assert!(compile_wait(&args, cx).is_err());
        }
    });
}

#[test]
fn wait_message_until_requires_an_event_after_begin() {
    compile_context_test(|cx| {
        let plan = compile_wait(&serde_json::json!({"until":{"Fact":{"kind":"message","version":1,"args":{"any":["ready"]}}},"max_ticks":4}),cx).unwrap();
        let mut s = ready();
        s.seed_chat_lines(vec![ChatLineView {
            sequence: 1,
            text: "ready".into(),
            type_: 0,
            username: None,
        }]);
        let mut ledger = None;
        let mut run = with_tick(&s, &mut ledger, 1, |t| {
            with_step(t, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))).is_pending());
        s.seed_chat_lines(vec![ChatLineView {
            sequence: 2,
            text: "ready".into(),
            type_: 0,
            username: None,
        }]);
        assert!(matches!(
            with_tick(&s, &mut ledger, 3, |t| with_step(t, |cx| run.poll(cx))),
            Poll::Ready(Ok(_))
        ));
    });
}

#[test]
fn public_chat_cannot_settle_or_set_message_state() {
    compile_context_test(|cx| {
        let message =
            compile_message(&serde_json::json!({"any":["grain in the hopper"]}), cx).unwrap();
        let state = compile_message_state(
            &serde_json::json!({"set":["grain in the hopper"],"clear":["hopper is empty"]}),
            cx,
        )
        .unwrap();
        let mut s = ready();
        s.seed_chat_lines(vec![ChatLineView {
            sequence: 10,
            text: "grain in the hopper".into(),
            type_: 2,
            username: Some("mallory".into()),
        }]);
        with_tick(&s, &mut None, 1, |t| {
            let pred = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            assert_eq!(message.evaluate(&pred), Truth::False);
            assert_eq!(state.evaluate(&pred), Truth::False);
        });
        assert!(compile_message(&serde_json::json!({"any":[""]}), cx).is_err());
    });
}

#[test]
fn preloaded_hopper_with_spare_grain_operates_instead_of_refilling() {
    let mut s = ready();
    s.seed_inventory(
        vec![
            ItemView {
                def: def(1931, "Pot"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            },
            ItemView {
                def: def(1947, "Grain"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 1,
                count: 1,
                actions: vec![],
                component_id: 3214,
            },
        ],
        28,
    );
    s.seed_chat_lines(vec![ChatLineView {
        sequence: 2,
        text: "There is already grain in the hopper.".into(),
        type_: 0,
        username: None,
    }]);
    s.seed_locs(vec![
        loc(2718, "Hopper controls", "Operate"),
        loc(2714, "Hopper", "Use"),
    ]);
    let plan = compile_context_test(|cx| {
        let mut document = crate::quester::compile::decode_cook().unwrap();
        // Isolate hopper recovery from the preceding navigation leg.
        document
            .quest
            .as_mut()
            .unwrap()
            .acquire
            .get_mut("acquire:flour")
            .unwrap()[2]
            .skip_if = PredicateDocument::Fact {
            kind: "has_item".into(),
            version: 1,
            args: serde_json::json!({"obj":"grain"}),
        };
        crate::quester::compile::compile_uncached_for_test(&document, cx.selected, cx.quests)
            .unwrap()
            .sequences[1]
            .steps[2]
            .plan
            .clone()
    });
    let mut ledger = None;
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    for tick in 2..5 {
        let _ = with_tick(&s, &mut ledger, tick, |t| with_step(t, |cx| run.poll(cx)));
    }
    assert!(matches!(emitted(&ledger),InteractReq::Loc {action,..} if action == "Operate"));
}

#[test]
fn loaded_hopper_without_spare_grain_reoperates_without_harvesting() {
    let mut s = ready();
    s.seed_inventory(
        vec![ItemView {
            def: def(1931, "Pot"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    s.seed_chat_lines(vec![ChatLineView {
        sequence: 2,
        text: "You put the grain in the hopper.".into(),
        type_: 0,
        username: None,
    }]);
    s.seed_locs(vec![loc(2718, "Hopper controls", "Operate")]);
    let plan = compile_context_test(|cx| {
        let document = crate::quester::compile::decode_cook().unwrap();
        let recipe = &document.quest.as_ref().unwrap().acquire["acquire:flour"];
        with_tick(&s, &mut None, 1, |t| {
            let pred = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            for index in [1, 2] {
                assert_eq!(
                    compile_predicate(&recipe[index].skip_if, cx)
                        .unwrap()
                        .evaluate(&pred),
                    Truth::True,
                    "{} must not repeat while the hopper is loaded",
                    recipe[index].id.0
                );
            }
        });
        crate::quester::compile::compile_uncached_for_test(&document, cx.selected, cx.quests)
            .unwrap()
            .sequences[1]
            .steps[2]
            .plan
            .clone()
    });
    let mut ledger = None;
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    for tick in 2..5 {
        let _ = with_tick(&s, &mut ledger, tick, |t| with_step(t, |cx| run.poll(cx)));
    }
    assert!(matches!(emitted(&ledger), InteractReq::Loc { action, .. } if action == "Operate"));
}

#[test]
fn real_empty_hopper_message_clears_the_loaded_hint() {
    compile_context_test(|cx| {
        let document = crate::quester::compile::decode_cook().unwrap();
        let hopper = &document.quest.as_ref().unwrap().acquire["acquire:flour"][3];
        let PredicateDocument::Any(skips) = &hopper.skip_if else {
            panic!("hopper skip must accept independent observations");
        };
        let loaded = compile_predicate(&skips[1], cx).unwrap();
        let mut s = ready();
        s.seed_chat_lines(vec![
            ChatLineView {
                sequence: 2,
                text: "There is already grain in the hopper.".into(),
                type_: 0,
                username: None,
            },
            ChatLineView {
                sequence: 3,
                text: "You operate the empty hopper. Nothing interesting happens.".into(),
                type_: 0,
                username: None,
            },
        ]);
        with_tick(&s, &mut None, 1, |t| {
            let pred = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            assert_eq!(loaded.evaluate(&pred), Truth::False);
        });
    });
}

#[test]
fn progress_predicates_require_known_same_run_evidence() {
    compile_context_test(|cx| {
        let stage = compile_predicate(
            &PredicateDocument::Fact {
                kind: "stage_in".into(),
                version: 1,
                args: serde_json::json!({"quest":"cook","any":["cook:1"]}),
            },
            cx,
        )
        .unwrap();
        let flag = compile_predicate(
            &PredicateDocument::Fact {
                kind: "flag".into(),
                version: 1,
                args: serde_json::json!({"quest":"cook","flag":"feather"}),
            },
            cx,
        )
        .unwrap();
        let snapshot = ready();
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |t| {
            let unknown = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            assert_eq!(stage.evaluate(&unknown), Truth::Unknown);
            assert_eq!(flag.evaluate(&unknown), Truth::Unknown);

            let evidence = t.cx.evidence();
            let pin = cx.selected.selected_pin().unwrap();
            let progress = [QuestProgress {
                quest: FactKey::new("cook"),
                stage: Knowledge::Known(FactKey::new("cook:1")),
                complete: Truth::False,
                signals: Arc::from(Vec::<api::selected::SignalRange>::new()),
                flags: Arc::from(vec![ProgressFlag {
                    flag: FactKey::new("feather"),
                    truth: Truth::True,
                    count: None,
                }]),
                evidence,
                binding: FactKey::new("journal:cook"),
                role: None,
                rule: Knowledge::Known(FactKey::new("cook:1")),
                pin,
            }];
            let known = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &progress,
                required_after: EvidenceStamp {
                    tick: evidence.tick + 10,
                    ..evidence
                },
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            assert_eq!(stage.evaluate(&known), Truth::True);
            assert_eq!(flag.evaluate(&known), Truth::True);
        });
    });
}

#[test]
fn progress_predicates_validate_references_and_count_requirements() {
    compile_context_test(|cx| {
        let fact = |kind: &str, args: serde_json::Value| PredicateDocument::Fact {
            kind: kind.into(),
            version: 1,
            args,
        };
        let error_code = |document: &PredicateDocument| {
            compile_predicate(document, cx)
                .err()
                .expect("predicate must be rejected")
                .code
        };

        assert_eq!(
            error_code(&fact(
                "stage_in",
                serde_json::json!({"quest":"cook","any":[]}),
            ))
            .as_ref(),
            "invalid-args"
        );
        assert_eq!(
            error_code(&fact(
                "stage_in",
                serde_json::json!({"quest":"cook","any":["cook:99"]}),
            ))
            .as_ref(),
            "unresolved-progress-stage"
        );
        assert_eq!(
            error_code(&fact("flag", serde_json::json!({"quest":"cook","flag":""}),)).as_ref(),
            "invalid-args"
        );
        assert_eq!(
            error_code(&fact(
                "flag",
                serde_json::json!({"quest":"cook","flag":"missing"}),
            ))
            .as_ref(),
            "unresolved-progress-flag"
        );
        assert_eq!(
            error_code(&fact(
                "stage_in",
                serde_json::json!({"quest":"imp","any":["imp:1"]}),
            ))
            .as_ref(),
            "foreign-progress-quest"
        );
        assert_eq!(
            error_code(&fact(
                "flag",
                serde_json::json!({"quest":"imp","flag":"feather"}),
            ))
            .as_ref(),
            "foreign-progress-quest"
        );
        assert_eq!(
            error_code(&fact(
                "flag",
                serde_json::json!({"quest":"cook","flag":"feather","count":1}),
            ))
            .as_ref(),
            "unresolved-progress-count"
        );
        assert_eq!(
            error_code(&fact(
                "flag",
                serde_json::json!({"quest":"cook","flag":"feather","at_least":1}),
            ))
            .as_ref(),
            "unresolved-progress-count"
        );
        assert!(compile_predicate(
            &fact(
                "flag",
                serde_json::json!({"quest":"cook","flag":"crystals","count":3}),
            ),
            cx
        )
        .is_ok());
        assert!(compile_predicate(
            &fact(
                "flag",
                serde_json::json!({"quest":"cook","flag":"crystals","at_least":2}),
            ),
            cx
        )
        .is_ok());
    });
}

#[test]
fn progress_predicates_cover_negation_counts_and_unknown_stage() {
    compile_context_test(|cx| {
        let fact = |args| PredicateDocument::Fact {
            kind: "flag".into(),
            version: 1,
            args,
        };
        let not_set = compile_predicate(
            &fact(serde_json::json!({"quest":"cook","flag":"feather","is":false})),
            cx,
        )
        .unwrap();
        let exact = compile_predicate(
            &fact(serde_json::json!({"quest":"cook","flag":"crystals","count":3})),
            cx,
        )
        .unwrap();
        let minimum = compile_predicate(
            &fact(serde_json::json!({"quest":"cook","flag":"crystals","at_least":2})),
            cx,
        )
        .unwrap();
        let stage = compile_predicate(
            &PredicateDocument::Fact {
                kind: "stage_in".into(),
                version: 1,
                args: serde_json::json!({"quest":"cook","any":["cook:1"]}),
            },
            cx,
        )
        .unwrap();
        let snapshot = ready();
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |t| {
            let evidence = t.cx.evidence();
            let pin = cx.selected.selected_pin().unwrap();
            let progress = [QuestProgress {
                quest: FactKey::new("cook"),
                stage: Knowledge::Known(FactKey::new("cook:1")),
                complete: Truth::False,
                signals: Arc::from(Vec::<api::selected::SignalRange>::new()),
                flags: Arc::from(vec![
                    ProgressFlag {
                        flag: FactKey::new("feather"),
                        truth: Truth::True,
                        count: None,
                    },
                    ProgressFlag {
                        flag: FactKey::new("crystals"),
                        truth: Truth::True,
                        count: Some(3),
                    },
                ]),
                evidence,
                binding: FactKey::new("journal:cook"),
                role: None,
                rule: Knowledge::Known(FactKey::new("cook:1")),
                pin: Arc::clone(&pin),
            }];
            let known = PredicateContext {
                cx: &t.cx,
                quests: cx.quests,
                progress: &progress,
                required_after: evidence,
                chat_since: 0,
                outcome: None,
                bank: &crate::quester::bank_memo::BankMemo::default(),
            };
            assert_eq!(not_set.evaluate(&known), Truth::False);
            assert_eq!(exact.evaluate(&known), Truth::True);
            assert_eq!(minimum.evaluate(&known), Truth::True);
            assert_eq!(stage.evaluate(&known), Truth::True);

            let unknown_stage = [QuestProgress {
                stage: Knowledge::Unknown(api::selected::Gap {
                    code: Arc::from("missing-stage"),
                    sources: Arc::from([]),
                }),
                ..progress[0].clone()
            }];
            let unknown = PredicateContext {
                progress: &unknown_stage,
                ..known
            };
            assert_eq!(stage.evaluate(&unknown), Truth::Unknown);
        });
    });
}

#[test]
fn dialogue_end_requires_game_tick_quiet_not_elapsed_host_time() {
    use super::dialogue::{Dialogue, DialogueArgs};
    let mut snapshot = ready();
    snapshot.seed_chat_modal(4882, vec![]);
    seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<Dialogue>(
                DialogueArgs {
                    id: 0,
                    npc: Arc::from("Aubury"),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut t.cx,
            )
            .unwrap()
    });
    snapshot.seed_chat_modal(-1, vec![]);
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.active_now = Duration::from_secs(30);
        assert!(
            t.actions.poll(&handle, &mut t.cx).is_pending(),
            "host delay without game progress cannot end a conversation"
        );
    });
    snapshot.seed_chat_modal(4893, vec![]);
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    snapshot.seed_chat_modal(-1, vec![]);
    for tick in 5..9 {
        with_tick(&snapshot, &mut ledger, tick, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
        });
    }
    with_tick(&snapshot, &mut ledger, 9, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
        ));
    });
}

#[test]
fn dialogue_closed_bulk_handover_waits_for_inventory_quiet_and_final_page() {
    use super::dialogue::{Dialogue, DialogueArgs};
    let mut snapshot = ready();
    snapshot.seed_chat_modal(4893, vec!["Give 'em here then.".into()]);
    snapshot.seed_chat_options(vec![], 4899);
    let mut wool = ItemView {
        def: def(1759, "Ball of wool"),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 20,
        actions: vec![],
        component_id: 3214,
    };
    snapshot.seed_inventory(vec![wool.clone()], 28);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                DialogueArgs {
                    id: 0,
                    npc: Arc::from("Fred the Farmer"),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    for tick in 3..=22 {
        wool.count = 23 - tick as i32;
        snapshot.seed_inventory(vec![wool.clone()], 28);
        assert!(
            with_tick(&snapshot, &mut ledger, tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "bulk handover is still active at tick {tick}"
        );
    }
    ledger.as_mut().unwrap().outbox.clear();
    snapshot.seed_chat_modal(4893, vec!["I guess I'd better pay you then.".into()]);
    snapshot.seed_chat_options(vec![], 4899);
    assert!(with_tick(&snapshot, &mut ledger, 23, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(emitted(&ledger), InteractReq::ContinueDialog));
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    for tick in 24..29 {
        assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 29, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
    ));
}

#[test]
fn dialogue_unrelated_inventory_churn_cannot_extend_closed_gap_forever() {
    use super::dialogue::{Dialogue, DialogueArgs};
    let mut snapshot = ready();
    let item = ItemView {
        def: def(1759, "Ball of wool"),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![],
        component_id: 3214,
    };
    snapshot.seed_inventory(vec![item.clone()], 28);
    snapshot.seed_chat_modal(4893, vec![]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                DialogueArgs {
                    id: 0,
                    npc: Arc::from("Fred the Farmer"),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut tick.cx,
            )
            .unwrap()
    });
    snapshot.seed_chat_modal(-1, vec![]);
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    // One starting unit plus four slack updates may re-arm; later unrelated
    // updates keep happening but must not postpone the fifth quiet deadline.
    for game_tick in 3..11 {
        snapshot.seed_inventory(
            vec![ItemView {
                slot: (game_tick % 2) as i32,
                ..item.clone()
            }],
            28,
        );
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    snapshot.seed_inventory(vec![ItemView { slot: 1, ..item }], 28);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 11, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
    ));
}

pub(crate) fn seed_dialogue_combat(snapshot: &mut GameSnapshot, in_combat: bool) {
    snapshot.seed_local_player(api::snapshot::LocalPlayerView {
        player: api::snapshot::PlayerView {
            index: 0,
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
                tile: tile(3253, 3401),
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
                in_combat,
            },
            combat_level: 3,
            skill_level: 0,
        },
        energy: 100,
        weight: 0,
    });
}

#[test]
fn dialogue_hostile_close_never_reports_success() {
    use super::dialogue::{Dialogue, DialogueArgs};
    let mut snapshot = ready();
    snapshot.seed_chat_modal(4882, vec![]);
    seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<Dialogue>(
                DialogueArgs {
                    id: 0,
                    npc: Arc::from("Aubury"),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut t.cx,
            )
            .unwrap()
    });
    snapshot.seed_chat_modal(-1, vec![]);
    seed_dialogue_combat(&mut snapshot, true);
    let mut result = Poll::Pending;
    for tick in 2..=6 {
        result = with_tick(&snapshot, &mut ledger, tick, |t| {
            t.actions.poll(&handle, &mut t.cx)
        });
        if result.is_ready() {
            break;
        }
    }
    assert!(
        matches!(
            result,
            Poll::Ready(Ok(
                crate::dialogue_outcome::DialogueOutcome::CombatInterrupted
            ))
        ),
        "a hostile close, including a zero-damage hit, cannot complete dialogue: {result:?}"
    );
}

#[test]
fn dialogue_combat_interruption_covers_open_and_page_acknowledgements() {
    use super::dialogue::{Dialogue, DialogueArgs};
    use crate::dialogue_outcome::DialogueOutcome;
    for continue_component in [None, Some(4883), Some(-1)] {
        let mut snapshot = ready();
        seed_dialogue_combat(&mut snapshot, false);
        if let Some(component) = continue_component {
            snapshot.seed_chat_modal(4882, vec![]);
            snapshot.seed_chat_options(
                if component == -1 {
                    vec![api::snapshot::ChatOptionView {
                        component_id: 1,
                        text: "I have a package for you.".into(),
                    }]
                } else {
                    vec![]
                },
                component,
            );
        }
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
            t.actions
                .begin::<Dialogue>(
                    DialogueArgs {
                        id: 0,
                        npc: Arc::from("Aubury"),
                        prefer: Arc::from([]),
                        choose: None,
                    },
                    &mut t.cx,
                )
                .unwrap()
        });
        with_tick(&snapshot, &mut ledger, 2, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
        });
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        seed_dialogue_combat(&mut snapshot, true);
        with_tick(&snapshot, &mut ledger, 3, |t| {
            assert!(matches!(
                t.actions.poll(&handle, &mut t.cx),
                Poll::Ready(Ok(DialogueOutcome::CombatInterrupted))
            ));
        });
    }
}

#[test]
fn dialogue_continues_fred_pages_reusing_the_same_root() {
    assert_reused_dialogue_page_is_acknowledged(
        "Fred the Farmer",
        4893,
        4899,
        "My sheep are getting mighty woolly. I'd be much obliged if you could shear them.",
        "Yes, that's it. Bring me 20 balls of wool. And I'm sure I could sort out some sort of payment.",
    );
}

#[test]
fn dialogue_continues_sedridor_pages_reusing_the_same_root() {
    assert_reused_dialogue_page_is_acknowledged(
        "Sedridor",
        4887,
        4892,
        "Welcome adventurer, to the world renowned Wizards' Tower.",
        "Have you delivered the research notes to my friend Aubury yet?",
    );
}

fn assert_reused_dialogue_page_is_acknowledged(
    npc: &str,
    root: i32,
    component: i32,
    first: &str,
    second: &str,
) {
    let mut snapshot = ready();
    snapshot.seed_chat_modal(root, vec![npc.into(), first.into()]);
    snapshot.seed_chat_options(vec![], component);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<dialogue::Dialogue>(
                dialogue::DialogueArgs {
                    id: 0,
                    npc: Arc::from(npc),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut t.cx,
            )
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(matches!(emitted(&ledger), InteractReq::ContinueDialog));
    ledger.as_mut().unwrap().outbox.clear();
    // A fresh snapshot of the old page is not an acknowledgement.
    with_tick(&snapshot, &mut ledger, 3, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_chat_modal(root, vec![npc.into(), second.into()]);
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    // Preserve the existing one-game-tick quiet period after the real page turn.
    with_tick(&snapshot, &mut ledger, 5, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(matches!(emitted(&ledger), InteractReq::ContinueDialog));
}

#[test]
fn dialogue_open_clock_starts_after_approaching_the_npc() {
    let mut snapshot = ready();
    snapshot.seed_chat_modal(-1, vec![]);
    let npc = api::snapshot::NpcView {
        index: 7,
        r#type: Some(758),
        name: Some("Fred the Farmer".into()),
        actions: vec![Some("Talk-to".into())],
        tile: tile(3189, 3273),
        distance: 5,
        animation: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health: 1,
        total_health: 1,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: tile(3189, 3273),
        x: 0,
        z: 0,
        yaw: 0,
    };
    snapshot.seed_npcs(vec![npc.clone()]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<dialogue::Dialogue>(
                dialogue::DialogueArgs {
                    id: 758,
                    npc: Arc::from("Fred the Farmer"),
                    prefer: Arc::from([]),
                    choose: None,
                },
                &mut t.cx,
            )
            .unwrap()
    });
    snapshot.seed_npcs(vec![api::snapshot::NpcView { distance: 1, ..npc }]);
    with_tick(&snapshot, &mut ledger, 32, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Npc { index: Some(7), .. }
    ));
    // The approach took 18.6 seconds; the new Open window is still eight seconds.
    with_tick(&snapshot, &mut ledger, 36, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    snapshot.seed_chat_modal(
        4893,
        vec!["Fred the Farmer".into(), "Well I need some wool...".into()],
    );
    snapshot.seed_chat_options(vec![], 4899);
    with_tick(&snapshot, &mut ledger, 37, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(matches!(emitted(&ledger), InteractReq::ContinueDialog));
}

#[test]
fn sheep_partial_hand_in_spins_only_the_remaining_unheld_balls() {
    use client::io::ServerProt;
    compile_context_test(|cx| {
        let document: crate::quester::path::PathDocument =
            serde_json::from_str(crate::quester::compile::SHEEP_JSON).unwrap();
        let here = WorldTile {
            x: 2982,
            z: 3315,
            level: 0,
        };
        let path =
            crate::quester::compile::compile_uncached_for_test(&document, cx.selected, cx.quests)
                .unwrap();
        let step = path.sequences[1]
            .steps
            .iter()
            .find(|step| step.id.0.as_ref() == "spin")
            .unwrap();
        let mut client = wool_menu_client();
        let mut snapshot = ready();
        seed_dialogue_combat(&mut snapshot, false);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.tile = here;
        snapshot.seed_local_player(player);
        snapshot.seed_locs(vec![LocView {
            tile: WorldTile {
                x: 2981,
                z: 3314,
                level: 0,
            },
            distance: 1,
            ..loc(2644, "Spinning wheel", "Spin")
        }]);
        snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
        let ball = ItemView {
            def: def(1759, "Ball of wool"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 6,
            actions: vec![],
            component_id: 3214,
        };
        let raw = ItemView {
            def: def(1737, "Wool"),
            slot: 1,
            ..ball.clone()
        };
        snapshot.seed_inventory(vec![ball.clone(), raw], 28);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_sheep_step(tick, 12, |cx| step.plan.begin(cx).unwrap())
        });
        for game_tick in 2..=4 {
            let result = with_tick(&snapshot, &mut ledger, game_tick, |tick| {
                with_sheep_step(tick, 12, |cx| run.poll(cx))
            });
            assert!(
                result.is_pending(),
                "tick {game_tick}: {:?}",
                result.map(|result| result.map(|_| ()))
            );
        }
        assert!(matches!(
            emitted(&ledger),
            InteractReq::IfButton { component_id: 2120 }
        ));
        client.dialog_input_open = true;
        client.bump_gens(ServerProt::IF_OPENCHAT);
        snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_sheep_step(tick, 12, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(emitted(&ledger), InteractReq::AnswerCount { value: 6 }),
            "12 still due with 6 held needs exactly 6 spun, not another 20"
        );
        client.dialog_input_open = false;
        client.chat_modal_id = -1;
        client.bump_gens(ServerProt::IF_CLOSE);
        snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
        snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
        for game_tick in 6..=7 {
            assert!(
                with_tick(&snapshot, &mut ledger, game_tick, |tick| {
                    with_sheep_step(tick, 12, |cx| run.poll(cx))
                })
                .is_pending(),
                "existing 6 balls are not the desired held output 12"
            );
        }
        snapshot.seed_inventory(
            vec![ItemView {
                count: 11,
                ..ball.clone()
            }],
            28,
        );
        let Poll::Ready(Ok(outcome)) = with_tick(&snapshot, &mut ledger, 8, |tick| {
            with_sheep_step(tick, 12, |cx| run.poll(cx))
        }) else {
            panic!("observed production progress must return to Path settlement");
        };
        let settle = |cx: &mut StepContext<'_, '_>| {
            step.settle.evaluate(&PredicateContext {
                cx: &cx.tick.cx,
                quests: cx.quests,
                progress: cx.progress,
                required_after: cx.required_after,
                chat_since: 0,
                outcome: Some(&outcome),
                bank: cx.bank,
            })
        };
        assert_eq!(
            with_tick(&snapshot, &mut ledger, 8, |tick| {
                with_sheep_step(tick, 12, settle)
            }),
            Truth::False,
            "11 held balls cannot settle the journal's remaining 12"
        );
        snapshot.seed_inventory(vec![ItemView { count: 12, ..ball }], 28);
        assert_eq!(
            with_tick(&snapshot, &mut ledger, 9, |tick| {
                with_sheep_step(tick, 12, settle)
            }),
            Truth::True
        );
    });
}

#[test]
fn dialogue_nearby_blocked_npc_keeps_approaching_until_clipping_allows_talk() {
    compile_context_test(|cx| {
        let row = cx.selected.npc_by_config("fred_the_farmer").unwrap();
        let npc_tile = tile(3186, 3273);
        let mut snapshot = ready();
        seed_dialogue_combat(&mut snapshot, false);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.tile = tile(3188, 3273);
        snapshot.seed_local_player(player);
        snapshot.seed_npcs(vec![api::snapshot::NpcView {
            index: 42,
            r#type: Some(row.id as usize),
            name: row.display.clone(),
            actions: vec![Some("Talk-to".into())],
            tile: npc_tile,
            distance: 2,
            animation: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: npc_tile,
            x: 0,
            z: 0,
            yaw: 0,
        }]);
        // The posted flood covers both actors; the closed barrier leaves the
        // NPC without a wall-valid adjacent dequeue rank.
        let blocked = api::query::ReachQueryView {
            available: true,
            base_x: 3186,
            base_z: 3272,
            level: 0,
            width: 3,
            height: 3,
            walkable: vec![0x1ff],
            reachable: vec![0],
            reachable_adj: vec![0],
            exact_rank: vec![u16::MAX; 9],
            adjacent_rank: vec![u16::MAX; 9],
            step: vec![0; 9],
            canlight: vec![],
        };
        let plan = compile_talk(&serde_json::json!({"npc":"fred_the_farmer"}), cx).unwrap();
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &blocked, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick_reach(&snapshot, &blocked, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
        match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
            HostEffect::Walk(request) => {
                assert_eq!(request.target, npc_tile);
                assert_eq!(
                    request.radius, 0,
                    "route to the NPC's side, not an adjacent tile across the barrier"
                );
            }
            HostEffect::Interaction(_) => {
                panic!("geometric proximity cannot bypass closed clipping")
            }
        }
        assert!(
            with_tick_reach(&snapshot, &blocked, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
        assert!(
            ledger.as_ref().unwrap().outbox.iter().all(|action| {
                !matches!(
                    action.effect,
                    HostEffect::Interaction(InteractReq::Npc { .. })
                )
            }),
            "do not cancel the door approach and talk while clipping remains blocked"
        );
        let mut opened = blocked;
        opened.reachable[0] = 1 << 1;
        opened.reachable_adj[0] = 1 << 1;
        opened.exact_rank[1] = 0;
        opened.adjacent_rank[1] = 0;
        assert!(with_tick_reach(&snapshot, &opened, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                emitted(&ledger),
                InteractReq::Npc {
                    index: Some(42),
                    ..
                }
            ),
            "resume the same conversation once the observed barrier is traversable"
        );
    });
}

#[test]
fn talk_walk_user_input_blocks_before_dialogue_interaction() {
    let snapshot = ready();
    let mut ledger = None;
    let mut run = TalkRun {
        id: 42,
        npc: Arc::from("test npc"),
        tile: Some(tile(3200, 3200)),
        leash: 1,
        prefer: Arc::from([]),
        choose: None,
        walk: None,
        dialogue: None,
        started: false,
    };
    assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    post_user_input_walk_receipt(&mut ledger, 2);
    assert_manual_movement(with_tick(&snapshot, &mut ledger, 2, |tick| {
        with_step(tick, |cx| run.poll(cx))
    }));
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| { matches!(&action.effect, HostEffect::Walk(_)) }));
}

#[test]
fn refused_talk_approach_blocks_without_requeueing_the_walk() {
    for end in [WalkEnd::Failed, WalkEnd::Blocked, WalkEnd::Refused] {
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(WorldTile {
            x: 2632,
            z: 3222,
            level: 0,
        }));
        let mut ledger = None;
        let mut run = TalkRun {
            id: 42,
            npc: Arc::from("test npc"),
            tile: Some(WorldTile {
                x: 3103,
                z: 3163,
                level: 2,
            }),
            leash: 6,
            prefer: Arc::from([]),
            choose: None,
            walk: None,
            dialogue: None,
            started: false,
        };
        assert!(with_tick(&snapshot, &mut ledger, 40, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let request_id = post_user_input_walk_receipt(&mut ledger, 66);
        ledger.as_mut().unwrap().walk.as_mut().unwrap().end = end.clone();
        let result = with_tick(&snapshot, &mut ledger, 66, |tick| {
            with_step(tick, |cx| run.poll(cx))
        });
        assert!(
            matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))),
            "the owner must see the refused {end:?} approach, not another pending walk"
        );
        let requests: Vec<_> = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .map(|action| action.request_id.get())
            .collect();
        assert_eq!(requests, vec![request_id], "no second walk or dialogue");
    }
}

#[test]
fn use_on_walk_user_input_blocks_before_interaction() {
    let snapshot = ready();
    let mut ledger = None;
    let mut run = UseOnRun {
        item: Arc::from("test item"),
        item_id: 1,
        target_id: 2,
        product: None,
        until: None,
        no_product: None,
        kind: Arc::from("loc"),
        target_name: Some(Arc::from("test target")),
        tile: Some(tile(3200, 3200)),
        radius: 1,
        deadline: None,
        settle_duration: Duration::from_secs(20),
        walk: None,
        interaction: None,
        accepted: false,
        round_before: None,
        chat_since: 0,
    };
    assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    run.deadline = Some(Duration::ZERO);
    post_user_input_walk_receipt(&mut ledger, 2);
    assert_manual_movement(with_tick(&snapshot, &mut ledger, 2, |tick| {
        with_step(tick, |cx| run.poll(cx))
    }));
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| { matches!(&action.effect, HostEffect::Walk(_)) }));
}

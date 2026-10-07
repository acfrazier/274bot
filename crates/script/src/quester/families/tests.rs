use super::*;
use crate::native::{
    ledger, HostEffect, NativeOutput, NativeTick, RetainedMemory, ScriptStatus, WalkEnd,
};
use api::obj_names::ItemDefView;
use api::quest_progress::{EvidenceStamp, ProgressFlag, QuestProgress};
use api::selected::{ClientRevision, FactKey, Knowledge, RunKey, Truth};
use api::snapshot::{
    GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
    SnapshotView,
};
use std::time::Instant;
fn test_args<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).expect("valid typed family test args")
}

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
    with_tick_output_reach(
        snapshot,
        None,
        Truth::Unknown,
        ledger,
        tick,
        output,
        None,
        f,
    )
}

pub(crate) fn with_tick_reach<R>(
    snapshot: &GameSnapshot,
    reach: &api::query::ReachQueryView,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(
        snapshot,
        Some(reach),
        Truth::Unknown,
        ledger,
        tick,
        &mut Output,
        None,
        f,
    )
}

/// A tick whose snapshot view carries the host-bound world type.
pub(crate) fn with_tick_world<R>(
    snapshot: &GameSnapshot,
    world_members: Truth,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(
        snapshot,
        None,
        world_members,
        ledger,
        tick,
        &mut Output,
        None,
        f,
    )
}

pub(crate) fn with_tick_bank<R>(
    snapshot: &GameSnapshot,
    bank: Option<&api::bank_memory::BankMemory>,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(
        snapshot,
        None,
        Truth::Unknown,
        ledger,
        tick,
        &mut Output,
        bank,
        f,
    )
}

pub(crate) fn with_tick_output_bank<R>(
    snapshot: &GameSnapshot,
    bank: Option<&api::bank_memory::BankMemory>,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    output: &mut dyn NativeOutput,
    f: impl FnOnce(&mut NativeTick<'_>) -> R,
) -> R {
    with_tick_output_reach(
        snapshot,
        None,
        Truth::Unknown,
        ledger,
        tick,
        output,
        bank,
        f,
    )
}

// The world type (members-world) and the bank memory (bank S3/S4) both reach
// the view through this one builder; the arg count is allowed.
#[allow(clippy::too_many_arguments)]
fn with_tick_output_reach<R>(
    snapshot: &GameSnapshot,
    reach: Option<&api::query::ReachQueryView>,
    world_members: Truth,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    output: &mut dyn NativeOutput,
    bank: Option<&api::bank_memory::BankMemory>,
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
            snapshot: SnapshotView::new(Some(snapshot), evidence)
                .with_reach(reach)
                .with_world_members(world_members)
                .with_bank_memory(bank),
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
                bank_memory: bank,
                collision: None,
                hold: false,
                world_members,
                interacts: Some(Vec::new()),
            },
        },
    };
    f(&mut native)
}
fn accept_last(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, accepted: bool) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted,
            chat_since: 0,
        },
    );
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
            network: tile,
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
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
            },
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
    }
}

pub(crate) fn policy_s2_recipe_run(child: Box<dyn StepRun>) -> Box<dyn StepRun> {
    Box::new(AcquireRun {
        recipe: Arc::from("policy-s2-recipe"),
        steps: Arc::from(vec![CompiledAcquireStep {
            id: FactKey::new("policy-child"),
            advances: false,
            skip_if: Arc::new(AnyPlan { items: vec![] }),
            skip_if_summary: Arc::from("never"),
            settle: Arc::new(AllPlan { items: vec![] }),
            plan: Arc::new(WaitPlan {
                until: Arc::new(AllPlan { items: vec![] }),
                max_ticks: 2,
            }),
        }]),
        goal: None,
        max_restarts: 0,
        restarts: 0,
        pass_began_child: false,
        goal_chat_since: 0,
        current: Some(child),
        child_outcome: None,
        index: 0,
        chat_since: 0,
        settling: false,
        settle_deadline: Duration::ZERO,
        waiting_for_read: false,
        selection_since: None,
        prayer_cleanup_owned: crate::combat::RaisedPrayers::empty(),
        clear_prayers: None,
        trace_events: std::collections::VecDeque::new(),
    })
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
        refusal: None,
        assessment: None,
        escape: None,
    });
    request_id
}

fn assert_manual_movement(result: Poll<Result<StepOutcome, ActionError>>) {
    assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
}
fn mapped_walk_receipt(
    end: crate::native::WalkEnd,
    detail: Option<Arc<str>>,
) -> crate::native::WalkReceipt {
    crate::native::WalkReceipt {
        request_id: 7,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 2,
                session: 3,
            },
            tick: 4,
            sequence: 5,
        },
        end,
        blocked: None,
        detail,
        refusal: None,
        assessment: None,
        escape: None,
    }
}

#[test]
fn quest_walk_step_requires_arrival_and_preserves_refusal_or_user_input() {
    let route_end = walk_step_evidence(mapped_walk_receipt(
        crate::native::WalkEnd::RouteEnded,
        Some(Arc::from("route stopped short")),
    ));
    assert!(matches!(
        route_end,
        Err(ActionError::Blocked(detail)) if detail.as_ref() == "route stopped short"
    ));
    assert_eq!(
        walk_step_evidence(mapped_walk_receipt(crate::native::WalkEnd::UserInput, None)),
        Err(ActionError::UserInput)
    );
    assert_eq!(
        walk_step_evidence(mapped_walk_receipt(crate::native::WalkEnd::Cancelled, None)),
        Err(ActionError::Cancelled)
    );
}

#[test]
fn quest_walk_step_keeps_needs_evidence_typed() {
    let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Complete(
        FactKey::new("reach-gate"),
    )]);
    let error = walk_step_evidence(mapped_walk_receipt(
        crate::native::WalkEnd::NeedsEvidence(Arc::clone(&gates)),
        None,
    ))
    .unwrap_err();
    assert_eq!(
        format!("{error:?}"),
        format!("NeedsEvidence({gates:?})"),
        "quester needs typed gates, not a debug-string Blocked"
    );
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
        target_tile: None,
        reachable_only: false,
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
fn reach_held_transports_exact_item_identity() {
    let mut snapshot = ready();
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(758, "IOU"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 7,
            count: 1,
            actions: vec![Some("Read".into())],
            component_id: 0,
        }],
        28,
    );
    let mut args = reach_args(
        reach::ReachKind::Held {
            id: 758,
            obj: Arc::from("IOU"),
        },
        false,
    );
    args.op = Arc::from("Read");
    let mut ledger = None;
    let _handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<reach::Reach>(args, &mut tick.cx)
            .unwrap()
    });

    assert!(matches!(
        emitted(&ledger),
        InteractReq::Held {
            name,
            action,
            slot: Some(7),
            target_item_id: Some(758),
        } if name == "IOU" && action == "Read"
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
    args.anchor = None;
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
            network: tile(3077, 3426),
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
                tile: tile(3077, 3426),
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
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
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
        HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
            panic!("open door recovery cannot select a bank or advisory")
        }
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
fn non_straight_wall_door_route_end_before_arrival_does_not_open_the_door() {
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
        end: crate::native::WalkEnd::RouteEnded,
        blocked: None,
        detail: Some(Arc::from("route stopped short")),
        refusal: None,
        assessment: None,
        escape: None,
    });
    assert!(matches!(
        with_tick_reach(&s, &reach, &mut ledger, 3, |t| t
            .actions
            .poll(&handle, &mut t.cx)),
        Poll::Ready(Err(ActionError::Blocked(reason))) if reason.as_ref() == "route stopped short"
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
fn reach_wait_walk_preserves_typed_needs_evidence() {
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
        HostEffect::Walk(_)
    ));
    let request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .request_id
        .get();
    let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Complete(
        FactKey::new("reach-gate"),
    )]);
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
        end: crate::native::WalkEnd::NeedsEvidence(Arc::clone(&gates)),
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });

    let result = with_tick_reach(&s, &reach, &mut ledger, 3, |t| {
        t.actions.poll(&handle, &mut t.cx)
    });
    assert_eq!(
        format!("{result:?}"),
        format!("Ready(Err(NeedsEvidence({gates:?})))"),
        "reach must retain typed gates instead of flattening them to Blocked"
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
        HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
            panic!("door approach cannot select a bank or advisory")
        }
    }
}

#[test]
fn vanished_clicked_loc_rejected_dispatch_fails_without_retargeting_a_replacement() {
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
        args.anchor = Some(original.tile);
        args.radius = 4;
        let mut ledger = None;
        let handle = with_tick(&s, &mut ledger, 1, |t| {
            t.actions.begin::<reach::Reach>(args, &mut t.cx).unwrap()
        });
        let mut replacement = original;
        replacement.tile.x += 1;
        s.seed_locs(vec![replacement]);
        accept_last(&mut ledger, 2, false);
        assert!(matches!(
            with_tick(&s, &mut ledger, 2, |t| t.actions.poll(&handle, &mut t.cx)),
            Poll::Ready(Ok(false))
        ));
    }
}

#[test]
fn vanished_clicked_loc_with_accepted_dispatch_does_not_retarget_replacements() {
    for kind in [
        reach::ReachKind::Loc {
            id: Some(1551),
            name: Some(Arc::from("Door")),
        },
        reach::ReachKind::Name {
            name: Arc::from("Door"),
        },
    ] {
        for replacement_offset in [None, Some(0), Some(1)] {
            let mut snapshot = ready();
            let original = loc(1551, "Door", "Open");
            snapshot.seed_locs(vec![original.clone()]);
            let mut args = reach_args(kind.clone(), false);
            args.op = Arc::from("Open");
            args.anchor = Some(original.tile);
            args.radius = 4;
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<reach::Reach>(args, &mut tick.cx)
                    .unwrap()
            });
            let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 2,
                        sequence: 2,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            snapshot.seed_locs(
                replacement_offset
                    .map(|offset| {
                        let mut opened = loc(1552, "Door", "Close");
                        opened.tile = original.tile;
                        opened.tile.x += offset;
                        vec![opened]
                    })
                    .unwrap_or_default(),
            );
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 2, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                }),
                Poll::Ready(Ok(true))
            ));
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    }
}

#[test]
fn vanished_clicked_loc_with_rejected_dispatch_fails_without_retargeting() {
    for kind in [
        reach::ReachKind::Loc {
            id: Some(1551),
            name: Some(Arc::from("Door")),
        },
        reach::ReachKind::Name {
            name: Arc::from("Door"),
        },
    ] {
        for replacement_offset in [None, Some(0), Some(1)] {
            let mut snapshot = ready();
            let original = loc(1551, "Door", "Open");
            snapshot.seed_locs(vec![original.clone()]);
            let mut args = reach_args(kind.clone(), false);
            args.op = Arc::from("Open");
            args.anchor = Some(original.tile);
            args.radius = 4;
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<reach::Reach>(args, &mut tick.cx)
                    .unwrap()
            });
            let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 2,
                        sequence: 2,
                    },
                    accepted: false,
                    chat_since: 0,
                },
            );
            snapshot.seed_locs(
                replacement_offset
                    .map(|offset| {
                        let mut opened = loc(1552, "Door", "Close");
                        opened.tile = original.tile;
                        opened.tile.x += offset;
                        vec![opened]
                    })
                    .unwrap_or_default(),
            );
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 2, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                }),
                Poll::Ready(Ok(false))
            ));
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    }
}
fn with_step<R>(t: &mut NativeTick<'_>, f: impl FnOnce(&mut StepContext<'_, '_>) -> R) -> R {
    with_step_banks(t, &Arc::new(api::named_banks::NamedBankFacts::empty()), f)
}
fn with_step_banks<R>(
    t: &mut NativeTick<'_>,
    banks: &Arc<api::named_banks::NamedBankFacts>,
    f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
) -> R {
    let quests = api::quest_facts::QuestCatalog::empty();
    let required_after = t.cx.evidence();
    f(&mut StepContext {
        tick: t,
        quests: &quests,
        progress: &[],
        required_after,
        banks,
        choices: &crate::quester::choices::QuestChoices::default(),
    })
}

fn path_bank_context<'a>(
    base: &CompileContext<'a>,
    bank_tile: WorldTile,
    required: bool,
) -> CompileContext<'a> {
    CompileContext {
        path: base.path,
        kind: base.kind,
        pair: base.pair,
        progress: base.progress,
        selected: base.selected,
        quests: base.quests,
        gathering: base.gathering,
        areas: base.areas,
        recipes: base.recipes,
        bank: Some(api::named_banks::NamedBank::new("Path bank", bank_tile)),
        bank_required: required,
        loadouts: base.loadouts,
        keep_ids: base.keep_ids,
    }
}

fn bank_pick_for_plan(
    plan: Arc<dyn StepPlan>,
    snapshot: &GameSnapshot,
    banks: &Arc<api::named_banks::NamedBankFacts>,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> Result<crate::native_bank::BankPickRequest, crate::native::ActionError> {
    let mut run = with_tick(snapshot, ledger, 1, |tick| {
        with_step_banks(tick, banks, |cx| plan.begin(cx).unwrap())
    });
    for now in 2..=5 {
        let result = with_tick(snapshot, ledger, now, |tick| {
            with_step_banks(tick, banks, |cx| run.poll(cx))
        });
        if let Some(request) = ledger.as_ref().and_then(|ledger| {
            ledger
                .outbox
                .iter()
                .find_map(|action| match &action.effect {
                    HostEffect::BankPick(request) => Some(request.clone()),
                    _ => None,
                })
        }) {
            return Ok(request);
        }
        match result {
            Poll::Pending => {}
            Poll::Ready(Err(error)) => return Err(error),
            Poll::Ready(Ok(_)) => {
                return Err(crate::native::ActionError::Unavailable(Arc::from(
                    "bank run completed without selection",
                )))
            }
        }
    }
    Err(crate::native::ActionError::Unavailable(Arc::from(
        "bank selection did not emit a request",
    )))
}

#[test]
fn bank_without_candidates_fails_before_walk_or_open() {
    compile_context_test(|compile| {
        let plan = s2::compile_bank(
            test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        seed_dialogue_combat(&mut snapshot, false);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(crate::native::ActionError::Unavailable(reason)))
                if reason.as_ref() == "no eligible bank"
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    });
}

#[test]
fn native_bank_without_explicit_selection_opens_the_context_bank() {
    compile_context_test(|compile| {
        let plan = s2::compile_bank(
            test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        seed_dialogue_combat(&mut snapshot, false);
        let bank_tile = snapshot.local_player().unwrap().player.actor.tile;
        let bank = api::named_banks::NamedBank::new("Native nearest", bank_tile);
        let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
        let mut booth = loc(2213, "Bank booth", "Use-quickly");
        booth.tile = bank_tile;
        booth.distance = 0;
        snapshot.seed_locs(vec![booth]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step_banks(tick, &banks, |cx| plan.begin(cx).unwrap())
        });
        for now in 2..=3 {
            assert!(with_tick(&snapshot, &mut ledger, now, |tick| {
                with_step_banks(tick, &banks, |cx| run.poll(cx))
            })
            .is_pending());
        }
        let action = ledger.as_mut().unwrap().outbox.pop().unwrap();
        let HostEffect::BankPick(request) = &action.effect else {
            panic!("native bank selection must precede walking or opening");
        };
        assert!(request.explicit_bank.is_none());
        ledger.as_mut().unwrap().complete_bank_pick(
            &action.authority(),
            crate::bank::BankPickReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: action.authority().run(),
                    tick: 3,
                    sequence: 3,
                },
                selected: crate::bank::SelectedBank {
                    bank_index: 0,
                    access_tile: bank_tile,
                    kind: crate::bank::PickKind::AirFallback,
                    access: Some(Arc::new(crate::bank::BankStandAccess {
                        bank,
                        stand_tile: bank_tile,
                        kind: crate::bank::AccessKind::Booth,
                        stand_op: 1,
                        name: None,
                        choose: None,
                    })),
                },
            },
        );
        for now in 4..=7 {
            assert!(with_tick(&snapshot, &mut ledger, now, |tick| {
                with_step_banks(tick, &banks, |cx| run.poll(cx))
            })
            .is_pending());
            if !ledger.as_ref().unwrap().outbox.is_empty() {
                break;
            }
        }
        assert!(matches!(
            emitted(&ledger),
            InteractReq::OpenStand {
                x: 3253,
                z: 3401,
                level: 0,
                kind,
                name,
                stand_op: Some(1),
                ..
            } if kind == "booth" && name.as_deref() == Some("Bank booth")
        ));
    });
}

#[test]
fn authored_draynor_varrock_and_unmatched_tiles_select_nearest_without_explicit_bank() {
    compile_context_test(|base| {
        let draynor = tile(3092, 3242);
        let varrock_west = tile(3185, 3438);
        let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
            api::named_banks::NamedBank::new("Draynor Village", draynor),
            api::named_banks::NamedBank::new("Varrock West", varrock_west),
        ]));
        for authored in [draynor, varrock_west, tile(3200, 3200)] {
            let compile = path_bank_context(base, authored, false);
            let plan = s2::compile_bank(
                test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
                &compile,
            )
            .unwrap();
            let mut snapshot = ready();
            snapshot.seed_local_player(local_player(tile(3100, 3240)));
            let mut ledger = None;
            let request = bank_pick_for_plan(plan, &snapshot, &banks, &mut ledger).unwrap();
            assert!(
                request.explicit_bank.is_none(),
                "authored tile {authored:?} must not constrain bank ranking"
            );
        }
    });
}

#[test]
fn required_draynor_bank_is_explicit_and_unmatched_required_bank_refuses() {
    compile_context_test(|base| {
        let draynor = tile(3092, 3242);
        let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
            api::named_banks::NamedBank::new("Draynor Village", draynor),
        ]));
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(3100, 3240)));
        let compile = path_bank_context(base, draynor, true);
        let plan = s2::compile_bank(
            test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
            &compile,
        )
        .unwrap();
        let mut ledger = None;
        let request = bank_pick_for_plan(plan, &snapshot, &banks, &mut ledger).unwrap();
        assert_eq!(request.explicit_bank, Some(0));

        let unmatched = tile(3200, 3200);
        let compile = path_bank_context(base, unmatched, true);
        let plan = s2::compile_bank(
            test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
            &compile,
        )
        .unwrap();
        let mut ledger = None;
        assert!(matches!(
            bank_pick_for_plan(plan, &snapshot, &banks, &mut ledger),
            Err(crate::native::ActionError::Unavailable(reason))
                if reason.as_ref() == "required bank is not in the bank catalog"
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    });
}

#[test]
fn bank_walk_manual_takeover_parks_without_opening_or_rewalking() {
    compile_context_test(|base| {
        let bank_tile = tile(3200, 3200);
        let bank = api::named_banks::NamedBank::new("Path bank", bank_tile);
        let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
        let compile = path_bank_context(base, bank_tile, false);
        let plan = s2::compile_bank(
            test_args::<s2::BankArgs>(serde_json::json!({"op": "scan"})),
            &compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(3100, 3200)));
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step_banks(tick, &banks, |cx| plan.begin(cx).unwrap())
        });
        for now in 2..=3 {
            assert!(with_tick(&snapshot, &mut ledger, now, |tick| {
                with_step_banks(tick, &banks, |cx| run.poll(cx))
            })
            .is_pending());
        }
        let action = ledger.as_mut().unwrap().outbox.pop().unwrap();
        let HostEffect::BankPick(request) = &action.effect else {
            panic!("native bank selection must precede walking");
        };
        assert!(request.explicit_bank.is_none());
        ledger.as_mut().unwrap().complete_bank_pick(
            &action.authority(),
            crate::bank::BankPickReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: action.authority().run(),
                    tick: 3,
                    sequence: 3,
                },
                selected: crate::bank::SelectedBank {
                    bank_index: 0,
                    access_tile: bank_tile,
                    kind: crate::bank::PickKind::Reachable,
                    access: Some(Arc::new(crate::bank::BankStandAccess {
                        bank,
                        stand_tile: bank_tile,
                        kind: crate::bank::AccessKind::Booth,
                        stand_op: 1,
                        name: None,
                        choose: None,
                    })),
                },
            },
        );
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step_banks(tick, &banks, |cx| run.poll(cx))
        })
        .is_pending());
        post_user_input_walk_receipt(&mut ledger, 5);
        ledger.as_mut().unwrap().outbox.clear();
        assert_manual_movement(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_step_banks(tick, &banks, |cx| run.poll(cx))
        }));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    });
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
        dialogue_cap: None,
        dialogue_page: None,
        missing_deadline: None,
        waiting: None,
        settle_duration: Duration::from_secs(20),
        walk: None,
        reach: Some(handle),
        default_dialogue: true,
        dialogue_options: None,
        dialogue: None,
        started: true,
        dialogue_started: None,
        dialogue_completed: false,
        accepted_tick: None,
        scene_activity_observed: false,
        scene_in_range_at_acceptance: false,
        until: None,
        target_tile: None,
        reachable_only: false,
        round_pick: None,
        retargets_left: reach::RETARGET_LIMIT,
        round_before: None,
        round_deadline: None,
        round_accepted: false,
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
        default_dialogue: true,
        dialogue_options: None,
        until: None,
        target_tile: None,
        reachable_only: false,
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
    s.seed_local_player(local_player(tile(3229, 3302)));
    let mut ledger = None;
    let plan = InteractPlan {
        kind: egg(true).kind,
        op: Arc::from("Take"),
        tile: None,
        radius: 2,
        wait_if_missing: true,
        settle_ms: Some(20_000),
        ambiguous: false,
        default_dialogue: true,
        dialogue_options: None,
        until: None,
        target_tile: None,
        reachable_only: false,
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
    assert!(with_tick(&s, &mut ledger, 102, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(matches!(
        with_tick(&s, &mut ledger, 103, |t| with_step(t, |cx| run.poll(cx))),
        Poll::Ready(Ok(_))
    ));
}
#[test]
fn use_on_item_target_emits_inventory_kind() {
    compile_context_test(|cx| {
        let source_id = resolve_obj(cx, "shears").unwrap();
        let target_id = resolve_obj(cx, "wool").unwrap();
        let source = ItemView {
            def: def(source_id, "Shears"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 3,
            count: 1,
            actions: vec![],
            component_id: 3214,
        };
        let target = ItemView {
            def: def(target_id, "Wool"),
            slot: 7,
            ..source.clone()
        };
        let mut snapshot = ready();
        snapshot.seed_inventory(vec![source, target], 28);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears",
                "target": {"item": "wool"},
                "settle_ms": 20_000,
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                source_item_id: Some(source),
                source_item_slot: Some(3),
                target_item_id: Some(target),
                target_item_slot: Some(7),
                ..
            } if kind == "inv" && *source == source_id && *target == target_id
        ));
    });
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
        kind: crate::quester::path::PathKind::Quest,
        pair: None,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        areas: &areas,
        recipes: &recipes,
        bank: None,
        bank_required: false,
        keep_ids: &[],
        loadouts: &crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([])),
    };
    let plan = compile_use_on(test_args::<UseOnArgs>(serde_json::json!({ "item": "grain", "target": {"loc": "hopper_lumbridge"}, "radius": 8, "settle_ms": 20000 })), &compile).unwrap();
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
        matches!(emitted(&ledger), InteractReq::UseOn { name, x: 3167, z: 3308, source_item_id: Some(source), source_item_slot: Some(7), target_item_id: Some(target), .. } if name == "Grain" && *source == id && *target == loc_id)
    );
}

#[test]
fn use_on_product_continues_post_use_modal_before_settling() {
    compile_context_test(|cx| {
        for (product_goal, chat_root) in [(true, 2100), (false, 2100), (true, -1), (false, -1)] {
            let source_id = resolve_obj(cx, "doogleleaves").unwrap();
            let target_id = resolve_obj(cx, "raw_sardine").unwrap();
            let product_id = resolve_obj(cx, "seasoned_sardine").unwrap();
            let source = ItemView {
                def: def(source_id, "Doogle leaves"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            };
            let target = ItemView {
                def: def(target_id, "Raw sardine"),
                slot: 1,
                ..source.clone()
            };
            let product = ItemView {
                def: def(product_id, "Seasoned sardine"),
                slot: 2,
                ..source.clone()
            };
            let mut snapshot = ready();
            snapshot.seed_inventory(vec![source, target], 28);
            let plan = compile_use_on(
                test_args::<UseOnArgs>(serde_json::json!({
                    "item": "doogleleaves",
                    "target": {"item": "raw_sardine"},
                    "product": product_goal.then_some("seasoned_sardine"),
                    "settle_ms": 20_000
                })),
                cx,
            )
            .unwrap();
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert!(matches!(
                emitted(&ledger),
                InteractReq::UseOn {
                    kind, source_item_id: Some(source), target_item_id: Some(target), ..
                } if kind == "inv" && *source == source_id && *target == target_id
            ));
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
                    chat_since: 0,
                },
            );
            snapshot.seed_chat_modal(
                chat_root,
                vec!["You rub the doogle leaves over the sardine.".into()],
            );
            snapshot.seed_chat_options(vec![], 2105);
            for tick in 3..=4 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
            }
            assert!(
                matches!(
                    emitted(&ledger),
                    InteractReq::ContinueDialog { component_id: None }
                ),
                "accepted use-on must drain root {chat_root} before product or no-product settlement"
            );
            let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 5,
                        sequence: 5,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            snapshot.seed_chat_modal(-1, vec![]);
            snapshot.seed_chat_options(vec![], -1);
            snapshot.seed_inventory(vec![product], 28);
            let mut settled = Poll::Pending;
            for tick in 5..=15 {
                settled = with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                });
                if settled.is_ready() {
                    break;
                }
            }
            assert!(
                matches!(settled, Poll::Ready(Ok(_))),
                "product goal {product_goal} must settle only after root {chat_root} is drained"
            );
        }
    });
}

#[test]
fn use_on_explicit_none_leaves_visible_pages_to_the_next_owner() {
    compile_context_test(|cx| {
        for goal in ["until", "product", "none"] {
            for root in [2100, -1] {
                let source_id = resolve_obj(cx, "doogleleaves").unwrap();
                let target_id = resolve_obj(cx, "raw_sardine").unwrap();
                let product_id = resolve_obj(cx, "seasoned_sardine").unwrap();
                let source = ItemView {
                    def: def(source_id, "Doogle leaves"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 0,
                    count: 1,
                    actions: vec![],
                    component_id: 3214,
                };
                let target = ItemView {
                    def: def(target_id, "Raw sardine"),
                    slot: 1,
                    ..source.clone()
                };
                let product = ItemView {
                    def: def(product_id, "Seasoned sardine"),
                    slot: 2,
                    ..source.clone()
                };
                let mut args = serde_json::json!({
                    "item": "doogleleaves",
                    "target": {"item": "raw_sardine"},
                    "dialogue": "none"
                });
                match goal {
                    "until" => {
                        args["until"] = serde_json::json!({"obj":"seasoned_sardine","qty":1})
                    }
                    "product" => args["product"] = serde_json::json!("seasoned_sardine"),
                    _ => {}
                }
                let plan = compile_use_on(test_args::<UseOnArgs>(args), cx).unwrap();
                let mut snapshot = ready();
                snapshot.seed_inventory(vec![source, target], 28);
                if goal == "until" {
                    snapshot.seed_chat_modal(root, vec!["An existing page.".into()]);
                    snapshot.seed_chat_options(vec![], 2105);
                }
                let mut ledger = None;
                let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                    with_step(tick, |cx| plan.begin(cx).unwrap())
                });
                assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                assert!(
                    matches!(emitted(&ledger), InteractReq::UseOn { kind, .. } if kind == "inv"),
                    "explicit none must not auto-continue before an until attempt"
                );
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
                        chat_since: 0,
                    },
                );
                snapshot.seed_chat_modal(root, vec!["A post-use page.".into()]);
                snapshot.seed_chat_options(vec![], 2105);
                snapshot.seed_inventory(vec![product], 28);
                assert!(
                    matches!(
                        with_tick(&snapshot, &mut ledger, 3, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        }),
                        Poll::Ready(Ok(_))
                    ),
                    "explicit none leaves root {root} to the next owner for {goal}"
                );
                assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));
                assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
                    tick.cx
                        .snapshot()
                        .chat_modal()
                        .is_some_and(|chat| chat.value.continue_component_id == 2105)
                }));
            }
        }
    });
}

fn use_on_footprint_fixture(
    cx: &CompileContext<'_>,
    from: WorldTile,
) -> (GameSnapshot, LocView, api::snapshot::SceneView) {
    let mut target = loc(
        resolve_loc(cx, "hopper_lumbridge").unwrap(),
        "Hopper",
        "Use",
    );
    target.shape = 10;
    target.angle = 1;
    target.width = 2;
    target.length = 3;
    target.footprint_width = 3;
    target.footprint_length = 2;
    target.force_approach = 14; // Rotates to 13: only the east side is open.
    target.distance = (from.x - target.tile.x)
        .abs()
        .max((from.z - target.tile.z).abs());
    let mut scene = api::snapshot::SceneView {
        available: true,
        base_x: target.tile.x - 4,
        base_z: target.tile.z - 3,
        level: target.tile.level,
        width: 10,
        height: 8,
        collision_flags: vec![0; 80],
    };
    for x in target.tile.x..target.tile.x + target.footprint_width {
        for z in target.tile.z..target.tile.z + target.footprint_length {
            let index = ((x - scene.base_x) * scene.height + z - scene.base_z) as usize;
            scene.collision_flags[index] = client::dash3d::CollisionFlag::SQ_BLOCKED;
        }
    }
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(from));
    snapshot.seed_scene(scene.clone());
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(resolve_obj(cx, "grain").unwrap(), "Grain"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 7,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    snapshot.seed_locs(vec![target.clone()]);
    (snapshot, target, scene)
}

#[test]
fn use_on_loc_walks_to_rotated_footprint_before_dispatch() {
    compile_context_test(|cx| {
        let origin = tile(3167, 3308);
        for offset in [-1, -3, 3] {
            let from = tile(origin.x + offset, origin.z);
            let (mut snapshot, mut target, mut scene) = use_on_footprint_fixture(cx, from);
            if offset == 3 {
                let index =
                    ((from.x - scene.base_x) * scene.height + from.z - scene.base_z) as usize;
                scene.collision_flags[index] |= client::dash3d::CollisionFlag::W_W;
                snapshot.seed_scene(scene.clone());
            }
            assert_eq!(
                api::query::loc_approach::can_operate_from(&target, &scene, from),
                Some(false)
            );
            let plan = compile_use_on(
                test_args::<UseOnArgs>(serde_json::json!({
                    "item": "grain", "target": {"loc": "hopper_lumbridge"}, "radius": 8
                })),
                cx,
            )
            .unwrap();
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert!(
                matches!(&ledger.as_ref().unwrap().outbox[0].effect,
                    HostEffect::Walk(request) if request.target == origin
                        && request.loc_id == Some(target.id) && request.radius == 1),
                "offset {offset} must approach the selected footprint before use-on"
            );
            let walk_request = ledger.as_ref().unwrap().outbox[0].request_id;
            assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);

            let stand = tile(origin.x + target.footprint_width, origin.z);
            let index = ((stand.x - scene.base_x) * scene.height + stand.z - scene.base_z) as usize;
            scene.collision_flags[index] = 0;
            snapshot.seed_scene(scene.clone());
            snapshot.seed_local_player(local_player(stand));
            target.distance = target.footprint_width;
            let mut nearer_same_id = target.clone();
            nearer_same_id.tile.x = stand.x + 1;
            nearer_same_id.distance = 1;
            let mut co_located_decoy = target.clone();
            co_located_decoy.id += 1;
            co_located_decoy.distance = 0;
            snapshot.seed_locs(vec![co_located_decoy, nearer_same_id, target.clone()]);
            assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
            assert_ne!(ledger.as_ref().unwrap().outbox[0].request_id, walk_request);
            assert!(matches!(
                emitted(&ledger),
                InteractReq::UseOn {
                    kind, x, z, target_name: Some(name),
                    source_item_id: Some(source), source_item_slot: Some(7),
                    target_item_id: Some(id), ..
                } if kind == "loc" && name == "Hopper"
                    && (*x, *z) == (origin.x, origin.z)
                    && *source == resolve_obj(cx, "grain").unwrap() && *id == target.id
            ));
        }
    });
}

#[test]
fn use_on_loc_ready_dispatch_keeps_exact_id_among_same_name_decoys() {
    compile_context_test(|cx| {
        let stand = tile(3170, 3308);
        let (mut snapshot, target, scene) = use_on_footprint_fixture(cx, stand);
        assert_eq!(
            api::query::loc_approach::can_operate_from(&target, &scene, stand),
            Some(true)
        );
        let mut decoy = target.clone();
        decoy.id += 1;
        decoy.distance = 0;
        snapshot.seed_locs(vec![decoy, target.clone()]);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "grain", "target": {"loc": "hopper_lumbridge"}, "radius": 8
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn { target_item_id: Some(id), .. } if *id == target.id
        ));
    });
}

#[test]
fn use_on_loc_unknown_scene_cannot_prove_readiness() {
    compile_context_test(|cx| {
        let (mut snapshot, target, mut scene) = use_on_footprint_fixture(cx, tile(3170, 3308));
        scene.available = false;
        snapshot.seed_scene(scene);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "grain", "target": {"loc": "hopper_lumbridge"}, "radius": 8
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        for tick in 2..=4 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
            assert!(matches!(&ledger.as_ref().unwrap().outbox[0].effect,
                HostEffect::Walk(request) if request.loc_id == Some(target.id)));
        }
    });
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
        steps: Arc::from(vec![CompiledAcquireStep {
            id: FactKey::new("flour-child"),
            advances: false,
            skip_if: Arc::new(AnyPlan { items: vec![] }),
            skip_if_summary: Arc::from("never"),
            settle: Arc::new(Message {
                needles: vec!["grain in the hopper".into()],
            }),
            plan: Arc::new(WaitPlan {
                until: Arc::new(AllPlan { items: vec![] }),
                max_ticks: 2,
            }),
        }]),
        ..AcquirePlan::default()
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
fn policy_s2_acquire_cleans_completed_child_debt_before_advancing() {
    struct CompletedChild(RaisedPrayers);
    impl StepRun for CompletedChild {
        fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
            Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            }))
        }
        fn cancel(&mut self, _: &mut NativeActions) {}
        fn prayer_cleanup(&self) -> RaisedPrayers {
            self.0
        }
    }
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let skin = data.prayer_by_name("Thick Skin").unwrap();
    let protect = data.prayer_by_name("Protect from Melee").unwrap();
    let mut owned = RaisedPrayers::empty();
    owned.accepted(protect.varp, true, 0);
    let varps = |raised| {
        data.prayers()
            .iter()
            .map(|row| api::snapshot::VarpView {
                index: row.varp,
                value: i32::from(row.varp == skin.varp || (raised && row.varp == protect.varp)),
            })
            .collect()
    };
    let mut snapshot = ready();
    snapshot.seed_varps(varps(true));
    let mut ledger = None;
    let mut run = policy_s2_recipe_run(Box::new(CompletedChild(owned)));
    assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(run.prayer_cleanup().contains(protect.varp));
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    let action = ledger.as_mut().unwrap().outbox.pop().unwrap();
    assert!(matches!(
        &action.effect,
        HostEffect::Interaction(InteractReq::IfButton { component_id })
            if *component_id == protect.button_com
    ));
    let authority = action.authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 4,
                sequence: 4,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    snapshot.seed_varps(varps(false));
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Ok(_))
    ));
    assert!(run.prayer_cleanup().is_empty());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert_eq!(
        snapshot
            .varps()
            .iter()
            .find(|row| row.index == skin.varp)
            .unwrap()
            .value,
        1
    );
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
                network: tile(3209, 3215),
                actor: api::snapshot::ActorView {
                    name: None,
                    actions: vec![],
                    tile: tile(3209, 3215),
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
                skill_level: 0,
                headicons: 0,
                weapon: None,
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
    compile_context_test_with_keep(&[], f)
}

fn compile_context_test_with_keep<R>(
    keep_ids: &[i32],
    f: impl FnOnce(&CompileContext<'_>) -> R,
) -> R {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::from_identity(data.quest_identity()).unwrap();
    let path = FactKey::new("cook");
    let progress = test_progress();
    f(&CompileContext {
        path: &path,
        kind: crate::quester::path::PathKind::Quest,
        pair: None,
        progress: &progress,
        selected: &data,
        quests: &quests,
        gathering: None,
        areas: &Default::default(),
        recipes: &Default::default(),
        bank: None,
        bank_required: false,
        keep_ids,
        loadouts: &crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([])),
    })
}

#[test]
fn unequip_all_keeps_shared_protected_equipment() {
    let protected_id = 7;
    let plan = compile_context_test_with_keep(&[protected_id], |cx| {
        super::s2::compile_unequip(
            test_args::<s2::EquipArgs>(serde_json::json!({"all": true})),
            cx,
        )
        .unwrap()
    });
    let equipment = |id, name| ItemView {
        def: def(id, name),
        container: ItemContainer::Equipment,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![],
        component_id: 0,
    };
    let mut snapshot = ready();
    snapshot.seed_equipment(vec![
        equipment(protected_id, "Protected"),
        equipment(42, "Other"),
    ]);
    let mut ledger = None;
    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });

    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Unequip { name } if name == "Other"
    ));
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
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
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
        let present = compile_npc_present(
            test_args::<NpcArg>(serde_json::json!({"npc":"king_bolren"})),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        with_tick(&s, &mut ledger, 1, |t| {
            assert_eq!(
                present.evaluate(&PredicateContext {
                    cx: &t.cx,
                    pairs: t.pairs,
                    quests: cx.quests,
                    progress: &[],
                    required_after: t.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                }),
                Truth::True
            );
        });
        for plan in [
            compile_talk(
                test_args::<TalkArgs>(serde_json::json!({"npc":"king_bolren"})),
                cx,
            )
            .unwrap(),
            compile_interact(
                test_args::<InteractArgs>(
                    serde_json::json!({"target":{"npc":"king_bolren"},"op":"Talk-to"}),
                ),
                cx,
            )
            .unwrap(),
        ] {
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
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
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
        let plan = compile_talk(
            test_args::<TalkArgs>(serde_json::json!({"npc":"king_bolren"})),
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
                assert_eq!(request.target, tile(2542, 3170));
                assert_eq!(request.radius, 1);
            }
            HostEffect::Interaction(_) | HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
                panic!("talked before approaching the NPC")
            }
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
fn use_on_chases_a_distant_npc_without_returning_to_the_initial_anchor() {
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
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
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
        far.seed_local_player(local_player(tile(3200, 3273)));
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
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears",
                "target": {"npc": "sheepunsheered"},
                "anchor": {"tile": [3200, 3270, 0], "source": "test"},
                "radius": 4
            })),
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
            HostEffect::Interaction(_) | HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
                panic!("used the item before approaching the NPC")
            }
        }
        // Approaching the NPC leaves the initial search radius. The next poll
        // must use the adjacent NPC, not send the player back to the anchor.
        far.seed_local_player(local_player(tile(3200, 3275)));
        far.seed_npcs(vec![npc(1)]);
        post_user_input_walk_receipt(&mut ledger, 3);
        ledger.as_mut().unwrap().walk.as_mut().unwrap().end = WalkEnd::Arrived;
        assert!(with_tick(&far, &mut ledger, 3, |t| {
            with_step(t, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.last().unwrap().effect,
                HostEffect::Interaction(_)
            ),
            "walked back to the anchor instead of using the adjacent NPC"
        );
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
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
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
                test_args::<UseOnArgs>(serde_json::json!({
                    "item": "shears",
                    "target": {"npc": "sheepunsheered"},
                    "radius": 8,
                    "product": "wool",
                    "no_product": {"Fact": {"kind": "message", "version": 1, "args": {"any": ["The sheep manages to get away from you!"]}}},
                    "settle_ms": 240_000
                })),
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
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(vec![shears.clone()], 28);
        // The target is held to isolate round settlement from movement. This
        // drives the same compiled UseOn machine used by the sheep Path.
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears", "target": {"item": "shears"},
                "until": {"obj": "wool", "qty": 20}, "settle_ms": 240_000,
                "no_product": {"Fact": {"kind": "message", "version": 1, "args": {
                    "any": ["The sheep manages to get away from you!"]
                }}}
            })),
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
fn use_on_until_retries_a_silent_round_without_waiting_for_step_timeout() {
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
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(vec![shears], 28);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears", "target": {"item": "shears"},
                "until": {"obj": "wool", "qty": 20}, "settle_ms": 240_000
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
        let first_id = authority.request_id();
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
                chat_since: 0,
            },
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "accepted click without product must wait a bounded round"
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                tick.cx.active_now = Duration::from_secs(12);
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "a silent round must retry inside the until loop, not park until settle_ms"
        );
        let last_id = ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .authority()
            .request_id();
        assert_ne!(
            last_id, first_id,
            "silent round must queue another UseOn before the 240s step timeout"
        );
        assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));
    });
}

#[test]
fn use_on_until_continues_objbox_before_the_next_attempt() {
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
            count: 1,
            ..shears.clone()
        };
        let mut snapshot = ready();
        snapshot.seed_inventory(vec![shears.clone(), wool], 28);
        snapshot.seed_chat_modal(2100, vec!["You get some wool.".into()]);
        snapshot.seed_chat_options(vec![], 2105);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears", "target": {"item": "shears"},
                "until": {"obj": "wool", "qty": 20}, "settle_ms": 240_000
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "begin the shared driver without dispatching another product round"
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                emitted(&ledger),
                InteractReq::ContinueDialog { component_id: None }
            ),
            "objbox from a successful shear must be continued before the next UseOn"
        );
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
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears", "target": {"item": "shears"}, "product": "wool",
                "no_product": {"Fact": {"kind": "message", "version": 1, "args": {"any": ["Nothing is produced."]}}}
            })),
            cx,
        )
            .unwrap();
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
    let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
    f(&mut StepContext {
        tick,
        quests: &quests,
        progress: &progress,
        required_after: evidence,
        banks: &banks,
        choices: &crate::quester::choices::QuestChoices::default(),
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
        snapshot.seed_local_player(local_player(tile(5, 5)));
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
        let bank = api::bank_memory::BankMemory::seeded(&[], api::bank_memory::Origin::Session);
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
            with_tick_bank(&snapshot, Some(&bank), &mut None, 1, |tick| {
                let progress = [counted_sheep_progress(cx.selected, tick.cx.evidence(), 20)];
                let context = PredicateContext {
                    cx: &tick.cx,
                    pairs: tick.pairs,
                    quests: cx.quests,
                    progress: &progress,
                    required_after: tick.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                };
                let crate::quester::select::SelectionDecision::Selected(selection) =
                    crate::quester::select::select(&path, 1, 0, &context)
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
        let plan = compile_wait(test_args::<WaitArgs>(args), cx).unwrap();
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
        assert!(serde_json::from_value::<WaitArgs>(serde_json::json!({})).is_err());
        let zero_bound =
            test_args::<WaitArgs>(serde_json::json!({"until":{"All":[]},"max_ticks":0}));
        assert!(compile_wait(zero_bound, cx).is_err());
        assert!(serde_json::from_value::<WaitArgs>(
            serde_json::json!({"until":{"All":[]},"max_ticks":"three"})
        )
        .is_err());
    });
}

#[test]
fn wait_message_until_requires_an_event_after_begin() {
    compile_context_test(|cx| {
        let plan = compile_wait(test_args::<WaitArgs>(serde_json::json!({"until":{"Fact":{"kind":"message","version":1,"args":{"any":["ready"]}}},"max_ticks":4})),cx).unwrap();
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
        let message = compile_message(
            test_args::<MessageArg>(serde_json::json!({"any":["grain in the hopper"]})),
            cx,
        )
        .unwrap();
        let state = compile_message_state(
            test_args::<MessageStateArg>(
                serde_json::json!({"set":["grain in the hopper"],"clear":["hopper is empty"]}),
            ),
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
                pairs: t.pairs,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
            };
            assert_eq!(message.evaluate(&pred), Truth::False);
            assert_eq!(state.evaluate(&pred), Truth::False);
        });
        assert!(
            compile_message(test_args::<MessageArg>(serde_json::json!({"any":[""]})), cx).is_err()
        );
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
    let mut controls = loc(2718, "Hopper controls", "Operate");
    controls.tile.level = 2;
    let mut hopper = loc(2714, "Hopper", "Use");
    hopper.tile.level = 2;
    s.seed_local_player(local_player(controls.tile));
    s.seed_locs(vec![controls, hopper]);
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
    let mut controls = loc(2718, "Hopper controls", "Operate");
    controls.tile.level = 2;
    s.seed_local_player(local_player(controls.tile));
    s.seed_locs(vec![controls]);
    let plan = compile_context_test(|cx| {
        let document = crate::quester::compile::decode_cook().unwrap();
        let recipe = &document.quest.as_ref().unwrap().acquire["acquire:flour"];
        with_tick(&s, &mut None, 1, |t| {
            let pred = PredicateContext {
                cx: &t.cx,
                pairs: t.pairs,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
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
                pairs: t.pairs,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
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
                pairs: t.pairs,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
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
                pairs: t.pairs,
                quests: cx.quests,
                progress: &progress,
                required_after: EvidenceStamp {
                    tick: evidence.tick + 10,
                    ..evidence
                },
                chat_since: 0,
                outcome: None,
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
                pairs: t.pairs,
                quests: cx.quests,
                progress: &progress,
                required_after: evidence,
                chat_since: 0,
                outcome: None,
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
    use super::dialogue::Dialogue;
    let mut snapshot = ready();
    snapshot.seed_chat_modal(4882, vec![]);
    seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<Dialogue>(
                dialogue::DialogueArgs {
                    target: dialogue::DialogueTarget::Npc {
                        id: 0,
                        name: Arc::from("Aubury"),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
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
    // The second gap starts at tick 5, so the quiet close completes at tick
    // 9 with the four-tick gap (tick 13 with the old eight-tick gap).
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
    use super::dialogue::Dialogue;
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
                dialogue::DialogueArgs {
                    target: dialogue::DialogueTarget::Npc {
                        id: 0,
                        name: Arc::from("Fred the Farmer"),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
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
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    // The second gap starts at tick 25, so the quiet close completes at
    // tick 29 with the four-tick gap (tick 33 with the old eight-tick gap).
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
    use super::dialogue::Dialogue;
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
                dialogue::DialogueArgs {
                    target: dialogue::DialogueTarget::Npc {
                        id: 0,
                        name: Arc::from("Fred the Farmer"),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
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
    // The gap starts at tick 2 and re-arms at 3, 4, 5, 6, 7, so it completes
    // at tick 11 with the four-tick gap (tick 15 with the old eight-tick gap).
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
            network: tile(3253, 3401),
            actor: api::snapshot::ActorView {
                name: None,
                actions: vec![],
                tile: tile(3253, 3401),
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
                in_combat,
            },
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
    });
}

#[test]
fn dialogue_hostile_close_never_reports_success() {
    use super::dialogue::Dialogue;
    let mut snapshot = ready();
    snapshot.seed_chat_modal(4882, vec![]);
    seed_dialogue_combat(&mut snapshot, false);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<Dialogue>(
                dialogue::DialogueArgs {
                    target: dialogue::DialogueTarget::Npc {
                        id: 0,
                        name: Arc::from("Aubury"),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
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
    use super::dialogue::Dialogue;
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
                    dialogue::DialogueArgs {
                        target: dialogue::DialogueTarget::Npc {
                            id: 0,
                            name: Arc::from("Aubury"),
                        },
                        options: dialogue::DialogueOptions {
                            prefer: Arc::from([]),
                            choose: (continue_component == Some(-1)).then_some(1),
                            ..Default::default()
                        },
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
                    target: dialogue::DialogueTarget::Npc {
                        id: 0,
                        name: Arc::from(npc),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
                },
                &mut t.cx,
            )
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
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
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
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
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
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
                    target: dialogue::DialogueTarget::Npc {
                        id: 758,
                        name: Arc::from("Fred the Farmer"),
                    },
                    options: dialogue::DialogueOptions {
                        prefer: Arc::from([]),
                        choose: None,
                        ..Default::default()
                    },
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
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
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
            if game_tick == 3 {
                accept_last(&mut ledger, game_tick, true);
            }
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
                pairs: cx.tick.pairs,
                quests: cx.quests,
                progress: cx.progress,
                required_after: cx.required_after,
                chat_since: 0,
                outcome: Some(&outcome),
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
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
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
        let plan = compile_talk(
            test_args::<TalkArgs>(serde_json::json!({"npc":"fred_the_farmer"})),
            cx,
        )
        .unwrap();
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
            HostEffect::Interaction(_) | HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
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
fn walk_protection_warning_is_status_not_a_terminal_or_rewalk() {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(3100, 3200)));
    let mut ledger = None;
    let plan = WalkPlan {
        tile: tile(3200, 3200),
        radius: 1,
        options: crate::native::WalkOptions::default(),
        cross: Box::default(),
        protect: true,
    };
    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    let (authority, request_id) = {
        let action = &ledger.as_ref().unwrap().outbox[0];
        (action.authority(), action.request_id.get())
    };
    ledger
        .as_mut()
        .unwrap()
        .walk_events
        .push(crate::native::WalkEvent {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 2,
                sequence: 2,
            },
            kind: crate::native::WalkEventKind::Unprotectable {
                protect: crate::combat::GuardProtect::Missiles,
            },
            detail: Arc::from("Prayer 40 needed for Protect from Missiles"),
        });
    for tick in 2..5 {
        assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let (label, detail) = run
            .waiting_for()
            .expect("warning surfaces through runner status");
        assert_eq!(label, "Walk protection");
        assert_eq!(
            detail.as_ref(),
            "Prayer 40 needed for Protect from Missiles"
        );
        assert!(
            authority.live(),
            "a warning must leave the walk's request live"
        );
        assert_eq!(
            ledger.as_ref().unwrap().outbox.len(),
            1,
            "keep the same walk"
        );
        assert!(
            ledger.as_ref().unwrap().walk_events.is_empty(),
            "consume the event once"
        );
    }
    snapshot.seed_local_player(local_player(tile(3200, 3200)));
    assert!(
        matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| with_step(tick, |cx| run
                .poll(cx))),
            Poll::Ready(Ok(_))
        ),
        "the warned walk can still arrive successfully"
    );
}

#[test]
fn talk_walk_user_input_blocks_before_dialogue_interaction() {
    let snapshot = ready();
    let mut ledger = None;
    let mut run = TalkRun {
        target: dialogue::DialogueTarget::Npc {
            id: 42,
            name: Arc::from("test npc"),
        },
        tile: Some(tile(3200, 3200)),
        leash: 1,
        options: dialogue::DialogueOptions {
            prefer: Arc::from([]),
            choose: None,
            ..Default::default()
        },
        expect_combat: None,
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
            target: dialogue::DialogueTarget::Npc {
                id: 42,
                name: Arc::from("test npc"),
            },
            tile: Some(WorldTile {
                x: 3103,
                z: 3163,
                level: 2,
            }),
            leash: 6,
            options: dialogue::DialogueOptions {
                prefer: Arc::from([]),
                choose: None,
                ..Default::default()
            },
            expect_combat: None,
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
        round_deadline: None,
        chat_since: 0,
        target_tile: None,
        anchored_stand_arrived: false,
        default_dialogue: true,
        dialogue_options: None,
        dialogue: None,
        dialogue_started: None,
        dialogue_completed: false,
        accepted_tick: None,
        scene_activity_observed: false,
        scene_in_range_at_acceptance: false,
        avoid: reach::Avoid::default(),
        chase: None,
        chase_rewalks: CHASE_REWALKS,
        anchor_walk: false,
        retargets_left: reach::RETARGET_LIMIT,
        dispatched: None,
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
#[test]
fn path_walk_permissions_decode_as_tri_state_options() {
    let inherited = super::parse_walk_plan(test_args::<WalkArgs>(serde_json::json!({
        "tile": [3224, 3200, 0],
        "source": "walk opt-in test",
        "radius": 1,
    })))
    .unwrap();
    assert_eq!(inherited.options, crate::native::WalkOptions::default());

    let explicit = super::parse_walk_plan(test_args::<WalkArgs>(serde_json::json!({
        "tile": [3224, 3200, 0],
        "source": "walk opt-in test",
        "radius": 1,
        "allow_teleports": true,
        "allow_wilderness": false,
    })))
    .unwrap();
    assert_eq!(
        explicit.options,
        crate::native::WalkOptions {
            allow_teleports: crate::native::WalkBit::Allow,
            allow_wilderness: crate::native::WalkBit::Forbid,
            allow_danger_zones: crate::native::WalkBit::Inherit,
        }
    );
}

#[test]
fn path_walk_radius_omitted_is_one_and_explicit_zero_is_the_exact_tile() {
    let radius = |radius: Option<u16>| {
        let mut args = serde_json::json!({
            "tile": [3224, 3200, 0],
            "source": "walk radius test",
        });
        if let Some(radius) = radius {
            args["radius"] = serde_json::json!(radius);
        }
        super::parse_walk_plan(test_args::<WalkArgs>(args))
            .unwrap()
            .radius
    };
    assert_eq!(radius(None), 1);
    assert_eq!(radius(Some(0)), 0);
    assert_eq!(radius(Some(1)), 1);
    assert_eq!(radius(Some(3)), 3);
}

#[test]
fn path_walk_crossing_and_protection_are_independent() {
    compile_context_test(|cx| {
        for protect in [false, true] {
            let mut args = serde_json::json!({
                "tile": [3224, 3200, 0],
                "source": "walk opt-in test",
                "radius": 1,
            });
            if protect {
                args["guard"] = serde_json::json!("protect");
            } else {
                args["cross"] = serde_json::json!(["death-plateau-throwers"]);
            }
            let plan = super::compile_walk(test_args::<WalkArgs>(args), cx).unwrap();
            let mut snapshot = ready();
            snapshot.seed_local_player(local_player(tile(3100, 3200)));
            let mut ledger = None;
            let _run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the compiled Path must emit a real native walk");
            };
            assert_eq!(request.protect, protect);
            if protect {
                assert!(request.cross.is_empty());
            } else {
                assert_eq!(request.cross.len(), 1);
                assert_eq!(&*request.cross[0], "death-plateau-throwers");
            }
        }
    });
}

#[test]
fn talk_expected_combat_only_hands_off_to_the_authored_opponent() {
    compile_context_test(|compile| {
        let expected = compile
            .selected
            .npc_by_config("desertminingcaptain")
            .unwrap();
        for (target_kind, target_index, opponent_type, attacking_local, succeeds) in [
            (
                api::snapshot::ActorKind::Npc,
                42,
                Some(expected.id as usize),
                true,
                true,
            ),
            (
                api::snapshot::ActorKind::Npc,
                43,
                Some(expected.id as usize),
                true,
                false,
            ),
            (
                api::snapshot::ActorKind::Player,
                42,
                Some(expected.id as usize),
                true,
                false,
            ),
            (api::snapshot::ActorKind::Npc, 42, Some(0), true, false),
            (api::snapshot::ActorKind::Npc, 42, None, true, false),
            (
                api::snapshot::ActorKind::Npc,
                42,
                Some(expected.id as usize),
                false,
                false,
            ),
        ] {
            let plan = compile_talk(
                test_args::<TalkArgs>(serde_json::json!({
                    "npc":"desertminingcaptain",
                    "expect_combat":{"npc":"desertminingcaptain"}
                })),
                compile,
            )
            .unwrap();
            let mut snapshot = ready();
            snapshot.seed_chat_modal(4882, vec!["It's a funny captain...".into()]);
            let mut player = local_player(tile(3270, 3029));
            snapshot.seed_local_player(player.clone());
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            snapshot.seed_chat_modal(-1, vec![]);
            player.player.actor.in_combat = true;
            player.player.actor.target = Some(api::snapshot::ActorTargetView {
                kind: target_kind,
                index: target_index,
            });
            snapshot.seed_local_player(player);
            snapshot.seed_npcs(vec![api::snapshot::NpcView {
                index: 42,
                r#type: opponent_type,
                name: expected.display.clone(),
                actions: vec![],
                tile: tile(3271, 3029),
                distance: 1,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 40,
                total_health: 40,
                face_entity: -1,
                target: attacking_local.then_some(api::snapshot::ActorTargetView {
                    kind: api::snapshot::ActorKind::Player,
                    index: 0,
                }),
                moving: false,
                running: false,
                in_combat: false,
                level: 40,
                size: 1,
                network: tile(3271, 3029),
                x: 0,
                z: 0,
                yaw: 0,
            }]);
            let result = with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            if succeeds {
                let Poll::Ready(Ok(outcome)) = result else {
                    panic!("expected authored captain combat handoff");
                };
                assert!(matches!(
                    outcome.receipt.as_ref().unwrap().as_any().downcast_ref::<TalkReceipt>(),
                    Some(TalkReceipt::HandedToCombat { npc_type, npc_index: 42 })
                        if *npc_type == expected.id
                ));
            } else {
                assert!(matches!(result, Poll::Ready(Err(ActionError::Blocked(_)))));
            }
        }
    });
}

#[test]
fn talk_expected_combat_rejects_unknown_npc_config() {
    compile_context_test(|cx| {
        let result = compile_talk(
            test_args::<TalkArgs>(
                serde_json::json!({"npc":"desertminingcaptain","expect_combat":{"npc":"missing"}}),
            ),
            cx,
        );
        assert!(
            matches!(result, Err(CompileError { code, .. }) if code.as_ref() == "unresolved-npc")
        );
    });
}

#[path = "chooser_tests.rs"]
mod chooser_tests;
#[path = "extension_tests.rs"]
mod extension_tests;
fn with_loadout_context<R>(f: impl FnOnce(&CompileContext<'_>) -> R) -> R {
    compile_context_test(|base| {
        let row = crate::loadouts_store::Loadout::new("cook/disguise")
            .with_slot("torso", "Desert shirt")
            .with_slot("feet", "Desert boots");
        let loadouts =
            crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([row]));
        f(&CompileContext {
            loadouts: &loadouts,
            ..*base
        })
    })
}

fn loadout_test_item(alias: &str, container: ItemContainer, slot: i32) -> ItemView {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let item = selected.item_by_alias(alias).unwrap();
    ItemView {
        def: def(item.id, item.name.as_deref().unwrap()),
        container,
        action_family: if container == ItemContainer::Equipment {
            ItemActionFamily::Component
        } else {
            ItemActionFamily::Held
        },
        slot,
        count: 1,
        actions: vec![],
        component_id: 0,
    }
}

fn loadout_predicate_truth(plan: &dyn PredicatePlan, snapshot: &GameSnapshot) -> Truth {
    let mut ledger = None;
    with_tick(snapshot, &mut ledger, 1, |tick| {
        plan.evaluate(&PredicateContext {
            cx: &tick.cx,
            quests: &api::quest_facts::QuestCatalog::empty(),
            progress: &[],
            required_after: tick.cx.evidence(),
            chat_since: 0,
            outcome: None,
            pairs: None,
        })
    })
}

#[test]
fn loadout_waits_for_inventory_observation_before_bank_plan() {
    compile_context_test(|base| {
        let row = crate::loadouts_store::Loadout::new("cook/carry").with_carry("Lobster", 1);
        let loadouts =
            crate::quester::loadouts::LoadoutOverlay::new(Arc::from([]), Arc::from([row]));
        let cx = CompileContext {
            loadouts: &loadouts,
            ..*base
        };
        let plan = s2::compile_loadout(
            test_args::<s2::LoadoutArgs>(serde_json::json!({"loadout":"carry","at":"nearest"})),
            &cx,
        )
        .unwrap();
        let lobster_id = resolve_obj(&cx, "lobster").unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "Loadout must wait until inventory and equipment are posted"
        );
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "Loadout must not select a bank from unposted observations"
        );
        snapshot.seed_inventory(
            vec![ItemView {
                def: def(lobster_id, "Lobster"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "inventory alone must not stand in for an unposted equipment observation"
        );
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
        snapshot.seed_equipment(vec![]);
        let ready_plan = s2::compile_loadout_ready(
            test_args::<s2::LoadoutArgs>(serde_json::json!({"loadout":"carry"})),
            &cx,
        )
        .unwrap();
        assert_eq!(
            loadout_predicate_truth(ready_plan.as_ref(), &snapshot),
            Truth::True,
            "fixture carries the complete loadout"
        );
        let result = with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        });
        assert!(
            matches!(result, Poll::Ready(Ok(_))),
            "Loadout must settle from fresh observations without a bank request"
        );
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "the observed held carry must not be withdrawn again"
        );
    });
}

#[test]
fn exclusive_loadout_removes_extra_then_equips_held_item_without_banking() {
    with_loadout_context(|cx| {
        let args = serde_json::json!({"loadout":"disguise","exclusive":true});
        let plan = s2::compile_loadout(test_args::<s2::LoadoutArgs>(args.clone()), cx).unwrap();
        let ready_plan = s2::compile_loadout_ready(test_args::<s2::LoadoutArgs>(args), cx).unwrap();
        let shirt = loadout_test_item("desert_shirt", ItemContainer::Equipment, 4);
        let boots = loadout_test_item("desert_boots", ItemContainer::Inventory, 0);
        let helmet = loadout_test_item("rune_full_helm", ItemContainer::Equipment, 0);
        let mut snapshot = ready();
        snapshot.seed_equipment(vec![shirt.clone(), helmet.clone()]);
        snapshot.seed_inventory(vec![boots.clone()], 28);
        assert_eq!(
            loadout_predicate_truth(ready_plan.as_ref(), &snapshot),
            Truth::False
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |step| plan.begin(step).unwrap())
        });
        for tick in 2..=3 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |step| run.poll(step))
            })
            .is_pending());
        }
        assert!(
            matches!(emitted(&ledger), InteractReq::Unequip { name } if name == "Rune full helm")
        );

        let mut held_helmet = helmet;
        held_helmet.container = ItemContainer::Inventory;
        held_helmet.slot = 1;
        snapshot.seed_equipment(vec![shirt.clone()]);
        snapshot.seed_inventory(vec![boots.clone(), held_helmet.clone()], 28);
        for tick in 4..=5 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |step| run.poll(step))
            })
            .is_pending());
        }
        assert!(matches!(emitted(&ledger), InteractReq::Wear { name } if name == "Desert boots"));
        let mut worn_boots = boots;
        worn_boots.container = ItemContainer::Equipment;
        worn_boots.slot = 10;
        snapshot.seed_equipment(vec![shirt, worn_boots]);
        snapshot.seed_inventory(vec![held_helmet], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 6, |tick| {
                with_step(tick, |step| run.poll(step))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(
            loadout_predicate_truth(ready_plan.as_ref(), &snapshot),
            Truth::True
        );
        // Admitting Wear revokes the prior Unequip owner's outbox.
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(ledger.as_ref().unwrap().outbox.iter().all(|action| {
            matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::Wear { .. } | InteractReq::Unequip { .. })
            )
        }));
    });
}

#[test]
fn exclusive_loadout_resumes_partial_outfit_without_stripping_correct_items() {
    with_loadout_context(|cx| {
        let plan = s2::compile_loadout(
            test_args::<s2::LoadoutArgs>(
                serde_json::json!({"loadout":"disguise","exclusive":true}),
            ),
            cx,
        )
        .unwrap();
        let shirt = loadout_test_item("desert_shirt", ItemContainer::Equipment, 4);
        let mut boots = loadout_test_item("desert_boots", ItemContainer::Inventory, 0);
        let mut snapshot = ready();
        snapshot.seed_equipment(vec![shirt.clone()]);
        snapshot.seed_inventory(vec![boots.clone()], 28);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |step| plan.begin(step).unwrap())
        });
        for tick in 2..=3 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |step| run.poll(step))
            })
            .is_pending());
        }
        assert!(matches!(emitted(&ledger), InteractReq::Wear { name } if name == "Desert boots"));
        boots.container = ItemContainer::Equipment;
        boots.slot = 10;
        snapshot.seed_equipment(vec![shirt, boots]);
        snapshot.seed_inventory(vec![], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |step| run.poll(step))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    });
}

#[test]
fn exclusive_loadout_blocks_before_removal_when_inventory_is_full() {
    with_loadout_context(|cx| {
        let plan = s2::compile_loadout(
            test_args::<s2::LoadoutArgs>(
                serde_json::json!({"loadout":"disguise","exclusive":true}),
            ),
            cx,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_equipment(vec![
            loadout_test_item("desert_shirt", ItemContainer::Equipment, 4),
            loadout_test_item("desert_boots", ItemContainer::Equipment, 10),
            loadout_test_item("rune_full_helm", ItemContainer::Equipment, 0),
        ]);
        snapshot.seed_inventory(
            (0..28)
                .map(|slot| loadout_test_item("lobster", ItemContainer::Inventory, slot))
                .collect(),
            28,
        );
        let mut ledger = None;
        let result = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |step| plan.begin(step).unwrap().poll(step))
        });
        assert!(
            matches!(result, Poll::Ready(Err(ActionError::Blocked(reason)))
            if reason.as_ref() == "exclusive loadout: inventory space required to remove worn items")
        );
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    });
}

#[test]
fn exclusive_loadout_can_remove_ammo_into_a_held_stack_with_full_inventory() {
    with_loadout_context(|cx| {
        let plan = s2::compile_loadout(
            test_args::<s2::LoadoutArgs>(
                serde_json::json!({"loadout":"disguise","exclusive":true}),
            ),
            cx,
        )
        .unwrap();
        let mut arrows = loadout_test_item("bronze_arrow", ItemContainer::Equipment, 13);
        arrows.def.stackable = true;
        arrows.count = 20;
        let mut held_arrows = arrows.clone();
        held_arrows.container = ItemContainer::Inventory;
        held_arrows.slot = 27;
        let mut inventory = (0..27)
            .map(|slot| loadout_test_item("lobster", ItemContainer::Inventory, slot))
            .collect::<Vec<_>>();
        inventory.push(held_arrows);
        let mut snapshot = ready();
        snapshot.seed_inventory(inventory, 28);
        snapshot.seed_equipment(vec![
            loadout_test_item("desert_shirt", ItemContainer::Equipment, 4),
            loadout_test_item("desert_boots", ItemContainer::Equipment, 10),
            arrows,
        ]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |step| plan.begin(step).unwrap())
        });
        for tick in 2..=3 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |step| run.poll(step))
            })
            .is_pending());
        }
        assert!(
            matches!(emitted(&ledger), InteractReq::Unequip { name } if name == "Bronze arrow")
        );
    });
}

#[test]
fn equipment_only_requires_exact_observed_set_and_preserves_unknown() {
    with_loadout_context(|cx| {
        let predicate = compile_predicate(
            &PredicateDocument::Fact {
                kind: "equipment_only".into(),
                version: 1,
                args: serde_json::json!({"objs":["desert_shirt","desert_boots"]}),
            },
            cx,
        )
        .unwrap();
        let mut snapshot = ready();
        assert_eq!(
            loadout_predicate_truth(predicate.as_ref(), &snapshot),
            Truth::Unknown
        );
        let shirt = loadout_test_item("desert_shirt", ItemContainer::Equipment, 4);
        let boots = loadout_test_item("desert_boots", ItemContainer::Equipment, 10);
        let helmet = loadout_test_item("rune_full_helm", ItemContainer::Equipment, 0);
        for (worn, expected) in [
            (vec![], Truth::False),
            (vec![shirt.clone()], Truth::False),
            (vec![shirt.clone(), boots.clone(), helmet], Truth::False),
            (vec![shirt, boots], Truth::True),
        ] {
            snapshot.seed_equipment(worn);
            assert_eq!(
                loadout_predicate_truth(predicate.as_ref(), &snapshot),
                expected
            );
        }
        let empty = s2::compile_equipment_only(
            test_args::<s2::EquipmentOnlyArgs>(serde_json::json!({"objs":[]})),
            cx,
        )
        .unwrap();
        assert_eq!(
            loadout_predicate_truth(empty.as_ref(), &snapshot),
            Truth::False
        );
        snapshot.seed_equipment(vec![]);
        assert_eq!(
            loadout_predicate_truth(empty.as_ref(), &snapshot),
            Truth::True
        );
    });
}

/// desertrescue's `stow-extras` skips on `pack_only` with its keep list: true
/// once a `deposit_all` with that list (plus the Path's protected items)
/// would move nothing, false while any other row is held, Unknown unobserved.
#[test]
fn pack_only_allows_listed_and_protected_rows_and_preserves_unknown() {
    compile_context_test(|base| {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let protected = [selected.item_by_alias("bronze_axe").unwrap().id];
        let cx = CompileContext {
            keep_ids: &protected,
            ..*base
        };
        let predicate = compile_predicate(
            &PredicateDocument::Fact {
                kind: "pack_only".into(),
                version: 1,
                args: serde_json::json!({"objs":["coins","shantay_pass","desert_shirt"]}),
            },
            &cx,
        )
        .unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        assert_eq!(
            loadout_predicate_truth(predicate.as_ref(), &snapshot),
            Truth::Unknown
        );
        let row = |alias, slot| loadout_test_item(alias, ItemContainer::Inventory, slot);
        let mut emptied_helm = row("rune_full_helm", 1);
        emptied_helm.count = 0;
        for (pack, expected) in [
            (vec![], Truth::True),
            (vec![row("coins", 0)], Truth::True),
            (vec![row("coins", 0), row("bronze_axe", 1)], Truth::True),
            (
                vec![row("coins", 0), row("rune_full_helm", 1)],
                Truth::False,
            ),
            (vec![emptied_helm, row("desert_shirt", 2)], Truth::True),
        ] {
            snapshot.seed_inventory(pack, 28);
            assert_eq!(
                loadout_predicate_truth(predicate.as_ref(), &snapshot),
                expected
            );
        }
    });
}

/// The CI6 desertrescue park: after the stage-0 buys and `desert-kit`, the
/// pack held only kept rows, yet `stow-extras` (`deposit_all`, empty settle)
/// was reselected every boundary until the watchdog parked it. Its bundled
/// skip must now prove that pack clean, admit every kept row, and still run
/// for anything else.
#[test]
fn desertrescue_stow_extras_skips_the_observed_clean_pack() {
    compile_context_test(|cx| {
        let document: crate::quester::path::PathDocument =
            serde_json::from_str(crate::quester::compile::DESERT_RESCUE_JSON).unwrap();
        let stow = document
            .roles
            .iter()
            .flat_map(|role| &role.sequences)
            .flat_map(|sequence| &sequence.steps)
            .find(|step| step.id.0.as_ref() == "stow-extras")
            .unwrap();
        let PredicateDocument::Any(items) = &stow.skip_if else {
            panic!("stow-extras skip_if is an Any");
        };
        let clean = items
            .iter()
            .find(
                |item| matches!(item, PredicateDocument::Fact { kind, .. } if kind == "pack_only"),
            )
            .expect("stow-extras skips once the pack holds only its keep list");
        let predicate = compile_predicate(clean, cx).unwrap();
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let row = |id: i32, slot: i32| ItemView {
            def: def(id, "row"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count: 1,
            actions: vec![],
            component_id: 3214,
        };
        // `EV/.../03-end-fail.json`: coins, passes, waterskins, bars,
        // feathers, hammer, rune scimitar and lobsters.
        let observed = [995, 1854, 1823, 2349, 314, 2347, 1333, 379];
        let mut snapshot = ready();
        snapshot.seed_inventory(
            observed
                .iter()
                .enumerate()
                .map(|(slot, id)| row(*id, slot as i32))
                .collect(),
            28,
        );
        assert_eq!(
            loadout_predicate_truth(predicate.as_ref(), &snapshot),
            Truth::True
        );
        for alias in stow.args["keep"].as_array().unwrap() {
            let id = selected.item_by_alias(alias.as_str().unwrap()).unwrap().id;
            snapshot.seed_inventory(vec![row(id, 0)], 28);
            assert_eq!(
                loadout_predicate_truth(predicate.as_ref(), &snapshot),
                Truth::True,
                "kept row {alias} must not rerun the sweep"
            );
        }
        let helm = selected.item_by_alias("rune_full_helm").unwrap().id;
        snapshot.seed_inventory(vec![row(995, 0), row(helm, 1)], 28);
        assert_eq!(
            loadout_predicate_truth(predicate.as_ref(), &snapshot),
            Truth::False
        );
    });
}

#[test]
fn exclusive_loadout_rejects_strip_and_lower_tier_in_step_and_predicate() {
    with_loadout_context(|cx| {
        for conflict in ["strip", "allow_lower_tier"] {
            let mut args = serde_json::json!({"loadout":"disguise","exclusive":true});
            args[conflict] = true.into();
            assert_eq!(
                s2::compile_loadout(test_args::<s2::LoadoutArgs>(args.clone()), cx)
                    .err()
                    .unwrap()
                    .code
                    .as_ref(),
                "exclusive-loadout-requires-exact-items"
            );
            assert_eq!(
                s2::compile_loadout_ready(test_args::<s2::LoadoutArgs>(args), cx)
                    .err()
                    .unwrap()
                    .code
                    .as_ref(),
                "exclusive-loadout-requires-exact-items"
            );
        }
    });
}

#[test]
fn anchored_use_on_loc_reaches_the_loc_from_the_anchor_radius_edge() {
    compile_context_test(|cx| {
        let origin = tile(3167, 3308);
        let from = tile(origin.x - 4, origin.z);
        let (mut snapshot, mut target, _) = use_on_footprint_fixture(cx, from);
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "grain", "target": {"loc": "hopper_lumbridge"},
                "anchor": {"tile": [3166, 3308, 0], "source": "unit fixture"},
                "radius": 3
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().and_then(|ledger| ledger.outbox.first()).map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == origin && request.loc_id == Some(target.id)
                        && request.radius == 1
            ),
            "arrival at the authored radius edge must still approach the actual loc"
        );

        snapshot.seed_local_player(local_player(tile(origin.x + 3, origin.z)));
        target.distance = 3;
        let mut decoy = target.clone();
        decoy.tile.x += 4;
        decoy.distance = 1;
        snapshot.seed_locs(vec![decoy, target.clone()]);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                emitted(&ledger),
                InteractReq::UseOn { x, z, target_item_id: Some(id), .. }
                    if (*x, *z, *id) == (origin.x, origin.z, target.id)
            ),
            "the anchor-selected loc stays pinned after approaching outside the anchor radius"
        );
    });
}

#[test]
fn anchored_use_on_loc_starts_the_settle_window_after_its_initial_stand_approach() {
    compile_context_test(|cx| {
        let origin = tile(3167, 3308);
        let (mut snapshot, mut target, _) =
            use_on_footprint_fixture(cx, tile(origin.x - 7, origin.z));
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "grain", "target": {"loc": "hopper_lumbridge"},
                "anchor": {"tile": [3166, 3308, 0], "source": "unit fixture"},
                "radius": 3
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().and_then(|ledger| ledger.outbox.first()).map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == origin && request.loc_id == Some(target.id)
            ),
            "a visible anchored loc is approached directly, not via the anchor radius"
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.cx.active_now = Duration::from_secs(30);
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "the initial stand approach must not spend the use-on settle window"
        );

        snapshot.seed_local_player(local_player(tile(origin.x + 3, origin.z)));
        target.distance = 3;
        snapshot.seed_locs(vec![target.clone()]);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.cx.active_now = Duration::from_secs(31);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                emitted(&ledger),
                InteractReq::UseOn { x, z, target_item_id: Some(id), .. }
                    if (*x, *z, *id) == (origin.x, origin.z, target.id)
            ),
            "arrival after more than twenty seconds still dispatches the selected loc use"
        );
        assert!(
            matches!(
                with_tick(&snapshot, &mut ledger, 5, |tick| {
                    tick.cx.active_now = Duration::from_secs(52);
                    with_step(tick, |cx| run.poll(cx))
                }),
                Poll::Ready(Err(ActionError::Failed(message))) if message.as_ref() == "use_on timeout"
            ),
            "the actual use-on settle window remains bounded after arrival"
        );
    });
}

#[test]
fn anchored_use_on_retries_share_the_first_stand_arrival_settle_window() {
    compile_context_test(|cx| {
        let origin = tile(3167, 3308);
        let (mut snapshot, mut target, _) =
            use_on_footprint_fixture(cx, tile(origin.x - 7, origin.z));
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "grain", "target": {"loc": "hopper_lumbridge"},
                "anchor": {"tile": [3166, 3308, 0], "source": "unit fixture"},
                "radius": 3, "until": {"obj": "wool", "qty": 1},
                "settle_ms": 20_000
            })),
            cx,
        )
        .unwrap();
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.active_now = Duration::from_secs(30);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());

        snapshot.seed_local_player(local_player(tile(origin.x + 3, origin.z)));
        target.distance = 1;
        snapshot.seed_locs(vec![target.clone()]);
        post_user_input_walk_receipt(&mut ledger, 4);
        ledger.as_mut().unwrap().walk.as_mut().unwrap().end = WalkEnd::Arrived;
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.cx.active_now = Duration::from_secs(31);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));

        accept_last(&mut ledger, 5, true);
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.cx.active_now = Duration::from_secs(31);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());

        // Let the silent round expire from away from the stand so its retry
        // must walk back before issuing the next use_on.
        snapshot.seed_local_player(local_player(tile(origin.x + 5, origin.z)));
        target.distance = 5;
        snapshot.seed_locs(vec![target.clone()]);
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            tick.cx.active_now = Duration::from_secs(39);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            ledger.as_ref().and_then(|ledger| ledger.outbox.last()).map(|entry| &entry.effect),
            Some(HostEffect::Walk(request))
                if request.target == origin && request.loc_id == Some(target.id)
        ));

        snapshot.seed_local_player(local_player(tile(origin.x + 3, origin.z)));
        target.distance = 1;
        snapshot.seed_locs(vec![target.clone()]);
        post_user_input_walk_receipt(&mut ledger, 7);
        ledger.as_mut().unwrap().walk.as_mut().unwrap().end = WalkEnd::Arrived;
        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            tick.cx.active_now = Duration::from_secs(45);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));

        accept_last(&mut ledger, 8, true);
        assert!(with_tick(&snapshot, &mut ledger, 8, |tick| {
            tick.cx.active_now = Duration::from_secs(46);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                with_tick(&snapshot, &mut ledger, 9, |tick| {
                    tick.cx.active_now = Duration::from_secs(51);
                    with_step(tick, |cx| run.poll(cx))
                }),
                Poll::Ready(Err(ActionError::Failed(message)))
                    if message.as_ref() == "use_on timeout"
            ),
            "repeated anchored attempts must not extend the settle deadline past the first stand arrival"
        );
    });
}

/// "This specific loc" is an explicit `target.tile` (the cog ladder's
/// contract): same-id locs nearer the player never replace it, before or
/// after the stand approach. Without a tile the target is fungible.
#[test]
fn exact_target_tile_interact_loc_ignores_nearer_same_id_decoys() {
    compile_context_test(|cx| {
        let origin = tile(3167, 3308);
        let (mut snapshot, target, _) = use_on_footprint_fixture(cx, tile(3163, 3308));
        let mut decoy = target.clone();
        decoy.tile.x = 3162;
        decoy.distance = 1;
        snapshot.seed_locs(vec![decoy.clone(), target.clone()]);
        let plan = InteractPlan {
            kind: reach::ReachKind::Loc {
                id: Some(target.id),
                name: None,
            },
            op: Arc::from("Use"),
            tile: Some(tile(3166, 3308)),
            radius: 3,
            wait_if_missing: false,
            settle_ms: None,
            ambiguous: false,
            default_dialogue: false,
            dialogue_options: None,
            until: None,
            target_tile: Some(origin),
            reachable_only: false,
        };
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.first().map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == origin && request.loc_id == Some(target.id)
            ),
            "a same-id loc near the player must not bypass the exact loc and its stand"
        );
        snapshot.seed_local_player(local_player(tile(3170, 3308)));
        decoy.tile.x = 3171;
        snapshot.seed_locs(vec![decoy, target.clone()]);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::Loc {
                x: 3167,
                z: 3308,
                ..
            }
        ));
    });
}

#[test]
fn members_world_predicate_reads_the_bound_profile_fact_not_the_account_flag() {
    let fact = PredicateDocument::Fact {
        kind: "members_world".into(),
        version: 1,
        args: serde_json::json!({}),
    };
    compile_context_test(|cx| {
        let plain = compile_predicate(&fact, cx).unwrap();
        let negated =
            compile_predicate(&PredicateDocument::Not(Box::new(fact.clone())), cx).unwrap();
        // The account flag the server sent at login is true in every case:
        // a members account on a free-to-play world must still read False.
        let mut snapshot = ready();
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..Default::default()
        });
        for (world, expected) in [
            (Truth::True, Truth::True),
            (Truth::False, Truth::False),
            (Truth::Unknown, Truth::Unknown),
        ] {
            with_tick_world(&snapshot, world, &mut None, 1, |t| {
                let context = PredicateContext {
                    cx: &t.cx,
                    pairs: t.pairs,
                    quests: cx.quests,
                    progress: &[],
                    required_after: t.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                };
                assert_eq!(plain.evaluate(&context), expected, "{world:?}");
                assert_eq!(negated.evaluate(&context), !expected, "not {world:?}");
            });
        }
        // A view with no host fact attached (no bound profile) is Unknown.
        with_tick(&snapshot, &mut None, 1, |t| {
            let context = PredicateContext {
                cx: &t.cx,
                pairs: t.pairs,
                quests: cx.quests,
                progress: &[],
                required_after: t.cx.evidence(),
                chat_since: 0,
                outcome: None,
            };
            assert_eq!(plain.evaluate(&context), Truth::Unknown);
        });
    });
}

#[test]
fn members_world_rejects_arguments() {
    let document = PredicateDocument::Fact {
        kind: "members_world".into(),
        version: 1,
        args: serde_json::json!({"members": true}),
    };
    compile_context_test(|cx| {
        assert_eq!(
            compile_predicate(&document, cx)
                .err()
                .unwrap()
                .code
                .as_ref(),
            "invalid-args"
        );
    });
}

/// Tenzing's door (`death_sherpa_door`, shape 0 angle 2): a straight wall on
/// the east edge of its own tile. The anchored pre-walk goes to the doorstep
/// east of it at radius 0, not to the door tile across the wall, then opens
/// the door from there.
#[test]
fn anchored_interact_wall_door_on_the_east_edge_walks_to_the_doorstep() {
    compile_context_test(|_| {
        let door_tile = tile(2822, 3555);
        let doorstep = tile(2823, 3555);
        let mut door = loc(3745, "Door", "Open");
        door.tile = door_tile;
        door.layer = LocLayer::Wall;
        door.shape = 0;
        door.angle = 2;
        door.distance = 8;
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(2830, 3555)));
        snapshot.seed_locs(vec![door.clone()]);
        let plan = InteractPlan {
            kind: reach::ReachKind::Loc {
                id: Some(door.id),
                name: None,
            },
            op: Arc::from("Open"),
            tile: Some(door_tile),
            radius: 2,
            wait_if_missing: false,
            settle_ms: None,
            ambiguous: false,
            default_dialogue: false,
            dialogue_options: None,
            until: None,
            target_tile: None,
            reachable_only: false,
        };
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.first().map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == doorstep && request.radius == 0
                        && request.loc_id.is_none()
            ),
            "a straight-wall door is approached on its facing side, not its own tile"
        );

        snapshot.seed_local_player(local_player(doorstep));
        door.distance = 1;
        snapshot.seed_locs(vec![door]);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::Loc { x: 2822, z: 3555, action, .. }
                if action.eq_ignore_ascii_case("open")
        ));
    });
}

/// The Priest in Peril crypt gate (`pip_underground_door1`, shape 0 angle 3,
/// the south edge of 3405,9895) is crossed both ways. Returning from the
/// monuments (south) walks to the facing tile 3405,9894; entering from the
/// north keeps the gate tile, which is on the player's side.
#[test]
fn anchored_interact_wall_gate_approaches_from_the_players_side() {
    compile_context_test(|_| {
        let gate_tile = WorldTile {
            x: 3405,
            z: 9895,
            level: 0,
        };
        for (from, target, radius) in [
            (tile(3428, 9891), tile(3405, 9894), 0),
            (tile(3405, 9899), gate_tile, 1),
            (tile(3403, 9895), gate_tile, 1),
        ] {
            let mut gate = loc(3444, "Gate", "Open");
            gate.tile = gate_tile;
            gate.layer = LocLayer::Wall;
            gate.shape = 0;
            gate.angle = 3;
            gate.distance = 4;
            let mut snapshot = ready();
            snapshot.seed_local_player(local_player(from));
            snapshot.seed_locs(vec![gate.clone()]);
            let plan = InteractPlan {
                kind: reach::ReachKind::Loc {
                    id: Some(gate.id),
                    name: None,
                },
                op: Arc::from("Open"),
                tile: Some(gate_tile),
                radius: 2,
                wait_if_missing: false,
                settle_ms: None,
                ambiguous: false,
                default_dialogue: false,
                dialogue_options: None,
                until: None,
                target_tile: None,
                reachable_only: false,
            };
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert!(
                matches!(
                    ledger.as_ref().unwrap().outbox.first().map(|entry| &entry.effect),
                    Some(HostEffect::Walk(request))
                        if request.target == target && request.radius == radius
                            && request.loc_id.is_none()
                ),
                "from {from:?} the gate is approached at {target:?} r{radius}"
            );
        }
    });
}

/// DIAG-STEP-LATENCY regression 3, Priest in Peril's temple door
/// (`priestperiltempledoorl`, shape 0 angle 2: the east edge of 3408,3489).
/// `crypt-exit-temple-door` leaves from inside (anchor 3409,3489) with the
/// player at 3415,3488. The pre-walk must stop at the inside facing tile:
/// walking to the door tile made nav's door transport cross out, the
/// authored Open from the door tile sent the player back in, and the acquire
/// settle timed out. From there exactly one Open is sent. `cell-enter-temple`
/// (anchor 3407,3489, outside) keeps the door tile, which is outside.
#[test]
fn temple_door_exit_prewalks_to_the_inside_stand_and_opens_once() {
    compile_context_test(|_| {
        let door_tile = tile(3408, 3489);
        let inside = tile(3409, 3489);
        let mut door = loc(3489, "Large door", "Open");
        door.tile = door_tile;
        door.layer = LocLayer::Wall;
        door.shape = 0;
        door.angle = 2;
        door.distance = 7;
        let door_id = door.id;
        let plan = |anchor: WorldTile, radius| InteractPlan {
            kind: reach::ReachKind::Loc {
                id: Some(door_id),
                name: None,
            },
            op: Arc::from("Open"),
            tile: Some(anchor),
            radius,
            wait_if_missing: false,
            settle_ms: None,
            ambiguous: false,
            default_dialogue: false,
            dialogue_options: None,
            until: None,
            target_tile: None,
            reachable_only: false,
        };
        let opens = |ledger: &Option<Box<ledger::Ledger>>| {
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .filter(|entry| {
                    matches!(
                        &entry.effect,
                        HostEffect::Interaction(InteractReq::Loc { action, .. })
                            if action.eq_ignore_ascii_case("open")
                    )
                })
                .count()
        };

        let exit = plan(inside, 1);
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(3415, 3488)));
        snapshot.seed_locs(vec![door.clone()]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| exit.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.first().map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == inside && request.radius == 0
                        && request.loc_id.is_none()
            ),
            "the exit pre-walk stops inside instead of routing through the door"
        );
        snapshot.seed_local_player(local_player(inside));
        door.distance = 1;
        snapshot.seed_locs(vec![door.clone()]);
        for tick in 3..6 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
        }
        assert_eq!(opens(&ledger), 1, "one authored Open from the inside stand");
        assert!(matches!(
            emitted(&ledger),
            InteractReq::Loc {
                x: 3408,
                z: 3489,
                ..
            }
        ));

        let enter = plan(tile(3407, 3489), 2);
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(3400, 3490)));
        door.distance = 8;
        snapshot.seed_locs(vec![door]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| enter.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.first().map(|entry| &entry.effect),
                Some(HostEffect::Walk(request))
                    if request.target == door_tile && request.radius == 1
                        && request.loc_id.is_none()
            ),
            "the outside entry keeps the door tile, which is on the player's side"
        );
    });
}

/// The side is the half-plane across the wall edge for each angle: strictly
/// on the facing side walks to the facing tile at r0; the loc's own side, the
/// wall line and diagonal or corner walls (shapes 1-3, 9) keep the loc tile at
/// r1. A footprint loc keeps its own arrival rule.
#[test]
fn loc_walk_request_picks_the_wall_side_by_half_plane_for_every_angle() {
    let origin = tile(10, 10);
    let wall = |shape: i32, angle: i32| {
        let mut wall = loc(1530, "Door", "Open");
        wall.tile = origin;
        wall.layer = LocLayer::Wall;
        wall.shape = shape;
        wall.angle = angle;
        wall
    };
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    let walk = |wall: &LocView, here: Option<WorldTile>| {
        let request = reach::loc_walk_request(wall, here, stamp);
        (request.target, request.radius, request.loc_id)
    };
    // angle, facing tile, far facing-side tile, far loc-side tile, wall-line tile
    for (angle, facing, far_facing, far_own, along) in [
        (0, tile(9, 10), tile(4, 13), tile(16, 7), tile(10, 15)),
        (1, tile(10, 11), tile(7, 16), tile(13, 4), tile(15, 10)),
        (2, tile(11, 10), tile(16, 7), tile(4, 13), tile(10, 5)),
        (3, tile(10, 9), tile(13, 4), tile(7, 16), tile(5, 10)),
    ] {
        let door = wall(0, angle);
        for here in [Some(facing), Some(far_facing), None] {
            assert_eq!(
                walk(&door, here),
                (facing, 0, None),
                "angle {angle} from {here:?}"
            );
        }
        for here in [far_own, along, origin] {
            assert_eq!(
                walk(&door, Some(here)),
                (origin, 1, None),
                "angle {angle} from {here:?}"
            );
        }
    }
    for shape in [1, 2, 3, 9] {
        assert_eq!(
            walk(&wall(shape, 2), Some(tile(16, 10))),
            (origin, 1, None),
            "shape {shape} has no single facing side"
        );
    }
    let mut footprint = wall(10, 0);
    footprint.layer = LocLayer::Ground;
    footprint.width = 2;
    footprint.length = 2;
    footprint.footprint_width = 2;
    footprint.footprint_length = 2;
    assert_eq!(
        walk(&footprint, Some(tile(16, 10))),
        (origin, 1, Some(footprint.id))
    );
}

use super::*;
use crate::native::{ledger, HostEffect, NativeOutput, NativeTick, RetainedMemory, ScriptStatus};
use api::obj_names::ItemDefView;
use api::selected::{ClientRevision, RunKey};
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
    let mut output = Output;
    let mut native = NativeTick {
        actions: &mut actions,
        cx: crate::native::ActionContext {
            evidence,
            pin: &pin,
            snapshot: SnapshotView::new(Some(snapshot), evidence),
            retained: &mut retained,
            action_id: 0,
            active_now: Duration::from_millis(tick * 600),
            wall_now: Instant::now(),
            ledger,
            budget: &mut budget,
            eligible: true,
        },
        output: &mut output,
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
fn def(id: i32, name: &str) -> ItemDefView {
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
fn with_step<R>(t: &mut NativeTick<'_>, f: impl FnOnce(&mut StepContext<'_, '_>) -> R) -> R {
    let quests = api::quest_facts::QuestCatalog::empty();
    let evidence: Arc<dyn api::quest_progress::EvidenceProvider> =
        Arc::new(crate::quester::progress::LiveEvidence {
            colours: vec![],
            varps: vec![],
            stamp: t.cx.evidence(),
        });
    let required_after = t.cx.evidence();
    f(&mut StepContext {
        tick: t,
        quests: &quests,
        progress: &[],
        required_after,
        walk_evidence: &evidence,
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
fn interact_spawn_wait_outlives_the_click_deadline() {
    let s = ready();
    let mut ledger = None;
    let plan = InteractPlan {
        kind: egg(true).kind,
        op: Arc::from("Take"),
        tile: None,
        radius: 2,
        wait_if_missing: true,
        settle_ms: Some(20_000),
    };
    let mut run = with_tick(&s, &mut ledger, 1, |t| {
        with_step(t, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&s, &mut ledger, 2, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(with_tick(&s, &mut ledger, 100, |t| with_step(t, |cx| run.poll(cx))).is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}
#[test]
fn use_on_waits_for_visibility_and_uses_resolved_inventory_identity() {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = api::quest_facts::QuestCatalog::empty();
    let areas = Default::default();
    let recipes = Default::default();
    let compile = CompileContext {
        selected: &data,
        quests: &quests,
        gathering: None,
        areas: &areas,
        recipes: &recipes,
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
            skip_if: Arc::new(AnyPlan { items: vec![] }),
            settle: Arc::new(Message {
                needles: vec!["grain in the hopper".into()],
            }),
            plan: Arc::new(WaitPlan { max_ticks: 1 }),
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

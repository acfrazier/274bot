use super::*;
use serde_json::json;

fn timed_plan(max_ticks: u32) -> Arc<dyn StepPlan> {
    compile_context_test(|cx| {
        compile_interact(
            test_args(json!({
                "target": {"ground": "egg"}, "op": "Take", "radius": 2,
                "max_ticks": max_ticks
            })),
            cx,
        )
        .unwrap()
    })
}

#[test]
fn timed_ground_compiler_rejects_bad_inputs() {
    if !api::test_support::game_data_available() {
        return;
    }
    compile_context_test(|cx| {
        for args in [
            json!({"target":{"ground":"egg"},"op":"Take","max_ticks":0}),
            json!({"target":{"ground":"not_a_real_obj"},"op":"Take","max_ticks":100}),
            json!({"target":{"held":"egg"},"op":"Take","max_ticks":100}),
            json!({"target":{"ground":"egg"},"op":"Examine","max_ticks":100}),
            json!({"target":{"ground":"egg"},"op":"Take","max_ticks":100,"settle_ms":3000}),
            json!({"target":{"ground":"egg"},"op":"Take","max_ticks":100,"until":{"obj":"egg","qty":2}}),
        ] {
            assert!(compile_interact(test_args(args), cx).is_err());
        }
    });
}

#[test]
fn timed_ground_budget_uses_observed_ticks_not_host_time_or_poll_count() {
    if !api::test_support::game_data_available() {
        return;
    }
    let snapshot = ready();
    let mut ledger = None;
    let plan = timed_plan(100);
    let mut run = with_tick(&snapshot, &mut ledger, 10, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    for _ in 0..150 {
        assert!(with_tick(&snapshot, &mut ledger, 10, |tick| {
            tick.cx.active_now = Duration::from_secs(99_999);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
    }
    assert!(with_tick(&snapshot, &mut ledger, 109, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(matches!(with_tick(&snapshot, &mut ledger, 110, |tick| {
        with_step(tick, |cx| run.poll(cx))
    }), Poll::Ready(Err(ActionError::Blocked(reason)))
        if reason.as_ref() == "ground pickup exceeded its tick budget"));
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

fn hold_egg(snapshot: &mut GameSnapshot) {
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(1944, "Egg"),
            count: 1,
            slot: 0,
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
}

#[test]
fn timed_ground_takes_late_spawn_and_receipts_inventory_gain() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(3229, 3302)));
    snapshot.seed_ground_items(vec![]);
    let mut ledger = None;
    let plan = timed_plan(100);
    let mut run = with_tick(&snapshot, &mut ledger, 10, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&snapshot, &mut ledger, 11, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_ground_items(vec![ground()]);
    assert!(with_tick(&snapshot, &mut ledger, 109, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(
        matches!(emitted(&ledger), InteractReq::Obj { id: Some(1944), action, .. } if action == "Take")
    );
    accept_last(&mut ledger, 109, true);
    hold_egg(&mut snapshot);
    let outcome = with_tick(&snapshot, &mut ledger, 110, |tick| {
        with_step(tick, |cx| run.poll(cx))
    });
    let Poll::Ready(Ok(outcome)) = outcome else {
        panic!("inventory at budget boundary must settle")
    };
    let receipt = outcome.receipt.unwrap();
    let receipt = receipt
        .as_any()
        .downcast_ref::<super::super::timed_ground::TimedGroundReceipt>()
        .unwrap();
    assert_eq!((receipt.waited_ticks, receipt.held), (100, 1));
}

#[test]
fn timed_ground_refuses_inventory_first_seen_after_deadline() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut snapshot = ready();
    let mut ledger = None;
    let plan = timed_plan(100);
    let mut run = with_tick(&snapshot, &mut ledger, 10, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    hold_egg(&mut snapshot);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 111, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Err(ActionError::Blocked(_)))
    ));
}

//! A guardian hold interrupts an in-flight native action: the guardian sends
//! its own interaction, and the host drops the queued native work of the held
//! pass (`script_observe.rs:1335-1344`, `ledger.rs` `HostCustody`). These tests
//! drive the real action machines (Talk's Dialogue, Use-on, Thieve, Walk)
//! through such a hold on a budget that outlives the tick, the way the slot's
//! does, and count the requests each one sends
//! (QUESTER-READ-KEEPS-STEP-R2, finding R1).
//!
//! The contract for every machine: nothing is sent on the first observed tick
//! after the hold (the guardian's last request may still be taking effect);
//! on the next, a request that was lost or taken over is sent again exactly
//! once, and only if the machine's own goal is not already met.
use super::super::dialogue::Dialogue;
use super::super::dialogue_main_tests::{
    content_npc_id, continues, cook_away_from_player, cook_beside_player, npc_args_for, talks,
};
use super::*;
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::ledger::TickBudget;
use crate::native::{take_resume_log, ActionHandle};

/// The guardian takes the slot with one request queued: the host drops it
/// unsent, and the hold ends.
fn guardian_hold_that_drops_the_queue(
    ledger: &mut Option<Box<ledger::Ledger>>,
    budget: &mut TickBudget,
) {
    budget.hold_begin();
    assert_eq!(host_drops_queued_work(ledger), 1, "one request was queued");
    budget.hold_end();
}

/// The guardian takes the slot after the host took and accepted the request:
/// its own interaction replaces the one the server was running.
fn guardian_hold_over_an_accepted_request(budget: &mut TickBudget) {
    budget.hold_begin();
    budget.hold_end();
}

fn poll_talk(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    budget: &mut TickBudget,
    tick: u64,
    handle: &ActionHandle<Dialogue>,
) -> Poll<Result<DialogueOutcome, ActionError>> {
    with_tick_budget(page, ledger, budget, tick, |tick| {
        tick.actions.poll(handle, &mut tick.cx)
    })
}

fn begin_cook_talk(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    budget: &mut TickBudget,
) -> ActionHandle<Dialogue> {
    with_tick_budget(page, ledger, budget, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(content_npc_id("cook"), "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    })
}

fn cook_page_open() -> GameSnapshot {
    let mut opened = cook_beside_player();
    opened.seed_chat_modal(100, vec!["What can I do for you?".to_owned()]);
    opened.seed_chat_options(vec![], 101);
    opened
}

/// R1, Talk, request dropped unsent: the click never reached the server. The
/// settle tick sends nothing; the next sends one new click.
#[test]
fn a_talk_dropped_unsent_by_a_guardian_hold_is_sent_again_once() {
    let page = cook_beside_player();
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = begin_cook_talk(&page, &mut ledger, &mut budget);
    assert_eq!(talks(&ledger), 1);
    let first = ledger.as_ref().unwrap().outbox[0].request_id;
    guardian_hold_that_drops_the_queue(&mut ledger, &mut budget);

    assert!(poll_talk(&page, &mut ledger, &mut budget, 9, &handle).is_pending());
    assert_eq!(
        talks(&ledger),
        0,
        "the first observed tick after the hold sends nothing"
    );
    assert!(poll_talk(&page, &mut ledger, &mut budget, 10, &handle).is_pending());
    assert_eq!(talks(&ledger), 1, "one click is sent again");
    assert_ne!(ledger.as_ref().unwrap().outbox[0].request_id, first);
    assert!(poll_talk(&page, &mut ledger, &mut budget, 11, &handle).is_pending());
    assert_eq!(
        talks(&ledger),
        1,
        "the host holds the new click: no second one"
    );
    let log = take_resume_log();
    assert_eq!(log.len(), 1, "one resume line: {log:?}");
    assert!(
        log[0].contains("re-issued its request once: Talk-to"),
        "{log:?}"
    );
}

/// R1, Talk, accepted then interrupted: the server took the click, then the
/// guardian's own interaction replaced it, so the player never reached the NPC
/// and no page opens. The first-page deadline (tick 6 here) would end the
/// action as "did not open"; the resume sends one new click instead.
#[test]
fn an_accepted_talk_whose_dialogue_never_opened_is_sent_again_not_failed_at_its_deadline() {
    let page = cook_beside_player();
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = begin_cook_talk(&page, &mut ledger, &mut budget);
    let taken = host_takes_queued_work(&mut ledger);
    assert_eq!(taken.len(), 1);
    host_accepts(&mut ledger, &taken[0].0, 1);
    for tick in 2..4 {
        assert!(poll_talk(&page, &mut ledger, &mut budget, tick, &handle).is_pending());
    }
    guardian_hold_over_an_accepted_request(&mut budget);

    assert!(poll_talk(&page, &mut ledger, &mut budget, 9, &handle).is_pending());
    assert_eq!(talks(&ledger), 0, "the settle tick sends nothing");
    let resumed = poll_talk(&page, &mut ledger, &mut budget, 10, &handle);
    assert!(
        resumed.is_pending(),
        "the deadline of the accepted click passed during the hold: {resumed:?}"
    );
    assert_eq!(talks(&ledger), 1, "exactly one click is sent again");
    let again = host_takes_queued_work(&mut ledger);
    host_accepts(&mut ledger, &again[0].0, 10);

    // The conversation then opens and is driven as on any Talk.
    let opened = cook_page_open();
    assert!(poll_talk(&opened, &mut ledger, &mut budget, 12, &handle).is_pending());
    assert_eq!(continues(&ledger), 1);
    assert_eq!(talks(&ledger), 0, "no further Talk-to");
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("re-issued its request once: Talk-to")),
        "{log:?}"
    );
}

/// R1, no re-issue when the goal is already reached: the page is open when
/// the hold ends, so the dialogue is driven and no second Talk-to goes out.
#[test]
fn a_talk_whose_page_is_open_after_the_hold_is_not_sent_again() {
    let page = cook_beside_player();
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = begin_cook_talk(&page, &mut ledger, &mut budget);
    let taken = host_takes_queued_work(&mut ledger);
    host_accepts(&mut ledger, &taken[0].0, 1);
    guardian_hold_over_an_accepted_request(&mut budget);

    let opened = cook_page_open();
    assert!(poll_talk(&opened, &mut ledger, &mut budget, 9, &handle).is_pending());
    assert!(poll_talk(&opened, &mut ledger, &mut budget, 10, &handle).is_pending());
    assert_eq!(talks(&ledger), 0, "the page is open: no Talk-to");
    assert_eq!(continues(&ledger), 1, "the open page is driven");
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("kept its request: the dialogue page is open")),
        "{log:?}"
    );
}

/// A Talk-to the host has not yet taken (no pass that may dispatch has run)
/// is the host's to send: the resume neither fails nor doubles it.
#[test]
fn a_talk_the_host_still_holds_after_the_hold_is_not_doubled() {
    let page = cook_beside_player();
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = begin_cook_talk(&page, &mut ledger, &mut budget);
    budget.hold_begin();
    // The click stays queued: the hold did not drop it (it was queued after).
    budget.hold_end();
    for tick in 9..12 {
        assert!(poll_talk(&page, &mut ledger, &mut budget, tick, &handle).is_pending());
    }
    assert_eq!(talks(&ledger), 1, "the one queued click, not a second");
}

fn use_on_snapshot(cx: &CompileContext<'_>, with_product: bool) -> GameSnapshot {
    use_on_snapshot_holding(cx, i32::from(with_product))
}

/// Shears and wool in the pack, and `products` balls of wool (none: no row).
fn use_on_snapshot_holding(cx: &CompileContext<'_>, products: i32) -> GameSnapshot {
    let source_id = resolve_obj(cx, "shears").unwrap();
    let target_id = resolve_obj(cx, "wool").unwrap();
    let product_id = resolve_obj(cx, "ball_of_wool").unwrap();
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
    let mut items = vec![source, target];
    if products > 0 {
        items.push(ItemView {
            def: def(product_id, "Ball of wool"),
            slot: 9,
            count: products,
            ..items[0].clone()
        });
    }
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(3200, 3200)));
    snapshot.seed_inventory(items, 28);
    snapshot
}

fn use_on_plan(cx: &CompileContext<'_>) -> Arc<dyn super::super::super::compile::StepPlan> {
    compile_use_on(
        test_args::<UseOnArgs>(serde_json::json!({
            "item": "shears",
            "target": {"item": "wool"},
            "product": "ball_of_wool",
            "settle_ms": 20_000,
            "dialogue": "none",
        })),
        cx,
    )
    .unwrap()
}

/// The same use with an `until` goal instead of a product: two balls of wool.
fn use_on_until_plan(cx: &CompileContext<'_>) -> Arc<dyn super::super::super::compile::StepPlan> {
    compile_use_on(
        test_args::<UseOnArgs>(serde_json::json!({
            "item": "shears",
            "target": {"item": "wool"},
            "until": {"obj": "ball_of_wool", "qty": 2},
            "settle_ms": 20_000,
            "dialogue": "none",
        })),
        cx,
    )
    .unwrap()
}

fn use_on_requests(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger.as_ref().map_or(0, |ledger| {
        ledger
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::UseOn { .. })
                )
            })
            .count()
    })
}

fn poll_use_on(
    run: &mut Box<dyn super::super::super::compile::StepRun>,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    budget: &mut TickBudget,
    tick: u64,
) -> Poll<Result<StepOutcome, ActionError>> {
    with_tick_budget(snapshot, ledger, budget, tick, |native| {
        with_step(native, |cx| run.poll(cx))
    })
}

/// What a use-on step did through a guardian hold that dropped its queued
/// request unsent: the step sends one request at tick 2 from `before`, the
/// hold drops it, and the step is polled on `after` at ticks 9 (settle), 10
/// (resume) and 11, until it completes.
struct DroppedUseOn {
    /// The new use-on requests queued at ticks 9, 10 and 11.
    sent: [usize; 3],
    /// What each poll returned, up to the one that completed the step.
    polls: Vec<Poll<Result<StepOutcome, ActionError>>>,
    /// The request id of the first send, and of the last one queued.
    first: u64,
    last: Option<u64>,
}

fn use_on_through_a_dropping_hold(
    plan: &Arc<dyn super::super::super::compile::StepPlan>,
    before: &GameSnapshot,
    after: &GameSnapshot,
) -> DroppedUseOn {
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let mut run = with_tick_budget(before, &mut ledger, &mut budget, 1, |native| {
        with_step(native, |step| plan.begin(step).unwrap())
    });
    assert!(poll_use_on(&mut run, before, &mut ledger, &mut budget, 2).is_pending());
    assert_eq!(use_on_requests(&ledger), 1, "the step sends its use");
    let first = ledger.as_ref().unwrap().outbox[0].request_id.get();
    guardian_hold_that_drops_the_queue(&mut ledger, &mut budget);
    let mut sent = [0; 3];
    let mut polls = Vec::new();
    for tick in [9, 10, 11] {
        let before_poll = use_on_requests(&ledger);
        let poll = poll_use_on(&mut run, after, &mut ledger, &mut budget, tick);
        sent[(tick - 9) as usize] = use_on_requests(&ledger) - before_poll;
        let done = poll.is_ready();
        polls.push(poll);
        if done {
            // A finished step is not polled again (`Quester::tick`).
            break;
        }
    }
    let last = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .map(|action| action.request_id.get());
    DroppedUseOn {
        sent,
        polls,
        first,
        last,
    }
}

/// R1, Use-on, request dropped unsent and the goal still unmet:
/// `UseOnAction` waited for a receipt that no pass would ever send. The
/// settle tick sends nothing; on the next, the step checks its goal (no ball
/// of wool yet) and sends exactly one new request.
#[test]
fn a_use_on_dropped_unsent_by_a_guardian_hold_is_sent_again_once() {
    compile_context_test(|cx| {
        let snapshot = use_on_snapshot(cx, false);
        let dropped = use_on_through_a_dropping_hold(&use_on_plan(cx), &snapshot, &snapshot);
        assert_eq!(
            dropped.sent,
            [0, 1, 0],
            "nothing, then one request, then none"
        );
        assert_eq!(dropped.polls.len(), 3);
        assert!(dropped.polls.iter().all(Poll::is_pending));
        assert_ne!(dropped.last, Some(dropped.first), "a new request");
        // M1 (round 3) memory tripwire: the resend is built by the step from
        // its own inputs, so the machine retains only the request id, not a
        // copy of the request (an `InteractReq` with owned strings).
        println!(
            "UseOnAction={} InteractReq={}",
            std::mem::size_of::<UseOnAction>(),
            std::mem::size_of::<InteractReq>()
        );
        assert_eq!(std::mem::size_of::<UseOnAction>(), 8);
        let log = take_resume_log();
        assert!(
            log.iter()
                .any(|line| line
                    .contains("returned its dropped request to its owner: use-on request")),
            "{log:?}"
        );
    });
}

/// R1.1 (round 3): the product came while the hold lasted (the guardian does
/// not make it, but the step's goal is the pack, not the dropped request).
/// The step checks its goal before it sends again: it completes, and no
/// request is sent. On `3d00cc159` the machine sent the dropped request again
/// without the check.
#[test]
fn a_use_on_dropped_unsent_whose_product_came_during_the_hold_is_not_sent_again() {
    compile_context_test(|cx| {
        let dropped = use_on_through_a_dropping_hold(
            &use_on_plan(cx),
            &use_on_snapshot(cx, false),
            &use_on_snapshot(cx, true),
        );
        assert_eq!(dropped.sent, [0, 0, 0], "no replacement request");
        assert!(dropped.polls[0].is_pending(), "the settle tick waits");
        assert_eq!(dropped.polls.len(), 2, "done on the resume tick");
        assert!(
            matches!(dropped.polls[1], Poll::Ready(Ok(_))),
            "the step completes on its goal"
        );
    });
}

/// R1.1 (round 3), the `until` form: the count reached two balls of wool
/// during the hold, so nothing is sent; with the count still short, exactly
/// one request is.
#[test]
fn a_use_on_dropped_unsent_checks_its_until_count_before_it_sends_again() {
    compile_context_test(|cx| {
        let plan = use_on_until_plan(cx);
        let one = use_on_snapshot_holding(cx, 1);
        let reached = use_on_through_a_dropping_hold(&plan, &one, &use_on_snapshot_holding(cx, 2));
        assert_eq!(reached.sent, [0, 0, 0], "the count is reached: no request");
        assert_eq!(reached.polls.len(), 2, "done on the resume tick");
        assert!(
            matches!(reached.polls[1], Poll::Ready(Ok(_))),
            "the step completes on its count"
        );

        let short = use_on_through_a_dropping_hold(&plan, &one, &one);
        assert_eq!(short.sent, [0, 1, 0], "one request, once");
        assert_eq!(short.polls.len(), 3);
        assert!(short.polls.iter().all(Poll::is_pending));
    });
}

/// R1, Use-on, accepted then interrupted: the host accepted the use, the
/// guardian's own interaction replaced it, and the product never came. The
/// step has no live handle after acceptance, so it resumes itself: the round
/// starts over once.
#[test]
fn an_accepted_use_on_whose_product_never_came_is_sent_again_once() {
    compile_context_test(|cx| {
        let snapshot = use_on_snapshot(cx, false);
        let plan = use_on_plan(cx);
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let mut run = with_tick_budget(&snapshot, &mut ledger, &mut budget, 1, |native| {
            with_step(native, |step| plan.begin(step).unwrap())
        });
        assert!(poll_use_on(&mut run, &snapshot, &mut ledger, &mut budget, 2).is_pending());
        let taken = host_takes_queued_work(&mut ledger);
        host_accepts(&mut ledger, &taken[0].0, 2);
        assert!(poll_use_on(&mut run, &snapshot, &mut ledger, &mut budget, 3).is_pending());
        assert_eq!(use_on_requests(&ledger), 0);
        guardian_hold_over_an_accepted_request(&mut budget);

        assert!(poll_use_on(&mut run, &snapshot, &mut ledger, &mut budget, 9).is_pending());
        assert_eq!(use_on_requests(&ledger), 0, "the settle tick sends nothing");
        assert!(poll_use_on(&mut run, &snapshot, &mut ledger, &mut budget, 10).is_pending());
        assert_eq!(
            use_on_requests(&ledger),
            1,
            "the unmet round is dispatched again, once"
        );
        assert!(poll_use_on(&mut run, &snapshot, &mut ledger, &mut budget, 11).is_pending());
        assert_eq!(use_on_requests(&ledger), 1, "and not a second time");
        let log = take_resume_log();
        assert!(
            log.iter()
                .any(|line| line.contains("re-issued its request once: use-on request")),
            "{log:?}"
        );
    });
}

/// R1, no re-issue when the goal was already reached: the effect of the
/// accepted use landed during the hold (the product is in the pack), so the
/// step completes and sends nothing.
#[test]
fn an_accepted_use_on_whose_product_arrived_is_not_sent_again() {
    compile_context_test(|cx| {
        let before = use_on_snapshot(cx, false);
        let after = use_on_snapshot(cx, true);
        let plan = use_on_plan(cx);
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let mut run = with_tick_budget(&before, &mut ledger, &mut budget, 1, |native| {
            with_step(native, |step| plan.begin(step).unwrap())
        });
        assert!(poll_use_on(&mut run, &before, &mut ledger, &mut budget, 2).is_pending());
        let taken = host_takes_queued_work(&mut ledger);
        host_accepts(&mut ledger, &taken[0].0, 2);
        assert!(poll_use_on(&mut run, &before, &mut ledger, &mut budget, 3).is_pending());
        guardian_hold_over_an_accepted_request(&mut budget);

        assert!(poll_use_on(&mut run, &after, &mut ledger, &mut budget, 9).is_pending());
        let done = poll_use_on(&mut run, &after, &mut ledger, &mut budget, 10);
        assert!(matches!(done, Poll::Ready(Ok(_))), "the step completes");
        assert_eq!(use_on_requests(&ledger), 0, "nothing is sent again");
        let log = take_resume_log();
        assert!(
            log.iter()
                .any(|line| line.contains("kept its request: the round's goal is already met")),
            "{log:?}"
        );
    });
}

/// R1, Walk: the host dropped the queued walk before it armed the route, so
/// no walk receipt will ever come. The walk is sent again, once.
#[test]
fn a_walk_dropped_unsent_by_a_guardian_hold_is_sent_again_once() {
    use crate::native::walk::Walk;
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(3200, 3200)));
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let goal = tile(3220, 3200);
    let handle = with_tick_budget(&snapshot, &mut ledger, &mut budget, 1, |tick| {
        let evidence = tick.cx.evidence();
        let handle = tick
            .actions
            .begin::<Walk>(reach::walk_request(goal, 1, None, evidence), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    let walks = |ledger: &Option<Box<ledger::Ledger>>| {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| matches!(&action.effect, HostEffect::Walk(request) if request.target == goal))
            .count()
    };
    assert_eq!(walks(&ledger), 1);
    let first = ledger.as_ref().unwrap().outbox[0].request_id;
    guardian_hold_that_drops_the_queue(&mut ledger, &mut budget);
    let poll = |ledger: &mut Option<Box<ledger::Ledger>>, budget: &mut TickBudget, tick: u64| {
        with_tick_budget(&snapshot, ledger, budget, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    };

    assert!(poll(&mut ledger, &mut budget, 9).is_pending());
    assert_eq!(walks(&ledger), 0, "the settle tick sends nothing");
    assert!(poll(&mut ledger, &mut budget, 10).is_pending());
    assert_eq!(walks(&ledger), 1, "exactly one walk is sent again");
    assert_ne!(ledger.as_ref().unwrap().outbox[0].request_id, first);
    assert!(poll(&mut ledger, &mut budget, 11).is_pending());
    assert_eq!(walks(&ledger), 1, "and no second one");
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("re-issued its request once: walk request")),
        "{log:?}"
    );
}

/// R1, Walk, the host armed the route: it freezes the follow through the hold
/// and carries on from where the player stands, so nothing is sent again.
#[test]
fn a_walk_the_host_armed_is_not_sent_again_after_a_guardian_hold() {
    use crate::native::walk::Walk;
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(3200, 3200)));
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let goal = tile(3220, 3200);
    let handle = with_tick_budget(&snapshot, &mut ledger, &mut budget, 1, |tick| {
        let evidence = tick.cx.evidence();
        let handle = tick
            .actions
            .begin::<Walk>(reach::walk_request(goal, 1, None, evidence), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert_eq!(host_takes_queued_work(&mut ledger).len(), 1);
    guardian_hold_over_an_accepted_request(&mut budget);
    for tick in 9..13 {
        let result = with_tick_budget(&snapshot, &mut ledger, &mut budget, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(result.is_pending(), "tick {tick}");
    }
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the armed route is followed, not sent again"
    );
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("kept its request: the host armed the walk")),
        "{log:?}"
    );
}

/// R1, no re-issue when the goal was already reached: the walk was dropped
/// unsent, but the player already stands at its goal.
#[test]
fn a_dropped_walk_is_not_sent_again_when_the_player_already_stands_at_its_goal() {
    use crate::native::walk::Walk;
    let goal = tile(3220, 3200);
    let mut away = ready();
    away.seed_local_player(local_player(tile(3200, 3200)));
    let mut there = ready();
    there.seed_local_player(local_player(goal));
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = with_tick_budget(&away, &mut ledger, &mut budget, 1, |tick| {
        let evidence = tick.cx.evidence();
        tick.actions
            .begin::<Walk>(reach::walk_request(goal, 1, None, evidence), &mut tick.cx)
            .unwrap()
    });
    guardian_hold_that_drops_the_queue(&mut ledger, &mut budget);
    for tick in [9, 10, 11] {
        let _ = with_tick_budget(&there, &mut ledger, &mut budget, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
    }
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the player stands at the goal: nothing is sent"
    );
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("kept its request: the player already stands at the goal")),
        "{log:?}"
    );
}

// --- Talk's own approach (`Dialogue`, `Phase::Approach | Phase::Plan`) ---

/// Route requests (walks and route assessments) in the outbox.
fn route_requests(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger.as_ref().map_or(0, |ledger| {
        ledger
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Walk(_) | HostEffect::AssessWalk(_)
                )
            })
            .count()
    })
}

fn cook_tile() -> WorldTile {
    WorldTile {
        x: 0,
        z: 5,
        level: 0,
    }
}

/// A Talk to the Cook ten tiles away on the real Dialogue machine. No reach
/// flood reaches the Cook, so it asks the router first (tick 1); the router
/// answers 15 route ticks, and the walk is queued on tick 2 with a budget to
/// observed tick 19 (15 route ticks plus 2 slack ticks).
fn cook_approach_walking(
    ledger: &mut Option<Box<ledger::Ledger>>,
    budget: &mut TickBudget,
) -> ActionHandle<Dialogue> {
    let far = cook_away_from_player(10);
    let handle = with_tick_budget(&far, ledger, budget, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(content_npc_id("cook"), "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert!(matches!(
        last_route_effect(ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile()
    ));
    post_assess_receipt(ledger, 1, 15, None);
    assert!(poll_talk(&far, ledger, budget, 2, &handle).is_pending());
    assert!(matches!(
        last_route_effect(ledger),
        HostEffect::Walk(request) if request.target == cook_tile()
    ));
    assert_eq!(talks(ledger), 0);
    handle
}

/// R1.2 (round 3), Talk's own approach: the hold dropped the queued walk
/// unsent, so no walk receipt will come and the route would only end at its
/// budget. The settle tick sends nothing; the next plans the route again,
/// once (the router first, as no flood reaches the Cook), and the walk
/// follows the router's answer.
#[test]
fn a_talk_approach_dropped_unsent_by_a_guardian_hold_is_planned_again_once() {
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = cook_approach_walking(&mut ledger, &mut budget);
    let far = cook_away_from_player(10);
    budget.hold_begin();
    assert!(
        host_drops_queued_work(&mut ledger) >= 1,
        "the walk was queued"
    );
    budget.hold_end();

    assert!(poll_talk(&far, &mut ledger, &mut budget, 9, &handle).is_pending());
    assert_eq!(route_requests(&ledger), 0, "the settle tick sends nothing");
    assert!(poll_talk(&far, &mut ledger, &mut budget, 10, &handle).is_pending());
    assert_eq!(route_requests(&ledger), 1, "one new route");
    assert!(matches!(
        last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile()
    ));
    for tick in [11, 12] {
        assert!(poll_talk(&far, &mut ledger, &mut budget, tick, &handle).is_pending());
    }
    assert_eq!(route_requests(&ledger), 1, "and no second one");
    post_assess_receipt(&mut ledger, 12, 15, None);
    assert!(poll_talk(&far, &mut ledger, &mut budget, 13, &handle).is_pending());
    assert!(matches!(
        last_route_effect(&ledger),
        HostEffect::Walk(request) if request.target == cook_tile()
    ));
    assert_eq!(talks(&ledger), 0, "the Cook is still out of reach");
    let log = take_resume_log();
    assert_eq!(log.len(), 1, "one resume line: {log:?}");
    assert!(
        log[0].contains("re-issued its request once: approach walk"),
        "{log:?}"
    );
}

/// R1.2 (round 3): the host armed the walk, but the hold outlasted the
/// route's observed-tick budget (tick 19). The route was not slow, the hold
/// spent its ticks: on the resume tick it is planned again once, without the
/// walk's one bounded retry. That retry is still there when the new route
/// ends short (tick 44): a fresh plan, not a failure.
#[test]
fn a_talk_approach_whose_route_budget_ran_out_during_the_hold_is_planned_again_once() {
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = cook_approach_walking(&mut ledger, &mut budget);
    let far = cook_away_from_player(10);
    assert!(!host_takes_queued_work(&mut ledger).is_empty());
    guardian_hold_over_an_accepted_request(&mut budget);

    assert!(poll_talk(&far, &mut ledger, &mut budget, 25, &handle).is_pending());
    assert_eq!(route_requests(&ledger), 0, "the settle tick sends nothing");
    assert!(poll_talk(&far, &mut ledger, &mut budget, 26, &handle).is_pending());
    assert_eq!(
        route_requests(&ledger),
        1,
        "one new route on the resume tick"
    );
    assert!(matches!(
        last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile()
    ));
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("re-issued its request once: approach walk")),
        "{log:?}"
    );
    // The router answers; the walk is queued on tick 27 with a budget to 44.
    post_assess_receipt(&mut ledger, 26, 15, None);
    assert!(poll_talk(&far, &mut ledger, &mut budget, 27, &handle).is_pending());
    assert!(matches!(
        last_route_effect(&ledger),
        HostEffect::Walk(request) if request.target == cook_tile()
    ));
    assert!(!host_takes_queued_work(&mut ledger).is_empty());
    assert!(poll_talk(&far, &mut ledger, &mut budget, 43, &handle).is_pending());
    let ended = poll_talk(&far, &mut ledger, &mut budget, 44, &handle);
    assert!(
        ended.is_pending(),
        "the walk's own retry was not spent by the hold: {ended:?}"
    );
    assert!(poll_talk(&far, &mut ledger, &mut budget, 45, &handle).is_pending());
    assert!(
        matches!(
            last_route_effect(&ledger),
            HostEffect::AssessWalk(request) if request.target == cook_tile()
        ),
        "the fresh plan follows one observed tick later"
    );
}

/// R1.2 (round 3), goal met: the walk was dropped, but the player stands
/// beside the Cook when the hold ends (the guardian walked it there). No
/// route is sent; the Talk-to is.
#[test]
fn a_talk_approach_that_ends_in_reach_after_the_hold_talks_instead_of_walking_again() {
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = cook_approach_walking(&mut ledger, &mut budget);
    budget.hold_begin();
    assert!(host_drops_queued_work(&mut ledger) >= 1);
    budget.hold_end();
    let near = cook_away_from_player(1);

    assert!(poll_talk(&near, &mut ledger, &mut budget, 9, &handle).is_pending());
    assert_eq!((route_requests(&ledger), talks(&ledger)), (0, 0));
    assert!(poll_talk(&near, &mut ledger, &mut budget, 10, &handle).is_pending());
    assert_eq!(route_requests(&ledger), 0, "no route: the Cook is in reach");
    assert_eq!(talks(&ledger), 1, "the Talk-to is sent");
    assert!(poll_talk(&near, &mut ledger, &mut budget, 11, &handle).is_pending());
    assert_eq!((route_requests(&ledger), talks(&ledger)), (0, 1));
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("kept its request: the NPC is already in reach")),
        "{log:?}"
    );
}

/// R1.2 (round 3), valid ownership: the host armed the walk and still
/// follows it, inside its budget. Nothing is sent again.
#[test]
fn a_talk_approach_the_host_still_follows_is_not_planned_again_after_a_hold() {
    let (mut ledger, mut budget) = (None, TickBudget::default());
    let handle = cook_approach_walking(&mut ledger, &mut budget);
    let far = cook_away_from_player(10);
    assert!(!host_takes_queued_work(&mut ledger).is_empty());
    guardian_hold_over_an_accepted_request(&mut budget);
    for tick in 9..=12 {
        assert!(poll_talk(&far, &mut ledger, &mut budget, tick, &handle).is_pending());
    }
    assert_eq!(route_requests(&ledger), 0, "the followed route is kept");
    assert_eq!(talks(&ledger), 0);
    let log = take_resume_log();
    assert!(
        log.iter()
            .any(|line| line.contains("kept its request: the approach is still followed")),
        "{log:?}"
    );
}

// --- The slot's own hold edges and position reports ---

/// Begins one native walk and polls it, as any compiled card does.
struct WalkScript {
    goal: WorldTile,
    handle: Option<ActionHandle<crate::native::walk::Walk>>,
}

impl crate::native::Script for WalkScript {
    fn tick(
        &mut self,
        t: &mut NativeTick<'_>,
    ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
        if self.handle.is_none() {
            let request = reach::walk_request(self.goal, 1, None, t.cx.evidence());
            self.handle = Some(
                t.actions
                    .begin::<crate::native::walk::Walk>(request, &mut t.cx)
                    .unwrap(),
            );
        }
        let _ = t.actions.poll(self.handle.as_ref().unwrap(), &mut t.cx);
        Ok(crate::native::ScriptFlow::Continue)
    }
}

/// Counts the teleports a guardian hold observed, one per hold.
struct TeleportProbe(Arc<std::sync::atomic::AtomicUsize>);

impl crate::native::Script for TeleportProbe {
    fn tick(
        &mut self,
        t: &mut NativeTick<'_>,
    ) -> Result<crate::native::ScriptFlow, crate::native::ScriptFailure> {
        if t.cx.take_hold_teleport() {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(crate::native::ScriptFlow::Continue)
    }
}

fn frame_at(here: WorldTile) -> GameSnapshot {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(here));
    snapshot
}

/// The whole chain on a real slot: the guardian's hold drops the queued walk
/// (the seat's gameplay drop marks its request unsent), the hold's edges move
/// the budget on, and the walk is sent again once, two observed ticks after
/// the hold ended and never twice.
#[test]
fn a_slots_guardian_hold_drops_a_queued_walk_and_the_walk_resumes_once() {
    let goal = tile(3220, 3200);
    let mut slot = test_script_slot(Box::new(WalkScript { goal, handle: None }));
    let start = frame_at(tile(3200, 3200));
    slot_tick(&mut slot, &start, 10);

    slot.seat_guardian_hold(true);
    slot.observe_player_fix(11, Some(&start));
    slot.seat_guardian_hold(false);
    slot.observe_player_fix(12, Some(&start));
    assert!(
        slot.take_native_action().is_none(),
        "the hold dropped the queued walk"
    );

    slot_tick(&mut slot, &start, 13);
    assert!(
        slot.take_native_action().is_none(),
        "the first observed tick after the hold sends nothing"
    );
    slot_tick(&mut slot, &start, 14);
    let action = slot.take_native_action().expect("one walk is sent again");
    assert!(matches!(&action.effect, HostEffect::Walk(request) if request.target == goal));
    assert!(slot.take_native_action().is_none(), "exactly one");
    slot_tick(&mut slot, &start, 15);
    slot_tick(&mut slot, &start, 16);
    assert!(slot.take_native_action().is_none(), "and never a second");
    slot.stop();
}

/// R2: a guardian walk of a few tiles is not a teleport; a jump walking
/// cannot explain in the observed ticks, and a plane change, are. The
/// Maze and Mime events return the player to the tile they left
/// (`macro_event_maze.rs2:15-18`, `macro_event_mime.rs2:9-11`), so the jump is
/// seen between two frames of the hold, not between its ends.
#[test]
fn a_slot_classifies_a_hold_by_the_player_positions_its_frames_show() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hold = |slot: &mut crate::slot::SlotScript, tick: u64, frames: &[WorldTile]| {
        slot.seat_guardian_hold(true);
        for (offset, here) in frames.iter().enumerate() {
            slot.observe_player_fix(tick + offset as u64, Some(&frame_at(*here)));
        }
        slot.seat_guardian_hold(false);
    };
    let origin = tile(3200, 3200);
    let cases: [(&str, Vec<WorldTile>, usize); 5] = [
        (
            "a guardian walk away of a few tiles",
            vec![tile(3202, 3200), tile(3204, 3201), tile(3206, 3201)],
            0,
        ),
        (
            "a teleport that stays away",
            vec![tile(3290, 3300), tile(3290, 3301)],
            1,
        ),
        (
            "a teleport out and back to the same tile",
            vec![tile(2900, 4555), tile(2900, 4556), origin],
            1,
        ),
        ("a plane change", vec![WorldTile { level: 1, ..origin }], 1),
        ("no hold at all", vec![], 0),
    ];
    for (name, frames, expected) in cases {
        let count = Arc::new(AtomicUsize::new(0));
        let mut slot = test_script_slot(Box::new(TeleportProbe(Arc::clone(&count))));
        let start = frame_at(origin);
        slot_tick(&mut slot, &start, 10);
        slot.observe_player_fix(10, Some(&start));
        if !frames.is_empty() {
            hold(&mut slot, 11, &frames);
        }
        slot.observe_player_fix(20, Some(&frame_at(*frames.last().unwrap_or(&origin))));
        slot_tick(&mut slot, &frame_at(*frames.last().unwrap_or(&origin)), 20);
        slot_tick(&mut slot, &frame_at(*frames.last().unwrap_or(&origin)), 21);
        assert_eq!(count.load(Ordering::Relaxed), expected, "{name}");
        slot.stop();
    }
}

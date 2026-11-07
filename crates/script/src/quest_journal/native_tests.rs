use super::*;
use crate::native::{ActionHandle, HostEffect, InteractionReceipt};
use crate::quester::families::tests::with_tick;
use api::snapshot::{GameSnapshot, QuestStatusView, WidgetKind, WidgetRoot, WidgetView};

fn fixture() -> (GameSnapshot, JournalRequest) {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    (
        snapshot,
        JournalRequest {
            quest: FactKey::new("cook"),
            facts: Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap()),
        },
    )
}

pub fn widget(component_id: i32, text: &str) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: 0,
        parent_id: 0,
        root_component_id: ROOT_289,
        root: WidgetRoot::Main,
        type_: 4,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        scroll_height: 0,
        scroll_position: 0,
        hidden: false,
        text: Some(text.into()),
        alternate_text: None,
        button_text: None,
        target_verb: None,
        target_base: None,
        target_mask: 0,
        model_type: 0,
        model_id: 0,
        alternate_model_type: 0,
        alternate_model_id: 0,
        scripts: None,
        script_comparators: None,
        script_operands: None,
        varp_bindings: Vec::new(),
        colour: 0,
        actions: Vec::new(),
        items: Vec::new(),
    }
}

fn acknowledge(ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().unwrap();
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: action.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    action.effect
}

#[test]
fn journal_owns_click_capture_close_and_releases_after_observed_close() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_some());
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        acknowledge(&mut ledger, 2),
        HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
    ));
    // A page in the dispatch frame is not a causal server observation.
    snapshot.seed_main_modal(
        ROOT_289,
        vec![
            widget(8145, "@str@First body"),
            widget(TITLE_289, "@dre@The Cook's Quest"),
        ],
    );
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    // The capture tick also sends the CloseModal (TICK-FIX #6, E-Q6): the
    // click spent tick 2's event, so tick 3's is free.
    with_tick(&snapshot, &mut ledger, 3, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        acknowledge(&mut ledger, 3),
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    snapshot.seed_main_modal(-1, vec![]);
    let read = with_tick(&snapshot, &mut ledger, 5, |t| {
        match t.actions.poll(&handle, &mut t.cx) {
            Poll::Ready(Ok(read)) => read,
            other => panic!("expected journal read, got {other:?}"),
        }
    });
    assert_eq!(read.lines.as_ref(), &[Arc::<str>::from("@str@First body")]);
    assert_eq!(read.acquired.tick, 3);
    assert_eq!(read.closed.tick, 5);
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_none());
}

/// R4S-1: the quiet-read lease carries the machine's own window, in observed
/// ticks, and each new window (the click, the capture) renews it.
#[test]
fn the_quiet_read_lease_carries_the_machine_window_and_renews_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let now = std::time::Instant::now;
    let deadline = |ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick| {
        ledger
            .as_mut()
            .unwrap()
            .quiet_read_wait(now(), std::time::Duration::ZERO, tick)
            .map(|(_, deadline)| deadline)
    };
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    assert_eq!(deadline(&mut ledger, 1), Some(1 + WINDOW_TICKS));
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 2);
    assert_eq!(
        deadline(&mut ledger, 2),
        Some(2 + WINDOW_TICKS),
        "the click arms a new window"
    );
    snapshot.seed_main_modal(
        ROOT_289,
        vec![
            widget(8145, "@str@First body"),
            widget(TITLE_289, "@dre@The Cook's Quest"),
        ],
    );
    with_tick(&snapshot, &mut ledger, 3, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert_eq!(
        deadline(&mut ledger, 3),
        Some(3 + WINDOW_TICKS),
        "the capture and its close arm a new window"
    );
    // The machine fails its read on its own deadline tick, before the host
    // sample of that tick; the lease ends with it.
    let expired = with_tick(&snapshot, &mut ledger, 3 + WINDOW_TICKS, |t| {
        t.actions.poll(&handle, &mut t.cx)
    });
    assert!(matches!(expired, Poll::Ready(Err(_))), "{expired:?}");
    assert_eq!(deadline(&mut ledger, 3 + WINDOW_TICKS), None);
}

/// F2 of `HOLD-MODEL-AB` (model property S6). The journal machine checks its
/// lease, queues its click, and only then does the host read the wall time
/// and sample the lease. The lease's own 10 s wall bound used to run from the
/// begin only: when it passed between the poll and the sample, the sample
/// returned no wait, the host pressed run, and the click's page closed at
/// once. The click now takes a fresh bound from its own poll, so the click
/// never goes out against a hold that ends in the frame that sends it.
#[test]
fn a_click_sent_in_the_frame_where_the_begin_bound_passes_keeps_its_hold() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    // The owner polls just before the begin's 10 s bound, and queues its click.
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(9.9);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    let click = ledger.as_ref().unwrap().outbox.first().expect("the click");
    assert!(matches!(
        click.effect,
        HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
    ));
    // The host reads the wall time after the poll, past the begin's bound.
    let sampled = ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(10.1), Duration::ZERO, 2);
    assert_eq!(
        sampled.map(|(_, deadline)| deadline),
        Some(2 + WINDOW_TICKS),
        "the click goes out with a live hold: the sample returns the wait"
    );
    assert!(
        ledger.as_ref().unwrap().outbox[0].live(),
        "the click keeps its authority with the hold"
    );
    // The hold's bound is the click's, not the begin's: it ends 10 s after the
    // poll that sent the click when the owner arms no later window.
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(19.8), Duration::ZERO, 2)
        .is_some());
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(19.9), Duration::ZERO, 2)
        .is_none());
}

/// The close click of the page takes a fresh hold bound as well.
#[test]
fn a_close_sent_in_the_frame_where_the_hold_bound_passes_keeps_its_hold() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(1.0);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 2);
    snapshot.seed_main_modal(
        ROOT_289,
        vec![
            widget(8145, "@str@First body"),
            widget(TITLE_289, "@dre@The Cook's Quest"),
        ],
    );
    // The capture and the close share a poll, just before the click's bound.
    with_tick(&snapshot, &mut ledger, 3, |t| {
        t.cx.wall_now = at(10.9);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .first()
            .map(|action| &action.effect),
        Some(HostEffect::Interaction(InteractReq::CloseModal))
    ));
    let sampled = ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(11.1), Duration::ZERO, 3);
    assert_eq!(
        sampled.map(|(_, deadline)| deadline),
        Some(3 + WINDOW_TICKS),
        "the close goes out with a live hold"
    );
    assert!(ledger.as_ref().unwrap().outbox[0].live());
}

/// N1 of `HOLD-MODEL-AB`: the poll renews the hold to its own wall time plus
/// 10 s and queues the click, and then the process stalls (a suspended or
/// sleeping machine) until the host reads the wall time, at or after the
/// renewed bound. The sample releases the expired hold. The click is the
/// owner's interaction and the hold is its cover: an expired hold makes it
/// stale. It must not be dispatched, and the owner fails its read at its
/// next poll and takes its own retry. A sample one instant before the bound
/// still holds.
#[test]
fn a_click_queued_before_a_stall_that_outlasts_its_hold_is_never_dispatched() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    // The owner polls just before the begin's bound; the poll renews the
    // hold to 19.9 s and queues the click.
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(9.9);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    let authority = {
        let click = ledger.as_ref().unwrap().outbox.first().expect("the click");
        assert!(matches!(
            click.effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
        ));
        click.authority()
    };
    // One instant before the renewed bound the hold is live and so is the click.
    let sampled = ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(19.89), Duration::ZERO, 2);
    assert_eq!(
        sampled.map(|(_, deadline)| deadline),
        Some(2 + WINDOW_TICKS)
    );
    assert!(
        authority.live(),
        "the click keeps its authority with the hold"
    );
    // The stall ends at the renewed bound. The sample releases the hold.
    let sampled = ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(19.9), Duration::ZERO, 2);
    assert!(sampled.is_none(), "the hold expired at the sample");
    assert!(
        !authority.live(),
        "the expired hold made the click stale: the host's fence drops it"
    );
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| !action.live()),
        "no live action is left for the host to dispatch"
    );
    // The owner sees the expired interaction at its next poll and fails the
    // read, which is its own bounded failure path.
    let failed = with_tick(&snapshot, &mut ledger, 3, |t| {
        t.cx.wall_now = at(19.95);
        t.actions.poll(&handle, &mut t.cx)
    });
    match failed {
        Poll::Ready(Err(ActionError::Failed(reason))) => {
            assert_eq!(&*reason, "journal quiet lease expired")
        }
        other => panic!("expected the expired read to fail, got {other:?}"),
    }
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

/// N1 for the close: the capture and the close share a poll that renews the
/// hold, and the stall outlasts it.
#[test]
fn a_close_queued_before_a_stall_that_outlasts_its_hold_is_never_dispatched() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(1.0);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    // The click was dispatched in time, and its receipt is in.
    acknowledge(&mut ledger, 2);
    snapshot.seed_main_modal(
        ROOT_289,
        vec![
            widget(8145, "@str@First body"),
            widget(TITLE_289, "@dre@The Cook's Quest"),
        ],
    );
    with_tick(&snapshot, &mut ledger, 3, |t| {
        t.cx.wall_now = at(10.9);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    let close = ledger.as_ref().unwrap().outbox.first().expect("the close");
    assert!(matches!(
        close.effect,
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    let authority = close.authority();
    let sampled = ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(20.9), Duration::ZERO, 3);
    assert!(sampled.is_none(), "the hold expired at the sample");
    assert!(!authority.live(), "the close is stale and never goes out");
    let failed = with_tick(&snapshot, &mut ledger, 4, |t| {
        t.cx.wall_now = at(20.95);
        t.actions.poll(&handle, &mut t.cx)
    });
    assert!(
        matches!(failed, Poll::Ready(Err(ActionError::Failed(_)))),
        "{failed:?}"
    );
}

/// A click that the host dispatched before the hold expired is unchanged: its
/// receipt stays, and the expiry cancels nothing.
#[test]
fn a_click_dispatched_before_its_hold_expired_keeps_its_receipt() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(1.0);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 2);
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read_wait(at(11.0), Duration::ZERO, 2)
        .is_none());
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .interaction
            .is_some_and(|receipt| receipt.accepted),
        "the dispatched click keeps its receipt"
    );
    let outcome = with_tick(&snapshot, &mut ledger, 3, |t| {
        t.cx.wall_now = at(11.05);
        t.actions.poll(&handle, &mut t.cx)
    });
    assert!(
        matches!(outcome, Poll::Ready(Err(ActionError::Failed(_)))),
        "{outcome:?}"
    );
}

/// The paint cap is the begin's 10 s and a later window does not move it:
/// the page stops being hidden there, while the hold of the click goes on.
#[test]
fn the_paint_cap_stays_at_the_begin_when_a_click_renews_the_hold() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let t0 = std::time::Instant::now();
    let at = |seconds: f64| t0 + Duration::from_secs_f64(seconds);
    let handle: ActionHandle<JournalMachine> = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.cx.wall_now = t0;
        t.actions.begin(request, &mut t.cx).unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        t.cx.wall_now = at(9.9);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    let ledger = ledger.as_mut().unwrap();
    assert!(ledger.quiet_paint_hidden(at(9.95)), "hidden before the cap");
    assert!(
        !ledger.quiet_paint_hidden(at(10.0)),
        "the page stops being hidden at the cap from the begin"
    );
    assert!(
        ledger.quiet_read(at(10.1)).is_some(),
        "the hold outlives the paint cap"
    );
}

#[test]
fn foreign_modal_busy_wrong_title_never_closed_and_cancel_revokes_click() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    snapshot.seed_chat_modal(123, vec!["dialogue".into()]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |t| {
        assert!(matches!(
            t.actions.begin::<JournalMachine>(request, &mut t.cx),
            Err(ActionError::Busy)
        ))
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    let (mut snapshot, request) = fixture();
    let handle = with_tick(&snapshot, &mut ledger, 2, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 3, |t| {
        let _ = t.actions.poll(&handle, &mut t.cx);
    });
    acknowledge(&mut ledger, 3);
    snapshot.seed_main_modal(ROOT_289, vec![widget(TITLE_289, "@dre@Romeo & Juliet")]);
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(_)))
        ))
    });
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_none());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    let (snapshot, request) = fixture();
    let handle = with_tick(&snapshot, &mut ledger, 5, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 6, |t| {
        let _ = t.actions.poll(&handle, &mut t.cx);
    });
    drop(handle);
    assert!(!ledger.as_ref().unwrap().outbox[0].live());
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_none());
}

#[test]
fn unreadable_and_held_journal_are_bounded_without_hiding_forever() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        let _ = t.actions.poll(&handle, &mut t.cx);
    });
    acknowledge(&mut ledger, 2);
    with_tick(&snapshot, &mut ledger, 100, |t| {
        t.cx.eligible = false;
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending());
    });
    with_tick(&snapshot, &mut ledger, 8, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(_)))
        ))
    });
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_none());
}

#[test]
fn journal_adopts_matching_open_page_without_click() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    snapshot.seed_main_modal(
        ROOT_289,
        vec![
            widget(8145, "@str@Already open"),
            widget(TITLE_289, "@dre@The Cook's Quest"),
        ],
    );
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    // The capture and its CloseModal share tick 2 (TICK-FIX #6, E-Q6).
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .first()
            .map(|action| &action.effect),
        Some(HostEffect::Interaction(InteractReq::CloseModal))
    ));
    acknowledge(&mut ledger, 2);
    snapshot.seed_main_modal(-1, vec![]);
    let read = with_tick(&snapshot, &mut ledger, 3, |t| {
        match t.actions.poll(&handle, &mut t.cx) {
            Poll::Ready(Ok(read)) => read,
            other => panic!("expected journal read, got {other:?}"),
        }
    });
    assert_eq!(
        read.lines.as_ref(),
        &[Arc::<str>::from("@str@Already open")]
    );
    assert_eq!(read.acquired.tick, 2);
    assert_eq!(read.closed.tick, 3);
}

#[test]
fn foreign_modal_begin_names_root_and_unknown_modal_stays_busy() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    snapshot.seed_main_modal(777, vec![]);
    snapshot.seed_main_modal_texts(vec!["Trade confirmation".into()]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |t| {
        assert!(matches!(
            t.actions.begin::<JournalMachine>(request, &mut t.cx),
            Err(ActionError::Failed(reason))
                if reason.contains("root 777") && reason.contains("Trade confirmation")
        ));
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    let (mut snapshot, request) = fixture();
    snapshot.seed_main_modal(ROOT_289, vec![]);
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(matches!(
            t.actions.begin::<JournalMachine>(request, &mut t.cx),
            Err(ActionError::Busy)
        ));
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn journal_overall_deadline_wins_after_phase_progress() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    // Spend most of each phase window without timing out that phase.
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 4);
    snapshot.seed_main_modal(ROOT_289, vec![widget(TITLE_289, "@dre@The Cook's Quest")]);
    with_tick(&snapshot, &mut ledger, 8, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    with_tick(&snapshot, &mut ledger, 12, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 12);
    with_tick(&snapshot, &mut ledger, 15, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    });
}

#[test]
fn journal_quiet_lease_expiry_is_failure_not_cancellation() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    let (lease, owner) = {
        let ledger = ledger.as_ref().unwrap();
        let lease = ledger.quiet_lease.unwrap().lease;
        (lease, Arc::clone(ledger.owner.as_ref().unwrap()))
    };
    owner.release_quiet(lease);
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    });
}

#[test]
fn journal_phase_window_counts_game_ticks_not_host_time() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 2);
    // A host hitch: well over 3 s of active time, but one observed tick.
    with_tick(&snapshot, &mut ledger, 3, |t| {
        t.cx.active_now = Duration::from_millis(5_000);
        assert!(
            t.actions.poll(&handle, &mut t.cx).is_pending(),
            "a frozen snapshot must not spend the acquire window"
        )
    });
    snapshot.seed_main_modal(ROOT_289, vec![widget(TITLE_289, "@dre@The Cook's Quest")]);
    with_tick(&snapshot, &mut ledger, 4, |t| {
        t.cx.active_now = Duration::from_millis(5_600);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    with_tick(&snapshot, &mut ledger, 5, |t| {
        t.cx.active_now = Duration::from_millis(6_200);
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        acknowledge(&mut ledger, 5),
        HostEffect::Interaction(InteractReq::CloseModal)
    ));

    // Five observed ticks without the page still name the modal timeout.
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    acknowledge(&mut ledger, 2);
    for tick in 3..7 {
        with_tick(&snapshot, &mut ledger, tick, |t| {
            assert!(
                t.actions.poll(&handle, &mut t.cx).is_pending(),
                "tick {tick}"
            )
        });
    }
    with_tick(&snapshot, &mut ledger, 7, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "journal modal timeout"
        ))
    });
}

#[test]
fn latched_chat_continue_keeps_the_journal_busy_before_its_click() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut snapshot, request) = fixture();
    snapshot.seed_chat_options(vec![], 105);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |t| {
        assert!(matches!(
            t.actions.begin::<JournalMachine>(request, &mut t.cx),
            Err(ActionError::Busy)
        ))
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    let (mut snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    snapshot.seed_chat_options(vec![], 105);
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Busy))
        ))
    });
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a latched continue must not eat the journal row click"
    );
}

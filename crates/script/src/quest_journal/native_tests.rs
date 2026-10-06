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

pub(crate) fn widget(component_id: i32, text: &str) -> WidgetView {
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
    with_tick(&snapshot, &mut ledger, 3, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    with_tick(&snapshot, &mut ledger, 4, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(matches!(
        acknowledge(&mut ledger, 4),
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    with_tick(&snapshot, &mut ledger, 5, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    snapshot.seed_main_modal(-1, vec![]);
    let read = with_tick(&snapshot, &mut ledger, 6, |t| {
        match t.actions.poll(&handle, &mut t.cx) {
            Poll::Ready(Ok(read)) => read,
            other => panic!("expected journal read, got {other:?}"),
        }
    });
    assert_eq!(read.lines.as_ref(), &[Arc::<str>::from("@str@First body")]);
    assert_eq!(read.acquired.tick, 3);
    assert_eq!(read.closed.tick, 6);
    assert!(ledger
        .as_mut()
        .unwrap()
        .quiet_read(std::time::Instant::now())
        .is_none());
}

#[test]
fn foreign_modal_busy_wrong_title_never_closed_and_cancel_revokes_click() {
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
    with_tick(&snapshot, &mut ledger, 2, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    with_tick(&snapshot, &mut ledger, 3, |t| {
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
    acknowledge(&mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    let read = with_tick(&snapshot, &mut ledger, 4, |t| {
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
    assert_eq!(read.closed.tick, 4);
}

#[test]
fn foreign_modal_begin_names_root_and_unknown_modal_stays_busy() {
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
    let (snapshot, request) = fixture();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    let (lease, owner) = {
        let ledger = ledger.as_ref().unwrap();
        let (_, lease, _) = ledger.quiet_since.unwrap();
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

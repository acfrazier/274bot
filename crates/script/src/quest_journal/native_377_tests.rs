use super::tests::widget;
use super::*;
use crate::native::{ActionHandle, HostEffect};
use crate::quester::families::tests::with_tick;
use api::snapshot::{GameSnapshot, QuestStatusView};

fn acknowledge(ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().unwrap();
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        crate::native::InteractionReceipt {
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

fn tick<R>(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    n: u64,
    f: impl FnOnce(&mut crate::native::NativeTick<'_>) -> R,
) -> R {
    with_tick(snapshot, ledger, n, |tick| {
        tick.cx.revision = ClientRevision::R377;
        f(tick)
    })
}

fn fixture(id: &str) -> (GameSnapshot, JournalRequest, String, i32) {
    let data = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let catalog = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let facts = catalog.quest(id).unwrap();
    let component = revision_377::admit(facts).unwrap();
    let title = facts.journal_title.as_ref().unwrap().to_string();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: facts.display.to_string(),
            component_id: component,
            colour: 0xf8f800,
        }],
        true,
    );
    (
        snapshot,
        JournalRequest {
            quest: FactKey::new(id),
            facts: catalog,
        },
        title,
        component,
    )
}

fn page(snapshot: &mut GameSnapshot, title: &str, body: &[&str]) {
    let mut widgets = vec![widget(revision_377::TITLE, &format!("@dre@{title}"))];
    for index in 0..100 {
        let component = match index {
            0 => 8145,
            1..=49 => 8146 + index,
            _ => 12124 + index,
        };
        let text = body.get(index as usize).copied().unwrap_or("");
        let mut row = widget(component, text);
        if text.is_empty() {
            row.text = None; // Real snapshots omit empty text, not the component.
        }
        widgets.push(row);
    }
    widgets.push(widget(8146, "Not body text"));
    widgets.reverse(); // Capture follows the selected split order, not traversal order.
    snapshot.seed_main_modal(revision_377::ROOT, widgets);
}

#[test]
fn selected_377_captures_the_discontinuous_tail_and_ignores_static_controls() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let (mut snapshot, _, title, _) = fixture("sheep");
    let mut body = [""; 100];
    body[0] = "first";
    body[49] = "before gap";
    body[50] = "after gap";
    body[99] = "last";
    page(&mut snapshot, &title, &body);
    let mut ledger = None;
    tick(&snapshot, &mut ledger, 1, |tick| {
        let lines = revision_377::body(tick.cx.snapshot()).unwrap();
        assert_eq!(lines, body);
    });
}

#[test]
fn selected_377_click_capture_close_is_fresh_ordered_and_normalized() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    for id in [
        "cook",
        "runemysteries",
        "romeojuliet",
        "sheep",
        "demon",
        "gobdip",
        "squire",
        "vampire",
        "hetty",
    ] {
        let (mut snapshot, request, title, component) = fixture(id);
        let mut ledger = None;
        let handle: ActionHandle<JournalMachine> = tick(&snapshot, &mut ledger, 1, |t| {
            t.actions.begin(request, &mut t.cx).unwrap()
        });
        tick(&snapshot, &mut ledger, 2, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
        });
        assert!(
            matches!(acknowledge(&mut ledger, 2), HostEffect::Interaction(InteractReq::IfButton {component_id}) if component_id == component)
        );
        page(
            &mut snapshot,
            &title,
            &["@dbl@I need to collect 7 more", "@dre@balls of wool."],
        );
        tick(&snapshot, &mut ledger, 2, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        tick(&snapshot, &mut ledger, 3, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
        });
        assert!(matches!(
            acknowledge(&mut ledger, 3),
            HostEffect::Interaction(InteractReq::CloseModal)
        ));
        snapshot.seed_main_modal(-1, vec![]);
        tick(&snapshot, &mut ledger, 3, |t| {
            assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
        });
        let read = tick(&snapshot, &mut ledger, 4, |t| {
            match t.actions.poll(&handle, &mut t.cx) {
                Poll::Ready(Ok(read)) => read,
                other => panic!("expected fresh journal read, got {other:?}"),
            }
        });
        assert_eq!(read.root, 8134);
        assert_eq!(read.lines.len(), 2);
        assert_eq!(
            crate::quester::progress::normalize_journal(&read.lines),
            "i need to collect 7 more balls of wool."
        );
        assert_eq!((read.acquired.tick, read.closed.tick), (3, 4));
        assert!(ledger
            .as_mut()
            .unwrap()
            .quiet_read(std::time::Instant::now())
            .is_none());
    }
}

#[test]
fn selected_377_adopts_only_matching_fresh_page_and_refuses_foreign_modals() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    for title in ["Rune Mysteries", "Sheep Shearer Extra"] {
        let (mut snapshot, request, _, _) = fixture("sheep");
        page(&mut snapshot, title, &["body"]);
        let mut ledger = None;
        tick(&snapshot, &mut ledger, 1, |t| {
            assert!(matches!(
                t.actions.begin::<JournalMachine>(request, &mut t.cx),
                Err(ActionError::Failed(_))
            ))
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
    let (mut snapshot, request, title, _) = fixture("sheep");
    page(&mut snapshot, &title, &["body"]);
    let mut ledger = None;
    let handle = tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    tick(&snapshot, &mut ledger, 1, |t| {
        assert!(t.actions.poll(&handle, &mut t.cx).is_pending())
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_chat_modal(123, vec!["foreign dialogue".into()]);
    tick(&snapshot, &mut ledger, 2, |t| {
        assert!(matches!(
            t.actions.poll(&handle, &mut t.cx),
            Poll::Ready(Err(ActionError::Failed(_)))
        ))
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    let (mut snapshot, request, _, _) = fixture("sheep");
    snapshot.seed_main_modal(777, vec![]);
    tick(&snapshot, &mut ledger, 3, |t| {
        assert!(
            matches!(t.actions.begin::<JournalMachine>(request, &mut t.cx), Err(ActionError::Failed(reason)) if reason.contains("777"))
        )
    });
}

#[test]
fn selected_377_missing_body_and_button_tables_fail_closed() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let (mut snapshot, request, title, _) = fixture("sheep");
    let mut ledger = None;
    let handle = tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<JournalMachine>(request, &mut t.cx)
            .unwrap()
    });
    tick(&snapshot, &mut ledger, 2, |t| {
        let _ = t.actions.poll(&handle, &mut t.cx);
    });
    acknowledge(&mut ledger, 2);
    snapshot.seed_main_modal(
        8134,
        vec![widget(8144, &title), widget(8145, "partial page")],
    );
    tick(&snapshot, &mut ledger, 3, |t| {
        assert!(
            matches!(t.actions.poll(&handle, &mut t.cx), Poll::Ready(Err(ActionError::Failed(reason))) if reason.contains("8147 unavailable"))
        )
    });
    assert!(ledger.as_ref().unwrap().outbox.is_empty());

    let data = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let catalog = QuestCatalog::from_identity(data.quest_identity()).unwrap();
    assert!(
        matches!(revision_377::admit(catalog.quest("imp").unwrap()), Err(ActionError::Unavailable(reason)) if reason.contains("button table unavailable"))
    );
}

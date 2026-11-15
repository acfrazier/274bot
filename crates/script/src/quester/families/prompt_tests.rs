use super::*;
use crate::native::{ledger, HostEffect, InteractionReceipt};
use crate::quester::families::tests::with_tick;
use api::snapshot::{GameSnapshot, WidgetKind, WidgetRoot};

const ROOT: i32 = 6675;
const LAYER: i32 = 8420;
const BUTTON: i32 = 8421;

fn widget(component_id: i32, parent_id: i32, hidden: bool, button_type: i32) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: parent_id,
        parent_id,
        root_component_id: ROOT,
        root: WidgetRoot::Main,
        type_: 4,
        button_type,
        client_code: -1,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        scroll_height: 0,
        inventory_ready: false,
        scroll_position: 0,
        hidden,
        text: None,
        alternate_text: None,
        button_text: None,
        target_verb: None,
        target_base: None,
        target_mask: 0,
        model_type: 0,
        model_id: -1,
        alternate_model_type: 0,
        alternate_model_id: -1,
        scripts: None,
        script_comparators: None,
        script_operands: None,
        varp_bindings: Vec::new(),
        colour: 0,
        actions: Vec::new(),
        items: Vec::new(),
    }
}

/// An interface root, a layer under it and a button in the layer: the shape of
/// `death_dice` (`com_27` is the hidden layer, `com_28` its button).
fn tree(layer_hidden: bool, button_type: i32) -> Vec<WidgetView> {
    vec![
        widget(ROOT, -1, false, 0),
        widget(LAYER, ROOT, layer_hidden, 0),
        widget(BUTTON, LAYER, false, button_type),
    ]
}

fn snapshot(count_open: bool, widgets: Vec<WidgetView>) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_count_dialog(count_open);
    snapshot.seed_main_modal(ROOT, widgets);
    snapshot
}

fn begin(
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    op: Op,
    max_ticks: u64,
) -> ActionHandle<Prompt> {
    with_tick(snapshot, ledger, tick, |tick| {
        tick.actions
            .begin::<Prompt>(PromptArgs { op, max_ticks }, &mut tick.cx)
            .unwrap()
    })
}

fn poll(
    handle: &ActionHandle<Prompt>,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> Poll<Result<EvidenceStamp, ActionError>> {
    with_tick(snapshot, ledger, tick, |tick| {
        tick.actions.poll(handle, &mut tick.cx)
    })
}

fn sent(ledger: &Option<Box<ledger::Ledger>>) -> Vec<InteractReq> {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|action| match &action.effect {
            HostEffect::Interaction(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

fn settle(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, accepted: bool) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
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

fn failed(poll: Poll<Result<EvidenceStamp, ActionError>>) -> String {
    match poll {
        Poll::Ready(Err(ActionError::Failed(message))) => message.to_string(),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn count_answer_is_sent_once_when_the_dialog_is_open() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut ledger = None;
    let open = snapshot(true, Vec::new());
    let handle = begin(&open, &mut ledger, 1, Op::Count(101), 4);
    assert!(sent(&ledger).is_empty(), "begin sends nothing");
    assert!(poll(&handle, &open, &mut ledger, 2).is_pending());
    assert_eq!(sent(&ledger), vec![InteractReq::AnswerCount { value: 101 }]);
    // The request is queued: later polls neither resend nor finish.
    assert!(poll(&handle, &open, &mut ledger, 3).is_pending());
    assert_eq!(sent(&ledger).len(), 1);
    settle(&mut ledger, 3, true);
    // Accepted, but the dialog still shows: not finished yet.
    assert!(poll(&handle, &open, &mut ledger, 4).is_pending());
    assert_eq!(sent(&ledger).len(), 1);
    let closed = snapshot(false, Vec::new());
    assert!(matches!(
        poll(&handle, &closed, &mut ledger, 5),
        Poll::Ready(Ok(evidence)) if evidence.tick == 5
    ));
    assert_eq!(sent(&ledger).len(), 1);
}

#[test]
fn count_answer_waits_for_the_dialog_then_fails_at_its_tick_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut ledger = None;
    let closed = snapshot(false, Vec::new());
    let handle = begin(&closed, &mut ledger, 10, Op::Count(5), 3);
    for tick in 11..=12 {
        assert!(poll(&handle, &closed, &mut ledger, tick).is_pending());
    }
    assert!(
        failed(poll(&handle, &closed, &mut ledger, 13)).contains("count answer"),
        "the wait ends at begin tick + max_ticks"
    );
    assert!(sent(&ledger).is_empty(), "no request without a dialog");

    // A dialog that opens on the last allowed tick is still answered.
    let mut ledger = None;
    let handle = begin(&closed, &mut ledger, 10, Op::Count(5), 3);
    let open = snapshot(true, Vec::new());
    assert!(poll(&handle, &open, &mut ledger, 13).is_pending());
    assert_eq!(sent(&ledger), vec![InteractReq::AnswerCount { value: 5 }]);
}

#[test]
fn count_answer_fails_on_a_refused_receipt_and_on_a_dialog_that_stays_open() {
    if !api::test_support::game_data_available() {
        return;
    }
    let open = snapshot(true, Vec::new());
    let mut ledger = None;
    let handle = begin(&open, &mut ledger, 1, Op::Count(7), 4);
    assert!(poll(&handle, &open, &mut ledger, 2).is_pending());
    settle(&mut ledger, 2, false);
    assert!(failed(poll(&handle, &open, &mut ledger, 3)).contains("refused"));

    let mut ledger = None;
    let handle = begin(&open, &mut ledger, 1, Op::Count(7), 4);
    assert!(poll(&handle, &open, &mut ledger, 2).is_pending());
    settle(&mut ledger, 2, true);
    assert!(poll(&handle, &open, &mut ledger, 4).is_pending());
    assert!(failed(poll(&handle, &open, &mut ledger, 5)).contains("stayed open"));
}

#[test]
fn button_is_pressed_only_when_it_and_every_layer_above_it_are_shown() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut ledger = None;
    let hidden = snapshot(false, tree(true, 1));
    let handle = begin(&hidden, &mut ledger, 1, Op::Button(BUTTON), 12);
    for tick in 2..=5 {
        assert!(poll(&handle, &hidden, &mut ledger, tick).is_pending());
    }
    assert!(
        sent(&ledger).is_empty(),
        "a button under a hidden layer is not pressed"
    );
    let shown = snapshot(false, tree(false, 1));
    assert!(poll(&handle, &shown, &mut ledger, 6).is_pending());
    assert_eq!(
        sent(&ledger),
        vec![InteractReq::IfButton {
            component_id: BUTTON
        }]
    );
    assert!(poll(&handle, &shown, &mut ledger, 7).is_pending());
    assert_eq!(sent(&ledger).len(), 1);
    settle(&mut ledger, 7, true);
    assert!(matches!(
        poll(&handle, &shown, &mut ledger, 8),
        Poll::Ready(Ok(evidence)) if evidence.tick == 8
    ));
    assert_eq!(sent(&ledger).len(), 1, "one press only");
}

#[test]
fn pause_button_sends_the_resume_press() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut ledger = None;
    let shown = snapshot(false, tree(false, BUTTON_TYPE_PAUSE));
    let handle = begin(&shown, &mut ledger, 1, Op::Button(BUTTON), 4);
    assert!(poll(&handle, &shown, &mut ledger, 2).is_pending());
    assert_eq!(
        sent(&ledger),
        vec![InteractReq::ContinueDialog {
            component_id: Some(BUTTON)
        }]
    );
}

#[test]
fn button_that_never_appears_or_is_refused_fails() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut ledger = None;
    let hidden = snapshot(false, tree(true, 1));
    let handle = begin(&hidden, &mut ledger, 1, Op::Button(BUTTON), 2);
    assert!(poll(&handle, &hidden, &mut ledger, 2).is_pending());
    assert!(failed(poll(&handle, &hidden, &mut ledger, 3)).contains("interface button"));

    // A component that is not on any open interface is the same wait.
    let mut ledger = None;
    let other = snapshot(false, Vec::new());
    let handle = begin(&other, &mut ledger, 1, Op::Button(BUTTON), 1);
    assert!(failed(poll(&handle, &other, &mut ledger, 2)).contains("interface button"));

    let mut ledger = None;
    let shown = snapshot(false, tree(false, 1));
    let handle = begin(&shown, &mut ledger, 1, Op::Button(BUTTON), 4);
    assert!(poll(&handle, &shown, &mut ledger, 2).is_pending());
    settle(&mut ledger, 2, false);
    assert!(failed(poll(&handle, &shown, &mut ledger, 3)).contains("refused"));
}

#[test]
fn shown_component_walks_the_layers_and_rejects_cycles_and_orphans() {
    let widgets = tree(false, 1);
    assert!(shown_component(&widgets, BUTTON).is_some());
    assert!(shown_component(&tree(true, 1), BUTTON).is_none());
    assert!(shown_component(&widgets, 1).is_none(), "unlisted component");
    // A parent that is not listed cannot prove the layers are shown.
    assert!(shown_component(&widgets[1..], BUTTON).is_none());
    // A parent cycle ends instead of looping.
    let cycle = vec![widget(1, 2, false, 1), widget(2, 1, false, 0)];
    assert!(shown_component(&cycle, 1).is_none());
}

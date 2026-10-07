use super::dialogue::{Dialogue, DialogueArgs, DialogueOptions, DialogueTarget, PAGE_SETTLE_TICKS};
use super::tests::with_tick;
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{
    ledger, ActionContext, ActionError, HostEffect, InteractionReceipt, NativeMachine,
};
use crate::shim::InteractReq;
use api::game_data::DialogueUiIds;
use api::quest_progress::EvidenceStamp;
use api::selected::ClientRevision;
use api::snapshot::{GameSnapshot, WidgetKind, WidgetRoot, WidgetView};
use std::sync::Arc;
use std::task::Poll;

fn snapshot() -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_main_modal(-1, vec![]);
    snapshot
}

fn dialogue_ui() -> DialogueUiIds {
    *api::game_data::for_revision(ClientRevision::R289)
        .unwrap()
        .dialogue_ui()
        .expect("the selected test revision has source-proven dialogue UI ids")
}

#[test]
fn operation_page_gate_adopts_only_chat_and_selected_documents() {
    let ids = dialogue_ui();
    let unsupported = ids.scroll_root.max(ids.book_root) + 100;
    for (root, expected) in [
        (-1, false),
        (unsupported, false),
        (ids.scroll_root, true),
        (ids.book_root, true),
    ] {
        let mut snapshot = snapshot();
        snapshot.seed_main_modal(root, vec![]);
        let mut ledger = None;
        assert_eq!(
            with_tick(&snapshot, &mut ledger, 1, |tick| {
                super::dialogue::page_open(&tick.cx)
            }),
            expected
        );
    }
    for (root, continue_id) in [(100, -1), (-1, 105)] {
        let mut snapshot = snapshot();
        snapshot.seed_chat_modal(root, vec!["Chat page.".into()]);
        snapshot.seed_chat_options(vec![], continue_id);
        let mut ledger = None;
        assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
            super::dialogue::page_open(&tick.cx)
        }));
    }
}

fn args(target: DialogueTarget) -> DialogueArgs {
    DialogueArgs {
        target,
        options: DialogueOptions::default(),
    }
}

fn continuation_args() -> DialogueArgs {
    args(DialogueTarget::Continuation)
}
fn continuation_args_with_gap(gap_ticks: u16) -> DialogueArgs {
    DialogueArgs {
        target: DialogueTarget::Continuation,
        options: DialogueOptions {
            gap_ticks: Some(gap_ticks),
            ..Default::default()
        },
    }
}

fn npc_args() -> DialogueArgs {
    args(DialogueTarget::Npc {
        id: 0,
        name: Arc::from("Aubury"),
    })
}

fn widget(root: i32, component_id: i32, text: Option<&str>, hidden: bool) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: component_id,
        parent_id: root,
        root_component_id: root,
        root: WidgetRoot::Main,
        type_: 1,
        button_type: 1,
        client_code: -1,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        scroll_height: 0,
        scroll_position: 0,
        hidden,
        text: text.map(str::to_owned),
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
        varp_bindings: vec![],
        colour: 0,
        actions: vec![],
        items: vec![],
    }
}

fn book_page(ids: DialogueUiIds, text: &str, forward_visible: bool) -> Vec<WidgetView> {
    let body_id = [
        ids.scroll_root,
        ids.book_root,
        ids.book_forward,
        ids.book_close,
        ids.book_forward_marker,
    ]
    .into_iter()
    .max()
    .unwrap()
        + 100;
    vec![
        widget(ids.book_root, body_id, Some(text), false),
        widget(ids.book_root, ids.book_forward, None, false),
        widget(
            ids.book_root,
            ids.book_forward_marker,
            None,
            !forward_visible,
        ),
        widget(ids.book_root, ids.book_close, None, false),
    ]
}

fn last_interaction(ledger: &Option<Box<ledger::Ledger>>) -> &InteractReq {
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Interaction(request) => request,
        _ => panic!("expected interaction"),
    }
}

fn complete_talk(
    ledger: &mut Option<Box<ledger::Ledger>>,
    evidence_tick: u64,
    accepted: bool,
    wrong_request: bool,
) {
    let authority = ledger.as_ref().unwrap().outbox[0].authority();
    let request_id = authority.request_id().get() + if wrong_request { 1 } else { 0 };
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: evidence_tick,
                sequence: evidence_tick,
            },
            accepted,
            chat_since: 0,
        },
    );
}
fn complete_last_interaction(
    ledger: &mut Option<Box<ledger::Ledger>>,
    evidence_tick: u64,
    accepted: bool,
) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: evidence_tick,
                sequence: evidence_tick,
            },
            accepted,
            chat_since: 0,
        },
    );
}

struct PageProbe {
    dialogue: Dialogue,
}

impl NativeMachine for PageProbe {
    type Args = DialogueArgs;
    type Output = bool;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            dialogue: <Dialogue as NativeMachine>::begin(args, cx)?,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.dialogue.owned_chat_page(cx).is_some() {
            Poll::Ready(Ok(true))
        } else {
            Poll::Pending
        }
    }

    fn cancel(&mut self) {}
}

struct AdvancingPageProbe {
    dialogue: Dialogue,
}

impl NativeMachine for AdvancingPageProbe {
    type Args = DialogueArgs;
    type Output = DialogueOutcome;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            dialogue: <Dialogue as NativeMachine>::begin(args, cx)?,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.dialogue.owned_chat_page(cx).is_none() {
            return Poll::Pending;
        }
        self.dialogue.poll(cx)
    }

    fn cancel(&mut self) {}
}

#[test]
fn continuation_closes_scroll_and_waits_for_observed_disappearance() {
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });

    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));

    snapshot.seed_main_modal(-1, vec![]);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3 + PAGE_SETTLE_TICKS, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

#[test]
fn continuation_advances_book_pages_then_closes_only_after_the_final_marker() {
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    snapshot.seed_main_modal(ids.book_root, book_page(ids, "Page one", true));
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });

    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id }
            if *component_id == ids.book_forward
    ));

    snapshot.seed_main_modal(ids.book_root, book_page(ids, "Page two", true));
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id }
            if *component_id == ids.book_forward
    ));

    snapshot.seed_main_modal(ids.book_root, book_page(ids, "Page three", false));
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id }
            if *component_id == ids.book_close
    ));

    snapshot.seed_main_modal(-1, vec![]);
    assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 8, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

#[test]
fn unsupported_or_ambiguous_main_dialogue_fails_without_clicking() {
    let ids = dialogue_ui();
    for (root, widgets) in [
        (ids.book_root + 1_000_000, vec![]),
        (ids.book_root, {
            let mut widgets = book_page(ids, "Ambiguous page", true);
            widgets.push(widget(ids.book_root, ids.book_forward_marker, None, false));
            widgets
        }),
    ] {
        let mut snapshot = snapshot();
        snapshot.seed_main_modal(root, widgets);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Dialogue>(continuation_args(), &mut tick.cx)
                .unwrap()
        });
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(DialogueOutcome::Failed))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        assert_eq!(snapshot.modals().main, root);
    }
}

#[test]
fn an_owned_main_modal_still_open_at_close_timeout_is_failure() {
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));

    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 8, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
    assert_eq!(snapshot.modals().main, ids.scroll_root);
}

#[test]
fn first_chat_page_requires_a_matching_accepted_talk_receipt_and_newer_evidence() {
    for (accepted, wrong_request, record_receipt) in [
        (true, true, true),
        (false, false, true),
        (true, false, false),
    ] {
        let mut snapshot = snapshot();
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<PageProbe>(npc_args(), &mut tick.cx)
                .unwrap()
        });
        assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
        snapshot.seed_chat_modal(4882, vec!["Fresh first page".into()]);
        if record_receipt {
            complete_talk(&mut ledger, 2, accepted, wrong_request);
        }
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    }

    let mut snapshot = snapshot();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<PageProbe>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    snapshot.seed_chat_modal(4882, vec!["Fresh first page".into()]);
    complete_talk(&mut ledger, 2, true, false);
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(true))
    ));
}

#[test]
fn a_preexisting_chat_page_is_not_owned_by_a_new_npc_dialogue() {
    let mut snapshot = snapshot();
    snapshot.seed_chat_modal(4882, vec!["Already open".into()]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<PageProbe>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn the_owned_first_chat_page_is_not_exposed_after_continue_is_emitted() {
    let mut snapshot = snapshot();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<AdvancingPageProbe>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    complete_talk(&mut ledger, 2, true, false);
    snapshot.seed_chat_options(vec![], 4899);
    snapshot.seed_chat_modal(4882, vec!["First page".into()]);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
    let action_count = ledger.as_ref().unwrap().outbox.len();

    snapshot.seed_chat_modal(4882, vec!["Second page".into()]);
    assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), action_count);
}

#[test]
fn grandtree_foreman_cutscene_rearms_the_authored_closed_chat_gap() {
    // Sourced from content (quests/quest_grandtree/scripts/foreman.rs2:12-38):
    // `if_close` at T, `p_walk` at T+3, `p_teleport` at T+6, `forcewalk` at
    // T+8, and the next chat page at T+9. The player walks the first leg
    // from an adjacent tile, so each leg re-arms this authored four-tick gap.
    // This replaces the unsourced fixture whose timings matched no content.
    let mut snapshot = snapshot();
    snapshot.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 2955,
        z: 3034,
        level: 0,
    }));
    snapshot.seed_chat_modal(100, vec!["Follow me.".into()]);
    snapshot.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args_with_gap(4), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    // T = 3 is `if_close`: the walk lands at T+3 = 6, the teleport at
    // T+6 = 9, the forcewalk at T+8 = 11, and the next page at T+9 = 12.
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    for game_tick in [3, 4, 5] {
        assert!(
            with_tick(&snapshot, &mut ledger, game_tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the cutscene is still active at tick {game_tick}"
        );
    }
    snapshot.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 2955,
        z: 3033,
        level: 0,
    }));
    for game_tick in [6, 7, 8] {
        assert!(
            with_tick(&snapshot, &mut ledger, game_tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the cutscene is still active at tick {game_tick}"
        );
    }
    snapshot.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 2954,
        z: 3028,
        level: 0,
    }));
    for game_tick in [9, 10] {
        assert!(
            with_tick(&snapshot, &mut ledger, game_tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the cutscene is still active at tick {game_tick}"
        );
    }
    snapshot.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 2954,
        z: 3025,
        level: 0,
    }));
    assert!(
        with_tick(&snapshot, &mut ledger, 11, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending(),
        "the cutscene is still active at tick 11"
    );
    snapshot.seed_chat_modal(100, vec!["Tell me again why you're here.".into()]);
    snapshot.seed_chat_options(vec![], 101);
    assert!(with_tick(&snapshot, &mut ledger, 12, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
    assert!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .all(|action| { matches!(action.effect, HostEffect::Interaction(_)) }),
        "dialogue must not send an authored return-to-spawn walk during the cutscene"
    );
}

#[test]
fn level_up_page_after_owned_scroll_close_is_drained_as_continuation() {
    // Cook's hand-in (DIAG-COOK-HEADED report section 1): chat pages, then
    // the quest-complete scroll on the Main surface, our `CloseModal`, then
    // a level-up chat page opens ("Congratulations, you just advanced a
    // Cooking level" with Continue). Before the fix `Surface::Main` plus an
    // active chat page returned `Failed` and the page stayed open; now the
    // owned close adopts it and drains it with the normal chat driver.
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    complete_last_interaction(&mut ledger, 3, true);
    snapshot.seed_main_modal(-1, vec![]);
    snapshot.seed_chat_modal(
        100,
        vec!["Congratulations, you just advanced a Cooking level.".into()],
    );
    snapshot.seed_chat_options(vec![], 101);
    // This poll returned `Failed` before the fix.
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

#[test]
fn chat_page_while_main_scroll_still_open_stays_failed() {
    // The level-up adoption above only applies once the owned Main modal is
    // observed closed. A chat page that opens while the scroll is still
    // open is still a failure, before and after the fix.
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    snapshot.seed_chat_modal(
        100,
        vec!["Congratulations, you just advanced a Cooking level.".into()],
    );
    snapshot.seed_chat_options(vec![], 101);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
}

#[test]
fn unchanged_position_does_not_extend_closed_chat_completion() {
    let mut snapshot = snapshot();
    snapshot.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 2954,
        z: 3025,
        level: 0,
    }));
    snapshot.seed_chat_modal(100, vec!["Finished.".into()]);
    snapshot.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    // The closed chat is first observed at tick 3; unchanged position does
    // not re-arm it, so the one-tick gap completes at tick 4.
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

#[test]
fn visible_continue_without_chat_root_is_not_a_combat_close() {
    let mut snapshot = snapshot();
    // The host can retain a Continue-only mesbox alongside a health-bar window.
    // The Continue component is page evidence even without an open chat root.
    snapshot.seed_chat_modal(-1, vec!["You overpower Lady Keli.".into()]);
    snapshot.seed_chat_options(vec![], 10005);
    super::tests::seed_dialogue_combat(&mut snapshot, true);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
    // A genuinely closed page in combat must still interrupt immediately.
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::CombatInterrupted))
    ));
}

#[test]
fn owned_chat_waits_for_a_fresh_acceptance_before_draining_a_selected_scroll() {
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    complete_talk(&mut ledger, 2, true, false);
    snapshot.seed_chat_modal(4882, vec!["Fresh first page".into()]);
    snapshot.seed_chat_options(vec![], 4883);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));

    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));

    complete_last_interaction(&mut ledger, 5, true);
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));

    snapshot.seed_main_modal(-1, vec![]);
    assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 8, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

#[test]
fn refused_chat_advance_does_not_adopt_a_main_scroll() {
    let ids = dialogue_ui();
    let mut snapshot = snapshot();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    complete_talk(&mut ledger, 2, true, false);
    snapshot.seed_chat_modal(4882, vec!["Fresh first page".into()]);
    snapshot.seed_chat_options(vec![], 4883);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    complete_last_interaction(&mut ledger, 3, false);
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    assert_eq!(snapshot.modals().main, ids.scroll_root);
}

#[test]
fn a_main_scroll_without_an_owned_chat_page_stays_fail_closed() {
    let ids = dialogue_ui();
    for root in [ids.scroll_root, ids.quest_scroll_root] {
        let mut snapshot = snapshot();
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Dialogue>(npc_args(), &mut tick.cx)
                .unwrap()
        });
        complete_talk(&mut ledger, 2, true, false);
        snapshot.seed_main_modal(root, vec![]);

        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(DialogueOutcome::Failed))
        ));
        assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
        assert_eq!(snapshot.modals().main, root);
    }
}

/// Captain Tobias's "Yes please." runs `~set_sail`, whose
/// `if_openmain(ship_journey)` replaces the chat page with a Main modal that
/// is neither a scroll nor a book. After our own accepted advance that is
/// the end of the conversation; a refused advance, or the same modal after
/// only the Talk-to, stays failed.
#[test]
fn a_main_modal_opened_by_our_accepted_advance_completes_the_dialogue() {
    let ids = dialogue_ui();
    let ship_journey = ids.scroll_root.max(ids.book_root) + 100;
    for (advance, accepted, expected) in [
        (true, true, DialogueOutcome::Completed),
        (true, false, DialogueOutcome::Failed),
        (false, true, DialogueOutcome::Failed),
    ] {
        let mut snapshot = snapshot();
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Dialogue>(npc_args(), &mut tick.cx)
                .unwrap()
        });
        complete_talk(&mut ledger, 2, true, false);
        if advance {
            snapshot.seed_chat_modal(4882, vec!["Yes please.".into()]);
            snapshot.seed_chat_options(vec![], 4883);
            assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending());
            assert!(matches!(
                last_interaction(&ledger),
                InteractReq::ContinueDialog { .. }
            ));
            complete_last_interaction(&mut ledger, 4, accepted);
        }
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        snapshot.seed_main_modal(ship_journey, vec![]);
        let outcome = with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            matches!(outcome, Poll::Ready(Ok(outcome)) if outcome == expected),
            "advance {advance}, accepted {accepted}: expected {expected:?}"
        );
        assert!(
            !ledger.as_ref().unwrap().outbox.iter().any(|entry| matches!(
                &entry.effect,
                HostEffect::Interaction(InteractReq::CloseModal)
            ))
        );
        assert_eq!(snapshot.modals().main, ship_journey);
    }
}

use super::dialogue::{Dialogue, DialogueArgs, DialogueOptions, DialogueTarget, PAGE_SETTLE_TICKS};
use super::tests::{with_tick, with_tick_snapshots};
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{
    ledger, ActionContext, ActionError, HostEffect, InteractionReceipt, NativeMachine,
};
use crate::quest_journal::{JournalMachine, JournalRequest};
use crate::shim::InteractReq;
use api::game_data::DialogueUiIds;
use api::quest_facts::QuestCatalog;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey};
use api::snapshot::{
    ChatLineView, GameSnapshot, QuestStatusView, WidgetKind, WidgetRoot, WidgetView,
};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

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

/// Revision 289 publishes the chapter page root; the test harness pins 289.
fn chapter_root(ids: DialogueUiIds) -> i32 {
    ids.chapter_root
        .expect("revision 289 publishes the chapter page root")
}

#[test]
fn a_revision_without_chapter_pages_never_treats_a_root_as_a_chapter_page() {
    let ids = dialogue_ui();
    let without_chapters = DialogueUiIds {
        chapter_root: None,
        ..ids
    };
    assert!(super::dialogue::is_scroll_root(
        ids.scroll_root,
        without_chapters
    ));
    assert!(super::dialogue::is_scroll_root(
        ids.quest_scroll_root,
        without_chapters
    ));
    assert!(!super::dialogue::is_scroll_root(
        chapter_root(ids),
        without_chapters
    ));
    assert!(super::dialogue::is_scroll_root(chapter_root(ids), ids));
}

#[test]
fn operation_page_gate_adopts_only_chat_and_selected_documents() {
    let ids = dialogue_ui();
    let unsupported = chapter_root(ids).max(ids.scroll_root.max(ids.book_root)) + 100;
    for (root, expected) in [
        (-1, false),
        (unsupported, false),
        (ids.scroll_root, true),
        (ids.book_root, true),
        (chapter_root(ids), true),
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
pub(super) fn content_npc_id(config: &str) -> i32 {
    api::game_data::for_revision(ClientRevision::R289)
        .unwrap()
        .npc_by_config(config)
        .unwrap_or_else(|| panic!("R289 content has NPC config {config:?}"))
        .id
}
fn content_wander_range(config: &str) -> i32 {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let npc_id = data
        .npc_by_config(config)
        .unwrap_or_else(|| panic!("R289 content has NPC config {config:?}"))
        .id;
    data.npc_name(npc_id)
        .unwrap_or_else(|| panic!("R289 content has NPC id {npc_id}"))
        .wanderrange
}

pub(super) fn npc_args_for(id: i32, name: &str) -> DialogueArgs {
    args(DialogueTarget::Npc {
        id,
        name: Arc::from(name),
    })
}

fn content_dialogue_npc(
    id: i32,
    name: &str,
    distance: i32,
    tile: api::WorldTile,
) -> api::snapshot::NpcView {
    let mut npc = dialogue_npc_at(distance, tile);
    npc.r#type = Some(usize::try_from(id).expect("R289 NPC ids are nonnegative"));
    npc.name = Some(name.to_owned());
    npc
}

fn game_message(sequence: i32, text: &str) -> ChatLineView {
    ChatLineView {
        type_: 0,
        username: None,
        text: text.to_owned(),
        sequence,
    }
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

/// The target of the newest walk that is still live in the outbox.
fn live_walk_target(ledger: &Option<Box<ledger::Ledger>>) -> Option<api::WorldTile> {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find_map(|entry| match (&entry.effect, entry.live()) {
            (HostEffect::Walk(request), true) => Some(request.target),
            _ => None,
        })
}

/// Every walk the machine has requested, live or superseded.
fn walks_requested(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|entry| matches!(&entry.effect, HostEffect::Walk(_)))
        .count()
}

fn live_walks(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|entry| matches!(&entry.effect, HostEffect::Walk(_)) && entry.live())
        .count()
}

/// The request id of the newest route assessment, live or cancelled.
fn last_assessment_id(ledger: &Option<Box<ledger::Ledger>>) -> u64 {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find(|entry| matches!(&entry.effect, HostEffect::AssessWalk(_)))
        .expect("a route assessment was requested")
        .request_id
        .get()
}

/// Distinct walk requests the machine has queued, counted across ticks: a
/// cancelled request leaves the outbox, so one look at the end is not enough.
#[derive(Default)]
struct WalkTally(Vec<u64>);

impl WalkTally {
    fn observe(&mut self, ledger: &Option<Box<ledger::Ledger>>) {
        for entry in &ledger.as_ref().unwrap().outbox {
            if matches!(&entry.effect, HostEffect::Walk(_))
                && !self.0.contains(&entry.request_id.get())
            {
                self.0.push(entry.request_id.get());
            }
        }
    }

    fn count(&self) -> usize {
        self.0.len()
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
fn close_chapter_page_after_accepted_advance(
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> u64 {
    let ids = dialogue_ui();
    let handle = with_tick(snapshot, ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap()
    });
    complete_talk(ledger, 2, true, false);
    snapshot.seed_chat_modal(4882, vec!["Yes please.".into()]);
    snapshot.seed_chat_options(vec![], 4883);
    assert!(with_tick(snapshot, ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(ledger),
        InteractReq::ContinueDialog { .. }
    ));
    complete_last_interaction(ledger, 4, true);
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_main_modal(chapter_root(ids), vec![]);
    assert!(with_tick(snapshot, ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(ledger), InteractReq::CloseModal));
    complete_last_interaction(ledger, 6, true);
    snapshot.seed_main_modal(-1, vec![]);
    assert!(with_tick(snapshot, ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    let settle_tick = 6 + PAGE_SETTLE_TICKS;
    assert!(matches!(
        with_tick(snapshot, ledger, settle_tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
    settle_tick
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
fn chapter_page_after_accepted_advance_is_closed_by_dialogue() {
    let mut snapshot = snapshot();
    let mut ledger = None;
    close_chapter_page_after_accepted_advance(&mut snapshot, &mut ledger);
    assert_eq!(snapshot.modals().main, -1);
}

#[test]
fn journal_read_begins_after_chapter_page_closes() {
    let mut snapshot = snapshot();
    let mut ledger = None;
    let settle_tick = close_chapter_page_after_accepted_advance(&mut snapshot, &mut ledger);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Monkey Madness".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let request = JournalRequest {
        quest: FactKey::new("mm"),
        facts: Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap()),
    };
    let journal = with_tick(&snapshot, &mut ledger, settle_tick + 1, |tick| {
        tick.actions
            .begin::<JournalMachine>(request, &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, settle_tick + 2, |tick| {
        tick.actions.poll(&journal, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
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
    super::tests::seed_dialogue_attacker(&mut snapshot, api::snapshot::ActorKind::Npc);
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::CombatInterrupted))
    ));
}

#[test]
fn continuation_waits_for_content_page_after_post_fight_close() {
    use super::tests::seed_dialogue_combat;

    let mut snapshot = snapshot();
    seed_dialogue_combat(&mut snapshot, true);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        let combat = tick.cx.snapshot().in_combat().unwrap().value;
        assert!(combat.in_combat);
        assert!(combat.target.is_none());
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });

    // The content closes the previous page before it opens its first page.
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    snapshot.seed_chat_modal(-1, vec!["STOP! Please... spare my son.".into()]);
    snapshot.seed_chat_options(vec![], 10005);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
}

#[test]
fn pending_first_continuation_requires_a_real_attacker_for_combat_interruption() {
    use super::tests::{seed_dialogue_attacker, seed_dialogue_combat};

    for attacker in [
        None,
        Some(api::snapshot::ActorKind::Npc),
        Some(api::snapshot::ActorKind::Player),
    ] {
        let mut snapshot = snapshot();
        seed_dialogue_combat(&mut snapshot, true);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Dialogue>(continuation_args(), &mut tick.cx)
                .unwrap()
        });
        if let Some(kind) = attacker {
            seed_dialogue_attacker(&mut snapshot, kind);
        }
        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        if attacker.is_some() {
            assert!(matches!(
                result,
                Poll::Ready(Err(ActionError::CombatInterrupted))
            ), "{attacker:?}");
        } else {
            assert!(result.is_pending(), "a stale flag is not an attacker");
        }
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
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
    for root in [ids.scroll_root, ids.quest_scroll_root, chapter_root(ids)] {
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
    let ship_journey = chapter_root(ids).max(ids.scroll_root.max(ids.book_root)) + 100;
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

fn spend_tick_interaction(tick: &mut crate::native::NativeTick<'_>) {
    tick.cx
        .emit(InteractReq::Npc {
            name: "Earlier action".to_owned(),
            action: "Talk-to".to_owned(),
            index: None,
        })
        .unwrap();
}

fn dialogue_npc(distance: i32) -> api::snapshot::NpcView {
    dialogue_npc_at(
        distance,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )
}

fn dialogue_npc_at(distance: i32, tile: api::WorldTile) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index: 7,
        r#type: Some(0),
        name: Some("Aubury".into()),
        actions: vec![Some("Talk-to".into())],
        tile,
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
        level: 0,
        size: 1,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

#[test]
fn dialogue_continue_after_same_tick_talk_defers_and_retries() {
    let mut before = snapshot();
    before.seed_chat_modal(-1, vec![]);
    before.seed_chat_options(vec![], -1);
    let mut page = snapshot();
    page.seed_chat_modal(100, vec!["Aubury has something to say.".into()]);
    page.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let (handle, same_tick) = with_tick_snapshots(&before, &page, &mut ledger, 1, |tick, page| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        tick.cx.snapshot = api::snapshot::SnapshotView::new(Some(page), tick.cx.evidence());
        let result = tick.actions.poll(&handle, &mut tick.cx);
        (handle, result)
    });
    assert!(same_tick.is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));

    assert!(with_tick(&page, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 2);
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
}

#[test]
fn dialogue_answer_after_same_tick_talk_defers_and_retries() {
    let mut before = snapshot();
    before.seed_chat_modal(-1, vec![]);
    before.seed_chat_options(vec![], -1);
    let mut page = snapshot();
    page.seed_chat_modal(100, vec!["Would you like to buy a staff?".into()]);
    page.seed_chat_options(
        vec![api::snapshot::ChatOptionView {
            component_id: 101,
            text: "Yes, please.".into(),
        }],
        -1,
    );
    let mut ledger = None;
    let (handle, same_tick) = with_tick_snapshots(&before, &page, &mut ledger, 1, |tick, page| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        tick.cx.snapshot = api::snapshot::SnapshotView::new(Some(page), tick.cx.evidence());
        let result = tick.actions.poll(&handle, &mut tick.cx);
        (handle, result)
    });
    assert!(same_tick.is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));

    assert!(with_tick(&page, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 2);
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::Answer { option: 1 }
    ));
}

#[test]
fn dialogue_open_accepts_first_page_within_tick_budget() {
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![dialogue_npc_at(
        1,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });

    let mut opened = snapshot();
    opened.seed_chat_modal(100, vec!["Greetings.".into()]);
    opened.seed_chat_options(vec![], 101);
    assert!(with_tick(&opened, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
}

#[test]
fn dialogue_open_timeout_counts_observed_ticks_not_elapsed_time() {
    let before = snapshot();
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });

    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.cx.active_now = Duration::from_secs(86_400);
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&before, &mut ledger, 4, |tick| {
        tick.cx.active_now = Duration::ZERO;
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&before, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "expected dialogue did not open"
    ));
}

#[test]
fn dialogue_stationary_cook_approach_uses_router_route_ticks_at_fixed_ticks() {
    let cook_id = content_npc_id("cook");
    assert_eq!(content_wander_range("cook"), 0);
    let cook_tile = api::WorldTile {
        x: 0,
        z: 5,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 10,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![content_dialogue_npc(cook_id, "Cook", 10, cook_tile)]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    // No flood reaches the Cook, so the router assesses the 10-tile route
    // first (15 route ticks) and walks only once it answers.
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile
    ));
    let mut walks = WalkTally::default();
    walks.observe(&ledger);
    assert_eq!(walks.count(), 0);
    super::tests::post_assess_receipt(&mut ledger, 1, 15, None);

    let mut six_tiles_away = snapshot();
    six_tiles_away.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 6,
        z: 5,
        level: 0,
    }));
    six_tiles_away.seed_npcs(vec![content_dialogue_npc(cook_id, "Cook", 6, cook_tile)]);
    // The walk is requested at tick 2: 15 route ticks plus 2 slack ticks.
    for observed in [2, 5, 18] {
        assert!(
            with_tick(&six_tiles_away, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the Cook is still out of reach at tick {observed}"
        );
        assert_eq!(live_walk_target(&ledger), Some(cook_tile));
        walks.observe(&ledger);
    }
    // Tick 19 ends the first route with the Cook still in place. This failure
    // retries the same tile; it does not spend the NPC-movement replan budget.
    assert!(with_tick(&six_tiles_away, &mut ledger, 19, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&six_tiles_away, &mut ledger, 20, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 20, 9, None);
    assert!(with_tick(&six_tiles_away, &mut ledger, 21, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    walks.observe(&ledger);
    assert_eq!(walks.count(), 2);
    assert!(with_tick(&six_tiles_away, &mut ledger, 31, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&six_tiles_away, &mut ledger, 32, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&six_tiles_away, &mut ledger, 33, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == cook_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 33, 9, None);
    assert!(with_tick(&six_tiles_away, &mut ledger, 34, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    walks.observe(&ledger);
    assert_eq!(walks.count(), 3);
    assert!(with_tick(&six_tiles_away, &mut ledger, 44, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&six_tiles_away, &mut ledger, 45, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "dialogue approach failed: the walk did not reach the NPC by observed tick 45"
    ));
}

#[test]
fn dialogue_no_route_retries_the_bounded_approach_budget() {
    let cook_id = content_npc_id("cook");
    let cook_tile = api::WorldTile {
        x: 0,
        z: 5,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 10,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![content_dialogue_npc(cook_id, "Cook", 10, cook_tile)]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    let no_route = Some(crate::native::WalkRefusal::NoRouteWithinBounds {
        tried: 0,
        last: None,
    });
    let first = super::tests::post_assess_receipt(&mut ledger, 1, 0, no_route);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        !ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .any(|entry| matches!(&entry.effect, HostEffect::AssessWalk(_))),
        "the fresh plan waits for the next observed tick"
    );
    assert!(with_tick(&before, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_ne!(
        last_assessment_id(&ledger),
        first,
        "one fresh plan from the observed position"
    );
    super::tests::post_assess_receipt(&mut ledger, 3, 0, no_route);
    assert!(with_tick(&before, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&before, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_ne!(last_assessment_id(&ledger), first);
    super::tests::post_assess_receipt(&mut ledger, 5, 0, no_route);
    assert!(matches!(
        with_tick(&before, &mut ledger, 6, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "dialogue approach failed: no route to the NPC: router found no route"
    ));
    assert_eq!(walks_requested(&ledger), 0, "no route, no walk");
}

fn begin_far_cook() -> (
    Option<Box<ledger::Ledger>>,
    crate::native::ActionHandle<Dialogue>,
    GameSnapshot,
    api::WorldTile,
) {
    let cook_id = content_npc_id("cook");
    let cook_tile = api::WorldTile {
        x: 0,
        z: 5,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 10,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![content_dialogue_npc(cook_id, "Cook", 10, cook_tile)]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    (ledger, handle, before, cook_tile)
}

/// The host never answers the route assessment (the stall seen in live run `d3c1`).
/// The wait is 8 observed ticks per plan, up to the shared three-attempt limit.
#[test]
fn dialogue_unanswered_route_assessment_retries_until_the_budget_is_spent() {
    let (mut ledger, handle, before, _) = begin_far_cook();
    let first = last_assessment_id(&ledger);
    for observed in 2..9 {
        assert!(
            with_tick(&before, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "tick {observed} is inside the 8-tick wait"
        );
        assert_eq!(last_assessment_id(&ledger), first);
    }
    // Tick 9 ends the wait; the fresh plan follows one tick later.
    assert!(with_tick(&before, &mut ledger, 9, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(with_tick(&before, &mut ledger, 10, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_ne!(last_assessment_id(&ledger), first);
    let second = last_assessment_id(&ledger);
    for observed in 11..18 {
        assert!(with_tick(&before, &mut ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    assert!(with_tick(&before, &mut ledger, 18, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    assert!(with_tick(&before, &mut ledger, 19, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_ne!(last_assessment_id(&ledger), second);
    for observed in 20..27 {
        assert!(with_tick(&before, &mut ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    assert!(matches!(
        with_tick(&before, &mut ledger, 27, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "dialogue approach failed: the router did not answer the route assessment by observed tick 27"
    ));
    assert_eq!(walks_requested(&ledger), 0);
}

#[test]
fn dialogue_route_assessment_answered_after_one_lost_request_sets_the_walk_budget() {
    let (mut ledger, handle, before, cook_tile) = begin_far_cook();
    for observed in 2..11 {
        assert!(with_tick(&before, &mut ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
    }
    // The second request, asked at tick 10, is answered with 15 route ticks.
    super::tests::post_assess_receipt(&mut ledger, 10, 15, None);
    assert!(with_tick(&before, &mut ledger, 11, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walk_target(&ledger), Some(cook_tile));
    // The walk was requested at tick 11: 15 route ticks plus 2 slack ticks.
    assert!(with_tick(&before, &mut ledger, 27, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walk_target(&ledger), Some(cook_tile));
}

#[test]
fn dialogue_route_ending_short_with_a_still_npc_is_a_walk_failure_not_a_replan() {
    let brundt_id = content_npc_id("viking_brundt");
    assert_eq!(content_wander_range("viking_brundt"), 2);
    let player_tile = api::WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![content_dialogue_npc(
        brundt_id,
        "Brundt the Chieftain",
        7,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(
                npc_args_for(brundt_id, "Brundt the Chieftain"),
                &mut tick.cx,
            )
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    super::tests::post_assess_receipt(&mut ledger, 1, 10, None);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    let mut walks = WalkTally::default();
    walks.observe(&ledger);
    assert_eq!(walks.count(), 1);

    // The route ends short and Brundt has not moved: no replan, one fresh plan.
    super::tests::post_user_input_walk_receipt(&mut ledger, 3);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(with_tick(&before, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a still NPC is not replanned: no walk and no assessment is queued"
    );
    assert!(with_tick(&before, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    super::tests::post_assess_receipt(&mut ledger, 4, 10, None);
    assert!(with_tick(&before, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    walks.observe(&ledger);
    assert_eq!(walks.count(), 2);

    super::tests::post_user_input_walk_receipt(&mut ledger, 6);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(with_tick(&before, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&before, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request)
            if request.target.x == 7
                && request.target.z == 5
                && request.target.level == 0
    ));
    assert_eq!(
        walks.count(),
        2,
        "a still NPC does not spend the movement-replan budget"
    );
}

#[test]
fn dialogue_open_timeout_uses_four_ticks_after_accepted_receipt() {
    let cook_id = content_npc_id("cook");
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 6,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![content_dialogue_npc(
        cook_id,
        "Cook",
        1,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    complete_talk(&mut ledger, 1, true, false);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&before, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&before, &mut ledger, 6, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "expected dialogue did not open"
    ));
}

#[test]
fn dialogue_siad_and_upass_pages_open_on_the_receipt_deadline_tick() {
    fn assert_page_at_deadline(config: &str, name: &str, first_message: &str) {
        let npc_id = content_npc_id(config);
        let mut before = snapshot();
        before.seed_local_player(super::tests::local_player(api::WorldTile {
            x: 5,
            z: 5,
            level: 0,
        }));
        before.seed_npcs(vec![content_dialogue_npc(
            npc_id,
            name,
            5,
            api::WorldTile {
                x: 10,
                z: 5,
                level: 0,
            },
        )]);
        let mut ledger = None;
        let handle = with_tick(&before, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Dialogue>(npc_args_for(npc_id, name), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });

        let mut arrived = snapshot();
        arrived.seed_local_player(super::tests::local_player(api::WorldTile {
            x: 6,
            z: 5,
            level: 0,
        }));
        arrived.seed_npcs(vec![content_dialogue_npc(
            npc_id,
            name,
            1,
            api::WorldTile {
                x: 7,
                z: 5,
                level: 0,
            },
        )]);
        assert!(with_tick(&arrived, &mut ledger, 32, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));

        complete_last_interaction(&mut ledger, 32, true);
        let mut progress = snapshot();
        progress.seed_local_player(super::tests::local_player(api::WorldTile {
            x: 6,
            z: 5,
            level: 0,
        }));
        progress.seed_npcs(vec![content_dialogue_npc(
            npc_id,
            name,
            1,
            api::WorldTile {
                x: 7,
                z: 5,
                level: 0,
            },
        )]);
        progress.seed_chat_lines(vec![game_message(1, first_message)]);
        assert!(with_tick(&progress, &mut ledger, 33, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert!(
            with_tick(&progress, &mut ledger, 36, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the four-tick opening deadline is tick 37"
        );

        let mut opened = snapshot();
        opened.seed_chat_modal(100, vec![first_message.to_owned()]);
        opened.seed_chat_options(vec![], 101);
        assert!(
            with_tick(&opened, &mut ledger, 37, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "a page on the deadline tick is observed before timeout"
        );
        assert!(matches!(
            last_interaction(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
    }

    assert_page_at_deadline(
        "capt_siad",
        "Captain Siad",
        "The captain looks up from his work as you address him.",
    );
    assert_page_at_deadline(
        "cave_slave1",
        "cave_slave1",
        "The man seems to be in a weak state of mind.",
    );
}

#[test]
fn dialogue_brundt_replan_cap_matches_24_nonhome_tiles_at_fixed_ticks() {
    let brundt_id = content_npc_id("viking_brundt");
    assert_eq!(content_wander_range("viking_brundt"), 2);
    let player_tile = api::WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![content_dialogue_npc(
        brundt_id,
        "Brundt the Chieftain",
        7,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(
                npc_args_for(brundt_id, "Brundt the Chieftain"),
                &mut tick.cx,
            )
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    // No flood reaches Brundt: the router assesses each route first.
    super::tests::post_assess_receipt(&mut ledger, 1, 10, None);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    let mut tick_number = 3;
    for dz in -2..=2 {
        for dx in -2..=2 {
            if dx == 0 && dz == 0 {
                continue;
            }
            let current_npc_tile = api::WorldTile {
                x: 7 + dx,
                z: 5 + dz,
                level: 0,
            };
            super::tests::post_user_input_walk_receipt(&mut ledger, tick_number);
            ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
            let mut moved = snapshot();
            moved.seed_local_player(super::tests::local_player(player_tile));
            moved.seed_npcs(vec![content_dialogue_npc(
                brundt_id,
                "Brundt the Chieftain",
                (player_tile.x - current_npc_tile.x)
                    .abs()
                    .max((player_tile.z - current_npc_tile.z).abs()),
                current_npc_tile,
            )]);
            assert!(
                with_tick(&moved, &mut ledger, tick_number, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                })
                .is_pending(),
                "Brundt stays out of reach through all 24 non-home tiles"
            );
            assert!(
                matches!(
                    super::tests::last_route_effect(&ledger),
                    HostEffect::AssessWalk(request) if request.target == current_npc_tile
                ),
                "every move to a new tile costs one replan, planned by the router"
            );
            super::tests::post_assess_receipt(&mut ledger, tick_number, 10, None);
            assert!(with_tick(&moved, &mut ledger, tick_number + 1, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending());
            assert_eq!(live_walk_target(&ledger), Some(current_npc_tile));
            tick_number += 2;
        }
    }
    assert_eq!(tick_number, 51);

    super::tests::post_user_input_walk_receipt(&mut ledger, 51);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    let mut still_moving = snapshot();
    still_moving.seed_local_player(super::tests::local_player(player_tile));
    still_moving.seed_npcs(vec![content_dialogue_npc(
        brundt_id,
        "Brundt the Chieftain",
        8,
        api::WorldTile {
            x: 8,
            z: 5,
            level: 0,
        },
    )]);
    assert!(matches!(
        with_tick(&still_moving, &mut ledger, 51, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "dialogue approach failed: the NPC kept moving through every replan"
    ));
}

#[test]
fn dialogue_stationary_cook_refusal_retries_once_at_fixed_ticks() {
    let cook_id = content_npc_id("cook");
    assert_eq!(content_wander_range("cook"), 0);
    let player_tile = api::WorldTile {
        x: 6,
        z: 5,
        level: 0,
    };
    let cook = content_dialogue_npc(
        cook_id,
        "Cook",
        1,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    );
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![cook.clone()]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    complete_talk(&mut ledger, 1, true, false);
    let mut refused = snapshot();
    refused.seed_local_player(super::tests::local_player(player_tile));
    refused.seed_npcs(vec![cook.clone()]);
    refused.seed_chat_lines(vec![game_message(1, "I can't reach that!")]);
    assert!(with_tick(&refused, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        matches!(
            super::tests::last_route_effect(&ledger),
            HostEffect::AssessWalk(request) if request.target == cook.tile
        ),
        "a stationary NPC still gets one approach retry, planned by the router"
    );
    assert_eq!(walks_requested(&ledger), 0);
    super::tests::post_assess_receipt(&mut ledger, 2, 3, None);
    let mut walking = snapshot();
    walking.seed_local_player(super::tests::local_player(player_tile));
    let mut cook_ahead = cook.clone();
    cook_ahead.distance = 4;
    walking.seed_npcs(vec![cook_ahead]);
    walking.seed_chat_lines(vec![game_message(1, "I can't reach that!")]);
    assert!(with_tick(&walking, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walks(&ledger), 1);

    super::tests::post_user_input_walk_receipt(&mut ledger, 4);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(with_tick(&refused, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    complete_last_interaction(&mut ledger, 4, true);

    let mut refused_again = refused;
    refused_again.seed_chat_lines(vec![game_message(2, "I can't reach that!")]);
    assert!(matches!(
        with_tick(&refused_again, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "server refused Talk-to: I can't reach that!"
    ));
    assert_eq!(
        live_walks(&ledger),
        0,
        "the refusal budget does not issue a second recovery walk"
    );
}

#[test]
fn dialogue_refusal_approach_failure_keeps_refusal_detail() {
    let brundt_id = content_npc_id("viking_brundt");
    assert_eq!(content_wander_range("viking_brundt"), 2);
    let player_tile = api::WorldTile {
        x: 6,
        z: 5,
        level: 0,
    };
    let brundt_tile = api::WorldTile {
        x: 7,
        z: 5,
        level: 0,
    };
    let brundt = content_dialogue_npc(brundt_id, "Brundt the Chieftain", 1, brundt_tile);
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![brundt.clone()]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(
                npc_args_for(brundt_id, "Brundt the Chieftain"),
                &mut tick.cx,
            )
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    complete_talk(&mut ledger, 1, true, false);
    let mut refused = snapshot();
    refused.seed_local_player(super::tests::local_player(player_tile));
    refused.seed_npcs(vec![brundt]);
    refused.seed_chat_lines(vec![game_message(1, "I can't reach that!")]);
    assert!(with_tick(&refused, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(_)
    ));
    assert_eq!(walks_requested(&ledger), 0);

    // Brundt is missing from the snapshot during the recovery route: the
    // route holds his last seen tile, and the failure keeps the refusal first.
    let mut missing = snapshot();
    missing.seed_local_player(super::tests::local_player(player_tile));
    let assessment = last_assessment_id(&ledger);
    assert!(with_tick(&missing, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(last_assessment_id(&ledger), assessment);
    let no_route = Some(crate::native::WalkRefusal::NoRouteWithinBounds {
        tried: 0,
        last: None,
    });
    super::tests::post_assess_receipt(&mut ledger, 3, 0, no_route);
    assert!(with_tick(&missing, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&missing, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == brundt_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 5, 0, no_route);
    assert!(with_tick(&missing, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&missing, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == brundt_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 7, 0, no_route);
    let terminal = with_tick(&missing, &mut ledger, 8, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(
        matches!(
            &terminal,
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref()
                    == "server refused Talk-to: I can't reach that!; recovery walk failed: \
                        no route to the NPC: router found no route; Brundt the Chieftain is not \
                        in view, last seen at (7, 5, level 0)"
        ),
        "unexpected terminal poll: {terminal:?}"
    );
    assert_eq!(walks_requested(&ledger), 0);
}

#[test]
fn dialogue_terminal_walk_receipts_keep_typed_user_input_evidence_and_danger_errors() {
    let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Complete(
        FactKey::new("walk-gate"),
    )]);
    let cases = [
        (crate::native::WalkEnd::UserInput, None, None),
        (
            crate::native::WalkEnd::NeedsEvidence(Arc::clone(&gates)),
            None,
            Some("walk needs evidence"),
        ),
        (
            crate::native::WalkEnd::Refused,
            Some(crate::native::WalkRefusal::Unsurvivable),
            Some("route refused by danger policy"),
        ),
    ];
    for (end, refusal, detail) in cases {
        let (mut ledger, handle, before, _) = begin_far_cook();
        super::tests::post_assess_receipt(&mut ledger, 1, 10, None);
        assert!(with_tick(&before, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let mut walks = WalkTally::default();
        walks.observe(&ledger);
        assert_eq!(walks.count(), 1);
        super::tests::post_walk_receipt_with(&mut ledger, 3, end.clone(), detail, refusal, None);
        let result = with_tick(&before, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        match end {
            crate::native::WalkEnd::UserInput => {
                assert!(matches!(result, Poll::Ready(Err(ActionError::UserInput))));
            }
            crate::native::WalkEnd::NeedsEvidence(expected) => {
                assert!(matches!(
                    result,
                    Poll::Ready(Err(ActionError::NeedsEvidence(actual)))
                        if actual.as_ref() == expected.as_ref()
                ));
            }
            crate::native::WalkEnd::Refused => {
                assert!(matches!(
                    result,
                    Poll::Ready(Err(ActionError::Blocked(detail)))
                        if detail.as_ref() == "route refused by danger policy"
                ));
            }
            _ => unreachable!("only terminal walk receipts are in this fixture"),
        }
        walks.observe(&ledger);
        assert_eq!(walks.count(), 1, "terminal receipts must not retry");
        assert!(
            !ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .any(|entry| matches!(&entry.effect, HostEffect::AssessWalk(_))),
            "terminal walk receipts must not start a fresh route assessment"
        );
    }
}

#[test]
fn dialogue_approach_exhaustion_keeps_the_last_walk_failure_detail() {
    let (mut ledger, handle, before, _) = begin_far_cook();
    super::tests::post_assess_receipt(&mut ledger, 1, 10, None);
    let mut walks = WalkTally::default();
    for (attempt, assessment_tick, detail) in [
        (1, 1, "first route failure"),
        (2, 4, "second route failure"),
        (3, 7, "last route failure"),
    ] {
        if attempt > 1 {
            assert!(with_tick(&before, &mut ledger, assessment_tick, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending());
            super::tests::post_assess_receipt(&mut ledger, assessment_tick, 10, None);
        }
        let walk_tick = assessment_tick + 1;
        assert!(with_tick(&before, &mut ledger, walk_tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        walks.observe(&ledger);
        super::tests::post_walk_receipt(
            &mut ledger,
            walk_tick + 1,
            crate::native::WalkEnd::NoPath,
            Some(detail),
        );
        let result = with_tick(&before, &mut ledger, walk_tick + 1, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        if attempt < 3 {
            assert!(
                result.is_pending(),
                "attempt {attempt} should schedule a retry"
            );
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Err(ActionError::Failed(reason)))
                    if reason.as_ref()
                        == "dialogue approach failed: the walk ended before the NPC was in reach: last route failure"
            ));
        }
    }
    assert_eq!(walks.count(), 3);
}

/// A Cook that the live flood cannot reach after a server refusal: the
/// recovery walk gets the router's route length, not the straight-line 2.
fn refused_cook_before_recovery() -> (
    Option<Box<ledger::Ledger>>,
    crate::native::ActionHandle<Dialogue>,
    GameSnapshot,
    api::query::ReachQueryView,
) {
    let cook_id = content_npc_id("cook");
    let player_tile = api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let cook = content_dialogue_npc(
        cook_id,
        "Cook",
        2,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    );
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![cook.clone()]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(cook_id, "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    complete_talk(&mut ledger, 1, true, false);
    let mut refused = snapshot();
    refused.seed_local_player(super::tests::local_player(player_tile));
    refused.seed_npcs(vec![cook]);
    refused.seed_chat_lines(vec![game_message(1, "I can't reach that!")]);
    let reach = super::tests::wall_door_reach_view();
    assert!(
        super::tests::with_tick_reach(&refused, &reach, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending()
    );
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.radius == 0
    ));
    (ledger, handle, refused, reach)
}

#[test]
fn dialogue_refusal_recovery_walk_budget_is_the_routers_route_length() {
    let (mut ledger, handle, refused, reach) = refused_cook_before_recovery();
    super::tests::post_assess_receipt(&mut ledger, 2, 15, None);
    let mut poll_at = |tick_number| {
        super::tests::with_tick_reach(&refused, &reach, &mut ledger, tick_number, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    };
    // The recovery walk is requested at tick 3: 15 route ticks plus 2 slack
    // ticks, not the 2 + 2 of the straight-line distance.
    for observed in [3, 6, 19] {
        let result = poll_at(observed);
        assert!(result.is_pending(), "tick {observed}: {result:?}");
    }
    assert_eq!(walks_requested(&ledger), 1);
}

#[test]
fn dialogue_refusal_recovery_keeps_the_refusal_when_the_router_finds_no_route() {
    let (mut ledger, handle, refused, reach) = refused_cook_before_recovery();
    let no_route = Some(crate::native::WalkRefusal::NoRouteWithinBounds {
        tried: 0,
        last: None,
    });
    super::tests::post_assess_receipt(&mut ledger, 2, 0, no_route);
    let poll_at = |ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick_number| {
        super::tests::with_tick_reach(&refused, &reach, ledger, tick_number, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    };
    assert!(poll_at(&mut ledger, 3).is_pending());
    assert!(poll_at(&mut ledger, 4).is_pending());
    super::tests::post_assess_receipt(&mut ledger, 4, 0, no_route);
    assert!(poll_at(&mut ledger, 5).is_pending());
    assert!(poll_at(&mut ledger, 6).is_pending());
    super::tests::post_assess_receipt(&mut ledger, 6, 0, no_route);
    assert!(matches!(
        poll_at(&mut ledger, 7),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "server refused Talk-to: I can't reach that!; recovery walk failed: no route to the NPC: router found no route"
    ));
    assert_eq!(walks_requested(&ledger), 0);
}

#[test]
fn dialogue_open_keeps_waiting_after_npc_content_message() {
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![dialogue_npc(1)]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });

    let mut progress = snapshot();
    progress.seed_npcs(vec![dialogue_npc(1)]);
    progress.seed_chat_lines(vec![game_message(
        1,
        "The captain looks up from his work as you address him.",
    )]);
    // The NPC's content message proves that the engine ran `opnpc1`, which
    // it does only for a click that the host sent: the host accepted the
    // Talk-to. The message anchors the page budget (tick 2 + 4 = tick 6).
    complete_talk(&mut ledger, 2, true, false);
    assert!(with_tick(&progress, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    let mut opened = snapshot();
    opened.seed_chat_modal(100, vec!["Greetings.".into()]);
    opened.seed_chat_options(vec![], 101);
    assert!(with_tick(&opened, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
}

#[test]
fn dialogue_viking_refusal_replans_and_retries_talk_for_visible_npc() {
    let player_tile = api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let initial_npc_tile = api::WorldTile {
        x: 7,
        z: 5,
        level: 0,
    };
    let moved_npc_tile = api::WorldTile {
        x: 8,
        z: 5,
        level: 0,
    };
    let brundt_id = api::game_data::for_revision(ClientRevision::R289)
        .expect("the pinned revision has NPC identity content")
        .npc_names()
        .expect("the pinned revision has NPC name facts")
        .rows
        .iter()
        .find(|row| row.config == "viking_brundt")
        .expect("the Viking content identifies Brundt")
        .id;
    let viking_npc = |distance, tile| {
        let mut npc = dialogue_npc_at(distance, tile);
        npc.r#type = Some(usize::try_from(brundt_id).expect("NPC ids are nonnegative"));
        npc.name = Some("Brundt the Chieftain".into());
        npc
    };
    let viking_args = DialogueArgs {
        target: DialogueTarget::Npc {
            id: brundt_id,
            name: Arc::from("Brundt the Chieftain"),
        },
        options: DialogueOptions::default(),
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![viking_npc(1, initial_npc_tile)]);
    before.seed_chat_lines(vec![game_message(7, "I can't reach that!")]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(viking_args.clone(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));

    complete_talk(&mut ledger, 1, true, false);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(!ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .any(|entry| { matches!(&entry.effect, HostEffect::Walk(_)) && entry.live() }));

    let mut refused = snapshot();
    refused.seed_local_player(super::tests::local_player(player_tile));
    refused.seed_npcs(vec![viking_npc(1, moved_npc_tile)]);
    refused.seed_chat_lines(vec![
        game_message(8, "I can't reach that!"),
        game_message(7, "I can't reach that!"),
    ]);
    assert!(with_tick(&refused, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        matches!(
            super::tests::last_route_effect(&ledger),
            HostEffect::AssessWalk(request) if request.target == moved_npc_tile
        ),
        "the refusal recovery must plan a route to the NPC's current tile"
    );
    super::tests::post_assess_receipt(&mut ledger, 3, 4, None);
    let mut walking = snapshot();
    walking.seed_local_player(super::tests::local_player(player_tile));
    walking.seed_npcs(vec![viking_npc(3, moved_npc_tile)]);
    walking.seed_chat_lines(vec![game_message(8, "I can't reach that!")]);
    assert!(with_tick(&walking, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        live_walk_target(&ledger),
        Some(moved_npc_tile),
        "the refusal recovery must target the NPC's current tile"
    );

    super::tests::post_user_input_walk_receipt(&mut ledger, 5);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    let mut settled = snapshot();
    settled.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 7,
        z: 5,
        level: 0,
    }));
    settled.seed_npcs(vec![viking_npc(1, moved_npc_tile)]);
    settled.seed_chat_lines(vec![game_message(8, "I can't reach that!")]);
    assert!(with_tick(&settled, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::Npc { index: Some(7), .. }
    ));
}

#[test]
fn dialogue_approach_replans_when_npc_moves_then_talks_after_it_settles() {
    let player_tile = api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let initial_npc_tile = api::WorldTile {
        x: 7,
        z: 5,
        level: 0,
    };
    let moved_npc_tile = api::WorldTile {
        x: 8,
        z: 5,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(player_tile));
    before.seed_npcs(vec![dialogue_npc_at(5, initial_npc_tile)]);
    let mut moved = snapshot();
    moved.seed_local_player(super::tests::local_player(player_tile));
    moved.seed_npcs(vec![dialogue_npc_at(5, moved_npc_tile)]);
    let mut settled = snapshot();
    settled.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 7,
        z: 5,
        level: 0,
    }));
    settled.seed_npcs(vec![dialogue_npc_at(1, moved_npc_tile)]);

    let mut ledger = None;
    let (handle, initial) = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        let result = tick.actions.poll(&handle, &mut tick.cx);
        (handle, result)
    });
    assert!(initial.is_pending());
    assert!(
        matches!(
            super::tests::last_route_effect(&ledger),
            HostEffect::AssessWalk(request) if request.target == initial_npc_tile
        ),
        "no flood reaches the NPC, so the router assesses the first route"
    );
    super::tests::post_assess_receipt(&mut ledger, 1, 5, None);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walk_target(&ledger), Some(initial_npc_tile));

    super::tests::post_user_input_walk_receipt(&mut ledger, 3);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(
        with_tick(&moved, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending(),
        "a moved, still-visible NPC should cause an approach re-plan"
    );
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == moved_npc_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 3, 5, None);
    assert!(with_tick(&moved, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        live_walk_target(&ledger),
        Some(moved_npc_tile),
        "the re-plan must target the NPC's current tile"
    );

    super::tests::post_user_input_walk_receipt(&mut ledger, 5);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(with_tick(&settled, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::Npc { index: Some(7), .. }
    ));
}

/// A snapshot with the player at `player` and no NPC in view.
fn unseen_npc_snapshot(player: api::WorldTile) -> GameSnapshot {
    let mut gone = snapshot();
    gone.seed_local_player(super::tests::local_player(player));
    gone.seed_npcs(Vec::new());
    gone
}

fn sent_interactions(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter(|entry| matches!(&entry.effect, HostEffect::Interaction(_)))
        .count()
}

/// Begin a Dialogue with Aubury 10 tiles east; the router assesses the route.
fn begin_far_aubury() -> (
    Option<Box<ledger::Ledger>>,
    crate::native::ActionHandle<Dialogue>,
    GameSnapshot,
    api::WorldTile,
) {
    let npc_tile = api::WorldTile {
        x: 10,
        z: 5,
        level: 0,
    };
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 0,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![dialogue_npc_at(10, npc_tile)]);
    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == npc_tile
    ));
    (ledger, handle, before, npc_tile)
}

/// Dig Site `pass-first-exam` (QUESTER-TALK-APPROACH-HOLD): one poll without
/// the NPC in the snapshot ended the step with `dialogue failed`. The route
/// now holds the last seen tile while the NPC is unseen, and the Talk-to
/// follows when the NPC is seen again.
#[test]
fn dialogue_approach_holds_the_route_while_the_npc_is_unseen_then_talks() {
    let (mut ledger, handle, _, npc_tile) = begin_far_aubury();
    let first = last_assessment_id(&ledger);
    // Unseen while the router assesses the route: the assessment stays.
    let start = unseen_npc_snapshot(api::WorldTile {
        x: 0,
        z: 5,
        level: 0,
    });
    assert!(
        with_tick(&start, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending(),
        "an unseen NPC during the route assessment is not a failure"
    );
    assert_eq!(last_assessment_id(&ledger), first);
    super::tests::post_assess_receipt(&mut ledger, 2, 10, None);
    // Still unseen when the answer arrives: the walk goes to the last seen tile.
    assert!(with_tick(&start, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walk_target(&ledger), Some(npc_tile));
    let mut walks = WalkTally::default();
    walks.observe(&ledger);
    for (observed, x) in [(4, 2), (5, 4), (6, 6)] {
        let walking = unseen_npc_snapshot(api::WorldTile { x, z: 5, level: 0 });
        assert!(
            with_tick(&walking, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "the unseen NPC does not end the approach at tick {observed}"
        );
        assert_eq!(live_walk_target(&ledger), Some(npc_tile));
        assert_eq!(live_walks(&ledger), 1);
        walks.observe(&ledger);
    }
    assert_eq!(walks.count(), 1, "the route continues: no new walk");
    assert_eq!(sent_interactions(&ledger), 0, "no Talk-to to an unseen NPC");

    // Seen again beside the bot: the Talk-to follows and the walk ends.
    let mut seen = snapshot();
    seen.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 9,
        z: 5,
        level: 0,
    }));
    seen.seed_npcs(vec![dialogue_npc_at(1, npc_tile)]);
    assert!(with_tick(&seen, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::Npc { index: Some(7), name, .. } if name == "Aubury"
    ));
    assert_eq!(live_walks(&ledger), 0);
}

/// The NPC never comes back: three routes to its last seen tile end, and the
/// final error names the NPC, last seen tile and last route detail.
#[test]
fn dialogue_approach_unseen_npc_fails_with_its_name_and_last_seen_tile() {
    let (mut ledger, handle, before, npc_tile) = begin_far_aubury();
    super::tests::post_assess_receipt(&mut ledger, 1, 10, None);
    assert!(with_tick(&before, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(live_walk_target(&ledger), Some(npc_tile));
    let mut walks = WalkTally::default();
    walks.observe(&ledger);
    for observed in 3..8 {
        let walking = unseen_npc_snapshot(api::WorldTile {
            x: i32::try_from(observed).unwrap(),
            z: 5,
            level: 0,
        });
        assert!(with_tick(&walking, &mut ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(live_walk_target(&ledger), Some(npc_tile));
    }
    // The walk arrives at the last seen tile and the NPC is not there.
    let arrived = unseen_npc_snapshot(api::WorldTile {
        x: 9,
        z: 5,
        level: 0,
    });
    super::tests::post_user_input_walk_receipt(&mut ledger, 8);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = crate::native::WalkEnd::Arrived;
    assert!(with_tick(&arrived, &mut ledger, 8, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the fresh plan waits one observed tick to read the snapshot again"
    );
    assert!(with_tick(&arrived, &mut ledger, 9, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == npc_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 9, 0, None);
    assert!(with_tick(&arrived, &mut ledger, 10, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    walks.observe(&ledger);
    assert_eq!(walks.count(), 2);
    super::tests::post_walk_receipt(&mut ledger, 11, crate::native::WalkEnd::Arrived, None);
    assert!(with_tick(&arrived, &mut ledger, 11, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&arrived, &mut ledger, 12, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        super::tests::last_route_effect(&ledger),
        HostEffect::AssessWalk(request) if request.target == npc_tile
    ));
    super::tests::post_assess_receipt(&mut ledger, 12, 0, None);
    assert!(with_tick(&arrived, &mut ledger, 13, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    walks.observe(&ledger);
    assert_eq!(walks.count(), 3);
    super::tests::post_walk_receipt(
        &mut ledger,
        14,
        crate::native::WalkEnd::NoPath,
        Some("last route failure"),
    );
    assert!(matches!(
        with_tick(&arrived, &mut ledger, 14, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref()
                == "dialogue approach failed: the walk ended before the NPC was in reach: last route failure; \
                    Aubury is not in view, last seen at (10, 5, level 0)"
    ));
    assert_eq!(sent_interactions(&ledger), 0, "no Talk-to to an unseen NPC");
    assert_eq!(live_walks(&ledger), 0);
}

#[test]
fn dialogue_approach_timeout_counts_observed_ticks_not_wall_time() {
    let mut before = snapshot();
    before.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 5,
        z: 5,
        level: 0,
    }));
    before.seed_npcs(vec![dialogue_npc(5)]);

    let mut ledger = None;
    let handle = with_tick(&before, &mut ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args(), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    });

    super::tests::post_assess_receipt(&mut ledger, 1, 5, None);
    for observed_tick in 2..=9 {
        let result = with_tick(&before, &mut ledger, observed_tick, |tick| {
            tick.cx.active_now = if observed_tick == 2 {
                Duration::from_secs(86_400)
            } else {
                Duration::ZERO
            };
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(
            result.is_pending(),
            "the five-tile route plus two slack ticks, counted from the walk at tick 2, ends at tick 9 with a fresh plan, not a failure"
        );
    }
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "the ended route leaves no request queued until the fresh plan"
    );
    assert!(with_tick(&before, &mut ledger, 10, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(
        matches!(
            super::tests::last_route_effect(&ledger),
            HostEffect::AssessWalk(_)
        ),
        "the fresh plan follows the ended route one observed tick later"
    );
}
#[test]
fn dialogue_main_scroll_and_book_actions_retry_after_budget_exhaustion() {
    let ids = dialogue_ui();
    for book in [false, true] {
        let mut snapshot = snapshot();
        if book {
            snapshot.seed_main_modal(ids.book_root, book_page(ids, "Page one", true));
        } else {
            snapshot.seed_main_modal(ids.scroll_root, vec![]);
        }
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick
                .actions
                .begin::<Dialogue>(continuation_args(), &mut tick.cx)
                .unwrap();
            spend_tick_interaction(tick);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);

        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        if book {
            assert!(matches!(
                last_interaction(&ledger),
                InteractReq::IfButton { component_id } if *component_id == ids.book_forward
            ));
        } else {
            assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
        }
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 2);
    }
}

#[test]
fn dialogue_gap_rearms_share_one_per_tick_marker_across_evidence_and_animation() {
    let mut snapshot = snapshot();
    let mut args = continuation_args();
    args.options.gap_ticks = Some(2);
    snapshot.seed_chat_modal(100, vec!["Still working.".into()]);
    snapshot.seed_chat_options(vec![], -1);
    let player_at = |x, z, animation| {
        let mut player = super::tests::local_player(api::WorldTile { x, z, level: 0 });
        player.player.actor.animation = animation;
        player
    };
    snapshot.seed_local_player(player_at(5, 5, -1));
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions.begin::<Dialogue>(args, &mut tick.cx).unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    snapshot.seed_inventory(
        vec![api::snapshot::ItemView {
            def: super::tests::def(1, "Token"),
            container: api::snapshot::ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_local_player(player_at(6, 5, -1));
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "The mechanism turns.".into(),
        type_: 0,
        username: None,
    }]);
    assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_local_player(player_at(7, 5, -1));
    assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());

    snapshot.seed_local_player(player_at(7, 5, 898));
    assert!(with_tick(&snapshot, &mut ledger, 11, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    snapshot.seed_inventory(
        vec![api::snapshot::ItemView {
            def: super::tests::def(1, "Token"),
            container: api::snapshot::ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot: 0,
            count: 2,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    assert!(with_tick(&snapshot, &mut ledger, 11, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&snapshot, &mut ledger, 15, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 19, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
}

/// One chat page with option rows. `continue_id` >= 0 makes it a Continue
/// page; -1 makes it a menu page.
fn menu_page(root: i32, texts: &[&str], options: &[&str], continue_id: i32) -> GameSnapshot {
    let mut page = snapshot();
    page.seed_chat_modal(root, texts.iter().map(|text| (*text).to_owned()).collect());
    page.seed_chat_options(
        options
            .iter()
            .enumerate()
            .map(|(index, text)| api::snapshot::ChatOptionView {
                component_id: root + 1 + index as i32,
                text: (*text).to_owned(),
            })
            .collect(),
        continue_id,
    );
    page
}

fn continuation_with(options: DialogueOptions) -> DialogueArgs {
    DialogueArgs {
        target: DialogueTarget::Continuation,
        options,
    }
}

fn answer_options(prefer: &[&str], answer_once: bool, strict: bool) -> DialogueOptions {
    DialogueOptions {
        prefer: prefer.iter().map(|fragment| Arc::from(*fragment)).collect(),
        answer_once,
        strict,
        ..DialogueOptions::default()
    }
}

fn answers(ledger: &Option<Box<ledger::Ledger>>) -> Vec<i32> {
    ledger
        .as_ref()
        .expect("dialogue test ledger")
        .outbox
        .iter()
        .filter_map(|entry| match &entry.effect {
            HostEffect::Interaction(InteractReq::Answer { option }) => Some(*option),
            _ => None,
        })
        .collect()
}

const FILLIMAN_MENU: [&str; 4] = [
    "I'm wearing an amulet of ghost speak!",
    "How long have you been a ghost?",
    "What's it like being a ghost?",
    "Ok, thanks.",
];

/// Filliman's shape (`filliman.rs2:14,255-269,233-234`): every question
/// reopens the same four-option menu, and "Ok, thanks." leaves. Returns the
/// answers sent and whether the conversation completed.
fn drive_filliman(answer_once: bool) -> (Vec<i32>, bool) {
    let first = menu_page(100, &["Oh, I understand you!"], &FILLIMAN_MENU, -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(
                    &[FILLIMAN_MENU[1], FILLIMAN_MENU[3]],
                    answer_once,
                    false,
                )),
                &mut tick.cx,
            )
            .unwrap()
    });
    let poll = |page: &GameSnapshot, tick: u64, ledger: &mut Option<Box<ledger::Ledger>>| {
        with_tick(page, ledger, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    };
    assert!(poll(&first, 2, &mut ledger).is_pending());
    assert_eq!(answers(&ledger), [2]);
    complete_last_interaction(&mut ledger, 2, true);

    let question = menu_page(100, &["How long have you been a ghost?"], &[], 101);
    assert!(poll(&question, 3, &mut ledger).is_pending());
    assert!(poll(&question, 4, &mut ledger).is_pending());
    let reply = menu_page(
        100,
        &["What?! Don't be preposterous! I'm not a ghost!"],
        &[],
        101,
    );
    assert!(poll(&reply, 5, &mut ledger).is_pending());
    let second_menu = menu_page(
        100,
        &["What?! Don't be preposterous! I'm not a ghost!"],
        &FILLIMAN_MENU,
        -1,
    );
    assert!(poll(&second_menu, 6, &mut ledger).is_pending());

    let thanks = menu_page(100, &["Ok, thanks."], &[], 101);
    assert!(poll(&thanks, 7, &mut ledger).is_pending());
    assert!(poll(&thanks, 8, &mut ledger).is_pending());
    let closed = menu_page(-1, &[], &[], -1);
    assert!(poll(&closed, 9, &mut ledger).is_pending());
    let completed = matches!(
        poll(&closed, 10, &mut ledger),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    );
    (answers(&ledger), completed)
}

#[test]
fn answer_once_answers_a_repeated_menu_question_once_then_leaves() {
    let (answers, completed) = drive_filliman(true);
    assert_eq!(answers, [2, 4]);
    assert!(completed, "the conversation leaves after Ok, thanks.");
}

#[test]
fn answer_once_false_repeats_the_first_preferred_answer() {
    let (answers, completed) = drive_filliman(false);
    assert_eq!(
        answers,
        [2, 2],
        "without answer_once the first preference answers every menu"
    );
    assert!(completed);
}

#[test]
fn answer_once_refused_answer_does_not_consume_the_entry() {
    let first = menu_page(100, &["Choose."], &["Buy", "Sell", "Leave"], -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(&["Buy", "Sell"], true, false)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [1]);
    // The host refuses the answer, yet another page arrives: queueing is not
    // acceptance, so the entry stays unused and answers again.
    complete_last_interaction(&mut ledger, 2, false);
    let second = menu_page(100, &["Choose again."], &["Buy", "Sell", "Leave"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&second, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        answers(&ledger),
        [1, 1],
        "a refused answer leaves its prefer entry unused for the next menu"
    );
}

#[test]
fn answer_once_accepted_answer_consumes_the_entry() {
    let first = menu_page(100, &["Choose."], &["Buy", "Sell", "Leave"], -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(&["Buy", "Sell"], true, false)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [1]);
    complete_last_interaction(&mut ledger, 2, true);
    let second = menu_page(100, &["Choose again."], &["Buy", "Sell", "Leave"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&second, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        answers(&ledger),
        [1, 2],
        "an accepted answer consumes its prefer entry"
    );
}

/// Runs a dialogue and reports the conversation's used-`prefer` mask when
/// the driver finishes.
struct AnswerMaskProbe {
    dialogue: Dialogue,
}

impl NativeMachine for AnswerMaskProbe {
    type Args = DialogueArgs;
    type Output = (DialogueOutcome, u32);

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            dialogue: <Dialogue as NativeMachine>::begin(args, cx)?,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match self.dialogue.poll(cx) {
            Poll::Ready(Ok(outcome)) => Poll::Ready(Ok((outcome, self.dialogue.answers_used()))),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn cancel(&mut self) {}
}

#[test]
fn answer_once_timeout_without_acknowledgement_does_not_consume_the_entry() {
    let page = menu_page(100, &["Choose."], &["Buy", "Leave"], -1);
    let mut ledger = None;
    let handle = with_tick(&page, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<AnswerMaskProbe>(
                continuation_with(answer_options(&["Buy"], true, false)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&page, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [1]);
    // The page never changes and no receipt arrives. The answer was sent at
    // 1_200 ms with PAGE_ACK_MS to acknowledge, so tick 8 fails the page.
    match with_tick(&page, &mut ledger, 8, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    }) {
        Poll::Ready(Ok((DialogueOutcome::Failed, used))) => assert_eq!(
            used, 0,
            "an unacknowledged answer must not consume its prefer entry"
        ),
        _ => panic!("an unacknowledged answer must fail the conversation"),
    }
}

#[test]
fn answer_once_marks_one_prefer_entry_per_page_even_for_one_option_text() {
    let first = menu_page(100, &["Choose."], &["Yes", "No"], -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(&["Yes", "Yes"], true, false)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    complete_last_interaction(&mut ledger, 2, true);
    let second = menu_page(100, &["Choose again."], &["Yes", "No"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&second, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        answers(&ledger),
        [1, 1],
        "the second entry answers with the same option, not the fallback"
    );
}

#[test]
fn answer_once_skips_a_used_entry_and_uses_an_unused_entry_on_a_changed_menu() {
    let first = menu_page(100, &["Choose."], &["Buy", "Leave"], -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(&["Buy", "Sell"], true, false)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    complete_last_interaction(&mut ledger, 2, true);
    let second = menu_page(100, &["Choose again."], &["Sell", "Leave"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&second, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [1, 1]);
}

#[test]
fn answer_once_strict_fails_when_every_matching_entry_was_used() {
    let first = menu_page(100, &["Choose."], &["Yes", "No"], -1);
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(answer_options(&["Yes"], true, true)),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [1]);
    complete_last_interaction(&mut ledger, 2, true);
    let second = menu_page(100, &["Choose again."], &["Yes", "No"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    let sent_before = ledger.as_ref().unwrap().outbox.len();
    assert!(matches!(
        with_tick(&second, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
    assert_eq!(answers(&ledger), [1]);
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        sent_before,
        "a failed answer rule sends nothing"
    );
}

#[test]
fn answer_once_line_rules_take_precedence_without_consuming_a_prefer_entry() {
    let first = menu_page(
        100,
        &["What is the correct incantation?"],
        &["Rule answer", "Other"],
        -1,
    );
    let mut ledger = None;
    let handle = with_tick(&first, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(
                continuation_with(DialogueOptions {
                    prefer: Arc::from([Arc::from("Rule answer")]),
                    line_rules: Arc::from([super::dialogue::LineRule {
                        when_line: Arc::from("incantation"),
                        choose: Arc::from("Other"),
                    }]),
                    answer_once: true,
                    ..DialogueOptions::default()
                }),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&first, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(answers(&ledger), [2], "the line rule answers first");
    let second = menu_page(100, &["Choose an option."], &["Rule answer", "Other"], -1);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(with_tick(&second, &mut ledger, 4, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(
        answers(&ledger),
        [2, 1],
        "the prefer entry stays unused after the line rule answers"
    );
}

#[test]
fn answer_once_rejects_more_prefer_entries_than_the_mask_width() {
    let document = |answer_once: bool| super::DialogueOptionsDocument {
        prefer: (0..=super::dialogue::ANSWER_ONCE_PREFER_MAX)
            .map(|index| format!("option {index}"))
            .collect(),
        choose: None,
        line_rules: Vec::new(),
        strict: false,
        answer_once,
        gap_ticks: None,
    };
    let error = match super::compile_dialogue_options(document(true)) {
        Err(error) => error,
        Ok(_) => panic!("answer_once must cap prefer at the mask width"),
    };
    assert_eq!(error.code.as_ref(), "invalid-dialogue-options");
    assert!(
        super::compile_dialogue_options(document(false)).is_ok(),
        "without answer_once the preference list keeps its existing width"
    );
}

/// Continue requests the driver has emitted, accepted or not.
pub(super) fn continues(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .expect("dialogue test ledger")
        .outbox
        .iter()
        .filter(|entry| {
            matches!(
                &entry.effect,
                HostEffect::Interaction(InteractReq::ContinueDialog { .. })
            )
        })
        .count()
}

/// Talk-to clicks queued in the outbox.
pub(super) fn talks(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger
        .as_ref()
        .expect("dialogue test ledger")
        .outbox
        .iter()
        .filter(|entry| {
            matches!(
                &entry.effect,
                HostEffect::Interaction(InteractReq::Npc { .. })
            )
        })
        .count()
}

/// Begin a continuation on `page` at tick 1 and send its first Continue at
/// tick 2. Returns the handle; the Continue was requested on observed tick 2.
fn continue_sent_at_tick_two(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> crate::native::ActionHandle<Dialogue> {
    let handle = with_tick(page, ledger, 1, |tick| {
        tick.actions
            .begin::<Dialogue>(continuation_args(), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(page, ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(continues(ledger), 1);
    handle
}

/// Merlin's Crystal sends `And wait...` twice in a row (`quest_arthur.rs2:15-16`).
/// The server reuses the chat root and the Continue component, so page two has
/// the same modal, Continue visibility and text as page one. The accepted
/// Continue receipt, read on a later observed tick, acknowledges page one;
/// the driver then presses Continue on page two instead of failing.
#[test]
fn two_identical_consecutive_continue_pages_are_both_acknowledged() {
    let wait = menu_page(100, &["And wait..."], &[], 101);
    let mut ledger = None;
    let handle = continue_sent_at_tick_two(&wait, &mut ledger);
    complete_last_interaction(&mut ledger, 2, true);
    let poll = |page: &GameSnapshot, tick: u64, ledger: &mut Option<Box<ledger::Ledger>>| {
        with_tick(page, ledger, tick, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
    };

    // The engine reads the Continue in the cycle after the receipt's tick and
    // writes page two after that cycle's PLAYER_INFO. Tick 3 may still show
    // page one's packets: an accept is not yet an acknowledgement.
    assert!(poll(&wait, 3, &mut ledger).is_pending());
    assert_eq!(continues(&ledger), 1, "no second Continue on the next tick");
    // Tick 4: page two is open and looks like page one.
    assert!(poll(&wait, 4, &mut ledger).is_pending());
    assert_eq!(
        continues(&ledger),
        2,
        "the accepted receipt acknowledges page one"
    );
    complete_last_interaction(&mut ledger, 4, true);

    let closed = menu_page(-1, &[], &[], -1);
    assert!(poll(&closed, 5, &mut ledger).is_pending());
    assert!(matches!(
        poll(&closed, 6, &mut ledger),
        Poll::Ready(Ok(DialogueOutcome::Completed))
    ));
    assert_eq!(continues(&ledger), 2);
}

/// The receipt is an extra signal: a page that changed before the receipt
/// matures is acknowledged at once, as before.
#[test]
fn a_changed_page_still_acknowledges_a_continue_without_a_receipt() {
    let first = menu_page(100, &["You climb inside the crate."], &[], 101);
    let mut ledger = None;
    let handle = continue_sent_at_tick_two(&first, &mut ledger);
    let second = menu_page(100, &["You wait."], &[], 101);
    assert!(with_tick(&second, &mut ledger, 3, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(continues(&ledger), 2);
}

/// A request that no accepted receipt reaches is bound by observed ticks, not
/// by elapsed time: sent on tick 2, it is acknowledged or failed by tick 12
/// (`PAGE_ACK_TICKS` = 10), whatever the wall clock says.
#[test]
fn continue_acknowledgement_bound_counts_observed_ticks_not_milliseconds() {
    let wait = menu_page(100, &["And wait..."], &[], 101);
    let mut ledger = None;
    let handle = continue_sent_at_tick_two(&wait, &mut ledger);
    for observed in 3..12 {
        let result = with_tick(&wait, &mut ledger, observed, |tick| {
            // An hour of active time passes. The bound ignores it.
            tick.cx.active_now = Duration::from_secs(3_600);
            tick.actions.poll(&handle, &mut tick.cx)
        });
        assert!(result.is_pending(), "observed tick {observed}");
    }
    let result = with_tick(&wait, &mut ledger, 12, |tick| {
        // Almost no wall time has passed. The tick bound still ends the wait.
        tick.cx.active_now = Duration::from_millis(1);
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(matches!(result, Poll::Ready(Ok(DialogueOutcome::Failed))));
    assert_eq!(continues(&ledger), 1, "no unacknowledged second Continue");
}

/// A refused Continue was not dispatched, so its receipt acknowledges nothing.
#[test]
fn a_refused_continue_receipt_does_not_acknowledge_the_page() {
    let wait = menu_page(100, &["And wait..."], &[], 101);
    let mut ledger = None;
    let handle = continue_sent_at_tick_two(&wait, &mut ledger);
    complete_last_interaction(&mut ledger, 2, false);
    for observed in 3..12 {
        assert!(
            with_tick(&wait, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    assert_eq!(continues(&ledger), 1);
    assert!(matches!(
        with_tick(&wait, &mut ledger, 12, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
}

/// A page that never changes, whatever the server does with the presses, is
/// bound by the tick limit of its first press. Each accepted receipt
/// acknowledges one press and the driver presses again, but the repeats add
/// no time and are not new pages: the wait ends 10 ticks after the first press
/// (tick 2), not after `DRIVE_STEPS` presses.
#[test]
fn a_page_that_never_changes_ends_at_the_tick_bound_of_its_first_press() {
    let stuck = menu_page(100, &["A page that will not advance."], &[], 101);
    let mut ledger = None;
    let handle = continue_sent_at_tick_two(&stuck, &mut ledger);
    complete_last_interaction(&mut ledger, 2, true);
    let mut result = Poll::Pending;
    let mut ended = 0;
    let mut accepted = 1;
    for observed in 3..40 {
        result = with_tick(&stuck, &mut ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        if result.is_ready() {
            ended = observed;
            break;
        }
        // The host accepts every press on the tick that the driver sent it.
        if continues(&ledger) > accepted {
            complete_last_interaction(&mut ledger, observed, true);
            accepted += 1;
        }
    }
    assert!(matches!(result, Poll::Ready(Ok(DialogueOutcome::Failed))));
    assert_eq!(
        ended, 13,
        "the receipt of the press at tick 12 matures at tick 14, one tick after the bound"
    );
    assert_eq!(
        continues(&ledger),
        6,
        "one press at tick 2 and one after each matured receipt"
    );
}

/// Press at tick 2 (bound: tick 12) and again at tick 4, with that press's
/// receipt accepted on tick 4. Returns the handle.
fn repeated_press_with_receipt_outstanding(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> crate::native::ActionHandle<Dialogue> {
    let handle = continue_sent_at_tick_two(page, ledger);
    complete_last_interaction(ledger, 2, true);
    for (observed, presses) in [(3, 1), (4, 2)] {
        assert!(with_tick(page, ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(continues(ledger), presses, "observed tick {observed}");
    }
    complete_last_interaction(ledger, 4, true);
    handle
}

/// Observations skip ticks (`snapshot/decode.rs:95-104`, a hold in
/// `script_observe.rs:967-994`). A poll that jumps over the first press's
/// bound with a matured accepted receipt outstanding must still end the wait:
/// the receipt cannot renew the bound of a page that did not change.
#[test]
fn a_poll_that_jumps_over_the_bound_does_not_renew_it_for_an_unchanged_page() {
    let stuck = menu_page(100, &["A page that will not advance."], &[], 101);
    let mut ledger = None;
    let handle = repeated_press_with_receipt_outstanding(&stuck, &mut ledger);
    // The bound is tick 12. The tick-4 receipt matured at tick 6.
    let result = with_tick(&stuck, &mut ledger, 13, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(matches!(result, Poll::Ready(Ok(DialogueOutcome::Failed))));
    assert_eq!(continues(&ledger), 2, "no press after the bound");
}

/// The same jump with a page that changed is an acknowledgement, as before:
/// the new page is pressed and gets its own bound.
#[test]
fn a_poll_that_jumps_over_the_bound_still_acknowledges_a_changed_page() {
    let stuck = menu_page(100, &["A page that will not advance."], &[], 101);
    let mut ledger = None;
    let handle = repeated_press_with_receipt_outstanding(&stuck, &mut ledger);
    let next = menu_page(100, &["The next page."], &[], 101);
    assert!(with_tick(&next, &mut ledger, 13, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(continues(&ledger), 3);
    complete_last_interaction(&mut ledger, 13, true);
    // The new press has its own ten ticks: the bound is tick 23.
    for observed in 14..23 {
        assert!(
            with_tick(&next, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    assert!(matches!(
        with_tick(&next, &mut ledger, 23, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
}

/// The Cook beside a player one tile away, as the open-wait tests use.
pub(super) fn cook_beside_player() -> GameSnapshot {
    let mut page = snapshot();
    page.seed_local_player(super::tests::local_player(api::WorldTile {
        x: 6,
        z: 5,
        level: 0,
    }));
    page.seed_npcs(vec![content_dialogue_npc(
        content_npc_id("cook"),
        "Cook",
        1,
        api::WorldTile {
            x: 7,
            z: 5,
            level: 0,
        },
    )]);
    page
}

/// The Cook on (0, 5) and the player `distance` tiles east of it on the same
/// row. No reach flood is posted, so an approach asks the router for the
/// route first (`Phase::Plan`); within two tiles the Cook is in reach.
pub(super) fn cook_away_from_player(distance: i32) -> GameSnapshot {
    let mut page = snapshot();
    page.seed_local_player(super::tests::local_player(api::WorldTile {
        x: distance,
        z: 5,
        level: 0,
    }));
    page.seed_npcs(vec![content_dialogue_npc(
        content_npc_id("cook"),
        "Cook",
        distance,
        api::WorldTile {
            x: 0,
            z: 5,
            level: 0,
        },
    )]);
    page
}

fn begin_cook_talk(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> crate::native::ActionHandle<Dialogue> {
    with_tick(page, ledger, 1, |tick| {
        let handle = tick
            .actions
            .begin::<Dialogue>(npc_args_for(content_npc_id("cook"), "Cook"), &mut tick.cx)
            .unwrap();
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
        handle
    })
}

/// The page budget starts at the host's acceptance of the Talk-to. A host
/// that accepts on tick 7 (the count from the emit tick would have failed on
/// tick 4) still gets the whole budget after the click, and a page on that
/// deadline is observed.
#[test]
fn a_talk_to_the_host_accepts_late_gets_the_page_budget_after_the_click() {
    let page = cook_beside_player();
    let mut ledger = None;
    let handle = begin_cook_talk(&page, &mut ledger);
    for observed in 2..8 {
        assert!(
            with_tick(&page, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}, host receipt not yet posted"
        );
    }
    complete_talk(&mut ledger, 7, true, false);
    // Arrival was observed on tick 2; the click tick is 7, so the engine's
    // trigger is tick 8 and the page deadline is tick 12.
    for observed in 8..12 {
        assert!(
            with_tick(&page, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    let mut opened = snapshot();
    opened.seed_chat_modal(100, vec!["What can I do for you?".to_owned()]);
    opened.seed_chat_options(vec![], 101);
    assert!(with_tick(&opened, &mut ledger, 12, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
}

/// The host still holds a Talk-to for as long as no pass that may dispatch
/// has run (a held or disconnected host does not take queued work,
/// `script_observe.rs:1344-1363`). That wait has no tick bound, sends no
/// second click and does not run the page budget: here the host takes the
/// click on tick 60 and the page budget then counts from that click.
#[test]
fn a_talk_to_the_host_still_holds_waits_without_a_tick_bound() {
    let page = cook_beside_player();
    let mut ledger = None;
    let handle = begin_cook_talk(&page, &mut ledger);
    for observed in 2..60 {
        assert!(
            with_tick(&page, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}: the host still holds the click"
        );
    }
    assert_eq!(talks(&ledger), 1, "a held click is not sent again");
    complete_talk(&mut ledger, 60, true, false);
    // Arrival was observed before the click; the click tick is 60, so the engine's
    // trigger is tick 61 and the page deadline is tick 65.
    for observed in 60..65 {
        assert!(
            with_tick(&page, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    assert!(matches!(
        with_tick(&page, &mut ledger, 65, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "expected dialogue did not open"
    ));
}

/// A held tick drops queued gameplay unsent (`SlotScript::seat_held_tick`).
/// The dropped Talk-to will never get a receipt, so the dialogue sends it
/// again on its next poll, and the page budget counts from the new click.
#[test]
fn a_talk_to_the_host_dropped_unsent_is_sent_again() {
    let page = cook_beside_player();
    let mut ledger = None;
    let handle = begin_cook_talk(&page, &mut ledger);
    let first = ledger.as_ref().unwrap().outbox[0].request_id;
    // The host's held tick drops the queued click.
    ledger.as_mut().unwrap().outbox.clear();
    assert!(with_tick(&page, &mut ledger, 9, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(talks(&ledger), 1, "one click is queued again");
    let second = ledger.as_ref().unwrap().outbox[0].request_id;
    assert_ne!(second, first, "the second click is a new request");
    complete_talk(&mut ledger, 9, true, false);
    // Arrival is observed again on tick 10; the trigger is tick 10 and the
    // deadline tick 14.
    for observed in 10..14 {
        assert!(
            with_tick(&page, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    assert!(matches!(
        with_tick(&page, &mut ledger, 14, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "expected dialogue did not open"
    ));
}

/// The host took the Talk-to in a pass that could dispatch it and refused it.
/// That receipt is settled (`Ledger::complete_interaction` keeps the first
/// one), so with no page open the wait ends on the next poll. The error names
/// the NPC, the refusal tick and the last game message since the click.
#[test]
fn a_refused_talk_to_ends_the_wait_at_once_with_the_npc_and_last_message() {
    let page = cook_beside_player();
    let mut ledger = None;
    let handle = begin_cook_talk(&page, &mut ledger);
    complete_talk(&mut ledger, 1, false, false);
    let mut later = cook_beside_player();
    later.seed_chat_lines(vec![game_message(1, "Nothing interesting happens.")]);
    let result = with_tick(&later, &mut ledger, 2, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    });
    assert!(
        matches!(
            &result,
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref()
                    == "expected dialogue did not open: Talk-to Cook: the host refused it on observed tick 1; last game message: Nothing interesting happens."
        ),
        "{result:?}"
    );
    assert_eq!(talks(&ledger), 1, "a refused click is not sent again");
}

/// Press an unchanged page at tick 2 (bound: tick 12); the host accepts every
/// press on its own tick, so a matured receipt is outstanding at tick 12.
/// Returns the handle after the poll of tick 11.
fn unchanged_page_pressed_up_to_the_bound(
    page: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> crate::native::ActionHandle<Dialogue> {
    let handle = continue_sent_at_tick_two(page, ledger);
    complete_last_interaction(ledger, 2, true);
    let mut accepted = 1;
    for observed in 3..12 {
        assert!(with_tick(page, ledger, observed, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        if continues(ledger) > accepted {
            complete_last_interaction(ledger, observed, true);
            accepted += 1;
        }
    }
    assert_eq!(continues(ledger), 5, "presses on ticks 2, 4, 6, 8 and 10");
    handle
}

/// Another operation spent tick 12's interaction allowance, so the repeat
/// that the tick-10 receipt allows at the bound is deferred. The deferred
/// repeat keeps its identity: the poll after the bound ends the wait with
/// the first press's bound and sends nothing.
#[test]
fn a_repeat_deferred_at_the_bound_does_not_renew_it() {
    let stuck = menu_page(100, &["A page that will not advance."], &[], 101);
    let mut ledger = None;
    let handle = unchanged_page_pressed_up_to_the_bound(&stuck, &mut ledger);
    assert!(with_tick(&stuck, &mut ledger, 12, |tick| {
        assert!(tick.cx.budget.event(false), "the tick's event was free");
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(continues(&ledger), 5, "the repeat at the bound is deferred");
    assert!(matches!(
        with_tick(&stuck, &mut ledger, 13, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
    assert_eq!(continues(&ledger), 5, "no press after the bound");
}

/// The same deferral, then a changed page: the new page is pressed and gets
/// its own bound (tick 23), as any acknowledged page does.
#[test]
fn a_changed_page_after_a_deferred_repeat_gets_its_own_bound() {
    let stuck = menu_page(100, &["A page that will not advance."], &[], 101);
    let mut ledger = None;
    let handle = unchanged_page_pressed_up_to_the_bound(&stuck, &mut ledger);
    assert!(with_tick(&stuck, &mut ledger, 12, |tick| {
        assert!(tick.cx.budget.event(false), "the tick's event was free");
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    let next = menu_page(100, &["The next page."], &[], 101);
    assert!(with_tick(&next, &mut ledger, 13, |tick| {
        tick.actions.poll(&handle, &mut tick.cx)
    })
    .is_pending());
    assert_eq!(continues(&ledger), 6);
    for observed in 14..23 {
        assert!(
            with_tick(&next, &mut ledger, observed, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending(),
            "observed tick {observed}"
        );
    }
    assert!(matches!(
        with_tick(&next, &mut ledger, 23, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Ok(DialogueOutcome::Failed))
    ));
}

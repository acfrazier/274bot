use super::*;
use crate::native::{HostEffect, InteractionReceipt};
use crate::quester::families::tests::with_tick;
use crate::quester::path::{PredicateDocument, ProgressRuleDocument};
use api::snapshot::{GameSnapshot, QuestStatusView};

fn fixture(journal: bool) -> (Quester, GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut doc = super::super::compile::decode_cook().unwrap();
    let step = &mut doc.roles[0].sequences[1].steps[0];
    step.kind = "wait".into();
    step.args = serde_json::json!({"until":{"All":[]},"max_ticks":10});
    step.skip_if = PredicateDocument::Any(vec![]);
    step.advances = journal;
    step.settle = if journal {
        PredicateDocument::Fact {
            kind: "stage_in".into(),
            version: 1,
            args: serde_json::json!({"quest":"cook","any":["cook:mid"]}),
        }
    } else {
        PredicateDocument::All(vec![])
    };
    if journal {
        let mut mid = doc.roles[0].sequences[1].clone();
        mid.stage = FactKey::new("cook:mid");
        mid.terminal = true;
        mid.steps.clear();
        doc.roles[0].sequences.push(mid);
        doc.roles[0].progress.as_mut().unwrap().rules = vec![
            ProgressRuleDocument {
                stage: FactKey::new("cook:mid"),
                all: vec!["next branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(2),
            },
            ProgressRuleDocument {
                stage: FactKey::new("cook:1"),
                all: vec!["seeded branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(1),
            },
            ProgressRuleDocument {
                stage: FactKey::new("cook:0"),
                all: vec!["first branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(0),
            },
        ];
    }
    let path = super::super::compile::compile_uncached_for_test(&doc, &data, &quests).unwrap();
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
        Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            quests,
        ),
        snapshot,
    )
}

type Ledger = Option<Box<crate::native::ledger::Ledger>>;
fn drive(
    script: &mut Quester,
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
) -> ScriptFlow {
    with_tick(snapshot, ledger, tick, |t| script.tick(t).unwrap())
}
fn ack(ledger: &mut Ledger, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().unwrap();
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
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
fn journal(snapshot: &mut GameSnapshot, body: &str) {
    snapshot.seed_main_modal(
        8134,
        vec![
            crate::quest_journal::test_widget(8144, "@dre@The Cook's Quest"),
            crate::quest_journal::test_widget(8145, body),
        ],
    );
}
fn finish_read(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
    body: &str,
) {
    drive(script, snapshot, ledger, tick);
    assert!(matches!(
        ack(ledger, tick),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
    journal(snapshot, body);
    drive(script, snapshot, ledger, tick + 1);
    drive(script, snapshot, ledger, tick + 2);
    assert!(matches!(
        ack(ledger, tick + 2),
        HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    drive(script, snapshot, ledger, tick + 3);
}

#[test]
fn unknown_skip_never_dispatches_and_wait_is_bounded() {
    let (mut script, mut snapshot) = fixture(false);
    let path = Arc::get_mut(&mut script.path).unwrap();
    struct InventoryUnknown;
    impl super::super::compile::PredicatePlan for InventoryUnknown {
        fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
            if cx.cx.snapshot().inventory().is_some() {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
    }
    path.sequences[1].steps[0].skip_if = Arc::new(InventoryUnknown);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(
        script.step.is_none(),
        "unknown inventory must not start a wait/action"
    );
    for tick in 2..20 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    assert!(!script.parked);
    snapshot.seed_inventory(vec![], 28);
    drive(&mut script, &snapshot, &mut ledger, 20);
    assert!(script.step.is_some());
    let (mut script, snapshot) = fixture(false);
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].skip_if =
        Arc::new(InventoryUnknown);
    for tick in 1..40 {
        drive(&mut script, &snapshot, &mut None, tick);
    }
    assert!(script.parked);
    assert_eq!(script.park_reason, "skip predicate evidence unavailable");
}

#[test]
fn advances_rereads_before_stage_settle_then_retargets_new_sequence() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(
        &mut script,
        &mut snapshot,
        &mut ledger,
        2,
        "@str@seeded branch",
    );
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(script.settling && script.needs_read);
    drive(&mut script, &snapshot, &mut ledger, 7);
    assert!(script.settling, "old proof cannot settle advances");
    finish_read(&mut script, &mut snapshot, &mut ledger, 8, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    assert!(!script.settling);
    assert!(script.progress().unwrap().signals.is_empty());
    assert_eq!(script.last_journal().unwrap().closed.tick, 11);
    assert_eq!(
        drive(&mut script, &snapshot, &mut ledger, 12),
        ScriptFlow::Complete
    );
}

#[test]
fn no_match_parks_with_raw_lines_and_no_invented_stage() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(
        &mut script,
        &mut snapshot,
        &mut ledger,
        2,
        "unrecognised content branch",
    );
    assert!(script.parked);
    assert!(script.stage().is_none());
    assert!(script.step.is_none());
    assert_eq!(
        script.last_journal().unwrap().lines[0].as_ref(),
        "unrecognised content branch"
    );
    assert!(matches!(
        script.progress().unwrap().stage,
        Knowledge::Unknown(_)
    ));
    assert!(script
        .journal_text
        .as_ref()
        .unwrap()
        .contains("unrecognised content branch"));
}

#[test]
fn explicit_read_is_boundary_latched_and_colour_only_never_opens_journal() {
    let (mut script, snapshot) = fixture(false);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    script.read_journal().unwrap();
    assert!(script.read_requested && !script.needs_read);
    drive(&mut script, &snapshot, &mut ledger, 2);
    drive(&mut script, &snapshot, &mut ledger, 3);
    assert!(script.read_requested);
    drive(&mut script, &snapshot, &mut ledger, 4);
    assert!(!script.read_requested);
    assert!(!script.journal_opened());
    assert!(script.last_journal().is_none());
    assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
}

#[test]
fn recipe_advances_preserves_recipe_until_fresh_stage_settles() {
    let (mut script, mut snapshot) = fixture(true);
    let step = &mut Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0];
    step.plan = Arc::new(super::super::families::AcquirePlan {
        recipe: Arc::from("synthetic-journal"),
        steps: vec![super::super::families::CompiledAcquireStep {
            advances: true,
            skip_if: Arc::clone(&step.skip_if),
            settle: Arc::clone(&step.settle),
            plan: Arc::clone(&step.plan),
        }],
    });
    step.advances = false;
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(script.step.is_some() && script.needs_read && !script.settling);
    drive(&mut script, &snapshot, &mut ledger, 7);
    finish_read(&mut script, &mut snapshot, &mut ledger, 8, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    assert!(script.settling && script.step.is_none());
    drive(&mut script, &snapshot, &mut ledger, 12);
    assert_eq!(
        drive(&mut script, &snapshot, &mut ledger, 13),
        ScriptFlow::Complete
    );
}

#[test]
fn stopping_mid_read_revokes_host_click_and_quiet_lease() {
    let (mut script, snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    script.on_stop(StopReason::Operator);
    let ledger = ledger.as_mut().unwrap();
    assert!(!ledger.outbox[0].live());
    assert!(ledger.quiet_read(std::time::Instant::now()).is_none());
    assert!(script.last_journal().is_none());
}

#[test]
fn read_journal_retries_a_park_with_fresh_failure_budgets() {
    let (mut script, snapshot) = fixture(true);
    script.parked = true;
    script.fail_streak = 3;
    script.attempts = 3;
    script.empty_reads = 2;
    script.unreadable_reads = 2;
    script.unreadable_since = Some(Duration::ZERO);
    script.last_error = Some(Arc::from("previous step failure"));
    script.park_reason = "previous watchdog park";
    for _ in 0..9 {
        script.watchdog.observe(None, None, &[], &[], 0);
    }
    assert_eq!(
        script.watchdog.observe(None, None, &[], &[], 0),
        WatchdogAction::Park
    );
    script.read_journal().unwrap();
    assert!(!script.parked);
    assert_eq!(script.fail_streak, 0);
    assert_eq!(script.attempts, 0);
    assert_eq!(script.empty_reads, 0);
    assert_eq!(script.unreadable_reads, 0);
    assert!(script.last_error.is_none());
    assert!(script.unreadable_since.is_none());
    assert_eq!(
        script.watchdog.observe(None, None, &[], &[], 0),
        WatchdogAction::None,
        "Read now on a park must reset the watchdog too"
    );
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    assert!(matches!(
        ack(&mut ledger, 2),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id: 42 })
    ));
}

#[test]
fn blocked_slot_read_journal_reopens_dispatch_and_emits_a_real_read() {
    let (mut script, snapshot) = fixture(true);
    script.parked = true;
    script.run.session = 0;
    let mut slot = crate::SlotScript::new();
    slot.bind_incarnation(1);
    slot.start_test_script(Box::new(script), None).unwrap();
    let tick = |slot: &mut crate::SlotScript, tick| {
        slot.on_game_tick(&mut crate::ScriptCtx {
            driver: &mut crate::ctx::test_support::NullDriver::default(),
            tick,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(&snapshot),
            obj_names: None,
            compiled: crate::CompiledTick::default(),
        });
    };
    tick(&mut slot, 1);
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Blocked);
    let run = slot.native_run().unwrap();
    slot.read_journal_compiled(run).unwrap();
    assert_eq!(slot.native_status().unwrap().phase, NativePhase::Waiting);
    assert!(slot.native_status().unwrap().failure.is_none());
    tick(&mut slot, 2);
    tick(&mut slot, 3);
    let action = slot.take_native_action().expect("Read now must dispatch");
    assert!(action.live());
    assert!(matches!(
        action.effect,
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id: 42 })
    ));
}

#[test]
fn transient_busy_during_read_retries_but_repeated_busy_is_bounded() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    assert!(!script.parked, "first in-flight Busy must not park");
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 3..=6 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    finish_read(&mut script, &mut snapshot, &mut ledger, 7, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);

    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    for tick in 1..40 {
        snapshot.seed_main_modal(-1, vec![]);
        drive(&mut script, &snapshot, &mut ledger, tick * 2 - 1);
        snapshot.seed_main_modal(123, vec![]);
        drive(&mut script, &snapshot, &mut ledger, tick * 2);
        if script.parked {
            break;
        }
    }
    assert!(script.parked, "transient retry must still have a wall");
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn repeated_busy_transactions_report_retry_limit_and_cause() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 3..=5 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert_eq!(script.journal_attempts, 2);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 7);
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 8..=10 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    drive(&mut script, &snapshot, &mut ledger, 11);
    assert_eq!(script.journal_attempts, 3);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 12);
    assert!(script.parked);
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal read retry limit reached (journal remained busy during read)"
    );
}

#[test]
fn ownership_lost_before_close_retries_without_closing_another_modal() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    assert!(!script.parked);
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    for tick in 5..=8 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    finish_read(&mut script, &mut snapshot, &mut ledger, 9, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
    assert_eq!(script.last_journal().unwrap().acquired.tick, 10);
}

#[test]
fn transient_journal_retry_requires_continuously_closed_observed_ticks() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    for tick in [5, 5, 5] {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(
            script.journal.is_none(),
            "same-frame retries must stay quiet"
        );
    }
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    drive(&mut script, &snapshot, &mut ledger, 6);
    snapshot.seed_chat_modal(-1, vec![]);
    for tick in 7..10 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(
            script.journal.is_none(),
            "chat must reset the quiet interval"
        );
    }
    drive(&mut script, &snapshot, &mut ledger, 10);
    finish_read(&mut script, &mut snapshot, &mut ledger, 11, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
}

#[test]
fn repeated_journal_ownership_loss_caps_row_clicks_per_read() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    let mut clicks = 0;
    for tick in 1..80 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            assert!(
                matches!(
                    ack(&mut ledger, tick),
                    HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
                ),
                "a lost page must never emit a compensating close"
            );
            clicks += 1;
            journal(&mut snapshot, "seeded branch");
        } else {
            snapshot.seed_main_modal(-1, vec![]);
        }
        if script.parked {
            break;
        }
    }
    assert!(script.parked);
    assert_eq!(clicks, 3, "a logical read must not spray row clicks");
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal read retry limit reached (journal ownership repeatedly lost)"
    );
    script.read_journal().unwrap();
    drive(&mut script, &snapshot, &mut ledger, 81);
    finish_read(&mut script, &mut snapshot, &mut ledger, 82, "seeded branch");
    assert!(!script.parked, "operator retry gets a fresh read budget");
}

#[test]
fn colour_only_read_wait_preserves_colour_failure_with_open_chat() {
    let (mut script, mut snapshot) = fixture(false);
    snapshot.seed_quest_statuses(vec![], false);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    let mut ledger = None;
    for tick in 1..=35 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if script.parked {
            break;
        }
    }

    assert!(script.parked);
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "quest colour unavailable or unknown stage"
    );
}

#[test]
fn quiet_gate_wait_names_open_chat_when_it_parks() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    assert!(script.journal_retry_pending);

    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    for tick in 5..=40 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if script.parked {
            break;
        }
    }

    assert!(script.parked);
    assert_eq!(script.park_reason, "journal retry quiet period unavailable");
    assert!(script
        .blocked_failure()
        .message
        .contains("modal root 4882 (Aubury)"));
}

#[test]
fn named_chat_at_read_start_waits_then_recovers_or_names_the_timeout() {
    let (mut script, mut snapshot) = fixture(true);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(!script.parked, "a transient conversation tail must wait");
    snapshot.seed_chat_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    finish_read(&mut script, &mut snapshot, &mut ledger, 3, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");

    let (mut script, mut snapshot) = fixture(true);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    for tick in 1..35 {
        drive(&mut script, &snapshot, &mut None, tick);
    }
    assert!(script.parked);
    assert!(script
        .last_error
        .as_ref()
        .is_some_and(|reason| reason.contains("4882") && reason.contains("Aubury")));
}

#[test]
fn rune_item_handoffs_reread_progress_before_selecting_recovery() {
    use crate::quester::families::tests::{def, seed_dialogue_combat};
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, WorldTile};

    fn finish_rune_read(
        script: &mut Quester,
        snapshot: &mut GameSnapshot,
        ledger: &mut Ledger,
        tick: u64,
        body: &str,
    ) {
        drive(script, snapshot, ledger, tick);
        assert!(
            matches!(
                ledger
                    .as_ref()
                    .and_then(|ledger| ledger.outbox.first())
                    .map(|action| &action.effect),
                Some(HostEffect::Interaction(
                    crate::shim::InteractReq::IfButton { .. }
                ))
            ),
            "a quest-changing handoff must query fresh journal progress before recovery"
        );
        ack(ledger, tick);
        snapshot.seed_main_modal(
            8134,
            vec![
                crate::quest_journal::test_widget(8144, "@dre@Rune Mysteries"),
                crate::quest_journal::test_widget(8145, body),
            ],
        );
        drive(script, snapshot, ledger, tick + 1);
        drive(script, snapshot, ledger, tick + 2);
        assert!(matches!(
            ack(ledger, tick + 2),
            HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
        ));
        snapshot.seed_main_modal(-1, vec![]);
        drive(script, snapshot, ledger, tick + 3);
    }

    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let document = serde_json::from_str(super::super::compile::RUNE_MYSTERIES_JSON).unwrap();
    let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    let spoken = "@str@I spoke to Duke Horacio";
    let talisman_pending =
        format!("{spoken}|I need to find the head wizard and give him the talisman");
    let package_pending =
        format!("{spoken}|I should take this Research Package to Aubury in Varrock");
    let package_delivered =
        format!("{spoken}|I took the research package to Varrock and delivered it.");
    let notes_received = format!("{package_delivered}|I should take the notes to Sedridor");
    let held = |alias: Option<&str>| {
        alias
            .map(|alias| ItemView {
                def: def(data.item_by_alias(alias).unwrap().id, alias),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            })
            .into_iter()
            .collect::<Vec<_>>()
    };
    for (step, input, output, before, after, next, here) in [
        (
            "deliver-talisman",
            Some("air_talisman"),
            Some("research_package"),
            talisman_pending.as_str(),
            package_pending.as_str(),
            "deliver-package",
            WorldTile {
                x: 3108,
                z: 9572,
                level: 0,
            },
        ),
        (
            "deliver-package",
            Some("research_package"),
            None,
            package_pending.as_str(),
            package_delivered.as_str(),
            "collect-notes",
            WorldTile {
                x: 3253,
                z: 3402,
                level: 0,
            },
        ),
        (
            "collect-notes",
            None,
            Some("research_notes"),
            package_delivered.as_str(),
            notes_received.as_str(),
            "deliver-notes",
            WorldTile {
                x: 3253,
                z: 3402,
                level: 0,
            },
        ),
    ] {
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&path),
            Arc::clone(&quests),
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        seed_dialogue_combat(&mut snapshot, false);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.tile = here;
        snapshot.seed_local_player(player);
        snapshot.seed_inventory(held(input), 28);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: quests.quest("runemysteries").unwrap().display.to_string(),
                component_id: 42,
                colour: 0xf8f800,
            }],
            true,
        );
        let mut ledger = None;
        drive(&mut script, &snapshot, &mut ledger, 1);
        finish_rune_read(&mut script, &mut snapshot, &mut ledger, 2, before);
        assert_eq!(script.current_step().unwrap().id.0.as_ref(), step);
        drive(&mut script, &snapshot, &mut ledger, 6);
        assert!(matches!(
            ack(&mut ledger, 6),
            HostEffect::Interaction(crate::shim::InteractReq::Npc { .. })
        ));
        snapshot.seed_chat_modal(4893, vec!["A handoff page".into()]);
        snapshot.seed_chat_options(vec![], 4899);
        drive(&mut script, &snapshot, &mut ledger, 7);
        assert!(matches!(
            ack(&mut ledger, 7),
            HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog)
        ));
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        snapshot.seed_inventory(held(output), 28);
        for tick in 8..=13 {
            drive(&mut script, &snapshot, &mut ledger, tick);
        }
        drive(&mut script, &snapshot, &mut ledger, 14);
        finish_rune_read(&mut script, &mut snapshot, &mut ledger, 15, after);
        drive(&mut script, &snapshot, &mut ledger, 19);
        assert_eq!(
            script.current_step().unwrap().id.0.as_ref(),
            next,
            "{step} must continue with fresh quest progress, not bank or Duke recovery"
        );
    }
}

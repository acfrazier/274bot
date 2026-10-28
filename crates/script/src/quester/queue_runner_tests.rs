use super::*;
use crate::quester::families::tests::with_tick_output;
use crate::quester::queue::{Queue, QueueSettings, ReleaseIndex};
use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView, VarpView};
use std::sync::LazyLock;

#[derive(Default)]
struct Capture(Vec<ScriptStatus>, Vec<String>);
impl NativeOutput for Capture {
    fn status(&mut self, status: ScriptStatus) {
        self.0.push(status);
    }
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.1.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

fn fixture(settings: QueueSettings) -> (QueuedQuester, GameSnapshot) {
    static INDEX: LazyLock<ReleaseIndex> =
        LazyLock::new(|| serde_json::from_str(crate::quester::compile::INDEX_JSON).unwrap());
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    let run = RunKey {
        slot: 1,
        run: 1,
        session: 1,
    };
    let queue = Queue::from_index(&INDEX, settings).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![], 28);
    (
        QueuedQuester::new(
            run,
            selected,
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
            queue,
        ),
        snapshot,
    )
}

fn cook_settings() -> QueueSettings {
    QueueSettings {
        quests: vec!["cook".into()],
        ..QueueSettings::default()
    }
}

fn runemysteries_settings() -> QueueSettings {
    QueueSettings {
        quests: vec!["runemysteries".into()],
        ..QueueSettings::default()
    }
}

#[test]
fn missing_login_quest_observation_waits_without_gameplay_then_admits() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    let mut ledger = None;
    let mut output = Capture::default();
    for tick in 1..=3 {
        let flow = with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
            queued.tick(native).unwrap()
        });
        assert_eq!(flow, ScriptFlow::Continue);
        assert_eq!(output.0.last().unwrap().phase, NativePhase::Waiting);
        assert!(queued.preparing.is_none());
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    with_tick_output(&snapshot, &mut ledger, 4, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Preparing);
    // Join the real off-pump compiler rather than make timing assertions about it.
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    with_tick_output(&snapshot, &mut ledger, 5, &mut output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_some());
    assert_eq!(
        queued.queue.rows()[0].status,
        crate::quester::queue::QueueStatus::Running
    );
}

#[test]
fn missing_login_quest_observation_times_out_with_recovery_instructions() {
    let (mut queued, snapshot) = fixture(cook_settings());
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    let flow = with_tick_output(&snapshot, &mut ledger, 52, &mut output, |native| {
        queued.tick(native).unwrap()
    });
    let ScriptFlow::Blocked(failure) = flow else {
        panic!("an unobserved quest list must reach a bounded, visible stop");
    };
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Blocked);
    assert!(failure.message.contains("quest"));
    assert!(failure.message.contains("log in"));
    assert!(failure.message.contains("Stop"));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}
#[test]
fn individually_missing_quest_row_stays_fail_closed() {
    let (mut queued, mut snapshot) = fixture(runemysteries_settings());
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.activate(native, path);
        assert!(matches!(
            queued.tick(native).unwrap(),
            ScriptFlow::Blocked(_)
        ));
    });
    assert!(queued.active.is_none());
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Blocked);
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn runemysteries_queue_uses_selected_identity_status_row() {
    let (mut queued, mut snapshot) = fixture(runemysteries_settings());
    snapshot.seed_varps(vec![VarpView {
        index: 101,
        value: 1,
    }]);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Rune Mysteries Quest".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Preparing);
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    assert_eq!(path.id.0.as_ref(), "runemysteries");
    assert_eq!(path.display_name.as_ref(), "Rune Mysteries");
    assert_eq!(
        queued
            .quests
            .quest("runemysteries")
            .unwrap()
            .display
            .as_ref(),
        "Rune Mysteries Quest"
    );
    let index = queued.active_index.expect("preparing Rune Mysteries row");
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_some());
    assert_eq!(
        queued.queue.status(index),
        Some(crate::quester::queue::QueueStatus::Running)
    );
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn completed_canonical_quest_row_wins_before_missing_requirements() {
    let (mut queued, mut snapshot) = fixture(runemysteries_settings());
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Rune Mysteries Quest".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    let index = queued.active_index.expect("preparing Rune Mysteries row");
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_none());
    assert_eq!(
        queued.queue.status(index),
        Some(crate::quester::queue::QueueStatus::Done)
    );
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn excluding_the_entire_default_queue_is_an_actionable_block() {
    let index: ReleaseIndex = serde_json::from_str(crate::quester::compile::INDEX_JSON).unwrap();
    let (mut queued, snapshot) = fixture(QueueSettings {
        skip: index.paths.into_iter().map(|entry| entry.id).collect(),
        ..QueueSettings::default()
    });
    let mut ledger = None;
    let mut output = Capture::default();
    let flow = with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap()
    });
    let ScriptFlow::Blocked(failure) = flow else {
        panic!("an excluded queue must not silently complete");
    };
    assert!(failure.message.contains("Script prefs"));
    assert!(failure.message.contains("Skip"));
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Blocked);
}

#[test]
fn a_parked_run_is_not_sent_to_script_prefs_but_a_requirement_block_is() {
    let (mut queued, _) = fixture(cook_settings());
    queued.queue.mark_parked(
        0,
        Arc::from("no route: blocked by danger zones: White Wolf Mountain"),
    );
    let parked = queued.blocked().message;
    assert!(parked.starts_with("no route: blocked by danger zones: White Wolf Mountain; "));
    assert!(parked.contains("Stop/Start Quester"));
    assert!(
        !parked.contains("Script prefs"),
        "a refused walk can't be fixed in Script prefs: {parked}"
    );

    queued
        .queue
        .mark_blocked(0, Arc::from("need 20 quest points"));
    let blocked = queued.blocked().message;
    assert!(blocked.starts_with("need 20 quest points; "));
    assert!(blocked.contains("Script prefs"));
}
#[test]
fn third_queued_death_stops_the_slot_and_retains_blocked_status() {
    let (mut queued, mut snapshot) = fixture(QueueSettings {
        quests: vec!["cook".into(), "sheep".into()],
        order_override: vec!["cook".into(), "sheep".into()],
        ..QueueSettings::default()
    });
    let selected = Arc::clone(&queued.selected);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    snapshot.seed_chat_lines(vec![]);

    queued.deaths = 1;
    let active_index = queued
        .queue
        .next_candidate()
        .expect("selected Cook's Assistant");
    assert_eq!(queued.queue.id(active_index), Some("cook"));
    queued.active_index = Some(active_index);
    let path = super::super::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&queued.quests);
        move |cap| {
            super::super::compile::compile_path(
                super::super::registry::bundled_path("cook").unwrap(),
                &selected,
                &quests,
                cap,
                &crate::local_state::OperatorLoadouts::absent(),
            )
        }
    })
    .unwrap();
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
        queued.activate(tick, path);
    });
    assert_eq!(
        queued.active.as_ref().unwrap().prior_deaths,
        1,
        "the queued session's prior death count must carry into the active Path"
    );

    let mut slot = crate::SlotScript::new();
    slot.bind_incarnation(1);
    slot.start_test_script(Box::new(queued), Some(selected))
        .unwrap();
    let tick = |slot: &mut crate::SlotScript, snapshot: &GameSnapshot, tick| {
        slot.on_game_tick(&mut crate::ScriptCtx {
            driver: &mut crate::ctx::test_support::NullDriver::default(),
            tick,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: crate::CompiledTick::default(),
        });
    };

    tick(&mut slot, &snapshot, 1);
    assert_eq!(slot.state(), crate::RunState::Running);
    for (sequence, tick_id) in [(1, 2), (2, 3)] {
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".into(),
            sequence,
        }]);
        tick(&mut slot, &snapshot, tick_id);
        if sequence == 1 {
            assert_eq!(
                slot.state(),
                crate::RunState::Running,
                "prior one plus active one still permits two deaths"
            );
        }
    }

    assert_eq!(slot.state(), crate::RunState::Idle);
    assert!(!slot.want_run);
    assert!(slot.native_run().is_none());
    let status = slot
        .native_status()
        .expect("the blocked Quester status must survive automatic Stop");
    assert_eq!(status.phase, NativePhase::Blocked);
    let failure = status.failure.as_ref().expect("terminal failure retained");
    assert_eq!(failure.code.as_ref(), "max-deaths");
    assert_eq!(
        failure.message.as_ref(),
        "maximum deaths exceeded; Stop/Start required"
    );
    let reported_deaths = status
        .fields
        .iter()
        .any(|field| field.key == "deaths" && matches!(&field.value, StatusValue::Integer(3)));
    assert!(reported_deaths);
}

#[test]
fn cached_path_does_not_share_account_choices_between_activations() {
    use crate::quester::choices::{CrestGauntlets, QuestChoices};
    let (mut cooking, mut snapshot) = fixture(cook_settings());
    let (mut goldsmith, _) = fixture(cook_settings());
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    cooking.set_choices(QuestChoices {
        crest_gauntlets: CrestGauntlets::Cooking,
    });
    goldsmith.set_choices(QuestChoices {
        crest_gauntlets: CrestGauntlets::Goldsmith,
    });
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        cooking.tick(native).unwrap();
    });
    let path = cooking.preparing.take().unwrap().join().unwrap().unwrap();
    // The second account receives the very same compiled document.
    goldsmith.active_index = Some(0);
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        cooking.activate(native, Arc::clone(&path));
        goldsmith.activate(native, Arc::clone(&path));
    });
    let a = cooking.active.as_ref().unwrap();
    let b = goldsmith.active.as_ref().unwrap();
    assert!(Arc::ptr_eq(&a.path, &b.path));
    assert_eq!(a.choices.crest_gauntlets, CrestGauntlets::Cooking);
    assert_eq!(b.choices.crest_gauntlets, CrestGauntlets::Goldsmith);
}

#[test]
fn folder_override_and_draft_activate_from_the_queue_and_publish_their_source() {
    use crate::quester::registry::{FolderSource, PathRegistry};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    for (id, source) in [("cook", "folder"), ("cook-draft", "draft")] {
        let folder = std::env::temp_dir().join(format!(
            "quester-queue-folder-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let mut document: serde_json::Value =
            serde_json::from_slice(crate::quester::compile::cook_bytes()).unwrap();
        document["id"] = serde_json::json!(id);
        document["roles"][0]["sequences"][0]["steps"][0]["comment"] =
            serde_json::json!("folder queue activation witness");
        let bytes = serde_json::to_vec(&document).unwrap();
        let digest =
            crate::quester::registry::digest_text(&crate::quester::compile::digest_bytes(&bytes));
        std::fs::write(folder.join(format!("{id}.json")), bytes).unwrap();
        let (mut queued, mut snapshot) = fixture(cook_settings());
        let selected = Arc::clone(&queued.selected);
        let quests = Arc::clone(&queued.quests);
        let source_folder = folder.clone();
        let registry = api::selected::FamilyPreparation::run(move |worker| {
            PathRegistry::load(
                &FolderSource {
                    enabled: true,
                    folder: source_folder,
                },
                &selected,
                &quests,
                worker,
            )
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
        queued.queue = Queue::from_registry(
            registry,
            QueueSettings {
                quests: vec![id.into()],
                ..Default::default()
            },
        )
        .unwrap();
        queued.refresh_fields();
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        let mut ledger = None;
        let mut output = Capture::default();
        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
            queued.tick(tick).unwrap();
        });
        let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
        assert_eq!(path.id.0.as_ref(), id);
        assert_eq!(
            path.sequences[0].steps[0].comment.as_deref(),
            Some("folder queue activation witness")
        );
        with_tick_output(&snapshot, &mut ledger, 2, &mut output, |tick| {
            queued.activate(tick, path);
            queued.tick(tick).unwrap();
        });
        assert!(queued.active.is_some());
        assert!(queued.fields.iter().any(|field| field.key == "path_source"
            && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == source)));
        assert!(output.1.iter().any(|line| {
            line.contains(&format!("source={source}"))
                && line.contains(&digest)
                && line.contains("step_comment=\"folder queue activation witness\"")
        }));
        // The run retains its loaded bytes even after the folder is removed.
        std::fs::remove_dir_all(&folder).unwrap();
        let bytes = queued
            .queue
            .path_bytes(queued.active_index.unwrap())
            .unwrap();
        assert!(bytes
            .as_ref()
            .windows(b"folder queue activation witness".len())
            .any(|window| window == b"folder queue activation witness"));
    }
}

/// T5.2 r2: a loadouts store the service refused (an unreadable or
/// lossy file) does not repair itself on a tick. The Path is blocked with
/// the typed reason, instead of waiting, and logging, on every tick.
#[test]
fn a_refused_loadouts_store_blocks_the_path_instead_of_waiting() {
    use api::local_state::{
        AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, ErrorCode,
        FailureReason, LocalStatePort, StoreVersion, SubmitReceipt, WriteSettlement,
    };
    struct Refusing;
    impl LocalStatePort for Refusing {
        fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
            Err(ErrorCode::Failed {
                domain: identity.domain(),
                reason: FailureReason::Schema,
            })
        }
        fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
            SubmitReceipt::Refused(ErrorCode::Busy {
                domain: edit.identity().domain(),
            })
        }
        fn observe(&self, _: api::identity::RequestId) -> Option<WriteSettlement> {
            None
        }
    }
    impl crate::local_state::StatePort for Refusing {
        fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
            self.lease(identity).map(|lease| lease.version())
        }
    }
    // The tick builder quantifies over its own lifetime: the capability it
    // borrows must outlive any of them, so it is a static.
    static STATE: LazyLock<crate::local_state::SlotState> = LazyLock::new(|| {
        crate::local_state::SlotState::new(
            Arc::new(Refusing),
            AccountKey::try_new("local-289", "alice").unwrap(),
            ContentScope::try_new(api::selected::ClientRevision::R289, "test-content").unwrap(),
        )
    });
    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        native.state = Some(&*STATE);
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert!(queued.preparing.is_none(), "nothing compiles without rows");
    assert_eq!(
        queued.queue.rows()[0].status,
        crate::quester::queue::QueueStatus::Blocked
    );
    let reason = queued.queue.reason(0).unwrap();
    assert!(reason.contains("operator loadouts"), "{reason}");
    assert!(
        !output
            .1
            .iter()
            .any(|line| line.contains("waits for operator loadouts")),
        "a refused store is not a cold one"
    );
}

fn skip_fixture(two_quests: bool) -> (QueuedQuester, GameSnapshot) {
    let (queued, mut snapshot) = fixture(QueueSettings {
        quests: if two_quests {
            vec!["cook".into(), "sheep".into()]
        } else {
            vec!["cook".into()]
        },
        order_override: if two_quests {
            vec!["cook".into(), "sheep".into()]
        } else {
            Vec::new()
        },
        ..QueueSettings::default()
    });
    snapshot.seed_quest_statuses(
        vec![
            QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            },
            QuestStatusView {
                name: "Sheep Shearer".into(),
                component_id: 43,
                colour: 0xf80000,
            },
        ],
        true,
    );
    (queued, snapshot)
}

fn activate_preparing(
    queued: &mut QueuedQuester,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    output: &mut Capture,
    tick: u64,
) {
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    with_tick_output(snapshot, ledger, tick, output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_some());
}

fn select_terminal_sequence(active: &mut Quester) {
    active.seq_index = active
        .path
        .sequences
        .iter()
        .position(|sequence| sequence.terminal && sequence.steps.is_empty())
        .expect("fixture has a terminal sequence");
    active.in_prelude = false;
    active.step_index = 0;
    active.cursor = 0;
    active.needs_read = false;
    active.settling = false;
    active.step = None;
}
struct SkipCancellation(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl StepRun for SkipCancellation {
    fn poll(&mut self, _: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        panic!("a skipped step must not be polled again");
    }

    fn cancel(&mut self, _: &mut crate::native::NativeActions) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
fn operator_skip_cancels_active_step_and_activates_next_quest_on_later_tick() {
    let (mut queued, snapshot) = skip_fixture(true);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    let cancelled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active = queued.active.as_mut().unwrap();
    active.step = Some(Box::new(SkipCancellation(Arc::clone(&cancelled))));
    let step_id = Arc::clone(&active.current_step().unwrap().id.0);
    let accepted = queued.skip_quest().unwrap();
    assert_eq!(accepted.quest_id.as_ref(), "cook");
    assert_eq!(accepted.step_id, step_id);
    assert_eq!(
        queued.skip_quest().unwrap_err().message.as_ref(),
        "skip already requested"
    );
    with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(cancelled.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(queued.active.is_none());
    assert!(
        queued.preparing.is_none(),
        "the next quest must wait for a later tick"
    );
    assert_eq!(queued.queue.status(0), Some(QueueStatus::Parked));
    assert_eq!(queued.queue.reason(0), Some("skipped by operator"));
    assert!(output.0.last().unwrap().failure.is_none());
    with_tick_output(&snapshot, &mut ledger, 4, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 5);
    assert_eq!(queued.active.as_ref().unwrap().path.id.0.as_ref(), "sheep");
    assert_eq!(queued.queue.reason(0), Some("skipped by operator"));

    let sheep = queued.active_index.unwrap();
    queued.active = None;
    queued.queue.mark_done(sheep);
    queued.queue.refresh_blocked();
    with_tick_output(&snapshot, &mut ledger, 6, &mut output, |native| {
        assert!(matches!(
            queued.tick(native).unwrap(),
            ScriptFlow::Blocked(_)
        ));
    });
    assert_eq!(queued.queue.next_candidate(), None);
    assert_eq!(queued.queue.status(0), Some(QueueStatus::Parked));
}

#[test]
fn operator_skip_last_row_keeps_existing_blocked_end_rule() {
    let (mut queued, snapshot) = skip_fixture(false);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    queued.skip_quest().unwrap();
    with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    let flow = with_tick_output(&snapshot, &mut ledger, 4, &mut output, |native| {
        queued.tick(native).unwrap()
    });
    let ScriptFlow::Blocked(failure) = flow else {
        panic!("a skipped last row is parked, not complete");
    };
    assert!(failure.message.starts_with("skipped by operator;"));
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Blocked);
    assert_eq!(queued.queue.next_candidate(), None);
}

#[test]
fn operator_skip_wins_on_the_tick_a_quest_would_complete() {
    let (mut natural, natural_snapshot) = skip_fixture(false);
    let mut natural_ledger = None;
    let mut natural_output = Capture::default();
    with_tick_output(
        &natural_snapshot,
        &mut natural_ledger,
        1,
        &mut natural_output,
        |native| natural.tick(native).unwrap(),
    );
    activate_preparing(
        &mut natural,
        &natural_snapshot,
        &mut natural_ledger,
        &mut natural_output,
        2,
    );
    select_terminal_sequence(natural.active.as_mut().unwrap());
    with_tick_output(
        &natural_snapshot,
        &mut natural_ledger,
        3,
        &mut natural_output,
        |native| natural.tick(native).unwrap(),
    );
    assert_eq!(natural.queue.status(0), Some(QueueStatus::Done));
    assert_eq!(natural.completed, 1);

    let (mut queued, snapshot) = skip_fixture(false);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    select_terminal_sequence(queued.active.as_mut().unwrap());
    queued.skip_quest().unwrap();
    with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });

    assert_eq!(queued.queue.status(0), Some(QueueStatus::Parked));
    assert_eq!(queued.queue.reason(0), Some("skipped by operator"));
    assert_eq!(queued.completed, 0);
    assert!(queued.active.is_none());
    assert!(queued.active_index.is_none());
    assert!(queued.preparing.is_none());
}

#[test]
fn manual_movement_stop_during_skip_tick_wins_over_skip() {
    let (mut queued, snapshot) = skip_fixture(true);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    queued.skip_quest().unwrap();
    let active = queued.active.as_mut().unwrap();
    active.parked = true;
    active.last_error_kind = QuesterFailureKind::ManualMovement;
    active.last_error = Some(Arc::from("cancelled by user input"));

    let flow = with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        queued.tick(native).unwrap()
    });
    let ScriptFlow::Blocked(failure) = flow else {
        panic!("manual movement must stop the run");
    };
    assert_eq!(failure.code.as_ref(), "manual-movement");
    assert_eq!(failure.message.as_ref(), "cancelled by user input");
    assert!(!queued.skip_requested);
    assert_eq!(
        queued.active.as_ref().unwrap().path.id.0.as_ref(),
        "cook",
        "the skip must not advance to Sheep"
    );
    assert_eq!(queued.active_index, Some(0));
    assert!(queued.preparing.is_none());
}

#[test]
fn max_deaths_during_skip_tick_stops_the_run_before_the_next_quest() {
    let (mut queued, mut snapshot) = skip_fixture(true);
    queued.deaths = 2;
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    queued.skip_quest().unwrap();
    snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence: 1,
    }]);

    let flow = with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        queued.tick(native).unwrap()
    });
    let ScriptFlow::Blocked(failure) = flow else {
        panic!("max-deaths must stop the run");
    };
    assert_eq!(failure.code.as_ref(), "max-deaths");
    assert_eq!(
        failure.message.as_ref(),
        "maximum deaths exceeded; Stop/Start required"
    );
    assert!(!queued.skip_requested);
    assert_eq!(
        queued.active.as_ref().unwrap().path.id.0.as_ref(),
        "cook",
        "the skip must not advance to Sheep"
    );
    assert_eq!(queued.active_index, Some(0));
    assert!(queued.preparing.is_none());
}

#[test]
fn operator_skip_refuses_an_already_parked_quest() {
    let (mut queued, snapshot) = skip_fixture(false);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    queued.active.as_mut().unwrap().parked = true;
    assert_eq!(
        queued.skip_quest().unwrap_err().message.as_ref(),
        "active quest is already parked"
    );
    assert!(!queued.skip_requested);
}
#[test]
fn operator_skip_refuses_before_activation_and_during_preparation() {
    let (mut queued, snapshot) = skip_fixture(false);
    assert_eq!(
        queued.skip_quest().unwrap_err().message.as_ref(),
        "no quest is active"
    );
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    assert!(queued.preparing.is_some());
    assert_eq!(
        queued.skip_quest().unwrap_err().message.as_ref(),
        "quest is still preparing; no quest is active"
    );
    assert!(!queued.skip_requested);
    let _ = queued.preparing.take().unwrap().join();
}

#[test]
fn operator_skip_waits_for_owned_prayer_cleanup_before_next_quest() {
    let (mut queued, mut snapshot) = skip_fixture(true);
    let (protect_varp, protect_button) = {
        let protect = queued
            .selected
            .prayer_by_name("Protect from Melee")
            .unwrap();
        (protect.varp, protect.button_com)
    };
    let varps = |raised| {
        queued
            .selected
            .prayers()
            .iter()
            .map(|prayer| VarpView {
                index: prayer.varp,
                value: i32::from(raised && prayer.varp == protect_varp),
            })
            .collect()
    };
    let raised = varps(true);
    let lowered = varps(false);
    snapshot.seed_varps(raised);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    queued
        .active
        .as_mut()
        .unwrap()
        .prayer_cleanup_owned
        .accepted(protect_varp, true, 0);
    queued.skip_quest().unwrap();
    let mut cleanup = None;
    for tick in 3..=12 {
        with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
            assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
        });
        assert!(
            queued.active.is_some(),
            "cleanup must retain the old executor"
        );
        assert!(queued.preparing.is_none());
        if let Some(action) = ledger.as_mut().and_then(|ledger| ledger.outbox.pop()) {
            cleanup = Some((tick, action));
            break;
        }
    }
    let (tick, action) = cleanup.expect("skip must issue the owned prayer cleanup");
    assert!(matches!(
        &action.effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id })
            if *component_id == protect_button
    ));
    let authority = action.authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    snapshot.seed_varps(lowered);
    for next in tick + 1..=tick + 8 {
        with_tick_output(&snapshot, &mut ledger, next, &mut output, |native| {
            assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
        });
        if queued.active.is_none() {
            break;
        }
    }
    assert!(queued.active.is_none());
    assert!(queued.preparing.is_none());
    assert_eq!(queued.queue.reason(0), Some("skipped by operator"));
}

#[test]
fn operator_skip_revokes_active_step_during_host_hold() {
    let (mut queued, snapshot) = skip_fixture(false);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    activate_preparing(&mut queued, &snapshot, &mut ledger, &mut output, 2);
    let cancelled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    queued.active.as_mut().unwrap().step = Some(Box::new(SkipCancellation(Arc::clone(&cancelled))));
    queued.skip_quest().unwrap();
    with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        native.cx.eligible = false;
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(cancelled.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(queued.active.as_ref().unwrap().step.is_none());
    assert!(queued.preparing.is_none());
    with_tick_output(&snapshot, &mut ledger, 4, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert!(queued.active.is_none());
    assert_eq!(queued.queue.reason(0), Some("skipped by operator"));
}

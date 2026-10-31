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
    assert!(failure.message.contains("start the Quester again"));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}
#[test]
fn a_parked_queue_row_publishes_its_stage_step_and_failure() {
    use crate::quester::path::{PredicateDocument, StepDocument};

    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    // One root step whose settle can never hold: the run retries it until the
    // failure streak parks (the runner's settle-timeout policy).
    let mut document = crate::quester::compile::decode_cook().unwrap();
    document.quest.as_mut().unwrap().owns_inventory = true;
    document.roles[0].sequences[0].steps = vec![StepDocument {
        id: api::selected::FactKey::new("queue-park-step"),
        kind: "wait".into(),
        version: 1,
        args: serde_json::json!({"until":{"All":[]},"max_ticks":100}),
        comment: None,
        advances: Some(false),
        max_attempts: None,
        stop: None,
        skip_if: PredicateDocument::Any(vec![]),
        settle: PredicateDocument::Any(vec![]),
    }];
    let path = crate::quester::compile::compile_uncached_for_test(
        &document,
        &queued.selected,
        &queued.quests,
    )
    .unwrap();
    let expected_stage = Arc::clone(&path.colour_not_started.0);
    let index = queued.queue.next_candidate().expect("selected cook row");
    assert_eq!(queued.queue.id(index), Some("cook"));
    queued.active_index = Some(index);

    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_some());
    let mut parked = false;
    for tick in 2..=200 {
        with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
            queued.tick(native).unwrap();
        });
        if queued.queue.status(index) == Some(crate::quester::queue::QueueStatus::Parked) {
            parked = true;
            break;
        }
    }
    assert!(parked, "the never-settling step must park the queued run");
    assert!(
        queued.active.is_none(),
        "the parked executor is dropped before the Blocked status is built"
    );

    let status = output.0.last().expect("blocked status was published");
    assert_eq!(status.phase, NativePhase::Blocked);
    let field = |key: &str| {
        status
            .fields
            .iter()
            .find(|field| field.key == key)
            .and_then(|field| match &field.value {
                StatusValue::Text(value) => Some(value.as_ref()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("status field {key} is missing or not text"))
    };
    assert_eq!(field("quest_id"), "cook");
    assert_eq!(field("stage"), expected_stage.as_ref());
    assert_eq!(field("step_id"), "queue-park-step");
    assert_eq!(field("last_failure"), "step settle timeout");
    assert_eq!(field("action_state"), "blocked");
    let failure = status.failure.as_ref().expect("terminal failure retained");
    assert!(
        failure.message.contains("step settle timeout")
            && failure.message.contains("Stop/Start Quester"),
        "{failure:?}"
    );
}

#[test]
fn a_queued_no_step_park_publishes_and_reuses_the_empty_step_id() {
    use crate::quester::path::PredicateDocument;

    let mut previous_empty_step = None;
    for _ in 0..2 {
        let (mut queued, mut snapshot) = fixture(cook_settings());
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        let mut document = crate::quester::compile::decode_cook().unwrap();
        document.quest.as_mut().unwrap().owns_inventory = true;
        for role in &mut document.roles {
            for sequence in &mut role.sequences {
                for step in &mut sequence.steps {
                    step.skip_if = PredicateDocument::All(Vec::new());
                }
            }
        }
        let path = crate::quester::compile::compile_uncached_for_test(
            &document,
            &queued.selected,
            &queued.quests,
        )
        .unwrap();
        let index = queued.queue.next_candidate().unwrap();
        queued.active_index = Some(index);
        let mut ledger = None;
        let mut output = Capture::default();
        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
            queued.activate(native, path);
        });
        for tick in 2..=10 {
            with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
                queued.tick(native).unwrap();
            });
            if queued.active.is_none() {
                break;
            }
        }
        let parked = queued.queue.parked(index).expect("the no-step run must park");
        assert_eq!(parked.step.as_ref(), "");
        if let Some(previous) = &previous_empty_step {
            assert!(Arc::ptr_eq(previous, &parked.step));
        }
        previous_empty_step = Some(Arc::clone(&parked.step));
        let status = output.0.last().expect("the queued Blocked status is published");
        assert_eq!(status.phase, NativePhase::Blocked);
        assert!(status.fields.iter().any(|field| {
            field.key == "step_id"
                && matches!(&field.value, StatusValue::Text(step) if step.is_empty())
        }));
        assert!(status.failure.as_ref().unwrap().message.contains("no step for stage"));
    }
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
    let status = output.0.last().unwrap();
    assert_eq!(status.phase, NativePhase::Blocked);
    // The row was blocked by requirements, so no step ran and the queue has no
    // parked position to publish.
    for key in [
        "stage",
        "quest_id",
        "step_id",
        "last_failure",
        "action_state",
    ] {
        assert!(
            !status.fields.iter().any(|field| field.key == key),
            "a queue row that never ran publishes no {key}"
        );
    }
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
        crate::quester::queue::ParkedRun {
            quest: Arc::from("cook"),
            stage: Some(Arc::from("not_started")),
            step: Arc::from("walk-draynor-bank"),
            failure: Arc::from("no route: blocked by danger zones: White Wolf Mountain"),
        },
    );
    let parked = queued.blocked().message;
    assert!(parked.starts_with("no route: blocked by danger zones: White Wolf Mountain; "));
    assert!(parked.contains("start the Quester again"));
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
                None,
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
                None,
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

fn cook_status(colour: i32) -> Vec<QuestStatusView> {
    vec![QuestStatusView {
        name: "Cook's Assistant".into(),
        component_id: 42,
        colour,
    }]
}

/// The kinds of effect the host was asked for, in order.
fn effects(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> Vec<&'static str> {
    ledger.as_ref().map_or_else(Vec::new, |ledger| {
        ledger
            .outbox
            .iter()
            .map(|action| match &action.effect {
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Close) => "close",
                crate::native::HostEffect::Interaction(_) => "interaction",
                _ => "other",
            })
            .collect()
    })
}

/// The host accepts the interaction at the end of the outbox.
fn accept_last_interaction(ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick: u64) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    ledger.as_mut().unwrap().outbox.clear();
}

fn open_bank(snapshot: &mut GameSnapshot, tick: u64) -> BankLease {
    snapshot.seed_bank_observation(1, tick, Some(Vec::new()), Vec::new());
    // The test frames carry login session 1.
    BankLease::new(1, snapshot.bank_session_generation())
}

#[test]
fn a_restarted_run_closes_the_open_bank_before_it_reads_or_plans_anything() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(cook_status(0xf80000), true);
    let lease = open_bank(&mut snapshot, 1);
    // The retained cell of the replaced instance carried the lease.
    queued.restore(&super::super::QuesterRetained {
        bank_lease: Some(lease),
        ..Default::default()
    });
    let mut ledger = None;
    let mut output = Capture::default();
    let flow = with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        let flow = queued.tick(native).unwrap();
        assert_eq!(
            native.cx.retained().quester().bank_lease,
            Some(lease),
            "the lease stays retained until the bank is shut"
        );
        flow
    });
    assert_eq!(flow, ScriptFlow::Continue);
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Waiting);
    assert_eq!(
        effects(&ledger),
        ["close"],
        "the close is the first and only input"
    );
    assert!(
        queued.preparing.is_none(),
        "no Path compile starts under the open bank"
    );
    accept_last_interaction(&mut ledger, 1);
    snapshot.seed_bank_observation(-1, 2, None, Vec::new());
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.tick(native).unwrap();
        assert_eq!(native.cx.retained().quester().bank_lease, None);
    });
    assert!(queued.owed_bank.is_none());
    assert!(
        effects(&ledger).iter().all(|effect| *effect != "close"),
        "one close only"
    );
}

#[test]
fn resume_closes_the_bank_the_run_left_open_before_the_journal_is_read() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(cook_status(0xf80000), true);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.activate(native, path);
    });
    assert!(queued.active.is_some());
    // Provisioning opened the bank, then the run was paused.
    let lease = open_bank(&mut snapshot, 3);
    queued
        .active
        .as_mut()
        .unwrap()
        .provisioner
        .set_lease_for_test(lease);
    queued.interrupt(Interrupt::Resume);
    assert_eq!(
        queued.owed_bank,
        Some(lease),
        "resume hands the bank to the closer"
    );
    with_tick_output(&snapshot, &mut ledger, 3, &mut output, |native| {
        assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(
        effects(&ledger),
        ["close"],
        "the close comes before the journal read that resume owes"
    );
    assert!(!queued.active.as_ref().unwrap().journal_opened());
}

#[test]
fn a_run_started_after_stop_leaves_an_open_bank_alone() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    open_bank(&mut snapshot, 1);
    // Stop discards the retained cell, so the next run starts without a lease.
    queued.restore(&super::super::QuesterRetained::default());
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    assert_eq!(effects(&ledger), Vec::<&str>::new());
    assert!(queued.owed_bank.is_none());
}

#[test]
fn operator_stop_hands_nothing_to_the_closer() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(cook_status(0xf80000), true);
    let mut ledger = None;
    let mut output = Capture::default();
    with_tick_output(&snapshot, &mut ledger, 1, &mut output, |native| {
        queued.tick(native).unwrap();
    });
    let path = queued.preparing.take().unwrap().join().unwrap().unwrap();
    with_tick_output(&snapshot, &mut ledger, 2, &mut output, |native| {
        queued.activate(native, path);
    });
    let lease = open_bank(&mut snapshot, 3);
    queued
        .active
        .as_mut()
        .unwrap()
        .provisioner
        .set_lease_for_test(lease);
    queued.on_stop(StopReason::Operator);
    assert!(
        queued.owed_bank.is_none(),
        "Stop does not reclaim the lease, so no tick can close it"
    );
    assert_eq!(effects(&ledger), Vec::<&str>::new());
}

#[test]
fn a_held_tick_sends_no_close() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    let lease = open_bank(&mut snapshot, 1);
    queued.restore(&super::super::QuesterRetained {
        bank_lease: Some(lease),
        ..Default::default()
    });
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let pin = data.selected_pin().unwrap();
    let evidence = EvidenceStamp {
        run: queued.run,
        tick: 1,
        sequence: 1,
    };
    let compiled = crate::CompiledTick {
        condition: None,
        poison_unknown_pulse: None,
        cure_poison: crate::survival::CurePoison::WhenNeeded,
        walk_danger: Some(&crate::native::AllowWalkDanger),
        selected: Some(&data),
        reach: None,
        reach_flood: None,
        bank_memory: None,
        collision: None,
        // The host holds the account: a user takeover or a survival hold.
        hold: true,
        world_members: Truth::Unknown,
        interacts: Some(Vec::new()),
        service_gate: None,
    };
    let frame = crate::native::HostFrame::for_test(None, Some(&snapshot), None, compiled);
    let mut ledger = None;
    let mut output = Capture::default();
    let flow = crate::native::with_test_tick(
        frame,
        evidence,
        &pin,
        (Duration::from_millis(600), std::time::Instant::now()),
        &mut ledger,
        &mut output,
        |native| queued.tick(native).unwrap(),
    );
    assert_eq!(flow, ScriptFlow::Continue);
    assert_eq!(effects(&ledger), Vec::<&str>::new());
    assert_eq!(
        queued.owed_bank,
        Some(lease),
        "the obligation waits for the hold to end"
    );
}

#[test]
fn a_close_the_bank_never_honours_is_bounded_and_then_dropped() {
    let (mut queued, mut snapshot) = fixture(cook_settings());
    let lease = open_bank(&mut snapshot, 1);
    queued.restore(&super::super::QuesterRetained {
        bank_lease: Some(lease),
        ..Default::default()
    });
    let mut ledger = None;
    let mut output = Capture::default();
    let mut sent = 0;
    for tick in 1..=400 {
        with_tick_output(&snapshot, &mut ledger, tick, &mut output, |native| {
            queued.tick(native).unwrap();
        });
        if effects(&ledger) == ["close"] {
            sent += 1;
            // The click is accepted but the interface never goes away.
            accept_last_interaction(&mut ledger, tick);
        }
        if queued.owed_bank.is_none() && !queued.owed_closer.active() {
            break;
        }
    }
    assert_eq!(sent, crate::acquire::run::MAX_CLOSE_ATTEMPTS);
    assert!(
        queued.owed_bank.is_none(),
        "the obligation is dropped, not retried forever"
    );
    assert!(
        output
            .1
            .iter()
            .any(|line| line.contains("the open bank was not closed")),
        "{:?}",
        output.1
    );
}

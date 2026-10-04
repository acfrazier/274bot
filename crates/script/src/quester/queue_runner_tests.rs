use super::*;
use crate::quester::families::tests::with_tick_output;
use crate::quester::queue::{Queue, QueueSettings, ReleaseIndex};
use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView};
use std::sync::LazyLock;

#[derive(Default)]
struct Capture(Vec<ScriptStatus>);
impl NativeOutput for Capture {
    fn status(&mut self, status: ScriptStatus) {
        self.0.push(status);
    }
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
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
    let (mut queued, mut snapshot) = fixture(cook_settings());
    snapshot.seed_quest_statuses(vec![], true);
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
    assert_eq!(output.0.last().unwrap().phase, NativePhase::Blocked);
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn excluding_the_entire_default_queue_is_an_actionable_block() {
    let (mut queued, snapshot) = fixture(QueueSettings {
        skip: ["cook", "sheep", "runemysteries", "romeojuliet", "imp"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
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
    let path = super::super::compile::compile_path(
        super::super::card::released_path("cook").unwrap(),
        &selected,
        &queued.quests,
    )
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

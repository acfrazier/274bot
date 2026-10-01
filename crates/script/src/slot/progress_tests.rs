use super::*;
use crate::ctx::test_support::NullDriver;
use crate::native::{HostEffect, InteractionReceipt};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FactKey, Truth};
use api::snapshot::{GameSnapshot, QuestStatusView};

fn load() -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(400);
    slot.start_load_with_settings_and_game_data(
        "export function tick() {}".into(),
        crate::load::LoadShape::NativeTick,
        None,
        vec![],
        Some(api::game_data::for_revision(ClientRevision::R289).unwrap()),
        Arc::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(StartOutcome::Ready) => break,
            StartPoll::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            _ => panic!("Load failed"),
        }
    }
    slot.on_is_up(true);
    slot
}
fn snapshot(colour: i32) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour,
        }],
        true,
    );
    snapshot
}
fn drive(slot: &mut SlotScript, snapshot: &GameSnapshot, tick: u64, hold: bool) {
    slot.tick_api(&mut ScriptCtx {
        driver: &mut NullDriver::default(),
        tick,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: crate::CompiledTick {
            hold,
            ..Default::default()
        },
    });
}
fn read(slot: &mut SlotScript, token: u64, quest: &str) {
    slot.consume_api_control(&InteractReq::ProgressRead {
        request_id: token,
        name: quest.into(),
    });
}
fn settle_read(slot: &mut SlotScript, snapshot: &GameSnapshot) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while slot.api_owns_foreground() {
        drive(slot, snapshot, 1, false);
        assert!(Instant::now() < deadline, "progress stalled");
        std::thread::yield_now();
    }
}
fn journal_fixture(slot: &mut SlotScript, token: u64) {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    let mut doc = crate::quester::compile::decode_cook().unwrap();
    let progress = doc.roles[0].progress.as_mut().unwrap();
    progress.rules = vec![crate::quester::path::ProgressRuleDocument {
        stage: FactKey::new("cook:1"),
        all: vec!["bring eggs".into()],
        any: vec![],
        not: vec![],
        varp: None,
    }];
    progress.flags = vec![crate::quester::path::ProgressFlagDocument {
        flag: FactKey::new("eggs"),
        all: vec!["eggs".into()],
        any: vec![],
        count: Some(r"(\d{1,9})".into()),
    }];
    let path =
        crate::quester::compile::compile_uncached_for_test(&doc, &selected, &quests).unwrap();
    let pin = selected.selected_pin().unwrap();
    let run = RunKey {
        slot: 400,
        run: token,
        session: slot.work_epoch,
    };
    let seat = slot.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
    seat.progress = ProgressSeat::Reading {
        token,
        run,
        prepared: PreparedProgress {
            selected,
            pin,
            path,
            quests,
        },
        since: None,
        journal: None,
        discarded: false,
    };
    seat.progress_page = Some(ProgressPage::Reading { token });
}
fn ack(slot: &mut SlotScript, tick: u64) -> HostEffect {
    let action = slot
        .take_native_action()
        .expect("owned native journal action");
    slot.complete_native_interaction(
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
fn show_journal(snapshot: &mut GameSnapshot, body: &str) {
    snapshot.seed_main_modal(
        8134,
        vec![
            crate::quest_journal::test_widget(8144, "@dre@The Cook's Quest"),
            crate::quest_journal::test_widget(8145, body),
        ],
    );
}
fn finish_journal(slot: &mut SlotScript, snapshot: &mut GameSnapshot, body: &str) {
    drive(slot, snapshot, 2, false);
    assert!(matches!(
        ack(slot, 2),
        HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
    ));
    show_journal(snapshot, body);
    drive(slot, snapshot, 3, false);
    drive(slot, snapshot, 4, false);
    assert!(matches!(
        ack(slot, 4),
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    drive(slot, snapshot, 5, false);
    assert!(!slot.has_native_actions());
    assert!(!slot.api_owns_foreground());
    assert!(slot
        .native_runtime
        .ledger
        .as_mut()
        .unwrap()
        .quiet_read(Instant::now())
        .is_none());
}

#[test]
fn unknown_path_refuses_without_preparation_and_duplicate_is_inert() {
    let mut slot = load();
    read(&mut slot, 1, "nope");
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { token: 1, reason }) if reason.as_ref() == "unknown-path")
    );
    assert!(matches!(
        slot.api.as_ref().unwrap().progress,
        ProgressSeat::Idle
    ));
    assert!(slot.api.as_ref().unwrap().progress_retiring.is_empty());
    assert!(slot.native_runtime.ledger.is_none());
    read(&mut slot, 1, "cook");
    assert!(!slot.api_owns_foreground());
    slot.stop();
}
#[test]
fn every_released_path_resolves_complete_colour_without_quiet_lease() {
    let mut slot = load();
    for (index, (quest, display, stage)) in [
        ("cook", "Cook's Assistant", "cook:2"),
        ("sheep", "Sheep Shearer", "sheep:2"),
        ("runemysteries", "Rune Mysteries Quest", "runemysteries:2"),
        ("romeojuliet", "Romeo & Juliet", "romeojuliet:100"),
    ]
    .into_iter()
    .enumerate()
    {
        let token = index as u64 + 2;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: display.into(),
                component_id: 42,
                colour: 0x00f800,
            }],
            true,
        );
        read(&mut slot, token, quest);
        settle_read(&mut slot, &snapshot);
        let Some(ProgressPage::Done { token: actual, row }) =
            &slot.api.as_ref().unwrap().progress_page
        else {
            panic!("missing done for {quest}");
        };
        assert_eq!(*actual, token);
        assert_eq!(row.quest.as_ref(), quest);
        assert!(matches!(&row.stage, Knowledge::Known(value) if value.as_ref() == stage));
        assert_eq!(row.complete, Truth::True);
        assert!(!row.journal_read);
        assert_eq!(row.evidence.run.slot, 400);
        assert!(
            slot.native_runtime.ledger.is_none(),
            "complete-colour reads never acquire a lease"
        );
    }
    slot.stop();
}
#[test]
fn journal_rule_reads_one_click_closes_and_resolves_stage_and_count() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    journal_fixture(&mut slot, 3);
    drive(&mut slot, &snapshot, 1, false);
    finish_journal(&mut slot, &mut snapshot, "Bring eggs: 12 eggs");
    let Some(ProgressPage::Done { row, .. }) = &slot.api.as_ref().unwrap().progress_page else {
        panic!("missing done");
    };
    assert!(row.journal_read);
    assert!(matches!(&row.stage, Knowledge::Known(stage) if stage.as_ref() == "cook:1"));
    assert!(matches!(&row.rule, Knowledge::Known(rule) if rule.as_ref() == "cook:1"));
    assert_eq!(row.flags[0].count, Some(12));
    assert_eq!(row.flags[0].truth, Truth::True);
    assert_eq!(row.evidence.tick, 5);
    slot.stop();
}
#[test]
fn journal_no_match_is_unknown_not_colour_fallback() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    journal_fixture(&mut slot, 4);
    drive(&mut slot, &snapshot, 1, false);
    finish_journal(&mut slot, &mut snapshot, "unrecognised branch");
    let Some(ProgressPage::Done { row, .. }) = &slot.api.as_ref().unwrap().progress_page else {
        panic!("missing done");
    };
    assert!(
        matches!(&row.stage, Knowledge::Unknown(gap) if gap.code.as_ref() == "journal-no-match")
    );
    assert_eq!(row.complete, Truth::Unknown);
    slot.stop();
}
#[test]
fn adopted_journal_closes_without_another_button() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    show_journal(&mut snapshot, "Bring eggs: 7 eggs");
    journal_fixture(&mut slot, 5);
    drive(&mut slot, &snapshot, 1, false);
    drive(&mut slot, &snapshot, 2, false);
    assert!(!slot.has_native_actions());
    drive(&mut slot, &snapshot, 3, false);
    assert!(matches!(
        ack(&mut slot, 3),
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut slot, &snapshot, 4, false);
    let Some(ProgressPage::Done { row, .. }) = &slot.api.as_ref().unwrap().progress_page else {
        panic!("missing adopted result");
    };
    assert_eq!(row.flags[0].count, Some(7));
    assert!(row.journal_read);
    slot.stop();
}
#[test]
fn gather_preparing_and_running_both_refuse_progress() {
    let mut slot = load();
    slot.consume_api_control(&InteractReq::GatherRun {
        request_id: 6,
        settings: Arc::default(),
    });
    read(&mut slot, 7, "cook");
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { reason, .. }) if reason.as_ref() == "busy")
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        drive(&mut slot, &snapshot(0x00f800), 1, true);
        if matches!(
            slot.api.as_ref().unwrap().gather,
            GatherSeat::Running { .. }
        ) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    read(&mut slot, 8, "cook");
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { token: 8, reason }) if reason.as_ref() == "busy")
    );
    slot.stop();
}
#[test]
fn reading_progress_refuses_gather_and_drops_only_game_rows() {
    let mut slot = load();
    journal_fixture(&mut slot, 9);
    slot.restore_interacts(vec![
        InteractReq::GatherRun {
            request_id: 10,
            settings: Arc::default(),
        },
        InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: None,
        },
        InteractReq::SetCameraYaw { yaw: 777 },
    ]);
    let (_, rows, owned) = slot.drain_host_interacts();
    assert!(owned);
    assert_eq!(rows, vec![InteractReq::SetCameraYaw { yaw: 777 }]);
    assert!(
        matches!(&slot.api.as_ref().unwrap().terminal, Some(GatherEnd::Refused { token: 10, reason }) if reason.as_ref() == "busy")
    );
    slot.stop();
}
#[test]
fn supersede_before_journal_replaces_preparing_and_reading() {
    let mut slot = load();
    read(&mut slot, 11, "cook");
    read(&mut slot, 12, "cook");
    assert_eq!(slot.api.as_ref().unwrap().progress.token(), Some(12));
    assert_eq!(slot.api.as_ref().unwrap().progress_retiring.len(), 1);
    settle_read(&mut slot, &snapshot(0x00f800));
    assert!(matches!(
        &slot.api.as_ref().unwrap().progress_page,
        Some(ProgressPage::Done { token: 12, .. })
    ));
    journal_fixture(&mut slot, 13);
    read(&mut slot, 14, "cook");
    settle_read(&mut slot, &snapshot(0x00f800));
    assert!(matches!(
        &slot.api.as_ref().unwrap().progress_page,
        Some(ProgressPage::Done { token: 14, .. })
    ));
    slot.stop();
}
#[test]
fn supersede_open_journal_refuses_second_closes_first_and_third_resolves() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    journal_fixture(&mut slot, 15);
    drive(&mut slot, &snapshot, 1, false);
    read(&mut slot, 16, "cook");
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { token: 16, reason }) if reason.as_ref() == "busy")
    );
    finish_journal(&mut slot, &mut snapshot, "Bring eggs: 3 eggs");
    assert!(
        matches!(
            &slot.api.as_ref().unwrap().progress_page,
            Some(ProgressPage::Refused { token: 16, .. })
        ),
        "abandoned first result cannot overwrite second refusal"
    );
    read(&mut slot, 17, "cook");
    settle_read(&mut slot, &snapshot);
    assert!(matches!(
        &slot.api.as_ref().unwrap().progress_page,
        Some(ProgressPage::Done { token: 17, .. })
    ));
    slot.stop();
}
#[test]
fn unavailable_colour_waits_eight_active_seconds_and_hold_does_not_read() {
    let mut slot = load();
    let snapshot = GameSnapshot::new();
    journal_fixture(&mut slot, 18);
    drive(&mut slot, &snapshot, 1, true);
    assert!(matches!(
        &slot.api.as_ref().unwrap().progress,
        ProgressSeat::Reading { since: None, .. }
    ));
    let now = Instant::now();
    slot.native_runtime
        .clock
        .observe(now - Duration::from_secs(7), true);
    if let ProgressSeat::Reading { since, .. } = &mut slot.api.as_mut().unwrap().progress {
        *since = Some(Duration::ZERO);
    }
    drive(&mut slot, &snapshot, 2, false);
    assert!(slot.api_owns_foreground());
    slot.native_runtime
        .clock
        .observe(now + Duration::from_secs(2), false);
    drive(&mut slot, &snapshot, 3, false);
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { reason, .. }) if reason.as_ref() == "unavailable:quest colour")
    );
    slot.stop();
}
#[test]
fn occupied_modal_refuses_busy_without_quiet_lease_or_click() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    snapshot.seed_main_modal(123, vec![]);
    journal_fixture(&mut slot, 19);
    drive(&mut slot, &snapshot, 1, false);
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { reason, .. }) if reason.as_ref() == "busy")
    );
    assert!(slot.native_runtime.ledger.is_none());
    slot.stop();
}
#[test]
fn reconnect_open_read_is_stale_and_explicit_reset_clears_pages_authority() {
    let mut slot = load();
    let snapshot = snapshot(0xf8f800);
    journal_fixture(&mut slot, 20);
    drive(&mut slot, &snapshot, 1, false);
    drive(&mut slot, &snapshot, 2, false);
    let authority = slot.take_native_action().unwrap().authority();
    assert!(slot.reconnect_session_work());
    assert!(!authority.live());
    slot.on_is_up(true);
    drive(&mut slot, &snapshot, 3, false);
    assert!(
        matches!(&slot.api.as_ref().unwrap().progress_page, Some(ProgressPage::Refused { token: 20, reason }) if reason.as_ref() == "stale")
    );
    journal_fixture(&mut slot, 21);
    drive(&mut slot, &snapshot, 4, false);
    slot.reset_session_work();
    assert!(!slot.api_owns_foreground());
    assert!(slot.api.as_ref().unwrap().progress_page.is_none());
    slot.stop();
    assert!(slot.api.is_none());
}
#[test]
fn established_load_progress_idle_polls_allocate_nothing_until_first_row() {
    let mut slot = load();
    let snapshot = snapshot(0x00f800);
    assert!(slot.api.is_none());
    let idle = allocation_counter::measure(|| {
        for tick in 1..1001 {
            drive(&mut slot, &snapshot, tick, false);
        }
    });
    assert_eq!(idle.count_total, 0);
    assert!(slot.api.is_none());
    println!(
        "progress idle Load: polls=1000 allocations={} bytes={} lazy_option={}B",
        idle.count_total,
        idle.bytes_total,
        std::mem::size_of::<Option<Box<ApiSeat>>>()
    );
    read(&mut slot, 22, "nope");
    assert!(slot.api.is_some());
    slot.stop();
}
#[test]
fn progress_flag_count_is_bounded_to_nine_digits_by_shared_resolver() {
    let mut slot = load();
    let mut snapshot = snapshot(0xf8f800);
    journal_fixture(&mut slot, 23);
    drive(&mut slot, &snapshot, 1, false);
    finish_journal(&mut slot, &mut snapshot, "Bring eggs: 999999999 eggs");
    let Some(ProgressPage::Done { row, .. }) = &slot.api.as_ref().unwrap().progress_page else {
        panic!("missing done");
    };
    assert_eq!(row.flags[0].count, Some(999999999));
    assert!(i32::try_from(row.flags[0].count.unwrap()).is_ok());
    snapshot = self::snapshot(0xf8f800);
    journal_fixture(&mut slot, 24);
    drive(&mut slot, &snapshot, 1, false);
    finish_journal(&mut slot, &mut snapshot, "Bring eggs: 1000000000 eggs");
    let Some(ProgressPage::Done { row, .. }) = &slot.api.as_ref().unwrap().progress_page else {
        panic!("missing done");
    };
    assert_eq!(row.flags[0].count, None);
    assert_eq!(row.flags[0].truth, Truth::Unknown);
    slot.stop();
}

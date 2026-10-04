use super::*;
use crate::ctx::test_support::NullDriver;
use crate::quester::families::tests::{local_player, with_tick};
use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView};
use std::time::Instant;

fn snapshot() -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_local_player(local_player(api::WorldTile {
        x: 3208,
        z: 3214,
        level: 0,
    }));
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf80000,
        }],
        true,
    );
    snapshot.seed_chat_lines(vec![]);
    snapshot
}

fn start(slot: &mut crate::SlotScript) {
    slot.start_compiled(
        "alice",
        CompiledId("Quester"),
        Arc::new(serde_json::from_value(serde_json::json!({"quests": ["cook"]})).unwrap()),
        api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
        Arc::default(),
    )
    .unwrap();
    settle(slot);
}

fn settle(slot: &mut crate::SlotScript) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            crate::StartPoll::Settled(crate::StartOutcome::Ready) => break,
            crate::StartPoll::NotOwed if slot.state() == crate::RunState::Running => break,
            crate::StartPoll::Pending | crate::StartPoll::NotOwed => {
                assert!(Instant::now() < deadline, "Quester preparation stalled");
                std::thread::yield_now();
            }
            result => panic!("Quester Start failed: {result:?}"),
        }
    }
}

fn tick(slot: &mut crate::SlotScript, snapshot: &GameSnapshot, id: u64) {
    slot.on_game_tick(&mut crate::ScriptCtx {
        driver: &mut NullDriver::default(),
        tick: id,
        here: snapshot
            .local_player()
            .map(|local| local.player.actor.tile)
            .map(|tile| (tile.x, tile.z, tile.level)),
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: crate::CompiledTick::default(),
    });
}

fn admit(slot: &mut crate::SlotScript, snapshot: &GameSnapshot, id: &mut u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        *id += 1;
        tick(slot, snapshot, *id);
        if slot
            .native_status()
            .is_some_and(|status| status.fields.iter().any(|field| field.key == "needs_read"))
        {
            return;
        }
        assert!(Instant::now() < deadline, "Quester Path activation stalled");
        assert_eq!(slot.state(), crate::RunState::Running);
        std::thread::yield_now();
    }
}

fn death(snapshot: &mut GameSnapshot, sequence: i32) {
    snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence,
    }]);
}

fn deaths(slot: &crate::SlotScript) -> i64 {
    slot.native_status()
        .unwrap()
        .fields
        .iter()
        .find_map(|field| match field {
            StatusField {
                key: "deaths",
                value: StatusValue::Integer(value),
                ..
            } => Some(*value),
            _ => None,
        })
        .expect("Quester publishes its queue-wide death count")
}

#[test]
fn recovery_r2_watchdog_walks_to_the_last_quest_stand_without_recreating() {
    let snapshot = snapshot();
    let stand = snapshot.local_player().unwrap().player.actor.tile;
    let mut slot = crate::SlotScript::new();
    slot.bind_incarnation(1);
    start(&mut slot);
    let mut id = 0;
    admit(&mut slot, &snapshot, &mut id);
    let before = slot.native_run();
    let now = Instant::now();
    let displaced = Some((stand.x + 20, stand.z, stand.level));
    slot.feed_watchdog(now, displaced, &[], false, true, &[]);
    let action = slot.feed_watchdog(
        now + crate::watchdog::WEDGE,
        displaced,
        &[],
        false,
        true,
        &[crate::shim::InteractReq::LoopSettled],
    );
    assert!(
        matches!(action, crate::WatchdogAction::ArmWalk { x, z, level }
            if x == stand.x && z == stand.z && level == stand.level),
        "a displaced Quester must use the shared slot walk-back, not discard its instance: {action:?}"
    );
    assert_eq!(slot.native_run(), before);
    slot.stop();
}

#[test]
fn recovery_r2_recreation_preserves_the_death_cap_and_does_not_replay_chat() {
    for death_in_gap in [false, true] {
        let mut snapshot = snapshot();
        let mut slot = crate::SlotScript::new();
        slot.bind_incarnation(1);
        start(&mut slot);
        let mut id = 0;
        admit(&mut slot, &snapshot, &mut id);
        for sequence in 1..=2 {
            death(&mut snapshot, sequence);
            id += 1;
            tick(&mut slot, &snapshot, id);
            assert_eq!(slot.state(), crate::RunState::Running);
            assert_eq!(deaths(&slot), i64::from(sequence));
        }
        let old_run = slot.native_run();
        slot.restart_from_identity(Instant::now()).unwrap();
        settle(&mut slot);
        assert_ne!(slot.native_run(), old_run);
        if death_in_gap {
            death(&mut snapshot, 3);
        }
        admit(&mut slot, &snapshot, &mut id);
        if !death_in_gap {
            assert_eq!(
                deaths(&slot),
                2,
                "recreation keeps deaths but consumes no old chat"
            );
            death(&mut snapshot, 3);
        }
        id += 1;
        tick(&mut slot, &snapshot, id);
        assert_eq!(
            slot.state(),
            crate::RunState::Idle,
            "a third death must Stop even across recreation"
        );
        let status = slot.native_status().unwrap();
        assert_eq!(status.failure.as_ref().unwrap().code.as_ref(), "max-deaths");
        assert_eq!(deaths(&slot), 3);

        // An explicit new Start, unlike watchdog recreation, gets a fresh budget.
        start(&mut slot);
        admit(&mut slot, &snapshot, &mut id);
        assert_eq!(deaths(&slot), 0);
        death(&mut snapshot, 4);
        id += 1;
        tick(&mut slot, &snapshot, id);
        assert_eq!(slot.state(), crate::RunState::Running);
        assert_eq!(deaths(&slot), 1);
        slot.stop();
    }
}

#[test]
fn recovery_r2_pause_hold_and_reconnect_start_a_fresh_unchanged_window() {
    for (freeze, thaw) in [
        (Interrupt::Pause, Interrupt::Resume),
        (Interrupt::Hold(true), Interrupt::Hold(false)),
        (Interrupt::SessionEnded, Interrupt::SessionReady),
    ] {
        let (mut quester, snapshot) = super::tests::fixture();
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |native| {
            for _ in 0..8 {
                quester.on_step_boundary(native);
            }
            assert!(!quester.parked);
        });
        quester.interrupt(freeze);
        quester.interrupt(thaw);
        with_tick(&snapshot, &mut ledger, 2, |native| {
            for _ in 0..8 {
                quester.on_step_boundary(native);
                assert!(
                    !quester.parked,
                    "{freeze:?}/{thaw:?} must not inherit a pre-freeze stall"
                );
            }
            quester.on_step_boundary(native);
            assert!(
                quester.parked,
                "new stalls still have the original eight-repeat bound"
            );
        });
    }
}

#[test]
fn recovery_r2_ineligible_frames_restamp_without_an_interrupt_callback() {
    let (mut quester, snapshot) = super::tests::fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |native| {
        for _ in 0..8 {
            quester.on_step_boundary(native);
        }
        native.cx.eligible = false;
        assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
    });
    with_tick(&snapshot, &mut ledger, 2, |native| {
        quester.on_step_boundary(native);
    });
    assert!(
        !quester.parked,
        "guardian eligibility holds must also discard stale stall counts"
    );
}

#[test]
fn recovery_r2_maximum_configured_cap_reports_the_first_disallowed_death() {
    let (mut quester, _) = super::tests::fixture();
    let mut snapshot = snapshot();
    quester.max_deaths = 255;
    snapshot.seed_chat_lines(vec![]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |native| {
        assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
    });
    for sequence in 1..=256 {
        death(&mut snapshot, sequence);
        let flow = with_tick(&snapshot, &mut ledger, sequence as u64 + 1, |native| {
            quester.tick(native).unwrap()
        });
        if sequence <= 255 {
            assert_eq!(flow, ScriptFlow::Continue);
        } else {
            assert!(matches!(flow, ScriptFlow::Blocked(_)));
        }
    }
    assert_eq!(
        usize::from(quester.deaths()),
        256,
        "the terminal death must be counted even at the configured u8 maximum"
    );
}

#[test]
fn recovery_r2_zero_hp_waits_for_death_chat_before_interrupting_dialogue() {
    use crate::quester::families::tests::seed_dialogue_combat;
    use api::snapshot::StatView;

    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut document = crate::quester::compile::decode_cook().unwrap();
    document.quest.as_mut().unwrap().owns_inventory = true;
    let step = &mut document.roles[0].sequences[0].steps[0];
    step.kind = "talk".into();
    step.args = serde_json::json!({"npc": "cook"});
    step.advances = Some(true);
    step.skip_if = crate::quester::path::PredicateDocument::Any(vec![]);
    step.settle = crate::quester::path::PredicateDocument::All(vec![]);
    let path =
        crate::quester::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    let mut quester = Quester::new(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        path,
        data,
        quests,
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
    let mut snapshot = snapshot();
    seed_dialogue_combat(&mut snapshot, false);
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "hitpoints".into(),
        effective: 10,
        base: 10,
        xp: 1154,
        used: true,
    }]);
    snapshot.seed_chat_modal(4882, vec![]);
    let mut ledger = None;
    for id in 1..=3 {
        with_tick(&snapshot, &mut ledger, id, |native| {
            assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
        });
    }
    assert!(quester.step.is_some(), "the live dialogue is being polled");
    let cursor = (quester.seq_index, quester.step_index);

    // Content applies lethal damage before its delayed death message. This is
    // not the living-player combat interruption covered by the existing test.
    snapshot.seed_chat_modal(-1, vec![]);
    seed_dialogue_combat(&mut snapshot, true);
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "hitpoints".into(),
        effective: 0,
        base: 10,
        xp: 1154,
        used: true,
    }]);
    for id in 4..=6 {
        with_tick(&snapshot, &mut ledger, id, |native| {
            assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
        });
        assert!(
            quester.step.is_some(),
            "HP alone does not cancel the dialogue"
        );
        assert_eq!((quester.seq_index, quester.step_index), cursor);
        assert_eq!(quester.deaths(), 0);
        assert!(!quester.parked);
    }

    death(&mut snapshot, 1);
    for id in 7..=8 {
        with_tick(&snapshot, &mut ledger, id, |native| {
            assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
        });
        assert_eq!(quester.deaths(), 1, "the delayed chat counts exactly once");
        assert!(quester.step.is_none(), "chat cancels the old dialogue");
        assert!(
            quester.needs_read,
            "server progress must be reread after death"
        );
    }
    seed_dialogue_combat(&mut snapshot, false);
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "hitpoints".into(),
        effective: 10,
        base: 10,
        xp: 1154,
        used: true,
    }]);
    with_tick(&snapshot, &mut ledger, 9, |native| {
        assert_eq!(quester.tick(native).unwrap(), ScriptFlow::Continue);
    });
    assert_eq!(quester.deaths(), 1);
    assert!(
        !quester.needs_read,
        "restored HP allows the authoritative reread"
    );
    assert!(!quester.parked);
}

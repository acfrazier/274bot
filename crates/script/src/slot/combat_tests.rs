//! Load-slot lifecycle of the `api.combat` seat: admission, busy, install,
//! Stop, reset, teardown and the Stop prayer handoff to the host.
use super::*;
use crate::api_combat::{CombatSessionRequest, InterruptCause, TargetSpec};
use crate::combat_session::tests::World;
use crate::ctx::test_support::NullDriver;
use crate::native::{HostEffect, InteractionReceipt};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap()
}

fn load_slot(incarnation: u64, data: bool) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_load_with_settings_and_game_data(
        "export function tick() {}".into(),
        crate::load::LoadShape::NativeTick,
        None,
        vec![],
        data.then(selected),
        Arc::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(outcome) => {
                assert_eq!(outcome, StartOutcome::Ready);
                break;
            }
            StartPoll::Pending => {
                assert!(Instant::now() < deadline, "Load start stalled");
                std::thread::yield_now();
            }
            StartPoll::NotOwed => panic!("no pending start"),
        }
    }
    slot.on_is_up(true);
    slot
}

fn request(target: TargetSpec) -> Arc<CombatSessionRequest> {
    Arc::new(CombatSessionRequest {
        target,
        ..CombatSessionRequest::default()
    })
}

fn imp() -> Arc<CombatSessionRequest> {
    request(TargetSpec::Names(Box::new([Arc::from("imp")])))
}

fn fight(slot: &mut SlotScript, token: u64, request: Arc<CombatSessionRequest>) {
    slot.consume_api_control(&InteractReq::CombatFight {
        request_id: token,
        request,
    });
}

fn stop(slot: &mut SlotScript, token: u64) {
    slot.consume_api_control(&InteractReq::CombatStop { request_id: token });
}

fn poll(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot, tick: u64) {
    slot.tick_api(&mut ScriptCtx {
        driver: &mut NullDriver::default(),
        tick,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(snapshot),
        obj_names: None,
        compiled: crate::CompiledTick::default(),
    });
}

fn seat(slot: &SlotScript) -> &ApiSeat {
    slot.api.as_ref().expect("API seat")
}

fn installed(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        poll(slot, snapshot, 1);
        if seat(slot)
            .combat_page
            .as_ref()
            .is_some_and(|page| page.phase == GatherPhase::Running)
        {
            return;
        }
        assert!(
            seat(slot).combat.token().is_some(),
            "refused: {:?}",
            seat(slot).combat_terminal
        );
        assert!(Instant::now() < deadline, "combat preparation stalled");
        std::thread::yield_now();
    }
}

fn prepared_terminal(slot: &mut SlotScript, snapshot: &api::snapshot::GameSnapshot) -> CombatEnd {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        poll(slot, snapshot, 1);
        if let Some(end) = seat(slot).combat_terminal.clone() {
            return end;
        }
        assert!(Instant::now() < deadline, "combat preparation stalled");
        std::thread::yield_now();
    }
}

fn accept_outbox(slot: &mut SlotScript, tick: u64) -> Vec<i32> {
    let mut buttons = Vec::new();
    let Some(ledger) = slot.native_runtime.ledger.as_mut() else {
        return buttons;
    };
    while !ledger.outbox.is_empty() {
        let action = ledger.outbox.remove(0);
        if let HostEffect::Interaction(request) = &action.effect {
            if let InteractReq::IfButton { component_id } = request {
                buttons.push(*component_id);
            }
            ledger.complete_interaction(
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
        }
    }
    buttons
}

#[test]
fn missing_game_data_refuses_and_a_duplicate_cannot_resurrect_it() {
    let mut slot = load_slot(301, false);
    fight(&mut slot, 301, imp());
    assert!(
        matches!(&seat(&slot).combat_terminal, Some(CombatEnd::Refused { token: 301, reason }) if reason.as_ref() == "unavailable:selected game data unavailable")
    );
    fight(&mut slot, 301, imp());
    assert!(!slot.api_owns_foreground());
    assert_eq!(seat(&slot).combat_terminal.as_ref().unwrap().token(), 301);
    slot.stop();
}

#[test]
fn an_unresolvable_name_is_a_keyed_host_refusal() {
    let world = World::new("imp");
    let mut slot = load_slot(302, true);
    fight(
        &mut slot,
        302,
        request(TargetSpec::Names(Box::new([Arc::from("no_such_npc")]))),
    );
    assert!(slot.api_owns_foreground(), "admitted while preparing");
    assert!(matches!(
        prepared_terminal(&mut slot, &world.snapshot),
        CombatEnd::Refused { token: 302, reason } if reason.as_ref() == "invalid-setting:target:unknown-npc"
    ));
    assert!(!slot.api_owns_foreground());
    assert!(seat(&slot).combat_page.is_none());
    slot.stop();
}

#[test]
fn one_api_session_per_slot_refuses_busy_both_ways() {
    let world = World::new("imp");
    let mut slot = load_slot(303, true);
    fight(&mut slot, 303, imp());
    assert!(slot.api_owns_foreground());
    // A second fight, a gather run and a progress read are all busy.
    fight(&mut slot, 304, imp());
    assert!(
        matches!(&seat(&slot).combat_terminal, Some(CombatEnd::Refused { token: 304, reason }) if reason.as_ref() == "busy")
    );
    slot.consume_api_control(&InteractReq::GatherRun {
        request_id: 305,
        settings: Arc::new(SettingsBag::new()),
    });
    assert!(
        matches!(&seat(&slot).terminal, Some(GatherEnd::Refused { token: 305, reason }) if reason.as_ref() == "busy")
    );
    slot.consume_api_control(&InteractReq::ProgressRead {
        request_id: 306,
        name: "cook".into(),
    });
    assert!(matches!(
        &seat(&slot).progress_page,
        Some(crate::api_progress::ProgressPage::Refused { token: 306, reason }) if reason.as_ref() == "busy"
    ));
    installed(&mut slot, &world.snapshot);
    assert_eq!(seat(&slot).combat_page.as_ref().unwrap().token, 303);
    stop(&mut slot, 303);
    assert!(!slot.api_owns_foreground());
    assert_eq!(
        seat(&slot).combat_terminal,
        Some(CombatEnd::Stopped { token: 303 })
    );
    // A live gather session makes a fight busy too.
    slot.consume_api_control(&InteractReq::GatherRun {
        request_id: 307,
        settings: Arc::new(SettingsBag::new()),
    });
    assert!(slot.api_owns_foreground());
    fight(&mut slot, 308, imp());
    assert!(
        matches!(&seat(&slot).combat_terminal, Some(CombatEnd::Refused { token: 308, reason }) if reason.as_ref() == "busy")
    );
    slot.stop();
}

#[test]
fn stop_hands_the_bot_raise_to_the_host_and_keeps_the_user_prayer() {
    let mut world = World::new("imp");
    let skin = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(skin.varp, true);
    world.attack();
    let mut slot = load_slot(310, true);
    fight(&mut slot, 310, imp());
    installed(&mut slot, &world.snapshot);
    let mut tick = 2;
    loop {
        assert!(tick < 30, "Combat never raised its protection");
        poll(&mut slot, &world.snapshot, tick);
        let clicked = accept_outbox(&mut slot, tick);
        tick += 1;
        if clicked.contains(&raised.button_com) {
            assert!(!clicked.contains(&skin.button_com));
            world.set_prayer(raised.varp, true);
            world.attack();
            poll(&mut slot, &world.snapshot, tick);
            accept_outbox(&mut slot, tick);
            break;
        }
    }
    assert_eq!(
        seat(&slot)
            .combat_page
            .as_ref()
            .unwrap()
            .status
            .as_ref()
            .map(|status| &status.fields[0].value),
        Some(&StatusValue::Text(Arc::from("fighting")))
    );
    stop(&mut slot, 310);
    assert_eq!(
        seat(&slot).combat_terminal,
        Some(CombatEnd::Stopped { token: 310 })
    );
    assert!(seat(&slot).combat_page.is_none());
    assert!(!slot.has_native_actions(), "Stop revokes the fight");
    let owed = slot.take_stop_prayer_cleanup();
    assert!(
        owed.contains(raised.varp),
        "the bot's raise is the host's to clear"
    );
    assert!(!owed.contains(skin.varp), "the user's prayer is never owed");
    // A late duplicate Stop does not replace the terminal.
    stop(&mut slot, 310);
    assert_eq!(
        seat(&slot).combat_terminal,
        Some(CombatEnd::Stopped { token: 310 })
    );
    slot.stop();
}

#[test]
fn pause_settles_interrupted_after_the_scoped_clear() {
    let mut world = World::new("imp");
    let skin = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(skin.varp, true);
    world.attack();
    let mut slot = load_slot(311, true);
    fight(&mut slot, 311, imp());
    installed(&mut slot, &world.snapshot);
    let mut tick = 2;
    loop {
        assert!(tick < 30, "Combat never raised its protection");
        poll(&mut slot, &world.snapshot, tick);
        let clicked = accept_outbox(&mut slot, tick);
        tick += 1;
        if clicked.contains(&raised.button_com) {
            world.set_prayer(raised.varp, true);
            world.attack();
            poll(&mut slot, &world.snapshot, tick);
            accept_outbox(&mut slot, tick);
            tick += 1;
            break;
        }
    }
    slot.pause();
    slot.resume();
    let mut cleared = Vec::new();
    while seat(&slot).combat_terminal.is_none() {
        assert!(tick < 60, "pause never settled");
        poll(&mut slot, &world.snapshot, tick);
        let clicked = accept_outbox(&mut slot, tick);
        if clicked.contains(&raised.button_com) {
            world.set_prayer(raised.varp, false);
        }
        cleared.extend(clicked);
        tick += 1;
    }
    assert_eq!(cleared, vec![raised.button_com]);
    assert_eq!(
        seat(&slot).combat_terminal,
        Some(CombatEnd::Interrupted {
            token: 311,
            cause: InterruptCause::Pause
        })
    );
    assert!(world.prayer_is_on(skin.varp));
    assert!(
        slot.take_stop_prayer_cleanup().is_empty(),
        "a cleared session owes the host nothing"
    );
    slot.stop();
}

#[test]
fn death_settles_and_owes_nothing() {
    let mut world = World::new("imp");
    let raised = world.prayer("Protect from Missiles");
    world.attack();
    let mut slot = load_slot(312, true);
    fight(&mut slot, 312, imp());
    installed(&mut slot, &world.snapshot);
    let mut tick = 2;
    loop {
        assert!(tick < 30, "Combat never raised its protection");
        poll(&mut slot, &world.snapshot, tick);
        let clicked = accept_outbox(&mut slot, tick);
        tick += 1;
        if clicked.contains(&raised.button_com) {
            world.set_prayer(raised.varp, true);
            world.attack();
            break;
        }
    }
    poll(&mut slot, &world.snapshot, tick);
    accept_outbox(&mut slot, tick);
    world.die();
    poll(&mut slot, &world.snapshot, tick + 1);
    assert_eq!(
        seat(&slot).combat_terminal,
        Some(CombatEnd::Interrupted {
            token: 312,
            cause: InterruptCause::Died
        })
    );
    assert!(slot.take_stop_prayer_cleanup().is_empty());
    assert!(!slot.api_owns_foreground());
    slot.stop();
}

#[test]
fn reset_ends_without_a_terminal_reconnect_keeps_and_teardown_drops_the_seat() {
    let world = World::new("imp");
    let mut slot = load_slot(313, true);
    fight(&mut slot, 313, imp());
    installed(&mut slot, &world.snapshot);
    slot.reset_session_work();
    assert!(!slot.api_owns_foreground());
    assert!(seat(&slot).combat_terminal.is_none());
    assert!(seat(&slot).combat_page.is_none());
    // Reconnect keeps a live session.
    fight(&mut slot, 314, imp());
    installed(&mut slot, &world.snapshot);
    assert!(slot.reconnect_session_work());
    assert!(slot.api_owns_foreground());
    slot.on_is_up(true);
    slot.teardown_api(StopReason::Replaced);
    assert!(slot.api.is_none());
    slot.stop();
}

#[test]
fn operator_stop_mid_fight_hands_the_bot_raise_to_the_host() {
    let mut world = World::new("imp");
    let skin = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(skin.varp, true);
    world.attack();
    let mut slot = load_slot(315, true);
    fight(&mut slot, 315, imp());
    installed(&mut slot, &world.snapshot);
    let mut tick = 2;
    loop {
        assert!(tick < 30, "Combat never raised its protection");
        poll(&mut slot, &world.snapshot, tick);
        let clicked = accept_outbox(&mut slot, tick);
        tick += 1;
        if clicked.contains(&raised.button_com) {
            world.set_prayer(raised.varp, true);
            world.attack();
            poll(&mut slot, &world.snapshot, tick);
            accept_outbox(&mut slot, tick);
            break;
        }
    }
    slot.stop();
    let owed = slot.take_stop_prayer_cleanup();
    assert!(owed.contains(raised.varp));
    assert!(!owed.contains(skin.varp));
}

use super::*;
use crate::api_combat::{CombatEnd, CombatSessionRequest, TargetSpec};
use crate::api_gather::{GatherEnd, GatherPhase};
use crate::api_progress::ProgressPage;
use crate::ctx::test_support::NullDriver;
use crate::isolate_fb::{DecodedSeatBatch, SeatInteractDecoded, SeatStartArgs, SeatStartInput};
use crate::native::request::NativeKind;
use crate::native::seat::SeatTerminal;
use crate::native::{HostEffect, InteractionReceipt, SettingsBag};
use crate::shim::InteractReq;
use api::identity::RequestId;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::GameSnapshot;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R289).unwrap()
}

fn load_slot(incarnation: u64) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_load_with_settings_and_game_data(
        "export function tick() {}".into(),
        crate::load::LoadShape::NativeTick,
        None,
        vec![],
        &[],
        Some(selected()),
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
            StartPoll::NotOwed => panic!("no pending Load start"),
        }
    }
    slot.on_is_up(true);
    slot
}

fn with_seat(incarnation: u64) -> SlotScript {
    let mut slot = load_slot(incarnation);
    let session = slot.work_epoch;
    slot.arm_seat(RunKey {
        slot: incarnation,
        run: 7,
        session,
    });
    slot
}

fn id(raw: u64) -> RequestId {
    RequestId::isolate(raw).unwrap()
}

fn gather_settings() -> SettingsBag {
    let mut settings = SettingsBag::new();
    settings.insert("disposition".into(), serde_json::json!("Power"));
    settings.insert("deathPolicy".into(), serde_json::json!("Stop"));
    settings
}

fn family_args(kind: NativeKind) -> SeatStartArgs {
    match kind {
        NativeKind::Gather => SeatStartArgs::Gather(gather_settings()),
        NativeKind::Combat => SeatStartArgs::Combat(CombatSessionRequest::default()),
        NativeKind::QuestProgress => SeatStartArgs::QuestProgress("cook".into()),
        NativeKind::Hunt => SeatStartArgs::Empty,
        _ => SeatStartArgs::Empty,
    }
}

fn start_input(slot: &SlotScript, raw: u64, kind: NativeKind) -> SeatStartInput {
    let run = slot.seat.as_ref().expect("armed seat").run();
    SeatStartInput {
        id: id(raw),
        parent: None,
        run: run.run,
        session: run.session,
        kind,
        args: family_args(kind),
    }
}

fn decoded(starts: Vec<SeatStartInput>, reqs: Vec<SeatInteractDecoded>) -> DecodedSeatBatch {
    DecodedSeatBatch {
        consumed_terminal_seq: 0,
        starts,
        decisions: Vec::new(),
        cancels: Vec::new(),
        mirror_revokes: Vec::new(),
        reqs,
    }
}

fn frame(slot: &mut SlotScript, snapshot: &GameSnapshot, tick: u64, hold: bool) {
    slot.on_game_tick(&mut ScriptCtx {
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

/// §28.7 case 8: both Capability/Session start orders go through the real
/// SlotScript batch consumer. A Capability root admitted first can coexist
/// with the later Session; a Session-first Capability root is Busy, including
/// under Hunt. Gather, Combat, and Progress own foreground; Hunt does not.
/// A second Session root is Busy in either order.
#[test]
fn fixture_08_slot_family_foreground_and_reverse_order_busy() {
    for (family_index, family) in [
        NativeKind::Gather,
        NativeKind::Combat,
        NativeKind::QuestProgress,
        NativeKind::Hunt,
    ]
    .into_iter()
    .enumerate()
    {
        for session_first in [false, true] {
            let incarnation = 700 + family_index as u64 * 2 + (if session_first { 1 } else { 0 });
            let mut slot = with_seat(incarnation);
            let family_raw = if session_first { 1 } else { 2 };
            let capability_raw = if session_first { 2 } else { 1 };
            let family_id = id(family_raw);
            let capability_id = id(capability_raw);
            let capability = start_input(&slot, capability_raw, NativeKind::BankNearest);
            let family_start = start_input(&slot, family_raw, family);
            let competing_family = if family == NativeKind::Hunt {
                NativeKind::Gather
            } else {
                NativeKind::Hunt
            };
            let competing_start = start_input(&slot, 3, competing_family);
            let starts = if session_first {
                vec![family_start, capability, competing_start]
            } else {
                vec![capability, family_start, competing_start]
            };
            let gameplay = InteractReq::WalkTo {
                x: 3200,
                z: 3201,
                level: 0,
            };
            slot.consume_seat_batch(
                decoded(
                    starts,
                    vec![SeatInteractDecoded {
                        lineage: Some(capability_id),
                        mirror_owner: None,
                        request: gameplay.clone(),
                    }],
                ),
                1,
                &[],
            );

            let foreground = (family != NativeKind::Hunt).then_some(family_id);
            assert_eq!(slot.seat.as_ref().unwrap().session(), Some(family_id));
            assert_eq!(slot.seat.as_ref().unwrap().foreground(), foreground);
            assert!(matches!(
                slot.seat
                    .as_ref()
                    .unwrap()
                    .results()
                    .iter()
                    .find(|result| result.id == id(3))
                    .map(|result| result.outcome),
                Some(SeatTerminal::Refused(api::script_contract::Refusal::Busy))
            ));
            if session_first {
                assert!(matches!(
                    slot.seat
                        .as_ref()
                        .unwrap()
                        .results()
                        .iter()
                        .find(|result| result.id == capability_id)
                        .map(|result| result.outcome),
                    Some(SeatTerminal::Refused(api::script_contract::Refusal::Busy))
                ));
            } else {
                assert!(slot
                    .seat
                    .as_ref()
                    .unwrap()
                    .live()
                    .iter()
                    .any(|entry| entry.id == capability_id));
            }

            match family {
                NativeKind::Gather => assert_eq!(
                    slot.api.as_ref().unwrap().page.as_ref().unwrap().token,
                    family_id.get()
                ),
                NativeKind::Combat => assert_eq!(
                    slot.api
                        .as_ref()
                        .unwrap()
                        .combat_page
                        .as_ref()
                        .unwrap()
                        .token,
                    family_id.get()
                ),
                NativeKind::QuestProgress => assert!(matches!(
                    &slot.api.as_ref().unwrap().progress_page,
                    Some(ProgressPage::Reading { token }) if *token == family_id.get()
                )),
                NativeKind::Hunt => assert!(slot.api.is_none()),
                _ => unreachable!(),
            }
            assert_eq!(slot.api_owns_foreground(), family != NativeKind::Hunt);
            let delivered = slot.drain_interacts();
            if family == NativeKind::Hunt && !session_first {
                assert_eq!(delivered, [gameplay]);
                assert_eq!(slot.seat.as_ref().unwrap().dropped(), 0);
            } else {
                assert!(delivered.is_empty());
                assert_eq!(slot.seat.as_ref().unwrap().dropped(), 1);
            }
            slot.stop();
        }
    }
}

/// §28.7 case 26: a Progress read started first keeps the shared API
/// foreground and refuses the reverse-order Gather and Combat starts.
#[test]
fn fixture_26_progress_first_refuses_gather_and_combat_as_busy() {
    let mut slot = with_seat(720);
    let progress = start_input(&slot, 720, NativeKind::QuestProgress);
    slot.consume_seat_batch(decoded(vec![progress], Vec::new()), 1, &[]);
    assert!(matches!(
        &slot.api.as_ref().unwrap().progress_page,
        Some(ProgressPage::Reading { token: 720 })
    ));
    assert!(slot.api_owns_foreground());

    for (raw, kind) in [(721, NativeKind::Gather), (722, NativeKind::Combat)] {
        let competing = start_input(&slot, raw, kind);
        slot.consume_seat_batch(decoded(vec![competing], Vec::new()), 1, &[]);
        assert!(matches!(
            slot.seat
                .as_ref()
                .unwrap()
                .results()
                .iter()
                .find(|result| result.id == id(raw))
                .map(|result| result.outcome),
            Some(SeatTerminal::Refused(api::script_contract::Refusal::Busy))
        ));
    }
    assert!(
        slot.api_owns_foreground(),
        "the original Progress read remains live"
    );
    assert!(slot.api.as_ref().unwrap().terminal.is_none());
    assert!(slot.api.as_ref().unwrap().combat_terminal.is_none());
    slot.stop();
}

/// §28.7 case 28: the legacy interaction control is dropped at the host
/// boundary; the same id is admitted only when carried as a typed seat start.
#[test]
fn fixture_28_legacy_gather_control_drops_but_typed_start_is_admitted() {
    let mut slot = with_seat(728);
    let token = 728;
    slot.restore_interacts(vec![InteractReq::GatherRun {
        request_id: token,
        settings: Arc::new(gather_settings()),
    }]);
    let (_, rows, foreground) = slot.drain_host_interacts();
    assert!(rows.is_empty());
    assert!(!foreground);
    assert!(slot.seat.as_ref().unwrap().live().is_empty());
    assert_eq!(slot.seat.as_ref().unwrap().dropped(), 1);
    assert!(slot.api.is_none());

    slot.consume_seat_batch(
        decoded(
            vec![start_input(&slot, token, NativeKind::Gather)],
            Vec::new(),
        ),
        1,
        &[],
    );
    assert!(slot
        .seat
        .as_ref()
        .unwrap()
        .live()
        .iter()
        .any(|entry| entry.id == id(token)));
    assert!(slot.api_owns_foreground());
    assert_eq!(
        slot.api.as_ref().unwrap().page.as_ref().unwrap().token,
        token
    );
    slot.stop();
}

/// §28.7 case 3: manual movement reaches the installed Gather family even
/// before it has a walk in flight; its real terminal releases foreground.
#[test]
fn fixture_03_manual_walk_interrupts_gather_without_an_active_bot_walk() {
    let mut slot = with_seat(730);
    let token = 730;
    slot.consume_seat_batch(
        decoded(
            vec![start_input(&slot, token, NativeKind::Gather)],
            Vec::new(),
        ),
        1,
        &[],
    );
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut tick = 1;
    while !slot
        .api
        .as_ref()
        .and_then(|seat| seat.page.as_ref())
        .is_some_and(|page| page.phase == GatherPhase::Running)
    {
        frame(&mut slot, &snapshot, tick, false);
        assert!(Instant::now() < deadline, "Gather preparation stalled");
        tick += 1;
    }
    assert!(slot.manual_input_owner());
    assert!(
        !slot.live_walking_operation(),
        "the manual click precedes any bot walk"
    );

    slot.note_manual_walk_takeover(41, tick);
    frame(&mut slot, &snapshot, tick + 1, false);
    assert!(matches!(
        &slot.api.as_ref().unwrap().terminal,
        Some(GatherEnd::Blocked { token: 730, failure, .. })
            if failure.code.as_ref() == "manual-movement"
    ));
    assert!(!slot.api_owns_foreground());
    assert!(matches!(
        slot.seat
            .as_ref()
            .unwrap()
            .results()
            .iter()
            .find(|result| result.id == id(token))
            .map(|result| result.outcome),
        Some(SeatTerminal::Failed)
    ));
    slot.stop();
}

/// §28.7 cases 10/29: guardian hold excludes a second admission, leaves the
/// live Gather foreground unchanged, and retains its reserved Stop release.
/// This direct decoded-batch fixture does not claim Load's deferred-start
/// carrier retry; the native guardian fixture owns that separate proof.
#[test]
fn fixture_10_guardian_hold_keeps_gather_stop_reserved_until_resume() {
    let mut slot = with_seat(740);
    let token = 740;
    slot.consume_seat_batch(
        decoded(
            vec![start_input(&slot, token, NativeKind::Gather)],
            Vec::new(),
        ),
        1,
        &[],
    );
    assert!(slot.api_owns_foreground());

    slot.seat_guardian_hold(true);
    slot.consume_seat_batch(
        decoded(
            vec![start_input(&slot, token + 1, NativeKind::Hunt)],
            Vec::new(),
        ),
        1,
        &[],
    );
    assert_eq!(slot.seat.as_ref().unwrap().live().len(), 1);
    assert!(!slot
        .seat
        .as_ref()
        .unwrap()
        .results()
        .iter()
        .any(|result| result.id == id(token + 1)));
    assert!(slot.api_owns_foreground());

    let stop = InteractReq::GatherStop { request_id: token };
    slot.consume_seat_batch(
        decoded(
            Vec::new(),
            vec![SeatInteractDecoded {
                lineage: Some(id(token)),
                mirror_owner: None,
                request: stop,
            }],
        ),
        1,
        &[],
    );
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 1);
    let (_, held_rows, held_foreground) = slot.drain_host_interacts();
    assert!(held_rows.is_empty());
    assert!(held_foreground);
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 1);
    assert!(slot.api.as_ref().unwrap().terminal.is_none());

    slot.seat_guardian_hold(false);
    let (_, rows, foreground) = slot.drain_host_interacts();
    assert!(
        rows.is_empty(),
        "GatherStop is consumed as a reserved control"
    );
    assert!(!foreground);
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 0);
    assert!(matches!(
        &slot.api.as_ref().unwrap().terminal,
        Some(GatherEnd::Stopped { token: 740, .. })
    ));
    assert!(matches!(
        slot.seat
            .as_ref()
            .unwrap()
            .results()
            .iter()
            .find(|result| result.id == id(token))
            .map(|result| result.outcome),
        Some(SeatTerminal::Done)
    ));
    slot.stop();
}

fn combat_request() -> CombatSessionRequest {
    CombatSessionRequest {
        target: TargetSpec::Names(Box::new([Arc::from("imp")])),
        ..CombatSessionRequest::default()
    }
}

fn wait_combat_running(slot: &mut SlotScript, snapshot: &GameSnapshot) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut tick = 1;
    loop {
        frame(slot, snapshot, tick, false);
        if slot
            .api
            .as_ref()
            .and_then(|seat| seat.combat_page.as_ref())
            .is_some_and(|page| page.phase == GatherPhase::Running)
        {
            return;
        }
        assert!(Instant::now() < deadline, "Combat preparation stalled");
        tick += 1;
    }
}

fn accept_native_interactions(slot: &mut SlotScript, tick: u64) -> Vec<i32> {
    let mut buttons = Vec::new();
    while let Some(action) = slot.take_native_action() {
        let HostEffect::Interaction(request) = &action.effect else {
            panic!("Combat emitted a non-interaction action");
        };
        if let InteractReq::IfButton { component_id } = request {
            buttons.push(*component_id);
        }
        let authority = action.authority();
        slot.complete_native_interaction(
            &authority,
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
    buttons
}

fn until_combat_raises(
    slot: &mut SlotScript,
    world: &mut crate::combat_session::tests::World,
    raised: i32,
    varp: i32,
) {
    let mut tick = 2;
    loop {
        assert!(tick < 30, "Combat never raised its protection");
        frame(slot, &world.snapshot, tick, false);
        let clicked = accept_native_interactions(slot, tick);
        tick += 1;
        if clicked.contains(&raised) {
            world.set_prayer(varp, true);
            world.attack();
            frame(slot, &world.snapshot, tick, false);
            accept_native_interactions(slot, tick);
            return;
        }
    }
}

/// §28.7 cases 3/29: a CombatStop delivered under the Combat foreground uses
/// its reserved release slot, filters competing gameplay, settles the real
/// family, and hands the bot-raised prayer to host cleanup.
#[test]
fn fixture_29_combat_stop_release_survives_foreground_and_hands_off_cleanup() {
    let mut world = crate::combat_session::tests::World::new("imp");
    let user_prayer = world.prayer("Thick Skin");
    let raised = world.prayer("Protect from Missiles");
    world.set_prayer(user_prayer.varp, true);
    world.attack();

    let mut slot = with_seat(750);
    let token = 750;
    let mut start = start_input(&slot, token, NativeKind::Combat);
    start.args = SeatStartArgs::Combat(combat_request());
    slot.consume_seat_batch(decoded(vec![start], Vec::new()), 1, &[]);
    wait_combat_running(&mut slot, &world.snapshot);
    until_combat_raises(&mut slot, &mut world, raised.button_com, raised.varp);
    assert!(slot.api_owns_foreground());

    let stop = InteractReq::CombatStop { request_id: token };
    slot.consume_seat_batch(
        decoded(
            Vec::new(),
            vec![
                SeatInteractDecoded {
                    lineage: None,
                    mirror_owner: None,
                    request: InteractReq::WalkTo {
                        x: 3225,
                        z: 3220,
                        level: 0,
                    },
                },
                SeatInteractDecoded {
                    lineage: Some(id(token)),
                    mirror_owner: None,
                    request: stop.clone(),
                },
            ],
        ),
        1,
        &[],
    );
    assert!(matches!(
        slot.seat_interact_delivery.as_slice(),
        [queued] if queued.req == stop
    ));
    assert!(slot.seat.as_ref().unwrap().dropped() >= 1);
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 0);

    let (_, rows, foreground) = slot.drain_host_interacts();
    assert!(rows.is_empty());
    assert!(!foreground);
    assert!(slot.seat_interact_delivery.is_empty());
    assert_eq!(
        slot.api.as_ref().unwrap().combat_terminal,
        Some(CombatEnd::Stopped { token })
    );
    assert!(slot.api.as_ref().unwrap().combat_page.is_none());
    assert!(matches!(
        slot.seat
            .as_ref()
            .unwrap()
            .results()
            .iter()
            .find(|result| result.id == id(token))
            .map(|result| result.outcome),
        Some(SeatTerminal::Done)
    ));
    let owed = slot.take_stop_prayer_cleanup();
    assert!(owed.contains(raised.varp));
    assert!(!owed.contains(user_prayer.varp));
    slot.stop();
}

/// §28.7 case 27 host half: a Hunt Session, local Walk mirror and typed
/// bankNearest Capability share the real slot consumer. The host-facing drain
/// must deliver the armed WalkNear, its fenced AbortWalk, and a mirror row
/// rebased to root after ordinary Hunt completion.
#[test]
fn fixture_27_hybrid_hunt_walk_bank_releases_and_rebases_at_host_drain() {
    let mut slot = with_seat(760);
    let hunt_id = id(760);
    let mirror_id = id(761);
    let bank_id = id(762);
    slot.consume_seat_batch(
        decoded(vec![start_input(&slot, 760, NativeKind::Hunt)], Vec::new()),
        0,
        &[],
    );
    assert_eq!(slot.seat.as_ref().unwrap().session(), Some(hunt_id));
    assert!(
        !slot.api_owns_foreground(),
        "Hunt's Session does not filter other gameplay"
    );

    let mut bank = start_input(&slot, 762, NativeKind::BankNearest);
    bank.parent = Some(hunt_id);
    let walk = InteractReq::WalkNear {
        x: 3300,
        z: 3300,
        level: 0,
        radius: 1,
        compat_tile_intent: false,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id: mirror_id.get(),
        avoid: Vec::new(),
        cross: Vec::new(),
    };
    slot.consume_seat_batch(
        decoded(
            vec![bank],
            vec![SeatInteractDecoded {
                lineage: Some(hunt_id),
                mirror_owner: Some(mirror_id),
                request: walk.clone(),
            }],
        ),
        0,
        &[],
    );
    assert_eq!(
        slot.seat
            .as_ref()
            .unwrap()
            .live()
            .iter()
            .find(|entry| entry.id == bank_id)
            .and_then(|entry| entry.parent),
        Some(hunt_id)
    );
    let (_, dispatched, foreground) = slot.drain_host_interacts();
    assert!(!foreground);
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].req, walk);

    let release = InteractReq::AbortWalk {
        request_id: mirror_id.get(),
    };
    slot.consume_seat_batch(
        decoded(
            Vec::new(),
            vec![SeatInteractDecoded {
                lineage: Some(hunt_id),
                mirror_owner: Some(mirror_id),
                request: release.clone(),
            }],
        ),
        0,
        &[],
    );
    let (_, dispatched, _) = slot.drain_host_interacts();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].req, release);
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 0);

    slot.seat
        .as_mut()
        .unwrap()
        .complete(hunt_id, false)
        .unwrap();
    let rebased = InteractReq::WalkTo {
        x: 3223,
        z: 3222,
        level: 0,
    };
    slot.consume_seat_batch(
        decoded(
            Vec::new(),
            vec![SeatInteractDecoded {
                lineage: None,
                mirror_owner: Some(mirror_id),
                request: rebased.clone(),
            }],
        ),
        0,
        &[],
    );
    assert_eq!(
        slot.seat
            .as_ref()
            .unwrap()
            .live()
            .iter()
            .find(|entry| entry.id == bank_id)
            .and_then(|entry| entry.parent),
        None,
        "ordinary Hunt completion promotes the surviving bank child"
    );
    let (_, dispatched, _) = slot.drain_host_interacts();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].req, rebased);
    let mut cancellation = decoded(Vec::new(), Vec::new());
    cancellation
        .cancels
        .push(crate::isolate_fb::SeatCancelDecoded {
            of: bank_id,
            reason: api::script_contract::CallbackCancelReason::ParentAborted,
        });
    slot.consume_seat_batch(cancellation, 0, &[]);
    assert!(matches!(
        slot.seat
            .as_ref()
            .unwrap()
            .results()
            .iter()
            .find(|result| result.id == bank_id)
            .map(|result| result.outcome),
        Some(SeatTerminal::Aborted(
            api::script_contract::AbortReason::ParentAborted
        ))
    ));
    slot.stop();
}

/// §28.7 row 24/29: four separate Walk heads reserve physical releases, and
/// their actual AbortWalk interactions pass the host drain after takeover.
#[test]
fn fixture_24_four_walk_abort_releases_reach_host_in_order() {
    let mut slot = with_seat(804);
    let owners = [804, 805, 806, 807];
    let starts = owners
        .iter()
        .map(|&raw| start_input(&slot, raw, NativeKind::Walk))
        .collect();
    slot.consume_seat_batch(decoded(starts, Vec::new()), 0, &[]);
    assert_eq!(slot.seat.as_ref().unwrap().live().len(), owners.len());

    let walks = owners
        .iter()
        .map(|&raw| SeatInteractDecoded {
            lineage: Some(id(raw)),
            mirror_owner: None,
            request: InteractReq::WalkNear {
                x: 3300,
                z: 3300,
                level: 0,
                radius: 1,
                compat_tile_intent: false,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: raw,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
        })
        .collect();
    slot.consume_seat_batch(decoded(Vec::new(), walks), 0, &[]);
    let (_, armed, _) = slot.drain_host_interacts();
    assert_eq!(armed.len(), owners.len());
    assert_eq!(
        slot.seat.as_ref().unwrap().reserved_releases(),
        owners.len()
    );

    for raw in owners {
        slot.seat.as_mut().unwrap().manual_walk(id(raw)).unwrap();
    }
    let releases = owners
        .iter()
        .map(|&raw| SeatInteractDecoded {
            lineage: Some(id(raw)),
            mirror_owner: None,
            request: InteractReq::AbortWalk { request_id: raw },
        })
        .collect();
    slot.consume_seat_batch(decoded(Vec::new(), releases), 0, &[]);
    let (_, dispatched, _) = slot.drain_host_interacts();
    assert_eq!(
        dispatched
            .into_iter()
            .map(|effect| effect.req)
            .collect::<Vec<_>>(),
        owners
            .into_iter()
            .map(|request_id| InteractReq::AbortWalk { request_id })
            .collect::<Vec<_>>(),
    );
    assert_eq!(slot.seat.as_ref().unwrap().reserved_releases(), 0);
    slot.stop();
}

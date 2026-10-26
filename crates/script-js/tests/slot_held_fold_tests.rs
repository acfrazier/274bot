use api::run_policy::RunEnergyMin;
use api::selected::ClientRevision;
use api::snapshot::stat_used;
use script::interact::{InteractReq, QueuedInteract};
use script::native::seat_rows::QueuedSeatBatch;
use script::native::snapshot_source::{
    NativeFactsInput, ReachViewInput, SnapshotInput, StatInput, TileInput,
};
use script::slot::{SlotScript, StartOutcome, StartPoll};
use script::surface::SurfaceStart;
use script_js::surface::{JsStartCard, JsSurfaceFactory};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn paint_slot(source: &str, incarnation: u64) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    let source: Arc<str> = source.into();
    let family = script_js::resolve_api_family(&source)
        .ok()
        .map(|(_, family)| family);
    slot.bind_surface_factory(Arc::new(JsSurfaceFactory::default()));
    slot.start_surface(SurfaceStart {
        card: Arc::new(JsStartCard {
            source,
            shape: script_js::LoadShape::CompatClass,
            siblings: Arc::from(Vec::<(String, String)>::new()),
            family,
        }),
        settings_bag: None,
        game_data: Some(
            api::game_data::for_revision(ClientRevision::R289).expect("selected game data"),
        ),
        named_banks: Arc::default(),
        loadouts: Arc::default(),
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            StartPoll::Settled(result) => {
                assert_eq!(result, StartOutcome::Ready);
                break;
            }
            StartPoll::Pending => assert!(Instant::now() < deadline, "Load start stalled"),
            StartPoll::NotOwed => panic!("start not owed"),
        }
        std::thread::yield_now();
    }
    slot.on_is_up(true);
    slot
}

fn paint_tick(slot: &mut SlotScript, tick: u64, held: bool) {
    let stats: Vec<_> = (0..64)
        .filter(|&index| stat_used(index))
        .map(|index| StatInput {
            index: index as i32,
            name: "skill",
            xp: 0,
            base: 1,
            effective: 1,
        })
        .collect();
    let input = SnapshotInput {
        tick,
        here: Some(TileInput {
            x: 3187,
            z: 3245,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &stats,
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: held,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    };
    assert!(slot.post_snapshot_delta(&input, NativeFactsInput::default(), false));
    slot.with_surface_run_for_test(|run| run.on_game_tick_at(tick, 0))
        .expect("active JS surface");
    slot.probe("true").unwrap();
}

fn take_batches(slot: &mut SlotScript) -> Vec<QueuedSeatBatch> {
    slot.with_surface_run_for_test(|run| run.drain_seat_batches())
        .expect("active JS surface")
}

fn restore_batches(slot: &mut SlotScript, batches: Vec<QueuedSeatBatch>) {
    slot.with_surface_run_for_test(|run| run.restore_seat_batches(batches))
        .expect("active JS surface");
}

fn held_rows(slot: &mut SlotScript) -> Vec<InteractReq> {
    let batches = take_batches(slot);
    let rows = batches
        .iter()
        .flat_map(|batch| batch.batch.reqs.iter().map(|row| row.request.clone()))
        .collect();
    restore_batches(slot, batches);
    rows
}

fn requests(reqs: Vec<QueuedInteract>) -> Vec<InteractReq> {
    reqs.into_iter().map(|queued| queued.req).collect()
}

fn dropped(slot: &SlotScript) -> u64 {
    slot.slot_metrics().dropped
}

#[test]
fn held_onpaint_policy_ends_at_the_eighteenth_update() {
    let source = r#"
import { RunManager } from '../../runtime/RunManager.js';
globalThis.__p = 0;
export default class T extends LoopingBot {
    loop() {}
    onPaint() { RunManager.override({ energyMin: globalThis.__p++ }); }
}
"#;
    let mut slot = paint_slot(source, 858);
    paint_tick(&mut slot, 1, false);
    let _ = slot.drain_host_interacts();
    slot.seat_guardian_hold(true);
    for tick in 2..=19 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let paints = slot.probe("globalThis.__p").unwrap();
    slot.seat_guardian_hold(false);
    let (policy, _, _) = slot.drain_host_interacts();
    slot.stop();
    assert_eq!(paints, serde_json::json!(19), "the producer ran all paints");
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(18))
    );
}

#[test]
fn held_onpaint_camera_yaws_keep_the_last_yaw() {
    let source = r#"
globalThis.__p = 0;
export default class T extends LoopingBot {
    loop() {}
    onPaint() {
        globalThis.__rs2b0t_host.interact.push({ op: 'set-camera-yaw', yaw: globalThis.__p++ });
    }
}
"#;
    let mut slot = paint_slot(source, 859);
    paint_tick(&mut slot, 1, false);
    let _ = slot.drain_host_interacts();
    slot.seat_guardian_hold(true);
    let dropped_before = dropped(&slot);
    for tick in 2..=143 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let paints = slot.probe("globalThis.__p").unwrap();
    let yaws: Vec<_> = held_rows(&mut slot)
        .into_iter()
        .filter(|req| matches!(req, InteractReq::SetCameraYaw { .. }))
        .collect();
    let dropped = dropped(&slot) - dropped_before;
    slot.stop();
    assert_eq!(paints, serde_json::json!(143));
    assert_eq!(
        (yaws, dropped),
        (vec![InteractReq::SetCameraYaw { yaw: 142 }], 0)
    );
}

#[test]
fn held_fold_keeps_a_wait_parked_after_an_encoder_overflow() {
    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
globalThis.__paint_count = 0;
globalThis.__hold_wait_parked = false;
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        Execution.noteProgress();
        Execution.noteProgress();
        await Promise.all(Array.from({length: 142}, () => Execution.delayTicks(1)));
    }
    onPaint() {
        const n = ++globalThis.__paint_count;
        if (n === 3) {
            globalThis.__hold_wait_parked = true;
            Execution.delay(600000).then(() => { globalThis.__hold_wait_parked = false; });
        }
    }
}
"#;
    let mut slot = paint_slot(source, 860);
    let now = Instant::now();
    paint_tick(&mut slot, 1, false);
    let _ = slot.drain_host_interacts();
    let initial = slot.drain_lifecycle();
    slot.feed_watchdog(now, None, &[], false, true, &initial);
    assert!(slot.watchdog().wait_active());
    // This normal tick settles the old waits before the guardian edge. Its
    // output is still in isolate custody when the host starts the hold.
    paint_tick(&mut slot, 2, false);
    let older = held_rows(&mut slot);
    slot.seat_guardian_hold(true);
    let _ = slot.drain_host_interacts();
    paint_tick(&mut slot, 3, true);
    let pages = take_batches(&mut slot);
    let newer: Vec<_> = pages
        .last()
        .unwrap()
        .batch
        .reqs
        .iter()
        .map(|row| row.request.clone())
        .collect();
    restore_batches(&mut slot, pages);
    let _ = slot.drain_host_interacts();
    assert_eq!(
        slot.probe("globalThis.__hold_wait_parked").unwrap(),
        serde_json::json!(true)
    );
    let mut reference = script::watchdog::ProgressWatchdog::new();
    for req in initial.iter().chain(&older).chain(&newer) {
        match req {
            InteractReq::WaitEnqueued => reference.on_wait_enqueued(now),
            InteractReq::WaitSettled => reference.on_wait_settled(now),
            _ => {}
        }
    }
    assert!(
        reference.wait_active(),
        "the delivered order leaves one wait"
    );
    slot.seat_guardian_hold(false);
    let _ = slot.drain_host_interacts();
    let folded = slot.drain_lifecycle();
    slot.feed_watchdog(now, None, &[], false, true, &folded);
    let count = slot.watchdog().wait_inflight();
    slot.stop();
    assert_eq!(
        count,
        reference.wait_inflight(),
        "the fold reproduces the event-order count of the real producer"
    );
}

#[test]
fn held_onpaint_refused_posts_reach_the_sender_once() {
    let source = r#"
globalThis.__p = 0;
globalThis.__errors = [];
export default class T extends LoopingBot {
    loop() {}
    onPaint() {
        if (!globalThis.__ch) {
            globalThis.__ch = new BroadcastChannel('jivekq:a,b');
            globalThis.__ch.onmessageerror = (event) => globalThis.__errors.push(event.error);
        }
        globalThis.__ch.postMessage({ n: globalThis.__p++ });
    }
}
"#;
    let mut slot = paint_slot(source, 862);
    // No host drain runs before the hold, so the hold carries the first
    // paint's open and post too.
    paint_tick(&mut slot, 1, false);
    slot.seat_guardian_hold(true);
    let dropped_before = dropped(&slot);
    for tick in 2..=170 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let errors = slot.probe("globalThis.__errors").unwrap();
    let all_posts = slot.probe("globalThis.__p").unwrap().as_u64().unwrap();
    let rows = held_rows(&mut slot);
    let posts = rows
        .iter()
        .filter(|req| matches!(req, InteractReq::ChannelPost { .. }))
        .count();
    let other_lane: Vec<_> = rows
        .iter()
        .filter(|req| {
            matches!(
                req,
                InteractReq::ChannelOpen { .. }
                    | InteractReq::ChannelClose { .. }
                    | InteractReq::InspectRoute { .. }
            )
        })
        .cloned()
        .collect();
    let dropped = dropped(&slot) - dropped_before;
    slot.stop();
    assert_eq!(
        errors,
        serde_json::json!(["BroadcastChannel post dropped while the session was held"]),
        "one status for the refused posts of this hold"
    );
    assert_eq!(
        other_lane,
        [InteractReq::ChannelOpen {
            channel_id: 1,
            name: "jivekq:a,b".into(),
        }],
        "the open is never refused"
    );
    assert_eq!(
        posts + other_lane.len(),
        api::script_contract::MAX_ENCODED_GAMEPLAY - 7,
        "the lane fills to its limit"
    );
    assert_eq!(
        dropped,
        all_posts - posts as u64,
        "each refused post is counted"
    );
}

#[test]
fn held_onpaint_resume_delivers_the_late_close() {
    let source = r#"
globalThis.__paints = 0;
export default class T extends LoopingBot {
    loop() {}
    onPaint() {
        const n = ++globalThis.__paints;
        if (n <= 141) {
            const ch = new BroadcastChannel('jivekq:a,b');
            if (n === 1) globalThis.__first = ch;
        } else if (n === 142) {
            globalThis.__first.close();
        }
    }
}
"#;
    let mut slot = paint_slot(source, 883);
    paint_tick(&mut slot, 1, false);
    slot.seat_guardian_hold(true);
    for tick in 2..=142 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let paints = slot.probe("globalThis.__paints").unwrap();
    let held = held_rows(&mut slot);
    let before = dropped(&slot);
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    let after = dropped(&slot);
    slot.stop();
    assert_eq!(paints, serde_json::json!(142));
    let lane = |rows: &[InteractReq]| {
        rows.iter()
            .filter(|req| {
                matches!(
                    req,
                    InteractReq::ChannelOpen { .. } | InteractReq::ChannelClose { .. }
                )
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let held_lane = lane(&held);
    assert_eq!(held_lane.len(), 142, "141 opens and the close are held");
    assert!(matches!(
        held_lane.last(),
        Some(InteractReq::ChannelClose { channel_id: 1, .. })
    ));
    assert_eq!(
        lane(&requests(reqs)),
        held_lane,
        "every open and the close reach the host, in order"
    );
    assert_eq!(after, before, "resume drops nothing");
}

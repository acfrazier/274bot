//! Guardian-hold fold: releases are never dropped, replacements keep the
//! newest value, wait facts fold into an exact summary, and the ordered
//! channel/inspect lane refuses only `ChannelPost` rows (round 5). Resume
//! hands the folded batch to the seat in pages that one take carries
//! whole, so `GameplaySeat::take` clears none of its rows (round 6).

use super::*;
use api::run_policy::{RunEnergyMin, RunPolicyOverride};

fn row(request: InteractReq) -> SeatInteractDecoded {
    SeatInteractDecoded {
        lineage: None,
        mirror_owner: None,
        request,
    }
}

fn page(slot: &SlotScript, reqs: Vec<InteractReq>) -> crate::load::QueuedSeatBatch {
    crate::load::QueuedSeatBatch {
        generation: slot.load.as_ref().unwrap().generation(),
        observed_walk_outcome_seq: 0,
        batch: decoded(Vec::new(), reqs.into_iter().map(row).collect()),
        machine_events: Vec::new(),
        held_waits: crate::load::HeldWaitFacts::default(),
        held_fold: false,
    }
}

/// One held host frame: the new page arrives behind the held batch, as the
/// isolate queues it, and the host drains the frame.
fn held_frame(slot: &mut SlotScript, reqs: Vec<InteractReq>) {
    let mut pages = slot.load.as_ref().unwrap().drain_seat_batches();
    pages.push(page(slot, reqs));
    slot.load.as_ref().unwrap().restore_seat_batches(pages);
    let _ = slot.drain_host_interacts();
}

fn held_rows(slot: &SlotScript) -> Vec<InteractReq> {
    let pages = slot.load.as_ref().unwrap().drain_seat_batches();
    let rows = pages
        .iter()
        .flat_map(|page| page.batch.reqs.iter().map(|row| row.request.clone()))
        .collect();
    slot.load.as_ref().unwrap().restore_seat_batches(pages);
    rows
}

fn floor(energy: i32) -> InteractReq {
    InteractReq::RunPolicyOverride {
        policy: Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(energy)),
        }),
    }
}

fn resumed_policy(slot: &mut SlotScript) -> Option<Option<RunPolicyOverride>> {
    slot.seat_guardian_hold(false);
    let (policy, _, _) = slot.drain_host_interacts();
    policy
}

// R4-M1 (Codex): `clear, run_auto=false, clear` must end at `clear`.
#[test]
fn held_policy_clear_override_clear_ends_at_the_last_clear() {
    let mut slot = with_seat(850);
    slot.seat_guardian_hold(true);
    let run_off = Some(RunPolicyOverride {
        run_auto: Some(false),
        energy_min: None,
    });
    for policy in [None, run_off, None] {
        held_frame(&mut slot, vec![InteractReq::RunPolicyOverride { policy }]);
    }
    let policy = resumed_policy(&mut slot);
    slot.stop();
    assert_eq!(policy, Some(None), "the last clear replaces the override");
}

// R4-M2 (Grok): floors 50, 0, 50 must end at 50.
#[test]
fn held_policy_returning_to_an_earlier_floor_keeps_the_newest() {
    let mut slot = with_seat(851);
    slot.seat_guardian_hold(true);
    for energy in [50, 0, 50] {
        held_frame(&mut slot, vec![floor(energy)]);
    }
    assert_eq!(held_rows(&slot), [floor(50)], "one policy row: the newest");
    let policy = resumed_policy(&mut slot);
    slot.stop();
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(50))
    );
}

// R4-M1 (Codex): 18 held floors must end at the eighteenth.
#[test]
fn held_policy_eighteen_floors_end_at_the_last() {
    let mut slot = with_seat(852);
    slot.seat_guardian_hold(true);
    for energy in 0..18 {
        held_frame(&mut slot, vec![floor(energy)]);
    }
    let policy = resumed_policy(&mut slot);
    slot.stop();
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(17))
    );
}

// R4-M1 (Grok): Stop releases after 17 distinct floors must survive.
#[test]
fn held_stop_releases_survive_any_number_of_policy_rows() {
    let mut slot = with_seat(853);
    slot.seat_guardian_hold(true);
    let dropped_before = slot.seat.as_ref().unwrap().dropped();
    for energy in 0..17 {
        held_frame(&mut slot, vec![floor(energy)]);
    }
    held_frame(
        &mut slot,
        vec![
            InteractReq::GatherStop { request_id: 99 },
            InteractReq::CombatStop { request_id: 98 },
            InteractReq::AbortWalk { request_id: 7 },
        ],
    );
    // A repeated release of the same target and id is idempotent.
    held_frame(
        &mut slot,
        vec![
            InteractReq::GatherStop { request_id: 99 },
            InteractReq::AbortWalk { request_id: 7 },
        ],
    );
    let rows = held_rows(&slot);
    let dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    slot.stop();
    assert_eq!(
        rows,
        [
            InteractReq::GatherStop { request_id: 99 },
            InteractReq::CombatStop { request_id: 98 },
            InteractReq::AbortWalk { request_id: 7 },
            floor(16),
        ]
    );
    assert_eq!(dropped, 0, "no release or control is discarded");
}

// R4-M3 (Codex): a 142-yaw hold ends at yaw 141 with nothing dropped.
#[test]
fn held_camera_yaws_end_at_the_last_yaw_without_a_hold_cap() {
    let mut slot = with_seat(854);
    slot.seat_guardian_hold(true);
    let dropped_before = slot.seat.as_ref().unwrap().dropped();
    for yaw in 0..142 {
        held_frame(&mut slot, vec![InteractReq::SetCameraYaw { yaw }]);
    }
    assert_eq!(held_rows(&slot), [InteractReq::SetCameraYaw { yaw: 141 }]);
    assert_eq!(slot.seat.as_ref().unwrap().dropped(), dropped_before);
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    slot.stop();
    assert_eq!(
        reqs.into_iter()
            .map(|queued| queued.req)
            .collect::<Vec<_>>(),
        [InteractReq::SetCameraYaw { yaw: 141 }],
        "the host applies the yaw that the producer set last"
    );
}

// R4-L2 (Grok): the whole-hold cap kept the oldest rows and lost the newest.
#[test]
fn held_late_yaw_survives_after_many_rows_and_an_early_release() {
    let mut slot = with_seat(855);
    slot.seat_guardian_hold(true);
    held_frame(&mut slot, vec![InteractReq::AbortWalk { request_id: 5 }]);
    for yaw in 0..142 {
        held_frame(&mut slot, vec![InteractReq::SetCameraYaw { yaw }]);
    }
    held_frame(&mut slot, vec![InteractReq::SetCameraYaw { yaw: 500 }]);
    let rows = held_rows(&slot);
    slot.stop();
    assert_eq!(
        rows,
        [
            InteractReq::AbortWalk { request_id: 5 },
            InteractReq::SetCameraYaw { yaw: 500 },
        ]
    );
}

/// Applies `facts` to a fresh watchdog whose wait count starts at `start`.
fn watchdog_count(start: u32, facts: impl IntoIterator<Item = InteractReq>) -> u32 {
    let now = Instant::now();
    let mut watchdog = crate::watchdog::ProgressWatchdog::new();
    for _ in 0..start {
        watchdog.on_wait_enqueued(now);
    }
    for fact in facts {
        match fact {
            InteractReq::WaitEnqueued => watchdog.on_wait_enqueued(now),
            InteractReq::WaitSettled => watchdog.on_wait_settled(now),
            _ => {}
        }
    }
    watchdog.wait_inflight()
}

fn wait_fact(enqueue: bool) -> InteractReq {
    if enqueue {
        InteractReq::WaitEnqueued
    } else {
        InteractReq::WaitSettled
    }
}

/// SplitMix64: a fixed-seed generator, so a failure reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// Property: for every event sequence and every starting count, replaying
/// the summary gives the count of the direct event-order replay. The pages
/// fold through `then`, as successive held frames do.
#[test]
fn held_wait_summary_matches_the_event_order_replay() {
    let summarize = |events: &[bool], cuts: &[usize]| {
        let mut summary = crate::load::HeldWaitFacts::default();
        let mut from = 0;
        for &to in cuts.iter().chain(std::iter::once(&events.len())) {
            let mut page = crate::load::HeldWaitFacts::default();
            for &enqueue in &events[from..to] {
                if enqueue {
                    page.enqueue();
                } else {
                    page.settle();
                }
            }
            summary = summary.then(page);
            from = to;
        }
        summary
    };
    let mut cases = 0u64;
    // Exhaustive: every sequence of up to 10 facts, two page splits each.
    for len in 0..=10usize {
        for bits in 0u32..(1 << len) {
            let events: Vec<bool> = (0..len).map(|i| bits & (1 << i) != 0).collect();
            for cuts in [vec![], vec![len / 2]] {
                let summary = summarize(&events, &cuts);
                assert_eq!(summary.stamped, len > 0);
                assert_eq!(summary.rows(), summary.replay().count() as u64);
                for start in 0..=6 {
                    let direct = watchdog_count(start, events.iter().map(|&e| wait_fact(e)));
                    let folded = watchdog_count(start, summary.replay());
                    assert_eq!(
                        folded, direct,
                        "start {start}, events {events:?}, cuts {cuts:?}, summary {summary:?}"
                    );
                    cases += 1;
                }
            }
        }
    }
    // Random: long sequences, random pages, random starting counts.
    let mut rng = Rng(0x7135_B005);
    for _ in 0..20_000 {
        let len = rng.below(300) as usize;
        let bias = rng.below(9) + 1;
        let events: Vec<bool> = (0..len).map(|_| rng.below(10) < bias).collect();
        let mut cuts: Vec<usize> = (0..rng.below(6))
            .map(|_| rng.below(len as u64 + 1) as usize)
            .collect();
        cuts.sort_unstable();
        let summary = summarize(&events, &cuts);
        let start = rng.below(200) as u32;
        assert_eq!(
            watchdog_count(start, summary.replay()),
            watchdog_count(start, events.iter().map(|&e| wait_fact(e))),
            "start {start}, len {len}, cuts {cuts:?}, summary {summary:?}"
        );
        cases += 1;
    }
    eprintln!("held wait summary property: {cases} cases passed");
}

/// The same property through the production path: held pages fold in
/// `drain_host_interacts`, the hold lifts, and `feed_watchdog` applies the
/// released facts to a watchdog that held a random count before the hold.
#[test]
fn held_wait_fold_matches_the_event_order_replay_through_the_slot() {
    let mut slot = with_seat(856);
    let mut rng = Rng(0x0005_1077);
    for case in 0..300 {
        let start = rng.below(8) as u32;
        let now = Instant::now();
        slot.watchdog = crate::watchdog::ProgressWatchdog::new();
        let before = vec![InteractReq::WaitEnqueued; start as usize];
        slot.feed_watchdog(now, None, &[], false, true, &before);
        assert_eq!(slot.watchdog.wait_inflight(), start);
        slot.seat_guardian_hold(true);
        let mut events = Vec::new();
        for _ in 0..rng.below(8) {
            let mut reqs = Vec::new();
            for _ in 0..rng.below(6) {
                let fact = wait_fact(rng.below(2) == 0);
                events.push(fact.clone());
                reqs.push(fact);
                if rng.below(4) == 0 {
                    reqs.push(InteractReq::NoteProgress);
                }
            }
            held_frame(&mut slot, reqs);
        }
        slot.seat_guardian_hold(false);
        let _ = slot.drain_host_interacts();
        let facts = slot.drain_lifecycle();
        slot.feed_watchdog(now, None, &[], false, true, &facts);
        assert_eq!(
            slot.watchdog.wait_inflight(),
            watchdog_count(start, events.iter().cloned()),
            "case {case}: start {start}, events {events:?}, released {facts:?}"
        );
    }
    slot.stop();
}

// R4-L1 (Grok) and R4-M2 (Codex): a settle before an enqueue saturates at
// zero; the later enqueue must stay counted.
#[test]
fn held_settle_then_enqueue_keeps_the_later_wait() {
    let mut slot = with_seat(857);
    slot.seat_guardian_hold(true);
    held_frame(&mut slot, vec![InteractReq::WaitSettled]);
    held_frame(&mut slot, vec![InteractReq::WaitEnqueued]);
    slot.seat_guardian_hold(false);
    let _ = slot.drain_host_interacts();
    let facts = slot.drain_lifecycle();
    slot.feed_watchdog(Instant::now(), None, &[], false, true, &facts);
    let active = slot.watchdog().wait_active();
    slot.stop();
    assert_eq!(
        facts,
        [InteractReq::WaitSettled, InteractReq::WaitEnqueued],
        "the summary keeps the saturating order"
    );
    assert!(
        active,
        "the enqueue after the saturated settle stays counted"
    );
}

fn paint_slot(source: &str, incarnation: u64) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    slot.start_load_with_settings_and_game_data(
        source.into(),
        crate::load::LoadShape::CompatClass,
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

/// Post an in-game snapshot and run one isolate tick to completion.
fn paint_tick(slot: &mut SlotScript, tick: u64, held: bool) {
    let mut input = crate::isolate_fb::tests::empty_input(tick);
    input.hold = held;
    let stats: Vec<_> = (0..64)
        .filter(|&i| api::snapshot::stat_used(i))
        .map(|index| crate::isolate_fb::StatInput {
            index: index as i32,
            name: "skill",
            xp: 0,
            base: 1,
            effective: 1,
        })
        .collect();
    input.stats = &stats;
    input.scene_state = 2;
    input.here = Some(crate::isolate_fb::TileInput {
        x: 3187,
        z: 3245,
        level: 0,
    });
    let bytes = slot.encode_snapshot_delta(&input, false);
    assert!(slot.post_snapshot(bytes));
    let isolate = slot.load.as_ref().unwrap();
    isolate.on_game_tick(tick);
    isolate.probe("true").unwrap();
}

// R4-M1 (Codex), real producer: 18 held `onPaint` floors end at Floor(18).
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
    let policy = resumed_policy(&mut slot);
    slot.stop();
    assert_eq!(paints, serde_json::json!(19), "the producer ran all paints");
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(18))
    );
}

// R4-M3 (Codex), real producer: 142 held `onPaint` yaws keep the last yaw.
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
    let dropped_before = slot.seat.as_ref().unwrap().dropped();
    for tick in 2..=143 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let paints = slot.probe("globalThis.__p").unwrap();
    let yaws: Vec<_> = held_rows(&slot)
        .into_iter()
        .filter(|req| matches!(req, InteractReq::SetCameraYaw { .. }))
        .collect();
    let dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    slot.stop();
    assert_eq!(paints, serde_json::json!(143));
    assert_eq!(
        (yaws, dropped),
        (vec![InteractReq::SetCameraYaw { yaw: 142 }], 0)
    );
}

// R4-M2 (Codex), real producer: old settles and an encoder overflow before
// the hold, then a new parked wait in a held `onPaint`.
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
    let older = held_rows(&slot);
    slot.seat_guardian_hold(true);
    let _ = slot.drain_host_interacts();
    paint_tick(&mut slot, 3, true);
    let pages = slot.load.as_ref().unwrap().drain_seat_batches();
    let newer: Vec<_> = pages
        .last()
        .unwrap()
        .batch
        .reqs
        .iter()
        .map(|row| row.request.clone())
        .collect();
    slot.load.as_ref().unwrap().restore_seat_batches(pages);
    let _ = slot.drain_host_interacts();
    assert_eq!(
        slot.probe("globalThis.__hold_wait_parked").unwrap(),
        serde_json::json!(true)
    );
    let mut reference = crate::watchdog::ProgressWatchdog::new();
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
    let count = slot.watchdog.wait_inflight();
    slot.stop();
    assert_eq!(
        count,
        reference.wait_inflight(),
        "the fold reproduces the event-order count of the real producer"
    );
}

fn channel_post(channel_id: u64, n: u8) -> InteractReq {
    InteractReq::ChannelPost {
        channel_id,
        name: "jivekq:a,b".into(),
        data: vec![n],
    }
}

fn inspect(request_id: u64) -> InteractReq {
    InteractReq::InspectRoute {
        x: 3200,
        z: 3200,
        level: 0,
        from_x: 3187,
        from_z: 3245,
        from_level: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        avoid: Vec::new(),
        cross: Vec::new(),
        request_id,
    }
}

// Main ruling 2-3: the channel/inspect lane keeps its order, never drops an
// open, a close or an inspect, and refuses only later posts. Each refused
// post is counted. The replacements follow the lane (round-6 ruling), and
// resume drops nothing more.
#[test]
fn held_channel_lane_refuses_only_posts_and_keeps_order() {
    let mut slot = with_seat(861);
    slot.seat_guardian_hold(true);
    let dropped_before = slot.seat.as_ref().unwrap().dropped();
    let open = InteractReq::ChannelOpen {
        channel_id: 3,
        name: "jivekq:a,b".into(),
    };
    let close = InteractReq::ChannelClose {
        channel_id: 3,
        name: "jivekq:a,b".into(),
    };
    held_frame(&mut slot, vec![open.clone(), InteractReq::NoteProgress]);
    for n in 0..200u8 {
        held_frame(
            &mut slot,
            vec![
                channel_post(3, n),
                InteractReq::SetCameraYaw { yaw: n.into() },
            ],
        );
    }
    held_frame(&mut slot, vec![inspect(41), close.clone(), inspect(0)]);
    let rows = held_rows(&slot);
    let dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    let lane_limit = HELD_LIFECYCLE_ROWS - HeldReplacement::KEYS;
    let posts = rows
        .iter()
        .filter(|req| matches!(req, InteractReq::ChannelPost { .. }))
        .count();
    assert_eq!(posts, lane_limit - 1, "the open takes one lane row");
    assert_eq!(
        dropped as usize,
        200 - posts,
        "each refused post is counted"
    );
    assert_eq!(rows[0], open);
    assert_eq!(rows[1], channel_post(3, 0));
    assert_eq!(rows[posts], channel_post(3, (posts - 1) as u8));
    assert_eq!(
        rows[posts + 1..],
        [
            inspect(41),
            close,
            inspect(0),
            InteractReq::NoteProgress,
            InteractReq::SetCameraYaw { yaw: 199 },
        ]
    );
    assert!(rows.len() > HELD_LIFECYCLE_ROWS - HeldReplacement::KEYS);
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    let delivered: Vec<_> = reqs.into_iter().map(|queued| queued.req).collect();
    let seat_dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    slot.stop();
    assert_eq!(seat_dropped, dropped, "resume drops nothing more");
    assert_eq!(
        delivered,
        rows.into_iter()
            .filter(|req| *req != InteractReq::NoteProgress)
            .collect::<Vec<_>>()
    );
}

// Main ruling 2b: a refused post is not silent. The sender's channel gets one
// `messageerror` with the hold reason in that hold, however many posts the
// hold refuses.
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
    let dropped_before = slot.seat.as_ref().unwrap().dropped();
    for tick in 2..=170 {
        paint_tick(&mut slot, tick, true);
        let _ = slot.drain_host_interacts();
    }
    let errors = slot.probe("globalThis.__errors").unwrap();
    let all_posts = slot.probe("globalThis.__p").unwrap().as_u64().unwrap();
    let rows = held_rows(&slot);
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
    let dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    slot.stop();
    assert_eq!(
        errors,
        serde_json::json!([HELD_CHANNEL_POST_REFUSED]),
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
        HELD_LIFECYCLE_ROWS - HeldReplacement::KEYS,
        "the lane fills to its limit"
    );
    assert_eq!(
        dropped,
        all_posts - posts as u64,
        "each refused post is counted"
    );
}

fn lane_open(channel_id: u64) -> InteractReq {
    InteractReq::ChannelOpen {
        channel_id,
        name: "jivekq:a,b".into(),
    }
}

fn lane_close(channel_id: u64) -> InteractReq {
    InteractReq::ChannelClose {
        channel_id,
        name: "jivekq:a,b".into(),
    }
}

fn requests(reqs: Vec<crate::load::QueuedInteract>) -> Vec<InteractReq> {
    reqs.into_iter().map(|queued| queued.req).collect()
}

fn dropped(slot: &SlotScript) -> u64 {
    slot.seat.as_ref().unwrap().dropped()
}

/// One row of each `HeldReplacement` key.
fn every_replacement() -> Vec<InteractReq> {
    vec![
        InteractReq::SetCameraYaw { yaw: 500 },
        InteractReq::RecoveryAnchor {
            x: 3200,
            z: 3200,
            level: 0,
        },
        InteractReq::InspectAck {
            seq: 3,
            generation: 1,
        },
        InteractReq::ChannelMessage {
            channel_id: 9,
            sender: "peer".into(),
            seq: 1,
            data: vec![1],
        },
        InteractReq::ChannelStatus {
            channel_id: 9,
            message: "status".into(),
        },
        InteractReq::NoteProgress,
        InteractReq::LoopSettled,
    ]
}

/// Rows that `dispatch_seat_rows` hands to the watchdog lane, not to the
/// host interact delivery.
fn watchdog_row(req: &InteractReq) -> bool {
    matches!(
        req,
        InteractReq::NoteProgress
            | InteractReq::LoopSettled
            | InteractReq::WaitEnqueued
            | InteractReq::WaitSettled
            | InteractReq::RecoveryAnchor { .. }
            | InteractReq::RecoveryAnchorNone
    )
}

// R5-B1 (Codex): 141 held previews, then an authorized inspect. Resume must
// deliver the token after the previews, and drop nothing.
#[test]
fn held_resume_delivers_an_authorized_inspect_after_141_previews() {
    let mut slot = with_seat(880);
    slot.seat_guardian_hold(true);
    for _ in 0..141 {
        held_frame(&mut slot, vec![inspect(0)]);
    }
    held_frame(&mut slot, vec![inspect(41)]);
    let rows = held_rows(&slot);
    let before = dropped(&slot);
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    let after = dropped(&slot);
    slot.stop();
    assert_eq!(
        rows.len(),
        142,
        "the hold retains every preview and the token"
    );
    assert_eq!(requests(reqs), rows, "every retained row, in order");
    assert_eq!(after, before, "resume drops nothing");
}

// R5-B1 (Grok, case 1): 141 held opens, then yaw 500, a policy, two
// releases, a close, an inspect, an ack and an anchor. Resume must deliver
// all of them, with the latest replacement values after the lane.
#[test]
fn held_resume_delivers_late_replacements_after_141_opens() {
    let mut slot = with_seat(881);
    slot.seat_guardian_hold(true);
    for channel_id in 1..=141 {
        held_frame(&mut slot, vec![lane_open(channel_id)]);
    }
    let ack = InteractReq::InspectAck {
        seq: 3,
        generation: 1,
    };
    let anchor = InteractReq::RecoveryAnchor {
        x: 3200,
        z: 3200,
        level: 0,
    };
    held_frame(
        &mut slot,
        vec![
            InteractReq::SetCameraYaw { yaw: 500 },
            floor(42),
            InteractReq::GatherStop { request_id: 77 },
            InteractReq::AbortWalk { request_id: 8 },
            lane_close(1),
            inspect(41),
            ack.clone(),
            anchor.clone(),
        ],
    );
    let before = dropped(&slot);
    slot.seat_guardian_hold(false);
    let (policy, reqs, _) = slot.drain_host_interacts();
    let watchdog = std::mem::take(&mut slot.seat_lifecycle);
    let after = dropped(&slot);
    slot.stop();
    let mut expected = vec![InteractReq::AbortWalk { request_id: 8 }];
    expected.extend((1..=141).map(lane_open));
    expected.extend([
        lane_close(1),
        inspect(41),
        InteractReq::SetCameraYaw { yaw: 500 },
        ack,
    ]);
    assert_eq!(
        requests(reqs),
        expected,
        "the late yaw, close, inspect and ack all reach the host, after the lane"
    );
    assert_eq!(watchdog, [anchor], "the anchor reaches the watchdog lane");
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(42))
    );
    assert_eq!(after, before, "resume drops nothing");
}

// R5-B1 (Grok, case 2): all seven replacement keys and a full post lane,
// then a close and an inspect. The hold refuses only the later posts; resume
// must lose the close and the inspect no more.
#[test]
fn held_resume_keeps_close_and_inspect_after_a_full_post_lane_and_every_replacement() {
    let mut slot = with_seat(882);
    slot.seat_guardian_hold(true);
    let start = dropped(&slot);
    held_frame(&mut slot, every_replacement());
    held_frame(&mut slot, vec![lane_open(3)]);
    for n in 0..160u8 {
        held_frame(&mut slot, vec![channel_post(3, n)]);
    }
    held_frame(&mut slot, vec![lane_close(3), inspect(41)]);
    let rows = held_rows(&slot);
    let refused = dropped(&slot) - start;
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    let after = dropped(&slot) - start;
    slot.stop();
    assert_eq!(
        rows.len(),
        143,
        "7 replacements, the open, 133 posts, close, inspect"
    );
    assert_eq!(refused, 27, "only the later posts are refused, and counted");
    let delivered = requests(reqs);
    assert_eq!(
        delivered,
        rows.into_iter()
            .filter(|req| !watchdog_row(req))
            .collect::<Vec<_>>(),
        "every retained row reaches the host, in order"
    );
    assert!(delivered.contains(&lane_close(3)));
    assert!(delivered.contains(&inspect(41)));
    assert_eq!(after, refused, "resume adds no drop to the refused posts");
}

// R5-B1 (Codex), real producer: a held `onPaint` opens 141 channels, then
// closes the first. Resume must deliver the close.
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
    let held = held_rows(&slot);
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

// Round-6 ruling: resume delivers every retained row across several takes,
// in order, with `dropped` unchanged. Releases and the policy go first, the
// lane follows in order, and the replacements come last.
#[test]
fn held_resume_delivers_every_retained_row_across_takes_in_order() {
    let mut slot = with_seat(884);
    slot.seat_guardian_hold(true);
    held_frame(
        &mut slot,
        vec![InteractReq::AbortWalk { request_id: 5 }, floor(10)],
    );
    for channel_id in 1..=150u64 {
        held_frame(
            &mut slot,
            vec![
                lane_open(channel_id),
                InteractReq::SetCameraYaw {
                    yaw: i32::try_from(channel_id).unwrap(),
                },
            ],
        );
    }
    for channel_id in 1..=150 {
        held_frame(&mut slot, vec![inspect(0), lane_close(channel_id)]);
    }
    held_frame(
        &mut slot,
        vec![
            inspect(41),
            InteractReq::InspectAck {
                seq: 2,
                generation: 1,
            },
        ],
    );
    let rows = held_rows(&slot);
    let before = dropped(&slot);
    slot.seat_guardian_hold(false);
    let (policy, reqs, _) = slot.drain_host_interacts();
    let after = dropped(&slot);
    slot.stop();
    let mut expected = vec![InteractReq::AbortWalk { request_id: 5 }];
    expected.extend((1..=150).map(lane_open));
    for channel_id in 1..=150 {
        expected.extend([inspect(0), lane_close(channel_id)]);
    }
    expected.extend([
        inspect(41),
        InteractReq::SetCameraYaw { yaw: 150 },
        InteractReq::InspectAck {
            seq: 2,
            generation: 1,
        },
    ]);
    assert!(
        expected.len() - 1 > 3 * api::script_contract::MAX_ENCODED_GAMEPLAY,
        "more non-reserved rows than three takes carry"
    );
    assert_eq!(after, before, "resume drops nothing");
    assert_eq!(requests(reqs), expected, "every retained row, in order");
    assert_eq!(
        rows.iter()
            .filter(|req| !matches!(req, InteractReq::RunPolicyOverride { .. }))
            .cloned()
            .collect::<Vec<_>>(),
        expected,
        "the fold keeps releases, the lane in order, then the replacements"
    );
    assert_eq!(
        policy.unwrap().unwrap().energy_min,
        Some(RunEnergyMin::Floor(10))
    );
}

// Round-6 ruling: a page holds only what the next take carries. Rows that
// already wait in the seat leave no room, so the retained batch waits for
// the next frame, and a batch produced after resume queues behind it.
#[test]
fn held_resume_pages_around_queued_rows_and_later_batches_queue_behind() {
    let mut slot = with_seat(885);
    slot.seat_guardian_hold(true);
    for channel_id in 1..=200 {
        held_frame(&mut slot, vec![lane_open(channel_id)]);
    }
    let rows = held_rows(&slot);
    slot.seat_guardian_hold(false);
    let waiting: Vec<_> = (1..=141)
        .map(|yaw| InteractReq::SetCameraYaw { yaw })
        .collect();
    for req in &waiting {
        slot.seat
            .as_mut()
            .unwrap()
            .push(
                SeatEffect::Interact(crate::load::QueuedInteract {
                    req: req.clone(),
                    observed_walk_outcome_seq: 0,
                }),
                api::script_contract::EmissionClass::Lifecycle,
                EmissionSource::Root,
                None,
            )
            .unwrap();
    }
    let mut pages = slot.load.as_ref().unwrap().drain_seat_batches();
    pages.push(page(&slot, vec![lane_open(999)]));
    slot.load.as_ref().unwrap().restore_seat_batches(pages);
    let before = dropped(&slot);
    let (_, first, _) = slot.drain_host_interacts();
    let (_, second, _) = slot.drain_host_interacts();
    let after = dropped(&slot);
    slot.stop();
    assert_eq!(
        requests(first),
        waiting,
        "the waiting rows take the first take"
    );
    let mut expected = rows;
    expected.push(lane_open(999));
    assert_eq!(
        requests(second),
        expected,
        "the retained rows follow, and the later batch queues behind them"
    );
    assert_eq!(after, before, "resume drops nothing");
}

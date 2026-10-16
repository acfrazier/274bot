//! Round-5 guardian-hold fold: releases are never dropped, replacements keep
//! the newest value, wait facts fold into an exact summary, and the ordered
//! channel/inspect lane refuses only `ChannelPost` rows.

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
// post is counted, and the whole fold fits one seat take after resume.
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
    assert_eq!(rows[1], InteractReq::NoteProgress);
    assert_eq!(rows[2], channel_post(3, 0));
    assert_eq!(rows[3], InteractReq::SetCameraYaw { yaw: 199 });
    assert_eq!(rows[rows.len() - 4], channel_post(3, (posts - 1) as u8));
    assert_eq!(rows[rows.len() - 3..], [inspect(41), close, inspect(0)]);
    assert!(rows.len() > HELD_LIFECYCLE_ROWS - HeldReplacement::KEYS);
    slot.seat_guardian_hold(false);
    let (_, reqs, _) = slot.drain_host_interacts();
    let delivered: Vec<_> = reqs.into_iter().map(|queued| queued.req).collect();
    let seat_dropped = slot.seat.as_ref().unwrap().dropped() - dropped_before;
    slot.stop();
    assert_eq!(
        seat_dropped, dropped,
        "the folded batch fits one seat take: resume drops nothing more"
    );
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

//! Guardian-hold fold: releases are never dropped, replacements keep the
//! newest value, wait facts fold into an exact summary, and the ordered
//! channel/inspect lane refuses only `ChannelPost` rows (round 5). Resume
//! hands the folded batch to the seat in pages that one take carries
//! whole, so `GameplaySeat::take` clears none of its rows (round 6). The
//! releases go to the seat whatever the room, and a second hold keeps the
//! controls of a fold whose pages still wait (round 7). That merge keeps a
//! machine phase that returns to an earlier value, and carries each isolate
//! repeat once (round 8). A held tick keeps the walk aborts and the newest
//! run policy for the first frame after the hold (rounds 9 and 10).

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
        generation: slot.surface.as_ref().unwrap().generation(),
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
    let mut pages = slot.surface.as_ref().unwrap().drain_seat_batches();
    pages.push(page(slot, reqs));
    slot.surface.as_ref().unwrap().restore_seat_batches(pages);
    let _ = slot.drain_host_interacts();
}

fn held_rows(slot: &SlotScript) -> Vec<InteractReq> {
    let pages = slot.surface.as_ref().unwrap().drain_seat_batches();
    let rows = pages
        .iter()
        .flat_map(|page| page.batch.reqs.iter().map(|row| row.request.clone()))
        .collect();
    slot.surface.as_ref().unwrap().restore_seat_batches(pages);
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
    let source: std::sync::Arc<str> = source.into();
    let family = crate::resolve_api_family(&source)
        .ok()
        .map(|(_, family)| family);
    slot.bind_surface_factory(std::sync::Arc::new(
        crate::load::surface::JsSurfaceFactory::default(),
    ));
    slot.start_surface(crate::surface::SurfaceStart {
        card: std::sync::Arc::new(crate::load::surface::JsStartCard {
            source,
            shape: crate::load::LoadShape::CompatClass,
            siblings: std::sync::Arc::from(Vec::<(String, String)>::new()),
            family,
        }),
        settings_bag: None,
        game_data: Some(selected()),
        named_banks: std::sync::Arc::default(),
        loadouts: std::sync::Arc::default(),
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
    assert!(slot.post_snapshot_delta(
        &input,
        crate::native::snapshot_source::NativeFactsInput::default(),
        false,
    ));
    let run = slot.surface.as_ref().unwrap();
    run.on_game_tick_at(tick, 0);
    run.probe("true").unwrap();
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
    let pages = slot.surface.as_ref().unwrap().drain_seat_batches();
    let newer: Vec<_> = pages
        .last()
        .unwrap()
        .batch
        .reqs
        .iter()
        .map(|row| row.request.clone())
        .collect();
    slot.surface.as_ref().unwrap().restore_seat_batches(pages);
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
    let mut pages = slot.surface.as_ref().unwrap().drain_seat_batches();
    pages.push(page(&slot, vec![lane_open(999)]));
    slot.surface.as_ref().unwrap().restore_seat_batches(pages);
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

fn queued_interact(req: InteractReq) -> SeatEffect {
    SeatEffect::Interact(crate::load::QueuedInteract {
        req,
        observed_walk_outcome_seq: 0,
    })
}

fn walk(request_id: u64) -> InteractReq {
    InteractReq::Walk {
        x: 3200,
        z: 3200,
        level: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id,
        avoid: Vec::new(),
        cross: Vec::new(),
    }
}

/// Queue one walk row that needs the release slot of `request_id`.
fn push_seat_walk(slot: &mut SlotScript, req: InteractReq, request_id: u64) {
    slot.seat
        .as_mut()
        .unwrap()
        .push_walk(queued_interact(req), EmissionSource::Root, id(request_id))
        .unwrap();
}

/// Arm all sixteen walk release slots, ids 1 to 16, and clear what the
/// host received.
fn arm_every_walk_slot(slot: &mut SlotScript) {
    for request_id in 1..=16 {
        push_seat_walk(slot, walk(request_id), request_id);
    }
    slot.dispatch_seat_rows();
    slot.seat_interact_delivery.clear();
    assert_eq!(
        slot.seat.as_ref().unwrap().reserved_releases(),
        api::script_contract::MAX_WALK_RELEASES
    );
}

/// Drain up to `frames` host frames. Every frame while retained rows
/// remain must deliver at least one retained row. Returns the retained rows
/// delivered, in order, and the number of frames used.
fn drain_retained_with_progress(
    slot: &mut SlotScript,
    retained: &[InteractReq],
    frames: usize,
) -> (Vec<InteractReq>, usize) {
    let mut delivered = Vec::new();
    for frame in 0..frames {
        if held_rows(slot).is_empty() {
            return (delivered, frame);
        }
        let (_, reqs, _) = slot.drain_host_interacts();
        let before = delivered.len();
        delivered.extend(
            requests(reqs)
                .into_iter()
                .filter(|req| retained.contains(req)),
        );
        assert!(
            delivered.len() > before,
            "frame {frame} delivered no retained row; {} rows still retained",
            held_rows(slot).len()
        );
    }
    (delivered, frames)
}

// R7 B1 (Codex `solr6_`): all sixteen walk release slots are armed, and
// 141 queued walk rows for id 17 fill the seat. Room is 0, and `take`
// stops at the first walk, which cannot reserve a slot. The retained
// `AbortWalk(1)` frees that slot, so the pager must push it whatever the
// room. Every drain makes progress, and every retained row arrives with
// `dropped` unchanged.
#[test]
fn held_resume_pushes_the_release_that_unblocks_a_full_walk_queue() {
    let mut slot = with_seat(886);
    arm_every_walk_slot(&mut slot);
    slot.seat_guardian_hold(true);
    held_frame(&mut slot, vec![InteractReq::AbortWalk { request_id: 1 }]);
    for channel_id in 1..=142 {
        held_frame(&mut slot, vec![lane_open(channel_id)]);
    }
    slot.seat_guardian_hold(false);
    for _ in 0..api::script_contract::MAX_ENCODED_GAMEPLAY {
        push_seat_walk(
            &mut slot,
            InteractReq::WalkTo {
                x: 30,
                z: 40,
                level: 0,
            },
            17,
        );
    }
    assert_eq!(slot.seat.as_ref().unwrap().take_room(), 0);
    let mut retained = vec![InteractReq::AbortWalk { request_id: 1 }];
    retained.extend((1..=142).map(lane_open));
    assert_eq!(held_rows(&slot), retained, "the fold under test");
    let before = dropped(&slot);
    let (delivered, frames) = drain_retained_with_progress(&mut slot, &retained, 3);
    let left = held_rows(&slot);
    let after = dropped(&slot);
    slot.stop();
    assert!(left.is_empty(), "rows still retained after {frames} frames");
    assert_eq!(delivered, retained, "every retained row, in order");
    assert_eq!(after, before, "resume drops nothing");
}

// R7 B1 / R6-L1 (Grok `probe_walk_blocked_full_queue_stalls_the_unpaged_
// release`): the same blocked queue as a walk for id 17 and 140 lifecycle
// rows, with a retained fold of `AbortWalk(1)` and 30 opens.
#[test]
fn held_resume_frees_the_walk_slot_that_a_blocked_full_queue_waits_for() {
    let mut slot = with_seat(887);
    arm_every_walk_slot(&mut slot);
    push_seat_walk(&mut slot, walk(17), 17);
    for yaw in 1..=140 {
        slot.seat
            .as_mut()
            .unwrap()
            .push(
                queued_interact(InteractReq::SetCameraYaw { yaw }),
                api::script_contract::EmissionClass::Lifecycle,
                EmissionSource::Root,
                None,
            )
            .unwrap();
    }
    assert_eq!(slot.seat.as_ref().unwrap().take_room(), 0);
    let mut retained = vec![InteractReq::AbortWalk { request_id: 1 }];
    retained.extend((1..=30).map(lane_open));
    let mut fold = page(&slot, retained.clone());
    fold.held_fold = true;
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![fold]);
    let before = dropped(&slot);
    let (delivered, frames) = drain_retained_with_progress(&mut slot, &retained, 4);
    let left = held_rows(&slot);
    let after = dropped(&slot);
    slot.stop();
    assert!(left.is_empty(), "rows still retained after {frames} frames");
    assert_eq!(delivered, retained, "every retained row, in order");
    assert_eq!(after, before, "resume drops nothing");
}

// R7 R6-L2 (Grok `probe_second_hold_while_pages_wait_drops_unapplied_
// controls`): a retained fold that carries a cancel waits for room. A
// second hold folds a new batch over it. The waiting fold's cancel must
// still apply after the last page, and the entry must not stay live.
#[test]
fn held_second_hold_keeps_the_controls_of_a_waiting_fold() {
    let mut slot = with_seat(888);
    let mut admit = page(&slot, Vec::new());
    admit.batch.starts = vec![start_input(&slot, 1, NativeKind::Hunt)];
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![admit]);
    let _ = slot.drain_host_interacts();
    let live = |slot: &SlotScript| {
        slot.seat
            .as_ref()
            .unwrap()
            .live()
            .iter()
            .any(|entry| entry.id == id(1))
    };
    assert!(live(&slot), "the start is admitted");
    slot.seat_guardian_hold(true);
    let mut fold = page(&slot, (1..=20).map(lane_open).collect());
    fold.held_fold = true;
    fold.batch.cancels = vec![crate::isolate_fb::SeatCancelDecoded {
        of: id(1),
        reason: api::script_contract::CallbackCancelReason::ParentAborted,
    }];
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![fold]);
    slot.seat_guardian_hold(false);
    for yaw in 1..=141 {
        slot.seat
            .as_mut()
            .unwrap()
            .push(
                queued_interact(InteractReq::SetCameraYaw { yaw }),
                api::script_contract::EmissionClass::Lifecycle,
                EmissionSource::Root,
                None,
            )
            .unwrap();
    }
    let (_, first, _) = slot.drain_host_interacts();
    assert_eq!(first.len(), 141, "the waiting rows take the first frame");
    assert!(live(&slot), "the cancel waits with the retained fold");
    slot.seat_guardian_hold(true);
    held_frame(&mut slot, vec![lane_open(50)]);
    slot.seat_guardian_hold(false);
    let before = dropped(&slot);
    let (_, resumed, _) = slot.drain_host_interacts();
    let still_live = live(&slot);
    let after = dropped(&slot);
    slot.stop();
    let mut expected: Vec<_> = (1..=20).map(lane_open).collect();
    expected.push(lane_open(50));
    assert_eq!(requests(resumed), expected, "every retained row, in order");
    assert!(!still_live, "the waiting fold's cancel applies");
    assert_eq!(after, before, "resume drops nothing");
}

/// Queue 141 lifecycle rows, so the next take has no room for a page.
fn fill_seat_queue(slot: &mut SlotScript) {
    for yaw in 1..=141 {
        slot.seat
            .as_mut()
            .unwrap()
            .push(
                queued_interact(InteractReq::SetCameraYaw { yaw }),
                api::script_contract::EmissionClass::Lifecycle,
                EmissionSource::Root,
                None,
            )
            .unwrap();
    }
}

fn machine_event(raw: u64, kind: crate::machine::MachineEventKind) -> crate::machine::MachineEvent {
    crate::machine::MachineEvent {
        id: id(raw),
        kind,
        admitted: true,
    }
}

fn mirror_revoke(raw: u64, seq: u64) -> crate::isolate_fb::SeatMirrorRevoke {
    crate::isolate_fb::SeatMirrorRevoke {
        owner: id(raw),
        seq,
    }
}

/// Hold with a retained fold of 20 opens, then for each newer batch: resume
/// with a full seat queue, so the fold waits, queue the newer batch behind
/// it, and hold again before the fold applies. Returns the carried fold.
fn rehold_while_the_fold_waits(
    slot: &mut SlotScript,
    initial: crate::load::QueuedSeatBatch,
    newer: Vec<crate::load::QueuedSeatBatch>,
) -> crate::load::QueuedSeatBatch {
    slot.seat_guardian_hold(true);
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![initial]);
    let _ = slot.drain_host_interacts();
    for batch in newer {
        slot.seat_guardian_hold(false);
        fill_seat_queue(slot);
        let mut batches = slot.surface.as_ref().unwrap().drain_seat_batches();
        batches.push(batch);
        slot.surface.as_ref().unwrap().restore_seat_batches(batches);
        let (_, first, _) = slot.drain_host_interacts();
        assert_eq!(first.len(), 141, "the blocking rows take the frame");
        assert_eq!(held_rows(slot).len(), 20, "the fold still waits");
        slot.seat_guardian_hold(true);
        let _ = slot.drain_host_interacts();
    }
    let mut carried = slot.surface.as_ref().unwrap().drain_seat_batches();
    assert_eq!(carried.len(), 1, "one folded batch");
    carried.pop().unwrap()
}

// R8 R7-H1 (Grok `probe_r7_phase_return_across_reholds_keeps_the_newest_
// phase`): the isolate keeps one phase for each id and replaces it, so a
// newer `Waiting` after `Working` is a new transition, not a repeat. Two
// reholds must carry `Waiting, Working, Waiting`, and the seat must end at
// `Waiting`.
#[test]
fn held_rehold_keeps_a_phase_that_returns_to_an_earlier_value() {
    use crate::machine::MachineEventKind::Phase;
    use crate::native::NativePhase;
    let mut slot = with_seat(889);
    let mut admit = page(&slot, Vec::new());
    admit.batch.starts = vec![start_input(&slot, 1, NativeKind::Hunt)];
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![admit]);
    let _ = slot.drain_host_interacts();
    let mut initial = page(&slot, (1..=20).map(lane_open).collect());
    initial.machine_events = vec![machine_event(1, Phase(NativePhase::Waiting))];
    let newer = [NativePhase::Working, NativePhase::Waiting]
        .into_iter()
        .map(|phase| {
            let mut update = page(&slot, Vec::new());
            update.machine_events = vec![machine_event(1, Phase(phase))];
            update
        })
        .collect();
    let carried = rehold_while_the_fold_waits(&mut slot, initial, newer);
    let events: Vec<_> = carried
        .machine_events
        .iter()
        .map(|event| event.kind)
        .collect();
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![carried]);
    slot.seat_guardian_hold(false);
    let _ = slot.drain_host_interacts();
    let phase = slot.seat.as_ref().unwrap().machine_phase(id(1));
    slot.stop();
    assert_eq!(
        events,
        [
            Phase(NativePhase::Waiting),
            Phase(NativePhase::Working),
            Phase(NativePhase::Waiting)
        ],
        "every transition, in order"
    );
    assert_eq!(phase, Some(NativePhase::Waiting), "the newest phase");
}

// R8 R7-M1 (Codex `solr7b_phase_aba_across_waiting_reholds_keeps_newest_
// phase`): the same A-B-A sequence with the start admitted by a direct
// batch. All 20 retained opens arrive in order, and the seat ends at the
// newest phase.
#[test]
fn held_rehold_phase_a_b_a_ends_at_the_newest_phase() {
    use crate::machine::MachineEventKind::Phase;
    use crate::native::NativePhase;
    let mut slot = with_seat(890);
    slot.consume_seat_batch(
        decoded(vec![start_input(&slot, 1, NativeKind::Hunt)], Vec::new()),
        0,
        &[],
    );
    let mut initial = page(&slot, (1..=20).map(lane_open).collect());
    initial.machine_events = vec![machine_event(1, Phase(NativePhase::Waiting))];
    let newer = [NativePhase::Working, NativePhase::Waiting]
        .into_iter()
        .map(|phase| {
            let mut update = page(&slot, Vec::new());
            update.machine_events = vec![machine_event(1, Phase(phase))];
            update
        })
        .collect();
    let carried = rehold_while_the_fold_waits(&mut slot, initial, newer);
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![carried]);
    slot.seat_guardian_hold(false);
    let before = dropped(&slot);
    let (_, delivered, _) = slot.drain_host_interacts();
    let phase = slot.seat.as_ref().unwrap().machine_phase(id(1));
    let after = dropped(&slot);
    slot.stop();
    assert_eq!(
        requests(delivered),
        (1..=20).map(lane_open).collect::<Vec<_>>(),
        "every retained row, in order"
    );
    assert_eq!(
        phase,
        Some(NativePhase::Waiting),
        "a return to an earlier phase is a transition, not a repeat"
    );
    assert_eq!(after, before, "resume drops nothing");
}

// R8: the isolate carries each unacknowledged mirror revoke in every batch
// until a page acknowledges it. Across two reholds the merge carries each
// owner once, so `validate_mirror_revokes` admits the batch, every revoke
// applies once, and no repeat counts as a late control.
#[test]
fn held_rehold_carries_a_repeated_mirror_revoke_once() {
    let mut slot = with_seat(891);
    let mut initial = page(&slot, (1..=20).map(lane_open).collect());
    initial.batch.mirror_revokes = vec![mirror_revoke(9, 1)];
    let newer = [
        vec![mirror_revoke(9, 1), mirror_revoke(8, 2)],
        vec![
            mirror_revoke(9, 1),
            mirror_revoke(8, 2),
            mirror_revoke(7, 3),
        ],
    ]
    .into_iter()
    .map(|revokes| {
        let mut update = page(&slot, Vec::new());
        update.batch.mirror_revokes = revokes;
        update
    })
    .collect();
    let carried = rehold_while_the_fold_waits(&mut slot, initial, newer);
    let revokes = carried.batch.mirror_revokes.clone();
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![carried]);
    slot.seat_guardian_hold(false);
    let late_before = slot.seat.as_ref().unwrap().late_controls();
    let before = dropped(&slot);
    let (_, delivered, _) = slot.drain_host_interacts();
    let seat = slot.seat.as_ref().unwrap();
    let (consumed, late_after) = (seat.mirror_revokes_consumed(), seat.late_controls());
    let after = dropped(&slot);
    slot.stop();
    assert_eq!(
        revokes,
        [
            mirror_revoke(9, 1),
            mirror_revoke(8, 2),
            mirror_revoke(7, 3)
        ],
        "each owner once, in sequence order"
    );
    assert_eq!(
        requests(delivered),
        (1..=20).map(lane_open).collect::<Vec<_>>(),
        "the batch is admitted and every retained row arrives"
    );
    assert_eq!(consumed, 3, "every revoke applies");
    assert_eq!(late_after, late_before, "no repeat counts as late");
    assert_eq!(after, before, "resume drops nothing");
}

// R8: each control class merges by the key that its producer keeps one
// control for. A newer control with the key of a kept control is the
// isolate's repeat of it: applied after the kept one it would be a
// duplicate or a late control, so the kept one stays and the repeat goes.
// A start keeps its id while the isolate replaces its arguments
// (`machine.rs` `carry_typed_start`), and a cancel keeps its target while
// the isolate raises its reason (`queue_cancel`).
#[test]
fn held_rehold_merges_each_control_by_its_producer_key() {
    use crate::isolate_fb::{SeatCancelDecoded, SeatDecisionInput};
    use crate::machine::MachineEventKind::{Done, Failed, Phase};
    use crate::native::NativePhase;
    use api::script_contract::{CallbackCancelReason, DecisionSequence};
    let cancel = |raw, reason| SeatCancelDecoded {
        of: id(raw),
        reason,
    };
    let decision = |raw, answer: &[u8]| SeatDecisionInput {
        of: id(raw),
        sequence: DecisionSequence::FIRST,
        answer: answer.to_vec(),
    };
    let mut slot = with_seat(892);
    let mut waiting = page(&slot, Vec::new());
    waiting.held_fold = true;
    waiting.batch.consumed_terminal_seq = 9;
    waiting.batch.starts = vec![start_input(&slot, 1, NativeKind::Gather)];
    waiting.batch.decisions = vec![decision(2, b"kept")];
    waiting.batch.cancels = vec![cancel(3, CallbackCancelReason::HookThrew)];
    waiting.batch.mirror_revokes = vec![mirror_revoke(4, 1)];
    waiting.machine_events = vec![
        machine_event(5, Phase(NativePhase::Waiting)),
        machine_event(6, Done),
    ];
    let mut newer = page(&slot, Vec::new());
    newer.batch.consumed_terminal_seq = 8;
    let mut replaced = start_input(&slot, 1, NativeKind::Gather);
    replaced.args = SeatStartArgs::Gather(SettingsBag::new());
    newer.batch.starts = vec![replaced, start_input(&slot, 7, NativeKind::Hunt)];
    newer.batch.decisions = vec![decision(2, b"repeat"), decision(8, b"new")];
    newer.batch.cancels = vec![
        cancel(3, CallbackCancelReason::ParentAborted),
        cancel(9, CallbackCancelReason::HookThrew),
    ];
    newer.batch.mirror_revokes = vec![mirror_revoke(4, 1), mirror_revoke(10, 2)];
    newer.machine_events = vec![
        machine_event(5, Phase(NativePhase::Waiting)),
        machine_event(6, Done),
        machine_event(11, Failed),
        machine_event(12, Phase(NativePhase::Working)),
    ];
    let expected_starts = vec![
        start_input(&slot, 1, NativeKind::Gather),
        start_input(&slot, 7, NativeKind::Hunt),
    ];
    let kept = slot
        .compact_held_seat_batches(vec![waiting, newer])
        .pop()
        .unwrap();
    slot.stop();
    assert_eq!(kept.batch.consumed_terminal_seq, 9, "the higher watermark");
    assert_eq!(kept.batch.starts, expected_starts, "starts by id");
    assert_eq!(
        kept.batch.decisions,
        [decision(2, b"kept"), decision(8, b"new")],
        "decisions by request and sequence"
    );
    assert_eq!(
        kept.batch.cancels,
        [
            cancel(3, CallbackCancelReason::HookThrew),
            cancel(9, CallbackCancelReason::HookThrew)
        ],
        "cancels by target"
    );
    assert_eq!(
        kept.batch.mirror_revokes,
        [mirror_revoke(4, 1), mirror_revoke(10, 2)],
        "mirror revokes by owner"
    );
    assert_eq!(
        kept.machine_events,
        [
            machine_event(5, Phase(NativePhase::Waiting)),
            machine_event(6, Done),
            machine_event(11, Failed),
            machine_event(12, Phase(NativePhase::Working)),
        ],
        "a phase equal to the last kept phase of its id, and a terminal \
         equal to a kept one, are repeats"
    );
}

// Round 9, operator decision 2026-10-16: a frame that the slot loop holds
// for a scene load or the welcome modal is a held tick, not the guardian's
// `SessionHeld`. The seat is not frozen, so the batch's start is admitted
// during the hold. The batch's gameplay row is dropped and counted. The
// walk abort is kept and leads the first frame after the hold. Synthetic
// sequence: the batch enters the isolate queue directly.
#[test]
fn host_held_tick_keeps_the_walk_abort_and_drops_gameplay() {
    let mut slot = with_seat(893);
    slot.seat_host_hold(true);
    let mut batch = page(
        &slot,
        vec![InteractReq::AbortWalk { request_id: 5 }, walk(6)],
    );
    batch.batch.starts = vec![start_input(&slot, 1, NativeKind::Hunt)];
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![batch]);
    let before = dropped(&slot);
    let (_, held, _) = slot.drain_host_interacts();
    let (_, still, _) = slot.drain_host_interacts();
    let seat = slot.seat.as_ref().unwrap();
    let (frozen, admitted) = (
        seat.held(),
        seat.live().iter().any(|entry| entry.id == id(1)),
    );
    let held_dropped = dropped(&slot) - before;
    slot.seat_host_hold(false);
    let (_, after, _) = slot.drain_host_interacts();
    let (_, later, _) = slot.drain_host_interacts();
    slot.stop();
    assert!(!frozen, "a host hold does not freeze the seat");
    assert!(
        admitted,
        "the seat applies the batch's start during the hold"
    );
    assert!(requests(held).is_empty(), "the held frame delivers nothing");
    assert!(requests(still).is_empty(), "the abort waits for the hold");
    assert_eq!(
        held_dropped, 1,
        "the held gameplay row is dropped and counted"
    );
    assert_eq!(
        requests(after),
        [InteractReq::AbortWalk { request_id: 5 }],
        "the first frame after the hold delivers the abort"
    );
    assert!(requests(later).is_empty(), "the abort is delivered once");
}

/// Put the watchdog in a recovery hold (`SamplingAnchor`).
fn hold_for_watchdog(slot: &mut SlotScript) {
    let start = Instant::now();
    slot.watchdog.arm_fresh(start);
    slot.watchdog
        .stamp_game_packet_tick(start + crate::watchdog::WEDGE);
    assert_eq!(
        slot.watchdog.observe(start + crate::watchdog::WEDGE, true),
        crate::watchdog::WatchdogAction::RequestAnchor
    );
    assert!(slot.watchdog.holds_script_actions());
}

fn end_watchdog_hold(slot: &mut SlotScript) {
    let _ = slot.watchdog.abort_owned_recovery();
    assert!(!slot.watchdog.holds_script_actions());
}

/// One host frame whose isolate batch has `reqs`. Returns what the drain
/// gives the host.
fn drained_frame(
    slot: &mut SlotScript,
    reqs: Vec<InteractReq>,
) -> (Option<Option<RunPolicyOverride>>, Vec<InteractReq>) {
    let mut pages = slot.surface.as_ref().unwrap().drain_seat_batches();
    pages.push(page(slot, reqs));
    slot.surface.as_ref().unwrap().restore_seat_batches(pages);
    let (policy, reqs, _) = slot.drain_host_interacts();
    (policy, requests(reqs))
}

fn floor_override(energy: i32) -> Option<Option<RunPolicyOverride>> {
    Some(Some(RunPolicyOverride {
        run_auto: None,
        energy_min: Some(RunEnergyMin::Floor(energy)),
    }))
}

// Round 10, Main decision 1 (operator decision 2026-10-16: aborts are never
// dropped): a watchdog recovery hold keeps the batch's walk aborts in the
// order the script sent them, at most one for each walk, and drops its
// gameplay. The first frame after the hold delivers the aborts once.
// Synthetic sequence: the batches enter the isolate queue directly.
#[test]
fn watchdog_held_tick_keeps_walk_aborts_in_send_order_and_drops_gameplay() {
    let mut slot = with_seat(894);
    hold_for_watchdog(&mut slot);
    let before = dropped(&slot);
    let first = drained_frame(
        &mut slot,
        vec![InteractReq::AbortWalk { request_id: 5 }, walk(6)],
    );
    slot.seat_held_tick();
    let second = drained_frame(
        &mut slot,
        vec![
            InteractReq::AbortWalk { request_id: 7 },
            InteractReq::AbortWalk { request_id: 5 },
            walk(8),
        ],
    );
    let held_dropped = dropped(&slot) - before;
    end_watchdog_hold(&mut slot);
    let (_, after, _) = slot.drain_host_interacts();
    let (_, later, _) = slot.drain_host_interacts();
    slot.stop();
    assert_eq!(first, (None, Vec::new()), "the held frame delivers nothing");
    assert_eq!(second, (None, Vec::new()), "the aborts wait for the hold");
    assert_eq!(
        held_dropped, 2,
        "only the two gameplay rows are dropped and counted"
    );
    assert_eq!(
        requests(after),
        [
            InteractReq::AbortWalk { request_id: 5 },
            InteractReq::AbortWalk { request_id: 7 },
        ],
        "the first frame after the hold delivers each abort once, in order"
    );
    assert!(requests(later).is_empty(), "the aborts are delivered once");
}

// Round 10, Main decision 1: the watchdog's held tick of the seat keeps a
// walk abort that waits in the seat queue, with its release slot, for the
// first frame after the hold. Synthetic sequence: the row enters the seat
// directly.
#[test]
fn watchdog_held_tick_keeps_a_walk_abort_queued_in_the_seat() {
    use api::script_contract::Release;
    let mut slot = with_seat(895);
    slot.seat
        .as_mut()
        .unwrap()
        .push(
            queued_interact(InteractReq::AbortWalk { request_id: 3 }),
            api::script_contract::EmissionClass::Release,
            EmissionSource::Root,
            Some(Release::AbortWalk(id(3))),
        )
        .unwrap();
    hold_for_watchdog(&mut slot);
    slot.seat_held_tick();
    let reserved = slot.seat.as_ref().unwrap().reserved_releases();
    let (_, held, _) = slot.drain_host_interacts();
    slot.seat_held_tick();
    end_watchdog_hold(&mut slot);
    let (_, after, _) = slot.drain_host_interacts();
    slot.stop();
    assert_eq!(reserved, 1, "the abort keeps its walk release slot");
    assert!(requests(held).is_empty(), "the abort waits for the hold");
    assert_eq!(
        requests(after),
        [InteractReq::AbortWalk { request_id: 3 }],
        "the first frame after the hold delivers the abort"
    );
}

// Round 10, Main decision 2: a watchdog recovery hold keeps the newest
// run-policy update and gives it to the first frame after the hold, after
// the kept abort. An older policy row that still waits in the seat does not
// replace it. The held frames give the host no policy. Synthetic sequence:
// the rows enter the seat and the isolate queue directly.
#[test]
fn watchdog_held_tick_keeps_the_newest_run_policy_for_after_the_hold() {
    let mut slot = with_seat(896);
    slot.seat
        .as_mut()
        .unwrap()
        .push_policy(queued_interact(floor(5)), EmissionSource::Root)
        .unwrap();
    hold_for_watchdog(&mut slot);
    slot.seat_held_tick();
    let first = drained_frame(
        &mut slot,
        vec![InteractReq::AbortWalk { request_id: 4 }, floor(10)],
    );
    slot.seat_held_tick();
    let second = drained_frame(&mut slot, vec![floor(20), walk(6)]);
    end_watchdog_hold(&mut slot);
    let after = drained_frame(&mut slot, Vec::new());
    let later = drained_frame(&mut slot, Vec::new());
    slot.stop();
    assert_eq!(first, (None, Vec::new()), "the held frame gives no policy");
    assert_eq!(second, (None, Vec::new()), "the policy waits for the hold");
    assert_eq!(
        after,
        (
            floor_override(20),
            vec![InteractReq::AbortWalk { request_id: 4 }]
        ),
        "the newest policy and the kept abort, in the first frame after the hold"
    );
    assert_eq!(later, (None, Vec::new()), "the policy is delivered once");
}

// Round 10, Main decision 2: a held tick of a frame that the slot loop
// holds (scene load, welcome modal) keeps a run-policy row that waits in
// the seat queue. The held frame gives the host no policy, and the first
// frame after the hold gives the kept one. Synthetic sequence: the row
// enters the seat directly.
#[test]
fn host_held_tick_keeps_the_seat_run_policy_for_after_the_hold() {
    let mut slot = with_seat(897);
    slot.seat
        .as_mut()
        .unwrap()
        .push_policy(queued_interact(floor(30)), EmissionSource::Root)
        .unwrap();
    slot.seat_host_hold(true);
    slot.seat_held_tick();
    let held = drained_frame(&mut slot, Vec::new());
    slot.seat_held_tick();
    let still = drained_frame(&mut slot, Vec::new());
    slot.seat_host_hold(false);
    let after = drained_frame(&mut slot, Vec::new());
    let later = drained_frame(&mut slot, Vec::new());
    slot.stop();
    assert_eq!(held, (None, Vec::new()), "the held frame gives no policy");
    assert_eq!(still, (None, Vec::new()), "the policy waits for the hold");
    assert_eq!(
        after,
        (floor_override(30), Vec::new()),
        "the first frame after the hold gives the kept policy"
    );
    assert_eq!(later, (None, Vec::new()), "the policy is delivered once");
}

// Round 10, Main decision 3: the precondition of the accepted fold-order
// limit (`carry_waiting_controls`). While the guardian holds, the seat does
// not complete or start a native (`native/seat.rs:790-793`, `:1016-1018`):
// neither a held frame nor a direct batch with `Done(1)` and a new start
// of request 2 changes the seat. So no terminal and no new start apply
// during the hold, and the seat state at resume is the state at the start
// of the hold.
#[test]
fn guardian_hold_neither_completes_nor_starts_a_native() {
    use crate::machine::MachineEventKind::Done;
    let mut slot = with_seat(898);
    slot.consume_seat_batch(
        decoded(vec![start_input(&slot, 1, NativeKind::Hunt)], Vec::new()),
        0,
        &[],
    );
    let state = |slot: &SlotScript| {
        let seat = slot.seat.as_ref().unwrap();
        (
            seat.live().iter().map(|entry| entry.id).collect::<Vec<_>>(),
            seat.results().len(),
        )
    };
    assert_eq!(state(&slot), (vec![id(1)], 0), "request 1 is live");
    slot.seat_guardian_hold(true);
    let mut held = page(&slot, Vec::new());
    held.batch.starts = vec![start_input(&slot, 2, NativeKind::Hunt)];
    held.machine_events = vec![machine_event(1, Done)];
    slot.surface
        .as_ref()
        .unwrap()
        .restore_seat_batches(vec![held]);
    let _ = slot.drain_host_interacts();
    let after_frame = state(&slot);
    slot.consume_seat_batch(
        decoded(vec![start_input(&slot, 2, NativeKind::Hunt)], Vec::new()),
        0,
        &[machine_event(1, Done)],
    );
    let after_batch = state(&slot);
    slot.stop();
    assert_eq!(
        after_frame,
        (vec![id(1)], 0),
        "a held frame neither completes request 1 nor starts request 2"
    );
    assert_eq!(
        after_batch,
        (vec![id(1)], 0),
        "a held seat neither completes request 1 nor starts request 2"
    );
}

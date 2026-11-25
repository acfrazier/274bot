use std::time::Duration;

use serde_json::Value;

use super::*;

fn interval(packets: u64, gap_ms: u64) -> ReceiveInterval {
    ReceiveInterval::new(packets, Duration::from_millis(gap_ms))
}

/// A proof window of `count` one-packet intervals of `tick_ms`.
fn steady(count: usize, tick_ms: u64) -> ProofWindow {
    let mut window = ProofWindow::default();
    for _ in 0..count {
        window.push(interval(1, tick_ms));
    }
    window
}

#[test]
fn measured_speed_equal_to_the_claim_counts() {
    let check = steady(20, 600).check(600);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Match);
    let receipt = check
        .into_result()
        .expect("a 600 ms proof counts as 600 ms");
    assert_eq!(receipt["claimed_engine_speed_ms"], 600);
    assert_eq!(receipt["engine_speed_ms"], 600.0);
    assert_eq!(receipt["verdict"], "match");
}

#[test]
fn null_engine_speed_does_not_count() {
    let check = ProofWindow::default().check(600);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Unmeasured);
    let receipt = check.receipt();
    assert_eq!(receipt["claimed_engine_speed_ms"], 600);
    assert_eq!(receipt["engine_speed_ms"], Value::Null);
    let error = check
        .into_result()
        .expect_err("engine_speed_ms: null must fail");
    assert!(error.contains("not measured"), "{error}");
    assert!(error.contains("\"claimed_engine_speed_ms\":600"), "{error}");
}

#[test]
fn measured_speed_different_from_the_claim_does_not_count() {
    // The ENGINE A SPEED incident: the engine ran at 200 ms while the cells
    // were read as 600 ms proof.
    for (claimed, actual) in [(600, 200), (600, 300), (300, 600), (600, 620)] {
        let check = steady(20, actual).check(claimed);
        assert_eq!(
            check.verdict(),
            EngineSpeedVerdict::Mismatch,
            "claimed {claimed} ms, measured {actual} ms"
        );
        let receipt = check.receipt();
        assert_eq!(receipt["claimed_engine_speed_ms"], claimed);
        assert_eq!(receipt["engine_speed_ms"], actual as f64);
        let error = check
            .into_result()
            .expect_err("a different speed must fail");
        assert!(
            error.contains(&format!("not the claimed {claimed} ms")),
            "{error}"
        );
    }
}

/// Review counterexample: the
/// proof completed after 20 intervals at 200 ms, then 34.8 s of 600 ms idle
/// followed, and the old meter measured the sequence as 600.0 ms. The idle
/// is outside the proof window, so the proof itself is measured.
#[test]
fn fast_proof_followed_by_claimed_speed_idle_does_not_count() {
    let check = steady(20, 200).check(600);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Mismatch);
    assert_eq!(check.engine_speed_ms(), Some(200.0));
}

/// Mirror: a proof at 600 ms followed by idle at the claimed 200 ms.
#[test]
fn slow_proof_followed_by_claimed_speed_idle_does_not_count() {
    let check = steady(20, 600).check(200);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Mismatch);
    assert_eq!(check.engine_speed_ms(), Some(600.0));
}

#[test]
fn proof_window_too_short_is_unmeasured() {
    // Five 600 ms ticks are 3.0 s, six are 3.6 s; the minimum is 3.5 s.
    let check = steady(5, 600).check(600);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Unmeasured);
    assert_eq!(check.engine_speed_ms(), None);
    assert_eq!(check.receipt()["window"]["ticks"], 5);
    assert_eq!(
        steady(6, 600).check(600).verdict(),
        EngineSpeedVerdict::Match
    );
}

#[test]
fn batched_intervals_count_their_ticks() {
    let mut batched = ProofWindow::default();
    for _ in 0..10 {
        batched.push(interval(2, 1200));
    }
    let check = batched.check(600);
    assert_eq!(check.engine_speed_ms(), Some(600.0));
    assert_eq!(check.verdict(), EngineSpeedVerdict::Match);
}

#[test]
fn a_minority_part_of_the_proof_at_another_speed_does_not_count() {
    // 30 ticks at 600 ms and 9 at 200 ms: the mean moves to 507.7 ms.
    let mut window = steady(30, 600);
    for _ in 0..9 {
        window.push(interval(1, 200));
    }
    let check = window.check(600);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Mismatch);
}

#[test]
fn parts_at_other_speeds_that_average_to_the_claim_do_not_count() {
    let mut window = ProofWindow::default();
    for _ in 0..10 {
        window.push(interval(1, 400));
        window.push(interval(1, 800));
    }
    let check = window.check(600);
    assert_eq!(check.engine_speed_ms(), Some(600.0));
    assert_eq!(check.receipt()["window"]["in_band_ticks"], 0);
    assert_eq!(check.verdict(), EngineSpeedVerdict::Mismatch);
}

#[test]
fn a_client_stall_and_its_catch_up_still_count() {
    // Thirty ticks with 0-59 ms receive jitter around the schedule, then a
    // 1.2 s client stall: the three ticks due in it are read at its end,
    // 22 and 23 ms apart, and the next tick is on schedule again.
    let mut window = ProofWindow::default();
    let mut lateness = 0_u64;
    for step in 0..30_u64 {
        let next = (step * 7) % 60;
        window.push(interval(1, 600 + next - lateness));
        lateness = next;
    }
    window.push(interval(1, 1_800 - lateness));
    window.push(interval(1, 22));
    window.push(interval(1, 23));
    window.push(interval(1, 555));
    let check = window.check(600);
    assert_eq!(
        check.verdict(),
        EngineSpeedVerdict::Match,
        "{}",
        check.receipt()
    );
    assert_eq!(check.engine_speed_ms(), Some(600.0));
}

#[test]
fn tolerance_is_two_percent_of_the_claim() {
    assert_eq!(
        steady(20, 612).check(600).verdict(),
        EngineSpeedVerdict::Match
    );
    assert_eq!(
        steady(20, 588).check(600).verdict(),
        EngineSpeedVerdict::Match
    );
    let mut outside = steady(19, 613);
    outside.push(interval(1, 613));
    assert_eq!(outside.check(600).verdict(), EngineSpeedVerdict::Mismatch);
}

#[test]
fn empty_drains_add_nothing() {
    let mut window = steady(5, 600);
    for _ in 0..50 {
        window.push(interval(0, 20));
    }
    assert_eq!(window.check(600).verdict(), EngineSpeedVerdict::Unmeasured);
}

#[test]
fn claimed_tick_defaults_to_the_engine_rate_and_refuses_invalid_values() {
    assert_eq!(
        parse_claimed_tick_ms("CLAIM", None),
        Ok(ENGINE_DEFAULT_TICK_MS)
    );
    assert_eq!(parse_claimed_tick_ms("CLAIM", Some("300")), Ok(300));
    assert!(parse_claimed_tick_ms("CLAIM", Some("19")).is_err());
    assert!(parse_claimed_tick_ms("CLAIM", Some("fast")).is_err());
}

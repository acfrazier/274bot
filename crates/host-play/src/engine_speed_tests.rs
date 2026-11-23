use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

/// One in-game session of `ticks` engine ticks at `tick_ms`, polled once
/// per tick.
fn session(meter: &mut TickPeriodMeter, start: Instant, first_tick: u32, ticks: u32, tick_ms: u64) {
    for step in 0..=ticks {
        let at = start + Duration::from_millis(u64::from(step) * tick_ms);
        meter.observe(first_tick + step, true, at);
    }
}

fn measured(ticks: u64, tick_ms: u64) -> Option<TickMeasurement> {
    Some(TickMeasurement::new(
        ticks,
        Duration::from_millis(ticks * tick_ms),
        1,
    ))
}

#[test]
fn measured_speed_equal_to_the_claim_counts() {
    let check = EngineSpeedCheck::new(600, measured(100, 600));
    assert_eq!(check.verdict(), EngineSpeedVerdict::Match);
    let receipt = check
        .into_result()
        .expect("a 600 ms measurement counts as 600 ms");
    assert_eq!(receipt["claimed_engine_speed_ms"], 600);
    assert_eq!(receipt["engine_speed_ms"], 600.0);
    assert_eq!(receipt["verdict"], "match");
}

#[test]
fn null_engine_speed_does_not_count() {
    let check = EngineSpeedCheck::new(600, None);
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
        let check = EngineSpeedCheck::new(claimed, measured(100, actual));
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

#[test]
fn tolerance_is_two_percent_of_the_claim() {
    let inside = EngineSpeedCheck::new(
        600,
        Some(TickMeasurement::new(100, Duration::from_millis(61_200), 1)),
    );
    assert!((inside.engine_speed_ms().expect("measured") - 612.0).abs() < 1e-9);
    assert_eq!(inside.verdict(), EngineSpeedVerdict::Match);
    let outside = EngineSpeedCheck::new(
        600,
        Some(TickMeasurement::new(100, Duration::from_millis(61_300), 1)),
    );
    assert_eq!(outside.verdict(), EngineSpeedVerdict::Mismatch);
}

/// Ticks at `tick_ms` that give a session three times the minimum window, so
/// the envelope span (at least a third) covers it.
fn long_session_ticks(tick_ms: u64) -> u32 {
    (3 * MIN_MEASURED_WINDOW.as_millis() as u64).div_ceil(tick_ms) as u32 + 3
}

#[test]
fn meter_measures_the_engine_period_from_tick_polls() {
    let start = Instant::now();
    for tick_ms in [600, 300, 200] {
        let mut meter = TickPeriodMeter::default();
        session(&mut meter, start, 7, long_session_ticks(tick_ms), tick_ms);
        let measurement = meter.measurement().expect("window covered");
        assert!((measurement.mean_ms() - tick_ms as f64).abs() < 1e-9);
        assert_eq!(
            meter.check(tick_ms as u32).verdict(),
            EngineSpeedVerdict::Match
        );
        let other = if tick_ms == 600 { 200 } else { 600 };
        assert_eq!(meter.check(other).verdict(), EngineSpeedVerdict::Mismatch);
    }
}

#[test]
fn meter_has_no_measurement_below_the_minimum_window() {
    let start = Instant::now();
    let mut meter = TickPeriodMeter::default();
    assert_eq!(meter.measurement(), None);
    let short = (MIN_MEASURED_WINDOW.as_millis() as u64 / 600) as u32 - 1;
    session(&mut meter, start, 0, short, 600);
    assert_eq!(meter.measurement(), None);
    assert_eq!(meter.check(600).verdict(), EngineSpeedVerdict::Unmeasured);
}

#[test]
fn meter_excludes_time_out_of_game_and_counter_resets() {
    let start = Instant::now();
    let mut meter = TickPeriodMeter::default();
    session(&mut meter, start, 1, 40, 600);
    // Logged out for 30 s: the poll out of game closes the session.
    let out = start + Duration::from_millis(40 * 600 + 100);
    meter.observe(41, false, out);
    // The relog starts a new session with a reset counter.
    let relog = out + Duration::from_secs(30);
    session(&mut meter, relog, 1, 40, 600);
    // A counter that goes back while in game is also a new session.
    let reset = relog + Duration::from_millis(40 * 600 + 5_000);
    session(&mut meter, reset, 3, 40, 600);
    let measurement = meter.measurement().expect("three 24 s sessions");
    // No gap between the sessions is in the measured time.
    assert_eq!(
        measurement.elapsed(),
        Duration::from_millis(measurement.ticks() * 600)
    );
    // Each envelope span covers at least the middle third: 41 points, 15 ticks.
    assert!(measurement.ticks() >= 3 * 15);
    assert_eq!(meter.check(600).receipt()["sessions"], 3);
}

#[test]
fn meter_ignores_repeated_polls_of_one_tick() {
    let start = Instant::now();
    let mut meter = TickPeriodMeter::default();
    for step in 0..=long_session_ticks(600) {
        let tick_at = start + Duration::from_millis(u64::from(step) * 600);
        // Three polls per tick, 20 ms apart; only the first sees the new tick.
        for poll in 0..3 {
            meter.observe(step, true, tick_at + Duration::from_millis(poll * 20));
        }
    }
    let mean = meter.measurement().map(|m| m.mean_ms()).expect("measured");
    assert!((mean - 600.0).abs() < 1e-9);
}

/// The live failure that motivated the envelope: at login the client loads
/// the scene, queued `PLAYER_INFO` packets arrive late and are then read one
/// per poll. Spanning from the first poll measured 571.7 ms on a 600 ms
/// engine.
#[test]
fn meter_is_not_shortened_by_late_ticks_after_a_client_stall() {
    for tick_ms in [600_u64, 300, 200] {
        let start = Instant::now();
        let ticks = long_session_ticks(tick_ms);
        let mut meter = TickPeriodMeter::default();
        // A 2.5 s stall at login, then catch-up one tick per 20 ms poll; a
        // second stall at the end of the session; steady polls with 0-19 ms
        // receive lateness in between.
        let stall = Duration::from_millis(2_500);
        let mut catch_up = start + stall;
        for tick in 0..=ticks {
            let due = start + Duration::from_millis(u64::from(tick) * tick_ms);
            let lateness = Duration::from_millis(u64::from(tick * 7 % 20));
            let mut seen = due + lateness;
            if seen < catch_up {
                seen = catch_up;
                catch_up += Duration::from_millis(20);
            }
            if tick + 3 > ticks {
                seen += Duration::from_millis(900);
            }
            meter.observe(tick, true, seen);
        }
        let mean = meter.measurement().expect("measured").mean_ms();
        assert!(
            (mean - tick_ms as f64).abs() < 0.01 * tick_ms as f64,
            "{tick_ms} ms engine measured as {mean} ms"
        );
        assert_eq!(
            meter.check(tick_ms as u32).verdict(),
            EngineSpeedVerdict::Match
        );
    }
}

#[test]
fn meter_records_no_tick_zero_and_no_short_session() {
    let start = Instant::now();
    let mut meter = TickPeriodMeter::default();
    // A short session at the wrong speed (29 ticks at 200 ms) gives no span.
    session(&mut meter, start, 1, MIN_SESSION_TICKS as u32 - 2, 200);
    meter.observe(0, false, start + Duration::from_secs(10));
    // Tick 0 is the login before the first PLAYER_INFO: 3 s before the first
    // tick of a 600 ms session, it is not a tick observation.
    let login = start + Duration::from_secs(20);
    meter.observe(0, true, login);
    session(
        &mut meter,
        login + Duration::from_secs(3),
        1,
        long_session_ticks(600),
        600,
    );
    let measurement = meter.measurement().expect("measured");
    assert!((measurement.mean_ms() - 600.0).abs() < 1e-9);
    assert_eq!(meter.check(600).receipt()["sessions"], 1);
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

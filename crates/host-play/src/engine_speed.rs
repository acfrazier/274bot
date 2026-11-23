//! Engine-speed oracle for the live cells.
//!
//! A live cell counts as proof only at the game-tick duration it claims. The
//! cell measures the duration from the client's own `PLAYER_INFO` receive
//! times and compares it with its claim before it prints PASS. A cell with no
//! measurement fails, and a cell whose measurement differs from its claim
//! fails. The receipt records the claimed and the measured value.
//!
//! Engine facts (revision 289 engine `ddd01b3`, revision 274 engine
//! `4c95f87`, `src/engine/World.ts`; line numbers are for 289):
//! - The default game tick is `World.TICKRATE = 600` ms (`World.ts:121`).
//!   `::speed N` replaces `World.tickRate` for the whole world and refuses
//!   `N < 20` (`network/game/client/handler/ClientCheatHandler.ts:160-167`).
//! - Each cycle sends `PLAYER_INFO` to every connected player
//!   (`World.ts:1096-1110`). The snapshot tick counts those packets
//!   (`api::snapshot::GameSnapshot::tick`), so one counter step is one
//!   engine tick.
//! - The cycle runs on a fixed schedule: `nextTick += tickRate` each cycle,
//!   and the next timeout subtracts the drift from that schedule
//!   (`World.ts:344,505,509`). Timer lateness therefore does not
//!   accumulate, and the mean period over a window is `tickRate`. A
//!   sustained cycle overrun makes the mean longer; that is a slower engine
//!   and the oracle correctly fails it.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Engine default game-tick duration (`World.ts:121`, `World.TICKRATE`).
pub const ENGINE_DEFAULT_TICK_MS: u32 = 600;

/// The smallest duration `::speed` accepts (`ClientCheatHandler.ts:160-163`).
pub const MIN_ENGINE_TICK_MS: u32 = 20;

/// Shortest tick span a measurement must cover before it counts.
///
/// [`TickPeriodMeter`] measures each in-game session between its two
/// earliest-observed ticks (see there), so the error of one session is the
/// difference of two small receive latencies: at most one cell poll interval
/// (20 ms in the live cells) plus one engine timer lateness, which the drift
/// correction keeps from accumulating. Two sessions (login and the fixture
/// relog) give at most four such ends, about 120 ms in total. Over 20 s that
/// is 0.6 % of the span, under a third of [`TOLERANCE_PERCENT`].
pub const MIN_MEASURED_WINDOW: Duration = Duration::from_secs(20);

/// Largest accepted difference between the measured mean and the claim, as a
/// percentage of the claim. Over three times the bound of
/// [`MIN_MEASURED_WINDOW`]; the speeds used in the lab (600, 300 and 200 ms)
/// differ by at least 33 %.
pub const TOLERANCE_PERCENT: u32 = 2;

/// Fewest observed ticks a session needs before it contributes, ten per
/// third.
///
/// After a login the client loads the scene while `PLAYER_INFO` packets
/// queue, then reads them a few per poll. A live trace on a 600 ms engine
/// showed three late ticks after a 1.4 s load. Ten points per third keep at
/// least one on-schedule point in the first third for a load of several
/// seconds; a shorter session gives no span at all rather than a wrong one.
pub const MIN_SESSION_TICKS: usize = 30;

/// Mean game-tick duration over the in-game windows a cell observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickMeasurement {
    ticks: u64,
    elapsed: Duration,
    sessions: u32,
}

impl TickMeasurement {
    /// A measurement of `ticks` engine ticks over `elapsed` in-game time.
    pub fn new(ticks: u64, elapsed: Duration, sessions: u32) -> Self {
        Self {
            ticks,
            elapsed,
            sessions,
        }
    }

    /// Mean milliseconds per engine tick.
    pub fn mean_ms(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1000.0 / self.ticks as f64
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

/// Measures the game-tick duration from snapshot polls.
///
/// Each in-game session is a separate series of the first poll that saw each
/// new tick: a logout, a disconnect or a tick counter that goes back (a new
/// session) ends the series, and time out of game is not counted. Tick 0 is
/// the session before its first `PLAYER_INFO` (`GameSnapshot::tick`), so it is
/// not a tick observation and is not recorded.
///
/// A poll sees a tick late when the client stalls, for example while it loads
/// a scene after login, and then catches up on the queued packets. A late
/// first or last point would shorten or stretch the span (a live run measured
/// 571.7 ms on a 600 ms engine that way). So the span of a session runs
/// between the point with the least lateness in its first third and the one
/// in its last third: a lower envelope of (tick, time). Lateness is never
/// negative, so these points lie on the engine's own schedule within one
/// poll interval.
#[derive(Debug, Default, Clone)]
pub struct TickPeriodMeter {
    closed: Vec<Vec<(u32, Instant)>>,
    current: Vec<(u32, Instant)>,
}

impl TickPeriodMeter {
    /// Record one poll: the snapshot tick, whether the client is in game, and
    /// the poll time.
    pub fn observe(&mut self, tick: u32, ingame: bool, at: Instant) {
        if !ingame || tick == 0 {
            self.close_session();
            return;
        }
        match self.current.last().map(|&(previous, _)| previous) {
            Some(previous) if tick == previous => {}
            Some(previous) if tick < previous => {
                self.close_session();
                self.current.push((tick, at));
            }
            _ => self.current.push((tick, at)),
        }
    }

    fn close_session(&mut self) {
        if !self.current.is_empty() {
            self.closed.push(std::mem::take(&mut self.current));
        }
    }

    /// The measurement, or `None` until the session spans cover
    /// [`MIN_MEASURED_WINDOW`].
    pub fn measurement(&self) -> Option<TickMeasurement> {
        let mut total = TickMeasurement::new(0, Duration::ZERO, 0);
        for session in self.closed.iter().chain(std::iter::once(&self.current)) {
            if let Some((ticks, elapsed)) = envelope_span(session) {
                total.ticks += ticks;
                total.elapsed += elapsed;
                total.sessions += 1;
            }
        }
        (total.ticks > 0 && total.elapsed >= MIN_MEASURED_WINDOW).then_some(total)
    }

    /// The claimed duration checked against this meter's measurement.
    pub fn check(&self, claimed_ms: u32) -> EngineSpeedCheck {
        EngineSpeedCheck::new(claimed_ms, self.measurement())
    }
}

/// Ticks and time between the least-late point of the first third of one
/// session and the least-late point of its last third.
///
/// Lateness is measured against the line through the two chosen points; the
/// choice starts from the whole span and is repeated until it is stable
/// (four rounds; one point move per round is the usual case).
fn envelope_span(points: &[(u32, Instant)]) -> Option<(u64, Duration)> {
    if points.len() < MIN_SESSION_TICKS {
        return None;
    }
    let third = points.len() / 3;
    let (first_tick, first_at) = points[0];
    let x = |i: usize| f64::from(points[i].0 - first_tick);
    let y = |i: usize| points[i].1.duration_since(first_at).as_secs_f64();
    let least_late = |range: std::ops::Range<usize>, slope: f64| {
        range
            .min_by(|&i, &j| (y(i) - slope * x(i)).total_cmp(&(y(j) - slope * x(j))))
            .expect("each third holds at least one point")
    };
    let (mut start, mut end) = (0, points.len() - 1);
    for _ in 0..4 {
        let slope = (y(end) - y(start)) / (x(end) - x(start));
        let next = (
            least_late(0..third, slope),
            least_late(points.len() - third..points.len(), slope),
        );
        if next == (start, end) {
            break;
        }
        (start, end) = next;
    }
    let ((start_tick, start_at), (end_tick, end_at)) = (points[start], points[end]);
    Some((
        u64::from(end_tick - start_tick),
        end_at.duration_since(start_at),
    ))
}

/// The oracle verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSpeedVerdict {
    /// The measured mean is within [`TOLERANCE_PERCENT`] of the claim.
    Match,
    /// There is no measurement (`engine_speed_ms: null`).
    Unmeasured,
    /// The measured mean differs from the claim by more than the tolerance.
    Mismatch,
}

impl EngineSpeedVerdict {
    fn label(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Unmeasured => "unmeasured",
            Self::Mismatch => "mismatch",
        }
    }
}

/// One claimed speed and one measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineSpeedCheck {
    claimed_ms: u32,
    measured: Option<TickMeasurement>,
}

impl EngineSpeedCheck {
    pub fn new(claimed_ms: u32, measured: Option<TickMeasurement>) -> Self {
        Self {
            claimed_ms,
            measured,
        }
    }

    pub fn claimed_ms(&self) -> u32 {
        self.claimed_ms
    }

    /// The measurement, if it has at least one tick.
    fn counted(&self) -> Option<TickMeasurement> {
        self.measured.filter(|measured| measured.ticks > 0)
    }

    /// The measured mean in milliseconds; `None` without a measurement.
    pub fn engine_speed_ms(&self) -> Option<f64> {
        self.counted().map(|measured| measured.mean_ms())
    }

    /// Largest accepted difference, in milliseconds.
    pub fn tolerance_ms(&self) -> f64 {
        f64::from(self.claimed_ms) * f64::from(TOLERANCE_PERCENT) / 100.0
    }

    pub fn verdict(&self) -> EngineSpeedVerdict {
        let Some(measured) = self.counted() else {
            return EngineSpeedVerdict::Unmeasured;
        };
        // Exact integer form of |elapsed / ticks - claimed| <= claimed * p / 100,
        // in nanoseconds, so the tolerance edge has no rounding.
        let claimed_window = u128::from(measured.ticks) * u128::from(self.claimed_ms) * 1_000_000;
        let difference = measured.elapsed.as_nanos().abs_diff(claimed_window);
        if difference * 100 <= claimed_window * u128::from(TOLERANCE_PERCENT) {
            EngineSpeedVerdict::Match
        } else {
            EngineSpeedVerdict::Mismatch
        }
    }

    /// Receipt fields: the claim, the measurement and the verdict.
    pub fn receipt(&self) -> Value {
        let measured = self.counted();
        json!({
            "claimed_engine_speed_ms": self.claimed_ms,
            "engine_speed_ms": self.engine_speed_ms(),
            "sampled_ticks": measured.map(|measured| measured.ticks),
            "sampled_ms": measured.map(|measured| measured.elapsed.as_millis() as u64),
            "sessions": measured.map(|measured| measured.sessions),
            "tolerance_ms": self.tolerance_ms(),
            "min_window_ms": MIN_MEASURED_WINDOW.as_millis() as u64,
            "source": "PLAYER_INFO receive times while in game",
            "verdict": self.verdict().label(),
        })
    }

    /// The receipt when the cell counts; otherwise an error that names both
    /// values.
    pub fn into_result(self) -> Result<Value, String> {
        let receipt = self.receipt();
        match self.verdict() {
            EngineSpeedVerdict::Match => Ok(receipt),
            EngineSpeedVerdict::Unmeasured => Err(format!(
                "engine speed was not measured (claimed {} ms); the cell does not count: {receipt}",
                self.claimed_ms
            )),
            EngineSpeedVerdict::Mismatch => Err(format!(
                "engine ran at {:.1} ms per tick, not the claimed {} ms; the cell does not count: {receipt}",
                self.engine_speed_ms().unwrap_or_default(),
                self.claimed_ms
            )),
        }
    }
}

/// Parse a claimed tick duration, as given by a cell's environment.
pub fn parse_claimed_tick_ms(name: &str, value: Option<&str>) -> Result<u32, String> {
    let Some(value) = value else {
        return Ok(ENGINE_DEFAULT_TICK_MS);
    };
    value
        .parse::<u32>()
        .ok()
        .filter(|ms| *ms >= MIN_ENGINE_TICK_MS)
        .ok_or_else(|| {
            format!(
                "{name} must be a tick duration of at least {MIN_ENGINE_TICK_MS} ms, got {value:?}"
            )
        })
}

#[cfg(test)]
#[path = "engine_speed_tests.rs"]
mod tests;

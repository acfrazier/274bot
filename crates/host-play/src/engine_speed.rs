//! Engine-speed oracle for the live cells.
//!
//! A live cell counts as proof only at the game-tick duration it claims, and
//! only for the span in which its proof ran. The cell opens a [`ProofWindow`]
//! at its start action and checks it at its PASS evidence. Receive intervals
//! from before the start action or after the PASS never enter the window, so
//! idle time cannot stand for the proof. A window with too few intervals is
//! [`EngineSpeedVerdict::Unmeasured`] and the cell fails. The receipt records
//! the claimed and the measured value.
//!
//! Input is one record per nonempty client drain: the `PLAYER_INFO` packets
//! it applied and the receive time since the previous nonempty drain of the
//! same session. This is the shape of the TickHealth receipt
//! (`packet_delta`, `drain_gap_ms`), so a cell can feed the window from that
//! receipt without a second clock.
//!
//! Engine facts (revision 289 engine `ddd01b3`, revision 274 engine
//! `4c95f87`, `src/engine/World.ts`; line numbers are for 289):
//! - The default game tick is `World.TICKRATE = 600` ms (`World.ts:121`).
//!   `::speed N` replaces `World.tickRate` for the whole world and refuses
//!   `N < 20` (`network/game/client/handler/ClientCheatHandler.ts:160-167`).
//! - Each cycle sends `PLAYER_INFO` to every connected player
//!   (`World.ts:1096-1110`), so one packet is one engine tick.
//! - The next cycle is scheduled `tickRate` after the planned start of this
//!   one, less the drift (`World.ts:344,505,509`). The engine does not bound
//!   timer lateness, client stalls or receive delay; the oracle therefore
//!   uses order statistics of the observed intervals, not a model of them.

use std::time::Duration;

use serde_json::{json, Value};

/// Engine default game-tick duration (`World.ts:121`, `World.TICKRATE`).
pub const ENGINE_DEFAULT_TICK_MS: u32 = 600;

/// The smallest duration `::speed` accepts (`ClientCheatHandler.ts:160-163`).
pub const MIN_ENGINE_TICK_MS: u32 = 20;

/// Fewest clean (one-packet) intervals a proof window needs before it has a
/// measurement. The same count as the TickHealth clean-interval median.
pub const MIN_CLEAN_INTERVALS: usize = 11;

/// Largest accepted difference between the median clean interval of the
/// proof window and the claim, as a percentage of the claim. The lab speeds
/// (600, 300 and 200 ms) differ by at least 33 %. The measured spread that
/// this value must cover is in the A3 evidence report.
pub const TOLERANCE_PERCENT: u32 = 2;

/// Per-interval band, as a percentage of the claim, inside which an
/// interval's per-packet time counts as at the claimed speed.
pub const INTERVAL_BAND_PERCENT: u32 = 25;

/// Smallest share of the proof window's ticks, as a percentage, that must lie
/// in intervals inside [`INTERVAL_BAND_PERCENT`]. The ticks outside are
/// client stalls and their catch-up, or a part of the proof that ran at
/// another speed; the median alone would not see a minority part.
pub const MIN_COVERAGE_PERCENT: u32 = 80;

/// One nonempty drain: packets applied and the receive time since the
/// previous nonempty drain of the same session (TickHealth `packet_delta`
/// and `drain_gap_ms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveInterval {
    pub packets: u64,
    pub gap: Duration,
}

impl ReceiveInterval {
    pub fn new(packets: u64, gap: Duration) -> Self {
        Self { packets, gap }
    }
}

/// The receive intervals of one proof, from its start action to its PASS.
#[derive(Debug, Default, Clone)]
pub struct ProofWindow {
    intervals: Vec<ReceiveInterval>,
}

impl ProofWindow {
    /// Add one interval observed inside the proof. Empty drains carry no
    /// interval and are ignored.
    pub fn push(&mut self, interval: ReceiveInterval) {
        if interval.packets > 0 {
            self.intervals.push(interval);
        }
    }

    /// The window's statistics, or `None` without a clean interval. A
    /// window with fewer than [`MIN_CLEAN_INTERVALS`] clean intervals has
    /// statistics for its receipt but no measured speed.
    pub fn measurement(&self) -> Option<WindowMeasurement> {
        let mut clean: Vec<Duration> = self
            .intervals
            .iter()
            .filter(|interval| interval.packets == 1)
            .map(|interval| interval.gap)
            .collect();
        if clean.is_empty() {
            return None;
        }
        clean.sort_unstable();
        let middle = clean.len() / 2;
        let median = if clean.len() % 2 == 1 {
            clean[middle]
        } else {
            (clean[middle - 1] + clean[middle]) / 2
        };
        let rank = |percent: usize| clean[(clean.len() - 1) * percent / 100];
        Some(WindowMeasurement {
            intervals: self.intervals.clone(),
            clean: clean.len(),
            median,
            p10: rank(10),
            p90: rank(90),
            min: clean[0],
            max: clean[clean.len() - 1],
        })
    }

    /// The claimed duration checked against this window.
    pub fn check(&self, claimed_ms: u32) -> EngineSpeedCheck {
        EngineSpeedCheck::new(claimed_ms, self.measurement())
    }
}

/// Order statistics of one proof window's receive intervals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMeasurement {
    intervals: Vec<ReceiveInterval>,
    clean: usize,
    median: Duration,
    p10: Duration,
    p90: Duration,
    min: Duration,
    max: Duration,
}

impl WindowMeasurement {
    pub fn median(&self) -> Duration {
        self.median
    }

    fn ticks(&self) -> u64 {
        self.intervals.iter().map(|interval| interval.packets).sum()
    }

    fn elapsed(&self) -> Duration {
        self.intervals.iter().map(|interval| interval.gap).sum()
    }

    /// Ticks in intervals whose per-packet time is within the band of the
    /// claim. Exact integer nanoseconds.
    fn in_band_ticks(&self, claimed_ms: u32) -> u64 {
        self.intervals
            .iter()
            .filter(|interval| {
                within_percent(
                    interval.gap.as_nanos(),
                    u128::from(interval.packets) * nanos(claimed_ms),
                    INTERVAL_BAND_PERCENT,
                )
            })
            .map(|interval| interval.packets)
            .sum()
    }
}

fn nanos(ms: u32) -> u128 {
    u128::from(ms) * 1_000_000
}

/// `|measured - expected| <= expected * percent / 100`, exactly.
fn within_percent(measured: u128, expected: u128, percent: u32) -> bool {
    measured.abs_diff(expected) * 100 <= expected * u128::from(percent)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The oracle verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSpeedVerdict {
    /// The median is within [`TOLERANCE_PERCENT`] of the claim and the
    /// coverage is at least [`MIN_COVERAGE_PERCENT`].
    Match,
    /// The proof window has no measurement (`engine_speed_ms: null`).
    Unmeasured,
    /// The median or the coverage fails.
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

/// One claimed speed and one proof-window measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineSpeedCheck {
    claimed_ms: u32,
    measured: Option<WindowMeasurement>,
}

impl EngineSpeedCheck {
    pub fn new(claimed_ms: u32, measured: Option<WindowMeasurement>) -> Self {
        Self {
            claimed_ms,
            measured,
        }
    }

    pub fn claimed_ms(&self) -> u32 {
        self.claimed_ms
    }

    /// The window statistics when they are a measurement: at least
    /// [`MIN_CLEAN_INTERVALS`] clean intervals.
    fn counted(&self) -> Option<&WindowMeasurement> {
        self.measured
            .as_ref()
            .filter(|measured| measured.clean >= MIN_CLEAN_INTERVALS)
    }

    /// The measured speed: the median clean interval in milliseconds; `None`
    /// without a measurement.
    pub fn engine_speed_ms(&self) -> Option<f64> {
        self.counted().map(|measured| ms(measured.median))
    }

    pub fn verdict(&self) -> EngineSpeedVerdict {
        let Some(measured) = self.counted() else {
            return EngineSpeedVerdict::Unmeasured;
        };
        let median_matches = within_percent(
            measured.median.as_nanos(),
            nanos(self.claimed_ms),
            TOLERANCE_PERCENT,
        );
        let covered = u128::from(measured.in_band_ticks(self.claimed_ms)) * 100
            >= u128::from(measured.ticks()) * u128::from(MIN_COVERAGE_PERCENT);
        if median_matches && covered {
            EngineSpeedVerdict::Match
        } else {
            EngineSpeedVerdict::Mismatch
        }
    }

    /// Receipt fields: the claim, the measurement and the verdict.
    pub fn receipt(&self) -> Value {
        let measured = self.measured.as_ref();
        json!({
            "claimed_engine_speed_ms": self.claimed_ms,
            "engine_speed_ms": self.engine_speed_ms(),
            "window": measured.map(|measured| json!({
                "intervals": measured.intervals.len(),
                "clean_intervals": measured.clean,
                "ticks": measured.ticks(),
                "elapsed_ms": ms(measured.elapsed()),
                "mean_tick_ms": ms(measured.elapsed()) / measured.ticks() as f64,
                "clean_median_ms": ms(measured.median),
                "clean_min_ms": ms(measured.min),
                "clean_p10_ms": ms(measured.p10),
                "clean_p90_ms": ms(measured.p90),
                "clean_max_ms": ms(measured.max),
                "in_band_ticks": measured.in_band_ticks(self.claimed_ms),
                "gaps_ms": measured.intervals.iter().map(|interval| {
                    json!([interval.packets, ms(interval.gap)])
                }).collect::<Vec<_>>(),
            })),
            "tolerance_percent": TOLERANCE_PERCENT,
            "interval_band_percent": INTERVAL_BAND_PERCENT,
            "min_coverage_percent": MIN_COVERAGE_PERCENT,
            "min_clean_intervals": MIN_CLEAN_INTERVALS,
            "source": "PLAYER_INFO receive intervals from the proof start action to PASS",
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
                "engine speed was not measured in the proof window (claimed {} ms); the cell does not count: {receipt}",
                self.claimed_ms
            )),
            EngineSpeedVerdict::Mismatch => Err(format!(
                "the proof ran at a median {:.1} ms per tick or partly at another speed, not the claimed {} ms; the cell does not count: {receipt}",
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

//! Engine-speed oracle for the live cells.
//!
//! A live cell counts as proof only at the game-tick duration it claims, and
//! only for the span in which its proof ran. The cell opens a [`ProofWindow`]
//! at its start action and checks it at its PASS evidence. Receive intervals
//! from before the start action or after the PASS never enter the window, so
//! idle time cannot stand for the proof. A window shorter than
//! [`MIN_WINDOW_SPAN`] is [`EngineSpeedVerdict::Unmeasured`] and the cell
//! fails. The receipt records the claimed and the measured value.
//!
//! Input is one record per nonempty client drain: the `PLAYER_INFO` packets
//! it applied and the receive time since the previous nonempty drain of the
//! same session. This is the shape of the TickHealth receipt
//! (`packet_delta`, `drain_gap_ms`), so a cell can feed the window from that
//! receipt without a second clock.
//!
//! The measured speed is the window mean: the summed receive time divided by
//! the ticks. It covers every tick of the proof, and a client stall inside
//! the window does not move it, because the stall's long interval and the
//! short catch-up intervals after it add up to the time the ticks took. Only
//! the lateness of the window's first and last observation enters it; see
//! [`MIN_WINDOW_SPAN`]. A coverage check rejects a window whose parts ran at
//! different speeds that average to the claim.
//!
//! Engine facts (revision 289 engine `ddd01b3`, revision 274 engine
//! `4c95f87`, `src/engine/World.ts`; line numbers are for 289):
//! - The default game tick is `World.TICKRATE = 600` ms (`World.ts:121`).
//!   `::speed N` replaces `World.tickRate` for the whole world and refuses
//!   `N < 20` (`network/game/client/handler/ClientCheatHandler.ts:160-167`).
//! - Each cycle sends `PLAYER_INFO` to every connected player
//!   (`World.ts:1096-1110`), so one packet is one engine tick.
//! - The next cycle is scheduled `tickRate` after the planned start of this
//!   one, less the drift (`World.ts:344,505,509`), so cycle lateness does not
//!   accumulate into the mean. The engine does not bound a single cycle's
//!   lateness, client stalls or receive delay; those bounds are measured.

use std::time::Duration;

use serde_json::{json, Value};

/// Engine default game-tick duration (`World.ts:121`, `World.TICKRATE`).
pub const ENGINE_DEFAULT_TICK_MS: u32 = 600;

/// The smallest duration `::speed` accepts (`ClientCheatHandler.ts:160-163`).
pub const MIN_ENGINE_TICK_MS: u32 = 20;

/// Largest accepted difference between the window mean and the claim, as a
/// percentage of the claim. The lab speeds (600, 300 and 200 ms) differ by at
/// least 33 %. Measured 600 ms windows of 6 to 10 settled ticks had means of
/// 596.0 to 603.8 ms on Engine A and 599.9 to 601.4 ms on Engine L (at most
/// 0.7 % off).
pub const TOLERANCE_PERCENT: u32 = 2;

/// Shortest summed receive time a proof window needs before it is a
/// measurement.
///
/// The window mean is off by the lateness difference of its first and last
/// observation divided by the span. The largest measured single-interval
/// deviation from 600 ms was 34 ms (Engine A) and 20 ms (Engine L), so the
/// lateness difference is taken as 35 ms. At 3.5 s that is 1 % of the span,
/// half of [`TOLERANCE_PERCENT`], at any claimed speed.
pub const MIN_WINDOW_SPAN: Duration = Duration::from_millis(3_500);

/// Per-interval band, as a percentage of the claim, inside which an
/// interval's per-packet time counts as at the claimed speed. Measured
/// steady intervals at 600 ms deviate by at most 6 %.
pub const INTERVAL_BAND_PERCENT: u32 = 25;

/// Smallest share of the proof window's ticks, as a percentage, that must lie
/// in intervals inside [`INTERVAL_BAND_PERCENT`]. The ticks outside are
/// client stalls and their catch-up, or parts of the proof at other speeds
/// that the mean alone would not see when they average to the claim.
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

    /// The claimed duration checked against this window.
    pub fn check(&self, claimed_ms: u32) -> EngineSpeedCheck {
        EngineSpeedCheck {
            claimed_ms,
            intervals: self.intervals.clone(),
        }
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
    /// The mean is within [`TOLERANCE_PERCENT`] of the claim and the coverage
    /// is at least [`MIN_COVERAGE_PERCENT`].
    Match,
    /// The proof window is shorter than [`MIN_WINDOW_SPAN`]
    /// (`engine_speed_ms: null`).
    Unmeasured,
    /// The mean or the coverage fails.
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

/// One claimed speed and one proof window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSpeedCheck {
    claimed_ms: u32,
    intervals: Vec<ReceiveInterval>,
}

impl EngineSpeedCheck {
    pub fn claimed_ms(&self) -> u32 {
        self.claimed_ms
    }

    fn ticks(&self) -> u64 {
        self.intervals.iter().map(|interval| interval.packets).sum()
    }

    fn span(&self) -> Duration {
        self.intervals.iter().map(|interval| interval.gap).sum()
    }

    fn measured(&self) -> bool {
        self.span() >= MIN_WINDOW_SPAN
    }

    /// Ticks in intervals whose per-packet time is within the band of the
    /// claim. Exact integer nanoseconds.
    fn in_band_ticks(&self) -> u64 {
        self.intervals
            .iter()
            .filter(|interval| {
                within_percent(
                    interval.gap.as_nanos(),
                    u128::from(interval.packets) * nanos(self.claimed_ms),
                    INTERVAL_BAND_PERCENT,
                )
            })
            .map(|interval| interval.packets)
            .sum()
    }

    /// The measured speed: the window mean in milliseconds per tick; `None`
    /// without a measurement.
    pub fn engine_speed_ms(&self) -> Option<f64> {
        self.measured()
            .then(|| ms(self.span()) / self.ticks() as f64)
    }

    pub fn verdict(&self) -> EngineSpeedVerdict {
        if !self.measured() {
            return EngineSpeedVerdict::Unmeasured;
        }
        let mean_matches = within_percent(
            self.span().as_nanos(),
            u128::from(self.ticks()) * nanos(self.claimed_ms),
            TOLERANCE_PERCENT,
        );
        let covered = u128::from(self.in_band_ticks()) * 100
            >= u128::from(self.ticks()) * u128::from(MIN_COVERAGE_PERCENT);
        if mean_matches && covered {
            EngineSpeedVerdict::Match
        } else {
            EngineSpeedVerdict::Mismatch
        }
    }

    /// Receipt fields: the claim, the measurement, the window and the
    /// verdict.
    pub fn receipt(&self) -> Value {
        json!({
            "claimed_engine_speed_ms": self.claimed_ms,
            "engine_speed_ms": self.engine_speed_ms(),
            "window": {
                "intervals": self.intervals.len(),
                "ticks": self.ticks(),
                "span_ms": ms(self.span()),
                "in_band_ticks": self.in_band_ticks(),
                "intervals_ms": self.intervals.iter().map(|interval| {
                    json!([interval.packets, ms(interval.gap)])
                }).collect::<Vec<_>>(),
            },
            "tolerance_percent": TOLERANCE_PERCENT,
            "min_window_span_ms": MIN_WINDOW_SPAN.as_millis() as u64,
            "interval_band_percent": INTERVAL_BAND_PERCENT,
            "min_coverage_percent": MIN_COVERAGE_PERCENT,
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
                "engine speed was not measured: the proof window is shorter than {} ms (claimed {} ms); the cell does not count: {receipt}",
                MIN_WINDOW_SPAN.as_millis(),
                self.claimed_ms
            )),
            EngineSpeedVerdict::Mismatch => Err(format!(
                "the proof ran at a mean {:.1} ms per tick, or partly at another speed, not the claimed {} ms; the cell does not count: {receipt}",
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

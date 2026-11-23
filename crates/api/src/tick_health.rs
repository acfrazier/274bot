//! Bounded observations of applied `PLAYER_INFO` packets, never frame or poll counts.
//!
//! A receive observation is the monotonic instant after a client drain. It is
//! not a socket timestamp for each packet. A batch loses intermediate state
//! coverage, not packet count. No wall interval implies a missing-tick count.

use std::time::{Duration, Instant};

use serde::Serialize;

const CLEAN_SAMPLES: usize = 11;

/// One drain's packet coverage and this session's receive measurements.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct TickHealthReceipt {
    /// Client session identity, replaced on logout or a successful login.
    pub session: u64,
    /// Raw lifetime `PLAYER_INFO` generation at the end of this drain.
    pub packet_generation: u64,
    /// Actual server tick packets applied in this drain.
    pub packet_delta: u64,
    /// Intermediate packet states not separately observed in this drain.
    pub coalesced_observations: u64,
    /// Actual server tick packets observed since this session's watermark.
    pub server_ticks: u64,
    /// Milliseconds between nonempty receive observations, absent at a baseline.
    pub drain_gap_ms: Option<u64>,
    /// Previous completed service's scheduling delay, not receive latency.
    /// This has one service of lag. `None` means no capacity measurement.
    pub host_service_delay_ms: Option<u64>,
    /// Nonempty receive intervals, including batched intervals.
    pub interval_samples: u64,
    /// Packets covered by those intervals, excluding the first observation.
    pub sampled_game_ticks: u64,
    /// Sum of monotonic receive intervals in milliseconds.
    pub elapsed_ms: u64,
    /// Mean receive interval per packet. This is not a declared server cadence.
    pub mean_tick_ms: Option<f64>,
    /// Minimum interval divided by that interval's packet delta.
    pub minimum_tick_ms: Option<f64>,
    /// Maximum interval divided by that interval's packet delta.
    pub maximum_tick_ms: Option<f64>,
    /// Median of the first eleven clean, one-packet receive intervals.
    pub median_ms: Option<u64>,
    /// Number of retained clean intervals, bounded by eleven.
    pub clean_interval_samples: usize,
    /// Intermediate observations lost to batches in this session.
    pub total_coalesced_observations: u64,
    /// Session replacements since this observer started, excluding its first.
    pub session_resets: u64,
}

/// Fixed storage for one receipt and eleven clean receive intervals.
/// Observation and receipt reads allocate nothing. Serialization is caller work.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TickHealth {
    #[serde(flatten)]
    receipt: TickHealthReceipt,
    #[serde(skip)]
    last: Option<(u64, u64)>,
    #[serde(skip)]
    received_at: Option<Instant>,
    #[serde(skip)]
    elapsed: Duration,
    #[serde(skip)]
    clean_ms: [u64; CLEAN_SAMPLES],
}

/// Packet count after the prior generation, or after the new session watermark.
/// A raw counter reset must also change the session identity. A wrap need not.
pub fn packet_delta(
    previous_session: u64,
    previous_generation: u64,
    session: u64,
    generation: u64,
    session_start_generation: u64,
) -> u64 {
    generation.wrapping_sub(if previous_session == session {
        previous_generation
    } else {
        session_start_generation
    })
}

impl TickHealth {
    /// Observe one drain after packets were applied. Empty drains do not move
    /// the receive baseline. A new session clears all rate and coverage totals.
    pub fn observe(
        &mut self,
        session: u64,
        generation: u64,
        session_start_generation: u64,
        received_at: Instant,
        host_service_delay: Option<Duration>,
    ) -> TickHealthReceipt {
        let changed = self.last.is_some_and(|(prior, _)| prior != session);
        let delta = self.last.map_or_else(
            || generation.wrapping_sub(session_start_generation),
            |(prior_session, prior_generation)| {
                packet_delta(
                    prior_session,
                    prior_generation,
                    session,
                    generation,
                    session_start_generation,
                )
            },
        );
        if changed {
            let resets = self.receipt.session_resets.saturating_add(1);
            *self = Self::default();
            self.receipt.session_resets = resets;
        }
        self.last = Some((session, generation));
        let receipt = &mut self.receipt;
        receipt.session = session;
        receipt.packet_generation = generation;
        receipt.packet_delta = delta;
        receipt.coalesced_observations = delta.saturating_sub(1);
        receipt.server_ticks = receipt.server_ticks.saturating_add(delta);
        receipt.total_coalesced_observations = receipt
            .total_coalesced_observations
            .saturating_add(receipt.coalesced_observations);
        receipt.drain_gap_ms = None;
        receipt.host_service_delay_ms = (!changed)
            .then_some(host_service_delay)
            .flatten()
            .map(milliseconds);
        if delta == 0 {
            return *receipt;
        }
        if let Some(previous_at) = self.received_at.replace(received_at) {
            let gap = received_at.saturating_duration_since(previous_at);
            let gap_ms = milliseconds(gap);
            receipt.drain_gap_ms = Some(gap_ms);
            self.elapsed = self.elapsed.saturating_add(gap);
            receipt.elapsed_ms = milliseconds(self.elapsed);
            receipt.interval_samples = receipt.interval_samples.saturating_add(1);
            receipt.sampled_game_ticks = receipt.sampled_game_ticks.saturating_add(delta);
            let per_packet = gap.as_secs_f64() * 1000.0 / delta as f64;
            receipt.mean_tick_ms = Some(
                self.elapsed.as_secs_f64() * 1000.0 / receipt.sampled_game_ticks as f64,
            );
            receipt.minimum_tick_ms = Some(
                receipt.minimum_tick_ms.map_or(per_packet, |old| old.min(per_packet)),
            );
            receipt.maximum_tick_ms = Some(
                receipt.maximum_tick_ms.map_or(per_packet, |old| old.max(per_packet)),
            );
            if delta == 1 && receipt.clean_interval_samples < CLEAN_SAMPLES {
                self.clean_ms[receipt.clean_interval_samples] = gap_ms;
                receipt.clean_interval_samples += 1;
                if receipt.clean_interval_samples == CLEAN_SAMPLES {
                    let mut sorted = self.clean_ms;
                    sorted.sort_unstable();
                    receipt.median_ms = Some(sorted[CLEAN_SAMPLES / 2]);
                }
            }
        }
        *receipt
    }

    /// Latest drain aggregate. Retain it before the next drain if needed.
    pub fn receipt(&self) -> TickHealthReceipt {
        self.receipt
    }
}

fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_batches_empty_drains_and_service_delay_are_separate() {
        let start = Instant::now();
        let mut health = TickHealth::default();
        let first = health.observe(1, 41, 40, start, None);
        assert_eq!(first.packet_delta, 1);
        assert_eq!(first.server_ticks, 1);
        assert_eq!(first.drain_gap_ms, None);
        let empty = health.observe(1, 41, 40, start + Duration::from_millis(20), None);
        assert_eq!(empty.packet_delta, 0);
        let batch = health.observe(
            1, 46, 40, start + Duration::from_secs(3), Some(Duration::from_millis(7)),
        );
        assert_eq!(batch.packet_delta, 5);
        assert_eq!(batch.server_ticks, 6);
        assert_eq!(batch.coalesced_observations, 4);
        assert_eq!(batch.total_coalesced_observations, 4);
        assert_eq!(batch.drain_gap_ms, Some(3000));
        assert_eq!(batch.host_service_delay_ms, Some(7));
        assert_eq!(batch.mean_tick_ms, Some(600.0));
        assert_eq!(batch.clean_interval_samples, 0);
    }

    #[test]
    fn long_receive_interval_never_invents_packets() {
        let start = Instant::now();
        let mut health = TickHealth::default();
        health.observe(1, 1, 0, start, None);
        let receipt = health.observe(1, 2, 0, start + Duration::from_secs(30), None);
        assert_eq!(receipt.packet_delta, 1);
        assert_eq!(receipt.coalesced_observations, 0);
        assert_eq!(receipt.drain_gap_ms, Some(30_000));
        assert_eq!(receipt.maximum_tick_ms, Some(30_000.0));
        assert_eq!(receipt.host_service_delay_ms, None);
    }

    #[test]
    fn session_replacement_excludes_old_packets_and_receive_time() {
        let start = Instant::now();
        let mut health = TickHealth::default();
        health.observe(1, 20, 0, start, None);
        health.observe(1, 21, 0, start + Duration::from_secs(1), None);
        let receipt = health.observe(
            2, 50, 48, start + Duration::from_secs(10), Some(Duration::from_secs(4)),
        );
        assert_eq!(receipt.packet_delta, 2);
        assert_eq!(receipt.server_ticks, 2);
        assert_eq!(receipt.drain_gap_ms, None);
        assert_eq!(receipt.host_service_delay_ms, None);
        assert_eq!(receipt.interval_samples, 0);
        assert_eq!(receipt.session_resets, 1);
    }

    #[test]
    fn generation_wrap_counts_real_packets() {
        let start = Instant::now();
        let mut health = TickHealth::default();
        health.observe(1, u64::MAX - 1, u64::MAX - 1, start, None);
        let receipt = health.observe(1, 1, u64::MAX - 1, start, None);
        assert_eq!(receipt.packet_delta, 3);
        assert_eq!(receipt.coalesced_observations, 2);
        assert_eq!(receipt.session_resets, 0);
    }

    #[test]
    fn clean_cadence_samples_are_bounded_and_do_not_include_batches() {
        let start = Instant::now();
        let mut health = TickHealth::default();
        for tick in 1..=12 {
            health.observe(1, tick, 0, start + Duration::from_millis(tick * 600), None);
        }
        assert_eq!(health.receipt().median_ms, Some(600));
        health.observe(1, 20, 0, start + Duration::from_secs(20), None);
        assert_eq!(health.receipt().clean_interval_samples, 11);
        assert_eq!(health.receipt().median_ms, Some(600));
    }

    #[test]
    fn fixed_storage_and_thousand_drains_allocate_nothing() {
        // Fixed storage plus at least 64 bytes of layout headroom.
        assert!(std::mem::size_of::<TickHealth>() <= 512);
        assert!(std::mem::size_of::<TickHealthReceipt>() <= 320);
        let start = Instant::now();
        let mut health = TickHealth::default();
        let allocations = allocation_counter::measure(|| {
            for tick in 0..1000 {
                std::hint::black_box(health.observe(
                    1, tick, 0, start + Duration::from_millis(tick * 600), None,
                ));
                std::hint::black_box(health.receipt());
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
}

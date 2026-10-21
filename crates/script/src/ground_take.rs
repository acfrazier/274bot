//! Pure exact-ground Take settlement and sparse per-run failed-key state.
//!
//! The caller captures pack and identical-row baselines when it queues a Take,
//! then constructs [`TakeSettlement`] with the actual dispatch receipt stamp.
//! This module owns no wall clock, scene copy, or polling allocation.

use api::quest_progress::EvidenceStamp;
use api::WorldTile;

/// Exact item identity at one world tile, including the plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroundKey {
    /// The exact object/item ID selected for Take.
    pub id: i32,
    /// Absolute tile and plane of the selected ground row.
    pub tile: WorldTile,
}

/// A typed refusal reported by the ground-pickup content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundRefusal {
    /// The inventory has no free capacity for the item.
    Capacity,
    /// This item cannot be picked up because a copy is already owned.
    AlreadyOwned {
        /// The item ID whose duplicate pickup was refused.
        id: i32,
    },
}

/// The settled result of one exact-ground Take attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeOutcome {
    /// The exact-ID pack count increased by this many units.
    Picked { units: i32 },
    /// The bound row was taken elsewhere or otherwise disappeared.
    LostRace,
    /// The attempt reached its observed-tick bound with the row still present.
    FailedKey,
    /// The attempt was invalidated or its settlement evidence was unavailable.
    Cancelled,
}

/// Number of later observed player-info edges allowed after dispatch.
pub const SETTLEMENT_TICKS: u64 = 4;
/// Observed game ticks to avoid a key after a true failed attempt.
pub use crate::quester::families::reach::AVOID_TICKS;
/// True failed attempts that permanently exclude a key for this run.
pub const MAX_KEY_FAILURES: u8 = 3;

/// Evidence-only state for one dispatched exact-ground Take.
///
/// The pack and row baselines are captured at queue emission. `dispatch_stamp`
/// must come from the receipt for the actual Take dispatch: no deadline runs
/// while the request is merely queued. Evaluation is allocation-free. A
/// returned outcome is consumed exactly once; later evaluations return
/// `None`.
#[derive(Debug)]
pub struct TakeSettlement {
    key: GroundKey,
    pack_baseline: i32,
    last_rows: usize,
    dispatch_stamp: EvidenceStamp,
    last_tick: u64,
    last_sequence: u64,
    first_zero_rows_tick: Option<u64>,
    row_decrease_observed: bool,
    settled: bool,
}

impl TakeSettlement {
    /// Start settlement from emission-time baselines and the actual dispatch.
    pub const fn new(
        key: GroundKey,
        pack_baseline: i32,
        identical_rows: usize,
        dispatch_stamp: EvidenceStamp,
    ) -> Self {
        Self {
            key,
            pack_baseline,
            last_rows: identical_rows,
            dispatch_stamp,
            last_tick: dispatch_stamp.tick,
            last_sequence: dispatch_stamp.sequence,
            first_zero_rows_tick: None,
            row_decrease_observed: false,
            settled: false,
        }
    }

    /// Exact object identity for this attempt.
    pub const fn key(&self) -> GroundKey {
        self.key
    }

    /// Actual dispatch receipt stamp, not the queued-emission tick.
    pub const fn dispatch_stamp(&self) -> EvidenceStamp {
        self.dispatch_stamp
    }

    /// Whether any valid observed row count decreased during this attempt.
    pub const fn row_decrease_observed(&self) -> bool {
        self.row_decrease_observed
    }

    /// Evaluate currently observed evidence, returning a terminal outcome once.
    ///
    /// `ready_scene` must be false when the scene is invalid or rebuilding.
    /// Missing counts are unknown, never zero. A positive pack delta takes
    /// precedence over every row observation. Only `EvidenceStamp::tick`
    /// advances the four-edge bound; repeated polls and wall time do not.
    pub fn evaluate(
        &mut self,
        evidence: EvidenceStamp,
        ready_scene: bool,
        pack_count: Option<i32>,
        identical_rows: Option<usize>,
    ) -> Option<TakeOutcome> {
        if self.settled {
            return None;
        }

        if !ready_scene || evidence.run != self.dispatch_stamp.run {
            return self.finish(TakeOutcome::Cancelled);
        }

        // Ignore pre-dispatch or out-of-order evidence. A same-tick snapshot
        // after the dispatch sequence is valid for an immediate floor pickup.
        if !evidence.meets(self.dispatch_stamp)
            || evidence.tick < self.last_tick
            || (evidence.tick == self.last_tick && evidence.sequence < self.last_sequence)
        {
            return None;
        }
        self.last_tick = evidence.tick;
        self.last_sequence = evidence.sequence;

        // OBJ_TAKEITEM updates inventory before removing the object. This must
        // precede row-loss handling, including on the final fence edge.
        if let Some(pack_count) = pack_count.filter(|count| *count > self.pack_baseline) {
            return self.finish(TakeOutcome::Picked {
                units: pack_count.saturating_sub(self.pack_baseline),
            });
        }

        if let Some(rows) = identical_rows {
            let decreased = rows < self.last_rows;
            self.last_rows = rows;
            if decreased {
                self.row_decrease_observed = true;
            }
            if decreased && rows == 0 && self.first_zero_rows_tick.is_none() {
                self.first_zero_rows_tick = Some(evidence.tick);
            }
        }

        let pack_unchanged = pack_count == Some(self.pack_baseline);
        if pack_unchanged
            && self
                .first_zero_rows_tick
                .is_some_and(|zero_tick| evidence.tick > zero_tick)
        {
            return self.finish(TakeOutcome::LostRace);
        }

        let ticks_after_dispatch = evidence.tick.saturating_sub(self.dispatch_stamp.tick);
        if ticks_after_dispatch < SETTLEMENT_TICKS {
            return None;
        }

        // The fourth later edge is the final inventory fence. At that point an
        // observed decrease is a penalty-free race; an unchanged positive row
        // is a true failed key. Missing or changed evidence cancels neutrally.
        match (pack_count, identical_rows) {
            (Some(pack), Some(rows)) if pack == self.pack_baseline => {
                if self.row_decrease_observed {
                    self.finish(TakeOutcome::LostRace)
                } else if rows > 0 {
                    self.finish(TakeOutcome::FailedKey)
                } else {
                    self.finish(TakeOutcome::Cancelled)
                }
            }
            _ => self.finish(TakeOutcome::Cancelled),
        }
    }

    fn finish(&mut self, outcome: TakeOutcome) -> Option<TakeOutcome> {
        self.settled = true;
        Some(outcome)
    }
}

/// Per-key disposition without allocating a diagnostic string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStatus {
    /// The key is currently cooling down after this many failures.
    Avoided {
        /// Number of true failed attempts recorded for this key.
        failures: u8,
        /// First tick at which the key is eligible again, if representable.
        expires_at: Option<u64>,
    },
    /// The key has a nonterminal record but its cooldown has expired.
    Eligible {
        /// Number of true failed attempts recorded for this key.
        failures: u8,
    },
    /// The third true failure excluded this key for the rest of the run.
    Excluded {
        /// Number of true failed attempts recorded for this key.
        failures: u8,
    },
}

/// One sparse true-failure record for an exact ID/tile/plane key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureRecord {
    /// Failed object identity, including its tile and plane.
    pub key: GroundKey,
    /// Observed tick of the latest true failed attempt.
    pub last_failure_tick: u64,
    /// Number of true failed attempts, capped at [`MAX_KEY_FAILURES`].
    pub failures: u8,
    /// Whether this key is permanently excluded for this run.
    pub excluded: bool,
}

impl FailureRecord {
    fn expires_at(self) -> Option<u64> {
        self.last_failure_tick.checked_add(AVOID_TICKS)
    }

    fn is_avoided(self, tick: u64) -> bool {
        !self.excluded && tick.saturating_sub(self.last_failure_tick) < AVOID_TICKS
    }
}

/// Sparse failed-key ledger. It allocates storage only after a true failure.
///
/// Keep one ledger for a run and create a fresh one only when the caller starts
/// a new run. Lost races, refusals, and cancellations must not be recorded.
#[derive(Debug, Default)]
pub struct FailureLedger {
    records: Vec<FailureRecord>,
}

impl FailureLedger {
    /// Record one true failed-key outcome at its observed settlement tick.
    ///
    /// Returns the resulting status. A permanently excluded key is unchanged.
    pub fn record_failure(&mut self, key: GroundKey, tick: u64) -> FailureStatus {
        if let Some(record) = self.records.iter_mut().find(|record| record.key == key) {
            if record.excluded {
                return FailureStatus::Excluded {
                    failures: record.failures,
                };
            }
            record.failures = record.failures.saturating_add(1).min(MAX_KEY_FAILURES);
            record.last_failure_tick = tick;
            record.excluded = record.failures >= MAX_KEY_FAILURES;
            return if record.excluded {
                FailureStatus::Excluded {
                    failures: record.failures,
                }
            } else {
                FailureStatus::Avoided {
                    failures: record.failures,
                    expires_at: record.last_failure_tick.checked_add(AVOID_TICKS),
                }
            };
        }

        self.records.push(FailureRecord {
            key,
            failures: 1,
            last_failure_tick: tick,
            excluded: false,
        });
        FailureStatus::Avoided {
            failures: 1,
            expires_at: tick.checked_add(AVOID_TICKS),
        }
    }

    /// Whether a key may be selected at this observed tick.
    pub fn is_eligible(&self, key: GroundKey, tick: u64) -> bool {
        self.records
            .iter()
            .find(|record| record.key == key)
            .is_none_or(|record| !record.excluded && !record.is_avoided(tick))
    }

    /// Earliest future cooldown expiry, excluding terminal keys.
    pub fn next_expiry(&self, tick: u64) -> Option<u64> {
        self.records
            .iter()
            .filter(|record| record.is_avoided(tick))
            .filter_map(|record| record.expires_at())
            .filter(|expires_at| *expires_at > tick)
            .min()
    }

    /// Current sparse records, borrowed without allocation.
    pub fn records(&self) -> &[FailureRecord] {
        &self.records
    }

    /// Current reason/status for a key, if a true failure record exists.
    pub fn status(&self, key: GroundKey, tick: u64) -> Option<FailureStatus> {
        let record = self.records.iter().find(|record| record.key == key)?;
        Some(if record.excluded {
            FailureStatus::Excluded {
                failures: record.failures,
            }
        } else if record.is_avoided(tick) {
            FailureStatus::Avoided {
                failures: record.failures,
                expires_at: record.expires_at(),
            }
        } else {
            FailureStatus::Eligible {
                failures: record.failures,
            }
        })
    }

    /// Clear a successful key's nonterminal failure record.
    ///
    /// A terminal exclusion is retained for the rest of this run.
    pub fn record_success(&mut self, key: GroundKey) -> bool {
        let Some(index) = self.records.iter().position(|record| record.key == key) else {
            return false;
        };
        if self.records[index].excluded {
            return false;
        }
        self.records.swap_remove(index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::RunKey;

    const RUN: RunKey = RunKey {
        slot: 4,
        run: 12,
        session: 8,
    };
    const KEY: GroundKey = GroundKey {
        id: 1053,
        tile: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
    };

    fn stamp(tick: u64, sequence: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: RUN,
            tick,
            sequence,
        }
    }

    fn settlement(rows: usize) -> TakeSettlement {
        TakeSettlement::new(KEY, 4, rows, stamp(100, 700))
    }

    fn observe(
        settlement: &mut TakeSettlement,
        tick: u64,
        pack: Option<i32>,
        rows: Option<usize>,
    ) -> Option<TakeOutcome> {
        settlement.evaluate(stamp(tick, 700 + tick), true, pack, rows)
    }

    #[test]
    fn positive_pack_delta_wins_over_row_loss_and_is_consumed_once() {
        let mut attempt = settlement(2);
        assert_eq!(
            observe(&mut attempt, 101, Some(7), Some(0)),
            Some(TakeOutcome::Picked { units: 3 })
        );
        assert_eq!(observe(&mut attempt, 102, Some(7), Some(0)), None);
        assert!(!attempt.row_decrease_observed());
    }

    #[test]
    fn immediate_floor_delta_can_settle_on_the_dispatch_tick() {
        let mut attempt = settlement(1);
        assert_eq!(
            attempt.evaluate(stamp(100, 701), true, Some(5), Some(1)),
            Some(TakeOutcome::Picked { units: 1 })
        );
    }

    #[test]
    fn the_deadline_starts_at_dispatch_and_needs_four_later_edges() {
        let mut attempt = settlement(1);
        for sequence in 701..=709 {
            assert_eq!(
                attempt.evaluate(stamp(100, sequence), true, Some(4), Some(1)),
                None,
                "same-tick polls do not advance the observed-tick deadline"
            );
        }
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(1)), None);
        assert_eq!(observe(&mut attempt, 102, Some(4), Some(1)), None);
        assert_eq!(observe(&mut attempt, 103, Some(4), Some(1)), None);
        assert_eq!(
            observe(&mut attempt, 104, Some(4), Some(1)),
            Some(TakeOutcome::FailedKey)
        );
        assert_eq!(observe(&mut attempt, 105, Some(4), Some(1)), None);
    }

    #[test]
    fn zero_rows_require_a_strictly_later_edge_before_lost_race() {
        let mut attempt = settlement(2);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(0)), None);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(0)), None);
        assert_eq!(
            observe(&mut attempt, 102, Some(4), Some(2)),
            Some(TakeOutcome::LostRace)
        );
        assert_eq!(observe(&mut attempt, 103, Some(4), Some(2)), None);
    }

    #[test]
    fn zero_rows_at_the_final_fence_settle_without_an_extra_edge() {
        let mut attempt = settlement(1);
        assert_eq!(
            observe(&mut attempt, 104, Some(4), Some(0)),
            Some(TakeOutcome::LostRace)
        );
    }

    #[test]
    fn positive_row_decrease_latches_through_replenishment_to_the_bound() {
        let mut attempt = settlement(2);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(1)), None);
        assert!(attempt.row_decrease_observed());
        assert_eq!(observe(&mut attempt, 102, Some(4), Some(2)), None);
        assert_eq!(observe(&mut attempt, 103, Some(4), Some(3)), None);
        assert_eq!(
            observe(&mut attempt, 104, Some(4), Some(2)),
            Some(TakeOutcome::LostRace)
        );
    }

    #[test]
    fn any_decrease_from_the_last_observed_count_latches() {
        let mut attempt = settlement(2);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(3)), None);
        assert_eq!(observe(&mut attempt, 102, Some(4), Some(2)), None);
        assert!(attempt.row_decrease_observed());
        assert_eq!(observe(&mut attempt, 103, Some(4), Some(3)), None);
        assert_eq!(
            observe(&mut attempt, 104, Some(4), Some(3)),
            Some(TakeOutcome::LostRace)
        );
    }

    #[test]
    fn zero_then_replenished_row_keeps_its_latched_removal_edge() {
        let mut attempt = settlement(2);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(0)), None);
        assert_eq!(
            observe(&mut attempt, 102, Some(4), Some(2)),
            Some(TakeOutcome::LostRace)
        );
    }

    #[test]
    fn delayed_success_on_fourth_edge_beats_failure_and_row_state() {
        let mut attempt = settlement(1);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(1)), None);
        assert_eq!(observe(&mut attempt, 102, Some(4), Some(1)), None);
        assert_eq!(observe(&mut attempt, 103, Some(4), Some(1)), None);
        assert_eq!(
            observe(&mut attempt, 104, Some(5), Some(1)),
            Some(TakeOutcome::Picked { units: 1 })
        );
    }

    #[test]
    fn unavailable_counts_never_prove_removal_or_a_failed_key() {
        let mut attempt = settlement(1);
        assert_eq!(observe(&mut attempt, 101, Some(4), None), None);
        assert_eq!(
            observe(&mut attempt, 104, Some(4), None),
            Some(TakeOutcome::Cancelled)
        );
        assert_eq!(observe(&mut attempt, 105, Some(4), Some(0)), None);

        let mut pack_unavailable = settlement(1);
        assert_eq!(
            observe(&mut pack_unavailable, 104, None, Some(1)),
            Some(TakeOutcome::Cancelled)
        );
    }

    #[test]
    fn an_unavailable_row_after_a_latched_zero_does_not_erase_prior_proof() {
        let mut attempt = settlement(1);
        assert_eq!(observe(&mut attempt, 101, Some(4), Some(0)), None);
        assert_eq!(
            observe(&mut attempt, 102, Some(4), None),
            Some(TakeOutcome::LostRace)
        );
    }

    #[test]
    fn invalid_scene_and_run_change_cancel_before_counting_pack_delta() {
        let mut invalid_scene = settlement(1);
        assert_eq!(
            invalid_scene.evaluate(stamp(101, 801), false, Some(8), Some(0)),
            Some(TakeOutcome::Cancelled)
        );
        assert_eq!(observe(&mut invalid_scene, 104, Some(8), Some(0)), None);

        let mut changed_run = settlement(1);
        let changed = EvidenceStamp {
            run: RunKey {
                session: RUN.session + 1,
                ..RUN
            },
            tick: 101,
            sequence: 801,
        };
        assert_eq!(
            changed_run.evaluate(changed, true, Some(8), Some(0)),
            Some(TakeOutcome::Cancelled)
        );
    }

    #[test]
    fn pre_dispatch_and_out_of_order_evidence_are_ignored() {
        let mut attempt = settlement(1);
        assert_eq!(
            attempt.evaluate(stamp(99, 999), true, Some(8), Some(0)),
            None
        );
        assert_eq!(
            attempt.evaluate(stamp(100, 699), true, Some(8), Some(0)),
            None
        );
        assert_eq!(
            attempt.evaluate(stamp(101, 801), true, Some(4), Some(1)),
            None
        );
        assert_eq!(
            attempt.evaluate(stamp(100, 702), true, Some(4), Some(1)),
            None
        );
    }

    #[test]
    fn three_races_do_not_penalize_a_later_respawn() {
        let mut ledger = FailureLedger::default();
        for attempt_number in 0..3_u64 {
            let dispatch = 100 + attempt_number * 10;
            let mut attempt = TakeSettlement::new(KEY, 4, 2, stamp(dispatch, 700 + dispatch));
            assert_eq!(
                attempt.evaluate(
                    stamp(dispatch + 1, 700 + dispatch + 1),
                    true,
                    Some(4),
                    Some(1),
                ),
                None
            );
            assert_eq!(
                attempt.evaluate(
                    stamp(dispatch + 2, 700 + dispatch + 2),
                    true,
                    Some(4),
                    Some(2),
                ),
                None
            );
            assert_eq!(
                attempt.evaluate(
                    stamp(dispatch + 4, 700 + dispatch + 4),
                    true,
                    Some(4),
                    Some(2),
                ),
                Some(TakeOutcome::LostRace)
            );
            assert!(ledger.is_eligible(KEY, dispatch + 4));
            assert!(ledger.records().is_empty());
        }

        let dispatch = 140;
        let mut respawn = TakeSettlement::new(KEY, 4, 1, stamp(dispatch, 840));
        assert_eq!(
            respawn.evaluate(stamp(dispatch + 1, 841), true, Some(5), Some(1)),
            Some(TakeOutcome::Picked { units: 1 })
        );
        assert!(ledger.is_eligible(KEY, dispatch + 1));
        assert!(ledger.records().is_empty());
    }

    #[test]
    fn true_failures_avoid_ten_ticks_and_exclude_on_the_third() {
        let mut ledger = FailureLedger::default();
        for failure in 1..=3 {
            let failed_at = u64::from(failure) * 20;
            let mut attempt = TakeSettlement::new(KEY, 4, 1, stamp(failed_at - 4, 900 + failed_at));
            assert_eq!(
                attempt.evaluate(stamp(failed_at, 900 + failed_at), true, Some(4), Some(1)),
                Some(TakeOutcome::FailedKey)
            );
            let status = ledger.record_failure(KEY, failed_at);
            if failure < MAX_KEY_FAILURES {
                assert_eq!(
                    status,
                    FailureStatus::Avoided {
                        failures: failure,
                        expires_at: Some(failed_at + AVOID_TICKS),
                    }
                );
                assert!(!ledger.is_eligible(KEY, failed_at + AVOID_TICKS - 1));
                assert!(ledger.is_eligible(KEY, failed_at + AVOID_TICKS));
                assert_eq!(ledger.next_expiry(failed_at), Some(failed_at + AVOID_TICKS));
            } else {
                assert_eq!(
                    status,
                    FailureStatus::Excluded {
                        failures: MAX_KEY_FAILURES
                    }
                );
                assert!(!ledger.is_eligible(KEY, u64::MAX));
                assert_eq!(ledger.next_expiry(failed_at), None);
            }
        }
        assert_eq!(ledger.records().len(), 1);
        assert_eq!(ledger.records()[0].failures, MAX_KEY_FAILURES);
        assert!(ledger.records()[0].excluded);

        let other_id_same_tile = GroundKey {
            id: KEY.id + 1,
            ..KEY
        };
        let other_plane = GroundKey {
            tile: WorldTile {
                level: 1,
                ..KEY.tile
            },
            ..KEY
        };
        assert!(ledger.is_eligible(other_id_same_tile, u64::MAX));
        assert!(ledger.is_eligible(other_plane, u64::MAX));
    }

    #[test]
    fn successful_attempt_clears_only_nonterminal_failure_record() {
        let mut ledger = FailureLedger::default();
        ledger.record_failure(KEY, 10);
        assert!(ledger.record_success(KEY));
        assert!(ledger.records().is_empty());

        for tick in [20, 40, 60] {
            ledger.record_failure(KEY, tick);
        }
        assert!(!ledger.record_success(KEY));
        assert_eq!(
            ledger.status(KEY, 61),
            Some(FailureStatus::Excluded { failures: 3 })
        );
    }

    #[test]
    fn failure_keys_include_id_tile_and_plane() {
        let mut ledger = FailureLedger::default();
        ledger.record_failure(KEY, 10);
        let other_id = GroundKey {
            id: KEY.id + 1,
            ..KEY
        };
        let other_tile = GroundKey {
            tile: WorldTile {
                x: KEY.tile.x + 1,
                ..KEY.tile
            },
            ..KEY
        };
        let other_plane = GroundKey {
            tile: WorldTile {
                level: 1,
                ..KEY.tile
            },
            ..KEY
        };
        assert!(!ledger.is_eligible(KEY, 10));
        assert!(ledger.is_eligible(other_id, 10));
        assert!(ledger.is_eligible(other_tile, 10));
        assert!(ledger.is_eligible(other_plane, 10));
    }

    #[test]
    fn refusal_types_keep_capacity_and_duplicate_reason_typed() {
        assert_ne!(
            GroundRefusal::Capacity,
            GroundRefusal::AlreadyOwned { id: 1053 }
        );
        assert_eq!(
            GroundRefusal::AlreadyOwned { id: 1053 },
            GroundRefusal::AlreadyOwned { id: 1053 }
        );
    }

    #[test]
    fn per_attempt_and_sparse_failure_storage_stay_compact() {
        assert!(std::mem::size_of::<GroundKey>() <= 16);
        assert!(std::mem::size_of::<TakeSettlement>() <= 128);
        assert!(std::mem::size_of::<FailureRecord>() <= 32);
        assert_eq!(
            std::mem::size_of::<FailureLedger>(),
            std::mem::size_of::<Vec<FailureRecord>>()
        );
        let mut ledger = FailureLedger::default();
        assert!(ledger.records.is_empty());
        assert_eq!(
            ledger.records.capacity(),
            0,
            "idle ledger has no heap allocation"
        );
        ledger.record_failure(KEY, 10);
        let capacity = ledger.records.capacity();
        assert!(capacity >= 1);
        ledger.record_failure(KEY, 20);
        assert_eq!(ledger.records.len(), 1);
        assert_eq!(ledger.records.capacity(), capacity);
        let record_storage = capacity.saturating_mul(std::mem::size_of::<FailureRecord>());
        println!(
            "ground Take layout: settle={}B key={}B record={}B ledger={}B failed-n={} capacity={} payload={}B",
            std::mem::size_of::<TakeSettlement>(),
            std::mem::size_of::<GroundKey>(),
            std::mem::size_of::<FailureRecord>(),
            std::mem::size_of::<FailureLedger>(),
            ledger.records.len(),
            capacity,
            record_storage,
        );
    }
}

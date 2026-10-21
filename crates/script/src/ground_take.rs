//! Sparse per-run failed-key policy for exact ground Takes.
//!
//! The shared evidence-only settlement lives in [`api::ground_take`].
//! This ledger owns only temporary avoidance and terminal run exclusions.

use api::ground_take::GroundKey;

/// Observed game ticks to avoid a key after a true failed attempt.
pub use crate::quester::families::reach::AVOID_TICKS;
/// True failed attempts that permanently exclude a key for this run.
pub const MAX_KEY_FAILURES: u8 = 3;

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
    use api::ground_take::{TakeOutcome, TakeSettlement};
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::WorldTile;

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

    #[test]
    fn three_races_do_not_penalize_a_later_respawn() {
        let ledger = FailureLedger::default();
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

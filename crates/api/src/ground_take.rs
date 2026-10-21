//! Pure exact-ground Take settlement shared by snapshot consumers.
//!
//! Capture pack and identical-row baselines when queuing a Take, then start
//! settlement with its actual dispatch receipt. This core owns no wall clock,
//! scene copy, action emission, or per-run failure policy.

use crate::quest_progress::EvidenceStamp;
use crate::WorldTile;

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
    Picked {
        /// Confirmed increase in the exact-ID inventory count.
        units: i32,
    },
    /// The bound row was taken elsewhere or otherwise disappeared.
    LostRace,
    /// The attempt reached its observed-tick bound with the row still present.
    FailedKey,
    /// The attempt was invalidated or its settlement evidence was unavailable.
    Cancelled,
}

/// Number of later observed player-info edges allowed after dispatch.
pub const SETTLEMENT_TICKS: u64 = 4;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::RunKey;

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

    fn stamp(tick: u64, sequence: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: RUN,
            tick,
            sequence,
        }
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
    fn evidence_only_attempt_storage_stays_compact() {
        assert_eq!(std::mem::size_of::<GroundKey>(), 16);
        assert_eq!(std::mem::size_of::<GroundRefusal>(), 8);
        assert_eq!(std::mem::size_of::<TakeOutcome>(), 8);
        assert!(std::mem::size_of::<TakeSettlement>() <= 128);
        println!(
            "ground API layout: key={}B refusal={}B outcome={}B settlement={}B",
            std::mem::size_of::<GroundKey>(),
            std::mem::size_of::<GroundRefusal>(),
            std::mem::size_of::<TakeOutcome>(),
            std::mem::size_of::<TakeSettlement>()
        );
    }
}

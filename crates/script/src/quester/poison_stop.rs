//! Operator decision D8: the Quester is poisoned at low HP and holds no food
//! and no cure it may use. It cures first when it can (the walk guard and
//! [`crate::survival::SurvivalStep`] drink a held antipoison through
//! [`crate::survival::cure`]); otherwise it walks to a tile outside the
//! stage's danger area and stops there, with the reason in its status.
//!
//! "Low HP" is the shared survival need [`SurvivalNeed::Cure`]: HP at or below
//! the poison-adjusted emergency line, or a poison outlook whose remaining
//! damage reaches HP ([`crate::survival::math::survival_need`]). Both come from
//! the content poison script: each `poison` timer fire queues
//! `(%poison + 4) / 5` damage and lowers `%poison` by one
//! (`poison.rs2:20-30`), and the timer runs every 30 ticks
//! (`poison.rs2:72-75`); `api::condition::poison_math` sums that series.
//!
//! "Safe" is a tile outside every zone that the stage and the current walk
//! step declare in `cross`: the danger area the Path accepted for this
//! stage. Routing keeps a walk out of every other zone.
//!
//! The stop is the survival park (design A6.5). It cannot wait forever:
//! poison ends when `%poison` reaches 0 and the timer clears
//! (`poison.rs2:21-23`), at most 55 fires (the largest R289
//! `poison_severity`) or 1650 ticks after onset; poison damage that reaches
//! 0 HP queues `player_death` (`poison.rs2:59-61`), which ends the session's
//! live condition. The host suppresses the client idle timer by default, so
//! no idle logout ends a park early (`Client::set_host_idle_timer`). The park
//! ends when fresh evidence shows that the poison can no longer kill.

use std::fmt;
use std::sync::Arc;

use api::condition::FoodSummary;
use api::WorldTile;
use nav_planner::contract::{ZoneTableView, ZoneView};

use crate::native::walk::Walk;
use crate::native::ActionHandle;
use crate::survival::math::SurvivalNeed;

/// Whether the survival need is the D8 state: a poison cure is needed and
/// the inventory holds no food. A [`SurvivalNeed::Cure`] reaches the runner
/// only when no allowed cure is held or clickable
/// (`crate::survival::step::SurvivalStep::want`).
pub(super) fn stranded(need: SurvivalNeed, food: &FoodSummary) -> bool {
    matches!(need, SurvivalNeed::Cure { .. }) && food.ordinary_count == 0 && food.combo_count == 0
}

/// What the runner does for a D8 need on this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopPlan {
    /// Stop here: the tile is outside the danger area.
    Park,
    /// The current walk step leaves the danger area: keep following it.
    KeepWalking,
    /// Walk back to this last observed safe tile, then stop.
    Retreat(WorldTile),
    /// Stop inside the danger area, because no safe walk is known.
    ParkInDanger(InDanger),
}

/// Why a stop is inside the danger area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InDanger {
    /// The zone table or the player's tile is not available.
    AreaUnknown,
    /// No tile outside the danger area was observed in this stage.
    NoSafeTile,
    /// The walk out ended without arrival.
    WalkOutFailed,
}

/// Choose the D8 response. `here` and `walk_target` say whether the player's
/// tile and the current walk step's target are inside the danger area
/// (`None`: unknown, or no walk step). `last_safe` is the last observed tile
/// outside the area.
pub(super) fn plan(
    here: Option<bool>,
    walk_target: Option<bool>,
    last_safe: Option<WorldTile>,
) -> StopPlan {
    match here {
        None => StopPlan::ParkInDanger(InDanger::AreaUnknown),
        Some(false) => StopPlan::Park,
        Some(true) if walk_target == Some(false) => StopPlan::KeepWalking,
        Some(true) => last_safe.map_or(
            StopPlan::ParkInDanger(InDanger::NoSafeTile),
            StopPlan::Retreat,
        ),
    }
}

/// The zones of the stage's danger area: the stage `cross` ids and the
/// current walk step's `cross` ids.
pub(super) struct DangerArea<'a> {
    pub(super) stage: &'a [Arc<str>],
    pub(super) step: &'a [Arc<str>],
}

/// A running walk step's target and its declared `cross` ids
/// (`StepPlan::walk_target`).
pub(super) type WalkTarget<'a> = (WorldTile, &'a [Arc<str>]);

impl DangerArea<'_> {
    /// Whether `tile` is inside the area. `None` when the zone table is
    /// unavailable and the area is not empty.
    pub(super) fn contains(
        &self,
        zones: Option<ZoneTableView<'_>>,
        tile: WorldTile,
    ) -> Option<bool> {
        if self.stage.is_empty() && self.step.is_empty() {
            return Some(false);
        }
        let zones = zones?;
        Some(zones.containing(tile).any(|zone| {
            self.stage
                .iter()
                .chain(self.step)
                .any(|id| names_zone(zones, zone, id))
        }))
    }
}

/// Whether a `cross` id names `zone`: its hazard kind, its exact spawn
/// identity, or its group (`ZoneTable::resolve`).
fn names_zone(zones: ZoneTableView<'_>, zone: ZoneView<'_>, id: &str) -> bool {
    zone.kind().id() == id
        || zones
            .resolve(id)
            .is_some_and(|key| key == zone.key() || zone.group() == Some(key))
}

/// The D8 stop's reason, built once for the status line and the log.
#[derive(Debug, Clone)]
pub(super) struct Stopped {
    pub(super) reason: Arc<str>,
}

impl Stopped {
    /// `tile` is where the player stopped; `danger` says why that tile is
    /// inside the danger area, if it is.
    pub(super) fn new(tile: Option<WorldTile>, danger: Option<InDanger>) -> Self {
        Self {
            reason: Arc::from(StopText { tile, danger }.to_string()),
        }
    }
}

struct StopText {
    tile: Option<WorldTile>,
    danger: Option<InDanger>,
}

impl fmt::Display for StopText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("poisoned at low HP with no food and no antipoison: stopped")?;
        if let Some(tile) = self.tile {
            write!(f, " at {},{},{}", tile.x, tile.z, tile.level)?;
        }
        match self.danger {
            None => f.write_str(" outside the danger area"),
            Some(InDanger::AreaUnknown) => {
                f.write_str(" where the danger area is unknown (no zone table or tile)")
            }
            Some(InDanger::NoSafeTile) => {
                f.write_str(" inside the danger area (no safe tile seen in this stage)")
            }
            Some(InDanger::WalkOutFailed) => {
                f.write_str(" inside the danger area (the walk out did not arrive)")
            }
        }
    }
}

/// The runner's D8 state: the last safe tile, and the D8 phase behind one
/// box. A D8 state is rare, and the Quester's per-bot layout is bounded
/// (`quester_struct_fits_the_per_bot_budget`).
#[derive(Default)]
pub(super) struct PoisonStop {
    /// Last observed tile outside the danger area.
    pub(super) last_safe: Option<WorldTile>,
    phase: Option<Box<Phase>>,
}

enum Phase {
    /// The current walk step is kept as the walk out; its log line is out.
    WalkingOut,
    /// The walk back to the last safe tile, and the need that started it.
    Retreat(ActionHandle<Walk>, SurvivalNeed),
    /// The stop, while the survival park holds.
    Stopped(Stopped),
}

impl PoisonStop {
    /// The walk out to the last safe tile, while it runs.
    pub(super) fn retreat(&self) -> Option<&ActionHandle<Walk>> {
        match self.phase.as_deref() {
            Some(Phase::Retreat(handle, _)) => Some(handle),
            _ => None,
        }
    }

    pub(super) fn start_retreat(&mut self, handle: ActionHandle<Walk>, need: SurvivalNeed) {
        self.phase = Some(Box::new(Phase::Retreat(handle, need)));
    }

    /// End the walk out and return the need that started it. Dropping the
    /// walk handle revokes the host request.
    pub(super) fn end_retreat(&mut self) -> Option<SurvivalNeed> {
        self.retreat()?;
        match self.phase.take().map(|phase| *phase) {
            Some(Phase::Retreat(_, need)) => Some(need),
            _ => None,
        }
    }

    /// The stop, while the survival park holds.
    pub(super) fn stopped(&self) -> Option<&Stopped> {
        match self.phase.as_deref() {
            Some(Phase::Stopped(stopped)) => Some(stopped),
            _ => None,
        }
    }

    pub(super) fn stop(&mut self, stopped: Stopped) {
        self.phase = Some(Box::new(Phase::Stopped(stopped)));
    }

    /// End the stop at the un-park and return it.
    pub(super) fn end_stop(&mut self) -> Option<Stopped> {
        self.stopped()?;
        match self.phase.take().map(|phase| *phase) {
            Some(Phase::Stopped(stopped)) => Some(stopped),
            _ => None,
        }
    }

    /// Keep the current walk as the walk out. True the first time, so the
    /// caller logs it once.
    pub(super) fn walk_out(&mut self) -> bool {
        if self.phase.is_some() {
            return false;
        }
        self.phase = Some(Box::new(Phase::WalkingOut));
        true
    }

    pub(super) fn walking_out(&self) -> bool {
        matches!(self.phase.as_deref(), Some(Phase::WalkingOut))
    }

    /// The need is gone before any stop: forget a kept walk out.
    pub(super) fn need_gone(&mut self) {
        if self.walking_out() {
            self.phase = None;
        }
    }

    /// Forget the walk out and the stop. The last safe tile stays valid.
    pub(super) fn clear(&mut self) {
        self.phase = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32) -> WorldTile {
        WorldTile { x, z: 0, level: 0 }
    }

    #[test]
    fn stranded_needs_a_cure_need_and_no_food() {
        let cure = SurvivalNeed::Cure {
            or_food: 3,
            ref_heal: 12,
            why: crate::survival::math::NeedWhy::Lethal,
        };
        let food_need = SurvivalNeed::Food {
            count: 2,
            ref_heal: 12,
        };
        let none = FoodSummary::default();
        assert!(stranded(cure, &none));
        assert!(!stranded(food_need, &none), "a food need is not poison");
        let ordinary = FoodSummary {
            ordinary_count: 1,
            ..FoodSummary::default()
        };
        assert!(!stranded(cure, &ordinary), "held food: the eater acts");
        let combo = FoodSummary {
            combo_count: 1,
            ..FoodSummary::default()
        };
        assert!(!stranded(cure, &combo), "held combo food: the eater acts");
    }

    #[test]
    fn plan_stops_only_outside_the_danger_area() {
        let safe = Some(tile(1));
        assert_eq!(plan(Some(false), None, safe), StopPlan::Park);
        assert_eq!(plan(Some(false), Some(true), safe), StopPlan::Park);
        assert_eq!(plan(Some(true), Some(false), safe), StopPlan::KeepWalking);
        assert_eq!(plan(Some(true), Some(false), None), StopPlan::KeepWalking);
        assert_eq!(
            plan(Some(true), Some(true), safe),
            StopPlan::Retreat(tile(1))
        );
        assert_eq!(plan(Some(true), None, safe), StopPlan::Retreat(tile(1)));
        assert_eq!(
            plan(Some(true), None, None),
            StopPlan::ParkInDanger(InDanger::NoSafeTile)
        );
        assert_eq!(
            plan(None, Some(false), safe),
            StopPlan::ParkInDanger(InDanger::AreaUnknown)
        );
    }

    #[test]
    fn stop_text_names_the_reason_tile_and_safety() {
        let outside = Stopped::new(
            Some(WorldTile {
                x: 2880,
                z: 3600,
                level: 0,
            }),
            None,
        );
        assert_eq!(
            outside.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped at 2880,3600,0 outside the danger area"
        );
        let inside = Stopped::new(None, Some(InDanger::NoSafeTile));
        assert_eq!(
            inside.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped inside the danger area (no safe tile seen in this stage)"
        );
    }

    #[test]
    fn danger_area_is_the_declared_cross_zones_only() {
        let lease = crate::gatherer::select::tests::test_zone_lease(
            &["throwers", "other"],
            (2, 5, 0, 5),
            tile(0),
        );
        let zones = lease.zones();
        let declared: [Arc<str>; 1] = [Arc::from("throwers")];
        let step_area = DangerArea {
            stage: &[],
            step: &declared,
        };
        let stage_area = DangerArea {
            stage: &declared,
            step: &[],
        };
        for area in [&step_area, &stage_area] {
            assert_eq!(area.contains(zones, tile(3)), Some(true));
            assert_eq!(area.contains(zones, tile(1)), Some(false));
            assert_eq!(area.contains(zones, tile(6)), Some(false));
            assert_eq!(area.contains(None, tile(3)), None, "no table is unknown");
        }
        let undeclared: [Arc<str>; 1] = [Arc::from("elsewhere")];
        let other = DangerArea {
            stage: &undeclared,
            step: &[],
        };
        assert_eq!(other.contains(zones, tile(3)), Some(false));
        let empty = DangerArea {
            stage: &[],
            step: &[],
        };
        assert_eq!(
            empty.contains(None, tile(3)),
            Some(false),
            "nothing declared"
        );
    }
}

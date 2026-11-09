//! Frozen cook-location inputs: the curated bank cook surfaces, the surface
//! kinds and the pairing constants, generated from the local game-data sources;
//! the placed surfaces themselves are selected content.

use crate::selected::embedded_table;
use crate::selected::FactError;
use crate::snapshot::WorldTile;
use serde::Deserialize;

/// Which cooking branch a surface takes: an oven burns less than a fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CookSurfaceKind {
    Oven,
    Fire,
}

impl CookSurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oven => "oven",
            Self::Fire => "fire",
        }
    }
}

/// One hand-walked bank cook surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookCamp {
    pub bank: &'static str,
    pub stand: WorldTile,
    pub approach: Option<WorldTile>,
    pub loc: WorldTile,
    pub loc_name: &'static str,
    pub kind: CookSurfaceKind,
    pub label: &'static str,
}

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
mod generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/cook_catalog.rs"));
    include!(concat!(env!("OUT_DIR"), "/fish_camp_cook_plans.rs"));
}

/// The loc a fish-camp cook surface uses (`CookSurfaceKind`, `data/cookingRanges.ts:9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishCookKind {
    /// A `Range`.
    Range,
    /// A `Fire`.
    Fire,
    /// A `Fireplace`.
    Fireplace,
}

impl FishCookKind {
    /// The `kind` string the script API publishes.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Range => "range",
            Self::Fire => "fire",
            Self::Fireplace => "fireplace",
        }
    }
}

/// One curated fish-camp cook surface (`CookingSurface`, `data/cookingRanges.ts:11-23`), generated from the
/// pinned source. `approach_first` records that the literal wrote `approach` before `stand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishCookSurface {
    /// The literal wrote `approach` as its first key.
    pub approach_first: bool,
    /// Waypoint walked first, when the surface has one.
    pub approach: Option<WorldTile>,
    /// Final stand next to the surface.
    pub stand: WorldTile,
    /// The loc's south-west tile, when the literal sets it.
    pub loc: Option<WorldTile>,
    /// Loc query name.
    pub loc_name: &'static str,
    /// Surface kind.
    pub kind: FishCookKind,
    /// Display label.
    pub label: Option<&'static str>,
    /// Free-text notes.
    pub notes: Option<&'static str>,
}

/// One camp's plan in [`fish_camp_cook_plans`] (`data/cookingRanges.ts:99-167`): indexes into
/// [`fish_camp_cook_surfaces`], so a surface two roles share is one entry (`CATHERBY_RANGE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishCampCookPlan {
    /// Fishing camp name.
    pub camp: &'static str,
    /// Surface near the pier (cook, then bank).
    pub pier: Option<usize>,
    /// Surface near the bank (bank raw, then cook).
    pub bank: Option<usize>,
}

/// Which surface a cook mode prefers (`CookSurfaceRole`, `data/cookingRanges.ts:77`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookRole {
    /// Near the fishing spot.
    Pier,
    /// Near the bank booth.
    Bank,
}

/// The curated fish-camp cook surfaces, in table order.
pub fn fish_camp_cook_surfaces() -> Result<&'static [FishCookSurface], FactError> {
    embedded_table!(generated::FISH_CAMP_COOK_SURFACES)
}

/// The curated fish-camp cook plans, in table order.
pub fn fish_camp_cook_plans() -> Result<&'static [FishCampCookPlan], FactError> {
    embedded_table!(generated::FISH_CAMP_COOK_PLANS)
}

/// Frozen `cookSurfaceForFishCamp` (`data/cookingRanges.ts:169-181`) as an index into
/// [`fish_camp_cook_surfaces`]: the bank role takes `bank ?? pier`, the pier role `pier ?? bank`.
pub fn fish_camp_cook_surface_index(
    camp: &str,
    role: CookRole,
) -> Result<Option<usize>, FactError> {
    let Some(plan) = fish_camp_cook_plans()?
        .iter()
        .find(|plan| plan.camp == camp)
    else {
        return Ok(None);
    };
    Ok(match role {
        CookRole::Bank => plan.bank.or(plan.pier),
        CookRole::Pier => plan.pier.or(plan.bank),
    })
}

/// [`fish_camp_cook_surface_index`] resolved to its surface.
pub fn fish_camp_cook_surface(
    camp: &str,
    role: CookRole,
) -> Result<Option<&'static FishCookSurface>, FactError> {
    let surfaces = fish_camp_cook_surfaces()?;
    Ok(fish_camp_cook_surface_index(camp, role)?.and_then(|index| surfaces.get(index)))
}

/// One cook surface placed in the selected map pack.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CookSurface {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub name: String,
    pub debugname: String,
    pub kind: CookSurfaceKind,
}

impl CookSurface {
    pub fn tile(&self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CookSurfaceFacts {
    pub rows: Vec<CookSurface>,
}

pub fn cook_camps() -> Result<&'static [CookCamp], FactError> {
    embedded_table!(generated::COOK_CAMPS)
}

pub fn default_obstacles() -> Result<&'static [&'static str], FactError> {
    embedded_table!(generated::DEFAULT_OBSTACLES)
}

pub fn max_surface_cheb() -> Result<i32, FactError> {
    embedded_table!(generated::MAX_SURFACE_CHEB)
}

pub fn derived_arrive_radius() -> Result<i32, FactError> {
    embedded_table!(generated::DERIVED_ARRIVE_RADIUS)
}

#[cfg(test)]
mod fish_camp_tests {
    use super::*;

    fn surface_index(camp: &str, role: CookRole) -> Option<usize> {
        fish_camp_cook_surface_index(camp, role).expect("fish camp plans")
    }

    #[test]
    fn roles_fall_back_to_the_other_surface_and_shared_surfaces_stay_one_entry() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let pier = surface_index("Catherby", CookRole::Pier);
        assert_eq!(pier, surface_index("Catherby", CookRole::Bank));
        assert_eq!(
            surface_index("Fishing Guild", CookRole::Bank),
            surface_index("Fishing Guild", CookRole::Pier),
            "a camp without a bank surface falls back to its pier surface"
        );
        assert_ne!(
            surface_index("Seers (fly fishing)", CookRole::Pier),
            surface_index("Seers (fly fishing)", CookRole::Bank)
        );
        assert!(fish_camp_cook_surface("Nowhere", CookRole::Pier)
            .expect("fish camp plans")
            .is_none());
        let seers = fish_camp_cook_surface("Seers (fly fishing)", CookRole::Pier)
            .expect("fish camp plans")
            .expect("plan");
        assert!(seers.approach_first && seers.approach.is_some());
        assert_eq!(seers.kind.as_str(), "range");
    }

    #[test]
    fn every_plan_index_names_a_surface_and_the_bank_catalog_agrees() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let surfaces = fish_camp_cook_surfaces().expect("fish camp surfaces");
        for plan in fish_camp_cook_plans().expect("fish camp plans") {
            for index in [plan.pier, plan.bank].into_iter().flatten() {
                assert!(index < surfaces.len(), "{}", plan.camp);
            }
        }
        // `cook_camps` (bank cook pairing) is the `bank ?? pier` surface of the camps `CURATED_CAMP` names.
        for camp in cook_camps().expect("cook camps") {
            let found = fish_camp_cook_plans()
                .expect("fish camp plans")
                .iter()
                .filter_map(|plan| {
                    fish_camp_cook_surface(plan.camp, CookRole::Bank).expect("fish camp plans")
                })
                .any(|surface| surface.stand == camp.stand && surface.loc_name == camp.loc_name);
            assert!(found, "{}", camp.bank);
        }
    }
}

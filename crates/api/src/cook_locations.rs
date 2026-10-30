//! Frozen cook-location inputs: the curated bank cook surfaces, the surface
//! kinds and the pairing constants, generated from the pinned rs2b0t sources
//! (`data/cookLocations.ts`, `data/cookingRanges.ts`,
//! `tools/cooking/gen-cooksurfaces.ts`); the placed surfaces themselves are
//! selected content ([`crate::game_data::SelectedGameData::cook_surfaces`]).

use crate::snapshot::WorldTile;
use serde::Deserialize;

/// Which cooking branch a surface takes: an oven burns less than a fire
/// (frozen `CookSurfaceKind`, `data/cookSurfaceTypes.ts:5`).
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

/// One hand-walked bank cook surface (frozen `curatedPlan`,
/// `data/cookLocations.ts:98-116`).
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

include!("../data/game-data/cook-catalog.rs");

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

/// One camp's plan in `FISH_CAMP_COOK_PLANS` (`data/cookingRanges.ts:99-167`): indexes into
/// [`FISH_CAMP_COOK_SURFACES`], so a surface two roles share is one entry (`CATHERBY_RANGE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishCampCookPlan {
    /// Fishing camp name.
    pub camp: &'static str,
    /// Surface near the pier (cook, then bank).
    pub pier: Option<usize>,
    /// Surface near the bank (bank raw, then cook).
    pub bank: Option<usize>,
}

include!("../data/game-data/fish-camp-cook-plans.rs");

/// Which surface a cook mode prefers (`CookSurfaceRole`, `data/cookingRanges.ts:77`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookRole {
    /// Near the fishing spot.
    Pier,
    /// Near the bank booth.
    Bank,
}

/// Frozen `cookSurfaceForFishCamp` (`data/cookingRanges.ts:169-181`) as an index into
/// [`FISH_CAMP_COOK_SURFACES`]: the bank role takes `bank ?? pier`, the pier role `pier ?? bank`.
pub fn fish_camp_cook_surface_index(camp: &str, role: CookRole) -> Option<usize> {
    let plan = FISH_CAMP_COOK_PLANS.iter().find(|plan| plan.camp == camp)?;
    match role {
        CookRole::Bank => plan.bank.or(plan.pier),
        CookRole::Pier => plan.pier.or(plan.bank),
    }
}

/// [`fish_camp_cook_surface_index`] resolved to its surface.
pub fn fish_camp_cook_surface(camp: &str, role: CookRole) -> Option<&'static FishCookSurface> {
    fish_camp_cook_surface_index(camp, role).and_then(|index| FISH_CAMP_COOK_SURFACES.get(index))
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

#[cfg(test)]
mod fish_camp_tests {
    use super::*;

    #[test]
    fn roles_fall_back_to_the_other_surface_and_shared_surfaces_stay_one_entry() {
        let pier = fish_camp_cook_surface_index("Catherby", CookRole::Pier);
        assert_eq!(
            pier,
            fish_camp_cook_surface_index("Catherby", CookRole::Bank)
        );
        assert_eq!(
            fish_camp_cook_surface_index("Fishing Guild", CookRole::Bank),
            fish_camp_cook_surface_index("Fishing Guild", CookRole::Pier),
            "a camp without a bank surface falls back to its pier surface"
        );
        assert_ne!(
            fish_camp_cook_surface_index("Seers (fly fishing)", CookRole::Pier),
            fish_camp_cook_surface_index("Seers (fly fishing)", CookRole::Bank)
        );
        assert!(fish_camp_cook_surface("Nowhere", CookRole::Pier).is_none());
        let seers = fish_camp_cook_surface("Seers (fly fishing)", CookRole::Pier).expect("plan");
        assert!(seers.approach_first && seers.approach.is_some());
        assert_eq!(seers.kind.as_str(), "range");
    }

    #[test]
    fn every_plan_index_names_a_surface_and_the_bank_catalog_agrees() {
        for plan in FISH_CAMP_COOK_PLANS {
            for index in [plan.pier, plan.bank].into_iter().flatten() {
                assert!(index < FISH_CAMP_COOK_SURFACES.len(), "{}", plan.camp);
            }
        }
        // `COOK_CAMPS` (bank cook pairing) is the `bank ?? pier` surface of the camps `CURATED_CAMP` names.
        for camp in COOK_CAMPS {
            let found = FISH_CAMP_COOK_PLANS
                .iter()
                .filter_map(|plan| fish_camp_cook_surface(plan.camp, CookRole::Bank))
                .any(|surface| surface.stand == camp.stand && surface.loc_name == camp.loc_name);
            assert!(found, "{}", camp.bank);
        }
    }
}

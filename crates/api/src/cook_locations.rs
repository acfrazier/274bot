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

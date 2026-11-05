//! Frozen cook-location inputs: the curated bank cook surfaces, the surface
//! kinds and the pairing constants, generated from the local game-data sources;
//! the placed surfaces themselves are selected content.

use crate::selected::FactError;
#[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
use crate::selected::LOCAL_GAME_DATA_FAMILY;
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

#[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
fn unavailable() -> FactError {
    FactError::FamilyUnavailable((*LOCAL_GAME_DATA_FAMILY).clone())
}

pub fn cook_camps() -> Result<&'static [CookCamp], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        return Ok(generated::COOK_CAMPS);
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(unavailable())
    }
}

pub fn default_obstacles() -> Result<&'static [&'static str], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        return Ok(generated::DEFAULT_OBSTACLES);
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(unavailable())
    }
}

pub fn max_surface_cheb() -> Result<i32, FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        return Ok(generated::MAX_SURFACE_CHEB);
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(unavailable())
    }
}

pub fn derived_arrive_radius() -> Result<i32, FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        return Ok(generated::DERIVED_ARRIVE_RADIUS);
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(unavailable())
    }
}

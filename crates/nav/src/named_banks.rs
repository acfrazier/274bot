//! Resolve the frozen roster against selected-content access placements and the
//! bound walk surface. Unsupported geometry stays visible for air fallback, but
//! never becomes a route target merely because the old catalog had a tile.

use api::named_banks::{BankDefinition, BankPlacement, NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;

/// The map footprint of one bank access: a placed booth or chest loc, or a
/// teller NPC (one tile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footprint {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub width: i32,
    pub length: i32,
}

/// Which neighbours of a footprint use its access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessReach {
    /// Any tile next to the footprint, diagonals included: the C1 named-bank
    /// stands, whose script walk then clicks a booth or chest and lets the
    /// server walk the last step.
    Adjacent,
    /// Only the tiles in line with the footprint's edges (frozen
    /// `bankStand`'s east/west/north/south): a teller across a booth
    /// answers only a player in line with the counter — live at Varrock
    /// West, the Bank op from the booth's diagonal never opened the bank.
    /// BankBudget, whose Open tries the teller first, arrives in line.
    InLine,
}

impl Footprint {
    /// A one-tile access at `tile` (a packed booth or teller stand).
    pub fn tile(tile: WorldTile) -> Self {
        Footprint {
            x: tile.x,
            z: tile.z,
            level: tile.level,
            width: 1,
            length: 1,
        }
    }

    /// The per-axis gap from `tile` to this footprint; `(0, 0)` inside it.
    fn gap(&self, tile: WorldTile) -> (i32, i32) {
        let dx = (self.x - tile.x)
            .max(tile.x - (self.x + self.width - 1))
            .max(0);
        let dz = (self.z - tile.z)
            .max(tile.z - (self.z + self.length - 1))
            .max(0);
        (dx, dz)
    }

    /// Chebyshev distance from `tile` to this footprint; 0 inside it.
    fn distance(&self, tile: WorldTile) -> i32 {
        let (dx, dz) = self.gap(tile);
        dx.max(dz)
    }

    /// Whether `tile` is next to this footprint under `reach`.
    fn touches(&self, tile: WorldTile, reach: AccessReach) -> bool {
        let (dx, dz) = self.gap(tile);
        match reach {
            AccessReach::Adjacent => dx.max(dz) == 1,
            AccessReach::InLine => dx + dz == 1,
        }
    }

    /// This footprint's bounding box grown by one tile, row by row.
    fn around(self) -> impl Iterator<Item = WorldTile> {
        (self.x - 1..=self.x + self.width).flat_map(move |x| {
            (self.z - 1..=self.z + self.length).map(move |z| WorldTile {
                x,
                z,
                level: self.level,
            })
        })
    }

    /// Every access tile ([`is_access_tile`]) of this footprint alone.
    pub fn access_tiles(
        self,
        reach: AccessReach,
        walkable: impl Fn(WorldTile) -> bool,
    ) -> impl Iterator<Item = WorldTile> {
        self.around().filter(move |&tile| {
            is_access_tile(tile, std::slice::from_ref(&self), reach, &walkable)
        })
    }
}

impl From<&BankPlacement> for Footprint {
    fn from(placement: &BankPlacement) -> Self {
        Footprint {
            x: placement.x,
            z: placement.z,
            level: placement.level,
            width: placement.width,
            length: placement.length,
        }
    }
}

/// Whether a bank access is used from `tile`: walkable, inside no footprint,
/// and next to at least one under `reach`. The one rule for "where a bank is
/// used from": the C1 named-bank stands ([`resolve`], [`AccessReach::Adjacent`])
/// and BankBudget's packed-stand access tiles
/// ([`crate::bank_fetch::bank_access_tiles`], [`AccessReach::InLine`]).
pub fn is_access_tile(
    tile: WorldTile,
    footprints: &[Footprint],
    reach: AccessReach,
    walkable: &impl Fn(WorldTile) -> bool,
) -> bool {
    walkable(tile)
        && footprints
            .iter()
            .all(|footprint| footprint.distance(tile) != 0)
        && footprints
            .iter()
            .any(|footprint| footprint.touches(tile, reach))
}

fn stand_for(
    definition: &BankDefinition,
    footprints: &[Footprint],
    walkable: &impl Fn(WorldTile) -> bool,
) -> Option<WorldTile> {
    let reach = AccessReach::Adjacent;
    if !footprints.is_empty() && is_access_tile(definition.tile, footprints, reach, walkable) {
        return Some(definition.tile);
    }
    footprints
        .iter()
        .flat_map(|footprint| footprint.around())
        .filter(|&tile| is_access_tile(tile, footprints, reach, walkable))
        .min_by_key(|tile| {
            let dx = i64::from(tile.x) - i64::from(definition.tile.x);
            let dz = i64::from(tile.z) - i64::from(definition.tile.z);
            (dx * dx + dz * dz, tile.x, tile.z)
        })
}

pub fn resolve(
    catalog: &'static [BankDefinition],
    placements: &[BankPlacement],
    walkable: impl Fn(WorldTile) -> bool,
) -> NamedBankFacts {
    NamedBankFacts::from_banks(
        catalog
            .iter()
            .map(|definition| {
                let matched: Vec<Footprint> = placements
                    .iter()
                    .filter(|placement| {
                        placement.name == definition.name
                            && placement.level == definition.tile.level
                            && placement.width > 0
                            && placement.length > 0
                            && (definition.npc.is_some()
                                == matches!(
                                    placement.kind,
                                    api::named_banks::BankPlacementKind::Npc
                                ))
                    })
                    .map(Footprint::from)
                    .collect();
                let stand = stand_for(definition, &matched, &walkable);
                NamedBank {
                    name: definition.name,
                    tile: stand.unwrap_or(definition.tile),
                    definition: Some(definition),
                    routable: stand.is_some(),
                }
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "named_banks_tests.rs"]
mod tests;

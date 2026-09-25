//! Resolve the frozen roster against selected-content access placements and the
//! bound walk surface. Unsupported geometry stays visible for air fallback, but
//! never becomes a route target merely because the old catalog had a tile.

use api::named_banks::{BankDefinition, BankPlacement, NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;

fn footprint_distance(tile: WorldTile, placement: &BankPlacement) -> i32 {
    let dx = (placement.x - tile.x).max(tile.x - (placement.x + placement.width - 1)).max(0);
    let dz = (placement.z - tile.z).max(tile.z - (placement.z + placement.length - 1)).max(0);
    dx.max(dz)
}

fn stand_for(
    definition: &BankDefinition,
    placements: &[&BankPlacement],
    walkable: &impl Fn(WorldTile) -> bool,
) -> Option<WorldTile> {
    let allowed = |tile: WorldTile| {
        walkable(tile)
            && placements.iter().all(|placement| footprint_distance(tile, placement) != 0)
            && placements.iter().any(|placement| footprint_distance(tile, placement) == 1)
    };
    if !placements.is_empty() && allowed(definition.tile) {
        return Some(definition.tile);
    }
    let mut best: Option<((i64, i32, i32), WorldTile)> = None;
    for placement in placements {
        for x in placement.x - 1..=placement.x + placement.width {
            for z in placement.z - 1..=placement.z + placement.length {
                let tile = WorldTile { x, z, level: placement.level };
                if !allowed(tile) { continue; }
                let dx = i64::from(x) - i64::from(definition.tile.x);
                let dz = i64::from(z) - i64::from(definition.tile.z);
                let key = (dx * dx + dz * dz, x, z);
                if best.is_none_or(|(old, _)| key < old) { best = Some((key, tile)); }
            }
        }
    }
    best.map(|(_, tile)| tile)
}

pub fn resolve(
    catalog: &'static [BankDefinition],
    placements: &[BankPlacement],
    walkable: impl Fn(WorldTile) -> bool,
) -> NamedBankFacts {
    NamedBankFacts::from_banks(catalog.iter().map(|definition| {
        let matched: Vec<_> = placements.iter().filter(|placement| {
            placement.name == definition.name && placement.level == definition.tile.level
                && placement.width > 0 && placement.length > 0
                && (definition.npc.is_some() == matches!(placement.kind, api::named_banks::BankPlacementKind::Npc))
        }).collect();
        let stand = stand_for(definition, &matched, &walkable);
        NamedBank {
            name: definition.name,
            tile: stand.unwrap_or(definition.tile),
            definition: Some(definition),
            routable: stand.is_some(),
        }
    }).collect())
}

#[cfg(test)]
#[path = "named_banks_tests.rs"]
mod tests;

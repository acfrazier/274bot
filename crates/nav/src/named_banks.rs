//! Accessible-geometry resolution for the catalog bank aliases published as
//! `content.named_banks`.
//!
//! The curated catalog preferences (alias names, preferred stands, cluster
//! booth preferences) come in from the caller — the host passes
//! `script::content::BANK_ALIASES` into [`crate::world::NavWorld::named_bank_facts`]
//! — so this module never depends on `script`. Its own inputs are world
//! facts: the bound world's packed booth tiles and its walk surface. A
//! preferred stand that fails adjacency or walkability is replaced by a
//! derived adjacent walkable tile, or the alias is omitted so the catalog's
//! `BANK_LOCATIONS.find` stays undefined. The packed stand table is not
//! copied into the published facts.

use api::named_banks::{BankAliasCandidate, NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;

/// Cardinal then diagonal Chebyshev-1 ring, matching native approach order.
const APPROACH_RING: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

fn cheb(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn is_packed_booth(packed: &[WorldTile], tile: WorldTile) -> bool {
    packed.contains(&tile)
}

fn adjacent_to_cluster(stand: WorldTile, matched: &[WorldTile]) -> bool {
    matched
        .iter()
        .any(|booth| booth.level == stand.level && cheb(stand, *booth) == 1)
}

fn derived_stand(
    matched: &[WorldTile],
    packed: &[WorldTile],
    walkable: impl Fn(WorldTile) -> bool,
) -> Option<WorldTile> {
    let mut booths = matched.to_vec();
    booths.sort_by_key(|b| (b.x, b.z, b.level));
    for booth in booths {
        for (dx, dz) in APPROACH_RING {
            let stand = WorldTile {
                x: booth.x + dx,
                z: booth.z + dz,
                level: booth.level,
            };
            if is_packed_booth(packed, stand) {
                continue;
            }
            if walkable(stand) {
                return Some(stand);
            }
        }
    }
    None
}

fn keep_alias(
    candidate: &BankAliasCandidate,
    packed: &[WorldTile],
    walkable: impl Fn(WorldTile) -> bool,
) -> Option<NamedBank> {
    let matched: Vec<WorldTile> = packed
        .iter()
        .copied()
        .filter(|booth| {
            booth.level == candidate.stand.level
                && candidate.booths.iter().any(|want| want == booth)
        })
        .collect();
    if matched.is_empty() {
        return None;
    }
    let preferred = candidate.stand;
    if walkable(preferred)
        && !is_packed_booth(packed, preferred)
        && adjacent_to_cluster(preferred, &matched)
    {
        return Some(NamedBank {
            name: candidate.name,
            tile: preferred,
        });
    }
    let stand = derived_stand(&matched, packed, walkable)?;
    Some(NamedBank {
        name: candidate.name,
        tile: stand,
    })
}

/// Keep each catalog alias candidate only when the bound packed booth set
/// and walk surface can support it. Missing booths, wrong plane, a blocked
/// stand with no adjacent replacement, or a stand on a booth loc omit the
/// alias. Candidate order is the published order.
pub fn resolve(
    candidates: &[BankAliasCandidate],
    packed_booths: &[WorldTile],
    walkable: impl Fn(WorldTile) -> bool,
) -> NamedBankFacts {
    NamedBankFacts::from_banks(
        candidates
            .iter()
            .filter_map(|candidate| keep_alias(candidate, packed_booths, &walkable))
            .collect(),
    )
}

#[cfg(test)]
#[path = "named_banks_tests.rs"]
mod tests;

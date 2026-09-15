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
mod tests {
    use super::*;

    const fn t(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    // Two synthetic clusters. Alpha's preferred stand sits in the gap
    // between its booths (Chebyshev-1 to both); Beta's preferred stand is
    // two tiles off the cluster, so it can only resolve by derivation.
    const ALPHA_BOOTHS: &[WorldTile] = &[t(2000, 2000), t(2000, 2002)];
    const BETA_BOOTHS: &[WorldTile] = &[t(3000, 3000)];
    const ALPHA_STAND: WorldTile = t(1999, 2001);
    const BETA_STAND: WorldTile = t(2500, 2500);

    const TEST_ALIASES: &[BankAliasCandidate] = &[
        BankAliasCandidate {
            name: "Alpha",
            stand: ALPHA_STAND,
            booths: ALPHA_BOOTHS,
        },
        BankAliasCandidate {
            name: "Beta",
            stand: BETA_STAND,
            booths: BETA_BOOTHS,
        },
    ];

    fn all_booths() -> Vec<WorldTile> {
        TEST_ALIASES
            .iter()
            .flat_map(|c| c.booths.iter().copied())
            .collect()
    }

    fn walk_all(_: WorldTile) -> bool {
        true
    }

    fn names(facts: &NamedBankFacts) -> Vec<&'static str> {
        facts.banks().iter().map(|b| b.name).collect()
    }

    fn tile_of(facts: &NamedBankFacts, name: &str) -> Option<WorldTile> {
        facts
            .banks()
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.tile)
    }

    #[test]
    fn absent_packed_booths_publish_nothing() {
        let facts = resolve(TEST_ALIASES, &[], walk_all);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn preferred_stand_kept_when_adjacent_and_walkable_others_derive() {
        let packed = all_booths();
        let facts = resolve(TEST_ALIASES, &packed, walk_all);
        assert_eq!(names(&facts), ["Alpha", "Beta"]);
        assert_eq!(tile_of(&facts, "Alpha"), Some(ALPHA_STAND));
        let beta = tile_of(&facts, "Beta").expect("derived Beta");
        assert_ne!(beta, BETA_STAND);
        assert!(!is_packed_booth(&packed, beta));
        assert!(adjacent_to_cluster(beta, BETA_BOOTHS));
    }

    #[test]
    fn derived_stand_walks_the_approach_ring_from_the_first_sorted_booth() {
        let packed = BETA_BOOTHS.to_vec();
        let facts = resolve(TEST_ALIASES, &packed, walk_all);
        assert_eq!(names(&facts), ["Beta"]);
        assert_eq!(tile_of(&facts, "Beta"), Some(t(3000, 3001)));
    }

    #[test]
    fn missing_cluster_omits_only_that_alias() {
        let packed: Vec<WorldTile> = all_booths()
            .into_iter()
            .filter(|b| !ALPHA_BOOTHS.contains(b))
            .collect();
        let facts = resolve(TEST_ALIASES, &packed, walk_all);
        assert_eq!(names(&facts), ["Beta"]);
    }

    #[test]
    fn wrong_plane_booths_omit_the_alias() {
        let packed = vec![WorldTile {
            x: 2000,
            z: 2000,
            level: 1,
        }];
        let facts = resolve(TEST_ALIASES, &packed, walk_all);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn blocked_cluster_with_no_adjacent_walkable_is_omitted() {
        let packed = ALPHA_BOOTHS.to_vec();
        let facts = resolve(TEST_ALIASES, &packed, |_| false);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn stand_on_a_packed_booth_is_not_published() {
        // The preferred stand is one of the cluster's own booth loc tiles:
        // it must never be published, so the alias derives an adjacent tile.
        const ON_BOOTH: &[BankAliasCandidate] = &[BankAliasCandidate {
            name: "Alpha",
            stand: t(2000, 2000),
            booths: ALPHA_BOOTHS,
        }];
        let packed = ALPHA_BOOTHS.to_vec();
        let facts = resolve(ON_BOOTH, &packed, walk_all);
        let stand = tile_of(&facts, "Alpha").expect("alpha");
        assert_ne!(stand, t(2000, 2000));
        assert!(!is_packed_booth(&packed, stand));
        assert!(adjacent_to_cluster(stand, ALPHA_BOOTHS));
    }

    #[test]
    fn blocked_preferred_stand_derives_an_adjacent_walkable_tile() {
        let packed = ALPHA_BOOTHS.to_vec();
        let facts = resolve(TEST_ALIASES, &packed, |tile| tile != ALPHA_STAND);
        let stand = tile_of(&facts, "Alpha").expect("alpha");
        assert_ne!(stand, ALPHA_STAND);
        assert!(!is_packed_booth(&packed, stand));
        assert!(adjacent_to_cluster(stand, ALPHA_BOOTHS));
    }
}

//! Host-owned F2P bank aliases for catalog `BANK_LOCATIONS`.
//!
//! Catalog names (`Falador East`, …) map onto packed `bankbooth` loc tiles
//! plus a distinct walkable stand. Resolve against the actually bound
//! world's booths and walk surface before isolate module evaluation.
//! Omitted names stay absent so `BANK_LOCATIONS.find` remains undefined.

use crate::snapshot::WorldTile;

/// One published alias: the catalog name and the walk stand (not the booth loc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedBank {
    pub name: &'static str,
    pub tile: WorldTile,
}

/// Immutable per-profile alias rows posted once onto `__rs2b0t_host.content`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedBankFacts {
    banks: Vec<NamedBank>,
}

impl NamedBankFacts {
    pub fn empty() -> Self {
        Self { banks: Vec::new() }
    }

    pub fn banks(&self) -> &[NamedBank] {
        &self.banks
    }
}

/// Candidate alias: preferred stand plus the packed booth loc tiles that
/// identify that named cluster.
#[derive(Debug, Clone, Copy)]
pub struct BankAliasCandidate {
    pub name: &'static str,
    pub stand: WorldTile,
    pub booths: &'static [WorldTile],
}

const fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

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

const FALADOR_EAST_BOOTHS: &[WorldTile] = &[
    t(3011, 3354),
    t(3012, 3354),
    t(3013, 3354),
    t(3014, 3354),
    t(3015, 3354),
];
const VARROCK_EAST_BOOTHS: &[WorldTile] =
    &[t(3252, 3419), t(3253, 3419), t(3254, 3419), t(3256, 3419)];
const EDGEVILLE_BOOTHS: &[WorldTile] = &[t(3095, 3491), t(3096, 3493)];
const DRAYNOR_BOOTHS: &[WorldTile] = &[t(3091, 3242), t(3091, 3243), t(3091, 3245)];
const AL_KHARID_BOOTHS: &[WorldTile] = &[
    t(3268, 3164),
    t(3268, 3165),
    t(3268, 3166),
    t(3268, 3167),
    t(3268, 3168),
    t(3268, 3169),
];

/// The five shim `RUNES.bank` names. Preferred stands that fail adjacency
/// or walkability are replaced by a derived adjacent walkable tile, or the
/// alias is omitted.
pub const CANDIDATES: &[BankAliasCandidate] = &[
    BankAliasCandidate {
        name: "Falador East",
        stand: t(3013, 3355),
        booths: FALADOR_EAST_BOOTHS,
    },
    BankAliasCandidate {
        name: "Varrock East",
        stand: t(3253, 3420),
        booths: VARROCK_EAST_BOOTHS,
    },
    BankAliasCandidate {
        name: "Edgeville",
        stand: t(3094, 3493),
        booths: EDGEVILLE_BOOTHS,
    },
    BankAliasCandidate {
        name: "Draynor",
        stand: t(3093, 3243),
        booths: DRAYNOR_BOOTHS,
    },
    BankAliasCandidate {
        name: "Al Kharid",
        stand: t(3269, 3167),
        booths: AL_KHARID_BOOTHS,
    },
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

/// Keep each candidate only when the bound packed booth set and walk
/// surface can support it. Missing booths, wrong plane, a blocked stand
/// with no adjacent replacement, or a stand on a booth loc omit the alias.
pub fn resolve(
    packed_booths: &[WorldTile],
    walkable: impl Fn(WorldTile) -> bool,
) -> NamedBankFacts {
    NamedBankFacts {
        banks: CANDIDATES
            .iter()
            .filter_map(|candidate| keep_alias(candidate, packed_booths, &walkable))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_candidate_booths() -> Vec<WorldTile> {
        CANDIDATES
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
        let facts = resolve(&[], walk_all);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn preferred_falador_varrock_al_kharid_keep_when_adjacent_and_walkable() {
        let packed = all_candidate_booths();
        let facts = resolve(&packed, walk_all);
        assert_eq!(
            names(&facts),
            [
                "Falador East",
                "Varrock East",
                "Edgeville",
                "Draynor",
                "Al Kharid"
            ]
        );
        assert_eq!(tile_of(&facts, "Falador East"), Some(t(3013, 3355)));
        assert_eq!(tile_of(&facts, "Varrock East"), Some(t(3253, 3420)));
        assert_eq!(tile_of(&facts, "Al Kharid"), Some(t(3269, 3167)));
        let edge = tile_of(&facts, "Edgeville").expect("derived Edgeville");
        assert_ne!(edge, t(3094, 3493));
        assert!(!is_packed_booth(&packed, edge));
        assert!(adjacent_to_cluster(edge, EDGEVILLE_BOOTHS));
        let dray = tile_of(&facts, "Draynor").expect("derived Draynor");
        assert_ne!(dray, t(3093, 3243));
        assert!(!is_packed_booth(&packed, dray));
        assert!(adjacent_to_cluster(dray, DRAYNOR_BOOTHS));
    }

    #[test]
    fn missing_cluster_omits_only_that_alias() {
        let packed: Vec<WorldTile> = all_candidate_booths()
            .into_iter()
            .filter(|b| !FALADOR_EAST_BOOTHS.contains(b))
            .collect();
        let facts = resolve(&packed, walk_all);
        assert!(!names(&facts).contains(&"Falador East"));
        assert!(names(&facts).contains(&"Varrock East"));
        assert!(names(&facts).contains(&"Al Kharid"));
    }

    #[test]
    fn wrong_plane_booths_omit_the_alias() {
        let packed = vec![WorldTile {
            x: 3011,
            z: 3354,
            level: 1,
        }];
        let facts = resolve(&packed, walk_all);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn blocked_cluster_with_no_adjacent_walkable_is_omitted() {
        let packed = EDGEVILLE_BOOTHS.to_vec();
        let facts = resolve(&packed, |_| false);
        assert!(facts.banks().is_empty());
    }

    #[test]
    fn stand_on_a_packed_booth_is_not_published() {
        let packed = FALADOR_EAST_BOOTHS.to_vec();
        let facts = resolve(&packed, |tile| {
            tile == t(3013, 3354) || tile == t(3013, 3355)
        });
        let stand = tile_of(&facts, "Falador East").expect("falador");
        assert_ne!(stand, t(3013, 3354));
        assert!(!is_packed_booth(&packed, stand));
        assert!(adjacent_to_cluster(stand, FALADOR_EAST_BOOTHS));
    }
}

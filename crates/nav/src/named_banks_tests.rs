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

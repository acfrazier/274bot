//! The shared escape ring: the one choice of outward tile that the Guardian's
//! walk and the Fight child's escape price both read.
use super::*;

/// The ring has eight tiles, all `distance` from the anchor in Chebyshev
/// distance, and every compass point once.
#[test]
fn the_ring_has_one_tile_per_compass_point_at_the_distance() {
    let anchor = (10, 20);
    let tiles: Vec<_> = escape_candidates(anchor, anchor, 17).collect();
    assert_eq!(tiles.len(), 8);
    assert_eq!(
        tiles[0],
        (10, 3),
        "north is first when the player stands on the anchor"
    );
    assert_eq!(tiles[7], (-7, 3), "north-west closes the compass");
    for tile in &tiles {
        let reach = (tile.0 - anchor.0).abs().max((tile.1 - anchor.1).abs());
        assert_eq!(reach, 17, "{tile:?} is on the ring");
    }
    let mut unique = tiles.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 8, "no compass point repeats");
}

/// The ring starts away from the anchor and through the player, so the order
/// depends on where the player stands. A different player tile makes a
/// different first candidate.
#[test]
fn the_order_starts_away_from_the_anchor_through_the_player() {
    let anchor = (100, 100);
    let first = |player: (i32, i32)| escape_candidates(anchor, player, 9).next().unwrap();
    assert_eq!(
        first((100, 96)),
        (100, 91),
        "north of the anchor flees north"
    );
    assert_eq!(first((104, 100)), (109, 100), "east flees east");
    assert_eq!(first((100, 105)), (100, 109), "south flees south");
    assert_eq!(first((96, 100)), (91, 100), "west flees west");
    assert_eq!(first((104, 104)), (109, 109), "south-east flees south-east");
    assert_ne!(first((100, 96)), first((100, 105)));
}

/// A tie between a cardinal and a diagonal direction keeps the cardinal one.
/// A player due east of the anchor scores east and both east diagonals alike;
/// a player on the anchor scores every direction alike and north leads.
#[test]
fn a_tie_prefers_a_cardinal_direction() {
    let anchor = (50, 50);
    assert_eq!(
        escape_direction(anchor, anchor),
        0,
        "the compass starts at north"
    );
    let first = escape_candidates(anchor, (53, 50), 5).next().unwrap();
    assert_eq!(
        first,
        (55, 50),
        "a player due east flees east, not diagonally"
    );
}

/// The distance is the content radius while the player stands inside it, and
/// one tile past the player once the player stands at or beyond it.
#[test]
fn the_distance_is_the_radius_or_one_past_the_player() {
    let anchor = (0, 0);
    assert_eq!(escape_distance(anchor, 7, (3, 2)), 7);
    assert_eq!(escape_distance(anchor, 7, (6, -6)), 7);
    assert_eq!(
        escape_distance(anchor, 7, (7, 0)),
        8,
        "on the ring: one past"
    );
    assert_eq!(escape_distance(anchor, 7, (-2, 12)), 13);
}

/// The first accepted candidate is the walk's target: refusals are skipped in
/// order, every candidate is offered once, and nothing accepted gives `None`.
#[test]
fn the_first_accepted_candidate_is_the_target() {
    let anchor = (200, 200);
    let player = (203, 200);
    let ring: Vec<_> = escape_candidates(anchor, player, 8).collect();
    let mut asked = Vec::new();
    let target = first_escape_target(anchor, player, 8, |tile| {
        asked.push(tile);
        tile == ring[2]
    });
    assert_eq!(target, Some(ring[2]));
    assert_eq!(
        asked,
        ring[..3],
        "the candidates are tried in ring order, no further"
    );

    let mut asked = 0;
    assert_eq!(
        first_escape_target(anchor, player, 8, |_| {
            asked += 1;
            false
        }),
        None
    );
    assert_eq!(asked, 8, "all eight are tried once");
}

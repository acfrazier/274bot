use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use super::*;
use crate::collision::WorldCollision;
use crate::router::{Leg, Route};
use crate::transport::{TransportEdge, TransportGraph, TransportKind};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

/// A `width`×`height` level-0 bake at (0,0) with the given per-tile
/// flags OR'd in. Planes 1..=3 stay empty (the per-level bake shape).
fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
    let mut plane = vec![0u32; width * height];
    for &(x, z, f) in extras {
        plane[z as usize * width + x as usize] |= f;
    }
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

fn edge(kind: TransportKind, at: WorldTile, to: WorldTile, loc_id: i32) -> TransportEdge {
    TransportEdge {
        kind,
        at,
        to,
        loc_id,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
    }
}

fn route(legs: Vec<Leg>) -> Route {
    let dest = legs
        .iter()
        .rev()
        .find_map(|l| match l {
            Leg::Walk { tiles } => tiles.last().copied(),
            Leg::Transport { edge } => Some(edge.to),
        })
        .unwrap_or(tile(0, 0, 0));
    Route {
        legs,
        dest,
        ticks: 0.0,
    }
}

#[test]
fn remaining_path_includes_transport_at_to() {
    // Walk then Door; here on first tile → remaining has walk + at + to
    let r = route(vec![
        Leg::Walk {
            tiles: vec![tile(0, 0, 0), tile(1, 0, 0)],
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
        },
    ]);
    let tiles = remaining_path_tiles(&r, Some(tile(0, 0, 0)));
    let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
    assert_eq!(
        flat,
        vec![tile(0, 0, 0), tile(1, 0, 0), tile(2, 0, 0), tile(3, 0, 0)],
        "here on the first tile keeps the whole path"
    );
    assert!(
        tiles[..2].iter().all(|p| !p.transport),
        "walk tiles are not transport"
    );
    assert!(
        tiles[2..].iter().all(|p| p.transport),
        "at and to carry the transport flag"
    );
}

#[test]
fn hop_captions_label_at_only_and_skip_teleports() {
    let r = route(vec![
        Leg::Walk {
            tiles: vec![tile(0, 0, 0), tile(1, 0, 0)],
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
        },
        Leg::Transport {
            edge: edge(TransportKind::Teleport, tile(3, 0, 0), tile(80, 80, 0), 1),
        },
    ]);
    let caps = hop_captions(&r, Some(tile(0, 0, 0)));
    assert_eq!(caps.len(), 1, "one caption per loc hop, none for teleport");
    assert_eq!(caps[0].at, tile(2, 0, 0));
    assert_eq!(caps[0].text, "Door");
}

#[test]
fn remaining_path_tiles_skips_done_legs_and_trims_here() {
    let r = route(vec![
        Leg::Walk {
            tiles: vec![tile(0, 0, 0), tile(1, 0, 0), tile(2, 0, 0)],
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(3, 0, 0), tile(4, 0, 0), 1530),
        },
        Leg::Walk {
            tiles: vec![tile(4, 0, 0), tile(5, 0, 0)],
        },
    ]);
    // Mid-leg: the current walk leg trims to here onward; the transport
    // and following walk leg stay; the crossing tile dedups to one.
    let tiles = remaining_path_tiles(&r, Some(tile(1, 0, 0)));
    let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
    assert_eq!(
        flat,
        vec![
            tile(1, 0, 0),
            tile(2, 0, 0),
            tile(3, 0, 0),
            tile(4, 0, 0),
            tile(5, 0, 0)
        ]
    );
    assert_eq!(tiles.len(), 5, "the crossing tile must dedup to one");
    // At a leg end the done walk leg is skipped entirely.
    let tiles = remaining_path_tiles(&r, Some(tile(2, 0, 0)));
    let flat: Vec<WorldTile> = tiles.iter().map(|p| p.tile).collect();
    assert_eq!(flat, vec![tile(3, 0, 0), tile(4, 0, 0), tile(5, 0, 0)]);
}

#[test]
fn select_draw_keeps_hops_and_terminal() {
    let force = vec![100];
    let idx = select_draw_indices(0, 400, &force);
    assert!(idx.contains(&0) && idx.contains(&399) && idx.contains(&100));
    assert!(idx.len() <= MAX_DRAW_TILES + 4);
}

#[test]
fn collision_at_reports_south_face_not_blocked_ground() {
    // raw W_S only → nsew.s true, blocked false (face ≠ blanket walkable)
    let c = bake(1, 1, &[(0, 0, CollisionFlag::W_S as u32)]);
    let fb = collision_at(&c, tile(0, 0, 0));
    assert!(fb.s);
    assert!(!fb.n && !fb.e && !fb.w);
    assert!(!fb.blocked, "a bare face flag is not blocked ground");
}

#[test]
fn collision_at_preserves_all_eight_wall_bits() {
    let raw = CollisionFlag::W_NW as u32
        | CollisionFlag::W_N as u32
        | CollisionFlag::W_NE as u32
        | CollisionFlag::W_E as u32
        | CollisionFlag::W_SE as u32
        | CollisionFlag::W_S as u32
        | CollisionFlag::W_SW as u32
        | CollisionFlag::W_W as u32;
    let fb = collision_at(&bake(1, 1, &[(0, 0, raw)]), tile(0, 0, 0));
    assert!(fb.n && fb.e && fb.s && fb.w);
    assert!(fb.ne && fb.se && fb.nw && fb.sw);
    assert_eq!(fb.raw, CollisionFlag::WALK_BLOCK_FLAGS as u8);
    assert!(!fb.blocked, "wall faces remain standable ground");
}

#[test]
fn collision_at_with_preserves_all_eight_sidecar_wall_bits() {
    let raw = CollisionFlag::W_NW as u32
        | CollisionFlag::W_N as u32
        | CollisionFlag::W_NE as u32
        | CollisionFlag::W_E as u32
        | CollisionFlag::W_SE as u32
        | CollisionFlag::W_S as u32
        | CollisionFlag::W_SW as u32
        | CollisionFlag::W_W as u32;
    let c = bake(1, 1, &[]);
    let flags = vec![raw; 4];
    let fb = collision_at_with(&c, tile(0, 0, 0), Some(&flags));
    assert!(fb.n && fb.e && fb.s && fb.w);
    assert!(fb.ne && fb.se && fb.nw && fb.sw);
    assert_eq!(fb.raw, CollisionFlag::WALK_BLOCK_FLAGS as u8);
    assert!(!fb.blocked, "wall faces remain standable ground");
}

#[test]
fn collision_at_blocks_ground_and_scenery() {
    let c = bake(
        1,
        3,
        &[
            (0, 0, CollisionFlag::WR_GRND as u32),
            (0, 1, CollisionFlag::WALK_SCENERY as u32),
            (0, 2, CollisionFlag::W_N as u32),
        ],
    );
    assert!(collision_at(&c, tile(0, 0, 0)).blocked);
    assert!(collision_at(&c, tile(0, 1, 0)).blocked);
    assert!(
        !collision_at(&c, tile(0, 2, 0)).blocked,
        "W_N is a face, not a ground block"
    );
}

#[test]
fn trail_two_tone_only_when_run_on() {
    let tiles = [tile(0, 0, 0), tile(0, 1, 0), tile(0, 2, 0)];
    assert!(trail_tones(&tiles, false)
        .iter()
        .all(|(_, t)| *t == TrailTone::Primary));
    let t = trail_tones(&tiles, true);
    assert_eq!(t[0].1, TrailTone::Primary);
    assert_eq!(t[1].1, TrailTone::RunAlt);
    assert_eq!(t[2].1, TrailTone::Primary);
    // Checkerboard by world tile, not list index: a diagonal step does
    // not flip just because it is the second entry.
    let diag = trail_tones(&[tile(0, 0, 0), tile(1, 1, 0)], true);
    assert_eq!(diag[0].1, TrailTone::Primary);
    assert_eq!(diag[1].1, TrailTone::Primary);
}

#[test]
fn remaining_trail_keeps_mid_path_but_clears_arrived_dest() {
    let tiles: Vec<WorldTile> = (0..21).map(|x| tile(x, 0, 0)).collect();
    assert_eq!(remaining_trail(&tiles, None).len(), 21);
    let rest = remaining_trail(&tiles, Some(tile(5, 0, 0)));
    assert_eq!(rest.len(), 16, "a 21-tile BFS is not capped at 9");
    assert_eq!(rest[0], tile(5, 0, 0));
    assert_eq!(rest.last().copied(), Some(tile(20, 0, 0)));
    assert!(
        remaining_trail(&tiles, Some(tile(20, 0, 0))).is_empty(),
        "arrived dest must not persist under the player"
    );
    // Off the path: retire. Keeping the whole click here is what
    // resurrected the cyan/yellow trail after arrival then a step west.
    assert!(
        remaining_trail(&tiles, Some(tile(99, 0, 0))).is_empty(),
        "off-path must not resurrect the last click"
    );
    assert_eq!(
        remaining_trail(&tiles, None).len(),
        21,
        "unknown here (startup / network wait) must not drop the trail"
    );
}

#[test]
fn hull_skips_teleport_and_missing_loc() {
    // A teleport hop has no loc scenery.
    let r = route(vec![Leg::Transport {
        edge: edge(TransportKind::Teleport, tile(0, 0, 0), tile(1, 0, 0), 0),
    }]);
    assert!(
        hull_targets(&r, None, 12).is_empty(),
        "teleport hops have no hull"
    );
    // NPC hops (boat/glider) have no loc either.
    let r = route(vec![Leg::Transport {
        edge: edge(TransportKind::Boat, tile(0, 0, 0), tile(1, 0, 0), 0),
    }]);
    assert!(
        hull_targets(&r, None, 12).is_empty(),
        "NPC hops have no hull"
    );
    // A door with a loc id resolves to one target.
    let r = route(vec![Leg::Transport {
        edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
    }]);
    assert_eq!(
        hull_targets(&r, None, 12),
        vec![HullTarget {
            loc_id: 1530,
            at: tile(2, 0, 0)
        }]
    );
}

#[test]
fn hull_targets_keeps_window_hops_and_always_the_next() {
    // The first ladder is far beyond the window but is the next loc
    // hop (always kept); the door past it is neither next nor in-window.
    let r = route(vec![
        Leg::Walk {
            tiles: (0..50).map(|x| tile(x, 0, 0)).collect(),
        },
        Leg::Transport {
            edge: edge(TransportKind::Ladder, tile(50, 0, 0), tile(51, 0, 0), 1111),
        },
        Leg::Walk {
            tiles: (51..101).map(|x| tile(x, 0, 0)).collect(),
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(101, 0, 0), tile(102, 0, 0), 1530),
        },
    ]);
    assert_eq!(
        hull_targets(&r, None, 12),
        vec![HullTarget {
            loc_id: 1111,
            at: tile(50, 0, 0)
        }],
        "only the next loc hop survives a far window"
    );
}

#[test]
fn hull_targets_keeps_in_window_hops() {
    let r = route(vec![
        Leg::Walk {
            tiles: vec![tile(0, 0, 0), tile(1, 0, 0)],
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
        },
        Leg::Walk {
            tiles: vec![tile(3, 0, 0), tile(4, 0, 0)],
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(5, 0, 0), tile(6, 0, 0), 1531),
        },
    ]);
    assert_eq!(
        hull_targets(&r, None, 12),
        vec![
            HullTarget {
                loc_id: 1530,
                at: tile(2, 0, 0)
            },
            HullTarget {
                loc_id: 1531,
                at: tile(5, 0, 0)
            },
        ]
    );
}

#[test]
fn hull_targets_window_counts_from_the_trimmed_start() {
    // `here` mid-walk trims the first walk leg, so `next_only_plus` is
    // measured from the same remaining start `remaining_path_tiles`
    // uses. Walk (0..5), door A, walk (6..12), door B, here=(3,0),
    // next_only_plus=12: the un-trimmed count puts door B at tile 13
    // (dropped), the trimmed count at tile 10 (kept).
    let r = route(vec![
        Leg::Walk {
            tiles: (0..5).map(|x| tile(x, 0, 0)).collect(),
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(5, 0, 0), tile(6, 0, 0), 1530),
        },
        Leg::Walk {
            tiles: (6..12).map(|x| tile(x, 0, 0)).collect(),
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(12, 0, 0), tile(13, 0, 0), 1531),
        },
    ]);
    assert_eq!(
        hull_targets(&r, Some(tile(3, 0, 0)), 12),
        vec![
            HullTarget {
                loc_id: 1530,
                at: tile(5, 0, 0)
            },
            HullTarget {
                loc_id: 1531,
                at: tile(12, 0, 0)
            },
        ],
        "the follow-on door is in the window from the trimmed start"
    );
}

#[test]
fn hull_targets_collapses_duplicate_door_targets() {
    // A door placement contributes two directed edges sharing one `at`.
    let r = route(vec![
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(3, 0, 0), 1530),
        },
        Leg::Transport {
            edge: edge(TransportKind::Door, tile(2, 0, 0), tile(1, 0, 0), 1530),
        },
    ]);
    assert_eq!(
        hull_targets(&r, None, 12).len(),
        1,
        "duplicate targets collapse"
    );
}

/// A 7×7 bake: a 3×3 open corner plus an isolated open tile moated by
/// WR_GRND; everything else blocked ground.
fn disconnected_world() -> WorldCollision {
    let mut extras = Vec::new();
    for z in 0..7 {
        for x in 0..7 {
            let open = (x < 3 && z < 3) || (x == 5 && z == 5);
            if !open {
                extras.push((x, z, CollisionFlag::WR_GRND as u32));
            }
        }
    }
    bake(7, 7, &extras)
}

#[test]
fn flood_two_seeds_disconnected_have_two_sizes() {
    // 3x3 open vs isolated tile with WR_GRND moat
    let c = disconnected_world();
    let (a, b) = flood_sizes(&c, tile(0, 0, 0), Some(tile(5, 5, 0)));
    assert!(a >= 1 && b.unwrap() >= 1);
    assert_ne!(
        flood_component_id(&c, &[tile(0, 0, 0)], tile(0, 0, 0)),
        flood_component_id(&c, &[tile(0, 0, 0)], tile(5, 5, 0))
    );
}

#[test]
fn flood_marks_two_components_from_two_seeds() {
    let c = disconnected_world();
    let seeds = [tile(0, 0, 0), tile(5, 5, 0)];
    assert_eq!(flood_component_id(&c, &seeds, tile(1, 1, 0)), Some(0));
    assert_eq!(flood_component_id(&c, &seeds, tile(5, 5, 0)), Some(1));
    // A moat tile belongs to no flood.
    assert_eq!(flood_component_id(&c, &seeds, tile(3, 3, 0)), None);
}

#[test]
fn flood_components_sets_match_component_ids() {
    let c = disconnected_world();
    let seeds = [tile(0, 0, 0), tile(5, 5, 0)];
    let comps = flood_components(&c, &seeds);
    assert_eq!(comps.len(), 2);
    assert!(comps[0].contains(&tile(1, 1, 0)));
    assert!(comps[1].contains(&tile(5, 5, 0)));
    assert!(
        !comps[0].contains(&tile(5, 5, 0)),
        "the moat keeps the disconnected seed out"
    );
    assert_eq!(flood_component_id(&c, &seeds, tile(1, 1, 0)), Some(0));
}

#[test]
fn flood_same_component_counts_once() {
    let c = bake(3, 3, &[]);
    assert_eq!(
        flood_sizes(&c, tile(0, 0, 0), Some(tile(2, 2, 0))),
        (9, None)
    );
}

/// A graph with the given `edges` and `teleports` (the `at` index is
/// irrelevant to the reach bake, which reads the edge lists directly).
fn graph(edges: Vec<TransportEdge>, teleports: Vec<TransportEdge>) -> TransportGraph {
    TransportGraph {
        edges,
        teleports,
        ..Default::default()
    }
}

#[test]
fn walled_courtyard_is_walkable_but_unreached() {
    // 5×5 open, inner 1×1 at (2,2) with all W_* faces on its four walls
    // and no transport. (2,2) walkable_word has faces; find from (0,0)
    // is NoPath; bake_reach does not set (2,2).
    let c = bake(5, 5, &[(2, 2, CollisionFlag::WALK_BLOCK_FLAGS as u32)]);
    let g = TransportGraph::default();
    let at = tile(2, 2, 0);
    assert_ne!(
        c.walkable_word(2, 2, 0) & CollisionFlag::WALK_BLOCK_FLAGS as u32,
        0,
        "the sealed courtyard word carries the face bits"
    );
    assert!(
        !collision_at(&c, at).blocked,
        "a walled floor is standable ground, not blocked"
    );
    assert!(
        crate::router::find(&c, &g, tile(0, 0, 0), at).is_err(),
        "find from outside the sealed courtyard is NoPath"
    );
    let bits = bake_reach(&c, &g);
    assert_eq!(bits.len(), c.walk.len().div_ceil(64));
    assert!(
        !reached(&bits, &c, at),
        "no transport seeds flood the sealed courtyard"
    );
    assert!(
        !reached(&bits, &c, tile(0, 0, 0)),
        "no seeds, nothing reached"
    );
    assert!(
        !reached(&bits, &c, tile(99, 99, 0)),
        "tiles outside the grid are never reached"
    );
}

#[test]
fn reach_seeds_cover_edges_and_teleport_landings() {
    // Teleports have no fixed origin — only the landing seeds the
    // any-tile layer's component; a regular edge seeds both ends.
    let g = graph(
        vec![edge(
            TransportKind::Door,
            tile(0, 0, 0),
            tile(1, 0, 0),
            1530,
        )],
        vec![edge(
            TransportKind::Teleport,
            tile(0, 0, 0),
            tile(4, 4, 0),
            0,
        )],
    );
    let seeds = reach_seeds(&g);
    assert!(seeds.contains(&tile(0, 0, 0)), "edge at seeds");
    assert!(seeds.contains(&tile(1, 0, 0)), "edge to seeds");
    assert!(seeds.contains(&tile(4, 4, 0)), "teleport to seeds");
}

#[test]
fn bake_reach_floods_the_walk_region_but_not_moated_tiles() {
    // The 7×7 disconnected world: a 3×3 open corner and an isolated
    // open tile moated by WR_GRND. A door edge inside the corner seeds
    // it; the flood covers the corner, never the island or the moat.
    let c = disconnected_world();
    let g = graph(
        vec![edge(
            TransportKind::Door,
            tile(0, 0, 0),
            tile(1, 1, 0),
            1530,
        )],
        vec![],
    );
    let bits = bake_reach(&c, &g);
    assert!(reached(&bits, &c, tile(0, 0, 0)), "the edge at is a seed");
    assert!(reached(&bits, &c, tile(1, 1, 0)), "the edge to is a seed");
    assert!(reached(&bits, &c, tile(2, 2, 0)), "the corner floods");
    assert!(
        !reached(&bits, &c, tile(5, 5, 0)),
        "the moated island stays unreached"
    );
    assert!(
        !reached(&bits, &c, tile(3, 3, 0)),
        "moat ground is never reached"
    );
    assert!(
        !reached(&bits, &c, tile(0, 0, 1)),
        "unknown levels read false"
    );
}

#[test]
fn reach_sidecar_bits_equal_bake_reach_on_disconnected_and_courtyard_worlds() {
    use crate::pack::{decode_reach_sidecar, encode, encode_reach_sidecar, sha256_hex};
    use sha2::{Digest, Sha256};

    let disconnected = (
        disconnected_world(),
        graph(
            vec![edge(
                TransportKind::Door,
                tile(0, 0, 0),
                tile(1, 1, 0),
                1530,
            )],
            vec![],
        ),
    );
    let courtyard = (
        bake(5, 5, &[(2, 2, CollisionFlag::WALK_BLOCK_FLAGS as u32)]),
        TransportGraph::default(),
    );
    for (name, (c, g)) in [("disconnected", disconnected), ("courtyard", courtyard)] {
        let expected = bake_reach(&c, &g);
        let pack = encode(&c, &g, &[]);
        let digest: [u8; 32] = Sha256::digest(&pack).into();
        let bytes = encode_reach_sidecar(c.origin, c.width, c.height, &expected, &digest);
        let side = decode_reach_sidecar(&bytes).unwrap();
        assert_eq!(
            side.bits, expected,
            "{name}: sidecar bits must equal bake_reach"
        );
        assert_eq!(side.word_count, expected.len());
        assert_eq!(side.origin, c.origin);
        assert_eq!((side.width, side.height), (c.width, c.height));
        assert_eq!(sha256_hex(&side.binding), sha256_hex(&digest));
    }
}

#[test]
fn same_geometry_different_transport_does_not_reuse_reach_binding() {
    use crate::pack::{encode, encode_reach_sidecar, sha256_hex};
    use sha2::{Digest, Sha256};

    let c = disconnected_world();
    let corner = graph(
        vec![edge(
            TransportKind::Door,
            tile(0, 0, 0),
            tile(1, 1, 0),
            1530,
        )],
        vec![],
    );
    let island = graph(
        vec![edge(
            TransportKind::Door,
            tile(5, 5, 0),
            tile(5, 5, 0),
            1531,
        )],
        vec![],
    );
    let bits_a = bake_reach(&c, &corner);
    let bits_b = bake_reach(&c, &island);
    assert_ne!(
        bits_a, bits_b,
        "different seeds must produce different reach"
    );
    let pack_a = encode(&c, &corner, &[]);
    let pack_b = encode(&c, &island, &[]);
    let digest_a: [u8; 32] = Sha256::digest(&pack_a).into();
    let digest_b: [u8; 32] = Sha256::digest(&pack_b).into();
    assert_ne!(digest_a, digest_b);
    let side_a = encode_reach_sidecar(c.origin, c.width, c.height, &bits_a, &digest_a);
    let decoded_a = crate::pack::decode_reach_sidecar(&side_a).unwrap();
    assert_eq!(sha256_hex(&decoded_a.binding), sha256_hex(&digest_a));
    assert_ne!(sha256_hex(&decoded_a.binding), sha256_hex(&digest_b));
    assert_ne!(decoded_a.bits, bits_b);
}

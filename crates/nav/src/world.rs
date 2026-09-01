//! The router's world, loaded from the baked nav pack: the whole-world
//! [`WorldCollision`] walk surface plus the transport [`TransportGraph`]
//! and the content-derived bank stand table ([`crate::pack::BankStand`]).
//! The v8 pack file stores the compact packed walk surface and the
//! transport edges, so the Dijkstra router ([`crate::router::find`])
//! consumes one artifact — live harnesses load this and route on the
//! packed collision. The legacy 274N grid pack (boolean walk bytes +
//! doors) still loads through [`NavWorld::from_grid`] as a fallback for
//! old `.navpack` files.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::grid::StepGrid;
use crate::pack::{decode, load_grid, BankStand, PackError};
use crate::transport::{TransportEdge, TransportGraph, TransportKind};

/// A fully-blocked stamp: every directional `PL_WALK_*` mask, so the
/// router's directional step test rejects the tile from any direction
/// (a 274N grid walk byte is boolean; there are no half-open faces to keep).
const BLOCKED: u32 = CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;

/// The whole-world collision + transport graph + bank stand table the
/// router consumes.
pub struct NavWorld {
    pub collision: WorldCollision,
    pub graph: TransportGraph,
    banks: Vec<BankStand>,
}

impl NavWorld {
    /// The baked bank stands ([`BankStand`]) the banking session walks to
    /// and opens, ordered by tile.
    pub fn banks(&self) -> &[BankStand] {
        &self.banks
    }

    /// Load the baked nav pack (`$NAV_PACK` or the default path) into the
    /// router's world. Whole-world packs (collision + transport graph +
    /// bank stands) load directly; legacy 274N grid packs fall back to
    /// [`Self::from_grid`]. The fallback triggers on bad magic only — a
    /// `274V` pack with a stale version stays [`PackError::BadVersion`]
    /// so the operator rebakes.
    pub fn load_pack(path: &Path) -> Result<Self, PackError> {
        let bytes = std::fs::read(path).map_err(PackError::Io)?;
        match decode(&bytes) {
            Ok((collision, graph, banks)) => Ok(NavWorld {
                collision,
                graph,
                banks,
            }),
            Err(PackError::BadMagic) => Ok(Self::from_grid(&load_grid(path)?)),
            Err(e) => Err(e),
        }
    }

    /// `$NAV_PACK` or `~/.274bot/274bot.navpack` (same default `nav-pack` writes).
    #[cfg(test)]
    pub(crate) fn default_pack_path() -> PathBuf {
        match std::env::var("NAV_PACK") {
            Ok(p) => PathBuf::from(p),
            Err(_) => match std::env::var("HOME") {
                Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
                Err(_) => PathBuf::from(".274bot/274bot.navpack"),
            },
        }
    }

    /// Whole-world pack proofs. GitHub has no pack and no Server tree —
    /// skip instead of panicking. Local with a rebake still runs them.
    #[cfg(test)]
    pub(crate) fn load_default_pack_or_skip() -> Option<Self> {
        let path = Self::default_pack_path();
        match Self::load_pack(&path) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("SKIP: no nav pack at {} ({e:?})", path.display());
                None
            }
        }
    }

    /// Derive the router's world from a 274N grid pack. Walkable tiles
    /// carry no flags; every blocked tile carries [`BLOCKED`] (the 274N
    /// grid has no per-direction data, so a blocked tile is a wall on all
    /// faces). Each pack door edge becomes a 1-tick `Door` transport
    /// edge; the pack stores both directions, so the graph indexes `at`
    /// exactly as the grid authored them.
    pub fn from_grid(grid: &StepGrid) -> Self {
        // The 274N grid is one level-0 plane; the 4-plane buffer keeps
        // upper-level lookups on their own (empty) planes instead of
        // panicking or reusing level 0.
        let plane = grid.width * grid.height;
        let mut flags = vec![0u32; 4 * plane];
        for z in 0..grid.height {
            for x in 0..grid.width {
                let t = crate::tile::Tile {
                    x: grid.origin.x + x as i32,
                    z: grid.origin.z + z as i32,
                    level: grid.origin.level,
                };
                flags[z * grid.width + x] = if grid.walkable(t) { 0 } else { BLOCKED };
            }
        }
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: grid.origin.x,
                z: grid.origin.z,
                level: grid.origin.level,
            },
            width: grid.width,
            height: grid.height,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        for d in &grid.doors {
            let i = graph.edges.len();
            graph.edges.push(TransportEdge {
                kind: TransportKind::Door,
                at: WorldTile {
                    x: d.from.x,
                    z: d.from.z,
                    level: d.from.level,
                },
                to: WorldTile {
                    x: d.to.x,
                    z: d.to.z,
                    level: d.to.level,
                },
                loc_id: d.loc_id,
                option: 1,
                ticks: 1,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
            });
            graph.at.entry(graph.edges[i].at).or_default().push(i);
        }
        NavWorld {
            collision,
            graph,
            banks: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;

    use super::{NavWorld, BLOCKED};
    use crate::collision::{walk_word_from_parts, WorldCollision};
    use crate::grid::StepGrid;
    use crate::pack::{encode, encode_grid, PackError};
    use crate::router::{find, find_allow_teleports, Leg};
    use crate::tile::Tile;
    use crate::transport::{TransportEdge, TransportGraph, TransportKind};

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    #[test]
    fn seers_street_walks_to_rock_crabs_on_foot() {
        // Task 4: Seers street (2725,3485) -> rock crabs (2710,3720) is on
        // foot after the L0 stamper rebake. Requires the pack: run
        // `cargo run -p nav --bin nav-pack` first. GitHub has neither pack
        // nor Server content — skip, do not panic.
        let Some(world) = NavWorld::load_default_pack_or_skip() else {
            return;
        };
        let from = WorldTile {
            x: 2725,
            z: 3485,
            level: 0,
        };
        let to = WorldTile {
            x: 2710,
            z: 3720,
            level: 0,
        };
        let r = find(&world.collision, &world.graph, from, to)
            .expect("on-foot path after stamper rebake");
        assert_eq!(r.dest, to);
        assert!(r.legs.iter().any(|l| matches!(l, Leg::Walk { .. })));
        let walked: Vec<WorldTile> = r
            .legs
            .iter()
            .flat_map(|l| match l {
                Leg::Walk { tiles } => tiles.clone(),
                Leg::Transport { .. } => vec![],
            })
            .collect();
        // No fake hop: every walked tile stays on plane 0 and the walk
        // must span the whole 235-tile north gap (a single invented cliff
        // transport would walk ~nothing).
        assert!(
            walked.iter().all(|t| t.level == 0),
            "every Walk tile stays on plane 0"
        );
        let north = to.z - from.z;
        assert!(north > 0);
        assert!(
            walked.len() >= north as usize,
            "walk spans the north gap ({} walked tiles for {north} north)",
            walked.len()
        );
        // Transports, if any, are road doors the player would Open — never
        // an invented cliff/teleport hop.
        for l in &r.legs {
            if let Leg::Transport { edge } = l {
                assert_eq!(
                    edge.kind,
                    TransportKind::Door,
                    "transport at {:?} is a road door, not a cliff hop",
                    edge.at
                );
            }
        }
    }

    #[test]
    fn open_grid_derives_an_all_walkable_world() {
        let w = NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        assert_eq!(w.collision.origin, tile(0, 0, 0));
        assert_eq!(w.collision.width, 3);
        assert_eq!(w.collision.height, 3);
        for (i, wd) in w.collision.walk.iter().enumerate() {
            assert_eq!(*wd, 0, "word {i} stays open");
        }
        assert!(w.graph.edges.is_empty());
    }

    #[test]
    fn blocked_tiles_stamp_every_direction_mask() {
        let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
        let door_tile = tile(2, 0, 0);
        let idx = (door_tile.z - w.collision.origin.z) as usize * w.collision.width
            + (door_tile.x - w.collision.origin.x) as usize;
        let blocked = (w.collision.blocked[idx >> 6] >> (idx & 63)) & 1 != 0;
        assert_eq!(
            walk_word_from_parts(w.collision.walk[idx], blocked),
            BLOCKED,
            "the full directional stamp is in the packed walk surface"
        );
        // The full directional stamp is in the walk block masks, so the
        // router never steps onto it.
        assert_eq!(
            walk_word_from_parts(w.collision.walk[idx], blocked)
                & CollisionFlag::WALK_BLOCK_FLAGS as u32,
            CollisionFlag::WALK_BLOCK_FLAGS as u32
        );
        assert!(w.collision.walkable(tile(0, 0, 0)));
        assert!(!w.collision.walkable(door_tile));
    }

    #[test]
    fn door_edges_become_transport_edges() {
        let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
        let fwd = w
            .graph
            .edges
            .iter()
            .find(|e| e.kind == TransportKind::Door && e.at == tile(1, 0, 0))
            .expect("door edge from the corridor's west side");
        assert_eq!(fwd.to, tile(3, 0, 0));
        assert_eq!(fwd.loc_id, 1530);
        assert_eq!(fwd.ticks, 1);
        assert_eq!(w.graph.at.get(&tile(1, 0, 0)).map(Vec::len), Some(1));
        // The fixture corridor is a single directed edge.
        assert_eq!(w.graph.at.get(&tile(3, 0, 0)), None);
    }

    #[test]
    fn find_uses_the_derived_world_across_the_corridor() {
        let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
        // The walled tile blocks walking; the door edge crosses it, so the
        // route splits into Walk -> Transport -> Walk legs. The origin is
        // within the door's interact radius of at=(1,0), so the door is
        // taken straight from it.
        let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
        // The 1-tick door taken from the origin + 1 walk tile (0.5).
        assert_eq!(r.ticks, 1.5);
        assert_eq!(r.legs.len(), 3);
        let (Leg::Walk { .. }, Leg::Transport { edge }, Leg::Walk { .. }) =
            (&r.legs[0], &r.legs[1], &r.legs[2])
        else {
            panic!("expected Walk, Transport, Walk legs");
        };
        assert_eq!(edge.at, tile(1, 0, 0));
        assert_eq!(edge.to, tile(3, 0, 0));
        assert_eq!(edge.loc_id, 1530);
    }

    #[test]
    fn load_pack_path_round_trips_a_fixture_grid() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-navworld-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.navpack");
        std::fs::write(&path, encode_grid(&StepGrid::fixture_door_corridor())).unwrap();
        let w = NavWorld::load_pack(&path).expect("pack loads");
        assert_eq!(w.collision.width, 5);
        assert_eq!(w.graph.edges.len(), 1);
        assert!(w.collision.walkable(tile(1, 0, 0)));
        assert!(!w.collision.walkable(tile(2, 0, 0)));
        // The decoded grid routes exactly like the authored one.
        let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
        // The 1-tick door taken from the origin (within the interact
        // radius of at=(1,0)) + 1 walk tile (0.5).
        assert_eq!(r.ticks, 1.5);
        assert_eq!(r.legs.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derived_world_routes_across_a_fixture_rect() {
        let w = NavWorld::from_grid(&StepGrid::fixture_rect_at(
            Tile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            64,
            64,
        ));
        let r = find(
            &w.collision,
            &w.graph,
            tile(3200, 3200, 0),
            tile(3263, 3263, 0),
        )
        .unwrap();
        assert_eq!(r.ticks, 31.5); // 63 run steps at 0.5 ticks each
        let Leg::Walk { tiles } = &r.legs[0] else {
            panic!("walk-only route");
        };
        assert_eq!(tiles.first(), Some(&tile(3200, 3200, 0)));
        assert_eq!(tiles.last(), Some(&tile(3263, 3263, 0)));
    }

    #[test]
    fn load_pack_keeps_stale_v5_wire_as_bad_version() {
        // A leftover v5 (pre-v6) `274V` pack is not a 274N grid pack:
        // load must surface BadVersion(5) so the operator rebakes, not
        // fall into the grid decoder and come out as a confusing BadMagic.
        let plane = vec![0u32; 4];
        let (walk, blocked) = crate::collision::pack_walk(&plane);
        let collision = WorldCollision {
            origin: tile(0, 0, 0),
            width: 2,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let mut bytes = encode(&collision, &TransportGraph::default(), &[]);
        bytes[4] = 5;
        let dir = std::env::temp_dir().join(format!(
            "274bot-navworld-v5-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture-v5.navpack");
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            NavWorld::load_pack(&path),
            Err(PackError::BadVersion(5))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_pack_path_round_trips_a_world() {
        // A 5-tile corridor split by a door: the packed collision and
        // transport graph route exactly like the authored world.
        let mut plane = vec![0u32; 5];
        plane[2] = BLOCKED;
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: tile(0, 0, 0),
            width: 5,
            height: 1,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at: tile(1, 0, 0),
            to: tile(3, 0, 0),
            loc_id: 1530,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });
        graph.at.entry(tile(1, 0, 0)).or_default().push(0);

        let dir = std::env::temp_dir().join(format!(
            "274bot-navworld-roundtrip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.navpack");
        std::fs::write(&path, encode(&collision, &graph, &[])).unwrap();
        let w = NavWorld::load_pack(&path).expect("pack loads");
        assert_eq!(w.collision.origin, tile(0, 0, 0));
        assert_eq!(w.collision.walk, collision.walk);
        assert!(w.collision.flags.is_none());
        assert_eq!(w.graph.edges, graph.edges);
        assert_eq!(w.graph.at, graph.at);
        let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
        // The 1-tick door taken from the origin (within the interact
        // radius of at=(1,0)) + 1 walk tile (0.5).
        assert_eq!(r.ticks, 1.5);
        assert_eq!(r.legs.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn world_round_trips_the_teleport_layer_off_the_default_find() {
        // A 5×5 bake walled down the middle: only the packed any-tile
        // teleport can cross it, and only under find_allow_teleports.
        let mut plane = vec![0u32; 25];
        for z in 0..5 {
            plane[z * 5 + 1] = BLOCKED;
            plane[z * 5 + 2] = BLOCKED;
        }
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: tile(0, 0, 0),
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            at: tile(0, 0, 0),
            to: tile(4, 4, 0),
            loc_id: 0,
            option: 0,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![(6, 25)],
            item_req: vec![(554, 1), (556, 3), (563, 1)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });

        let dir = std::env::temp_dir().join(format!(
            "274bot-navworld-teles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture-teles.navpack");
        std::fs::write(&path, encode(&collision, &graph, &[])).unwrap();
        let w = NavWorld::load_pack(&path).expect("pack loads");
        assert_eq!(w.graph.teleports, graph.teleports);
        assert!(w.graph.edges.is_empty());
        assert!(w.graph.at.is_empty());
        // Default find ignores the teleport layer entirely…
        assert!(find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 4, 0)).is_err());
        // …and find_allow_teleports unions it in from anywhere (the state
        // proves the Varrock cast: Magic 25 + fire/air/law runes).
        let cast = crate::world_state::WorldState {
            stats: std::collections::HashMap::from([(6, 25)]),
            inv: std::collections::HashMap::from([(554, 1), (556, 3), (563, 1)]),
            ..crate::world_state::WorldState::default()
        };
        let r = find_allow_teleports(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 4, 0), &cast)
            .unwrap();
        assert_eq!(r.ticks, 3.0);
        assert!(r.legs.iter().any(|l| matches!(l, Leg::Transport { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_pack_round_trips_bank_stands() {
        // The v8 pack stores the bank stand table; NavWorld::banks exposes
        // it to the Banking session.
        let mut plane = vec![0u32; 4];
        plane[0] = BLOCKED;
        let mut flags = vec![0u32; 4 * plane.len()];
        flags[..plane.len()].copy_from_slice(&plane);
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: tile(0, 0, 0),
            width: 2,
            height: 2,
            walk,
            blocked,
            flags: None,
        };
        let banks = vec![crate::pack::BankStand {
            name: "Bank booth".into(),
            tile: tile(1, 1, 0),
            access: crate::pack::BankAccess::Booth { op: 2 },
        }];
        let dir = std::env::temp_dir().join(format!(
            "274bot-navworld-banks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture-banks.navpack");
        std::fs::write(
            &path,
            encode(&collision, &TransportGraph::default(), &banks),
        )
        .unwrap();
        let w = NavWorld::load_pack(&path).expect("pack loads");
        assert_eq!(w.banks(), &banks);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

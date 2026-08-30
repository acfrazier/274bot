//! Random walkable seed tiles for the 50-box wall (nav world, else Lumbridge).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use api::snapshot::WorldTile;
use nav::world::NavWorld;

/// Lumbridge courtyard fallback when no nav pack is on disk.
const LUMBRIDGE: WorldTile = WorldTile {
    x: 3220,
    z: 3218,
    level: 0,
};

fn pack_path() -> PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

/// The world's level-0 walkable tiles, row-major (z then x): a tile is a
/// seed candidate when the collision bake's blanket `walkable` check passes
/// (the standable test, not a directional mask).
fn walkable_seeds(world: &NavWorld) -> Vec<WorldTile> {
    let c = &world.collision;
    let o = c.origin;
    (0..c.height)
        .flat_map(|z| {
            (0..c.width).map(move |x| WorldTile {
                x: o.x + x as i32,
                z: o.z + z as i32,
                level: o.level,
            })
        })
        .filter(|t| {
            c.walkable(WorldTile {
                x: t.x,
                z: t.z,
                level: t.level,
            })
        })
        .collect()
}

fn shuffled_walkable() -> &'static [WorldTile] {
    static TILES: OnceLock<Vec<WorldTile>> = OnceLock::new();
    TILES.get_or_init(|| {
        let mut tiles = NavWorld::load_pack(&pack_path())
            .map(|w| walkable_seeds(&w))
            .unwrap_or_default();
        if tiles.is_empty() {
            tiles.push(LUMBRIDGE);
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let mut rng = nanos ^ 0x9e37_79b9_7f4a_7c15;
        for i in (1..tiles.len()).rev() {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let j = (rng as usize) % (i + 1);
            tiles.swap(i, j);
        }
        tiles
    })
}

/// A walkable tile for this uid (stable for the process, shuffled pack).
pub fn scatter_tile_for(uid: i32) -> WorldTile {
    let tiles = shuffled_walkable();
    tiles[(uid.unsigned_abs() as usize) % tiles.len()]
}

/// `::tele` payload for tests.
pub fn tele_args(tile: WorldTile) -> String {
    api::interact::tele_args(tile.level, tile.x, tile.z)
}

#[cfg(test)]
mod tests {
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;
    use nav::collision::WorldCollision;
    use nav::transport::TransportGraph;

    use super::*;

    #[test]
    fn scatter_tile_for_is_deterministic_in_process() {
        let a = scatter_tile_for(274_000_100);
        let b = scatter_tile_for(274_000_100);
        assert_eq!(a, b);
    }

    #[test]
    fn lumbridge_tele_args_match_off_island_shape() {
        let t = LUMBRIDGE;
        assert_eq!(tele_args(t), api::interact::tele_args(0, 3220, 3218));
    }

    #[test]
    fn walkable_seeds_lists_only_level0_walkable_tiles() {
        // A 3x2 world with the (3200,3201) cell blocked: seeds come only
        // from the collision bake's blanket `walkable` check.
        let mut flags = vec![0u32; 6];
        flags[3] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                },
                width: 3,
                height: 2,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph: TransportGraph::default(),
        };
        let seeds = walkable_seeds(&world);
        assert_eq!(seeds.len(), 5);
        assert!(!seeds.contains(&WorldTile {
            x: 3200,
            z: 3201,
            level: 0
        }));
        assert!(seeds.contains(&WorldTile {
            x: 3201,
            z: 3200,
            level: 0
        }));
    }
}

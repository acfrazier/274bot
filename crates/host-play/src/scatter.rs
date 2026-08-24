//! Random walkable seed tiles for the 50-box wall (nav pack, else Lumbridge).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use nav::pack::{load_pack, walkable_dots};
use nav::tile::Tile;

/// Lumbridge courtyard fallback when no nav pack is on disk.
const LUMBRIDGE: Tile = Tile {
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

fn shuffled_walkable() -> &'static [Tile] {
    static TILES: OnceLock<Vec<Tile>> = OnceLock::new();
    TILES.get_or_init(|| {
        let mut tiles = load_pack(&pack_path())
            .map(|g| {
                (0..4)
                    .flat_map(|level| walkable_dots(&g, level))
                    .collect::<Vec<_>>()
            })
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
pub fn scatter_tile_for(uid: i32) -> Tile {
    let tiles = shuffled_walkable();
    tiles[(uid.unsigned_abs() as usize) % tiles.len()]
}

/// `::tele` payload for tests.
pub fn tele_args(tile: Tile) -> String {
    api::interact::tele_args(tile.level, tile.x, tile.z)
}

#[cfg(test)]
mod tests {
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
}

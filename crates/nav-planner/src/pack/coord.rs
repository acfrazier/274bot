//! Packed v18 coordinate keys: `level << 28 | x << 14 | z`.
//!
//! Every v18 coordinate key uses this order, including S3 multiway keys
//! and packed game tiles. Bits 30 and 31 stay zero.

use api::snapshot::WorldTile;

use super::PackError;

/// Inclusive plane count for packed keys (`level` in `0..LEVELS`).
pub const KEY_LEVELS: i32 = 4;
/// Packed x and z each occupy 14 bits.
pub const KEY_COORD_BITS: u32 = 14;
/// Exclusive upper bound for packed x and z.
pub const KEY_COORD_MAX: i32 = 1 << KEY_COORD_BITS;
const KEY_COORD_MASK: u32 = (1 << KEY_COORD_BITS) - 1;
const KEY_RESERVED_MASK: u32 = 0xc000_0000;

/// Pack a game tile as `level << 28 | x << 14 | z`.
///
/// Rejects a plane outside `0..4`, a coordinate outside `0..1<<14`, or a
/// value that would set bits 30 or 31.
pub fn pack_tile_key(tile: WorldTile) -> Result<u32, PackError> {
    if !(0..KEY_LEVELS).contains(&tile.level) {
        return Err(PackError::BadLength(format!(
            "packed tile level {} is outside 0..{KEY_LEVELS}",
            tile.level
        )));
    }
    if !(0..KEY_COORD_MAX).contains(&tile.x) || !(0..KEY_COORD_MAX).contains(&tile.z) {
        return Err(PackError::BadLength(format!(
            "packed tile ({}, {}, {}) has a coordinate outside 0..{KEY_COORD_MAX}",
            tile.x, tile.z, tile.level
        )));
    }
    let key = ((tile.level as u32) << 28) | ((tile.x as u32) << 14) | (tile.z as u32);
    if key & KEY_RESERVED_MASK != 0 {
        return Err(PackError::BadLength(
            "packed tile key sets reserved bits 30 or 31".into(),
        ));
    }
    Ok(key)
}

/// Decode a packed v18 tile key. Reserved bits 30 and 31 must be zero.
pub fn unpack_tile_key(key: u32) -> Result<WorldTile, PackError> {
    if key & KEY_RESERVED_MASK != 0 {
        return Err(PackError::BadLength(format!(
            "packed tile key {key:#x} sets reserved bits 30 or 31"
        )));
    }
    let z = (key & KEY_COORD_MASK) as i32;
    let x = ((key >> 14) & KEY_COORD_MASK) as i32;
    let level = (key >> 28) as i32;
    if !(0..KEY_LEVELS).contains(&level) {
        return Err(PackError::BadLength(format!(
            "packed tile key {key:#x} has level {level} outside 0..{KEY_LEVELS}"
        )));
    }
    Ok(WorldTile { x, z, level })
}

#[cfg(test)]
mod tests {
    use super::{pack_tile_key, unpack_tile_key, KEY_COORD_MAX};
    use api::snapshot::WorldTile;

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    #[test]
    fn pack_uses_level_then_x_then_z() {
        let key = pack_tile_key(tile(1, 2, 3)).unwrap();
        assert_eq!(key, (3 << 28) | (1 << 14) | 2);
        assert_eq!(unpack_tile_key(key).unwrap(), tile(1, 2, 3));
    }

    #[test]
    fn pack_rejects_out_of_range_plane_and_coordinates() {
        assert!(pack_tile_key(tile(0, 0, 4)).is_err());
        assert!(pack_tile_key(tile(0, 0, -1)).is_err());
        assert!(pack_tile_key(tile(KEY_COORD_MAX, 0, 0)).is_err());
        assert!(pack_tile_key(tile(0, KEY_COORD_MAX, 0)).is_err());
        assert!(pack_tile_key(tile(-1, 0, 0)).is_err());
    }

    #[test]
    fn unpack_rejects_reserved_bits() {
        let key = pack_tile_key(tile(3, 4, 1)).unwrap();
        assert!(unpack_tile_key(key | (1 << 30)).is_err());
        assert!(unpack_tile_key(key | (1 << 31)).is_err());
    }
}

//! Survivable S3 multiway key list: a little-endian `u32` count followed by
//! that many sorted unique packed v18 tile keys.

use super::coord::{pack_tile_key, unpack_tile_key};
use super::{read_u32, PackError, PackRead};

/// Encode sorted unique packed multiway keys as a section body.
pub fn encode_multiway_keys(keys: &[u32]) -> Result<Vec<u8>, PackError> {
    validate_multiway_keys(keys)?;
    let count = u32::try_from(keys.len())
        .map_err(|_| PackError::BadLength("multiway key count exceeds u32".into()))?;
    let bytes = count
        .checked_mul(4)
        .and_then(|payload| payload.checked_add(4))
        .ok_or_else(|| PackError::BadLength("multiway section size overflows".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(bytes as usize).map_err(|error| {
        PackError::BadLength(format!(
            "cannot reserve {bytes} bytes for multiway keys: {error}"
        ))
    })?;
    out.extend_from_slice(&count.to_le_bytes());
    for key in keys {
        out.extend_from_slice(&key.to_le_bytes());
    }
    Ok(out)
}

/// Decode a complete multiway section from its payload bytes.
pub fn decode_multiway_section(bytes: &[u8]) -> Result<Box<[u32]>, PackError> {
    use std::io::Cursor;
    decode_multiway_keys(&mut Cursor::new(bytes))
}

/// Decode a multiway key list from the current reader position.
pub(crate) fn decode_multiway_keys<R: PackRead>(r: &mut R) -> Result<Box<[u32]>, PackError> {
    let count = read_u32(r)? as usize;
    let needed = count
        .checked_mul(4)
        .ok_or_else(|| PackError::BadLength("multiway key count overflows".into()))?;
    if needed > r.remaining() {
        return Err(PackError::Truncated);
    }
    let mut keys = Vec::with_capacity(count);
    for _ in 0..count {
        keys.push(read_u32(r)?);
    }
    validate_multiway_keys(&keys)?;
    Ok(keys.into_boxed_slice())
}

/// Sorted unique packed keys, each a valid v18 coordinate.
pub fn validate_multiway_keys(keys: &[u32]) -> Result<(), PackError> {
    let mut previous = None;
    for &key in keys {
        unpack_tile_key(key)?;
        if let Some(last) = previous {
            if key <= last {
                return Err(PackError::BadLength(
                    "multiway keys must be sorted unique packed tiles".into(),
                ));
            }
        }
        previous = Some(key);
    }
    Ok(())
}

/// Pack already-checked game tiles into sorted unique v18 keys.
pub fn keys_from_tiles(
    tiles: impl IntoIterator<Item = api::snapshot::WorldTile>,
) -> Result<Box<[u32]>, PackError> {
    let mut keys = Vec::new();
    for tile in tiles {
        keys.push(pack_tile_key(tile)?);
    }
    keys.sort_unstable();
    keys.dedup();
    validate_multiway_keys(&keys)?;
    Ok(keys.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::{decode_multiway_keys, encode_multiway_keys, validate_multiway_keys};
    use crate::pack::coord::pack_tile_key;
    use api::snapshot::WorldTile;
    use std::io::Cursor;

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    #[test]
    fn section_roundtrips_sorted_keys() {
        let keys = [
            pack_tile_key(tile(8, 16, 0)).unwrap(),
            pack_tile_key(tile(40, 8, 0)).unwrap(),
            pack_tile_key(tile(0, 0, 1)).unwrap(),
        ];
        let mut sorted = keys;
        sorted.sort_unstable();
        let bytes = encode_multiway_keys(&sorted).unwrap();
        let decoded = super::decode_multiway_section(&bytes).unwrap();
        assert_eq!(&*decoded, &sorted);
    }

    #[test]
    fn decoder_rejects_unsorted_or_duplicate_keys() {
        let a = pack_tile_key(tile(1, 1, 0)).unwrap();
        let b = pack_tile_key(tile(2, 2, 0)).unwrap();
        assert!(validate_multiway_keys(&[b, a]).is_err());
        assert!(validate_multiway_keys(&[a, a]).is_err());
    }

    #[test]
    fn decoder_rejects_truncated_payload() {
        let mut bytes = 2u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            decode_multiway_keys(&mut Cursor::new(bytes.as_slice())),
            Err(crate::pack::PackError::Truncated)
        ));
    }
}

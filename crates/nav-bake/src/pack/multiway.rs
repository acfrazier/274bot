//! Survivable S3 multiway producer: read a revision's `maps/multiway.csv`
//! and emit sorted unique packed v18 keys.

use std::path::Path;

use api::snapshot::WorldTile;
use nav_planner::pack::{pack_tile_key, PackError};

use crate::GenerationError;

const SQUARE: i32 = 64;
const LEVELS: i32 = 4;

/// Failures while reading or packing a revision's multiway table.
#[derive(Debug, thiserror::Error)]
pub enum MultiwayError {
    /// A data line is not `level_mapX_mapZ_localX_localZ`.
    #[error("multiway line {line}: {token:?} is not level_mapX_mapZ_localX_localZ")]
    Malformed { line: usize, token: String },
    /// A packed-coord component is not an integer.
    #[error("multiway line {line}: {token:?} has a non-integer {component}")]
    ComponentInvalid {
        line: usize,
        token: String,
        component: &'static str,
    },
    /// The plane is outside `0..=3`.
    #[error("multiway line {line}: {token:?} has level {level} outside 0..=3")]
    LevelInvalid {
        line: usize,
        token: String,
        level: i32,
    },
    /// A local mapsquare coordinate is outside `0..64`.
    #[error("multiway line {line}: {token:?} has a local coordinate outside 0..64")]
    LocalInvalid { line: usize, token: String },
    /// A mapsquare coordinate expands past the 32-bit tile range.
    #[error("multiway line {line}: {token:?} expands outside the 32-bit tile range")]
    CoordinateOverflow { line: usize, token: String },
    /// Packing the resolved game tile failed bounds checks.
    #[error("multiway line {line}: {source}")]
    Pack {
        line: usize,
        #[source]
        source: PackError,
    },
    /// The multiway table could not be read.
    #[error("multiway {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Produce sorted unique packed keys from a `multiway.csv` body.
pub fn produce_multiway_keys(csv: &str) -> Result<Box<[u32]>, MultiwayError> {
    let mut keys = Vec::new();
    for (index, raw) in csv.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let tile = parse_content_coord(index + 1, line)?;
        let key = pack_tile_key(tile).map_err(|source| MultiwayError::Pack {
            line: index + 1,
            source,
        })?;
        keys.push(key);
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys.into_boxed_slice())
}

/// Read a revision's `maps/multiway.csv` and produce packed keys.
pub fn produce_multiway_keys_from_path(path: &Path) -> Result<Box<[u32]>, GenerationError> {
    let csv = std::fs::read_to_string(path).map_err(|source| MultiwayError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(produce_multiway_keys(&csv)?)
}

fn parse_content_coord(line: usize, token: &str) -> Result<WorldTile, MultiwayError> {
    let mut parts = token.split('_');
    let mut next = |component: &'static str| -> Result<i32, MultiwayError> {
        let part = parts.next().ok_or_else(|| MultiwayError::Malformed {
            line,
            token: token.to_owned(),
        })?;
        part.parse::<i32>()
            .map_err(|_| MultiwayError::ComponentInvalid {
                line,
                token: token.to_owned(),
                component,
            })
    };
    let level = next("level")?;
    let map_x = next("mapX")?;
    let map_z = next("mapZ")?;
    let local_x = next("localX")?;
    let local_z = next("localZ")?;
    if parts.next().is_some() {
        return Err(MultiwayError::Malformed {
            line,
            token: token.to_owned(),
        });
    }
    if !(0..LEVELS).contains(&level) {
        return Err(MultiwayError::LevelInvalid {
            line,
            token: token.to_owned(),
            level,
        });
    }
    if !(0..SQUARE).contains(&local_x) || !(0..SQUARE).contains(&local_z) {
        return Err(MultiwayError::LocalInvalid {
            line,
            token: token.to_owned(),
        });
    }
    // `i32::MAX * 64` would wrap in release and panic in the dev profile.
    let expand = |map: i32, local: i32| {
        map.checked_mul(SQUARE)
            .and_then(|base| base.checked_add(local))
            .ok_or_else(|| MultiwayError::CoordinateOverflow {
                line,
                token: token.to_owned(),
            })
    };
    Ok(WorldTile {
        x: expand(map_x, local_x)?,
        z: expand(map_z, local_z)?,
        level,
    })
}

#[cfg(test)]
mod tests {
    use super::{produce_multiway_keys, MultiwayError};
    use api::snapshot::WorldTile;
    use nav_planner::pack::{pack_tile_key, unpack_tile_key};

    #[test]
    fn producer_skips_comments_sorts_and_deduplicates() {
        let csv = "\
// wilderness
0_46_61_40_8
0_46_61_40_16
0_46_61_40_8
1_0_0_0_0
";
        let keys = produce_multiway_keys(csv).unwrap();
        assert_eq!(keys.len(), 3);
        let tiles: Vec<_> = keys
            .iter()
            .map(|&key| unpack_tile_key(key).unwrap())
            .collect();
        assert_eq!(
            tiles,
            [
                WorldTile {
                    x: 46 * 64 + 40,
                    z: 61 * 64 + 8,
                    level: 0
                },
                WorldTile {
                    x: 46 * 64 + 40,
                    z: 61 * 64 + 16,
                    level: 0
                },
                WorldTile {
                    x: 0,
                    z: 0,
                    level: 1
                },
            ]
        );
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            keys[0],
            pack_tile_key(WorldTile {
                x: 46 * 64 + 40,
                z: 61 * 64 + 8,
                level: 0
            })
            .unwrap()
        );
    }

    #[test]
    fn producer_rejects_malformed_and_out_of_range_rows() {
        assert!(matches!(
            produce_multiway_keys("not-a-coord").unwrap_err(),
            MultiwayError::ComponentInvalid { line: 1, .. }
        ));
        assert!(matches!(
            produce_multiway_keys("0_1_2_3").unwrap_err(),
            MultiwayError::Malformed { line: 1, .. }
        ));
        assert!(matches!(
            produce_multiway_keys("4_0_0_0_0").unwrap_err(),
            MultiwayError::LevelInvalid { level: 4, .. }
        ));
        assert!(matches!(
            produce_multiway_keys("0_0_0_64_0").unwrap_err(),
            MultiwayError::LocalInvalid { .. }
        ));
        assert!(matches!(
            produce_multiway_keys("0_a_0_0_0").unwrap_err(),
            MultiwayError::ComponentInvalid {
                component: "mapX",
                ..
            }
        ));
        for token in [
            "0_67108914_0_0_0",
            "0_2147483647_0_0_0",
            "0_0_2147483647_0_0",
        ] {
            assert!(
                matches!(
                    produce_multiway_keys(token).unwrap_err(),
                    MultiwayError::CoordinateOverflow { line: 1, .. }
                ),
                "{token}"
            );
        }
        assert!(matches!(
            produce_multiway_keys("0_256_0_0_0").unwrap_err(),
            MultiwayError::Pack { line: 1, .. }
        ));
    }

    #[test]
    fn real_289_multiway_keys_match_distinct_content_rows() {
        let Some(path) = api::test_support::local_asset_path(
            "BOT_NAV_CONTENT_DIR_MULTIWAY_CSV",
            "/Users/acfrazier/experiments/lostcity-289/content/maps/multiway.csv",
            "289 content maps/multiway.csv",
        ) else {
            return;
        };
        let csv = std::fs::read_to_string(&path).expect("289 multiway.csv");
        let keys = produce_multiway_keys(&csv).unwrap();

        let mut content_tiles = std::collections::BTreeSet::new();
        for (index, raw) in csv.lines().enumerate() {
            let row = raw.trim();
            if row.is_empty() || row.starts_with("//") {
                continue;
            }
            let mut parts = row.split('_');
            let mut component = || {
                let part = parts
                    .next()
                    .unwrap_or_else(|| panic!("multiway line {}: {row:?}", index + 1));
                part.parse::<i32>()
                    .unwrap_or_else(|_| panic!("multiway line {}: {row:?}", index + 1))
            };
            let level = component();
            let map_x = component();
            let map_z = component();
            let local_x = component();
            let local_z = component();
            assert!(
                parts.next().is_none(),
                "multiway line {}: {row:?}",
                index + 1
            );
            content_tiles.insert((level, map_x * 64 + local_x, map_z * 64 + local_z));
        }

        assert!(
            keys.windows(2).all(|pair| pair[0] < pair[1]),
            "multiway keys must be strictly sorted, with no duplicates"
        );
        let packed_tiles: std::collections::BTreeSet<_> = keys
            .iter()
            .map(|&key| {
                let tile = unpack_tile_key(key).unwrap();
                (tile.level, tile.x, tile.z)
            })
            .collect();
        assert_eq!(
            packed_tiles.len(),
            keys.len(),
            "multiway keys must not repeat a tile"
        );
        assert_eq!(
            packed_tiles, content_tiles,
            "every packed key must trace to a row of the CSV it read"
        );
        assert_eq!(
            keys.len(),
            content_tiles.len(),
            "key count must equal the distinct rows counted independently from the CSV"
        );

        let bytes = nav_planner::pack::encode_multiway_keys(&keys).unwrap();
        let decoded = nav_planner::pack::decode_multiway_section(&bytes).unwrap();
        assert_eq!(&*decoded, &*keys);
    }
}

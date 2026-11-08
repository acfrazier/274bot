use super::*;

fn collision() -> WorldCollision {
    WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 2,
        height: 2,
        walk: vec![0; 16],
        blocked: vec![0],
        flags: None,
    }
}

#[test]
fn canlight_cell_index_uses_geometry_and_level_planes() {
    let collision = collision();
    assert_eq!(canlight_cell_index(&collision, collision.origin), Some(0));
    assert_eq!(
        canlight_cell_index(
            &collision,
            WorldTile {
                x: 3201,
                z: 3201,
                level: 0,
            },
        ),
        Some(3)
    );
    assert_eq!(
        canlight_cell_index(
            &collision,
            WorldTile {
                x: 3200,
                z: 3200,
                level: 2,
            },
        ),
        Some(8)
    );
    assert_eq!(
        canlight_cell_index(
            &collision,
            WorldTile {
                x: 3202,
                z: 3200,
                level: 0,
            },
        ),
        None
    );
    assert_eq!(
        canlight_cell_index(
            &collision,
            WorldTile {
                x: 3200,
                z: 3200,
                level: 4,
            },
        ),
        None
    );
}

#[test]
fn canlight_at_reads_only_the_geometry_indexed_bit() {
    let collision = collision();
    let bits = [1u64 << 3 | 1u64 << 8];
    assert!(canlight_at(
        &bits,
        &collision,
        WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
    ));
    assert!(canlight_at(
        &bits,
        &collision,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 2,
        },
    ));
    assert!(!canlight_at(&bits, &collision, collision.origin));
    assert!(!canlight_at(
        &bits,
        &collision,
        WorldTile {
            x: 3202,
            z: 3200,
            level: 0,
        },
    ));
}

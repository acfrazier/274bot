use super::*;
use crate::collision::bake_from_maps;
use api::obj_names::LocDefs;
use client::config::LocType;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

struct FixtureDir(PathBuf);

impl FixtureDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("274bot-canlight-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        FixtureDir(dir)
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn defs(locs: &[LocType]) -> LocDefs {
    LocDefs::from_locs(locs)
}

fn open_map(extra_loc: &str) -> String {
    format!(
        "==== MAP ====\n\
             0 0 0: h1 o6 u48\n\
             ==== LOC ====\n\
             {extra_loc}"
    )
}

#[test]
fn ve_bank_pairs_unpack_to_the_diagnosed_boxes() {
    let a = unpack_packed_coord("0_50_53_50_24").unwrap();
    let b = unpack_packed_coord("1_50_53_57_32").unwrap();
    assert_eq!(
        (a.x, a.z, a.level, b.x, b.z, b.level),
        (3250, 3416, 0, 3257, 3424, 1)
    );
    let c = unpack_packed_coord("0_50_53_53_33").unwrap();
    let d = unpack_packed_coord("0_50_53_53_35").unwrap();
    assert_eq!((c.x, c.z, d.x, d.z), (3253, 3425, 3253, 3427));
    let zone = BankZone { from: a, to: b };
    assert!(zone.contains(WorldTile {
        x: 3252,
        z: 3420,
        level: 0
    }));
    assert!(!zone.contains(WorldTile {
        x: 3261,
        z: 3429,
        level: 0
    }));
}

#[test]
fn parse_bank_zones_reads_coord_pairs_and_rejects_empty_or_malformed() {
    let text = "\
[bank_zones]
table=coord_pair_table
// varrock east bank
data=coord_pair,0_50_53_50_24,1_50_53_57_32
data=coord_pair,0_50_53_53_33,0_50_53_53_35
";
    let zones = parse_bank_zones(text).unwrap();
    assert_eq!(zones.len(), 2);
    assert!(parse_bank_zones("").unwrap_err().contains("header"));
    assert!(parse_bank_zones("[bank_zones]\ntable=coord_pair_table\n")
        .unwrap_err()
        .contains("no coord_pair"));
    assert!(
        parse_bank_zones("[bank_zones]\ndata=coord_pair,not-a-coord,0_0_0_0_0\n")
            .unwrap_err()
            .contains("packed coord")
    );
    assert!(parse_bank_zones("[bank_zones]\ndata=npc,1,2\n")
        .unwrap_err()
        .contains("coord_pair"));
    assert!(parse_bank_zones("[bank_zones]\nbanana=1\n")
        .unwrap_err()
        .contains("unexpected"));
}

#[test]
fn policy_digest_tracks_bank_bytes_revision_and_algorithm_not_pack() {
    let a = policy_digest(289, b"zones-a");
    let b = policy_digest(289, b"zones-b");
    let c = policy_digest(274, b"zones-a");
    assert_ne!(a, b);
    assert_ne!(a, c);
    let pack = [0x11u8; 32];
    assert_ne!(header_binding(&pack, &a), header_binding(&pack, &b));
    assert_eq!(header_binding(&pack, &a), header_binding(&[0x11u8; 32], &a));
    let other_pack = [0x22u8; 32];
    assert_ne!(header_binding(&pack, &a), header_binding(&other_pack, &a));
}

#[test]
fn equal_pack_inputs_with_different_active_loc_masks_have_different_identities() {
    let fix = FixtureDir::new("identity-mask");
    let open = "==== MAP ====\n0 0 0: h1 o6 u48\n==== LOC ====\n";
    let moved = "==== MAP ====\n0 0 0: h1 o6 u48\n==== LOC ====\n0 1 0: 77 0 0\n";
    let locs = [LocType {
        id: 77,
        blockwalk: false,
        active: true,
        ..LocType::default()
    }];
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (open_collision, open_bits) = bake_square(&fix.0, "m50_50.jm2", open, &locs, &[zone]);
    fs::write(fix.0.join("m50_50.jm2"), moved).unwrap();
    let moved_collision = bake_from_maps(&fix.0, &defs(&locs), &HashSet::new()).unwrap();
    let moved_flags = moved_collision.flags.as_ref().unwrap();
    let moved_bits =
        bake_canlight(&moved_collision, moved_flags, &fix.0, &defs(&locs), &[zone]).unwrap();
    let open_pack = crate::pack::encode(
        &open_collision,
        &crate::transport::TransportGraph::default(),
        &[],
    );
    let moved_pack = crate::pack::encode(
        &moved_collision,
        &crate::transport::TransportGraph::default(),
        &[],
    );
    assert_eq!(
        open_pack, moved_pack,
        "nonblocking loc move keeps pack equal"
    );
    assert_ne!(open_bits, moved_bits, "active loc changes the baked mask");
    let open_id = identity_digest(289, b"same-bank-zones", &open_bits);
    let moved_id = identity_digest(289, b"same-bank-zones", &moved_bits);
    assert_ne!(open_id, moved_id, "mask identity follows baked output");
}

#[test]
fn expected_header_binding_roundtrips_hex_and_rejects_malformed() {
    let pack = [0xABu8; 32];
    let policy = policy_digest(289, b"row");
    let binding = header_binding(&pack, &policy);
    let got = expected_header_binding(
        &crate::pack::sha256_hex(&pack),
        &crate::pack::sha256_hex(&policy),
    )
    .unwrap();
    assert_eq!(got, binding);
    assert!(expected_header_binding("zz", &"aa".repeat(32)).is_err());
    assert!(expected_header_binding(&"aa".repeat(32), "short").is_err());
}

fn bake_square(
    maps: &Path,
    name: &str,
    text: &str,
    locs: &[LocType],
    zones: &[BankZone],
) -> (WorldCollision, Vec<u64>) {
    fs::write(maps.join(name), text).unwrap();
    let collision = bake_from_maps(maps, &defs(locs), &HashSet::new()).unwrap();
    let flags = collision.flags.as_ref().unwrap();
    let bits = bake_canlight(&collision, flags, maps, &defs(locs), zones).unwrap();
    (collision, bits)
}

fn ve_zone() -> BankZone {
    BankZone {
        from: unpack_packed_coord("0_50_53_50_24").unwrap(),
        to: unpack_packed_coord("1_50_53_57_32").unwrap(),
    }
}

#[test]
fn bank_inside_is_unset_outside_is_set_on_open_ground() {
    let fix = FixtureDir::new("bank-in-out");
    let text = "\
==== MAP ====
0 52 28: h1 o6 u48
0 61 37: h1 o6 u48
==== LOC ====
";
    let (c, bits) = bake_square(&fix.0, "m50_53.jm2", text, &[], &[ve_zone()]);
    let inside = WorldTile {
        x: 3252,
        z: 3420,
        level: 0,
    };
    let outside = WorldTile {
        x: 3261,
        z: 3429,
        level: 0,
    };
    assert!(c.walkable(inside), "bank floor is walkable");
    assert!(c.walkable(outside));
    assert!(!canlight_at(&bits, &c, inside));
    assert!(canlight_at(&bits, &c, outside));
    assert!(!canlight_at(
        &bits,
        &c,
        WorldTile {
            x: 3252,
            z: 3420,
            level: 1
        }
    ));
}

#[test]
fn wr_grnd_and_walk_scenery_unset_and_face_only_does_not() {
    let fix = FixtureDir::new("raw-flags");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
0 0 1: f1 u48
==== LOC ====
0 0 2: 10 10 0
0 0 3: 11 0 0
";
    let locs = [
        LocType {
            id: 10,
            width: 1,
            length: 1,
            blockwalk: true,
            active: false,
            ..LocType::default()
        },
        LocType {
            id: 11,
            blockwalk: true,
            active: false,
            ..LocType::default()
        },
    ];
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (c, bits) = bake_square(&fix.0, "m50_50.jm2", text, &locs, &[zone]);
    let open = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let floor = WorldTile {
        x: 3200,
        z: 3201,
        level: 0,
    };
    let scenery = WorldTile {
        x: 3200,
        z: 3202,
        level: 0,
    };
    let wall = WorldTile {
        x: 3200,
        z: 3203,
        level: 0,
    };
    assert!(canlight_at(&bits, &c, open));
    assert!(!canlight_at(&bits, &c, floor));
    assert_ne!(
        c.flag(floor.x, floor.z, floor.level) & CollisionFlag::WR_GRND as u32,
        0
    );
    assert!(!canlight_at(&bits, &c, scenery));
    assert_ne!(
        c.flag(scenery.x, scenery.z, scenery.level) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
    // Closed-style wall face is not MAP_BLOCKED / locaddunsafe when the
    // loc is inactive: walkable() may still be true; canlight stays set.
    assert_eq!(
        c.flag(wall.x, wall.z, wall.level) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
    assert!(canlight_at(&bits, &c, wall));
    assert!(c.walkable(open));
    assert!(!c.walkable(floor));
    assert!(!c.walkable(scenery));
}

#[test]
fn ground_decor_needs_blockwalk_and_active_like_change_loc_collision() {
    let fix = FixtureDir::new("ground-decor");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 0 1: 20 22 0
0 0 2: 21 22 0
";
    let locs = [
        LocType {
            id: 20,
            blockwalk: true,
            active: true,
            ..LocType::default()
        },
        LocType {
            id: 21,
            blockwalk: false,
            active: true,
            ..LocType::default()
        },
    ];
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (c, bits) = bake_square(&fix.0, "m50_50.jm2", text, &locs, &[zone]);
    let blocked = WorldTile {
        x: 3200,
        z: 3201,
        level: 0,
    };
    let active_only = WorldTile {
        x: 3200,
        z: 3202,
        level: 0,
    };
    assert_ne!(
        c.flag(blocked.x, blocked.z, blocked.level) & CollisionFlag::WR_GRND as u32,
        0
    );
    assert!(!canlight_at(&bits, &c, blocked));
    // active && !blockwalk does not stamp FLOOR, but MAP_LOCADDUNSAFE
    // still covers the origin (conservative static active loc).
    assert_eq!(
        c.flag(active_only.x, active_only.z, active_only.level) & CollisionFlag::WR_GRND as u32,
        0
    );
    assert!(!canlight_at(&bits, &c, active_only));
    assert!(canlight_at(
        &bits,
        &c,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }
    ));
}

#[test]
fn rotated_active_footprint_unsets_the_span_even_when_not_blockwalk() {
    let fix = FixtureDir::new("rotated");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
0 1 1: 30 10 1
";
    let locs = [LocType {
        id: 30,
        width: 2,
        length: 1,
        blockwalk: false,
        active: true,
        ..LocType::default()
    }];
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (c, bits) = bake_square(&fix.0, "m50_50.jm2", text, &locs, &[zone]);
    // NORTH swaps to span_x=length=1, span_z=width=2 → (3201,3201) and (3201,3202).
    let origin = WorldTile {
        x: 3201,
        z: 3201,
        level: 0,
    };
    let extra = WorldTile {
        x: 3201,
        z: 3202,
        level: 0,
    };
    let beside = WorldTile {
        x: 3202,
        z: 3201,
        level: 0,
    };
    assert!(!canlight_at(&bits, &c, origin));
    assert!(!canlight_at(&bits, &c, extra));
    assert!(canlight_at(&bits, &c, beside));
    assert_eq!(
        c.flag(origin.x, origin.z, 0) & CollisionFlag::WALK_SCENERY as u32,
        0
    );
}

#[test]
fn link_below_places_the_active_loc_on_the_bridged_plane() {
    let fix = FixtureDir::new("bridge");
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
1 0 0: f2 u50
==== LOC ====
1 0 0: 40 10 0
";
    let locs = [LocType {
        id: 40,
        width: 1,
        length: 1,
        blockwalk: false,
        active: true,
        ..LocType::default()
    }];
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (c, bits) = bake_square(&fix.0, "m50_50.jm2", text, &locs, &[zone]);
    let ground = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let upper = WorldTile {
        x: 3200,
        z: 3200,
        level: 1,
    };
    assert!(!canlight_at(&bits, &c, ground));
    assert!(canlight_at(&bits, &c, upper));
}

#[test]
fn bbox_holes_are_unset_on_every_plane_including_unused_upper() {
    let fix = FixtureDir::new("holes");
    let open = "\
==== MAP ====
0 0 0: h1 o6 u48
==== LOC ====
";
    fs::write(fix.0.join("m50_50.jm2"), open).unwrap();
    fs::write(fix.0.join("m52_50.jm2"), open).unwrap();
    let collision = bake_from_maps(&fix.0, &defs(&[]), &HashSet::new()).unwrap();
    let flags = collision.flags.as_ref().unwrap();
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let bits = bake_canlight(&collision, flags, &fix.0, &defs(&[]), &[zone]).unwrap();
    // m51_50 sits in the bbox between m50_50 and m52_50.
    let hole = WorldTile {
        x: 3264,
        z: 3200,
        level: 0,
    };
    let hole_upper = WorldTile {
        x: 3264,
        z: 3200,
        level: 3,
    };
    let covered = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    assert_eq!(collision.origin.x, 3200);
    assert!(collision.width > 64, "bbox spans the missing square");
    assert!(!canlight_at(&bits, &collision, hole));
    assert!(!canlight_at(&bits, &collision, hole_upper));
    // Nav leaves an unused upper plane walkable at gaps; canlight must not.
    assert!(collision.walkable(WorldTile {
        x: 3264,
        z: 3200,
        level: 3
    }));
    assert!(canlight_at(&bits, &collision, covered));
    assert!(!canlight_at(
        &bits,
        &collision,
        WorldTile {
            x: 3199,
            z: 3200,
            level: 0
        }
    ));
}

#[test]
fn missing_loc_def_is_conservative_one_by_one_exclude() {
    let fix = FixtureDir::new("missing-def");
    let text = open_map("0 0 1: 99 10 0\n");
    let zone = BankZone {
        from: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
    };
    let (c, bits) = bake_square(&fix.0, "m50_50.jm2", &text, &[], &[zone]);
    assert!(!canlight_at(
        &bits,
        &c,
        WorldTile {
            x: 3200,
            z: 3201,
            level: 0
        }
    ));
    assert!(canlight_at(
        &bits,
        &c,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }
    ));
}

#[test]
fn same_sized_sidecar_with_new_bank_policy_has_a_new_binding() {
    let pack = [0xCDu8; 32];
    let old_policy = policy_digest(289, b"old-zones");
    let new_policy = policy_digest(289, b"new-zones");
    assert_eq!(old_policy.len(), new_policy.len());
    let old = header_binding(&pack, &old_policy);
    let new = header_binding(&pack, &new_policy);
    assert_ne!(old, new);
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let bits = [0u64; 1];
    let a = crate::pack::encode_canlight_sidecar(origin, 2, 1, &bits, &old);
    let b = crate::pack::encode_canlight_sidecar(origin, 2, 1, &bits, &new);
    assert_eq!(a.len(), b.len());
    let da = crate::pack::decode_canlight_sidecar(&a).unwrap();
    let expected_new = expected_header_binding(
        &crate::pack::sha256_hex(&pack),
        &crate::pack::sha256_hex(&new_policy),
    )
    .unwrap();
    assert_ne!(da.binding, expected_new);
}

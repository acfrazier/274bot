use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use super::{
    decode, decode_canlight_sidecar, decode_flags_sidecar, decode_grid, decode_reach_sidecar,
    derive_banks, encode, encode_canlight_sidecar, encode_flags_sidecar, encode_grid,
    encode_reach_sidecar, merge_squares, parse_door_config, parse_door_config_ids,
    parse_door_open_ids, parse_mapsquare_text, parse_passable_locs, sha256_hex, walkable_dots,
    BankAccess, BankStand, Mapsquare, FORMAT_ID, MAGIC, SQUARE, VERSION,
};
use crate::collision::{derive_walkable, pack_walk, walk_word_from_parts, WorldCollision};
use crate::grid::StepGrid;
use crate::pack::PackError;
use crate::tile::Tile;
use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

#[test]
fn format_id_names_the_current_wire() {
    assert_eq!(
        FORMAT_ID,
        format!("{}{VERSION}", std::str::from_utf8(MAGIC).unwrap())
    );
}

#[test]
fn pack_roundtrip_fixture_door() {
    let g = StepGrid::fixture_door_corridor();
    let bytes = encode_grid(&g);
    let h = decode_grid(&bytes).unwrap();
    assert!(h.walkable(Tile {
        x: 0,
        z: 0,
        level: 0
    }));
    assert_eq!(h.doors.len(), g.doors.len());
}

#[test]
fn pack_walk_roundtrips_step_ok_vs_u32_flags() {
    let mut flags = vec![0u32; 4 * 3 * 3];
    flags[1] = CollisionFlag::W_S as u32; // face only
    flags[3] = CollisionFlag::WALK_SCENERY as u32 | CollisionFlag::WR_GRND as u32;
    let (face, blocked) = pack_walk(&flags);
    assert_eq!(face.len(), flags.len());
    for (i, f) in flags.iter().enumerate() {
        let derived = derive_walkable(&[*f])[0];
        let blocked = (blocked[i >> 6] >> (i & 63)) & 1 != 0;
        assert_eq!(walk_word_from_parts(face[i], blocked), derived);
    }
}

#[test]
fn v8_pack_has_no_resident_flags() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let bytes = encode(&collision, &TransportGraph::default(), &[]);
    assert_eq!(bytes[4], VERSION);
    let (c, _, _) = decode(&bytes).unwrap();
    assert!(c.flags.is_none());
    assert_eq!(c.walk.len(), 16);
}

#[test]
fn v8_decode_rejects_v7_and_older() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let mut bytes = encode(&collision, &TransportGraph::default(), &[]);
    bytes[4] = 7;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(7))));
    bytes[4] = 6;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(6))));
    bytes[4] = 5;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(5))));
    bytes[4] = 4;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(4))));
    bytes[4] = 8;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(8))));
}

#[test]
fn flags_sidecar_roundtrips_origin_and_cells() {
    let flags = vec![1u32, 2, 3, 4];
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let bytes = encode_flags_sidecar(origin, 2, 2, &flags);
    assert_eq!(&bytes[..4], b"274F");
    let (o, w, h, out) = decode_flags_sidecar(&bytes).unwrap();
    assert_eq!(o, origin);
    assert_eq!((w, h), (2, 2));
    assert_eq!(out, flags);
}

/// Scalar Cursor `read_u32` reference used only to pin bulk decode
/// semantics against the pre-bulk path (not production decode).
fn decode_flags_sidecar_scalar_ref(
    bytes: &[u8],
) -> Result<(WorldTile, usize, usize, Vec<u32>), PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != super::MAGIC_FLAGS {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != super::VERSION_FLAGS {
        return Err(PackError::BadVersion(version[0]));
    }
    let origin = WorldTile {
        x: super::read_i32(&mut r)?,
        z: super::read_i32(&mut r)?,
        level: super::read_i32(&mut r)?,
    };
    let width = super::read_u32(&mut r)? as usize;
    let height = super::read_u32(&mut r)? as usize;
    if width == 0 || height == 0 || width > super::MAX_GRID || height > super::MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {} tile cap",
            super::MAX_GRID
        )));
    }
    let remaining = bytes.len().saturating_sub(r.position() as usize);
    if !remaining.is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    let mut flags = Vec::with_capacity(remaining / 4);
    for _ in 0..remaining / 4 {
        flags.push(super::read_u32(&mut r)?);
    }
    Ok((origin, width, height, flags))
}

fn flags_header(origin: WorldTile, width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"274F");
    bytes.push(1);
    for v in [origin.x, origin.z, origin.level] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes
}

#[test]
fn flags_sidecar_rejects_bad_magic_and_version() {
    assert!(matches!(
        decode_flags_sidecar(b"XXXX"),
        Err(PackError::BadMagic)
    ));
    let mut bad_ver = flags_header(
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        1,
        1,
    );
    bad_ver[4] = 9;
    assert!(matches!(
        decode_flags_sidecar(&bad_ver),
        Err(PackError::BadVersion(9))
    ));
}

#[test]
fn flags_sidecar_rejects_truncated_header_and_partial_payload() {
    let full = encode_flags_sidecar(
        WorldTile {
            x: 1,
            z: 2,
            level: 3,
        },
        1,
        1,
        &[0xA1B2C3D4],
    );
    // Cut inside the fixed header (before any payload words).
    assert!(matches!(
        decode_flags_sidecar(&full[..10]),
        Err(PackError::Truncated)
    ));
    // Complete header + one trailing byte (partial u32).
    let mut partial = flags_header(
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        1,
        1,
    );
    partial.push(0x11);
    assert!(matches!(
        decode_flags_sidecar(&partial),
        Err(PackError::Truncated)
    ));
    // Three of four payload bytes after a valid word-aligned start.
    let mut almost = full.clone();
    almost.pop();
    assert!(matches!(
        decode_flags_sidecar(&almost),
        Err(PackError::Truncated)
    ));
}

#[test]
fn flags_sidecar_accepts_empty_payload_and_endian_distinct_words() {
    let origin = WorldTile {
        x: -7,
        z: 99,
        level: 2,
    };
    let empty = encode_flags_sidecar(origin, 2, 2, &[]);
    let (o, w, h, out) = decode_flags_sidecar(&empty).unwrap();
    assert_eq!((o, w, h, out), (origin, 2, 2, vec![]));

    // Multi-byte values that differ under LE vs BE interpretation.
    let flags = vec![0x0000_00FFu32, 0x0102_0304, 0xAABB_CCDD, 0x8000_0001];
    let bytes = encode_flags_sidecar(origin, 2, 2, &flags);
    // Payload starts after 25-byte header; first word is FF 00 00 00 LE.
    assert_eq!(&bytes[25..29], &[0xFF, 0x00, 0x00, 0x00]);
    assert_eq!(&bytes[29..33], &[0x04, 0x03, 0x02, 0x01]);
    let (_, _, _, out) = decode_flags_sidecar(&bytes).unwrap();
    assert_eq!(out, flags);
    assert_eq!(
        decode_flags_sidecar_scalar_ref(&bytes).unwrap().3,
        out,
        "bulk path must match scalar Cursor reference"
    );
}

#[test]
fn flags_sidecar_bulk_matches_scalar_on_errors_and_roundtrip() {
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let flags: Vec<u32> = (0u32..64).map(|i| i.wrapping_mul(0x0100_0307)).collect();
    let good = encode_flags_sidecar(origin, 4, 4, &flags);
    assert_eq!(
        decode_flags_sidecar(&good).unwrap(),
        decode_flags_sidecar_scalar_ref(&good).unwrap()
    );

    for bad in [
        &b"XXXX"[..],
        &good[..3],
        &good[..24], // header short of height
    ] {
        let bulk = decode_flags_sidecar(bad);
        let scalar = decode_flags_sidecar_scalar_ref(bad);
        assert_eq!(format!("{bulk:?}"), format!("{scalar:?}"));
    }
    let mut partial = good.clone();
    partial.push(0x42); // breaks word alignment
    assert_eq!(
        format!("{:?}", decode_flags_sidecar(&partial)),
        format!("{:?}", decode_flags_sidecar_scalar_ref(&partial))
    );
}

#[test]
fn reach_sidecar_roundtrips_geometry_words_and_binding() {
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let bits = vec![0x0102_0304_0506_0708u64, 0x8000_0000_0000_0001];
    let binding = [0xABu8; 32];
    let bytes = encode_reach_sidecar(origin, 2, 2, &bits, &binding);
    assert_eq!(&bytes[..4], b"274R");
    let side = decode_reach_sidecar(&bytes).unwrap();
    assert_eq!(side.origin, origin);
    assert_eq!((side.width, side.height, side.word_count), (2, 2, 2));
    assert_eq!(side.binding, binding);
    assert_eq!(side.bits, bits);
    assert_eq!(sha256_hex(&binding).len(), 64);
}

#[test]
fn reach_sidecar_rejects_bad_magic_version_and_truncated_payload() {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let binding = [1u8; 32];
    let full = encode_reach_sidecar(origin, 2, 2, &[0u64], &binding);
    assert!(matches!(
        decode_reach_sidecar(b"XXXX"),
        Err(PackError::BadMagic)
    ));
    let mut bad_ver = full.clone();
    bad_ver[4] = 2;
    assert!(matches!(
        decode_reach_sidecar(&bad_ver),
        Err(PackError::BadVersion(2))
    ));
    assert!(matches!(
        decode_reach_sidecar(&full[..10]),
        Err(PackError::Truncated)
    ));
    let mut partial = full.clone();
    partial.pop();
    assert!(matches!(
        decode_reach_sidecar(&partial),
        Err(PackError::Truncated)
    ));
    let mut extra = full.clone();
    extra.push(0);
    assert!(matches!(
        decode_reach_sidecar(&extra),
        Err(PackError::Truncated)
    ));
}

#[test]
fn reach_sidecar_binding_is_independent_of_geometry() {
    let origin = WorldTile {
        x: 100,
        z: 200,
        level: 0,
    };
    let bits = vec![0xFFu64];
    let a = encode_reach_sidecar(origin, 4, 4, &bits, &[0x11u8; 32]);
    let b = encode_reach_sidecar(origin, 4, 4, &bits, &[0x22u8; 32]);
    let da = decode_reach_sidecar(&a).unwrap();
    let db = decode_reach_sidecar(&b).unwrap();
    assert_eq!(da.origin, db.origin);
    assert_eq!((da.width, da.height), (db.width, db.height));
    assert_eq!(da.bits, db.bits);
    assert_ne!(da.binding, db.binding);
}

#[test]
fn canlight_sidecar_roundtrips_and_rejects_bad_magic_version_truncation() {
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let bits = vec![0x0102_0304_0506_0708u64];
    let binding = [0xCDu8; 32];
    let bytes = encode_canlight_sidecar(origin, 2, 1, &bits, &binding);
    assert_eq!(&bytes[..4], b"274L");
    let side = decode_canlight_sidecar(&bytes).unwrap();
    assert_eq!(side.origin, origin);
    assert_eq!((side.width, side.height, side.word_count), (2, 1, 1));
    assert_eq!(side.binding, binding);
    assert_eq!(side.bits, bits);
    assert!(matches!(
        decode_canlight_sidecar(b"XXXX"),
        Err(PackError::BadMagic)
    ));
    let mut bad_ver = bytes.clone();
    bad_ver[4] = 2;
    assert!(matches!(
        decode_canlight_sidecar(&bad_ver),
        Err(PackError::BadVersion(2))
    ));
    assert!(matches!(
        decode_canlight_sidecar(&bytes[..10]),
        Err(PackError::Truncated)
    ));
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(matches!(
        decode_canlight_sidecar(&extra),
        Err(PackError::Truncated)
    ));
    let reach = encode_reach_sidecar(origin, 2, 1, &bits, &binding);
    assert!(matches!(
        decode_canlight_sidecar(&reach),
        Err(PackError::BadMagic)
    ));
}

#[test]
fn roundtrip_collision_and_transport_graph() {
    let plane = vec![0, 0, 1, 0, 0, 0];
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    // Distinct upper-plane content pins the four-plane wire layout.
    flags[plane.len()..2 * plane.len()].copy_from_slice(&[7; 6]);
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 3,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    let door = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        loc_id: 1530,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::N),
        open_loc_id: Some(1531),
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![772], // dramen_staff on the Zanaris shed door
        members_req: false,
        wildy_cap: None,
    };
    let ladder = TransportEdge {
        kind: TransportKind::Ladder,
        at: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3201,
            z: 3201,
            level: 1,
        },
        loc_id: 1747,
        option: 1,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(16, 5)],
        item_req: vec![(995, 10)],
        quest_req: vec!["Restless Ghost".into()],
        varp_req: vec![(4, 1)],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let di = graph.edges.len();
    graph.edges.push(door);
    let li = graph.edges.len();
    graph.edges.push(ladder);
    let glider = TransportEdge {
        kind: TransportKind::Glider,
        at: WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
        to: WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        },
        loc_id: 170,
        option: 1,
        ticks: 4,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![(150, 160)],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let gi = graph.edges.len();
    graph.edges.push(glider);
    // A spirit-tree edge (kind 7) and the reserved NPC kind (8) ride
    // the same wire byte without a version bump.
    let spirit = TransportEdge {
        kind: TransportKind::SpiritTree,
        at: WorldTile {
            x: 2460,
            z: 3445,
            level: 0,
        },
        to: WorldTile {
            x: 2542,
            z: 3169,
            level: 0,
        },
        loc_id: 1293,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![(150, 160)],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let si = graph.edges.len();
    graph.edges.push(spirit);
    let npc = TransportEdge {
        kind: TransportKind::Npc,
        at: WorldTile {
            x: 2500,
            z: 3500,
            level: 0,
        },
        to: WorldTile {
            x: 2600,
            z: 3400,
            level: 0,
        },
        loc_id: 1,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let ni = graph.edges.len();
    graph.edges.push(npc);
    // The any-tile teleport layer (Varrock spell): stored as a kind-4
    // edge in the same array, split back out on decode.
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
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
        members_req: false,
        wildy_cap: None,
    });
    graph.at.entry(graph.edges[di].at).or_default().push(di);
    graph.at.entry(graph.edges[li].at).or_default().push(li);
    graph.at.entry(graph.edges[gi].at).or_default().push(gi);
    graph.at.entry(graph.edges[si].at).or_default().push(si);
    graph.at.entry(graph.edges[ni].at).or_default().push(ni);

    let bytes = encode(&collision, &graph, &[]);
    let (c, g, _) = decode(&bytes).unwrap();
    assert_eq!(c.origin, collision.origin);
    assert_eq!(c.width, collision.width);
    assert_eq!(c.height, collision.height);
    assert_eq!(c.walk, collision.walk);
    assert!(c.flags.is_none());
    assert_eq!(g.edges, graph.edges);
    // The door edge's new fields round-trip on the wire.
    assert_eq!(g.edges[di].dir, Some(DoorDir::N));
    assert_eq!(g.edges[di].open_loc_id, Some(1531));
    assert_eq!(g.edges[di].worn_req, vec![772]);
    // The new kinds round-trip on the v4 wire (7 spirit tree, 8 NPC).
    assert_eq!(g.edges[si].kind, TransportKind::SpiritTree);
    assert_eq!(g.edges[si].varp_req, vec![(150, 160)]);
    assert_eq!(g.edges[ni].kind, TransportKind::Npc);
    // Teleports round-trip in their own layer, and the at-index is
    // rebuilt from the ordinary edges only.
    assert_eq!(g.teleports, graph.teleports);
    assert_eq!(g.at, graph.at);
    assert!(!g.at.contains_key(&WorldTile {
        x: 0,
        z: 0,
        level: 0
    }));
    // The two formats do not cross-decode: the grid rejects pack magic
    // and vice versa.
    assert!(matches!(decode_grid(&bytes), Err(PackError::BadMagic)));
    assert!(matches!(
        decode(&encode_grid(&StepGrid::fixture_open_3x3())),
        Err(PackError::BadMagic)
    ));
}

#[test]
fn decode_rejects_old_version_streams() {
    // A version-2 or version-3 stream (pre-four-plane wire) is
    // rejected, not mis-read: the re-bake immediately rewrites it at
    // the current version. Versions 4, 5, and 6 are rejected too (see
    // `v8_decode_rejects_v7_and_older`).
    let plane = vec![0, 0, 1, 0, 0, 0];
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 3,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let graph = TransportGraph::default();
    let mut bytes = encode(&collision, &graph, &[]);
    // The version byte sits right after the 4-byte magic.
    bytes[4] = 3;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(3))));
    bytes[4] = 2;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(2))));
}

#[test]
fn v8_roundtrips_worn_req() {
    // v8 carries the fifth per-edge req list (the worn-item ids) on
    // the packed-walk wire; no pre-v8 stream decodes.
    let plane = vec![0, 0, 1, 0, 0, 0];
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 3,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let door = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        loc_id: 2406,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::N),
        open_loc_id: Some(1532),
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec!["Lost City".into()],
        varp_req: vec![],
        worn_req: vec![772],
        members_req: false,
        wildy_cap: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(door.clone());
    graph.at.entry(door.at).or_default().push(0);
    let bytes = encode(&collision, &graph, &[]);
    // The version byte sits right after the 4-byte magic: v8 now.
    assert_eq!(bytes[4], VERSION);
    let (c, g, _) = decode(&bytes).unwrap();
    assert_eq!(g.edges, graph.edges);
    assert_eq!(g.edges[0].worn_req, vec![772]);
    assert_eq!(g.at, graph.at);
    assert_eq!(c.walk, collision.walk);
    assert!(c.flags.is_none());
}

#[test]
fn v9_roundtrips_members_req_true_and_false() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let edge = |members_req| TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 1,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 2,
            z: 0,
            level: 0,
        },
        loc_id: 1596,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::E),
        open_loc_id: Some(1560),
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req,
        wildy_cap: None,
    };
    for members_req in [true, false] {
        let mut graph = TransportGraph::default();
        graph.edges.push(edge(members_req));
        let bytes = encode(&collision, &graph, &[]);
        assert_eq!(bytes[4], VERSION);
        assert_eq!(FORMAT_ID, "274V10");
        let (_, g, _) = decode(&bytes).unwrap();
        assert_eq!(g.edges[0].members_req, members_req);
    }
}

#[test]
fn v10_decode_rejects_older_version_bytes() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let mut bytes = encode(&collision, &TransportGraph::default(), &[]);
    bytes[4] = 8;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(8))));
    bytes[4] = 9;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(9))));
}

#[test]
fn v9_decode_rejects_invalid_members_req_flag() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let door = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 1,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 2,
            z: 0,
            level: 0,
        },
        loc_id: 1596,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::E),
        open_loc_id: Some(1560),
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(door);
    let mut bytes = encode(&collision, &graph, &[]);
    // members_req sits just before wildy_cap (i32), the bank-stand count
    // (u32), and the wilderness trailer (zone count + divisor + offset).
    let flag_at = bytes.len() - 12 - 4 - 4 - 1;
    bytes[flag_at] = 2;
    assert!(matches!(decode(&bytes), Err(PackError::BadLength(_))));
}

#[test]
fn decode_grid_rejects_bad_magic() {
    assert!(matches!(decode_grid(b"XXXX"), Err(PackError::BadMagic)));
}

#[test]
fn decode_grid_rejects_truncated_pack() {
    let bytes = encode_grid(&StepGrid::fixture_door_corridor());
    assert!(matches!(
        decode_grid(&bytes[..bytes.len() - 1]),
        Err(PackError::Truncated)
    ));
}

#[test]
fn decode_grid_rejects_oversized_grid() {
    // Huge width would try to allocate GiB of walk bytes.
    let bytes = header(0, u32::MAX, 1);
    assert!(matches!(decode_grid(&bytes), Err(PackError::BadLength(_))));
}

#[test]
fn decode_grid_rejects_zero_grid() {
    assert!(matches!(
        decode_grid(&header(0, 0, 1)),
        Err(PackError::BadLength(_))
    ));
    assert!(matches!(
        decode_grid(&header(0, 1, 0)),
        Err(PackError::BadLength(_))
    ));
}

/// Magic + version + zero origin + width/height, nothing else.
fn header(level: i32, width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"274N");
    bytes.push(1);
    for v in [0i32, 0, level] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes
}

#[test]
fn walkable_dots_lists_only_walkable_tiles() {
    let g = StepGrid::fixture_door_corridor();
    let dots: Vec<Tile> = walkable_dots(&g, 0).collect();
    assert_eq!(dots.len(), 4);
    assert!(!dots.contains(&Tile {
        x: 2,
        z: 0,
        level: 0
    }));
    assert_eq!(walkable_dots(&g, 1).count(), 0);
}

#[test]
fn parse_door_config_collects_openable_doors() {
    // Mirrors the real config format: closed/open counterpart blocks.
    let text = "\
[loc_1512]
name=Large door
op1=Open
category=door_closed

[loc_1513]
op1=Open

[loc_1514]
op1=Close
category=door_opened

[loc_1530]
op1=Open
category=door_closed

[membergatel]
name=Gate
op1=Open
";
    let ids = parse_door_config(text);
    assert!(ids.contains(&1512));
    assert!(ids.contains(&1513));
    assert!(ids.contains(&1530));
    assert!(!ids.contains(&1514));
    assert!(!ids.contains(&1531));
}

#[test]
fn parse_door_config_ids_resolves_named_blocks() {
    // Mirrors `scripts/areas/area_alkharid/configs/border_gate.loc`:
    // name-keyed blocks that `parse_door_config` (numeric-only) skips.
    let text = "\
[border_gate_toll_left]
name=Gate
op1=Open
category=border_gate_toll_left
param=next_loc_stage,loc_1562

[border_gate_toll_right]
name=Gate
op1=Open
category=border_gate_toll_right
param=next_loc_stage,loc_1563
";
    let mut ids = HashMap::new();
    ids.insert("border_gate_toll_left".to_string(), 2882);
    ids.insert("border_gate_toll_right".to_string(), 2883);
    ids.insert("loc_1562".to_string(), 1562);
    ids.insert("loc_1563".to_string(), 1563);
    let doors = parse_door_config_ids(text, &ids);
    assert!(doors.contains(&2882));
    assert!(doors.contains(&2883));
    // The numeric-only view still ignores the name-keyed blocks.
    assert!(!parse_door_config(text).contains(&2882));
    // The open-leaf params resolve under the named blocks too.
    let open = parse_door_open_ids(text, &ids);
    assert_eq!(open.get(&2882), Some(&1562));
    assert_eq!(open.get(&2883), Some(&1563));
}

#[test]
fn parse_door_config_collects_gate_closed_categories() {
    // Mirrors gates.loc: closed/open counterpart blocks carrying the
    // fence-gate categories. `gate_main_closed` / `gate_outer_closed`
    // are openable like `door_closed`; the `*_open` counterpart states
    // (`op1=Close`) are not.
    let text = "\
[loc_1551]
name=Gate
op1=Open
category=gate_main_closed

[loc_1552]
op1=Close
category=gate_main_open

[loc_1553]
op1=Open
category=gate_outer_closed
";
    let ids = parse_door_config(text);
    assert!(ids.contains(&1551));
    assert!(!ids.contains(&1552));
    assert!(ids.contains(&1553));
}

#[test]
fn parse_door_open_ids_reads_next_loc_stage() {
    let text = "\
[loc_1530]
name=Door
op1=Open
category=door_closed
param=next_loc_stage,loc_1531

[loc_1512]
op1=Open
param=next_loc_stage,loc_1513

[loc_1514]
op1=Open
param=next_loc_stage,elenagateopen

[membergatel]
name=Gate
op1=Open
";
    // Numeric `loc_N` values parse directly; the name-valued one
    // resolves through the loc id map.
    let ids = {
        let mut m = std::collections::HashMap::new();
        m.insert("elenagateopen".to_string(), 1535);
        m
    };
    let open = parse_door_open_ids(text, &ids);
    assert_eq!(open.get(&1530), Some(&1531));
    assert_eq!(open.get(&1512), Some(&1513));
    assert_eq!(open.get(&1514), Some(&1535));
    // Non-numeric headers and unknown names carry nothing.
    assert_eq!(open.get(&1534), None);
}

/// A 64×64 level-0 collision at the given mapsquare with the given
/// local `(x, z, flag)` stamps, everything else walkable — the door
/// snap's view of the world (mirrors the flags `bake_from_maps` stamps
/// for the fixture's MAP/LOC lines; `V_*` range flags are omitted since
/// they are not in the walk mask).
fn square_collision(mx: i32, mz: i32, extras: &[(usize, usize, u32)]) -> WorldCollision {
    let mut flags = vec![0u32; SQUARE * SQUARE];
    for &(x, z, f) in extras {
        flags[z * SQUARE + x] |= f;
    }
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: WorldTile {
            x: mx * SQUARE as i32,
            z: mz * SQUARE as i32,
            level: 0,
        },
        width: SQUARE,
        height: SQUARE,
        walk,
        blocked,
        flags: None,
    }
}

#[test]
fn parse_jm2_text_pins_catherby_door() {
    let door_ids = parse_door_config(
        "[loc_1530]\nop1=Open\ncategory=door_closed\n[loc_980]\nop1=Close\ncategory=door_opened\n",
    );
    let text = "\
==== MAP ====
0 0 0: h1 o6 u48
0 1 0: f1 u48
0 0 1: f16 u50
0 0 46: h1 o6 f1 u50
==== LOC ====
0 0 46: 1530 0 1
0 1 46: 980 0 0
1 0 46: 1530 0 1
0 0 47: 1530 0 7
==== NPC ====
0 0 0: 1234
";
    // The bake for this text: (0,46) carries MAP f1 (WR_GRND) plus the
    // door's W_N and wall 980's W_E; (0,47) the door's W_S face stamp.
    let collision = square_collision(
        44,
        53,
        &[
            (1, 0, CollisionFlag::WR_GRND as u32),
            (
                0,
                46,
                CollisionFlag::WR_GRND as u32
                    | CollisionFlag::W_N as u32
                    | CollisionFlag::W_E as u32,
            ),
            (1, 46, CollisionFlag::W_W as u32),
            (0, 47, CollisionFlag::W_S as u32),
        ],
    );
    let sq = parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new(), &collision).unwrap();
    // (0,0): no f flag -> walkable; (1,0): f1 bit 0 -> blocked;
    // (0,1): f16 bit 0 clear -> walkable; (2,0): no MAP line -> blocked.
    assert_eq!(sq.walk[0], 1);
    assert_eq!(sq.walk[1], 0);
    assert_eq!(sq.walk[64], 1);
    assert_eq!(sq.walk[2], 0);
    // The Catherby closed door: 1530 @ local (0,46) -> 2816,3438,0,
    // both from→to and to→from (same loc, loc_id). The north side
    // snaps past (0,47) — the door's own blocked south-face stamp — to
    // the next walkable tile.
    assert_eq!(sq.doors.len(), 2);
    let south = Tile {
        x: 2816,
        z: 3437,
        level: 0,
    };
    let north = Tile {
        x: 2816,
        z: 3440,
        level: 0,
    };
    let loc = Tile {
        x: 2816,
        z: 3438,
        level: 0,
    };
    let fwd = sq
        .doors
        .iter()
        .find(|d| d.from == south && d.to == north)
        .expect("Catherby south→north door");
    let rev = sq
        .doors
        .iter()
        .find(|d| d.from == north && d.to == south)
        .expect("Catherby reverse neighbour");
    assert_eq!(fwd.loc, loc);
    assert_eq!(fwd.loc_id, 1530);
    assert_eq!(rev.loc, loc);
    assert_eq!(rev.loc_id, 1530);
    // The door tile is a wall: not walkable.
    assert_eq!(sq.walk[46 * 64], 0);
}

#[test]
fn parse_passable_locs_blockwalk_no_and_open_door() {
    let text = "\
[loc_980]
name=Fence
[loc_1124]
blockwalk=no
[loc_1531]
op1=Close
category=door_opened
[loc_1259]
blockwalk=yes
";
    let ids = parse_passable_locs(text);
    assert!(ids.contains(&1124));
    assert!(ids.contains(&1531));
    assert!(!ids.contains(&980));
    assert!(!ids.contains(&1259));
}

#[test]
fn parse_jm2_blocking_loc_marks_tile_unwalkable() {
    // loc 980 (fencing) is unknown / default-block; local (0,45) of
    // mapsquare 44,53 is absolute 2816,3437.
    let text = "\
==== MAP ====
0 0 45: h1 o6 u50
==== LOC ====
0 0 45: 980 0 0
";
    let collision = square_collision(44, 53, &[]);
    let sq =
        parse_mapsquare_text(text, 44, 53, &HashSet::new(), &HashSet::new(), &collision).unwrap();
    let grid = merge_squares(&[sq]);
    assert!(!grid.walkable(Tile {
        x: 2816,
        z: 3437,
        level: 0
    }));
}

#[test]
fn parse_jm2_snaps_door_from_to_past_a_wall_loc() {
    let door_ids = parse_door_config("[loc_1530]\nop1=Open\ncategory=door_closed\n");
    let text = "\
==== MAP ====
0 0 45: h1 o6 u50
0 0 46: h1 o6 u50
0 0 47: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
0 0 45: 980 0 0
";
    // Wall 980 right outside the door's south side: the door from/to
    // snap past it to (2816,3436)/(2816,3440) instead of landing on
    // the wall tile, which stays blocked by the loc.
    let collision = square_collision(
        44,
        53,
        &[
            (0, 45, CollisionFlag::W_W as u32),
            (0, 46, CollisionFlag::W_N as u32),
            (0, 47, CollisionFlag::W_S as u32),
        ],
    );
    let sq = parse_mapsquare_text(text, 44, 53, &door_ids, &HashSet::new(), &collision).unwrap();
    assert_eq!(sq.doors.len(), 2);
    let south = Tile {
        x: 2816,
        z: 3436,
        level: 0,
    };
    let north = Tile {
        x: 2816,
        z: 3440,
        level: 0,
    };
    assert!(sq.doors.iter().any(|d| d.from == south && d.to == north));
    assert!(sq.doors.iter().any(|d| d.from == north && d.to == south));
    let grid = merge_squares(&[sq]);
    assert!(!grid.walkable(Tile {
        x: 2816,
        z: 3438,
        level: 0
    }));
    assert!(!grid.walkable(Tile {
        x: 2816,
        z: 3437,
        level: 0
    }));
    assert!(grid.walkable(Tile {
        x: 2816,
        z: 3439,
        level: 0
    }));
}

#[test]
fn merge_squares_builds_one_bbox_level0_grid() {
    let a = one_tile_square(50, 50);
    let b = one_tile_square(52, 52);
    let grid = merge_squares(&[a, b]);
    assert_eq!(grid.doors.len(), 0);
    assert!(grid.walkable(Tile {
        x: 3200,
        z: 3200,
        level: 0
    }));
    assert!(grid.walkable(Tile {
        x: 3328,
        z: 3328,
        level: 0
    }));
    // The square gap between the two squares stays blocked.
    assert!(!grid.walkable(Tile {
        x: 3264,
        z: 3264,
        level: 0
    }));
    assert!(!grid.walkable(Tile {
        x: 2816,
        z: 3200,
        level: 0
    }));
}

/// A 64×64 square with only its local (0,0) tile walkable.
fn one_tile_square(x: i32, z: i32) -> Mapsquare {
    let mut walk = vec![0u8; 64 * 64];
    walk[0] = 1;
    Mapsquare {
        x,
        z,
        walk,
        doors: vec![],
    }
}

#[test]
fn v8_roundtrips_bank_stands() {
    // The v8 wire carries the bank stand table after the transport
    // edges: a booth round-trips its name, tile, and the Use-quickly
    // op; the NPC variant round-trips its npc name, op, and the
    // optional dialog choice.
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let banks = vec![
        BankStand {
            name: "Bank booth".into(),
            tile: WorldTile {
                x: 3205,
                z: 3441,
                level: 0,
            },
            access: BankAccess::Booth { op: 2 },
        },
        BankStand {
            name: "Banker".into(),
            tile: WorldTile {
                x: 2810,
                z: 3445,
                level: 0,
            },
            access: BankAccess::Npc {
                name: "shilobanker".into(),
                op: 3,
                choose: Some("I'd like to access my bank account, please.".into()),
            },
        },
    ];
    let bytes = encode(&collision, &TransportGraph::default(), &banks);
    assert_eq!(bytes[4], VERSION);
    let (c, g, out) = decode(&bytes).unwrap();
    assert_eq!(out, banks);
    assert_eq!(c.walk, collision.walk);
    assert!(g.edges.is_empty());
}

#[test]
fn bake_emits_bankbooth_use_quickly_only() {
    // The bake derives booth stands from the same content loc pass as
    // the collision bake: `bankbooth` placements join the table with
    // the Use-quickly op (2); the closed booth (`bankboothclosed`) and
    // the tutorial `newbiebankbooth` never do.
    let dir = std::env::temp_dir().join(format!("274bot-nav-banks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["", "pack", "scripts/interface_bank/configs", "maps"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(
        dir.join("pack/loc.pack"),
        "2213=bankbooth\n2215=bankboothclosed\n3045=newbiebankbooth\n",
    )
    .unwrap();
    std::fs::write(
            dir.join("scripts/interface_bank/configs/bank_booth.loc"),
            "[bankbooth]\nname=Bank booth\nop2=Use-quickly\n\n[bankboothclosed]\nname=Closed bank booth\n",
        )
        .unwrap();
    std::fs::write(
            dir.join("maps/m50_50.jm2"),
            "==== MAP ====\n0 0 0: h1 o6 u48\n==== LOC ====\n0 12 32: 2213 10 1\n0 13 32: 2215 10 1\n0 14 32: 3045 10 1\n",
        )
        .unwrap();
    let banks = derive_banks(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        banks.len(),
        1,
        "only the bankbooth placement becomes a stand"
    );
    assert_eq!(banks[0].name, "Bank booth");
    assert_eq!(
        banks[0].tile,
        WorldTile {
            x: 50 * 64 + 12,
            z: 50 * 64 + 32,
            level: 0,
        }
    );
    assert_eq!(banks[0].access, BankAccess::Booth { op: 2 });
}

#[test]
fn v10_roundtrips_wilderness_rules_and_wildy_cap() {
    let flags = vec![0u32; 4 * 2 * 2];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 2,
        height: 2,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph {
        wilderness: crate::transport::WildernessRules {
            zones: vec![crate::transport::WildernessZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 6399,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            }],
            divisor: 8,
            offset: 1,
        },
        ..TransportGraph::default()
    };
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
        loc_id: 0,
        option: 0,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(6, 25)],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: Some(20),
    });
    let bytes = encode(&collision, &graph, &[]);
    assert_eq!(bytes[4], VERSION);
    assert_eq!(FORMAT_ID, "274V10");
    let (_, g, _) = decode(&bytes).unwrap();
    assert_eq!(g.wilderness, graph.wilderness);
    assert_eq!(g.teleports[0].wildy_cap, Some(20));
}

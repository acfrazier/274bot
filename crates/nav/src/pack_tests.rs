use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Cursor, Read};

use super::{
    decode, decode_canlight_sidecar, decode_flags_sidecar, decode_grid, decode_reach_sidecar,
    derive_banks, encode, encode_canlight_sidecar, encode_flags_sidecar, encode_grid,
    encode_reach_sidecar, merge_squares, parse_door_config, parse_door_config_ids,
    parse_door_open_ids, parse_mapsquare_text, parse_passable_locs, read_canlight_sidecar,
    read_flags_sidecar, read_reach_sidecar, sha256_hex, walkable_dots, BankAccess, BankStand,
    Mapsquare, FORMAT_ID, MAGIC, SQUARE, VERSION,
};
use crate::collision::{derive_walkable, pack_walk, walk_word_from_parts, WorldCollision};
use crate::grid::StepGrid;
use crate::pack::PackError;
use crate::quest_gates::tests::{
    family as gate_family, family_schema as gate_family_schema, window as gate_window,
};
use crate::quest_gates::QuestGates;
use crate::router::AvoidRect;
use crate::tile::Tile;
use crate::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind, WildernessRules};
use crate::world::NavWorld;
use crate::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable};
use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

/// A BufRead that yields at most one byte per fill, exercising all parser
/// fields across real short reads rather than a single in-memory read call.
struct ShortReadBuf<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ShortReadBuf<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
}

impl Read for ShortReadBuf<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let available = BufRead::fill_buf(self)?;
        let n = buf.len().min(available.len());
        buf[..n].copy_from_slice(&available[..n]);
        BufRead::consume(self, n);
        Ok(n)
    }
}

impl BufRead for ShortReadBuf<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let end = self.position.saturating_add(1).min(self.bytes.len());
        Ok(&self.bytes[self.position..end])
    }

    fn consume(&mut self, amount: usize) {
        self.position = self.position.saturating_add(amount).min(self.bytes.len());
    }
}

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
fn read_flags_sidecar_streams_words_and_digest() {
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    // Exercise the streamed word and digest results on a substantial payload.
    let words: Vec<u32> = (0..256 * 1024).map(|i| i as u32).collect();
    let bytes = encode_flags_sidecar(origin, 512, 512, &words);
    let path = std::env::temp_dir().join(format!(
        "274bot-flags-stream-{}.navflags",
        std::process::id()
    ));
    std::fs::write(&path, &bytes).unwrap();
    let expected = crate::manifest::hash_bytes(&bytes);

    let loaded = read_flags_sidecar(&path, true).expect("stream load");
    assert_eq!(loaded.origin, origin);
    assert_eq!((loaded.width, loaded.height), (512, 512));
    assert_eq!(loaded.flags.as_slice(), words.as_slice());
    assert_eq!(loaded.content_sha256.as_deref(), Some(expected.as_str()));
    let (_, _, _, decoded) = decode_flags_sidecar(&bytes).unwrap();
    assert_eq!(decoded.as_slice(), loaded.flags.as_slice());
    let trusted = read_flags_sidecar(&path, false).expect("no-hash load");
    let _ = std::fs::remove_file(&path);
    assert_eq!(trusted.flags.as_slice(), words.as_slice());
    assert!(trusted.content_sha256.is_none());
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
fn read_reach_and_canlight_sidecars_stream_into_arc() {
    let origin = WorldTile {
        x: -123,
        z: 456,
        level: 2,
    };
    let bits = [
        0x0102_0304_0506_0708,
        0x8000_0000_0000_0001,
        0xAABB_CCDD_EEFF_1020,
    ];
    let binding = [0x5Au8; 32];
    let reach_bytes = encode_reach_sidecar(origin, 7, 9, &bits, &binding);
    let mut reach_reader = ShortReadBuf::new(&reach_bytes);
    let reach = read_reach_sidecar(&mut reach_reader, reach_bytes.len()).unwrap();
    assert_eq!(reach.origin, origin);
    assert_eq!((reach.width, reach.height, reach.word_count), (7, 9, 3));
    assert_eq!(reach.binding, binding);
    assert_eq!(reach.bits.as_ref(), bits.as_slice());
    assert_eq!(reach_reader.position, reach_bytes.len());

    let canlight_bytes = encode_canlight_sidecar(origin, 7, 9, &bits, &binding);
    let mut canlight_reader = ShortReadBuf::new(&canlight_bytes);
    let canlight = read_canlight_sidecar(&mut canlight_reader, canlight_bytes.len()).unwrap();
    assert_eq!(canlight.origin, origin);
    assert_eq!(
        (canlight.width, canlight.height, canlight.word_count),
        (7, 9, 3)
    );
    assert_eq!(canlight.binding, binding);
    assert_eq!(canlight.bits.as_ref(), bits.as_slice());

    let empty_bytes = encode_reach_sidecar(origin, 1, 1, &[], &binding);
    let mut empty_reader = ShortReadBuf::new(&empty_bytes);
    let empty = read_reach_sidecar(&mut empty_reader, empty_bytes.len()).unwrap();
    assert!(empty.bits.is_empty());
}

#[test]
fn read_sidecars_reject_malformed_lengths_and_partial_arc_initialization() {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let binding = [0xA5u8; 32];
    let words: Vec<u64> = (0..515)
        .map(|word| word as u64 ^ 0xA5A5_A5A5_A5A5_A5A5)
        .collect();
    let bytes = encode_reach_sidecar(origin, 2, 2, &words, &binding);

    let mut bad_magic_reader = ShortReadBuf::new(b"XXXX");
    assert!(matches!(
        read_reach_sidecar(&mut bad_magic_reader, 4),
        Err(PackError::BadMagic)
    ));
    let mut stale = bytes.clone();
    stale[4] = 2;
    let mut stale_reader = ShortReadBuf::new(&stale);
    assert!(matches!(
        read_reach_sidecar(&mut stale_reader, stale.len()),
        Err(PackError::BadVersion(2))
    ));
    let mut bad_geometry = bytes.clone();
    bad_geometry[17..21].copy_from_slice(&0u32.to_le_bytes());
    let mut geometry_reader = ShortReadBuf::new(&bad_geometry);
    assert!(matches!(
        read_reach_sidecar(&mut geometry_reader, bad_geometry.len()),
        Err(PackError::BadLength(_))
    ));

    let mut wrong_count = bytes.clone();
    wrong_count[25..29].copy_from_slice(&1u32.to_le_bytes());
    let mut count_reader = ShortReadBuf::new(&wrong_count);
    assert!(matches!(
        read_reach_sidecar(&mut count_reader, wrong_count.len()),
        Err(PackError::Truncated)
    ));
    let mut extra = bytes.clone();
    extra.push(0);
    let mut extra_reader = ShortReadBuf::new(&extra);
    assert!(matches!(
        read_reach_sidecar(&mut extra_reader, extra.len()),
        Err(PackError::Truncated)
    ));

    // The advertised length is the complete file, but the physical stream
    // ends before the payload finishes. The later cuts occur after one full
    // 4 KiB batch, exercising a failed drop with both initialized and
    // uninitialized Arc slots.
    const HEADER_LEN: usize = 4 + 1 + 12 + 8 + 4 + 32;
    for cut in [
        0,
        3,
        4,
        HEADER_LEN - 1,
        HEADER_LEN,
        HEADER_LEN + 1,
        HEADER_LEN + 7,
        HEADER_LEN + 4095,
        HEADER_LEN + 4096,
        HEADER_LEN + 4097,
        bytes.len() - 1,
    ] {
        let mut truncated_reader = ShortReadBuf::new(&bytes[..cut]);
        assert!(
            matches!(
                read_reach_sidecar(&mut truncated_reader, bytes.len()),
                Err(PackError::Truncated)
            ),
            "truncated at {cut} bytes"
        );
    }

    let canlight = encode_canlight_sidecar(origin, 2, 2, &[1], &binding);
    let mut bad_canlight = canlight.clone();
    bad_canlight[4] = 4;
    let mut bad_canlight_reader = ShortReadBuf::new(&bad_canlight);
    assert!(matches!(
        read_canlight_sidecar(&mut bad_canlight_reader, bad_canlight.len()),
        Err(PackError::BadVersion(4))
    ));
    let mut bad_canlight_geometry = canlight.clone();
    bad_canlight_geometry[17..21].copy_from_slice(&0u32.to_le_bytes());
    let mut bad_canlight_geometry_reader = ShortReadBuf::new(&bad_canlight_geometry);
    assert!(matches!(
        read_canlight_sidecar(
            &mut bad_canlight_geometry_reader,
            bad_canlight_geometry.len()
        ),
        Err(PackError::BadLength(_))
    ));
    let mut bad_canlight_magic_reader = ShortReadBuf::new(&bytes);
    assert!(matches!(
        read_canlight_sidecar(&mut bad_canlight_magic_reader, bytes.len()),
        Err(PackError::BadMagic)
    ));
    let mut truncated_canlight_reader = ShortReadBuf::new(&canlight[..10]);
    assert!(matches!(
        read_canlight_sidecar(&mut truncated_canlight_reader, 10),
        Err(PackError::Truncated)
    ));
    let mut extra_canlight = canlight.clone();
    extra_canlight.push(0);
    let mut extra_canlight_reader = ShortReadBuf::new(&extra_canlight);
    assert!(matches!(
        read_canlight_sidecar(&mut extra_canlight_reader, extra_canlight.len()),
        Err(PackError::Truncated)
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
        members_req: true,
        wildy_cap: None,
        quest_gates: None,
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
        quest_gates: None,
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
        quest_gates: None,
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
        quest_gates: None,
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
        quest_gates: None,
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
        quest_gates: None,
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

fn zone_table_pack_fixture() -> (Vec<u8>, ZoneTable) {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let mut zones = vec![
        Zone::npc(
            WorldTile {
                x: 1,
                z: 1,
                level: 0,
            },
            1,
            ZoneClass::LevelRule,
            10,
            0,
        ),
        Zone::shaped_npc(
            WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            1,
            1,
            ZoneClass::Always,
            u16::MAX,
            0,
            0,
        ),
        Zone::hazard(
            AvoidRect {
                min_x: 3,
                min_z: 3,
                max_x: 3,
                max_z: 3,
                level: Some(0),
            },
            0,
            1,
        ),
    ];
    zones[0].group = 0;
    let wilderness = WildernessRules::default();
    let table = ZoneTable::from_parts(
        zones,
        vec![
            ZoneKind::new("ranger", "Ranger", 123, 5, true, false),
            ZoneKind::new("lava-bridge", "Lava bridge", -1, 0, false, false),
        ],
        vec![ZoneGroup::new(
            "wolf-pack",
            "Wolf pack",
            AvoidRect {
                min_x: 0,
                min_z: 0,
                max_x: 2,
                max_z: 2,
                level: Some(0),
            },
            vec![0].into_boxed_slice(),
        )],
        vec![(
            0,
            AvoidRect {
                min_x: 0,
                min_z: 0,
                max_x: 0,
                max_z: 0,
                level: Some(0),
            },
        )],
        vec![0b0011_1000],
        origin,
        4,
        4,
        &wilderness,
    )
    .unwrap();
    let (walk, blocked) = pack_walk(&[0u32; 4 * 4 * 4]);
    let collision = WorldCollision {
        origin,
        width: 4,
        height: 4,
        walk,
        blocked,
        flags: None,
    };
    let graph = TransportGraph {
        zones: Some(table.clone()),
        ..Default::default()
    };
    (encode(&collision, &graph, &[]), table)
}

#[test]
fn v12_pack_keeps_an_empty_zone_table_present() {
    let bytes = encode(&tiny_collision(), &TransportGraph::default(), &[]);
    let (_, graph, _) = decode(&bytes).unwrap();
    let table = graph.zones.as_ref().expect("v12 declares a zone table");
    assert!(table.zones().is_empty());
    assert!(table.kinds().is_empty());
}

#[test]
fn v12_zone_table_roundtrips_rows_and_rebuilds_its_index() {
    let (bytes, table) = zone_table_pack_fixture();
    assert_eq!(bytes[4], VERSION);
    assert_eq!(super::zones::wire_size(Some(&table)), 211);
    let (_, decoded, _) = decode(&bytes).unwrap();
    let decoded_table = decoded.zones.as_ref().unwrap();
    assert_eq!(decoded_table, &table);
    assert_eq!(
        decoded_table
            .at(WorldTile {
                x: 0,
                z: 0,
                level: 0,
            })
            .collect::<Vec<_>>(),
        Vec::<u16>::new()
    );
    assert_eq!(
        decoded_table
            .at(WorldTile {
                x: 3,
                z: 2,
                level: 0,
            })
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn v12_shape_rows_preserve_square_and_rectangular_bounds() {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let square_bits = (1u64 << 1)
        | (1 << 2)
        | (1 << 4)
        | (1 << 5)
        | (1 << 6)
        | (1 << 7)
        | (1 << 8)
        | (1 << 9)
        | (1 << 10)
        | (1 << 11)
        | (1 << 13)
        | (1 << 14);
    let rectangular_bits = (1u64 << 1)
        | (1 << 2)
        | (1 << 3)
        | (1 << 5)
        | (1 << 6)
        | (1 << 7)
        | (1 << 8)
        | (1 << 9)
        | (1 << 11)
        | (1 << 12)
        | (1 << 13);
    let table = ZoneTable::from_parts(
        vec![
            Zone::shaped_npc(
                WorldTile {
                    x: 3,
                    z: 3,
                    level: 0,
                },
                2,
                2,
                ZoneClass::Always,
                u16::MAX,
                0,
                0,
            ),
            Zone::shaped_npc(
                WorldTile {
                    x: 9,
                    z: 3,
                    level: 0,
                },
                3,
                1,
                ZoneClass::Always,
                u16::MAX,
                0,
                1,
            ),
        ],
        vec![ZoneKind::new("hunter", "Hunter", 456, 10, false, false)],
        vec![],
        vec![],
        vec![square_bits, rectangular_bits],
        origin,
        14,
        8,
        &WildernessRules::default(),
    )
    .unwrap();
    let (walk, blocked) = pack_walk(&[0u32; 4 * 14 * 8]);
    let collision = WorldCollision {
        origin,
        width: 14,
        height: 8,
        walk,
        blocked,
        flags: None,
    };
    let graph = TransportGraph {
        zones: Some(table.clone()),
        ..Default::default()
    };

    assert_eq!(super::zones::wire_size(Some(&table)), 97);
    let bytes = encode(&collision, &graph, &[]);
    let (_, decoded, _) = decode(&bytes).unwrap();
    let decoded_table = decoded.zones.as_ref().unwrap();
    assert_eq!(decoded_table, &table);
    assert_eq!(
        decoded_table
            .at(WorldTile {
                x: 5,
                z: 3,
                level: 0,
            })
            .collect::<Vec<_>>(),
        vec![0]
    );
    assert!(decoded_table
        .at(WorldTile {
            x: 5,
            z: 2,
            level: 0,
        })
        .next()
        .is_none());
    assert_eq!(
        decoded_table
            .at(WorldTile {
                x: 12,
                z: 3,
                level: 0,
            })
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(decoded_table
        .at(WorldTile {
            x: 12,
            z: 4,
            level: 0,
        })
        .next()
        .is_none());
}

#[test]
fn v12_decode_rejects_malformed_zone_rows_and_shapes() {
    let (bytes, table) = zone_table_pack_fixture();
    let zone_start = bytes.len() - super::zones::wire_size(Some(&table));
    let kind_0_size = 15 + "ranger".len() + "Ranger".len();
    let kind_1_start = zone_start + 4 + kind_0_size;
    let zone_count_at = kind_1_start + 15 + "lava-bridge".len() + "Lava bridge".len();

    let mut too_many_zones = bytes.clone();
    too_many_zones[zone_count_at..zone_count_at + 4].copy_from_slice(&32_768u32.to_le_bytes());
    assert!(matches!(
        decode(&too_many_zones),
        Err(PackError::BadLength(_))
    ));

    let mut wrong_hazard_kind = bytes.clone();
    wrong_hazard_kind[kind_1_start..kind_1_start + 4].copy_from_slice(&0i32.to_le_bytes());
    assert!(matches!(
        decode(&wrong_hazard_kind),
        Err(PackError::BadLength(_))
    ));

    let mut bad_shape_index = bytes.clone();
    let shape_row = bytes.len() - 11;
    bad_shape_index[shape_row..shape_row + 2].copy_from_slice(&3u16.to_le_bytes());
    assert!(matches!(
        decode(&bad_shape_index),
        Err(PackError::BadLength(_))
    ));

    let mut hazard_shape = bytes.clone();
    hazard_shape[shape_row..shape_row + 2].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(
        decode(&hazard_shape),
        Err(PackError::BadLength(_))
    ));

    let mut oversized_east_extent = bytes.clone();
    let shaped_npc_radius = zone_count_at + 4 + 14 + 9;
    oversized_east_extent[shaped_npc_radius] = 7;
    assert!(matches!(
        decode(&oversized_east_extent),
        Err(PackError::BadLength(_))
    ));

    let mut oversized_north_extent = bytes.clone();
    oversized_north_extent[shape_row + 2] = 7;
    assert!(matches!(
        decode(&oversized_north_extent),
        Err(PackError::BadLength(_))
    ));
    assert!(matches!(
        decode(&bytes[..bytes.len() - 1]),
        Err(PackError::BadLength(_))
    ));

    let mut duplicate_shape = bytes;
    let shape_count_at = duplicate_shape.len() - 15;
    duplicate_shape[shape_count_at..shape_count_at + 4].copy_from_slice(&2u32.to_le_bytes());
    duplicate_shape.extend_from_slice(&1u16.to_le_bytes());
    duplicate_shape.push(1);
    duplicate_shape.extend_from_slice(&0b0011_1000u64.to_le_bytes());
    assert!(matches!(
        decode(&duplicate_shape),
        Err(PackError::BadLength(_))
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
        quest_gates: None,
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
        quest_gates: None,
    };
    for members_req in [true, false] {
        let mut graph = TransportGraph::default();
        graph.edges.push(edge(members_req));
        let bytes = encode(&collision, &graph, &[]);
        assert_eq!(bytes[4], VERSION);
        let (_, g, _) = decode(&bytes).unwrap();
        assert_eq!(g.edges[0].members_req, members_req);
    }
}

#[test]
fn v12_decode_rejects_older_version_bytes() {
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
    // A v11 pack predates zone tables and is refused for a rebake.
    bytes[4] = 11;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(11))));
    // A v10 pack predates quest-family binding and stage gates.
    bytes[4] = 10;
    assert!(matches!(decode(&bytes), Err(PackError::BadVersion(10))));
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
        quest_gates: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(door);
    let mut bytes = encode(&collision, &graph, &[]);
    // members_req sits just before wildy_cap (i32), the edge's quest-gate
    // count (u32), bank-stand count (u32), wilderness rules (12 B), and
    // the five zero-count zone tables (20 B).
    let flag_at = bytes.len() - 20 - 12 - 4 - 4 - 4 - 1;
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
fn bake_emits_bank_teller_npc_stands_and_skips_non_tellers() {
    // NPC tellers (`category=bank_teller`) join from jm2 NPC placements.
    // Closed/tutorial booths still stay out; a goblin without the
    // category does not become a stand.
    let dir = std::env::temp_dir().join(format!("274bot-nav-banks-npc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in [
        "",
        "pack",
        "scripts/interface_bank/configs",
        "scripts/areas/area_goblin/configs",
        "maps",
    ] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(
        dir.join("pack/loc.pack"),
        "2213=bankbooth\n2215=bankboothclosed\n3045=newbiebankbooth\n",
    )
    .unwrap();
    std::fs::write(dir.join("pack/npc.pack"), "1036=werewolfbanker\n1=goblin\n").unwrap();
    std::fs::write(
        dir.join("scripts/interface_bank/configs/bank_booth.loc"),
        "[bankbooth]\nname=Bank booth\nop2=Use-quickly\n\n[bankboothclosed]\nname=Closed bank booth\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("scripts/interface_bank/configs/banker.npc"),
        "[werewolfbanker]\nname=Banker\nop1=Talk-to\nop3=Bank\ncategory=bank_teller\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("scripts/areas/area_goblin/configs/goblin.npc"),
        "[goblin]\nname=Goblin\nop1=Talk-to\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("maps/m50_50.jm2"),
        "==== MAP ====\n0 0 0: h1 o6 u48\n==== LOC ====\n0 12 32: 2213 10 1\n0 13 32: 2215 10 1\n0 14 32: 3045 10 1\n==== NPC ====\n0 20 21: 1036\n0 22 22: 1\n",
    )
    .unwrap();
    let banks = derive_banks(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(banks.len(), 2, "booth + teller, not closed/tutorial/goblin");
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
    assert_eq!(banks[1].name, "Banker");
    assert_eq!(
        banks[1].tile,
        WorldTile {
            x: 50 * 64 + 20,
            z: 50 * 64 + 21,
            level: 0,
        }
    );
    assert_eq!(
        banks[1].access,
        BankAccess::Npc {
            name: "Banker".into(),
            op: 3,
            choose: None,
        }
    );
}

#[test]
fn bake_emits_bank_teller_when_the_tree_has_no_booths() {
    let dir =
        std::env::temp_dir().join(format!("274bot-nav-banks-npc-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["", "pack", "scripts/interface_bank/configs", "maps"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("pack/loc.pack"), "1=crate\n").unwrap();
    std::fs::write(dir.join("pack/npc.pack"), "1036=werewolfbanker\n").unwrap();
    std::fs::write(
        dir.join("scripts/interface_bank/configs/banker.npc"),
        "[werewolfbanker]\nname=Banker\nop1=Talk-to\nop3=Bank\ncategory=bank_teller\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("maps/m50_50.jm2"),
        "==== MAP ====\n==== LOC ====\n==== NPC ====\n0 20 21: 1036\n",
    )
    .unwrap();
    let banks = derive_banks(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(banks.len(), 1, "tellers still pack without a bankbooth loc");
    assert_eq!(
        banks[0].access,
        BankAccess::Npc {
            name: "Banker".into(),
            op: 3,
            choose: None,
        }
    );
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
        quest_gates: None,
    });
    let bytes = encode(&collision, &graph, &[]);
    assert_eq!(bytes[4], VERSION);
    let (_, g, _) = decode(&bytes).unwrap();
    assert_eq!(g.wilderness, graph.wilderness);
    assert_eq!(g.teleports[0].wildy_cap, Some(20));
}

/// A 2×2 open collision: the smallest pack body the gate tests append to.
fn tiny_collision() -> WorldCollision {
    let (walk, blocked) = pack_walk(&[0u32; 4 * 2 * 2]);
    WorldCollision {
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
    }
}

/// One door crossing carrying `quest_gates`.
fn gated_door(quest_gates: Option<QuestGates>) -> TransportEdge {
    TransportEdge {
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
        loc_id: 2621,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::E),
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates,
    }
}

/// v11 binds the quest family in the header and carries each edge's typed
/// stage gates: an equality window, an upper-only window and a completed
/// quest come back exactly, keys and open bounds included. A bake that
/// consumed no family binds none.
#[test]
fn v11_roundtrips_quest_family_and_stage_gates() {
    let family = gate_family(7);
    let gates = QuestGates::new(
        family,
        [
            gate_window("tbwt", "tbwt_main", Some(3), Some(3)),
            gate_window("heroes", "heroes_main", None, Some(4)),
            QuestGate::Complete(FactKey::new("arrav")),
        ],
    )
    .unwrap();
    let mut graph = TransportGraph {
        quest_family: Some(family),
        ..TransportGraph::default()
    };
    graph.edges.push(gated_door(Some(gates.clone())));
    let completed = QuestGates::new(family, [QuestGate::Complete(FactKey::new("tbwt"))]).unwrap();
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        ..gated_door(Some(completed))
    });
    let bytes = encode(&tiny_collision(), &graph, &[]);
    let (_, g, _) = decode(&bytes).unwrap();
    assert_eq!(g.quest_family, Some(family));
    assert_eq!(g.edges, graph.edges);
    assert_eq!(g.edges[0].quest_gates, Some(gates));
    assert_eq!(g.teleports, graph.teleports);

    let plain = encode(&tiny_collision(), &TransportGraph::default(), &[]);
    assert_eq!(decode(&plain).unwrap().1.quest_family, None);
}

#[test]
fn nav_world_from_reader_streams_pack_and_legacy_grid() {
    let family = gate_family(13);
    let gates = QuestGates::new(
        family,
        [gate_window("heroes", "heroes_main", Some(2), Some(5))],
    )
    .unwrap();
    let mut graph = TransportGraph {
        quest_family: Some(family),
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
    graph.edges.push(gated_door(Some(gates)));
    let banks = vec![BankStand {
        name: "Lumbridge booth".into(),
        tile: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        access: BankAccess::Npc {
            name: "Banker".into(),
            op: 3,
            choose: Some("Bank".into()),
        },
    }];
    let mut bytes = encode(&tiny_collision(), &graph, &banks);
    bytes.extend_from_slice(&[0xA1, 0xB2, 0xC3]);

    let expected = NavWorld::from_bytes(&bytes).unwrap();
    let mut reader = ShortReadBuf::new(&bytes);
    let streamed = NavWorld::from_reader(&mut reader, bytes.len()).unwrap();
    assert_eq!(reader.position, bytes.len());
    assert_eq!(streamed.collision.origin, expected.collision.origin);
    assert_eq!(streamed.collision.width, expected.collision.width);
    assert_eq!(streamed.collision.height, expected.collision.height);
    assert_eq!(streamed.collision.walk, expected.collision.walk);
    assert_eq!(streamed.collision.blocked, expected.collision.blocked);
    assert_eq!(streamed.graph.edges, expected.graph.edges);
    assert_eq!(streamed.graph.teleports, expected.graph.teleports);
    assert_eq!(streamed.graph.at, expected.graph.at);
    assert_eq!(streamed.graph.quest_family, expected.graph.quest_family);
    assert_eq!(streamed.graph.wilderness, expected.graph.wilderness);
    assert_eq!(streamed.banks(), expected.banks());

    let pack_length = bytes.len() - 3;
    let mut bounded_reader = ShortReadBuf::new(&bytes);
    let _bounded = NavWorld::from_reader(&mut bounded_reader, pack_length).unwrap();
    assert_eq!(bounded_reader.position, pack_length);

    let truncated_pack = &bytes[..pack_length - 1];
    let mut truncated_reader = ShortReadBuf::new(truncated_pack);
    assert!(matches!(
        NavWorld::from_reader(&mut truncated_reader, truncated_pack.len()),
        Err(PackError::Truncated)
    ));
    let mut stale_version = bytes.clone();
    stale_version[4] = VERSION - 1;
    let mut stale_reader = ShortReadBuf::new(&stale_version);
    assert!(matches!(
        NavWorld::from_reader(&mut stale_reader, stale_version.len()),
        Err(PackError::BadVersion(version)) if version == VERSION - 1
    ));
    let mut unknown_magic = ShortReadBuf::new(b"????");
    assert!(matches!(
        NavWorld::from_reader(&mut unknown_magic, 4),
        Err(PackError::BadMagic)
    ));

    let grid = StepGrid::fixture_open_3x3();
    let legacy_bytes = encode_grid(&grid);
    let expected_legacy = NavWorld::from_bytes(&legacy_bytes).unwrap();
    let mut legacy_reader = ShortReadBuf::new(&legacy_bytes);
    let streamed_legacy = NavWorld::from_reader(&mut legacy_reader, legacy_bytes.len()).unwrap();
    assert_eq!(
        streamed_legacy.collision.origin,
        expected_legacy.collision.origin
    );
    assert_eq!(
        streamed_legacy.collision.walk,
        expected_legacy.collision.walk
    );
    assert_eq!(
        streamed_legacy.collision.blocked,
        expected_legacy.collision.blocked
    );
    assert_eq!(streamed_legacy.graph.edges, expected_legacy.graph.edges);
    assert_eq!(streamed_legacy.graph.at, expected_legacy.graph.at);
}

/// Encode never writes a family binding its own decoder refuses: every
/// family a caller can build — extractor schema 1 through `u16::MAX`, the
/// schema 0 the decoder rejects being unconstructible — binds a gated pack
/// that decodes to the same family and gates.
#[test]
fn every_constructible_quest_family_encodes_a_decodable_pack() {
    for (digest, schema) in [(0x00, 1), (0x7f, 0x0100), (0xff, u16::MAX)] {
        let family = gate_family_schema(digest, schema);
        let gates =
            QuestGates::new(family, [gate_window("tbwt", "tbwt_main", Some(3), Some(3))]).unwrap();
        let mut graph = TransportGraph {
            quest_family: Some(family),
            ..TransportGraph::default()
        };
        graph.edges.push(gated_door(Some(gates.clone())));
        let (_, g, _) = decode(&encode(&tiny_collision(), &graph, &[]))
            .unwrap_or_else(|e| panic!("schema {schema}: {e:?}"));
        assert_eq!(g.quest_family, Some(family), "schema {schema}");
        assert_eq!(g.edges[0].quest_gates, Some(gates), "schema {schema}");
    }
}

/// Gates a pack cannot bind, or a malformed binding, refuse the whole pack
/// as inconsistent; nothing loads as silently ungated.
#[test]
fn v12_decode_refuses_unbound_or_malformed_quest_gates() {
    // The single edge's gate list ends its record, before the bank-stand
    // count (u32), wilderness trailer (12 B), and zone counts (20 B).
    const TRAILER: usize = 4 + 12 + 20;
    let mut graph = TransportGraph::default();
    graph.edges.push(gated_door(None));
    let unbound = encode(&tiny_collision(), &graph, &[]);
    let count_at = unbound.len() - TRAILER - 4;
    let mut spliced = unbound[..count_at].to_vec();
    spliced.extend_from_slice(&1u32.to_le_bytes());
    spliced.push(0); // a completed-quest gate
    spliced.extend_from_slice(&5u32.to_le_bytes());
    spliced.extend_from_slice(b"arrav");
    spliced.extend_from_slice(&unbound[count_at + 4..]);
    assert!(
        matches!(decode(&spliced), Err(PackError::BadLength(_))),
        "gates on a pack that binds no quest family"
    );
    let mut bad_tag = unbound.clone();
    bad_tag[5] = 2; // the binding tag follows the version byte
    assert!(matches!(decode(&bad_tag), Err(PackError::BadLength(_))));

    let family = gate_family(7);
    let mut bound = TransportGraph {
        quest_family: Some(family),
        ..TransportGraph::default()
    };
    let exact = gate_window("tbwt", "tbwt_main", Some(3), Some(3));
    bound
        .edges
        .push(gated_door(Some(QuestGates::new(family, [exact]).unwrap())));
    let bytes = encode(&tiny_collision(), &bound, &[]);
    assert!(decode(&bytes).is_ok());
    let mut schema_zero = bytes.clone();
    schema_zero[6 + 32..6 + 34].copy_from_slice(&0u16.to_le_bytes());
    assert!(matches!(decode(&schema_zero), Err(PackError::BadLength(_))));
    // The window's bounds close the record: min flag + i32, max flag + i32.
    let max_at = bytes.len() - TRAILER - 4;
    let min_at = max_at - 1 - 4;
    let mut empty_window = bytes.clone();
    empty_window[min_at..min_at + 4].copy_from_slice(&4i32.to_le_bytes());
    assert!(
        matches!(decode(&empty_window), Err(PackError::BadLength(_))),
        "a [4,3] window admits no value"
    );
    let mut bad_flag = bytes.clone();
    bad_flag[max_at - 1] = 2;
    assert!(matches!(decode(&bad_flag), Err(PackError::BadLength(_))));
}

/// One pack binds one quest family: an edge whose gates name another family
/// cannot be written, so its keys are never silently rebound.
#[test]
#[should_panic(expected = "another quest family")]
fn encode_refuses_gates_of_another_quest_family() {
    let foreign = QuestGates::new(
        gate_family(8),
        [gate_window("tbwt", "tbwt_main", Some(3), Some(3))],
    )
    .unwrap();
    let mut graph = TransportGraph {
        quest_family: Some(gate_family(7)),
        ..TransportGraph::default()
    };
    graph.edges.push(gated_door(Some(foreign)));
    encode(&tiny_collision(), &graph, &[]);
}

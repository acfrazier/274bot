//! Independent dense-oracle regressions for 32×32 tiled collision storage.
//! Test-only: never linked into production. Compares tiled queries/encode
//! against a separate dense buffer oracle, not two calls through the same helper.

use super::*;
use crate::pack::{decode, encode, PackError};
use crate::router::{find, step_ok, RouteError};
use crate::transport::TransportGraph;
use api::snapshot::WorldTile;

/// Independent dense oracle retained only in tests.
struct DenseOracle {
    origin: WorldTile,
    width: usize,
    height: usize,
    walk: Vec<u8>,
    blocked: Vec<u64>,
    flags: Option<Vec<u32>>,
}

impl DenseOracle {
    fn from_parts(
        origin: WorldTile,
        width: usize,
        height: usize,
        walk: Vec<u8>,
        blocked: Vec<u64>,
        flags: Option<Vec<u32>>,
    ) -> Self {
        Self {
            origin,
            width,
            height,
            walk,
            blocked,
            flags,
        }
    }

    fn plane_cells(&self) -> usize {
        self.width * self.height
    }

    fn pair_at(&self, index: usize) -> Option<(u8, bool)> {
        if index >= self.walk.len() {
            return None;
        }
        let blk = (self.blocked[index >> 6] >> (index & 63)) & 1 != 0;
        Some((self.walk[index], blk))
    }

    fn flag(&self, x: i32, z: i32, level: i32) -> u32 {
        let Some(flags) = self.flags.as_ref() else {
            return 0;
        };
        if !(0..4).contains(&level) {
            return 0;
        }
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        flags[level as usize * self.plane_cells() + lz * self.width + lx]
    }

    fn walkable_word(&self, x: i32, z: i32, level: i32) -> u32 {
        if !(0..4).contains(&level) {
            return 0;
        }
        let lx = x - self.origin.x;
        let lz = z - self.origin.z;
        if lx < 0 || lz < 0 {
            return 0;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return 0;
        }
        let idx = level as usize * self.plane_cells() + lz * self.width + lx;
        let (face, blk) = self.pair_at(idx).expect("dense oracle geometric index");
        walk_word_from_parts(face, blk)
    }

    fn walkable(&self, t: WorldTile) -> bool {
        if !(0..4).contains(&t.level) {
            return false;
        }
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        let idx = t.level as usize * self.plane_cells() + lz * self.width + lx;
        match &self.flags {
            Some(flags) => flags[idx] & WALK_BLOCK == 0,
            None => {
                let (face, blk) = self.pair_at(idx).expect("dense oracle geometric index");
                walk_word_from_parts(face, blk) & WALK_BLOCK == 0
            }
        }
    }

    fn standable(&self, t: WorldTile) -> bool {
        if !(0..4).contains(&t.level) {
            return false;
        }
        let lx = t.x - self.origin.x;
        let lz = t.z - self.origin.z;
        if lx < 0 || lz < 0 {
            return false;
        }
        let (lx, lz) = (lx as usize, lz as usize);
        if lx >= self.width || lz >= self.height {
            return false;
        }
        let idx = t.level as usize * self.plane_cells() + lz * self.width + lx;
        match &self.flags {
            Some(flags) => flags[idx] & SQ_BLOCKED == 0,
            None => {
                let (_face, blk) = self.pair_at(idx).expect("dense oracle geometric index");
                !blk
            }
        }
    }
}

fn freeze(
    origin: WorldTile,
    width: usize,
    height: usize,
    walk: Vec<u8>,
    blocked: Vec<u64>,
    flags: Option<Vec<u32>>,
) -> (DenseOracle, WorldCollision) {
    let oracle = DenseOracle::from_parts(
        origin,
        width,
        height,
        walk.clone(),
        blocked.clone(),
        flags.clone(),
    );
    let tiled = WorldCollision::from_packed_parts(origin, width, height, walk, blocked, flags)
        .expect("packed parts");
    (oracle, tiled)
}

fn open_plane(width: usize, height: usize, planes: usize) -> (Vec<u8>, Vec<u64>) {
    let cells = width * height * planes;
    (vec![0u8; cells], vec![0u64; cells.div_ceil(64)])
}

fn assert_all_cells(oracle: &DenseOracle, tiled: &WorldCollision) {
    assert_eq!(oracle.walk.len(), tiled.logical_cell_count());
    assert_eq!(oracle.blocked.len(), tiled.blocked_word_count());
    assert_eq!(oracle.origin, tiled.origin());
    assert_eq!(oracle.width, tiled.width());
    assert_eq!(oracle.height, tiled.height());

    for i in 0..oracle.walk.len() {
        let o = oracle.pair_at(i).unwrap();
        let t = tiled.packed_pair_at(i).unwrap();
        assert_eq!(o, t, "pair mismatch at index {i}");
    }
    assert_eq!(tiled.packed_pair_at(oracle.walk.len()), None);

    for (i, w) in tiled.packed_blocked_words().enumerate() {
        assert_eq!(w, oracle.blocked[i], "blocked word {i}");
    }

    let ox = oracle.origin.x;
    let oz = oracle.origin.z;
    let present_planes = oracle.walk.len() / oracle.plane_cells().max(1);
    for level in 0..4i32 {
        // Short synthetic fixtures only retain complete present planes; probes
        // on absent upper planes panic on both dense and tiled arms — skip.
        if (level as usize) >= present_planes {
            continue;
        }
        for z in 0..oracle.height as i32 {
            for x in 0..oracle.width as i32 {
                let wx = ox + x;
                let wz = oz + z;
                assert_eq!(
                    oracle.walkable_word(wx, wz, level),
                    tiled.walkable_word(wx, wz, level),
                    "walkable_word ({wx},{wz},{level})"
                );
                let t = WorldTile {
                    x: wx,
                    z: wz,
                    level,
                };
                assert_eq!(oracle.walkable(t), tiled.walkable(t), "walkable {t:?}");
                assert_eq!(oracle.standable(t), tiled.standable(t), "standable {t:?}");
                assert_eq!(oracle.flag(wx, wz, level), tiled.flag(wx, wz, level));
            }
        }
    }

    // Outside / invalid probes.
    for (x, z, level) in [
        (ox - 1, oz, 0),
        (ox, oz - 1, 0),
        (ox + oracle.width as i32, oz, 0),
        (ox, oz + oracle.height as i32, 0),
        (ox, oz, -1),
        (ox, oz, 4),
    ] {
        assert_eq!(oracle.walkable_word(x, z, level), tiled.walkable_word(x, z, level));
        assert_eq!(
            oracle.walkable(WorldTile { x, z, level }),
            tiled.walkable(WorldTile { x, z, level })
        );
        assert_eq!(
            oracle.standable(WorldTile { x, z, level }),
            tiled.standable(WorldTile { x, z, level })
        );
        assert_eq!(oracle.flag(x, z, level), tiled.flag(x, z, level));
    }

    let (ew, eb) = tiled.to_packed_parts();
    assert_eq!(ew, oracle.walk);
    assert_eq!(eb, oracle.blocked);
}

#[test]
fn all_512_uniform_pairs_round_trip_through_tiled_storage() {
    // 32×32 single plane: one full tile. Every face×blocked pair as uniform.
    for face in 0u8..=255 {
        for blk in [false, true] {
            let walk = vec![face; TILE_CELLS];
            let mut blocked = vec![0u64; TILE_CELLS / 64];
            if blk {
                blocked.fill(u64::MAX);
            }
            let (oracle, tiled) = freeze(
                WorldTile {
                    x: 10,
                    z: -3,
                    level: 0,
                },
                TILE,
                TILE,
                walk,
                blocked,
                None,
            );
            assert_eq!(tiled.dense_pool_len(), 0, "uniform pair must not allocate dense");
            assert_eq!(tiled.directory_len(), 1);
            assert_all_cells(&oracle, &tiled);
            let (f, b) = tiled.packed_pair_at(0).unwrap();
            assert_eq!((f, b), (face, blk));
        }
    }
}

#[test]
fn mixed_tile_and_edge_dimensions_match_dense_oracle() {
    let dims = [1usize, 31, 32, 33, 63, 64, 65];
    for &w in &dims {
        for &h in &dims {
            // Seeded mixed pattern across one plane.
            let cells = w * h;
            let mut walk = vec![0u8; cells];
            let mut blocked = vec![0u64; cells.div_ceil(64)];
            for i in 0..cells {
                walk[i] = ((i * 17 + w * 3 + h) % 251) as u8;
                if (i * 13 + h) % 7 == 0 {
                    blocked[i >> 6] |= 1u64 << (i & 63);
                }
            }
            // Preserve a non-zero high bit on the final blocked word when not aligned.
            if cells % 64 != 0 {
                let rem = cells % 64;
                if let Some(last) = blocked.last_mut() {
                    *last |= 1u64 << rem; // unused high bit
                }
            }
            let (oracle, tiled) = freeze(
                WorldTile {
                    x: -(w as i32),
                    z: h as i32,
                    level: 0,
                },
                w,
                h,
                walk,
                blocked,
                None,
            );
            assert_all_cells(&oracle, &tiled);
            let (tc, tr) = tiled.tile_grid();
            assert_eq!(tc, w.div_ceil(TILE));
            assert_eq!(tr, h.div_ceil(TILE));
        }
    }
}

#[test]
fn single_differing_cell_forces_dense_and_matches_neighbors() {
    let mut walk = vec![0u8; TILE_CELLS];
    let blocked = vec![0u64; TILE_CELLS / 64];
    walk[TILE_CELLS - 1] = 0x3c; // only last cell differs
    let (oracle, tiled) = freeze(
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        TILE,
        TILE,
        walk,
        blocked,
        None,
    );
    assert_eq!(tiled.dense_pool_len(), 1);
    assert_all_cells(&oracle, &tiled);
}

#[test]
fn four_plane_distinct_contents_and_padding() {
    let w = 33usize;
    let h = 17usize;
    let planes = 4usize;
    let cells = w * h * planes;
    let mut walk = vec![0u8; cells];
    let mut blocked = vec![0u64; cells.div_ceil(64)];
    for p in 0..planes {
        for z in 0..h {
            for x in 0..w {
                let i = p * w * h + z * w + x;
                walk[i] = ((p * 40 + x + z * 3) & 0xff) as u8;
                if (x + z + p) % 5 == 0 {
                    blocked[i >> 6] |= 1u64 << (i & 63);
                }
            }
        }
    }
    if cells % 64 != 0 {
        let rem = cells % 64;
        blocked.last_mut().map(|l| *l |= 0xa5u64 << rem);
    }
    let (oracle, tiled) = freeze(
        WorldTile {
            x: 100,
            z: 200,
            level: 0,
        },
        w,
        h,
        walk,
        blocked,
        None,
    );
    assert_eq!(tiled.logical_cell_count(), cells);
    assert_all_cells(&oracle, &tiled);
}

#[test]
fn raw_sidecar_attach_drop_matches_oracle_and_preserves_walk() {
    let w = 5usize;
    let h = 5usize;
    let mut flags = vec![0u32; 4 * w * h];
    flags[0] = CollisionFlag::W_N as u32;
    flags[1] = CollisionFlag::WR_GRND as u32;
    flags[2] = CollisionFlag::WALK_SCENERY as u32;
    let (walk, blocked) = pack_walk(&flags);
    let (mut oracle, mut tiled) = freeze(
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        w,
        h,
        walk,
        blocked,
        None,
    );
    // Without sidecar: both use packed pairs.
    assert_all_cells(&oracle, &tiled);

    oracle.flags = Some(flags.clone());
    tiled.attach_flags(flags);
    assert_all_cells(&oracle, &tiled);
    let word = tiled.walkable_word(0, 0, 0);
    tiled.drop_flags();
    oracle.flags = None;
    assert_eq!(tiled.walkable_word(0, 0, 0), word);
    assert_all_cells(&oracle, &tiled);
}

#[test]
fn encode_bytes_match_dense_baseline_and_roundtrip() {
    let w = 35usize;
    let h = 12usize;
    let (mut walk, mut blocked) = open_plane(w, h, 4);
    for i in 0..walk.len() {
        walk[i] = (i % 200) as u8;
        if i % 11 == 0 {
            blocked[i >> 6] |= 1u64 << (i & 63);
        }
    }
    if walk.len() % 64 != 0 {
        let rem = walk.len() % 64;
        blocked.last_mut().map(|l| *l |= 1u64 << (rem + 1).min(63));
    }
    let origin = WorldTile {
        x: 7,
        z: -9,
        level: 0,
    };
    let walk_b = walk.clone();
    let blocked_b = blocked.clone();
    let (oracle, tiled) = freeze(origin, w, h, walk, blocked, None);
    assert_all_cells(&oracle, &tiled);

    // Dense-baseline encode: rebuild a temporary old-style encode via to_packed_parts path
    // is the tiled path; independent baseline streams oracle buffers.
    let mut baseline = Vec::new();
    baseline.extend_from_slice(b"274V");
    baseline.push(8);
    for v in [origin.x, origin.z, origin.level] {
        baseline.extend_from_slice(&v.to_le_bytes());
    }
    baseline.extend_from_slice(&(w as u32).to_le_bytes());
    baseline.extend_from_slice(&(h as u32).to_le_bytes());
    baseline.extend_from_slice(&walk_b);
    for word in &blocked_b {
        baseline.extend_from_slice(&word.to_le_bytes());
    }
    baseline.extend_from_slice(&0u32.to_le_bytes()); // edges
    baseline.extend_from_slice(&0u32.to_le_bytes()); // banks

    let graph = TransportGraph::default();
    let got = encode(&tiled, &graph, &[]);
    assert_eq!(got, baseline, "tiled encode must match dense baseline bytes");

    let (back, g2, banks) = decode(&got).expect("decode");
    assert!(banks.is_empty());
    assert!(g2.edges.is_empty() && g2.teleports.is_empty());
    let (oracle2, _) = freeze(origin, w, h, walk_b, blocked_b, None);
    assert_all_cells(&oracle2, &back);
    assert_eq!(encode(&back, &graph, &[]), baseline);
}

#[test]
fn constructor_rejects_malformed_shapes_without_replacing_wire_errors() {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    assert!(matches!(
        WorldCollision::from_packed_parts(origin, 0, 1, vec![0], vec![0], None),
        Err(PackedPartsError::ZeroDimension)
    ));
    match WorldCollision::from_packed_parts(origin, 2, 2, vec![0; 5], vec![0], None) {
        Err(PackedPartsError::InconsistentLength {
            width: 2,
            height: 2,
            walk_len: 5,
            blocked_words: 1,
            expected_blocked_words: 1,
        }) => {}
        Err(e) => panic!("expected inconsistent length, got {e}"),
        Ok(_) => panic!("expected inconsistent length, got Ok"),
    }
    // Wire error path remains PackError, not PackedPartsError.
    let err = match decode(b"XXXX") {
        Err(e) => e,
        Ok(_) => panic!("expected BadMagic"),
    };
    assert!(matches!(err, PackError::BadMagic));
    let mut bad_ver = Vec::from(&b"274V"[..]);
    bad_ver.push(7);
    assert!(matches!(decode(&bad_ver), Err(PackError::BadVersion(7))));
}

#[test]
fn malformed_wire_corpus_matches_pack_error_variants() {
    // Differential vs the same decode path that freezes only after success:
    // every failure must be PackError (never PackedPartsError) with the same
    // variant/payload the dense-era decoder produced before conversion.
    // Freeze (`from_packed_parts`) runs only after full parse success.
    fn expect(bytes: &[u8], check: impl FnOnce(PackError)) {
        match decode(bytes) {
            Err(e) => {
                // Wire failures stay PackError; never rewrite through constructor.
                let _ = &e as &PackError;
                check(e);
            }
            Ok(_) => panic!("expected decode error"),
        }
    }

    expect(b"XXXX", |e| assert!(matches!(e, PackError::BadMagic)));
    expect(b"274N\x08", |e| assert!(matches!(e, PackError::BadMagic)));
    expect(b"274V", |e| assert!(matches!(e, PackError::Truncated)));
    expect(b"274V\x08", |e| assert!(matches!(e, PackError::Truncated)));

    for ver in [0u8, 1, 2, 3, 4, 5, 6, 7, 9, 255] {
        let mut b = Vec::from(&b"274V"[..]);
        b.push(ver);
        expect(&b, |e| assert!(matches!(e, PackError::BadVersion(v) if v == ver)));
    }

    // Header complete enough to fail dimension checks before body reads.
    let hdr = |w: u32, h: u32| {
        let mut b = Vec::from(&b"274V\x08"[..]);
        for v in [0i32, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        b
    };
    expect(&hdr(0, 1), |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m.contains("exceeds the") || m.contains("0x")
        ))
    });
    expect(&hdr(1, 0), |e| assert!(matches!(e, PackError::BadLength(_))));
    expect(&hdr(u32::MAX, u32::MAX), |e| {
        assert!(matches!(e, PackError::BadLength(_)))
    });
    // Oversize vs MAX_GRID (16384).
    expect(&hdr(1 << 20, 1), |e| assert!(matches!(e, PackError::BadLength(_))));

    // Truncated face plane after valid header 1×1 four planes = 4 faces.
    let mut short_faces = hdr(1, 1);
    short_faces.push(0); // one face only
    expect(&short_faces, |e| assert!(matches!(e, PackError::Truncated)));

    // Full faces, truncated blocked words (1×1×4 = 4 cells → 1 word needed).
    let mut short_bits = hdr(1, 1);
    short_bits.extend_from_slice(&[0u8; 4]);
    expect(&short_bits, |e| assert!(matches!(e, PackError::Truncated)));

    // Faces+blocked present, truncated edge count / edge body.
    let mut collision_only = hdr(1, 1);
    collision_only.extend_from_slice(&[0u8; 4]);
    collision_only.extend_from_slice(&0u64.to_le_bytes());
    expect(&collision_only, |e| assert!(matches!(e, PackError::Truncated)));

    let mut edge_hdr = collision_only.clone();
    edge_hdr.extend_from_slice(&1u32.to_le_bytes()); // claims 1 edge
    expect(&edge_hdr, |e| assert!(matches!(e, PackError::Truncated)));

    // Bad transport kind tag: kind is read before body fields.
    let mut bad_kind = collision_only.clone();
    bad_kind.extend_from_slice(&1u32.to_le_bytes());
    bad_kind.push(99); // unknown kind
    bad_kind.extend_from_slice(&[0u8; 80]);
    expect(&bad_kind, |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m == "unknown transport kind 99"
        ))
    });

    // Edge fixed fields through ticks (kind + 9×i32), then dir/open_loc/reqs.
    fn push_edge_fixed(out: &mut Vec<u8>, kind: u8, dir: u8) {
        out.push(kind);
        for v in [0i32; 9] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(dir);
        out.extend_from_slice(&(-1i32).to_le_bytes()); // open_loc_id None
    }
    fn push_empty_req_tail(out: &mut Vec<u8>) {
        // skill, item, quest, varp, worn — five empty count prefixes
        for _ in 0..5 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }

    // Unknown door dir tag (dir=5) with enough body so dir is reached.
    let mut bad_dir = collision_only.clone();
    bad_dir.extend_from_slice(&1u32.to_le_bytes());
    push_edge_fixed(&mut bad_dir, 0, 5);
    push_empty_req_tail(&mut bad_dir);
    bad_dir.extend_from_slice(&0u32.to_le_bytes()); // banks
    expect(&bad_dir, |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m == "unknown door dir 5"
        ))
    });

    // Requirement section truncation: skill_req claims 1 pair, no body.
    let mut short_req = collision_only.clone();
    short_req.extend_from_slice(&1u32.to_le_bytes());
    push_edge_fixed(&mut short_req, 0, 0);
    // skill_req count=1 with no pair bytes → Truncated on skill pair read
        short_req.extend_from_slice(&1u32.to_le_bytes());
        expect(&short_req, |e| assert!(matches!(e, PackError::Truncated)));

    // Quest req claims one string of length 1 with invalid UTF-8 payload.
    let mut bad_quest_utf8 = collision_only.clone();
    bad_quest_utf8.extend_from_slice(&1u32.to_le_bytes());
    push_edge_fixed(&mut bad_quest_utf8, 0, 0);
    bad_quest_utf8.extend_from_slice(&0u32.to_le_bytes()); // skill empty
    bad_quest_utf8.extend_from_slice(&0u32.to_le_bytes()); // item empty
    bad_quest_utf8.extend_from_slice(&1u32.to_le_bytes()); // one quest
    bad_quest_utf8.extend_from_slice(&1u32.to_le_bytes()); // len=1
    bad_quest_utf8.push(0xff); // invalid UTF-8; remaining reqs unused
        expect(&bad_quest_utf8, |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m == "quest req is not UTF-8"
        ))
    });

    // Empty edges, bank section truncated after count=1.
    let mut short_bank = collision_only.clone();
    short_bank.extend_from_slice(&0u32.to_le_bytes()); // edges
    short_bank.extend_from_slice(&1u32.to_le_bytes()); // banks=1
    expect(&short_bank, |e| assert!(matches!(e, PackError::Truncated)));

    // Bank stand name is not UTF-8.
    let mut bad_bank_utf8 = collision_only.clone();
    bad_bank_utf8.extend_from_slice(&0u32.to_le_bytes()); // edges
    bad_bank_utf8.extend_from_slice(&1u32.to_le_bytes()); // banks=1
    bad_bank_utf8.extend_from_slice(&1u32.to_le_bytes()); // name len=1
    bad_bank_utf8.push(0xff); // invalid UTF-8 name
    expect(&bad_bank_utf8, |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m == "bank stand name is not UTF-8"
        ))
    });

    // Unknown bank access tag after a valid empty name + tile.
    let mut bad_bank_tag = collision_only.clone();
    bad_bank_tag.extend_from_slice(&0u32.to_le_bytes()); // edges
    bad_bank_tag.extend_from_slice(&1u32.to_le_bytes()); // banks=1
    bad_bank_tag.extend_from_slice(&0u32.to_le_bytes()); // empty name
    for v in [0i32; 3] {
        bad_bank_tag.extend_from_slice(&v.to_le_bytes());
    }
    bad_bank_tag.push(2); // unknown access tag
    bad_bank_tag.extend_from_slice(&[0u8; 8]);
    expect(&bad_bank_tag, |e| {
        assert!(matches!(
            e,
            PackError::BadLength(m) if m == "unknown bank access tag 2"
        ))
    });

    // Valid empty graph/banks pack (baseline acceptance) still round-trips.
    let mut ok = collision_only.clone();
    ok.extend_from_slice(&0u32.to_le_bytes()); // edges
    ok.extend_from_slice(&0u32.to_le_bytes()); // banks
    let (c, g, banks) = decode(&ok).expect("minimal valid pack");
    assert_eq!(c.logical_cell_count(), 4);
    assert!(g.edges.is_empty() && g.teleports.is_empty());
    assert!(banks.is_empty());
    // Trailing garbage after a valid pack remains accepted (unread).
    let mut trailing = ok.clone();
    trailing.extend_from_slice(b"GARBAGE");
    assert!(decode(&trailing).is_ok());

    // Flags sidecar errors stay PackError and never become constructor errors.
    use crate::pack::decode_flags_sidecar;
    assert!(matches!(
        decode_flags_sidecar(b"XXXX"),
        Err(PackError::BadMagic)
    ));
    assert!(matches!(
        decode_flags_sidecar(b"274F\x02"),
        Err(PackError::BadVersion(2))
    ));
    // Sidecar truncated body after valid header.
    let mut short_flags = Vec::from(&b"274F\x01"[..]);
    for v in [0i32, 0, 0] {
        short_flags.extend_from_slice(&v.to_le_bytes());
    }
    short_flags.extend_from_slice(&1u32.to_le_bytes());
    short_flags.extend_from_slice(&1u32.to_le_bytes());
    // no flag u32 body — decode_flags_sidecar reads to end; empty is Ok with
    // empty flags, but a partial trailing u32 is Truncated. One leftover byte:
    short_flags.push(0);
    assert!(matches!(
        decode_flags_sidecar(&short_flags),
        Err(PackError::Truncated)
    ));
}

/// Independent dense `step_ok` — same masks/order as the router, but reads
/// only through [`DenseOracle::walkable_word`] (never tiled storage).
fn dense_step_ok(oracle: &DenseOracle, cur: WorldTile, d: (i32, i32)) -> bool {
    use client::dash3d::CollisionFlag;
    const MASK_N: u32 = CollisionFlag::PL_WALK_N as u32;
    const MASK_E: u32 = CollisionFlag::PL_WALK_E as u32;
    const MASK_S: u32 = CollisionFlag::PL_WALK_S as u32;
    const MASK_W: u32 = CollisionFlag::PL_WALK_W as u32;
    const MASK_NE: u32 = CollisionFlag::PL_WALK_NE as u32;
    const MASK_SE: u32 = CollisionFlag::PL_WALK_SE as u32;
    const MASK_NW: u32 = CollisionFlag::PL_WALK_NW as u32;
    const MASK_SW: u32 = CollisionFlag::PL_WALK_SW as u32;

    let nb = WorldTile {
        x: cur.x + d.0,
        z: cur.z + d.1,
        level: cur.level,
    };
    let lx = nb.x - oracle.origin.x;
    let lz = nb.z - oracle.origin.z;
    if lx < 0 || lz < 0 {
        return false;
    }
    if lx as usize >= oracle.width || lz as usize >= oracle.height {
        return false;
    }
    let f = |x: i32, z: i32| oracle.walkable_word(x, z, nb.level);
    match (d.0, d.1) {
        (0, 1) => f(nb.x, nb.z) & MASK_S == 0,
        (0, -1) => f(nb.x, nb.z) & MASK_N == 0,
        (1, 0) => f(nb.x, nb.z) & MASK_W == 0,
        (-1, 0) => f(nb.x, nb.z) & MASK_E == 0,
        (-1, -1) => {
            f(nb.x, nb.z) & MASK_NE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (1, -1) => {
            f(nb.x, nb.z) & MASK_NW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z - 1) & MASK_N == 0
        }
        (-1, 1) => {
            f(nb.x, nb.z) & MASK_SE == 0
                && f(cur.x - 1, cur.z) & MASK_E == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        (1, 1) => {
            f(nb.x, nb.z) & MASK_SW == 0
                && f(cur.x + 1, cur.z) & MASK_W == 0
                && f(cur.x, cur.z + 1) & MASK_S == 0
        }
        _ => false,
    }
}

/// Walk-only dense Dijkstra matching router cost/tie-break/neighbor order.
/// Test-only; empty graph (no transports/teleports/essence).
fn dense_find_walk(
    oracle: &DenseOracle,
    from: WorldTile,
    to: WorldTile,
) -> Result<crate::router::Route, RouteError> {
    use crate::router::{CostModel, Leg, Route};
    use std::cmp::Ordering;
    use std::collections::{BinaryHeap, HashMap, HashSet};

    const STEPS: [(i32, i32); 8] = [
        (0, 1),
        (0, -1),
        (1, 0),
        (-1, 0),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (1, 1),
    ];
    const NODE_BUDGET: usize = 4_000_000;
    let model = CostModel::running();

    if from == to {
        return Ok(Route {
            legs: vec![Leg::Walk { tiles: vec![from] }],
            dest: to,
            ticks: 0.0,
        });
    }

    struct HeapNode {
        cost: f64,
        tile: WorldTile,
    }
    impl PartialEq for HeapNode {
        fn eq(&self, other: &Self) -> bool {
            self.cost == other.cost && self.tile == other.tile
        }
    }
    impl Eq for HeapNode {}
    impl PartialOrd for HeapNode {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for HeapNode {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .cost
                .total_cmp(&self.cost)
                .then_with(|| self.tile.x.cmp(&other.tile.x))
                .then_with(|| self.tile.z.cmp(&other.tile.z))
                .then_with(|| self.tile.level.cmp(&other.tile.level))
        }
    }

    let mut dist: HashMap<WorldTile, f64> = HashMap::new();
    let mut came: HashMap<WorldTile, WorldTile> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let mut done = HashSet::new();
    dist.insert(from, 0.0);
    heap.push(HeapNode {
        cost: 0.0,
        tile: from,
    });
    let mut expanded = 0usize;
    while let Some(n) = heap.pop() {
        let cur = n.tile;
        if dist.get(&cur) != Some(&n.cost) {
            continue;
        }
        if !done.insert(cur) {
            continue;
        }
        expanded += 1;
        if expanded > NODE_BUDGET {
            return Err(RouteError::BudgetExhausted);
        }
        if cur == to {
            // Reconstruct walk tiles.
            let mut walk_rev = vec![to];
            let mut t = to;
            while let Some(&prev) = came.get(&t) {
                walk_rev.push(prev);
                t = prev;
            }
            walk_rev.reverse();
            let ticks = walk_rev.len().saturating_sub(1) as f64 * model.run_per_step;
            return Ok(Route {
                legs: vec![Leg::Walk { tiles: walk_rev }],
                dest: to,
                ticks,
            });
        }
        for d in STEPS {
            if dense_step_ok(oracle, cur, d) {
                let nb = WorldTile {
                    x: cur.x + d.0,
                    z: cur.z + d.1,
                    level: cur.level,
                };
                let nd = n.cost + model.run_per_step;
                if !done.contains(&nb) && dist.get(&nb).is_none_or(|&g| g > nd) {
                    dist.insert(nb, nd);
                    came.insert(nb, cur);
                    heap.push(HeapNode { cost: nd, tile: nb });
                }
            }
        }
    }
    Err(RouteError::NoPath)
}

#[test]
fn bounded_route_and_corner_masks_match_dense_oracle() {
    // 5×5 open with a center blocked footprint and a face wall.
    let w = 5usize;
    let h = 5usize;
    let mut walk = vec![0u8; w * h];
    let mut blocked = vec![0u64; (w * h).div_ceil(64)];
    let center = 2 * w + 2;
    blocked[0] |= 1 << center;
    walk[2 * w + 1] = 1; // face wall on west of center
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let (oracle, tiled) = freeze(origin, w, h, walk, blocked, None);
    assert_all_cells(&oracle, &tiled);

    let dirs = [
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
    ];
    // Exhaust local 3×3 around center: dense oracle step vs tiled router step_ok.
    for z in 1..4 {
        for x in 1..4 {
            let cur = WorldTile { x, z, level: 0 };
            for d in dirs {
                assert_eq!(
                    dense_step_ok(&oracle, cur, d),
                    step_ok(&tiled, cur, d),
                    "dense vs tiled step_ok at {cur:?} {d:?}"
                );
            }
        }
    }
    // Also probe tile/plane edges of the 5×5.
    for z in 0..5 {
        for x in 0..5 {
            let cur = WorldTile { x, z, level: 0 };
            for d in dirs {
                assert_eq!(
                    dense_step_ok(&oracle, cur, d),
                    step_ok(&tiled, cur, d),
                    "edge step_ok {cur:?} {d:?}"
                );
            }
        }
    }

    // Exhaust all start/dest pairs: full Route equality vs independent dense find.
    let graph = TransportGraph::default();
    for sz in 0..5i32 {
        for sx in 0..5i32 {
            for dz in 0..5i32 {
                for dx in 0..5i32 {
                    let from = WorldTile {
                        x: sx,
                        z: sz,
                        level: 0,
                    };
                    let to = WorldTile {
                        x: dx,
                        z: dz,
                        level: 0,
                    };
                    let dense = dense_find_walk(&oracle, from, to);
                    let tiled_r = find(&tiled, &graph, from, to);
                    match (&dense, &tiled_r) {
                        (Ok(a), Ok(b)) => {
                            assert_eq!(a, b, "full route mismatch {from:?}->{to:?}");
                        }
                        (Err(RouteError::NoPath), Err(RouteError::NoPath)) => {}
                        (Err(RouteError::BudgetExhausted), Err(RouteError::BudgetExhausted)) => {}
                        _ => panic!("dense vs tiled route disagree {from:?}->{to:?}: {dense:?} vs {tiled_r:?}"),
                    }
                }
            }
        }
    }
}

#[test]
fn layout_accounting_sizes_are_exact_for_known_grids() {
    let (walk, blocked) = open_plane(64, 64, 1);
    let tiled = WorldCollision::from_packed_parts(
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        64,
        64,
        walk,
        blocked,
        None,
    )
    .unwrap();
    // 64×64 → 2×2 tiles, all uniform open → 0 dense.
    assert_eq!(tiled.directory_len(), 4);
    assert_eq!(tiled.dense_pool_len(), 0);
    assert_eq!(std::mem::size_of::<DenseTile>(), DENSE_TILE_BYTES);
    assert_eq!(std::mem::size_of::<WorldCollision>(), 120);
}

#[test]
fn coordinate_pair_read_matches_linear_pair_read() {
    let width = 33usize;
    let height = 17usize;
    let cells = width * height;
    let mut walk = vec![0u8; cells];
    let mut blocked = vec![0u64; cells.div_ceil(64)];
    let index = 16 * width + 32;
    walk[index] = 0xa5;
    blocked[index >> 6] |= 1u64 << (index & 63);
    let (_, tiled) = freeze(
        WorldTile {
            x: -40,
            z: 70,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        None,
    );

    assert_eq!(tiled.pair_at_coords(0, 32, 16), tiled.pair_at_index(index));
}

fn panic_payload(f: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("operation must panic");
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => panic!("panic payload was not a string"),
        },
    }
}

#[test]
fn missing_synthetic_plane_preserves_coordinate_api_panics() {
    let origin = WorldTile {
        x: -7,
        z: 11,
        level: 3,
    };
    let (walk, blocked) = open_plane(2, 2, 1);
    let (_, tiled) = freeze(origin, 2, 2, walk, blocked, None);
    let absent = WorldTile {
        x: origin.x,
        z: origin.z,
        level: 1,
    };
    let expected = "collision index 4 past logical length 4";

    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.walkable_word(absent.x, absent.z, absent.level));
        }),
        expected
    );
    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.walkable(absent));
        }),
        expected
    );
    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.standable(absent));
        }),
        expected
    );

    // Bounds and unknown-level rejection happen before the logical-length read.
    assert_eq!(tiled.walkable_word(origin.x - 1, origin.z, 1), 0);
    assert!(!tiled.walkable(WorldTile {
        x: origin.x,
        z: origin.z,
        level: 4
    }));
    assert!(!tiled.standable(WorldTile {
        x: origin.x,
        z: origin.z,
        level: -1
    }));
}

#[test]
fn short_raw_flags_keep_their_distinct_index_failure_order() {
    let origin = WorldTile {
        x: 20,
        z: -30,
        level: 2,
    };
    let (walk, blocked) = open_plane(2, 1, 4);
    let (_, tiled) = freeze(origin, 2, 1, walk, blocked, Some(vec![0]));
    let second = WorldTile {
        x: origin.x + 1,
        z: origin.z,
        level: 0,
    };
    let expected = "index out of bounds: the len is 1 but the index is 1";

    // walkable_word always ignores raw flags and reads the packed pair.
    assert_eq!(tiled.walkable_word(second.x, second.z, second.level), 0);
    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.flag(second.x, second.z, 0));
        }),
        expected
    );
    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.walkable(second));
        }),
        expected
    );
    assert_eq!(
        panic_payload(|| {
            std::hint::black_box(tiled.standable(second));
        }),
        expected
    );

    // Bounds and unknown levels still return before indexing the short sidecar.
    assert_eq!(tiled.flag(origin.x + 2, origin.z, 0), 0);
    assert!(!tiled.walkable(WorldTile {
        x: second.x,
        z: second.z,
        level: 4
    }));
}

#[test]
fn every_dense_face_blocked_pair_matches_all_coordinate_apis() {
    let width = 32usize;
    let height = 32usize;
    let mut walk = vec![0u8; width * height];
    let mut blocked = vec![0u64; walk.len().div_ceil(64)];
    for face in 0u16..=255 {
        for blocked_value in 0usize..=1 {
            let index = face as usize * 4 + blocked_value;
            walk[index] = face as u8;
            if blocked_value != 0 {
                blocked[index >> 6] |= 1u64 << (index & 63);
            }
        }
    }
    let origin = WorldTile {
        x: -400,
        z: 900,
        level: 3,
    };
    let (oracle, tiled) = freeze(origin, width, height, walk, blocked, None);
    assert_eq!(tiled.dense_pool_len(), 1);
    for face in 0u16..=255 {
        for blocked_value in 0usize..=1 {
            let index = face as usize * 4 + blocked_value;
            let x = origin.x + (index % width) as i32;
            let z = origin.z + (index / width) as i32;
            let tile = WorldTile { x, z, level: 0 };
            assert_eq!(tiled.packed_pair_at(index), oracle.pair_at(index));
            assert_eq!(tiled.walkable_word(x, z, 0), oracle.walkable_word(x, z, 0));
            assert_eq!(tiled.walkable(tile), oracle.walkable(tile));
            assert_eq!(tiled.standable(tile), oracle.standable(tile));
        }
    }
}

#[test]
fn one_through_four_planes_preserve_absolute_level_selection() {
    for planes in 1usize..=4 {
        let width = 33usize;
        let height = 31usize;
        let cells = width * height * planes;
        let mut walk = vec![0u8; cells];
        let mut blocked = vec![0u64; cells.div_ceil(64)];
        for plane in 0..planes {
            let index = plane * width * height + 30 * width + 32;
            walk[index] = (plane as u8).wrapping_mul(61).wrapping_add(7);
            if plane & 1 != 0 {
                blocked[index >> 6] |= 1u64 << (index & 63);
            }
        }
        let origin = WorldTile {
            x: 55,
            z: -89,
            level: 3,
        };
        let (oracle, tiled) = freeze(origin, width, height, walk, blocked, None);
        for level in 0..planes as i32 {
            assert_eq!(
                tiled.walkable_word(origin.x + 32, origin.z + 30, level),
                oracle.walkable_word(origin.x + 32, origin.z + 30, level)
            );
        }
    }
}

fn generated_read_sample(
    collision: &WorldCollision,
    probes: &[(i32, i32, i32)],
    passes: usize,
) -> (u128, u64) {
    let start = std::time::Instant::now();
    let mut checksum = 0u64;
    for pass in 0..passes {
        for &(x, z, level) in probes {
            let word = std::hint::black_box(collision.walkable_word(x, z, level));
            let walkable = std::hint::black_box(collision.walkable(WorldTile { x, z, level }));
            let standable = std::hint::black_box(collision.standable(WorldTile { x, z, level }));
            checksum = checksum
                .rotate_left(7)
                .wrapping_add(word as u64)
                .wrapping_add((walkable as u64) << 32)
                .wrapping_add((standable as u64) << 40)
                .wrapping_add(pass as u64);
        }
    }
    (start.elapsed().as_nanos(), std::hint::black_box(checksum))
}

fn generated_route_sample(
    collision: &WorldCollision,
    routes: &[(WorldTile, WorldTile)],
) -> (u128, u64) {
    let graph = TransportGraph::default();
    let start = std::time::Instant::now();
    let mut checksum = 0u64;
    for &(from, to) in routes {
        let route = find(collision, &graph, from, to).expect("generated open route");
        checksum = checksum
            .wrapping_mul(0x9e37_79b9)
            .wrapping_add(route.dest.x as u64)
            .wrapping_add((route.dest.z as u64).rotate_left(13))
            .wrapping_add(route.ticks.to_bits());
        for leg in route.legs {
            match leg {
                crate::router::Leg::Walk { tiles } => {
                    checksum = checksum.wrapping_add(tiles.len() as u64);
                    for tile in tiles {
                        checksum = checksum
                            .rotate_left(3)
                            .wrapping_add(tile.x as u64)
                            .wrapping_add((tile.z as u64).rotate_left(17))
                            .wrapping_add((tile.level as u64).rotate_left(29));
                    }
                }
                crate::router::Leg::Transport { .. } => panic!("generated graph is empty"),
            }
        }
    }
    (start.elapsed().as_nanos(), std::hint::black_box(checksum))
}

#[test]
#[ignore = "bounded release-only coordinate lookup diagnostic"]
fn generated_coordinate_lookup_benchmark() {
    const READ_WIDTH: usize = 257;
    const READ_HEIGHT: usize = 193;
    const READ_PLANES: usize = 4;
    const PROBES: usize = 65_536;
    const READ_PASSES: usize = 24;
    const ROUTE_WIDTH: usize = 96;
    const ROUTE_HEIGHT: usize = 96;
    const SAMPLES: usize = 7;

    let read_cells = READ_WIDTH * READ_HEIGHT * READ_PLANES;
    let mut walk = vec![0u8; read_cells];
    let mut blocked = vec![0u64; read_cells.div_ceil(64)];
    for (index, face) in walk.iter_mut().enumerate() {
        *face = index.wrapping_mul(37).wrapping_add(index / READ_WIDTH) as u8;
        if index.wrapping_mul(13).wrapping_add(17) % 11 == 0 {
            blocked[index >> 6] |= 1u64 << (index & 63);
        }
    }
    let read_origin = WorldTile {
        x: -1_000,
        z: 2_000,
        level: 3,
    };
    let (_, read_world) = freeze(read_origin, READ_WIDTH, READ_HEIGHT, walk, blocked, None);
    let mut probes = Vec::with_capacity(PROBES);
    let mut state = 0x4d59_5df4_d0f3_3173u64;
    for _ in 0..PROBES {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let x = (state as usize) % READ_WIDTH;
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let z = (state as usize) % READ_HEIGHT;
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let level = (state as usize) % READ_PLANES;
        probes.push((
            read_origin.x + x as i32,
            read_origin.z + z as i32,
            level as i32,
        ));
    }

    let route_origin = WorldTile {
        x: 1_000,
        z: 1_000,
        level: 2,
    };
    let (route_walk, route_blocked) = open_plane(ROUTE_WIDTH, ROUTE_HEIGHT, 1);
    let (_, route_world) = freeze(
        route_origin,
        ROUTE_WIDTH,
        ROUTE_HEIGHT,
        route_walk,
        route_blocked,
        None,
    );
    let tile = |x: i32, z: i32| WorldTile {
        x: route_origin.x + x,
        z: route_origin.z + z,
        level: 0,
    };
    let routes = [
        (tile(1, 1), tile(94, 94)),
        (tile(94, 1), tile(1, 94)),
        (tile(1, 48), tile(94, 48)),
        (tile(48, 1), tile(48, 94)),
        (tile(10, 10), tile(85, 70)),
        (tile(85, 15), tile(12, 80)),
        (tile(5, 90), tile(90, 5)),
        (tile(20, 75), tile(75, 20)),
    ];

    let (_, warm_read_checksum) = generated_read_sample(&read_world, &probes, 2);
    let (_, warm_route_checksum) = generated_route_sample(&route_world, &routes[..2]);
    let mut read_ns = Vec::with_capacity(SAMPLES);
    let mut route_ns = Vec::with_capacity(SAMPLES);
    let mut read_checksum = 0;
    let mut route_checksum = 0;
    for _ in 0..SAMPLES {
        let (elapsed, checksum) = generated_read_sample(&read_world, &probes, READ_PASSES);
        read_ns.push(elapsed);
        read_checksum = checksum;
        let (elapsed, checksum) = generated_route_sample(&route_world, &routes);
        route_ns.push(elapsed);
        route_checksum = checksum;
    }

    let arm = std::env::var("NAV_COORD_ARM").unwrap_or_else(|_| "unspecified".to_owned());
    println!(
        "NAV_COORD_BENCH {{\"schema\":\"nav-coordinate-lookup-v1\",\"arm\":\"{}\",\"read\":{{\"width\":{},\"height\":{},\"planes\":{},\"probes\":{},\"passes\":{},\"samples_ns\":{:?},\"checksum\":{},\"warm_checksum\":{}}},\"route\":{{\"width\":{},\"height\":{},\"routes\":{},\"samples_ns\":{:?},\"checksum\":{},\"warm_checksum\":{}}},\"layout\":{{\"world_size\":{},\"dense_tile_size\":{},\"read_directory\":{},\"read_dense\":{},\"route_directory\":{},\"route_dense\":{}}}}}",
        arm,
        READ_WIDTH,
        READ_HEIGHT,
        READ_PLANES,
        PROBES,
        READ_PASSES,
        read_ns,
        read_checksum,
        warm_read_checksum,
        ROUTE_WIDTH,
        ROUTE_HEIGHT,
        routes.len(),
        route_ns,
        route_checksum,
        warm_route_checksum,
        std::mem::size_of::<WorldCollision>(),
        std::mem::size_of::<DenseTile>(),
        read_world.directory_len(),
        read_world.dense_pool_len(),
        route_world.directory_len(),
        route_world.dense_pool_len(),
    );
}

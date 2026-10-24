use std::io::Cursor;

use api::snapshot::WorldTile;

use crate::collision::{pack_walk, walk_word_from_parts, SQ_BLOCKED};
use crate::pack::{
    decode, decode_flags_sidecar, decode_grid, decode_reach_sidecar, read_takeoff, PackError,
};

#[test]
fn decoder_rejects_stale_pack_version_before_reading_body() {
    assert!(matches!(decode(b"274V\x10"), Err(PackError::BadVersion(16))));
}

#[test]
fn grid_decoder_rejects_bad_magic_and_zero_dimensions() {
    assert!(matches!(decode_grid(b"XXXX"), Err(PackError::BadMagic)));

    let mut bytes = b"274N\x01".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    assert!(matches!(decode_grid(&bytes), Err(PackError::BadLength(_))));
}

#[test]
fn grid_decoder_rejects_truncated_walk_payload() {
    let mut bytes = b"274N\x01".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    assert!(matches!(decode_grid(&bytes), Err(PackError::Truncated)));
}

#[test]
fn flags_decoder_rejects_partial_words() {
    let mut bytes = b"274F\x01".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.push(0x7f);
    assert!(matches!(decode_flags_sidecar(&bytes), Err(PackError::Truncated)));
}

#[test]
fn reach_decoder_rejects_incomplete_word_payload() {
    let mut bytes = b"274R\x01".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 32]);
    bytes.extend_from_slice(&[0; 7]);
    assert!(matches!(decode_reach_sidecar(&bytes), Err(PackError::Truncated)));
}

#[test]
fn malformed_takeoff_tag_is_rejected_by_runtime_decoder_helper() {
    let mut bytes = vec![2];
    bytes.extend_from_slice(&[0; 12]);
    assert!(matches!(
        read_takeoff(
            &mut Cursor::new(bytes),
            WorldTile {
                x: 1,
                z: 2,
                level: 0,
            },
        ),
        Err(PackError::BadLength(_))
    ));
}

#[test]
fn packed_walk_roundtrips_blocked_base() {
    let flags = [0, SQ_BLOCKED, 0, SQ_BLOCKED | 0x10];
    let (faces, blocked) = pack_walk(&flags);
    assert_eq!(faces, [0, 0, 0, 0x10]);
    assert_eq!(blocked, [0b1010]);
    assert_eq!(walk_word_from_parts(faces[1], true) & SQ_BLOCKED, SQ_BLOCKED);
    assert_eq!(walk_word_from_parts(faces[3], false), 0x10);
}

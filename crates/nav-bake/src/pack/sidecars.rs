use api::snapshot::WorldTile;
use nav_planner::pack::{
    MAGIC_CANLIGHT, MAGIC_FLAGS, MAGIC_REACH, VERSION_CANLIGHT, VERSION_FLAGS, VERSION_REACH,
};

/// Serialize the raw baked flags to the sidecar byte format: magic
/// `b"274F"`, version 1, the same origin/width/height header as the pack,
/// then
/// the level-major u32le flags. The flag count is implicit — the trailing
/// bytes are the flags, so a `width × height` test grid round-trips
/// without plane arithmetic (the decoder reads the flags to the end).
pub fn encode_flags_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    flags: &[u32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 12 + 8 + flags.len() * 4);
    out.extend_from_slice(MAGIC_FLAGS);
    out.push(VERSION_FLAGS);
    for v in [origin.x, origin.z, origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    for f in flags {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Serialize a paint-reach bitset: magic `b"274R"`, version 1, the same
/// origin/width/height header as the pack, explicit word count, the 32-byte
/// pack SHA-256 binding, then `word_count` little-endian `u64` words.
pub fn encode_reach_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    encode_bitset_sidecar(
        MAGIC_REACH,
        VERSION_REACH,
        origin,
        width,
        height,
        bits,
        binding,
    )
}

/// Serialize a static canlight bitset: magic `b"274L"`, version 1, the same
/// header dialect as [`encode_reach_sidecar`].
pub fn encode_canlight_sidecar(
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    encode_bitset_sidecar(
        MAGIC_CANLIGHT,
        VERSION_CANLIGHT,
        origin,
        width,
        height,
        bits,
        binding,
    )
}

pub(super) fn encode_bitset_sidecar(
    magic: &[u8; 4],
    version: u8,
    origin: WorldTile,
    width: usize,
    height: usize,
    bits: &[u64],
    binding: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 12 + 8 + 4 + 32 + bits.len() * 8);
    out.extend_from_slice(magic);
    out.push(version);
    for v in [origin.x, origin.z, origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    out.extend_from_slice(&(bits.len() as u32).to_le_bytes());
    out.extend_from_slice(binding);
    for word in bits {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

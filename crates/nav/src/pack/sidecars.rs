use super::*;

/// Flags sidecar format version.
pub(super) const VERSION_FLAGS: u8 = 1;
/// Flags sidecar magic.
pub(super) const MAGIC_FLAGS: &[u8; 4] = b"274F";
/// Paint-reach sidecar format version.
pub(super) const VERSION_REACH: u8 = 1;
/// Paint-reach sidecar magic.
pub(super) const MAGIC_REACH: &[u8; 4] = b"274R";
/// Static canlight sidecar format version.
pub(super) const VERSION_CANLIGHT: u8 = 1;
/// Static canlight sidecar magic.
pub(super) const MAGIC_CANLIGHT: &[u8; 4] = b"274L";

/// Serialize the raw baked flags to the sidecar byte format: magic
/// `b"274F"`, version 1, the same origin/width/height header as the pack,
/// then
/// the level-major u32le flags. The flag count is implicit — the trailing
/// bytes are the flags, so a `width × height` test grid round-trips
/// without plane arithmetic ([`decode_flags_sidecar`] reads to the end).
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

/// Deserialize a flags sidecar, validating magic, version, and the grid
/// header, then reading the trailing u32le flags to the end of the
/// buffer (a partial trailing u32 is [`PackError::Truncated`]).
///
/// After the header is validated, the already length-checked trailing
/// payload is bulk-converted with little-endian `u32` interpretation —
/// not a per-word [`Cursor`] `read_exact` loop — so large sidecars do not
/// stall startup on scalar decode overhead.
pub fn decode_flags_sidecar(
    bytes: &[u8],
) -> Result<(WorldTile, usize, usize, Vec<u32>), PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != MAGIC_FLAGS {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != VERSION_FLAGS {
        return Err(PackError::BadVersion(version[0]));
    }
    let origin = WorldTile {
        x: read_i32(&mut r)?,
        z: read_i32(&mut r)?,
        level: read_i32(&mut r)?,
    };
    let width = read_u32(&mut r)? as usize;
    let height = read_u32(&mut r)? as usize;
    if width == 0 || height == 0 || width > MAX_GRID || height > MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {MAX_GRID} tile cap"
        )));
    }
    let payload = &bytes[r.position() as usize..];
    if !payload.len().is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    Ok((origin, width, height, decode_u32le_words(payload)))
}

/// Bulk little-endian `u32` words from a length-checked payload (`len % 4 == 0`).
pub(super) fn decode_u32le_words(payload: &[u8]) -> Vec<u32> {
    debug_assert!(payload.len().is_multiple_of(4));
    let n = payload.len() / 4;
    let mut flags = Vec::with_capacity(n);
    for chunk in payload.as_chunks::<4>().0 {
        // as_chunks guarantees 4 bytes; avoid Cursor/read_exact per word.
        flags.push(u32::from_le_bytes(*chunk));
    }
    flags
}

/// Decoded paint-reach sidecar: geometry, word count, pack-identity binding
/// and the `bake_reach` bitset. Geometry alone is not identity — the binding
/// is the SHA-256 of the pack that produced the bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachSidecar {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    pub word_count: usize,
    pub binding: [u8; 32],
    pub bits: Vec<u64>,
}

/// Hex form of a 32-byte SHA-256 (lowercase), matching [`crate::manifest::hash_bytes`].
pub fn sha256_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Parse a 64-char lowercase-or-mixed SHA-256 hex digest into 32 bytes.
pub fn sha256_from_hex(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("is not a SHA-256 hex digest".into());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(chunk).map_err(|_| "is not a SHA-256 hex digest")?;
        out[i] = u8::from_str_radix(text, 16).map_err(|_| "is not a SHA-256 hex digest")?;
    }
    Ok(out)
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

/// Deserialize a paint-reach sidecar, validating magic, version, grid
/// header, word count and that the trailing payload is exactly that many
/// `u64le` words. Binding bytes are not interpreted here — the caller
/// compares them to the pack identity.
pub fn decode_reach_sidecar(bytes: &[u8]) -> Result<ReachSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_REACH, VERSION_REACH)
}

/// Decoded static canlight sidecar. Same geometry as [`ReachSidecar`]; the
/// binding is pack+policy identity, not pack SHA alone.
pub type CanlightSidecar = ReachSidecar;

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

/// Deserialize a static canlight sidecar. Binding bytes are not interpreted
/// here — the caller compares them to pack+policy identity.
pub fn decode_canlight_sidecar(bytes: &[u8]) -> Result<CanlightSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_CANLIGHT, VERSION_CANLIGHT)
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

pub(super) fn decode_bitset_sidecar(
    bytes: &[u8],
    expected_magic: &[u8; 4],
    expected_version: u8,
) -> Result<ReachSidecar, PackError> {
    let mut r = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| PackError::Truncated)?;
    if &magic != expected_magic {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 1];
    r.read_exact(&mut version)
        .map_err(|_| PackError::Truncated)?;
    if version[0] != expected_version {
        return Err(PackError::BadVersion(version[0]));
    }
    let origin = WorldTile {
        x: read_i32(&mut r)?,
        z: read_i32(&mut r)?,
        level: read_i32(&mut r)?,
    };
    let width = read_u32(&mut r)? as usize;
    let height = read_u32(&mut r)? as usize;
    if width == 0 || height == 0 || width > MAX_GRID || height > MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {MAX_GRID} tile cap"
        )));
    }
    let word_count = read_u32(&mut r)? as usize;
    let mut binding = [0u8; 32];
    r.read_exact(&mut binding)
        .map_err(|_| PackError::Truncated)?;
    let payload = &bytes[r.position() as usize..];
    if payload.len() != word_count.saturating_mul(8) {
        return Err(PackError::Truncated);
    }
    Ok(ReachSidecar {
        origin,
        width,
        height,
        word_count,
        binding,
        bits: decode_u64le_words(payload),
    })
}

/// Bulk little-endian `u64` words from a length-checked payload (`len % 8 == 0`).
pub(super) fn decode_u64le_words(payload: &[u8]) -> Vec<u64> {
    debug_assert!(payload.len().is_multiple_of(8));
    payload
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect()
}

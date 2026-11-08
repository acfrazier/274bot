use super::*;
use std::fs;
use std::sync::Arc;

/// On-disk flags header length: magic(4) + version(1) + origin(12) + width/height(8).
pub const FLAGS_HEADER_LEN: usize = 4 + 1 + 12 + 8;

/// Streaming read chunk for [`read_flags_sidecar`] — keeps the transient
/// byte buffer far smaller than the decoded payload. Heap-backed so the
/// 1 MiB slot thread is not charged a 64 KiB stack frame.
const FLAGS_READ_CHUNK: usize = 64 * 1024;

/// Flags sidecar format version.
pub const VERSION_FLAGS: u8 = 1;
/// Flags sidecar magic.
pub const MAGIC_FLAGS: &[u8; 4] = b"274F";
/// Paint-reach sidecar format version.
pub const VERSION_REACH: u8 = 2;
/// Paint-reach sidecar magic.
pub const MAGIC_REACH: &[u8; 4] = b"274R";
/// Static canlight sidecar format version.
pub const VERSION_CANLIGHT: u8 = 2;
/// Static canlight sidecar magic.
pub const MAGIC_CANLIGHT: &[u8; 4] = b"274L";

pub fn decode_flags_sidecar(
    bytes: &[u8],
) -> Result<(WorldTile, usize, usize, Vec<u32>), PackError> {
    let (origin, width, height, payload) = flags_header_and_payload(bytes)?;
    Ok((origin, width, height, decode_u32le_words(payload)))
}

/// Like [`decode_flags_sidecar`], but the trailing words land directly in
/// one `Arc<Vec<u32>>` so the payload is not copied into an `Arc<[u32]>`
/// layout. Prefer [`read_flags_sidecar`] when loading from disk so the raw
/// file buffer is never retained beside the decoded payload.
#[expect(
    clippy::type_complexity,
    reason = "Returns grid geometry with the shared flags payload, without copying that payload."
)]
pub fn decode_flags_sidecar_arc(
    bytes: &[u8],
) -> Result<(WorldTile, usize, usize, Arc<Vec<u32>>), PackError> {
    let (origin, width, height, payload) = flags_header_and_payload(bytes)?;
    Ok((origin, width, height, Arc::new(decode_u32le_words(payload))))
}

/// Stream a flags sidecar from disk into one resident `Arc<Vec<u32>>` payload.
///
/// Peak owned memory is the decoded word buffer (plus a small read chunk and
/// the final Arc), not a full-file `Vec<u8>` living beside a decoded `Vec<u32>`
/// and a third Arc copy.
pub fn read_flags_sidecar(path: &Path) -> Result<FlagsSidecarLoad, PackError> {
    let mut file = fs::File::open(path).map_err(PackError::Io)?;
    let file_len = file.metadata().map_err(PackError::Io)?.len() as usize;
    if file_len < FLAGS_HEADER_LEN {
        return Err(PackError::Truncated);
    }
    let mut header = [0u8; FLAGS_HEADER_LEN];
    file.read_exact(&mut header)
        .map_err(|_| PackError::Truncated)?;
    let (origin, width, height) = parse_flags_header(&header)?;
    let payload_len = file_len - FLAGS_HEADER_LEN;
    if !payload_len.is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    let word_count = payload_len / 4;

    // One payload-sized allocation: stream LE bytes straight into u32 words.
    let mut words = Vec::with_capacity(word_count);
    // Heap chunk: slot threads only have a 1 MiB stack.
    let mut chunk = vec![0u8; FLAGS_READ_CHUNK];
    let mut remaining = payload_len;
    while remaining > 0 {
        let n = remaining.min(FLAGS_READ_CHUNK);
        file.read_exact(&mut chunk[..n])
            .map_err(|_| PackError::Truncated)?;
        for word in chunk[..n].as_chunks::<4>().0 {
            words.push(u32::from_le_bytes(*word));
        }
        remaining -= n;
    }
    debug_assert_eq!(words.len(), word_count);
    // Wrap the sole payload buffer; Arc owns the Vec header and shares the
    // existing word allocation (no second payload-sized copy into Arc<[u32]>).
    let flags = Arc::new(words);
    Ok(FlagsSidecarLoad {
        origin,
        width,
        height,
        flags,
    })
}

/// Decoded flags sidecar loaded from disk (see [`read_flags_sidecar`]).
#[derive(Debug, Clone)]
pub struct FlagsSidecarLoad {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    /// Single resident payload; Arc shares the Vec without re-copying words.
    pub flags: Arc<Vec<u32>>,
}

fn flags_header_and_payload(bytes: &[u8]) -> Result<(WorldTile, usize, usize, &[u8]), PackError> {
    // Magic is the first four bytes: a wrong magic on a short buffer is still
    // BadMagic (not Truncated), matching the historical Cursor decode order.
    if bytes.len() < 4 {
        return Err(PackError::Truncated);
    }
    if &bytes[..4] != MAGIC_FLAGS {
        return Err(PackError::BadMagic);
    }
    if bytes.len() < FLAGS_HEADER_LEN {
        return Err(PackError::Truncated);
    }
    let (origin, width, height) = parse_flags_header(&bytes[..FLAGS_HEADER_LEN])?;
    let payload = &bytes[FLAGS_HEADER_LEN..];
    if !payload.len().is_multiple_of(4) {
        return Err(PackError::Truncated);
    }
    Ok((origin, width, height, payload))
}

fn parse_flags_header(header: &[u8]) -> Result<(WorldTile, usize, usize), PackError> {
    debug_assert!(header.len() >= FLAGS_HEADER_LEN);
    let mut r = Cursor::new(header);
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
    Ok((origin, width, height))
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

/// Decoded paint-reach sidecar: geometry, word count and the `bake_reach` bitset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachSidecar {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    pub word_count: usize,
    pub bits: Vec<u64>,
}

/// Streaming sidecar result whose words occupy the final shared allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachSidecarLoad {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    pub word_count: usize,
    pub bits: Arc<[u64]>,
}

/// Validated bitset geometry and word count, without retaining payload words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachSidecarHeader {
    pub origin: WorldTile,
    pub width: usize,
    pub height: usize,
    pub word_count: usize,
}

/// Static canlight uses the same streaming result layout as paint reach.
pub type CanlightSidecarLoad = ReachSidecarLoad;

/// Hex form of a 32-byte SHA-256 digest.
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

/// Deserialize a paint-reach sidecar, validating magic, version, grid
/// header, word count and that the trailing payload is exactly that many
/// `u64le` words.
pub fn decode_reach_sidecar(bytes: &[u8]) -> Result<ReachSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_REACH, VERSION_REACH)
}

/// Stream a paint-reach sidecar into its final `Arc<[u64]>` allocation.
/// `length` bounds reads and must be the opened file's byte length.
pub fn read_reach_sidecar(
    reader: &mut impl BufRead,
    length: usize,
) -> Result<ReachSidecarLoad, PackError> {
    read_bitset_sidecar(reader, length, MAGIC_REACH, VERSION_REACH)
}

/// Read and validate paint-reach geometry and payload length, leaving the
/// reader at the payload for a later lazy load.
pub fn read_reach_sidecar_header(
    reader: &mut impl BufRead,
    length: usize,
) -> Result<ReachSidecarHeader, PackError> {
    let mut r = BoundedReader::new(reader, length);
    let (origin, width, height, word_count) =
        read_bitset_header(&mut r, MAGIC_REACH, VERSION_REACH)?;
    if r.remaining() != word_count.checked_mul(8).ok_or(PackError::Truncated)? {
        return Err(PackError::Truncated);
    }
    Ok(ReachSidecarHeader {
        origin,
        width,
        height,
        word_count,
    })
}

/// Decoded static canlight sidecar. Same geometry as [`ReachSidecar`].
pub type CanlightSidecar = ReachSidecar;

/// Deserialize a static canlight sidecar using its ordinary format checks.
pub fn decode_canlight_sidecar(bytes: &[u8]) -> Result<CanlightSidecar, PackError> {
    decode_bitset_sidecar(bytes, MAGIC_CANLIGHT, VERSION_CANLIGHT)
}

/// Stream a static canlight sidecar into its final `Arc<[u64]>` allocation.
pub fn read_canlight_sidecar(
    reader: &mut impl BufRead,
    length: usize,
) -> Result<CanlightSidecarLoad, PackError> {
    read_bitset_sidecar(reader, length, MAGIC_CANLIGHT, VERSION_CANLIGHT)
}

pub(super) fn decode_bitset_sidecar(
    bytes: &[u8],
    expected_magic: &[u8; 4],
    expected_version: u8,
) -> Result<ReachSidecar, PackError> {
    let mut r = Cursor::new(bytes);
    let (origin, width, height, word_count) =
        read_bitset_header(&mut r, expected_magic, expected_version)?;
    let payload = &bytes[r.position() as usize..];
    let payload_len = word_count.checked_mul(8).ok_or(PackError::Truncated)?;
    if payload.len() != payload_len {
        return Err(PackError::Truncated);
    }
    Ok(ReachSidecar {
        origin,
        width,
        height,
        word_count,
        bits: decode_u64le_words(payload),
    })
}

fn read_bitset_sidecar(
    reader: &mut impl BufRead,
    length: usize,
    expected_magic: &[u8; 4],
    expected_version: u8,
) -> Result<ReachSidecarLoad, PackError> {
    let mut r = BoundedReader::new(reader, length);
    let (origin, width, height, word_count) =
        read_bitset_header(&mut r, expected_magic, expected_version)?;
    let payload_len = word_count.checked_mul(8).ok_or(PackError::Truncated)?;
    if r.remaining() != payload_len {
        return Err(PackError::Truncated);
    }

    // One exact-size allocation (a TrustedLen collect), filled in place.
    let mut bits: Arc<[u64]> = std::iter::repeat_n(0u64, word_count).collect();
    {
        let slots = Arc::get_mut(&mut bits).expect("new Arc is uniquely owned");
        let mut chunk = [0u8; 4096];
        let mut remaining = payload_len;
        let mut word_index = 0;
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            r.read_bytes_exact(&mut chunk[..n])?;
            for word in chunk[..n].as_chunks::<8>().0 {
                slots[word_index] = u64::from_le_bytes(*word);
                word_index += 1;
            }
            remaining -= n;
        }
        debug_assert_eq!(word_index, word_count);
    }
    Ok(ReachSidecarLoad {
        origin,
        width,
        height,
        word_count,
        bits,
    })
}

fn read_bitset_header<R: PackRead>(
    r: &mut R,
    expected_magic: &[u8; 4],
    expected_version: u8,
) -> Result<(WorldTile, usize, usize, usize), PackError> {
    let mut magic = [0u8; 4];
    r.read_bytes_exact(&mut magic)?;
    if &magic != expected_magic {
        return Err(PackError::BadMagic);
    }
    let version = read_u8(r)?;
    if version != expected_version {
        return Err(PackError::BadVersion(version));
    }
    let origin = WorldTile {
        x: read_i32(r)?,
        z: read_i32(r)?,
        level: read_i32(r)?,
    };
    let width = read_u32(r)? as usize;
    let height = read_u32(r)? as usize;
    if width == 0 || height == 0 || width > MAX_GRID || height > MAX_GRID {
        return Err(PackError::BadLength(format!(
            "grid {width}x{height} exceeds the {MAX_GRID} tile cap"
        )));
    }
    let word_count = read_u32(r)? as usize;
    Ok((origin, width, height, word_count))
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

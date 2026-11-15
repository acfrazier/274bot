//! Embedded game-data identities baked by the API build script.

use serde::Serialize;

/// Identity of one exact uncompressed core game-data pack embedded in this build.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GameDataPackIdentity {
    /// Client/cache revision selected by this pack.
    pub revision: i32,
    /// Stable API-crate-relative path of the source core JSON file.
    pub path: &'static str,
    /// Lowercase SHA-256 digest of the exact uncompressed source bytes.
    pub sha256: &'static str,
    /// Number of uncompressed source bytes.
    pub bytes: u64,
}

const GAME_DATA_PACKS: &[GameDataPackIdentity] =
    include!(concat!(env!("OUT_DIR"), "/game_data_pack_identities.rs"));

/// Return the baked identity for an embedded core pack, if this build includes it.
pub fn game_data_pack(revision: i32) -> Option<GameDataPackIdentity> {
    GAME_DATA_PACKS
        .iter()
        .find(|identity| identity.revision == revision)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::game_data_pack;
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::Path;

    fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
        const HEX: &[u8; 16] = b"0123456789abcdef";

        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 16 * 1024];
        let mut bytes = 0_u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            bytes += count as u64;
        }

        let mut digest = String::with_capacity(64);
        for byte in hasher.finalize() {
            digest.push(HEX[(byte >> 4) as usize] as char);
            digest.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Ok((digest, bytes))
    }

    #[test]
    fn embedded_pack_digest_matches_available_core_source() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for revision in [274, 289, 377] {
            let Some(identity) = game_data_pack(revision) else {
                continue;
            };
            let (digest, bytes) = sha256_file(&manifest_dir.join(identity.path))
                .expect("an embedded pack must have its core source file");
            assert_eq!(identity.sha256, digest);
            assert_eq!(identity.bytes, bytes);
        }
    }

    #[test]
    fn unknown_revision_has_no_embedded_pack_identity() {
        assert!(game_data_pack(-1).is_none());
    }
}

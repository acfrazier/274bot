use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const HASH_CHUNK_SIZE: usize = 1024 * 1024;

/// Cache archives that define one immutable client content identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CacheManifest {
    pub revision: u16,
    pub archives: BTreeMap<String, String>,
}

impl CacheManifest {
    pub const ARCHIVES: [&'static str; 8] = [
        "title",
        "config",
        "interface",
        "media",
        "versionlist",
        "textures",
        "wordenc",
        "sounds",
    ];

    /// Capture the checked archive hashes for an explicitly selected revision.
    pub fn capture(revision: u16, cache_dir: &Path) -> Result<Self, String> {
        Self::capture_with_progress(revision, cache_dir, |_, _| {})
    }

    /// Capture archive hashes while reporting only completed archive checks.
    pub fn capture_with_progress(
        revision: u16,
        cache_dir: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<Self, String> {
        validate_revision(revision)?;
        let mut archives = BTreeMap::new();
        let total = Self::ARCHIVES.len() as u64;
        progress(0, total);
        for (index, name) in Self::ARCHIVES.into_iter().enumerate() {
            archives.insert(name.into(), hash_file(&cache_dir.join(name))?);
            progress(index as u64 + 1, total);
        }
        Ok(Self { revision, archives })
    }

    /// Verify both the asserted revision and all archive bytes.
    pub fn verify(&self, revision: u16, cache_dir: &Path) -> Result<(), String> {
        validate_revision(revision)?;
        if self.revision != revision {
            return Err(format!(
                "cache manifest revision {} does not match selected revision {revision}",
                self.revision
            ));
        }
        let actual = Self::capture(revision, cache_dir)?;
        if actual != *self {
            return Err(format!(
                "cache manifest does not match cache bytes at {}",
                cache_dir.display()
            ));
        }
        Ok(())
    }

    /// Stable cache identity: revision plus sorted archive name/hash pairs.
    pub fn identity(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(self.revision.to_be_bytes());
        for (name, hash) in &self.archives {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(hash.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }
}

/// Sidecar manifest binding a v8 navigation pack to world/cache inputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NavManifest {
    pub revision: u16,
    pub cache_id: String,
    pub nav_sha256: String,
    pub flags_sha256: Option<String>,
    #[serde(default)]
    pub reach_sha256: Option<String>,
    #[serde(default)]
    pub canlight_sha256: Option<String>,
    #[serde(default)]
    pub pois_sha256: Option<String>,
    /// Decoded (`274DCI01`) identity of the cache the pack was baked from.
    /// Legacy sidecars omit it; `verify_pack` refuses it when the caller
    /// asserts a decoded id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    /// Digest of the actual server content/config consumed by the baker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
}

impl NavManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        revision: u16,
        cache: &CacheManifest,
        nav: &[u8],
        flags: Option<&[u8]>,
        reach: Option<&[u8]>,
        canlight: Option<&[u8]>,
        pois: Option<&[u8]>,
    ) -> Result<Self, String> {
        validate_revision(revision)?;
        if cache.revision != revision {
            return Err(format!(
                "cache manifest revision {} does not match selected revision {revision}",
                cache.revision
            ));
        }
        Ok(Self {
            revision,
            cache_id: cache.identity(),
            nav_sha256: hash_bytes(nav),
            flags_sha256: flags.map(hash_bytes),
            reach_sha256: reach.map(hash_bytes),
            canlight_sha256: canlight.map(hash_bytes),
            pois_sha256: pois.map(hash_bytes),
            content_id: None,
            source_sha256: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        &self,
        revision: u16,
        cache: &CacheManifest,
        nav: &[u8],
        flags: Option<&[u8]>,
        reach: Option<&[u8]>,
        canlight: Option<&[u8]>,
        pois: Option<&[u8]>,
    ) -> Result<(), String> {
        let actual = Self::capture(revision, cache, nav, flags, reach, canlight, pois)?;
        if actual != *self {
            return Err(
                "navigation/profile mismatch: revision, cache identity or pack/flags content differs"
                    .into(),
            );
        }
        Ok(())
    }

    /// Compare revision, cache identity and pack digest only. Flags stay
    /// optional and are checked later at paint-on against `flags_sha256`.
    /// `content_id` is the decoded identity the running cache must present:
    /// when the caller asserts one, a legacy sidecar without it is refused
    /// (no packed-hash alias) and any differing value fails.
    pub fn verify_pack(
        &self,
        revision: u16,
        cache: &CacheManifest,
        nav_sha256: &str,
        content_id: Option<&str>,
    ) -> Result<(), String> {
        validate_revision(revision)?;
        if self.revision != revision
            || cache.revision != revision
            || (content_id.is_none() && self.cache_id != cache.identity())
            || self.nav_sha256 != nav_sha256
        {
            return Err(
                "navigation/profile mismatch: revision, cache identity or pack/flags content differs"
                    .into(),
            );
        }
        match (content_id, &self.content_id) {
            (None, _) => {}
            (Some(_), None) => {
                return Err(
                    "navigation/profile mismatch: pack has no decoded content identity provenance"
                        .into(),
                )
            }
            (Some(wanted), Some(actual)) if wanted == actual => {}
            (Some(_), Some(_)) => {
                return Err("navigation/profile mismatch: decoded content identity differs".into())
            }
        }
        if content_id.is_some() && !self.source_sha256.as_deref().is_some_and(is_sha256) {
            return Err("navigation/profile mismatch: missing or malformed source provenance; rebake the pack".into());
        }
        Ok(())
    }
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn nav_manifest_path(pack: &Path) -> std::path::PathBuf {
    let mut path = pack.as_os_str().to_os_string();
    path.push(".json");
    path.into()
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    hash_file_with_progress(path, |_, _| {})
}

/// Hash a resource in bounded chunks and report actual bytes processed.
/// The final `(total, total)` update is emitted only after SHA-256 finishes.
pub fn hash_file_with_progress(
    path: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<String, String> {
    let resource_error = |error| format!("resource {}: {error}", path.display());
    let mut file = std::fs::File::open(path).map_err(resource_error)?;
    let expected = file.metadata().map_err(resource_error)?.len();
    if expected > 0 {
        progress(0, expected);
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK_SIZE];
    let mut completed = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(resource_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        completed = completed.saturating_add(read as u64);
        if completed < expected {
            progress(completed, expected);
        }
    }
    let hash = format!("{:x}", digest.finalize());
    progress(completed, completed);
    Ok(hash)
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hash_bytes_with_progress(bytes, |_, _| {})
}

/// Hash an already-read buffer in the same 1 MiB chunks as [`hash_file_with_progress`].
pub fn hash_bytes_with_progress(bytes: &[u8], mut progress: impl FnMut(u64, u64)) -> String {
    let total = bytes.len() as u64;
    if total > 0 {
        progress(0, total);
    }
    let mut digest = Sha256::new();
    let mut completed = 0_u64;
    for chunk in bytes.chunks(HASH_CHUNK_SIZE) {
        digest.update(chunk);
        completed = completed.saturating_add(chunk.len() as u64);
        if completed < total {
            progress(completed, total);
        }
    }
    let hash = format!("{:x}", digest.finalize());
    progress(completed, completed);
    hash
}

fn validate_revision(revision: u16) -> Result<(), String> {
    match revision {
        274 | 289 => Ok(()),
        _ => Err(format!("unsupported revision {revision}; use 274 or 289")),
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;

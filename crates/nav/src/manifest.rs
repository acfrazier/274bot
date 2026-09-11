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
}

impl NavManifest {
    pub fn capture(
        revision: u16,
        cache: &CacheManifest,
        nav: &[u8],
        flags: Option<&[u8]>,
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
        })
    }

    pub fn verify(
        &self,
        revision: u16,
        cache: &CacheManifest,
        nav: &[u8],
        flags: Option<&[u8]>,
    ) -> Result<(), String> {
        let actual = Self::capture(revision, cache, nav, flags)?;
        if actual != *self {
            return Err(
                "navigation/profile mismatch: revision, cache identity or pack/flags content differs"
                    .into(),
            );
        }
        Ok(())
    }
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
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_revision(revision: u16) -> Result<(), String> {
    match revision {
        274 | 289 => Ok(()),
        _ => Err(format!("unsupported revision {revision}; use 274 or 289")),
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_bytes, hash_file_with_progress, HASH_CHUNK_SIZE};

    #[test]
    fn streamed_hash_matches_bytes_at_a_chunk_boundary_without_early_completion() {
        let path = std::env::temp_dir().join(format!(
            "274bot-nav-hash-boundary-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bytes: Vec<u8> = (0..HASH_CHUNK_SIZE * 2)
            .map(|index| (index % 251) as u8)
            .collect();
        std::fs::write(&path, &bytes).unwrap();

        let mut updates = Vec::new();
        let digest = hash_file_with_progress(&path, |completed, total| {
            updates.push((completed, total));
        })
        .unwrap();

        assert_eq!(digest, hash_bytes(&bytes));
        assert_eq!(updates.first(), Some(&(0, bytes.len() as u64)));
        assert_eq!(
            updates.last(),
            Some(&(bytes.len() as u64, bytes.len() as u64))
        );
        assert!(updates[..updates.len() - 1]
            .iter()
            .all(|(completed, total)| completed < total));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn streamed_hash_keeps_the_resource_error_for_an_unreadable_path() {
        let path = std::env::temp_dir().join(format!(
            "274bot-nav-hash-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);

        let error = hash_file_with_progress(&path, |_, _| {}).unwrap_err();

        assert!(error.starts_with(&format!("resource {}:", path.display())));
    }
}

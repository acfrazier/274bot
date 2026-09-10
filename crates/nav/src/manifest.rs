use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
        validate_revision(revision)?;
        let mut archives = BTreeMap::new();
        for name in Self::ARCHIVES {
            archives.insert(name.into(), hash_file(&cache_dir.join(name))?);
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
    let bytes = std::fs::read(path).map_err(|e| format!("resource {}: {e}", path.display()))?;
    Ok(hash_bytes(&bytes))
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

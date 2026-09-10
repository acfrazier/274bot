use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use nav::manifest::{CacheManifest, NavManifest};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-nav-manifest-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_cache(path: &Path) {
    for name in CacheManifest::ARCHIVES {
        std::fs::write(path.join(name), format!("fixture-{name}")).unwrap();
    }
}

#[test]
fn cache_identity_is_revision_bound_and_verifies_bytes() {
    let dir = Fixture::new();
    fixture_cache(&dir.0);
    let manifest = CacheManifest::capture(289, &dir.0).unwrap();
    manifest.verify(289, &dir.0).unwrap();
    assert!(manifest
        .verify(274, &dir.0)
        .unwrap_err()
        .contains("revision"));
    std::fs::write(dir.0.join("config"), b"changed").unwrap();
    assert!(manifest.verify(289, &dir.0).unwrap_err().contains("cache"));
}

#[test]
fn cache_identity_changes_with_revision_for_identical_archives() {
    let archives = BTreeMap::from([("config".to_string(), "abc".to_string())]);
    let first = CacheManifest {
        revision: 274,
        archives: archives.clone(),
    };
    let second = CacheManifest {
        revision: 289,
        archives,
    };
    assert_ne!(first.identity(), second.identity());
}

#[test]
fn nav_manifest_binds_revision_cache_pack_and_optional_flags() {
    let dir = Fixture::new();
    fixture_cache(&dir.0);
    let cache = CacheManifest::capture(289, &dir.0).unwrap();
    let manifest = NavManifest::capture(289, &cache, b"pack", Some(b"flags")).unwrap();
    manifest
        .verify(289, &cache, b"pack", Some(b"flags"))
        .unwrap();
    assert!(manifest
        .verify(274, &cache, b"pack", Some(b"flags"))
        .is_err());
    assert!(manifest
        .verify(289, &cache, b"other", Some(b"flags"))
        .is_err());
    assert!(manifest.verify(289, &cache, b"pack", None).is_err());
}

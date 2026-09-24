use super::{
    hash_bytes, hash_bytes_with_progress, hash_file_with_progress, CacheManifest, NavManifest,
    HASH_CHUNK_SIZE,
};

/// Packed-transfer verification stays legacy-legal for offline binds; the
/// decoded content identity is enforced only when the caller asserts one.
#[test]
fn manifest_content_id_mismatch_rejects_the_pack() {
    let revision = 289;
    let bytes = b"nav".to_vec();
    let cache = CacheManifest {
        revision,
        archives: [("versionlist".to_string(), "6dcb".repeat(16))]
            .into_iter()
            .collect(),
    };
    let mut manifest = NavManifest::capture(revision, &cache, &bytes, None, None, None).unwrap();
    assert!(
        manifest.content_id.is_none(),
        "capture without a decoded identity stays unset"
    );
    let nav_hash = super::hash_bytes(&bytes);
    assert!(manifest
        .verify(revision, &cache, &bytes, None, None, None)
        .is_ok());
    assert!(manifest
        .verify_pack(revision, &cache, &nav_hash, None)
        .is_ok());
    let err = manifest
        .verify_pack(revision, &cache, &nav_hash, Some("cdb2f161"))
        .unwrap_err();
    assert!(
        err.contains("decoded content identity"),
        "runtime verification must refuse a pack with no decoded identity: {err}"
    );
    manifest.content_id = Some("cdb2f161".into());
    let err = manifest
        .verify_pack(revision, &cache, &nav_hash, Some("cdb2f161"))
        .expect_err("decoded identity alone must not certify the baker inputs");
    assert!(err.contains("source provenance"), "{err}");
    manifest.source_sha256 = Some("ab".repeat(32));
    let mut equivalent_transfer = cache.clone();
    equivalent_transfer
        .archives
        .insert("versionlist".into(), "other packed bytes".into());
    assert!(manifest
        .verify_pack(revision, &equivalent_transfer, &nav_hash, Some("cdb2f161"))
        .is_ok());
    assert!(manifest
        .verify_pack(revision, &equivalent_transfer, &nav_hash, None)
        .is_err());
    assert!(manifest
        .verify_pack(274, &equivalent_transfer, &nav_hash, Some("cdb2f161"))
        .is_err());
    assert!(manifest
        .verify_pack(
            revision,
            &equivalent_transfer,
            "changed pack",
            Some("cdb2f161")
        )
        .is_err());
    assert!(manifest
        .verify_pack(revision, &cache, &nav_hash, Some("cdb2f161"))
        .is_ok());
    assert!(manifest
        .verify_pack(revision, &cache, &nav_hash, Some("other"))
        .is_err());
}

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

#[test]
fn in_memory_hash_matches_streamed_file_hash_without_early_completion() {
    let bytes: Vec<u8> = (0..HASH_CHUNK_SIZE + 7)
        .map(|index| (index % 251) as u8)
        .collect();
    let mut updates = Vec::new();
    let digest = hash_bytes_with_progress(&bytes, |completed, total| {
        updates.push((completed, total));
    });
    assert_eq!(digest, hash_bytes(&bytes));
    assert_eq!(updates.first(), Some(&(0, bytes.len() as u64)));
    assert_eq!(
        updates.last(),
        Some(&(bytes.len() as u64, bytes.len() as u64))
    );
    assert!(updates[..updates.len() - 1]
        .iter()
        .all(|(completed, total)| completed < total));
}

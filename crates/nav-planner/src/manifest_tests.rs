use super::{hash_bytes, hash_bytes_with_progress, hash_file_with_progress, HASH_CHUNK_SIZE};

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

#[test]
fn revision_377_cache_capture_keeps_revision_and_archive_checks() {
    use super::CacheManifest;

    let root = std::env::temp_dir().join(format!(
        "274bot-nav-cache-377-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for name in CacheManifest::ARCHIVES {
        std::fs::write(root.join(name), name.as_bytes()).unwrap();
    }
    let manifest = CacheManifest::capture(377, &root).unwrap();
    assert_eq!(manifest.revision, 377);
    assert_eq!(manifest.archives.len(), CacheManifest::ARCHIVES.len());
    manifest.verify(377, &root).unwrap();
    assert!(manifest.verify(289, &root).is_err());
    assert!(CacheManifest::capture(378, &root).is_err());
    std::fs::write(root.join("config"), b"different").unwrap();
    assert!(manifest.verify(377, &root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

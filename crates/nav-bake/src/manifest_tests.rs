use super::capture_nav_manifest;
use nav_planner::manifest::CacheManifest;

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
    let mut manifest =
        nav_bake::manifest::capture_nav_manifest(revision, &cache, &bytes, None, None, None, None, 0, 0).unwrap();
    assert!(
        manifest.content_id.is_none(),
        "capture without a decoded identity stays unset"
    );
    let nav_hash = super::hash_bytes(&bytes);
    assert!(manifest
        .verify(revision, &cache, &bytes, None, None, None, None, 0, 0)
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
fn manifest_verification_binds_decoded_zone_counts() {
    let revision = 289;
    let cache = CacheManifest {
        revision,
        archives: [("versionlist".to_string(), "6dcb".repeat(16))]
            .into_iter()
            .collect(),
    };
    let bytes = b"nav";
    let manifest =
        nav_bake::manifest::capture_nav_manifest(revision, &cache, bytes, None, None, None, None, 3, 2).unwrap();

    assert_eq!(manifest.zone_count, 3);
    assert_eq!(manifest.zone_npc_count, 2);
    assert!(manifest
        .verify(revision, &cache, bytes, None, None, None, None, 3, 2)
        .is_ok());
    assert!(manifest
        .verify(revision, &cache, bytes, None, None, None, None, 2, 2)
        .is_err());
}

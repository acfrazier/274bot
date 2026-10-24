use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn stale_stamp_error(error: crate::GenerationError) -> crate::StaleStampError {
    match error {
        crate::GenerationError::StaleStamp(error) => error,
        other => panic!("expected stale-stamp error, got {other:?}"),
    }
}

const STAMP_POIS_SHA256: &str = "5656565656565656565656565656565656565656565656565656565656565656";
const STAMP_MANIFEST_SHA256: &str =
    "6767676767676767676767676767676767676767676767676767676767676767";
const STAMP_INPUT_SHA256: &str = "7878787878787878787878787878787878787878787878787878787878787878";
const STAMP_PACK_SHA256: &str = "abababababababababababababababababababababababababababababababab";
const STAMP_FLAGS_SHA256: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const STAMP_REACH_SHA256: &str = "efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef";
const STAMP_CANLIGHT_SHA256: &str =
    "1212121212121212121212121212121212121212121212121212121212121212";

fn stamp(generator: &str, format: &str, cache_id: &str, pack_bytes: u64) -> BakeStamp {
    BakeStamp {
        content_id: None,
        source_sha256: None,
        generator: generator.into(),
        format: format.into(),
        revision: 289,
        cache_id: cache_id.into(),
        cache_manifest: None,
        nav_sha256: STAMP_PACK_SHA256.into(),
        flags_sha256: STAMP_FLAGS_SHA256.into(),
        reach_sha256: STAMP_REACH_SHA256.into(),
        canlight_sha256: STAMP_CANLIGHT_SHA256.into(),
        canlight_identity: "34".repeat(32),
        pack_bytes,
        flags_bytes: 7,
        reach_bytes: 9,
        canlight_bytes: 5,
        manifest_sha256: Some(STAMP_MANIFEST_SHA256.into()),
        manifest_bytes: Some(13),
        relative_pack: "nav/289/274bot.navpack".into(),
        relative_flags: "nav/289/274bot.navflags".into(),
        relative_reach: "nav/289/274bot.navreach".into(),
        relative_canlight: "nav/289/274bot.navcanlight".into(),
        relative_manifest: Some("nav/289/274bot.navpack.json".into()),
        pois_sha256: Some(STAMP_POIS_SHA256.into()),
        pois_bytes: Some(3),
        relative_pois: Some("nav/289/274bot.navpois".into()),
        pois_generator: Some("pois-1".into()),
        inputs: vec![InputFingerprint {
            path: "/content/maps/m1.jm2".into(),
            bytes: 10,
            modified_nanos: 5,
            sha256: STAMP_INPUT_SHA256.into(),
        }],
    }
}

fn expectation<'a>(inputs: &'a [InputFingerprint]) -> StampExpectation<'a> {
    StampExpectation {
        revision: 289,
        format: "274V16",
        generator: "gen-1",
        cache_id: "cache-1",
        inputs,
        staged_pack_bytes: Some(11),
        staged_pack_sha256: Some(STAMP_PACK_SHA256),
        staged_flags_bytes: Some(7),
        staged_flags_sha256: Some(STAMP_FLAGS_SHA256),
        staged_reach_bytes: Some(9),
        staged_reach_sha256: Some(STAMP_REACH_SHA256),
        staged_canlight_bytes: Some(5),
        staged_canlight_sha256: Some(STAMP_CANLIGHT_SHA256),
        staged_manifest_bytes: Some(13),
        staged_manifest_sha256: Some(STAMP_MANIFEST_SHA256),
        staged_pois_bytes: Some(3),
        staged_pois_sha256: Some(STAMP_POIS_SHA256),
        pois_generator: "pois-1",
    }
}

#[test]
fn a_matching_stamp_covers_and_every_change_is_stale() {
    let baked = stamp("gen-1", "274V16", "cache-1", 11);
    let inputs = baked.inputs.clone();
    assert!(baked.covers(&expectation(&inputs)).is_ok());

    let mut changed = expectation(&inputs);
    changed.cache_id = "cache-2";
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::CacheIdentityChanged
    );

    let mut changed = expectation(&inputs);
    changed.generator = "gen-2";
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::GeneratorChanged
    );

    let mut changed = expectation(&inputs);
    changed.format = "274V9";
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::FormatMismatch {
            stamped: "274V16".into(),
            selected: "274V9".into(),
        }
    );

    let mut changed = expectation(&inputs);
    changed.revision = 274;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::RevisionMismatch {
            stamped: 289,
            selected: 274,
        }
    );

    for (artifact, path, changed_size) in [
        (crate::StagedArtifact::Pack, "nav/289/274bot.navpack", 12),
        (crate::StagedArtifact::Flags, "nav/289/274bot.navflags", 8),
        (crate::StagedArtifact::Reach, "nav/289/274bot.navreach", 10),
        (
            crate::StagedArtifact::Canlight,
            "nav/289/274bot.navcanlight",
            6,
        ),
    ] {
        let mut changed = expectation(&inputs);
        match artifact {
            crate::StagedArtifact::Pack => changed.staged_pack_bytes = Some(changed_size),
            crate::StagedArtifact::Flags => changed.staged_flags_bytes = Some(changed_size),
            crate::StagedArtifact::Reach => changed.staged_reach_bytes = Some(changed_size),
            crate::StagedArtifact::Canlight => {
                changed.staged_canlight_bytes = Some(changed_size)
            }
            _ => unreachable!("only staged size fields are enumerated"),
        }
        let stamped_bytes = match artifact {
            crate::StagedArtifact::Pack => 11,
            crate::StagedArtifact::Flags => 7,
            crate::StagedArtifact::Reach => 9,
            crate::StagedArtifact::Canlight => 5,
            _ => unreachable!("only staged size fields are enumerated"),
        };
        assert_eq!(
            stale_stamp_error(baked.covers(&changed).unwrap_err()),
            crate::StaleStampError::StagedArtifactSizeMismatch {
                artifact,
                path: path.into(),
                actual_bytes: changed_size,
                stamped_bytes,
            }
        );
    }

    for (artifact, path) in [
        (crate::StagedArtifact::Pack, "nav/289/274bot.navpack"),
        (crate::StagedArtifact::Flags, "nav/289/274bot.navflags"),
        (crate::StagedArtifact::Reach, "nav/289/274bot.navreach"),
        (crate::StagedArtifact::Canlight, "nav/289/274bot.navcanlight"),
    ] {
        let mut changed = expectation(&inputs);
        match artifact {
            crate::StagedArtifact::Pack => changed.staged_pack_bytes = None,
            crate::StagedArtifact::Flags => changed.staged_flags_bytes = None,
            crate::StagedArtifact::Reach => changed.staged_reach_bytes = None,
            crate::StagedArtifact::Canlight => changed.staged_canlight_bytes = None,
            _ => unreachable!("only staged size fields are enumerated"),
        }
        assert_eq!(
            stale_stamp_error(baked.covers(&changed).unwrap_err()),
            crate::StaleStampError::StagedArtifactMissing {
                artifact,
                path: path.into(),
            }
        );
    }

    let mut changed = expectation(&inputs);
    changed.staged_pois_bytes = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedArtifactMissing {
            artifact: crate::StagedArtifact::NavPois,
            path: "nav/289/274bot.navpois".into(),
        }
    );

    let mut changed = expectation(&inputs);
    changed.staged_pois_bytes = Some(4);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::NavPoisSizeMismatch {
            actual_bytes: 4,
            stamped_bytes: 3,
        }
    );

    let mut no_stamped_pois_bytes = baked.clone();
    no_stamped_pois_bytes.pois_bytes = None;
    assert_eq!(
        stale_stamp_error(no_stamped_pois_bytes.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::NavPoisBytesMissingFromStamp
    );

    let mut changed = expectation(&inputs);
    changed.staged_pois_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::NavPois,
        }
    );

    for artifact in [
        crate::StagedArtifact::Pack,
        crate::StagedArtifact::Flags,
        crate::StagedArtifact::Reach,
        crate::StagedArtifact::Canlight,
        crate::StagedArtifact::Manifest,
    ] {
        let mut changed = expectation(&inputs);
        match artifact {
            crate::StagedArtifact::Pack => changed.staged_pack_sha256 = None,
            crate::StagedArtifact::Flags => changed.staged_flags_sha256 = None,
            crate::StagedArtifact::Reach => changed.staged_reach_sha256 = None,
            crate::StagedArtifact::Canlight => changed.staged_canlight_sha256 = None,
            crate::StagedArtifact::Manifest => changed.staged_manifest_sha256 = None,
            crate::StagedArtifact::NavPois => unreachable!("navpois uses a separate digest gate"),
        }
        assert_eq!(
            stale_stamp_error(baked.covers(&changed).unwrap_err()),
            crate::StaleStampError::StagedDigestMissing { artifact }
        );
    }

    let mut no_manifest_digest = baked.clone();
    no_manifest_digest.manifest_sha256 = None;
    assert_eq!(
        stale_stamp_error(no_manifest_digest.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::ManifestDigestMissingFromStamp
    );

    let mut changed = expectation(&inputs);
    changed.staged_pack_sha256 = Some(STAMP_INPUT_SHA256);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::Pack,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_flags_sha256 = Some(STAMP_INPUT_SHA256);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::Flags,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_reach_sha256 = Some(STAMP_INPUT_SHA256);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::Reach,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_canlight_sha256 = Some(STAMP_INPUT_SHA256);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::Canlight,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_manifest_sha256 = Some(STAMP_INPUT_SHA256);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::Manifest,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_pois_sha256 = Some("00");
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMismatch {
            artifact: crate::StagedArtifact::NavPois,
        }
    );

    let mut changed = expectation(&inputs);
    changed.staged_pack_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::Pack,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_flags_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::Flags,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_reach_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::Reach,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_canlight_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::Canlight,
        }
    );
    let mut changed = expectation(&inputs);
    changed.staged_manifest_sha256 = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::StagedDigestMissing {
            artifact: crate::StagedArtifact::Manifest,
        }
    );

    let mut changed = expectation(&inputs);
    changed.staged_manifest_bytes = None;
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::ManifestBytesMissingFromDisk
    );
    let mut no_manifest_bytes = baked.clone();
    no_manifest_bytes.manifest_bytes = None;
    assert_eq!(
        stale_stamp_error(no_manifest_bytes.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::ManifestBytesMissingFromStamp
    );
    let mut changed = expectation(&inputs);
    changed.staged_manifest_bytes = Some(14);
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::ManifestSizeMismatch {
            actual_bytes: 14,
            stamped_bytes: 13,
        }
    );
    let mut no_manifest_path = baked.clone();
    no_manifest_path.relative_manifest = None;
    assert_eq!(
        stale_stamp_error(no_manifest_path.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::ManifestPathMissingFromStamp
    );
    let mut no_pois_digest = baked.clone();
    no_pois_digest.pois_sha256 = None;
    assert_eq!(
        stale_stamp_error(no_pois_digest.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::NavPoisDigestMissingFromStamp
    );
    let mut changed = expectation(&inputs);
    changed.pois_generator = "pois-2";
    assert_eq!(
        stale_stamp_error(baked.covers(&changed).unwrap_err()),
        crate::StaleStampError::NavPoisGeneratorChanged {
            stamped: Some("pois-1".into()),
            expected: "pois-2".into(),
        }
    );

    let mut no_pois_generator = baked.clone();
    no_pois_generator.pois_generator = None;
    assert_eq!(
        stale_stamp_error(no_pois_generator.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::NavPoisGeneratorChanged {
            stamped: None,
            expected: "pois-1".into(),
        }
    );

    let modified = [InputFingerprint {
        path: "/content/maps/m1.jm2".into(),
        bytes: 10,
        modified_nanos: 6,
        sha256: STAMP_INPUT_SHA256.into(),
    }];
    assert_eq!(
        stale_stamp_error(baked.covers(&expectation(&modified)).unwrap_err()),
        crate::StaleStampError::CanonicalInputChanged {
            path: "/content/maps/m1.jm2".into(),
        }
    );
    let same_metadata_replacement = [InputFingerprint {
        path: "/content/maps/m1.jm2".into(),
        bytes: 10,
        modified_nanos: 5,
        sha256: "99".repeat(32),
    }];
    assert_eq!(
        stale_stamp_error(
            baked
                .covers(&expectation(&same_metadata_replacement))
                .unwrap_err()
        ),
        crate::StaleStampError::CanonicalInputChanged {
            path: "/content/maps/m1.jm2".into(),
        }
    );

    let added = [
        inputs[0].clone(),
        InputFingerprint {
            path: "/content/maps/m2.jm2".into(),
            bytes: 1,
            modified_nanos: 1,
            sha256: STAMP_INPUT_SHA256.into(),
        },
    ];
    assert_eq!(
        stale_stamp_error(baked.covers(&expectation(&added)).unwrap_err()),
        crate::StaleStampError::CanonicalInputCountMismatch {
            stamped: 1,
            present: 2,
        }
    );
}

#[test]
fn legacy_v8_stamp_without_manifest_or_input_hashes_is_stale() {
    let json = r#"{
        "generator":"gen-1",
        "format":"274V8",
        "revision":289,
        "cache_id":"cache-1",
        "cache_manifest":null,
        "nav_sha256":"abababababababababababababababababababababababababababababababab",
        "flags_sha256":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
        "reach_sha256":"efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef",
        "canlight_sha256":"1212121212121212121212121212121212121212121212121212121212121212",
        "canlight_identity":"3434343434343434343434343434343434343434343434343434343434343434",
        "pack_bytes":11,
        "flags_bytes":7,
        "reach_bytes":9,
        "canlight_bytes":5,
        "relative_pack":"nav/289/274bot.navpack",
        "relative_flags":"nav/289/274bot.navflags",
        "relative_reach":"nav/289/274bot.navreach",
        "relative_canlight":"nav/289/274bot.navcanlight",
        "inputs":[{"path":"/content/maps/m1.jm2","bytes":10,"modified_nanos":5}]
    }"#;
    let stamp: BakeStamp = serde_json::from_str(json).expect("legacy stamp still parses");
    assert_eq!(stamp.format, "274V8");
    assert!(stamp.pois_sha256.is_none());
    assert!(stamp.manifest_sha256.is_none());
    assert!(stamp.manifest_bytes.is_none());
    assert!(stamp.relative_manifest.is_none());
    assert_eq!(stamp.inputs[0].sha256, "");

    let err = stamp
        .covers(&expectation(&stamp.inputs))
        .expect_err("pre-N4 stamp is stale");
    assert_eq!(
        stale_stamp_error(err),
        crate::StaleStampError::FormatMismatch {
            stamped: "274V8".into(),
            selected: "274V16".into(),
        }
    );
}

#[test]
fn current_format_stamp_without_manifest_pois_or_input_hashes_is_stale() {
    // The legacy v8 literal above asserts on the first gate (`format`), so
    // these stamp-side None/empty checks need a current-format stamp to run.
    let baked = stamp("gen-1", "274V16", "cache-1", 11);
    let inputs = baked.inputs.clone();

    let mut no_pois = baked.clone();
    no_pois.pois_sha256 = None;
    assert_eq!(
        stale_stamp_error(no_pois.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::NavPoisDigestMissingFromStamp
    );

    let mut no_manifest = baked.clone();
    no_manifest.manifest_sha256 = None;
    no_manifest.manifest_bytes = None;
    no_manifest.relative_manifest = None;
    assert_eq!(
        stale_stamp_error(no_manifest.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::ManifestBytesMissingFromStamp
    );

    let mut empty_input_hash = baked;
    empty_input_hash.inputs[0].sha256 = String::new();
    assert_eq!(
        stale_stamp_error(empty_input_hash.covers(&expectation(&inputs)).unwrap_err()),
        crate::StaleStampError::CanonicalInputChanged {
            path: "/content/maps/m1.jm2".into(),
        }
    );
}

#[test]
fn content_digest_detects_same_size_replacement() {
    let root = std::env::temp_dir().join(format!("nav-source-digest-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("map.jm2");
    std::fs::write(&file, b"one").unwrap();
    let first = super::source_digest(&root, &[]).unwrap();
    std::fs::write(&file, b"two").unwrap();
    assert_ne!(first, super::source_digest(&root, &[]).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_source_fingerprint_is_a_typed_input_error() {
    let root = Path::new("/content");
    let source = Path::new("/content/config");
    let inputs: [InputFingerprint; 0] = [];

    assert!(matches!(
        source_digest_from_fingerprints(root, &[source], &inputs),
        Err(crate::GenerationError::SourceInput(
            crate::SourceInputError::NotFingerprinted { path }
        )) if path.as_path() == source
    ));
}

#[test]
fn content_digest_reuses_the_captured_fingerprint_hashes() {
    let root = std::env::temp_dir().join(format!("nav-source-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("map.jm2");
    let explicit = root.with_extension("config");
    std::fs::write(&file, b"one").unwrap();
    std::fs::write(&explicit, b"config-one").unwrap();
    let snapshot = fingerprints(&root, &[&explicit]).unwrap();
    let before = source_digest_from_fingerprints(&root, &[&explicit], &snapshot).unwrap();

    std::fs::write(&file, b"two").unwrap();
    std::fs::write(&explicit, b"config-two").unwrap();
    assert_eq!(
        source_digest_from_fingerprints(&root, &[&explicit], &snapshot).unwrap(),
        before,
        "a snapshot digest must consume the stored hashes, not re-read files"
    );
    assert_ne!(source_digest(&root, &[&explicit]).unwrap(), before);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_file(explicit).unwrap();
}

/// Provenance is machine-independent: normalized `/` labels determine order,
/// even when the native separator sorts differently.
#[test]
fn content_digest_is_the_same_on_every_platform() {
    let root = std::env::temp_dir().join(format!("nav-source-labels-{}", std::process::id()));
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::write(root.join("a").join("file"), b"nested").unwrap();
    std::fs::write(root.join("a;"), b"punctuation").unwrap();
    // `a.x` sorts below `/` after the shared `a` prefix: component order
    // puts `a/file` first, while a `/`-label String sort puts `a.x` first.
    // Without this row the pinned digest cannot tell the R4 ordering apart.
    std::fs::write(root.join("a.x"), b"dot").unwrap();
    let inputs = fingerprints(&root, &[]).unwrap();
    assert_eq!(inputs.len(), 3);
    let reversed: Vec<_> = inputs.into_iter().rev().collect();
    let digest = source_digest_from_fingerprints(&root, &[], &reversed).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        digest,
        "68d57c6c9ae106a260c4c19925a477685a5654e082fb174013e1a5a5ddd67cec"
    );
}

#[test]
fn fingerprints_cover_the_tree_and_detect_edits() {
    let root = std::env::temp_dir().join(format!("274bot-bundle-{}", std::process::id()));
    let nested = root.join("maps");
    std::fs::create_dir_all(&nested).unwrap();
    let map = nested.join("m1.jm2");
    std::fs::write(&map, b"one").unwrap();
    let extra = root.join("..").join("outside.loc");
    std::fs::write(&extra, b"loc").unwrap();

    let first = fingerprints(&nested, &[&extra]).unwrap();
    assert_eq!(first.len(), 2);
    assert!(first.iter().any(|row| row.path.ends_with("m1.jm2")));
    assert!(first.iter().any(|row| row.path.ends_with("outside.loc")));

    // Same bytes, newer mtime: still a change (explicit mtime, no sleep).
    std::fs::write(&map, b"one").unwrap();
    let bumped = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(&map)
        .unwrap()
        .set_modified(bumped)
        .unwrap();
    let second = fingerprints(&nested, &[&extra]).unwrap();
    assert_ne!(first, second);

    let missing = root.join("missing");
    assert!(matches!(
        fingerprints(&missing, &[]),
        Err(crate::GenerationError::SourceInput(
            crate::SourceInputError::Io {
                context: crate::SourceInputContext::BakeInputDirectory,
                path,
                source,
            }
        )) if path == missing && source.kind() == std::io::ErrorKind::NotFound
    ));
    std::fs::remove_file(&extra).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn generated_rows_win_over_superseded_checked_in_rows() {
    let generated = manifest::NavIdentityRow {
        revision: 289,
        cache_id: "cache".into(),
        content_id: None,
        source_sha256: None,
        format: "274V8".into(),
        nav_sha256: "aa".repeat(32),
        flags_sha256: None,
        reach_sha256: None,
        canlight_sha256: None,
        canlight_identity: None,
        pois_sha256: None,
        relative_path: "nav/289/274bot.navpack".into(),
    };
    let superseded = manifest::NavIdentityRow {
        nav_sha256: "bb".repeat(32),
        ..generated.clone()
    };
    let kept = manifest::NavIdentityRow {
        revision: 274,
        cache_id: "other".into(),
        content_id: None,
        source_sha256: None,
        format: "274V8".into(),
        nav_sha256: "cc".repeat(32),
        flags_sha256: None,
        reach_sha256: None,
        canlight_sha256: None,
        canlight_identity: None,
        pois_sha256: None,
        relative_path: "prebuilt/274bot.navpack".into(),
    };
    let (rows, notes) = merge_identity_rows(vec![generated], vec![superseded, kept.clone()]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], kept);
    assert_eq!(rows[1].nav_sha256, "aa".repeat(32));
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("superseded"), "{}", notes[0]);
    // Checked-in rows alone (a skip-mode or prebuilt-only build) survive.
    let (rows, notes) = merge_identity_rows(Vec::new(), vec![kept.clone()]);
    assert_eq!(rows, vec![kept]);
    assert!(notes.is_empty());
}

#[test]
fn artifact_layout_is_install_relative_and_revision_scoped() {
    let layout = artifact_layout(289);
    assert_eq!(layout.dir, PathBuf::from("nav/289"));
    assert_eq!(layout.relative_pack, "nav/289/274bot.navpack");
    assert_eq!(layout.relative_flags, "nav/289/274bot.navflags");
    assert_eq!(layout.relative_reach, "nav/289/274bot.navreach");
    assert_eq!(layout.relative_canlight, "nav/289/274bot.navcanlight");
    assert_eq!(layout.relative_pois, "nav/289/274bot.navpois");
    assert_eq!(layout.relative_stamp, "nav/289/nav-build.json");
    assert!(!Path::new(&layout.relative_pack).is_absolute());
    assert_eq!(artifact_layout(274).relative_pack, "nav/274/274bot.navpack");
    assert_ne!(artifact_layout(289).dir, artifact_layout(274).dir);
}

#[test]
fn build_resource_root_is_the_profile_directory_or_the_override() {
    let out = Path::new("/w/target/debug/build/host-play-abc123/out");
    assert_eq!(
        resource_root_for_build(out, None).unwrap(),
        PathBuf::from("/w/target/debug")
    );
    // Cross builds keep the target-triple directory.
    let out = Path::new("/w/target/x86_64-pc-windows-msvc/release/build/host-play-abc/out");
    assert_eq!(
        resource_root_for_build(out, None).unwrap(),
        PathBuf::from("/w/target/x86_64-pc-windows-msvc/release")
    );
    // A macOS bundle (or installer staging dir) stages into its own
    // absolute directory instead; an absolute override passes through.
    let bundle = std::env::temp_dir().join("274bot.app/Contents/Resources");
    assert_eq!(resource_root_for_build(out, Some(&bundle)).unwrap(), bundle);
    assert!(resource_root_for_build(Path::new("/unexpected/out"), None).is_err());
    assert!(resource_root_for_build(out, Some(Path::new(""))).is_err());
}

// -----------------------------------------------------------------------
// Required canonical content inputs.
// -----------------------------------------------------------------------

struct RequiredFixture(PathBuf);

impl RequiredFixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-nav-required-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for RequiredFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Write one file of an inventory row under `root`, creating parents.
fn write_required_file(root: &Path, path: &str) {
    let file = root.join(path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&file, b"fixture").unwrap();
}

/// A fixture of exactly the inventory: every required input written.
fn write_inventory_tree(root: &Path, revision: u16) {
    for input in required_content_inputs(revision).expect("inventory") {
        write_required_file(root, input.path);
    }
}

/// Whether a kind names a file of one of the baker's recursive scans.
fn is_scan_kind(kind: RequiredKind) -> bool {
    matches!(
        kind,
        RequiredKind::Rs2 | RequiredKind::Constant | RequiredKind::Obj | RequiredKind::Loc
    )
}

/// The scan classes' roots and extensions, mirroring the baker
/// (`transport.rs` `visit_rs2`, `script_constants`, `slash_weapon_ids`,
/// `door_config_names`).
const SCAN_CLASSES: &[(RequiredKind, &[&str], &str)] = &[
    (RequiredKind::Rs2, &["scripts"], "rs2"),
    (RequiredKind::Constant, &["scripts"], "constant"),
    (RequiredKind::Obj, &["scripts"], "obj"),
    (
        RequiredKind::Loc,
        &[
            "scripts/doors/configs",
            "scripts/quests",
            "scripts/areas",
            "scripts/general_use/configs",
        ],
        "loc",
    ),
];

/// Every file of the scan classes under a canonical content tree, as
/// `content-relative path → class kind`.
fn scan_inputs(root: &Path) -> BTreeMap<String, RequiredKind> {
    let mut out = BTreeMap::new();
    for (kind, roots, extension) in SCAN_CLASSES {
        for root_rel in *roots {
            collect_scan_files(root, &root.join(root_rel), extension, *kind, &mut out);
        }
    }
    out
}

fn collect_scan_files(
    root: &Path,
    dir: &Path,
    extension: &str,
    kind: RequiredKind,
    out: &mut BTreeMap<String, RequiredKind>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_scan_files(root, &path, extension, kind, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some(extension) {
            let relative = path
                .strip_prefix(root)
                .expect("scan file is under the content root")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(relative, kind);
        }
    }
}

/// Every canonical mapsquare of a content tree, `maps/*.jm2`.
fn canonical_maps(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(root.join("maps")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".jm2") {
                out.insert(format!("maps/{name}"));
            }
        }
    }
    out
}

/// The canonical content root selected for this build, when explicitly configured.
fn canonical_content_root(revision: u16) -> Option<std::path::PathBuf> {
    let selected = std::env::var("BOT_NAV_REVISION")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(289);
    if selected != revision {
        return None;
    }
    let root = std::env::var_os("BOT_NAV_CONTENT_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let engine = std::env::var_os("BOT_NAV_ENGINE_DIR")
                .or_else(|| std::env::var_os("ENGINE_DIR"))?;
            std::path::PathBuf::from(engine)
                .parent()
                .map(|parent| parent.join("content"))
        })?;
    root.is_dir().then_some(root)
}

fn mapsquares(revision: u16) -> usize {
    required_content_inputs(revision)
        .expect("inventory")
        .iter()
        .filter(|row| row.kind == RequiredKind::Mapsquare)
        .count()
}

#[test]
fn the_inventory_names_the_implicit_bake_inputs_of_each_release_revision() {
    let rows = required_content_inputs(289).expect("289 inventory");
    let paths: Vec<&str> = rows.iter().map(|row| row.path).collect();
    for expected in [
        "pack/loc.pack",
        "pack/obj.pack",
        "pack/varp.pack",
        "scripts/interface_bank/configs/bank_booth.loc",
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "scripts/ladders+stairs/scripts/stairs.rs2",
        "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
        "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "scripts/minigames/game_ranging/configs/ranging.loc",
        "scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2",
        "scripts/skill_magic/configs/magic_spells.dbrow",
        "scripts/skill_magic/configs/enchanted_jewelry.obj",
        "scripts/skill_firemaking/configs/bank_zones.dbrow",
        "scripts/areas/area_wilderness/configs/wilderness_zones.dbrow",
    ] {
        assert!(paths.contains(&expected), "{expected} is required");
    }
    // The scans' members are named one row each, not anchored by their
    // directory: the concrete holes of the round-1 guard.
    for scanned in [
        "scripts/skill_agility/scripts/shortcuts.rs2",
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
        "scripts/doors/configs/doubledoors.constant",
    ] {
        assert!(
            rows.iter()
                .any(|row| row.path == scanned && is_scan_kind(row.kind)),
            "{scanned} is a scan-class input"
        );
    }
    // Every scan-class row is the file its kind promises, under the scan
    // roots, and every class is populated in both revisions.
    for revision in [289u16, 274] {
        let rows = required_content_inputs(revision).expect("inventory");
        for row in &rows {
            let extension = match row.kind {
                RequiredKind::Rs2 => Some("rs2"),
                RequiredKind::Constant => Some("constant"),
                RequiredKind::Obj => Some("obj"),
                RequiredKind::Loc => Some("loc"),
                RequiredKind::Mapsquare => Some("jm2"),
                RequiredKind::File => None,
            };
            if matches!(row.kind, RequiredKind::Mapsquare) {
                assert!(row.path.starts_with("maps/"), "{}", row.path);
            } else if let Some(extension) = extension {
                assert!(
                    row.path.ends_with(&format!(".{extension}")),
                    "{} is a {} row",
                    row.path,
                    extension
                );
                assert!(row.path.starts_with("scripts/"), "{}", row.path);
            }
        }
        for kind in [
            RequiredKind::Rs2,
            RequiredKind::Constant,
            RequiredKind::Obj,
            RequiredKind::Loc,
        ] {
            assert!(
                rows.iter().any(|row| row.kind == kind),
                "revision {revision} has {} rows",
                kind.name()
            );
        }
    }
    assert_eq!(mapsquares(289), 534, "the canonical 289 map set");
    assert_eq!(mapsquares(274), 483, "the canonical 274 map set");
    let mut unique = paths.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), paths.len(), "no duplicate inventory rows");
    assert!(paths
        .iter()
        .all(|path| !path.starts_with('/') && !path.split('/').any(|part| part == "..")));
    // The two revisions are distinct inventories, and a revision without
    // one reports none (the build script rejects it before this point).
    assert_ne!(rows.len(), required_content_inputs(274).unwrap().len());
    assert!(required_content_inputs(1).expect("no inventory").is_empty());
}

#[test]
fn a_complete_inventory_tree_passes_and_the_reduced_round_one_set_fails() {
    let complete = RequiredFixture::new("complete");
    write_inventory_tree(&complete.0, 289);
    assert!(
        missing_content_inputs(289, &complete.0)
            .expect("guard")
            .is_empty(),
        "a fixture of exactly the inventory is complete"
    );
    // Bytes are never the guard's business: an edit or an addition stays
    // eligible for the ordinary fingerprint/staleness path.
    std::fs::write(complete.0.join("pack/loc.pack"), b"edited bytes").unwrap();
    std::fs::write(
        complete.0.join("scripts/general_use/configs/extra.loc"),
        b"added",
    )
    .unwrap();
    std::fs::write(complete.0.join("maps/m99_99.jm2"), b"added").unwrap();
    assert!(missing_content_inputs(289, &complete.0)
        .expect("guard")
        .is_empty());

    // The round-1 native input set (docs/compat/release-p1b-native.md):
    // maps, the door configs and gates.loc, nothing else.
    let reduced = RequiredFixture::new("reduced");
    for input in required_content_inputs(289).expect("inventory") {
        let kept = match input.kind {
            RequiredKind::Mapsquare => true,
            _ => input.path.contains("doors/configs") || input.path.ends_with("gates.loc"),
        };
        if kept {
            write_required_file(&reduced.0, input.path);
        }
    }
    let missing = missing_content_inputs(289, &reduced.0).expect("guard");
    for expected in [
        "pack/loc.pack",
        "pack/obj.pack",
        "scripts/interface_bank/configs/bank_booth.loc",
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "scripts/skill_magic/configs/magic_spells.dbrow",
        "scripts/skill_agility/scripts/shortcuts.rs2",
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
    ] {
        assert!(
            missing.iter().any(|row| row.starts_with(expected)),
            "{expected} is reported missing: {missing:?}"
        );
    }
    assert!(
        !missing.iter().any(|row| row.starts_with("maps/")),
        "the reduced set keeps every mapsquare: {missing:?}"
    );
    assert!(missing.len() > 10, "the reduced set is far from complete");
}

#[test]
fn a_missing_scan_child_is_reported_while_its_siblings_remain() {
    let root = RequiredFixture::new("scan-child");
    write_inventory_tree(&root.0, 289);
    assert!(missing_content_inputs(289, &root.0)
        .expect("guard")
        .is_empty());

    // Consumed children whose directory anchor stayed non-empty: the
    // concrete holes of the round-1 directory-anchor guard.
    let removed = [
        "scripts/skill_agility/scripts/shortcuts.rs2",
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
    ];
    for path in removed {
        std::fs::remove_file(root.0.join(path)).unwrap();
        let siblings = std::fs::read_dir(root.0.join(path).parent().unwrap())
            .unwrap()
            .count();
        assert!(siblings > 0, "{path}: its directory is not empty");
    }

    let missing = missing_content_inputs(289, &root.0).expect("guard");
    assert_eq!(missing.len(), removed.len(), "{missing:?}");
    for path in removed {
        assert!(
            missing
                .iter()
                .any(|row| row.starts_with(&format!("{path} ("))
                    && row.contains("recursive scripts/**/*.rs2 scan")),
            "{path} is reported missing: {missing:?}"
        );
    }
}

#[test]
fn a_single_missing_mapsquare_or_named_input_is_reported() {
    let root = RequiredFixture::new("single-missing");
    write_inventory_tree(&root.0, 274);
    assert!(missing_content_inputs(274, &root.0)
        .expect("guard")
        .is_empty());

    let map = required_content_inputs(274)
        .expect("inventory")
        .into_iter()
        .find(|row| row.kind == RequiredKind::Mapsquare)
        .expect("a required mapsquare")
        .path
        .to_string();
    std::fs::remove_file(root.0.join(&map)).unwrap();
    std::fs::remove_file(root.0.join("scripts/ladders+stairs/scripts/stairs.rs2")).unwrap();

    let missing = missing_content_inputs(274, &root.0).expect("guard");
    assert_eq!(missing.len(), 2, "{missing:?}");
    assert!(
        missing
            .iter()
            .any(|row| row.starts_with(&map)
                && row.contains("canonical mapsquare of revision 274")),
        "{missing:?}"
    );
    assert!(
        missing
            .iter()
            .any(|row| row.starts_with("scripts/ladders+stairs/scripts/stairs.rs2 (stair edges)")),
        "{missing:?}"
    );
}

#[test]
fn a_missing_ranging_loc_is_reported_without_duplicating_scan_owned_rs2() {
    const RANGING_LOC: &str = "scripts/minigames/game_ranging/configs/ranging.loc";
    const RANGING_DOOR_RS2: &str = "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2";
    for revision in [289u16, 274] {
        let rows = required_content_inputs(revision).expect("inventory");
        let loc_rows: Vec<_> = rows.iter().filter(|row| row.path == RANGING_LOC).collect();
        assert_eq!(
            loc_rows.len(),
            1,
            "revision {revision}: {RANGING_LOC} is one named file row"
        );
        assert_eq!(
            loc_rows[0].kind,
            RequiredKind::File,
            "revision {revision}: ranging.loc is outside the loc scan roots"
        );
        assert!(
            rows.iter()
                .filter(|row| row.path == RANGING_DOOR_RS2)
                .all(|row| row.kind == RequiredKind::Rs2),
            "revision {revision}: {RANGING_DOOR_RS2} stays scan-owned only"
        );

        let root = RequiredFixture::new(&format!("ranging-loc-{revision}"));
        write_inventory_tree(&root.0, revision);
        std::fs::remove_file(root.0.join(RANGING_LOC)).unwrap();
        let missing = missing_content_inputs(revision, &root.0).expect("guard");
        assert_eq!(missing.len(), 1, "revision {revision}: {missing:?}");
        assert!(
            missing[0].starts_with(&format!("{RANGING_LOC} (Ranging Guild door edges)")),
            "revision {revision}: {missing:?}"
        );
    }
}

#[test]
#[ignore = "reads the configured canonical 289/274 content trees"]
fn the_canonical_content_trees_satisfy_their_inventory() {
    for revision in [289u16, 274] {
        let Some(root) = canonical_content_root(revision) else {
            println!("revision {revision}: no canonical content tree here");
            continue;
        };
        let rows = required_content_inputs(revision).expect("inventory");
        let missing = missing_content_inputs(revision, &root).expect("guard");
        assert!(missing.is_empty(), "{}: {missing:?}", root.display());

        // The scan classes are file membership of the canonical tree, not
        // a sample: every file the scans read is a row of its class (or a
        // named `file` row), and no class row names a file the tree does
        // not read. A mismatch here means the inventory needs refreshing.
        let scanned = scan_inputs(&root);
        let mut class_rows = 0;
        for row in &rows {
            if !is_scan_kind(row.kind) {
                continue;
            }
            class_rows += 1;
            assert_eq!(
                scanned.get(row.path).copied(),
                Some(row.kind),
                "revision {revision}: {} is not a canonical {} scan input",
                row.path,
                row.kind.name()
            );
        }
        for (path, kind) in &scanned {
            let row_kind = rows.iter().find(|row| row.path == path).map(|row| row.kind);
            assert!(
                row_kind == Some(*kind) || row_kind == Some(RequiredKind::File),
                "revision {revision}: canonical {} scan input {path} is {row_kind:?} \
                 in the inventory",
                kind.name()
            );
        }

        // The mapsquare rows are the canonical map set, all of it.
        let maps: BTreeSet<String> = rows
            .iter()
            .filter(|row| row.kind == RequiredKind::Mapsquare)
            .map(|row| row.path.to_string())
            .collect();
        assert_eq!(maps, canonical_maps(&root), "the canonical map set");

        println!(
            "revision {revision}: {} required inputs ({class_rows} scan-class rows) present in {}",
            rows.len(),
            root.display()
        );
    }
}

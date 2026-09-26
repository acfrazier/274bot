use super::*;

fn digest(sha: &str) -> InputDigest {
    InputDigest {
        target: "/catalog".into(),
        sha256: Some(sha.into()),
        bytes: Some(1),
        files: 1,
        note: None,
    }
}

fn identity() -> RunIdentity {
    serde_json::from_value(json!({
            "manifest_sha256": "a",
            "suite_id": "s",
            "reference_commit": "c",
            "reference_tree": "t",
            "reference_archive_sha256": "z",
            "host": {"available": true, "commit": "h", "branch": "b", "dirty": false, "content_sha256": "hc", "note": null},
            "client": {"available": true, "commit": "k", "branch": null, "dirty": false, "content_sha256": "kc", "note": null},
            "binaries": {"core": {"kind": "direct", "program": "p", "args": [], "sha256": "d", "size": 1, "note": null}},
            "profile": {
                "profile": "local-274", "revision": null, "host": null, "port": null,
                "selection": "local-274",
                "resolved": {"selection": "local-274", "cache": "/cache", "vault": "/vault", "nav_pack": "/nav", "nav_flags": "/navflags", "content": "/content", "unpack": "/unpack"},
                "engine": null,
                "cache": {"target": "/cache", "sha256": "cc", "bytes": 2, "files": 1, "note": null},
                "catalog": {"target": "/catalog", "sha256": "cat", "bytes": 2, "files": 1, "note": null},
                "vault": {"target": "/vault", "sha256": "vv", "bytes": 2, "files": 1, "note": null},
                "nav_pack": {"target": "/nav", "sha256": "np", "bytes": 1, "files": 1, "note": null},
                "nav_flags": {"target": "/navflags", "sha256": "nf", "bytes": 1, "files": 1, "note": null},
                "content": {"target": "/content", "sha256": "cn", "bytes": 1, "files": 1, "note": null},
                "unpack": {"target": "/unpack", "sha256": "un", "bytes": 1, "files": 1, "note": null},
                "lowmem": true, "mainland": false, "jobs": 1
            },
            "settings": {
                "level": "quick", "only": [], "changed_paths": [], "changed_source": "supplied",
                "child_args": [], "child_env_keys": []
            },
            "selection": ["thiever"]
        }))
        .unwrap()
}

#[test]
fn resume_differences_name_every_changed_component() {
    let base = identity();
    assert!(base.differences(&base).is_empty());
    assert!(base.unresolved().is_empty(), "{:?}", base.unresolved());

    let mut changed = base.clone();
    changed.manifest_sha256 = "b".into();
    assert_eq!(changed.differences(&base), vec!["manifest_sha256"]);

    // A same-path, changed-content catalog refuses: the digest is what is compared.
    let mut changed = base.clone();
    changed.profile.catalog = digest("other-catalog");
    assert_eq!(
        changed.differences(&base),
        vec!["profile/input configuration"]
    );

    let mut changed = base.clone();
    changed.profile.vault = digest("other-vault");
    assert_eq!(
        changed.differences(&base),
        vec!["profile/input configuration"]
    );

    let mut changed = base.clone();
    changed.selection = vec!["thiever".into(), "ardy_fighter".into()];
    assert_eq!(changed.differences(&base), vec!["selection"]);

    let mut changed = base.clone();
    changed.host.commit = Some("other".into());
    assert_eq!(changed.differences(&base), vec!["host revision"]);

    // The same commit with a different working tree is a different identity.
    let mut changed = base.clone();
    changed.host.content_sha256 = Some("other-content".into());
    assert_eq!(changed.differences(&base), vec!["host revision"]);

    let mut changed = base.clone();
    changed.binaries.get_mut("core").unwrap().sha256 = Some("other".into());
    assert_eq!(changed.differences(&base), vec!["native binary"]);

    let mut changed = base.clone();
    changed.settings.only = vec!["thiever".into()];
    assert_eq!(changed.differences(&base), vec!["settings"]);

    let mut changed = base.clone();
    changed.settings.child_env_keys = vec!["BOT_DEBUG".into()];
    assert_eq!(changed.differences(&base), vec!["settings"]);
}

/// The loader smoke's input identity is its path *and* its bytes, and only a selection
/// that launches it resolves the file at all.
#[test]
fn external_source_binds_path_and_content_and_refuses_relative_inputs() {
    let fixture = host_play::external_loader::default_frozen_source();
    let default = ExternalSource::resolve(None).expect("the tracked fixture resolves");
    assert!(default.default_fixture);
    assert_eq!(default.sha256, host_play::external_loader::FROZEN_SHA256);
    assert!(
        default.frozen_match,
        "the tracked fixture is the frozen one"
    );
    assert_eq!(
        default.frozen_sha256,
        host_play::external_loader::FROZEN_SHA256
    );
    assert_eq!(
        std::fs::canonicalize(&fixture)
            .unwrap()
            .display()
            .to_string(),
        default.path
    );
    assert!(default.bytes > 0);
    // The whitespace transform the producer applies to its owned copy, hashed by the suite:
    // a real digest of the bound bytes plus one newline, not an assumed constant.
    let bytes = std::fs::read(&fixture).unwrap();
    let mut whitespace = bytes.clone();
    whitespace.push(b'\n');
    assert_eq!(
        default.harmless_whitespace_sha256,
        super::sha256(&whitespace)
    );
    assert_ne!(default.harmless_whitespace_sha256, default.sha256);

    let error = ExternalSource::resolve(Some(Path::new("ExampleBot.ts"))).unwrap_err();
    assert!(error.contains("relative"), "{error}");
    let error =
        ExternalSource::resolve(Some(Path::new("/definitely/missing/ExampleBot.ts"))).unwrap_err();
    assert!(error.contains("--external-ts"), "{error}");

    // A copy at another path with the same bytes is a different input identity.
    let dir = std::env::temp_dir().join(format!("274bot-external-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("ExampleBot.ts");
    std::fs::copy(&fixture, &copy).unwrap();
    let explicit = ExternalSource::resolve(Some(&copy)).unwrap();
    assert!(!explicit.default_fixture);
    assert!(explicit.frozen_match);
    assert_eq!(explicit.sha256, default.sha256);
    assert_ne!(explicit.path, default.path);
    assert_ne!(explicit, default, "the bound path is part of the identity");

    // The same path with changed bytes is a different identity, and records that the
    // producer's frozen contract no longer holds.
    std::fs::write(&copy, "// not the bot\n").unwrap();
    let changed = ExternalSource::resolve(Some(&copy)).unwrap();
    assert!(!changed.frozen_match);
    assert_ne!(changed.sha256, default.sha256);
    assert_ne!(changed, explicit);
    std::fs::remove_dir_all(dir).unwrap();
}

/// An ordinary core/pair identity carries no external component, and a resume refuses when
/// the bound external source (bytes or path) changes.
#[test]
fn an_external_source_difference_refuses_resume_and_is_absent_by_default() {
    let base = identity();
    assert!(
        base.external.is_none(),
        "a core/pair selection binds no external input"
    );

    let mut bound = base.clone();
    bound.external = Some(ExternalSource {
        path: "/tmp/ExampleBot.ts".into(),
        sha256: "sha-one".into(),
        bytes: 10,
        default_fixture: true,
        frozen_sha256: host_play::external_loader::FROZEN_SHA256.into(),
        frozen_match: true,
        harmless_whitespace_sha256: "sha-whitespace".into(),
    });
    assert_eq!(bound.differences(&base), vec!["external loader source"]);
    assert_eq!(base.differences(&bound), vec!["external loader source"]);

    let mut changed = bound.clone();
    changed.external.as_mut().unwrap().sha256 = "sha-two".into();
    assert_eq!(changed.differences(&bound), vec!["external loader source"]);

    let mut path_changed = bound.clone();
    path_changed.external.as_mut().unwrap().path = "/tmp/other.ts".into();
    assert_eq!(
        path_changed.differences(&bound),
        vec!["external loader source"]
    );

    let mut unbound = bound.clone();
    unbound.external = None;
    assert_eq!(unbound.differences(&bound), vec!["external loader source"]);
}

#[test]
fn unresolved_components_are_named_not_ignored() {
    let mut identity = identity();
    identity.host.content_sha256 = None;
    identity.profile.catalog.sha256 = None;
    identity.binaries.get_mut("core").unwrap().sha256 = None;
    assert_eq!(
        identity.unresolved(),
        vec![
            "host source content",
            "core executable content",
            "catalog script content"
        ]
    );
}

#[test]
fn hash_is_stable_and_content_addressed() {
    let base = identity();
    assert_eq!(base.hash(), base.hash());
    let mut changed = base.clone();
    changed.selection.push("other".into());
    assert_ne!(base.hash(), changed.hash());
    assert_eq!(sha256(b"274bot").len(), 64);
}

#[test]
fn unresolved_git_and_client_are_recorded_not_invented() {
    let missing = GitIdentity::capture(Path::new("/definitely/not/a/repo"));
    assert!(!missing.available);
    assert!(missing.commit.is_none());
    assert!(!missing.resolved());
    assert!(missing.note.is_some());
}

#[test]
fn tree_digests_bind_content_and_stay_bounded() {
    let root = std::env::temp_dir().join(format!("274bot-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("a.ts"), "one").unwrap();
    std::fs::write(root.join("nested/b.ts"), "two").unwrap();

    let first = InputDigest::tree(&root);
    assert!(first.resolved(), "{:?}", first.note);
    assert_eq!(first.files, 2);
    assert_eq!(first.bytes, Some(6));

    // Same path, same size, different bytes: the digest changes.
    std::fs::write(root.join("a.ts"), "eno").unwrap();
    let second = InputDigest::tree(&root);
    assert_ne!(first.sha256, second.sha256);

    let absent = InputDigest::absent(&root.join("missing"), "no vault file");
    assert!(absent.resolved(), "an absence is a defined identity");
    assert_ne!(absent.sha256, second.sha256);
    std::fs::remove_dir_all(root).unwrap();
}

/// A tree that cannot be bounded is unresolved *before* it is walked unbounded. Each
/// bound is checked where it applies: a symlink (which can leave the tree or loop) is
/// rejected rather than followed, an oversized file is rejected before its bytes are
/// read, and an entry that cannot be read is unresolved instead of silently dropped.
#[test]
fn a_tree_that_cannot_be_bounded_is_unresolved_not_walked_unbounded() {
    let root = std::env::temp_dir().join(format!("274bot-tree-bounds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::fs::write(root.join("real/a.ts"), "one").unwrap();

    // The file bound is enforced during the walk, not measured after it.
    let started = std::time::Instant::now();
    let bounded = InputDigest::tree_bounded(
        &root,
        Bounds {
            files: 0,
            ..Bounds::default()
        },
    );
    assert!(!bounded.resolved(), "{:?}", bounded.note);
    assert!(
        bounded
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("more than 0 files"),
        "{:?}",
        bounded.note
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    // A directory symlink is not followed: a link back to the tree's own parent would
    // otherwise loop without bound.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let started = std::time::Instant::now();
        let linked = InputDigest::tree(&root);
        assert!(!linked.resolved(), "{:?}", linked.note);
        assert!(
            linked
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("symlink"),
            "{:?}",
            linked.note
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "a symlink must be rejected, not followed"
        );
        std::fs::remove_file(root.join("loop")).unwrap();
    }

    // A file past the per-file bound is rejected before its bytes are read.
    let big = root.join("real/big.bin");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(MAX_DIGEST_FILE_BYTES + 1).unwrap();
    drop(file);
    let started = std::time::Instant::now();
    let oversized = InputDigest::tree(&root);
    assert!(!oversized.resolved(), "{:?}", oversized.note);
    assert!(
        oversized
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("past the suite's bounded digest"),
        "{:?}",
        oversized.note
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the bound is known from metadata, not by reading the file"
    );
    std::fs::remove_file(&big).unwrap();

    // A file the walker cannot read is unresolved, never silently skipped.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let secret = root.join("real/secret.ts");
        std::fs::write(&secret, "secret").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = InputDigest::tree(&root);
        assert!(!unreadable.resolved(), "{:?}", unreadable.note);
        assert!(
            unreadable
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("secret.ts"),
            "{:?}",
            unreadable.note
        );
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn catalog_digest_covers_the_script_sources_only() {
    let root = std::env::temp_dir().join(format!("274bot-catalog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src/bot/scripts/Thiever")).unwrap();
    std::fs::write(root.join("src/bot/scripts/index.ts"), "registry").unwrap();
    std::fs::write(root.join("src/bot/scripts/Thiever/index.ts"), "script").unwrap();
    std::fs::write(root.join("package.json"), "{\"name\":\"x\"}").unwrap();
    let digest = InputDigest::catalog(&root);
    assert!(digest.resolved(), "{:?}", digest.note);
    assert_eq!(digest.files, 2, "the package.json is not a script source");

    // A script edit changes the bound identity; an unrelated file does not.
    std::fs::write(root.join("src/bot/scripts/Thiever/index.ts"), "script v2").unwrap();
    let edited = InputDigest::catalog(&root);
    assert_ne!(digest.sha256, edited.sha256);
    std::fs::write(root.join("package.json"), "{\"name\":\"y\"}").unwrap();
    assert_eq!(edited.sha256, InputDigest::catalog(&root).sha256);
    std::fs::remove_dir_all(root).unwrap();

    // A catalog without the script tree is unresolved, never invented.
    let missing = std::env::temp_dir().join("274bot-catalog-absent");
    let _ = std::fs::remove_dir_all(&missing);
    std::fs::create_dir_all(&missing).unwrap();
    let digest = InputDigest::catalog(&missing);
    assert!(!digest.resolved());
    assert!(digest.note.is_some());
    std::fs::remove_dir_all(missing).unwrap();
}

#[test]
fn bind_input_follows_a_linked_file_and_only_not_found_is_absence() {
    let root = std::env::temp_dir().join(format!("274bot-bind-input-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("vault-bytes");
    std::fs::write(&target, "vault v1").unwrap();
    #[cfg(unix)]
    {
        let linked = root.join("vault");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let first = bind_input(&linked);
        assert!(first.resolved(), "{:?}", first.note);
        std::fs::write(&target, "vault v2").unwrap();
        let second = bind_input(&linked);
        assert_ne!(first.sha256, second.sha256, "followed content must change");

        let dangling = root.join("dangling");
        std::os::unix::fs::symlink(root.join("missing-target"), &dangling).unwrap();
        let digest = bind_input(&dangling);
        assert!(!digest.resolved(), "{:?}", digest.note);
        assert!(
            digest
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("dangling"),
            "{:?}",
            digest.note
        );
    }
    let absent = bind_input(&root.join("no-such"));
    assert!(absent.resolved(), "NotFound is a defined absence");
    assert_eq!(
        absent.sha256,
        InputDigest::absent(&root.join("no-such"), "x").sha256
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_past_the_digest_cap_is_unresolved_without_an_unbounded_read() {
    let path = std::env::temp_dir().join(format!("274bot-file-cap-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_DIGEST_FILE_BYTES + 1).unwrap();
    drop(file);
    let started = std::time::Instant::now();
    let digest = InputDigest::file(&path);
    assert!(!digest.resolved(), "{:?}", digest.note);
    assert!(
        digest
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("past the suite's bounded digest"),
        "{:?}",
        digest.note
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    std::fs::remove_file(&path).unwrap();
}

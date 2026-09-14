//! Real build output: the staged navigation bundle, the compiled identity
//! table and the canonical cache must agree, and the bundled fast path must
//! read + decode the staged pack without hashing it.
//!
//!   cargo test -p host-play --test bundled_nav_build -- --ignored --nocapture
//!
//! Prerequisites: a default application build in this target profile
//! (`cargo build -p panel --bin panel-play`, i.e. `BOT_NAV_BUILD=require`)
//! and the canonical local engine + cache for every published revision. The
//! test is `#[ignore]`d because the cached cargo test binary for the same
//! target profile is what it reads: a fresh clone without the canonical trees
//! cannot run it.

use std::path::{Path, PathBuf};

use host_play::profile::{CacheManifest, NavAvailability, ProfileEnvironment, ProfileOptions};
use host_play::{bundled_nav_identities, NavOrigin, SharedClientTemplate};

/// Same rule as the build script: `BOT_NAV_ENGINE_DIR`, else `ENGINE_DIR`,
/// else the revision's canonical local engine.
fn engine_dir(revision: u16) -> PathBuf {
    for key in ["BOT_NAV_ENGINE_DIR", "ENGINE_DIR"] {
        if let Some(dir) = std::env::var_os(key).map(PathBuf::from) {
            return dir;
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(if revision == 289 {
        "experiments/lostcity-289/engine"
    } else {
        "experiments/Server/engine"
    })
}

fn known_identities() -> Vec<CacheManifest> {
    serde_json::from_str(include_str!("../src/known-cache-identities.json"))
        .expect("known cache identities")
}

#[test]
#[ignore = "needs a default application build and the canonical cache for its revision"]
fn staged_bundle_selects_the_install_layout_without_runtime_hashing() {
    let table = bundled_nav_identities();
    assert!(
        !table.is_empty(),
        "no compiled navigation identity: run `cargo build -p panel --bin panel-play` \
         (default BOT_NAV_BUILD=require) before this check"
    );
    // The test binary runs from `<profile>/deps`; the install resource root a
    // plain executable resolves is the profile directory holding it.
    let exe = std::env::current_exe().expect("test binary path");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("test binary lives in the cargo profile directory")
        .to_path_buf();
    let known = known_identities();

    for row in table {
        let engine = engine_dir(row.revision);
        let cache_dir = engine.join("data/pack/client");
        let declared = known
            .iter()
            .find(|manifest| manifest.revision == row.revision)
            .unwrap_or_else(|| panic!("no known cache identity for revision {}", row.revision));
        let actual = CacheManifest::capture(row.revision, &cache_dir)
            .unwrap_or_else(|e| panic!("canonical cache {}: {e}", cache_dir.display()));
        assert_eq!(
            actual.identity(),
            row.cache_id,
            "the published identity is not this cache's identity; rebuild after the cache changes"
        );
        assert_eq!(&actual, declared, "the canonical cache is a known identity");

        let root = std::env::temp_dir().join(format!(
            "274bot-bundled-nav-{}-{}",
            std::process::id(),
            row.revision
        ));
        std::fs::create_dir_all(&root).expect("temp root");
        let manifest_path = root.join("cache-manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(declared).expect("manifest json"),
        )
        .expect("manifest write");

        let options = ProfileOptions {
            revision: Some(row.revision.to_string()),
            engine_dir: Some(engine),
            cache_dir: Some(cache_dir),
            unpack_dir: Some(root.join("unpack")),
            cache_manifest: Some(manifest_path),
            ..ProfileOptions::default()
        };
        let environment = ProfileEnvironment {
            home: Some(root.clone()),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..ProfileEnvironment::default()
        };
        let profile = options
            .resolve_with_env(None, &environment)
            .and_then(|selection| {
                selection.bind_with_nav_identities(
                    &host_play::progress::ProfileProgressObserver::default(),
                    table,
                    Some(profile_dir.as_path()),
                )
            })
            .unwrap_or_else(|e| panic!("bind: {e}"));

        match profile.nav_origin() {
            NavOrigin::Bundled { path, identity } => {
                assert_eq!(path, &profile_dir.join(&row.relative_path));
                assert!(path.is_file(), "staged pack {} is missing", path.display());
                assert_eq!(identity.nav_sha256, row.nav_sha256);
                let reach_path = path.with_extension("navreach");
                assert!(
                    reach_path.is_file(),
                    "staged reach {} is missing",
                    reach_path.display()
                );
                let canlight_path = path.with_extension("navcanlight");
                assert!(
                    canlight_path.is_file(),
                    "staged canlight {} is missing",
                    canlight_path.display()
                );
                assert!(
                    row.canlight_sha256.as_ref().is_some_and(|h| h.len() == 64),
                    "bundled canlight identity is required"
                );
                assert!(
                    row.canlight_identity
                        .as_ref()
                        .is_some_and(|h| h.len() == 64),
                    "bundled canlight policy identity is required"
                );
                let pack_bytes = std::fs::read(path).expect("staged pack");
                let world = nav::world::NavWorld::from_bytes(&pack_bytes).expect("decode pack");
                let expected = nav::paint::bake_reach(&world.collision, &world.graph);
                let side = nav::pack::decode_reach_sidecar(
                    &std::fs::read(&reach_path).expect("staged reach"),
                )
                .expect("decode reach");
                assert_eq!(side.bits, expected, "staged reach must equal bake_reach");
                assert_eq!(
                    nav::pack::sha256_hex(&side.binding),
                    row.nav_sha256,
                    "reach binding is the pack identity"
                );
                let canlight_bytes = std::fs::read(&canlight_path).expect("staged canlight");
                let canlight =
                    nav::pack::decode_canlight_sidecar(&canlight_bytes).expect("decode canlight");
                let policy = row
                    .canlight_identity
                    .as_deref()
                    .expect("canlight policy identity");
                let expected = nav::canlight::expected_header_binding(&row.nav_sha256, policy)
                    .expect("canlight binding");
                assert_eq!(
                    canlight.binding, expected,
                    "canlight binding is pack+policy identity"
                );
                assert_eq!(canlight.origin, world.collision.origin);
                assert_eq!(canlight.width, world.collision.width);
                assert_eq!(canlight.height, world.collision.height);
                println!(
                    "bundled nav: revision {} pack {} ({}) in {}",
                    row.revision,
                    row.nav_sha256,
                    row.format,
                    path.display()
                );
            }
            other => panic!("expected the staged bundle, selected {other:?}"),
        }
        assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
        let counters = profile.nav_load_counters();
        assert_eq!(
            (
                counters.pack_reads,
                counters.pack_hashes,
                counters.pack_decodes
            ),
            (1, 0, 1),
            "the bundled path reads and decodes the staged pack exactly once and never hashes it"
        );
        assert_eq!(
            (counters.reach_reads, counters.reach_hashes),
            (1, 0),
            "bundled reach is read once and never content-hashed"
        );
        assert_eq!(
            (counters.canlight_reads, counters.canlight_hashes),
            (1, 0),
            "bundled canlight is read once and never content-hashed"
        );
        assert!(profile.reach().is_some());
        assert!(profile.canlight().is_some());
        let template = SharedClientTemplate::load(profile).expect("shared template");
        assert!(template.world().is_some(), "bundled world decoded");
        std::fs::remove_dir_all(&root).ok();
    }
}

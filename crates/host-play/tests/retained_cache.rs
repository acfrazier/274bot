//! Read-only, explicit qualification against the operator's retained assets.
//! Run with --ignored; no connection is opened and no source is changed.
//! NAV_VERIFY_ROOT selects artifacts produced by the ordinary application build.
use host_play::{ProfileOptions, SharedClientTemplate};
use host_play::profile::ProfileEnvironment;
use std::path::PathBuf;

#[test]
#[ignore = "requires freshly staged nav artifacts and retained caches"]
fn rebuilt_artifacts_bind_decoded_inputs_and_actual_baker_sources() {
    use nav::manifest::{CacheManifest, NavManifest, hash_file};
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let root = PathBuf::from(std::env::var_os("NAV_VERIFY_ROOT").expect("NAV_VERIFY_ROOT"));
    for (revision, engine, snapshots) in [
        (274, "experiments/Server/engine", ".274bot/unpack"),
        (289, "experiments/lostcity-289/engine", ".274bot/unpack-289"),
    ] {
        let engine = home.join(engine);
        let cache_dir = engine.join("data/pack/client");
        let cache = CacheManifest::capture(revision, &cache_dir).unwrap();
        let id = nav::bake::decoded_identity(revision, &cache_dir, &home.join(snapshots)).unwrap();
        let pack = root.join(format!("nav/{revision}/274bot.navpack"));
        let manifest: NavManifest = serde_json::from_slice(&std::fs::read(nav::manifest::nav_manifest_path(&pack)).unwrap()).unwrap();
        let source = nav::bundle::source_digest(&engine.parent().unwrap().join("content"), &[&nav::bake::config_jag_for(revision, &cache_dir).unwrap()]).unwrap();
        assert_eq!(manifest.source_sha256.as_deref(), Some(source.as_str()));
        let hash = hash_file(&pack).unwrap();
        manifest.verify_pack(revision, &cache, &hash, Some(&id)).unwrap();
        for (extension, expected) in [("navflags", &manifest.flags_sha256), ("navreach", &manifest.reach_sha256), ("navcanlight", &manifest.canlight_sha256)] {
            assert_eq!(Some(hash_file(&pack.with_extension(extension)).unwrap()), *expected);
        }
        let row = host_play::BundledNavIdentity {
            revision, cache_id: cache.identity(), content_id: manifest.content_id.clone(),
            source_sha256: manifest.source_sha256.clone(), format: nav::pack::FORMAT_ID.into(),
            nav_sha256: hash.clone(), flags_sha256: manifest.flags_sha256.clone(),
            reach_sha256: manifest.reach_sha256.clone(), canlight_sha256: manifest.canlight_sha256.clone(),
            canlight_identity: None, relative_path: format!("nav/{revision}/274bot.navpack"),
        };
        assert!(host_play::nav_identity::select_nav_origin(&[row], Some(&root), revision, &id, false, &pack).unwrap().is_bundled());
        assert!(manifest.verify_pack(revision, &cache, &hash, Some(&"00".repeat(32))).is_err());
        if revision == 289 {
            // Independently retained public-packed snapshot, not a fabricated alias.
            let public = home.join(".274bot/unpack-289/6dcb7c4ad1b85372");
            let public_id = client::content_identity::compute_decoded_content_identity(289, &public, &public).unwrap().content_id_hex();
            let public_cache = CacheManifest::capture(289, &public).unwrap();
            assert_ne!(cache.identity(), public_cache.identity());
            assert_eq!(id, public_id);
            manifest.verify_pack(289, &public_cache, &hash, Some(&public_id)).unwrap();
        }
    }
}

#[test]
#[ignore = "requires the retained 274 and 289 engine caches"]
fn custom_endpoint_template_does_not_inherit_supported_server_facts() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    for (revision, engine) in [("274", "experiments/Server/engine"), ("289", "experiments/lostcity-289/engine")] {
        let options = ProfileOptions {
            revision: Some(revision.into()),
            engine_dir: Some(home.join(engine)),
            host: Some("127.0.0.1".into()),
            // Loopback plus a known cache is not enough: the game port must
            // disagree with that engine's world.json so this stays custom.
            port: Some(9),
            nav_pack: Some(home.join(".274bot/no-such-test-resource.navpack")),
            ..Default::default()
        };
        let env = ProfileEnvironment {
            home: Some(home.clone()),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..Default::default()
        };
        let profile = options.resolve_with_env(None, &env).unwrap().bind().unwrap();
        // The cache is a known one: raw API selection can return facts, but
        // the host profile's explicit custom-server boundary must win.
        assert!(api::game_data::for_optional_profile(profile.revision(), profile.cache_id()).unwrap().is_some());
        assert!(profile.game_data().is_none());
        let template = SharedClientTemplate::load(profile).unwrap();
        assert!(template.game_data().is_none(), "custom {revision} template inherited built-in facts");
    }
}

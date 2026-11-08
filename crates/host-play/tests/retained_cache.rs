//! Explicit custom-endpoint profiles do not inherit supported-server facts.
//! Run with --ignored against retained engine and client-cache assets.
//! Real local paths are resolved with the shared test-asset helper.
use host_play::profile::ProfileEnvironment;
use host_play::{ProfileOptions, SharedClientTemplate};

#[test]
#[ignore = "requires retained 274 and 289 engine/cache assets"]
fn custom_endpoint_template_does_not_inherit_supported_server_facts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let Some(home) =
        api::test_support::local_asset_path("HOME", "", "home directory with retained game data")
    else {
        return;
    };
    for (revision, engine_rel) in [
        ("274", "experiments/Server/engine"),
        ("289", "experiments/lostcity-289/engine"),
    ] {
        let engine_path = home.join(engine_rel);
        let Some(engine) = api::test_support::local_asset_path(
            "",
            &engine_path.to_string_lossy(),
            "retained engine directory",
        ) else {
            return;
        };
        let cache_path = engine.join("data/pack/client");
        let Some(cache_dir) = api::test_support::local_asset_path(
            "",
            &cache_path.to_string_lossy(),
            "retained client cache",
        ) else {
            return;
        };
        let options = ProfileOptions {
            revision: Some(revision.into()),
            engine_dir: Some(engine.clone()),
            cache_dir: Some(cache_dir),
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
        let profile = options
            .resolve_with_env(None, &env)
            .unwrap()
            .bind()
            .unwrap();
        let _selected_data = api::game_data::for_revision(profile.revision())
            .expect("embedded facts remain available for the selected revision");
        assert!(profile.game_data().is_none());
        assert!(matches!(
            profile.game_data_status(),
            host_play::GameDataStatus::Unsupported { .. }
        ));
        let template = SharedClientTemplate::load(profile).unwrap();
        assert!(
            template.game_data().is_none(),
            "custom {revision} template inherited built-in facts"
        );
    }
}

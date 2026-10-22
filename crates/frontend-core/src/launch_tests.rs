//! Profile preparation on the shared worker and vault passphrases.

#[test]
fn a_vault_opens_with_the_typed_or_the_legacy_trimmed_passphrase() {
    let dir = std::env::temp_dir().join(format!(
        "fc-vault-typed-unlock-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    // Made in the terminal with the spaces kept: only the typed text opens it.
    let terminal = dir.join("terminal");
    drop(vault::Vault::create(&terminal, " pad ").unwrap());
    assert!(super::open_vault_typed(&terminal, " pad ").is_ok());

    // Made by the 0.1.9.1 panel, which stored the trimmed text.
    let legacy = dir.join("legacy");
    drop(vault::Vault::create(&legacy, "pad").unwrap());
    assert!(super::open_vault_typed(&legacy, "  pad ").is_ok());

    assert!(matches!(
        super::open_vault_typed(&legacy, " nope "),
        Err(vault::VaultError::WrongPassphrase)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn profile_preparation_with_embedded_289_game_data_completes_on_shared_worker() {
    use super::{ProfileArgs, ProfileEnv, ProfileLauncher};
    use std::time::{Duration, Instant};

    let stores = crate::test_support::IsolatedStores::enter("profile-preparation-worker");
    let cache = stores.dir().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    // Embed the tiny repository fixture. Hosted CI needs no engine, downloaded
    // cache, navigation pack, or source checkout at runtime.
    macro_rules! write_fixtures {
        ($($name:literal),+ $(,)?) => {
            $(std::fs::write(
                cache.join($name),
                include_bytes!(concat!("../../host-play/tests/fixtures/profile/", $name)),
            ).unwrap();)+
        };
    }
    write_fixtures!(
        "config",
        "interface",
        "media",
        "sounds",
        "textures",
        "title",
        "versionlist",
        "wordenc",
        "manifest-289.json",
    );
    let options = host_play::ProfileOptions {
        revision: Some("289".into()),
        cache_dir: Some(cache.clone()),
        cache_manifest: Some(cache.join("manifest-289.json")),
        unpack_dir: Some(stores.dir().join("unpack")),
        nav_pack: Some(stores.dir().join("missing.navpack")),
        ..Default::default()
    };
    let environment = host_play::profile::ProfileEnvironment {
        home: Some(stores.home().to_path_buf()),
        engine_dir: Some(stores.dir().join("fixture-engine")),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..Default::default()
    };
    let selection = options.resolve_with_env(Some(289), &environment).unwrap();
    let mut launcher = ProfileLauncher::legacy_local();
    launcher
        .configure(
            ProfileArgs::from_host(options),
            ProfileEnv::from_host(environment),
        )
        .unwrap();
    let mut preparation = launcher.preparation(289).unwrap();
    // Cache fill and source attestation are separate concerns. Start at the
    // real Load stage, but retain submit, admission, and PreparationSteps::step.
    // This must fail if that step calls off-worker TemplatePreparation::step.
    preparation.load_fixture = Some(selection);
    let job = preparation.submit().unwrap();
    let started = Instant::now();
    while !job.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "profile preparation blocked its own shared worker"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let template = job
        .join()
        .unwrap_or_else(|error| panic!("profile preparation failed: {error}"));
    let data = template.0.game_data().expect("embedded R289 game data");
    assert!(
        data.try_quests().is_some(),
        "quest facts must be ready before play"
    );
}

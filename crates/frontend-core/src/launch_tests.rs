//! Profile preparation on the shared worker and vault passphrases.

use std::sync::Arc;

#[test]
fn opaque_unlock_error_keeps_its_borrowed_vault_cause() {
    let error = super::UnlockError::from_vault(vault::VaultError::WrongPassphrase);
    assert_eq!(error.stage(), super::UnlockStage::Vault);
    assert!(matches!(
        error.vault_error(),
        Some(vault::VaultError::WrongPassphrase)
    ));
    assert_eq!(
        error.to_string(),
        vault::VaultError::WrongPassphrase.to_string()
    );
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn opaque_unlock_error_keeps_the_operation_message_and_stage() {
    let error = super::UnlockError::new(super::UnlockStage::Launch, "launch refused".to_owned());
    assert_eq!(error.stage(), super::UnlockStage::Launch);
    assert_eq!(error.to_string(), "launch refused");
    assert!(error.vault_error().is_none());
    assert!(std::error::Error::source(&error).is_none());
}

const PROFILE_PREPARATION_CHILD: &str = "FRONTEND_CORE_PROFILE_PREPARATION_TEST_CHILD";

/// The child's exact filter: the full test path that runs the checks.
const PROFILE_PREPARATION_TEST: &str =
    "launch::tests::profile_preparation_with_embedded_289_game_data_completes_on_shared_worker";

/// Run one test from this binary in a child process and require that it ran.
///
/// libtest exits 0 when `--exact` matches nothing, so the exit status alone
/// does not prove the child executed its checks. Require libtest's summary for
/// exactly one passed test as well; a filter that matches nothing is reported
/// as a failure that names the filter.
fn run_child_test(filter: &str) -> Result<(), String> {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", filter, "--nocapture"])
        .env(PROFILE_PREPARATION_CHILD, "1")
        .output()
        .map_err(|error| format!("could not start the child for filter {filter:?}: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "child for filter {filter:?} exited with {}:\n{stdout}\n{stderr}",
            output.status
        ));
    }
    if !stdout.contains("test result: ok. 1 passed;") {
        return Err(format!(
            "child for filter {filter:?} ran no passing test (libtest exits 0 when a filter \
             matches nothing), expected \"test result: ok. 1 passed;\":\n{stdout}\n{stderr}"
        ));
    }
    Ok(())
}

#[test]
fn child_runner_reports_a_filter_that_matches_no_test() {
    let filter = "launch::tests::this_test_does_not_exist";
    let error = run_child_test(filter).expect_err("a filter that matches no test must fail");
    assert!(
        error.contains(filter),
        "the failure names the filter: {error}"
    );
    assert!(
        error.contains("ran no passing test"),
        "the failure names the zero-test case: {error}"
    );
}

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
fn launcher_resolves_vault_from_the_committed_roster_before_opening_it() {
    use super::{ProfileArgs, ProfileEnv, ProfileLauncher};
    use host_play::local_state::{LocalStateService, StateRoot};
    use host_play::{LoginKey, Servers};
    use std::time::Duration;

    let stores = crate::test_support::IsolatedStores::enter("profile-roster-before-vault");
    let root = StateRoot::new(stores.home().join(".274bot"));
    let mut servers = Servers::builtins();
    let selected = &mut servers.servers[0];
    selected.name = "operator".into();
    selected.vault = "operator.vault".into();
    selected.login_key = LoginKey::EngineDir {
        engine_dir: Some(stores.dir().join("engine")),
    };
    std::fs::create_dir_all(root.path()).unwrap();
    vault::write_private_file(
        &root.path().join("servers.json"),
        &serde_json::to_vec(&servers).unwrap(),
    )
    .unwrap();

    let service = LocalStateService::open_process(root.clone());
    service
        .wait_server_roster(Duration::from_secs(5))
        .expect("the state writer publishes the imported server roster");
    std::fs::write(
        root.path().join("servers.json"),
        b"not the committed roster",
    )
    .unwrap();

    let options = host_play::ProfileOptions {
        profile: Some("operator".into()),
        ..Default::default()
    };
    let environment = host_play::profile::ProfileEnvironment {
        home: Some(stores.home().to_path_buf()),
        working_dir: Some(stores.dir().to_path_buf()),
        ..Default::default()
    };
    let mut launcher = ProfileLauncher::legacy_local();
    launcher
        .configure(
            ProfileArgs::from_host(options),
            ProfileEnv::from_host_with_root(environment, root.clone()),
        )
        .unwrap();
    launcher.bind_local_state(service);

    assert_eq!(
        launcher.unbound_vault_path(None).unwrap(),
        root.path().join("operator.vault")
    );
}

#[test]
fn profile_preparation_with_embedded_289_game_data_completes_on_shared_worker() {
    if !api::test_support::game_data_available() {
        return;
    }
    if std::env::var_os(PROFILE_PREPARATION_CHILD).is_none() {
        run_child_test(PROFILE_PREPARATION_TEST)
            .unwrap_or_else(|error| panic!("profile preparation test child failed: {error}"));
        return;
    }
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
    );
    let options = host_play::ProfileOptions {
        revision: Some("289".into()),
        cache_dir: Some(cache.clone()),
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
    let root = host_play::local_state::StateRoot::new(stores.home().join(".274bot"));
    let local_state = host_play::local_state::LocalStateService::open_process(root.clone());
    let roster = local_state
        .wait_server_roster(Duration::from_secs(5))
        .expect("the state writer publishes its initial server roster");
    let selection = options
        .resolve_with_roster(Some(289), &environment, &root, &roster)
        .unwrap();
    let mut launcher = ProfileLauncher::legacy_local();
    launcher
        .configure(
            ProfileArgs::from_host(options),
            ProfileEnv::from_host_with_root(environment, root),
        )
        .unwrap();
    launcher.bind_local_state(Arc::clone(&local_state));
    let mut preparation = launcher.preparation(289).unwrap();
    // Start at the real Load stage, but retain submit, admission, and
    // PreparationSteps::step.
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
    let play = host_play::run_prepared_template_and_state(
        template.0.validate_for_play().unwrap(),
        false,
        Some(Arc::clone(&local_state)),
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
    .unwrap();
    let retained = play.local_state_for_test().unwrap();
    assert!(Arc::ptr_eq(retained, &local_state));
    assert_eq!(
        retained.server_roster().unwrap().version(),
        roster.version()
    );
}

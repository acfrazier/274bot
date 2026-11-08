//! Profile selection and actual shared-client construction, without game servers.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use client::io::{ClientRevision, Packet};
use host_play::profile::{NavAvailability, ProfileEnvironment};
use host_play::progress::{
    ProfileProgress, ProfileProgressObserver, ProfileProgressStage, ProfileProgressUnit,
};
use host_play::{parse_profile_args, LoginKey, ProfileOptions, Servers, SharedClientTemplate};

#[path = "support/upgrade_home.rs"]
mod upgrade_home;

static NEXT: AtomicUsize = AtomicUsize::new(0);
static CLIENTS: Mutex<()> = Mutex::new(());
const ARCHIVES: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
        for file in ARCHIVES {
            std::fs::copy(source.join(file), path.join(file)).unwrap();
        }
        Self(path)
    }
    fn write_public_roster(
        &self,
        worlds: impl IntoIterator<Item = host_play::public_worlds::PublicWorld>,
    ) -> PathBuf {
        let mut servers = Servers::builtins();
        servers
            .servers
            .iter_mut()
            .find(|profile| profile.name == "rs2b2t")
            .unwrap()
            .worlds = worlds.into_iter().map(Into::into).collect();
        servers.validate().unwrap();
        let directory = self.0.join(".274bot");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("servers.json");
        std::fs::write(&path, serde_json::to_vec(&servers).unwrap()).unwrap();
        path
    }
    fn env(&self) -> ProfileEnvironment {
        ProfileEnvironment {
            home: Some(self.0.clone()),
            engine_dir: Some(self.0.join("fixture-engine")),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..Default::default()
        }
    }
    fn options(&self, revision: u16) -> ProfileOptions {
        ProfileOptions {
            revision: Some(revision.to_string()),
            cache_dir: Some(self.0.clone()),
            asset_host: Some("127.0.0.1".into()),
            nav_pack: Some(self.0.join("missing.navpack")),
            ..Default::default()
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture_crc_body(fixture: &Fixture) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (index, name) in ARCHIVES.iter().enumerate() {
        let bytes = std::fs::read(fixture.0.join(name)).unwrap();
        checksums[index + 1] = Packet::getcrc(&bytes, 0, bytes.len());
    }
    let mut body = Packet::alloc(0);
    for checksum in checksums {
        body.p4(checksum);
    }
    let mut hash = 1234i32;
    for checksum in checksums {
        hash = hash.wrapping_shl(1).wrapping_add(checksum);
    }
    body.p4(hash);
    body.data()[..body.pos].to_vec()
}

fn serve_fixture_crc(
    fixture: &Fixture,
) -> (
    u16,
    std::sync::mpsc::Receiver<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = fixture_crc_body(fixture);
    let (served_tx, served_rx) = std::sync::mpsc::sync_channel(1);
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => {
                    // Accepted sockets inherit O_NONBLOCK from the listener on macOS/BSD.
                    let _ = socket.set_nonblocking(false);
                    break socket;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fixture client never requested /crc"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("fixture /crc accept: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match socket.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => request.extend_from_slice(&buffer[..read]),
                Err(error) => panic!("fixture /crc request: {error}"),
            }
        }
        assert_eq!(
            String::from_utf8_lossy(&request).split_whitespace().nth(1),
            Some("/crc")
        );
        let response = [
            b"HTTP/1.0 200 OK\r\nContent-Length: ".as_slice(),
            body.len().to_string().as_bytes(),
            b"\r\n\r\n",
            &body,
        ]
        .concat();
        socket.write_all(&response).unwrap();
        served_tx.send(()).unwrap();
    });
    (port, served_rx, server)
}

#[test]
fn flags_are_order_independent_and_override_saved_or_environment_revision() {
    let fixture = Fixture::new();
    let mut environment = fixture.env();
    environment.revision = Some("377".into());
    for args in [
        vec![
            "--revision",
            "289",
            "--host",
            "localhost",
            "--port",
            "45123",
            "--script",
            "foo",
        ],
        vec![
            "--port",
            "45123",
            "--script",
            "foo",
            "--host",
            "localhost",
            "--revision",
            "289",
        ],
    ] {
        let (options, rest) = parse_profile_args(args).unwrap();
        assert_eq!(rest, ["--script", "foo"]);
        let selected = options.resolve_with_env(Some(377), &environment).unwrap();
        assert_eq!(selected.revision(), ClientRevision::R289);
        assert_eq!(selected.game_host(), "localhost");
        assert_eq!(selected.game_port(), 45123);
        assert_eq!(selected.asset_port(), 1080);
        assert!(selected.vault_path().ends_with("vault-289"));
        assert!(selected.cache_dir().ends_with(".274bot/unpack-289"));
        assert!(selected.nav_pack().ends_with("289/274bot.navpack"));
    }
    let (options, _) = parse_profile_args(["--profile", "local-274"]).unwrap();
    environment.profile = Some("unsupported".into());
    let selected = options.resolve_with_env(Some(289), &environment).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R274);
    assert_eq!(selected.game_port(), 43594);
    assert!(selected.vault_path().ends_with(".274bot/vault"));

    let mut environment = fixture.env();
    environment.working_dir = Some(fixture.0.clone());
    let (options, _) =
        parse_profile_args(["--cache", "relative-cache", "--vault", "relative-vault"]).unwrap();
    let selected = options.resolve_with_env(None, &environment).unwrap();
    assert_eq!(selected.cache_dir(), fixture.0.join("relative-cache"));
    assert_eq!(selected.vault_path(), fixture.0.join("relative-vault"));
}

#[test]
fn unpack_root_inside_selected_engine_or_content_is_rejected() {
    let fixture = Fixture::new();
    let mut options = ProfileOptions {
        profile: Some("local-289".into()),
        engine_dir: Some(fixture.0.join("engine")),
        content_dir: Some(fixture.0.join("content")),
        nav_pack: Some(fixture.0.join("missing.navpack")),
        ..Default::default()
    };
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(selected.unpack_dir().ends_with(".274bot/unpack-289"));
    assert!(!selected.unpack_dir().starts_with(selected.content_dir()));
    assert!(!selected.unpack_dir().starts_with(selected.engine_dir()));

    for (label, root) in [
        ("content", fixture.0.join("content")),
        ("engine", fixture.0.join("engine")),
    ] {
        options.unpack_dir = Some(root.join("runtime"));
        let error = options.resolve_with_env(None, &fixture.env()).unwrap_err();
        assert!(
            error.contains("unpack root") && error.contains(label),
            "{error}"
        );
    }
}

#[test]
fn rs2b2t_defaults_and_named_profile_override_lower_priority_inputs() {
    let fixture = Fixture::new();
    let mut lower_priority = fixture.env();
    lower_priority.profile = Some("local-274".into());
    lower_priority.revision = Some("274".into());

    let (named, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
    let selected = named.resolve_with_env(Some(274), &lower_priority).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.transport(), client::Transport::Wss);
    assert_eq!(selected.profile_class(), host_play::ProfileClass::Remote);
    assert_eq!(selected.game_host(), "w1.rs2b2t.com");
    assert_eq!(selected.game_port(), 443);
    assert_eq!(selected.asset_host(), "w1.rs2b2t.com");
    assert_eq!(selected.asset_port(), 443);
    assert!(selected.unpack_dir().ends_with(".274bot/unpack-289"));
    assert_eq!(selected.cache_dir(), selected.unpack_dir());
    assert!(selected.nav_pack().ends_with(".274bot/289/274bot.navpack"));
    assert!(selected
        .nav_flags()
        .ends_with(".274bot/289/274bot.navflags"));
    assert_eq!(selected.content_dir(), fixture.0.join("content"));
    assert!(selected.vault_path().ends_with(".274bot/vault-prod"));

    let (rs2b2t, _) = parse_profile_args(["--rs2b2t"]).unwrap();
    let selected = rs2b2t.resolve_with_env(Some(274), &fixture.env()).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.profile_class(), host_play::ProfileClass::Remote);

    let mut named_environment = fixture.env();
    named_environment.profile = Some("public-289".into());
    let selected = ProfileOptions::default()
        .resolve_with_env(Some(274), &named_environment)
        .unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.profile_class(), host_play::ProfileClass::Remote);

    let mut legacy_environment = fixture.env();
    legacy_environment.legacy_target = Some("prod".into());
    assert!(ProfileOptions::default()
        .resolve_with_env(Some(274), &legacy_environment)
        .unwrap_err()
        .contains("BOT_TARGET was removed"));

    let local_default = ProfileOptions::default()
        .resolve_with_env(None, &fixture.env())
        .unwrap();
    assert_eq!(local_default.revision(), ClientRevision::R274);
    assert_eq!(
        local_default.profile_class(),
        host_play::ProfileClass::Local
    );
    let saved_local_289 = ProfileOptions::default()
        .resolve_with_env(Some(289), &fixture.env())
        .unwrap();
    assert_eq!(saved_local_289.revision(), ClientRevision::R289);
    assert_eq!(
        saved_local_289.profile_class(),
        host_play::ProfileClass::Local
    );

    let explicit = ProfileOptions {
        profile: Some("public-289".into()),
        cache_dir: Some("explicit-cache".into()),
        unpack_dir: Some("explicit-unpack".into()),
        nav_pack: Some("explicit.navpack".into()),
        nav_flags: Some("explicit.navflags".into()),
        content_dir: Some("explicit-content".into()),
        vault_path: Some("explicit-vault".into()),
        ..ProfileOptions::default()
    };
    let mut explicit_environment = fixture.env();
    explicit_environment.working_dir = Some(fixture.0.clone());
    let selected = explicit
        .resolve_with_env(Some(274), &explicit_environment)
        .unwrap();
    assert_eq!(selected.cache_dir(), fixture.0.join("explicit-cache"));
    assert_eq!(selected.unpack_dir(), fixture.0.join("explicit-unpack"));
    assert_eq!(selected.nav_pack(), fixture.0.join("explicit.navpack"));
    assert_eq!(selected.nav_flags(), fixture.0.join("explicit.navflags"));
    assert_eq!(selected.content_dir(), fixture.0.join("explicit-content"));
    assert_eq!(selected.vault_path(), fixture.0.join("explicit-vault"));
}

#[test]
fn local_profile_requires_engine_and_new_builtin_file_omits_engine_paths() {
    let fixture = Fixture::new();
    let mut environment = fixture.env();
    environment.engine_dir = None;
    let (options, _) = parse_profile_args(["--profile", "local-289"]).unwrap();
    let error = options.resolve_with_env(None, &environment).unwrap_err();
    assert!(
        error.contains("ENGINE_DIR")
            && error.contains("servers.json")
            && error.contains("--engine"),
        "{error}"
    );
    assert!(
        !error.contains(&fixture.0.display().to_string()),
        "the diagnostic must not invent a path under HOME: {error}"
    );

    let servers_json = std::fs::read_to_string(fixture.0.join(".274bot/servers.json")).unwrap();
    assert!(!servers_json.contains("engine_dir"), "{servers_json}");
}

#[test]
fn engine_dir_precedence_is_cli_then_environment_then_saved_profile() {
    let fixture = Fixture::new();
    let bot_dir = fixture.0.join(".274bot");
    std::fs::create_dir_all(&bot_dir).unwrap();
    let saved = fixture.0.join("saved-engine");
    let mut servers = Servers::builtins();
    let local_289 = servers
        .servers
        .iter_mut()
        .find(|profile| profile.name == "local-289")
        .unwrap();
    local_289.login_key = LoginKey::EngineDir {
        engine_dir: Some(saved.clone()),
    };
    let servers_path = bot_dir.join("servers.json");
    let original_json = serde_json::to_vec_pretty(&servers).unwrap();
    std::fs::write(&servers_path, &original_json).unwrap();

    let mut environment = fixture.env();
    let from_env = fixture.0.join("environment-engine");
    environment.engine_dir = Some(from_env.clone());
    environment.working_dir = Some(fixture.0.clone());
    let cli_engine = fixture.0.join("cli-engine");
    let (options, _) = parse_profile_args([
        "--profile".to_owned(),
        "local-289".to_owned(),
        "--engine".to_owned(),
        cli_engine.to_string_lossy().into_owned(),
    ])
    .unwrap();
    assert_eq!(
        options
            .resolve_with_env(None, &environment)
            .unwrap()
            .engine_dir(),
        fixture.0.join("cli-engine")
    );

    let without_cli = ProfileOptions {
        engine_dir: None,
        ..options.clone()
    };
    assert_eq!(
        without_cli
            .resolve_with_env(None, &environment)
            .unwrap()
            .engine_dir(),
        from_env
    );

    environment.engine_dir = None;
    assert_eq!(
        without_cli
            .resolve_with_env(None, &environment)
            .unwrap()
            .engine_dir(),
        saved
    );
    assert_eq!(std::fs::read(&servers_path).unwrap(), original_json);
}

#[test]
fn supported_public_roster_preserves_order_and_endpoint_checks() {
    let fixture = Fixture::new();
    let roster: host_play::public_worlds::PublicWorlds = serde_json::from_str(
        r#"{"schema_version":1,"worlds":[{"number":2,"host":"w2.rs2b2t.com","port":443,"node_id":11},{"number":1,"host":"w1.rs2b2t.com","port":443,"node_id":10}]}"#,
    )
    .unwrap();
    let servers_path = fixture.write_public_roster(roster.worlds);
    let (mut options, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert_eq!(
        (selected.game_host(), selected.game_port()),
        ("w2.rs2b2t.com", 443)
    );
    let original = std::fs::read(&servers_path).unwrap();
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert_eq!(selected.game_host(), "w2.rs2b2t.com");
    assert_eq!(std::fs::read(&servers_path).unwrap(), original);

    options.host = Some("attacker.example".into());
    assert!(options.resolve_with_env(None, &fixture.env()).is_err());
    options.host = Some("w1.rs2b2t.com".into());
    assert_eq!(
        options
            .resolve_with_env(None, &fixture.env())
            .unwrap()
            .game_host(),
        "w1.rs2b2t.com"
    );
}

#[test]
fn revision_377_selects_its_own_loopback_resources() {
    let fixture = Fixture::new();
    let (options, rest) = parse_profile_args(["--revision", "377"]).unwrap();
    assert!(rest.is_empty());
    let selected = options.resolve_with_env(Some(289), &fixture.env()).unwrap();
    assert_eq!(selected.name(), "local-377");
    assert_eq!(selected.revision(), ClientRevision::R377);
    assert_eq!(selected.transport(), client::Transport::Tcp);
    assert_eq!(selected.profile_class(), host_play::ProfileClass::Local);
    assert_eq!(
        (selected.game_host(), selected.game_port()),
        ("127.0.0.1", 43595)
    );
    assert_eq!(
        (selected.asset_host(), selected.asset_port()),
        ("127.0.0.1", 81)
    );
    assert!(selected.unpack_dir().ends_with(".274bot/unpack-377"));
    assert_eq!(selected.cache_dir(), selected.unpack_dir());
    assert!(selected.vault_path().ends_with(".274bot/vault-377"));
    assert!(selected.nav_pack().ends_with(".274bot/377/274bot.navpack"));
    assert!(selected
        .nav_flags()
        .ends_with(".274bot/377/274bot.navflags"));
    assert!(selected.supported_server());
    selected.require_bot_operation().unwrap();
    assert!(matches!(
        selected.selection().login_key,
        LoginKey::EngineDir { engine_dir: None }
    ));
}

#[test]
fn revision_377_environment_and_saved_selection_do_not_change_the_default() {
    let fixture = Fixture::new();
    let mut environment = fixture.env();
    environment.revision = Some("377".into());
    let from_environment = ProfileOptions::default()
        .resolve_with_env(Some(289), &environment)
        .unwrap();
    assert_eq!(from_environment.revision(), ClientRevision::R377);
    let from_saved = ProfileOptions::default()
        .resolve_with_env(Some(377), &fixture.env())
        .unwrap();
    assert_eq!(from_saved.revision(), ClientRevision::R377);
    let default = ProfileOptions::default()
        .resolve_with_env(None, &fixture.env())
        .unwrap();
    assert_eq!(default.revision(), ClientRevision::R274);
}

#[test]
fn revision_377_requires_an_engine_and_rejects_a_named_revision_conflict() {
    let fixture = Fixture::new();
    let (options, _) = parse_profile_args(["--revision", "377"]).unwrap();
    let mut environment = fixture.env();
    environment.engine_dir = None;
    assert!(options
        .resolve_with_env(None, &environment)
        .unwrap_err()
        .contains("ENGINE_DIR"));
    let (conflict, _) =
        parse_profile_args(["--profile", "local-289", "--revision", "377"]).unwrap();
    assert!(conflict
        .resolve_with_env(None, &fixture.env())
        .unwrap_err()
        .contains("--revision conflicts with the selected --profile"));
}

#[test]

fn removed_legacy_flags_and_profile_conflicts_fail_before_vault_access() {
    let fixture = Fixture::new();
    for args in [
        vec!["--revision", "378"],
        vec!["--revision"],
        vec!["--port", "0"],
        vec!["--revision", "--prod"],
        vec!["--prod"],
    ] {
        assert!(parse_profile_args(args).is_err());
    }
    for args in [
        vec!["--profile", "local-274", "--revision", "289"],
        vec!["--profile", "local-289", "--rs2b2t"],
        vec!["--profile", "public-274"],
        vec!["--profile", "public-289", "--host", "localhost"],
        vec!["--profile", "public-289", "--port", "43594"],
        vec!["--profile", "public-289", "--asset-host", "localhost"],
        vec!["--profile", "public-289", "--http-port", "80"],
        vec!["--rs2b2t", "--revision", "274"],
    ] {
        let (options, _) = parse_profile_args(args).unwrap();
        assert!(options.resolve_with_env(None, &fixture.env()).is_err());
    }
    let mut env_profile = fixture.env();
    env_profile.profile = Some("rs2b2t".into());
    let (options, _) = parse_profile_args(["--revision", "274"]).unwrap();
    let error = options.resolve_with_env(None, &env_profile).unwrap_err();
    assert!(
        error.contains("--revision conflicts with the selected --profile"),
        "{error}"
    );
    let mut legacy = fixture.env();
    legacy.legacy_target = Some("prod".into());
    assert!(ProfileOptions::default()
        .resolve_with_env(None, &legacy)
        .unwrap_err()
        .contains("BOT_TARGET was removed"));
    assert!(
        !fixture.0.join(".274bot/vault-prod").exists(),
        "profile resolution must not open a vault"
    );
}

#[test]
fn invalid_explicit_rsa_fails_without_fallback() {
    let fixture = Fixture::new();
    let options = fixture.options(274);
    for modulus in ["", "not-a-number", "0", "-23"] {
        let mut environment = fixture.env();
        environment.rsa_modulus = Some(modulus.into());
        assert!(options
            .resolve_with_env(None, &environment)
            .unwrap()
            .bind()
            .is_err());
    }
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert!(matches!(
        profile.nav_availability(),
        NavAvailability::Unavailable(_)
    ));
    assert!(!profile.vault_path().exists());
}

#[test]
fn observed_preparation_reports_only_completed_game_file_steps() {
    let fixture = Fixture::new();
    let updates = Arc::new(Mutex::new(Vec::<ProfileProgress>::new()));
    let worker_updates = Arc::clone(&updates);
    let observer = ProfileProgressObserver::new(move |progress| {
        worker_updates.lock().unwrap().push(progress);
    });
    let selection = fixture
        .options(274)
        .resolve_with_env(None, &fixture.env())
        .unwrap();

    let profile = selection.bind_with_progress(&observer).unwrap();
    let template = SharedClientTemplate::load_with_progress(profile, &observer).unwrap();
    template.validate_for_play_with_progress(&observer).unwrap();

    let updates = updates.lock().unwrap();
    let game_checks: Vec<_> = updates
        .iter()
        .filter(|progress| progress.stage == ProfileProgressStage::CheckingGameFiles)
        .collect();
    assert_eq!(game_checks.len(), ARCHIVES.len() + 1);
    assert_eq!(game_checks.first().unwrap().completed, 0);
    assert_eq!(game_checks.first().unwrap().total, ARCHIVES.len() as u64);
    assert!(
        game_checks.iter().enumerate().all(|(completed, progress)| {
            progress.completed == completed as u64
                && progress.total == ARCHIVES.len() as u64
                && progress.unit == ProfileProgressUnit::Files
        }),
        "game-file progress advances once for each archive read"
    );
    assert!(game_checks
        .windows(2)
        .all(|pair| pair[1].completed == 0 || pair[1].completed >= pair[0].completed));
    assert_eq!(
        game_checks
            .iter()
            .filter(|progress| progress.completed == progress.total)
            .count(),
        1,
        "one terminal event follows the completed archive reads"
    );
    let archive_reads: Vec<_> = updates
        .iter()
        .filter(|progress| progress.stage == ProfileProgressStage::ReadingCacheArchives)
        .collect();
    assert_eq!(archive_reads.first().unwrap().completed, 0);
    assert_eq!(archive_reads.last().unwrap().completed, 8);
    assert!(archive_reads
        .windows(2)
        .all(|pair| pair[1].completed >= pair[0].completed));
    let game_loads: Vec<_> = updates
        .iter()
        .filter(|progress| progress.stage == ProfileProgressStage::LoadingGameData)
        .collect();
    assert_eq!(game_loads.first().unwrap().completed, 0);
    assert_eq!(game_loads.last().unwrap().completed, 4);
    assert!(game_loads
        .windows(2)
        .all(|pair| pair[1].completed >= pair[0].completed));
    assert_eq!(
        updates.last(),
        Some(&ProfileProgress::steps(
            ProfileProgressStage::FinalChecks,
            1,
            1
        ))
    );
}

#[test]
fn both_revisions_reach_real_shared_client_constructor_and_keep_the_binding() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _clients = CLIENTS.lock().unwrap();
    for revision in [274, 289] {
        let fixture = Fixture::new();
        let options = fixture.options(revision);
        let profile = options
            .resolve_with_env(None, &fixture.env())
            .unwrap()
            .bind()
            .unwrap();
        let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
        let mut first = template.prepare_client(740_001, false).unwrap();
        let second = template.prepare_client(740_002, true).unwrap();
        assert_eq!(first.revision().as_i32(), i32::from(revision));
        assert_eq!(first.login_uid, 740_001);
        assert_eq!(second.login_uid, 740_002);
        assert!(Arc::ptr_eq(
            first.session_profile().unwrap(),
            profile.client()
        ));
        assert!(Arc::ptr_eq(
            first.session_profile().unwrap(),
            second.session_profile().unwrap()
        ));
        assert!(Arc::ptr_eq(&first.cache, &second.cache));
        assert!(Arc::ptr_eq(&first.ifaces, &second.ifaces));
        assert_eq!(first.cache.objs[0].name, "Fixture obj");
        assert_eq!(first.session_transport(), client::Transport::Tcp);
        assert_eq!(first.cheat_admission(), client::CheatAdmission::Granted);
        first.config.host = "invalid.invalid".into();
        first.config.port = 1;
        first.config.cache_dir = "invalid-cache".into();
        assert_eq!(first.session_profile().unwrap().game_host(), "127.0.0.1");
        assert_eq!(
            first.session_profile().unwrap().cache_dir(),
            fixture.0.as_path()
        );
        assert_eq!(template.scatter_tile_for(42), template.scatter_tile_for(42));
    }
}

#[test]
fn shared_template_client_members_follow_world_members_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    for (uid, members) in [(740_101, false), (740_102, true)] {
        let mut options = fixture.options(289);
        options.world_members = Some(members);
        let profile = options
            .resolve_with_env(None, &fixture.env())
            .unwrap()
            .bind()
            .unwrap();
        let template = SharedClientTemplate::load(profile).unwrap();
        let client = template.prepare_client(uid, false).unwrap();
        assert_eq!(
            client.config.members, members,
            "bound client must use WORLD membership, not a hard-coded account flag"
        );
    }
}
#[test]
fn stop_slot_aborts_an_unreachable_asset_retry_promptly() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    let dead_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    dead_listener.set_nonblocking(true).unwrap();
    let dead_port = dead_listener.local_addr().unwrap().port();
    let mut options = fixture.options(289);
    options.asset_host = Some("127.0.0.1".into());
    options.http_port = Some(dead_port);
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let account = vault::Profile {
        username: "stop-fixture".into(),
        password: "fixture".into(),
        uid: 43,
        settings: Default::default(),
    };
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
    .unwrap();
    play.try_spawn_slot(
        account.clone(),
        None,
        None,
        Some(host_play::SlotArm::new(account.uid, false)),
    )
    .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let _ = dead_listener.accept();
        if play.statuses().iter().any(|status| {
            status.username == account.username
                && status.startup_progress_message.contains("Will retry in")
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "slot never entered the unreachable-server retry countdown"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(0);
    let stop = std::thread::spawn(move || {
        play.stop_slot("stop-fixture");
        done_tx.send(()).unwrap();
    });
    done_rx
        .recv_timeout(std::time::Duration::from_millis(1_500))
        .expect("stop_slot waited for the five-second asset retry tick");
    stop.join().unwrap();
}

#[test]
fn qualified_revision_289_accepts_an_unarmed_slot_and_script_loading() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    let (asset_port, served, server) = serve_fixture_crc(&fixture);
    let mut options = fixture.options(289);
    options.asset_host = Some("127.0.0.1".into());
    options.http_port = Some(asset_port);
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let account = vault::Profile {
        username: "fixture".into(),
        password: "fixture".into(),
        uid: 42,
        settings: Default::default(),
    };
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
    .unwrap();
    // An unarmed slot exercises the real spawn path without a login handshake.
    play.try_spawn_slot(
        account.clone(),
        None,
        None,
        Some(host_play::SlotArm::new(account.uid, false)),
    )
    .unwrap();
    served
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("slot did not reach the local /crc fixture");
    server.join().unwrap();
    assert!(play.arm("fixture").is_some());
    assert!(play.login_queue_uids().is_empty());
    #[cfg(feature = "js")]
    {
        // The loader/start handle accepts the user's script under revision 289.
        // Runtime actions remain subject to the actual host/client capabilities.
        play.script_start_handle()
            .start_load(
                "fixture",
                "export function tick(api) {}".into(),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .unwrap();
        play.script_stop("fixture");
    }
    play.stop_slot("fixture");
    assert!(play.arm("fixture").is_none());
    assert!(!profile.vault_path().exists());
}

#[test]
fn bound_public_client_refuses_fixture_cheats_after_mutable_config_changes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    let mut options = fixture.options(289);
    options.rs2b2t = true;
    options.asset_host = None;
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.profile_class(), host_play::ProfileClass::Remote);
    assert_eq!(profile.transport(), client::Transport::Wss);
    assert_eq!(profile.revision(), ClientRevision::R289);
    let template = SharedClientTemplate::load(profile).unwrap();
    let mut client = template.prepare_client(740_003, true).unwrap();
    client.config.host = "127.0.0.1".into();
    client.config.port = 43594;
    let position = client.out.pos;
    assert_eq!(
        api::interact::cheat(&mut client, "ping"),
        client::CheatSend::Refused(client::CheatRefusal::RemoteProfile {
            transport: client::Transport::Wss,
        })
    );
    api::interact::mainland_hop(&mut client);
    assert_eq!(client.out.pos, position);
}

#[test]
fn catalog_is_a_default_path_and_binding_does_not_read_the_script_tree() {
    let fixture = Fixture::new();
    let root = fixture.0.join("catalog");
    std::fs::create_dir_all(root.join("src/bot")).unwrap();
    std::fs::write(root.join("src/bot/Fixture.js"), "export default {};").unwrap();
    let mut options = fixture.options(274);
    options.catalog_root = Some(root.clone());
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.catalog_root(), Some(root.as_path()));
    std::fs::write(
        root.join("src/bot/Fixture.js"),
        "export default { changed: true };",
    )
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    // Binding a server neither reads nor qualifies the user's script tree.
    options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
}

#[test]
fn navigation_and_scatter_use_the_selected_shared_world_and_keep_it_after_disk_edits() {
    use api::snapshot::WorldTile;
    use nav_planner::collision::{pack_walk, WorldCollision};
    use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};
    let fixture = Fixture::new();
    let mut options = fixture.options(289);
    let origin = WorldTile {
        x: 4500,
        z: 4600,
        level: 0,
    };
    let adjacent = WorldTile { x: 4501, ..origin };
    let (walk, blocked) = pack_walk(&[0; 8]);
    let collision = WorldCollision {
        origin,
        width: 2,
        height: 1,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: origin,
        to: adjacent,
        loc_id: 1,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });
    let bytes = nav_bake::pack::encode(&collision, &graph, &[]).unwrap();
    let flags = nav_bake::pack::encode_flags_sidecar(origin, 2, 1, &[0u32; 2]);
    let pack = fixture.0.join("selected.navpack");
    let flags_path = fixture.0.join("selected.navflags");
    std::fs::write(&pack, &bytes).unwrap();
    std::fs::write(&flags_path, &flags).unwrap();
    options.nav_pack = Some(pack.clone());
    options.nav_flags = Some(flags_path.clone());
    let selection = options.resolve_with_env(None, &fixture.env()).unwrap();
    let updates = Arc::new(Mutex::new(Vec::<ProfileProgress>::new()));
    let worker_updates = Arc::clone(&updates);
    let observer = ProfileProgressObserver::new(move |progress| {
        worker_updates.lock().unwrap().push(progress);
    });
    let profile = selection.bind_with_progress(&observer).unwrap();
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    assert_eq!(profile.nav_pack(), pack.as_path());
    assert_eq!(profile.nav_load_counters().pack_reads, 1);
    assert_eq!(profile.nav_load_counters().pack_decodes, 1);
    let stages: Vec<_> = updates
        .lock()
        .unwrap()
        .iter()
        .map(|progress| progress.stage)
        .collect();
    assert!(stages.contains(&ProfileProgressStage::PreparingNavigation));
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let first = template.world().unwrap();
    let second = template.world().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &profile.world().unwrap()));
    assert_eq!(first.collision.origin, origin);
    for uid in [0, 1, -5, i32::MIN] {
        assert!([origin, adjacent].contains(&template.scatter_tile_for(uid)));
    }
    std::fs::write(&flags_path, b"different flags").unwrap();
    std::fs::write(&pack, b"different navigation").unwrap();
    template
        .validate_for_play()
        .expect("already-loaded world is not re-read at Play");
    assert!(Arc::ptr_eq(&first, &template.world().unwrap()));

    let error = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap_err();
    assert!(
        error.contains("pack") || error.contains("navigation"),
        "{error}"
    );
    std::fs::write(&pack, &bytes).unwrap();
    let rebound_profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let rebound_world = rebound_profile.world().unwrap();
    assert!(!Arc::ptr_eq(&first, &rebound_world));
    assert_eq!(rebound_world.collision.origin, origin);
}

#[test]
fn consuming_ticket_does_not_rehash_cache() {
    let fixture = Fixture::new();
    let profile = fixture
        .options(274)
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let config = fixture.0.join("config");
    let ticket = template.validate_for_play().unwrap();
    std::fs::write(&config, b"changed after the final validation").unwrap();
    let play =
        host_play::run_prepared_template(ticket, false, vec![], |_| (None, None), |_, _, _| {})
            .unwrap();
    assert!(Arc::ptr_eq(play.server_profile().unwrap(), &profile));
}

fn tiny_pack() -> Vec<u8> {
    use api::snapshot::WorldTile;
    use nav_planner::collision::{pack_walk, WorldCollision};
    use nav_planner::transport::TransportGraph;
    let (walk, blocked) = pack_walk(&[0; 8]);
    nav_bake::pack::encode(
        &WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        },
        &TransportGraph::default(),
        &[],
    )
    .unwrap()
}

fn tiny_pack_at(x: i32) -> Vec<u8> {
    use api::snapshot::WorldTile;
    use nav_planner::collision::{pack_walk, WorldCollision};
    use nav_planner::transport::TransportGraph;
    let (walk, blocked) = pack_walk(&[0; 8]);
    nav_bake::pack::encode(
        &WorldCollision {
            origin: WorldTile {
                x,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        },
        &TransportGraph::default(),
        &[],
    )
    .unwrap()
}

fn write_revision_sidecars(root: &Path, pack: &[u8], revision: u16) {
    let world = nav_planner::world::NavWorld::from_bytes(pack).unwrap();
    let layout = host_play::nav_assets::artifact_layout(revision);
    let pack_path = root.join(&layout.relative_pack);
    let flags_path = root.join(&layout.relative_flags);
    let reach_path = root.join(&layout.relative_reach);
    let canlight_path = root.join(&layout.relative_canlight);
    for path in [&pack_path, &flags_path, &reach_path, &canlight_path] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    std::fs::write(&pack_path, pack).unwrap();
    std::fs::write(
        flags_path,
        nav_bake::pack::encode_flags_sidecar(world.collision.origin, 2, 1, &[0u32; 2]),
    )
    .unwrap();
    let reach = nav_planner::paint::bake_reach(&world.collision, &world.graph);
    std::fs::write(
        reach_path,
        nav_bake::pack::encode_reach_sidecar(
            world.collision.origin,
            world.collision.width,
            world.collision.height,
            &reach,
        ),
    )
    .unwrap();
    std::fs::write(
        canlight_path,
        nav_bake::pack::encode_canlight_sidecar(
            world.collision.origin,
            world.collision.width,
            world.collision.height,
            &[0],
        ),
    )
    .unwrap();
}

#[test]
fn bundled_payloads_decode_normally_and_memoize_shared_sidecars() {
    let fixture = Fixture::new();
    let root = fixture.0.join("Resources");
    let bytes = tiny_pack();
    write_revision_sidecars(&root, &bytes, 289);
    let layout = host_play::nav_assets::artifact_layout(289);
    let pack = root.join(layout.relative_pack);
    let mut options = fixture.options(289);
    options.nav_pack = None;
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind_with_resource_root(&ProfileProgressObserver::default(), Some(&root))
        .unwrap();
    assert_eq!(profile.nav_pack(), pack.as_path());
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    assert_eq!(profile.nav_load_counters().pack_reads, 1);
    assert_eq!(profile.nav_load_counters().pack_decodes, 1);
    let first_reach = profile.reach().expect("staged reach sidecar");
    let reach_reads = profile.nav_load_counters().reach_reads;
    assert!(reach_reads > 0);
    assert!(Arc::ptr_eq(
        &first_reach,
        &profile.reach().expect("memoized reach sidecar")
    ));
    assert_eq!(profile.nav_load_counters().reach_reads, reach_reads);
    let first_canlight = profile.canlight().expect("staged canlight sidecar");
    let canlight_reads = profile.nav_load_counters().canlight_reads;
    assert!(canlight_reads > 0);
    assert!(Arc::ptr_eq(
        &first_canlight,
        &profile.canlight().expect("memoized canlight sidecar")
    ));
    assert_eq!(profile.nav_load_counters().canlight_reads, canlight_reads);
    let first = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let second = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    assert!(Arc::ptr_eq(
        &first.world().unwrap(),
        &second.world().unwrap()
    ));
    std::fs::write(&pack, b"tampered").unwrap();
    first.validate_for_play().unwrap();
    assert!(Arc::ptr_eq(
        &first.world().unwrap(),
        &profile.world().unwrap()
    ));
}

#[test]
fn explicit_pack_and_flags_paths_override_revision_resources() {
    let fixture = Fixture::new();
    let root = fixture.0.join("Resources");
    write_revision_sidecars(&root, &tiny_pack(), 289);
    let custom = fixture.0.join("custom.navpack");
    let custom_bytes = tiny_pack_at(3300);
    std::fs::write(&custom, &custom_bytes).unwrap();
    let custom_flags = fixture.0.join("custom.navflags");
    std::fs::write(
        &custom_flags,
        nav_bake::pack::encode_flags_sidecar(
            api::snapshot::WorldTile {
                x: 3300,
                z: 3200,
                level: 0,
            },
            2,
            1,
            &[0u32; 2],
        ),
    )
    .unwrap();
    let mut options = fixture.options(289);
    options.nav_pack = Some(custom.clone());
    options.nav_flags = Some(custom_flags.clone());
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind_with_resource_root(&ProfileProgressObserver::default(), Some(&root))
        .unwrap();
    assert_eq!(profile.nav_pack(), custom.as_path());
    assert_eq!(profile.nav_flags(), custom_flags.as_path());
    assert_eq!(profile.world().unwrap().collision.origin.x, 3300);
}

#[test]
fn malformed_explicit_packs_report_format_errors() {
    let fixture = Fixture::new();
    let current = tiny_pack();
    let mut unsupported = current.clone();
    unsupported[4] = nav_planner::pack::VERSION + 1;
    let mut bad_magic = current;
    bad_magic[..4].copy_from_slice(b"nope");
    for (name, bytes, expected) in [
        (
            "unsupported.navpack",
            unsupported,
            "unsupported pack version",
        ),
        ("bad-magic.navpack", bad_magic, "bad pack magic"),
    ] {
        let pack = fixture.0.join(name);
        std::fs::write(&pack, bytes).unwrap();
        let mut options = fixture.options(289);
        options.nav_pack = Some(pack);
        let error = options
            .resolve_with_env(None, &fixture.env())
            .unwrap()
            .bind()
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn missing_explicit_pack_is_unavailable_without_reading() {
    let fixture = Fixture::new();
    let profile = fixture
        .options(289)
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert!(matches!(
        profile.nav_availability(),
        NavAvailability::Unavailable(_)
    ));
    assert_eq!(profile.nav_load_counters().pack_reads, 0);
}

fn assert_external_old_pack_reports_format_error(bytes: &[u8]) {
    let fixture = Fixture::new();
    let pack = fixture.0.join(format!("old-v{}.navpack", bytes[4]));
    std::fs::write(&pack, bytes).unwrap();
    let mut options = fixture.options(289);
    options.nav_pack = Some(pack.clone());

    let error = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap_err();
    assert!(error.contains("unsupported pack version"), "{error}");
    assert_eq!(std::fs::read(pack).unwrap(), bytes);
}

fn version_patched_pack(version: u8) -> Vec<u8> {
    let mut bytes = tiny_pack();
    bytes[4] = version;
    bytes
}

#[test]
fn external_v8_pack_reports_format_error_without_deleting_user_file() {
    assert_external_old_pack_reports_format_error(&version_patched_pack(8));
}

#[test]
fn external_v9_pack_reports_format_error_without_deleting_user_file() {
    assert_external_old_pack_reports_format_error(&version_patched_pack(9));
}

/// The pack 0.1.9.x wrote, byte for byte (`274V10` encoder output).
#[test]
fn external_real_v10_pack_reports_format_error_without_deleting_user_file() {
    assert!(matches!(
        nav_planner::pack::decode(upgrade_home::V10_PACK),
        Err(nav_planner::pack::PackError::BadVersion(10))
    ));
    assert_external_old_pack_reports_format_error(upgrade_home::V10_PACK);
}

/// A stale home pack never shadows the staged format-19 revision bundle.
/// The packaged world is shared by every template.
#[test]
fn packaged_v19_bundle_supersedes_a_stale_v10_home_pack() {
    let home = upgrade_home::UpgradeHome::new();
    let profile = home.bind(None);
    let layout = host_play::nav_assets::artifact_layout(289);
    let expected_pack = home.resources.join(layout.relative_pack);
    assert_eq!(profile.nav_pack(), expected_pack.as_path());
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    assert_eq!(profile.nav_load_counters().pack_decodes, 1);
    let world = profile.world().expect("the staged world decodes");
    let first = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let second = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    assert!(Arc::ptr_eq(&first.world().unwrap(), &world));
    assert!(Arc::ptr_eq(&second.world().unwrap(), &world));
    assert!(home.old_pack_untouched());
}

#[test]
fn revision_274_grid_pack_without_sidecar_binds_and_decodes_once() {
    let fixture = Fixture::new();
    let bytes = nav_bake::pack::encode_grid(&nav_planner::grid::StepGrid::fixture_door_corridor());
    let pack = fixture.0.join("legacy.navpack");
    std::fs::write(&pack, &bytes).unwrap();
    let mut options = fixture.options(274);
    options.nav_pack = Some(pack);
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    assert_eq!(profile.nav_load_counters().pack_reads, 1);
    assert_eq!(profile.nav_load_counters().pack_decodes, 1);
    assert_eq!(profile.world().unwrap().collision.width, 5);
}

#[test]
fn bundled_and_external_sidecars_expose_identical_capabilities() {
    let fixture = Fixture::new();
    let bytes = tiny_pack();
    let bundled_root = fixture.0.join("Resources");
    write_revision_sidecars(&bundled_root, &bytes, 289);
    let layout = host_play::nav_assets::artifact_layout(289);
    let bundled_pack = bundled_root.join(&layout.relative_pack);
    let mut options = fixture.options(289);
    options.nav_pack = None;
    let bundled = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind_with_resource_root(&ProfileProgressObserver::default(), Some(&bundled_root))
        .unwrap();

    let external_root = fixture.0.join("external");
    write_revision_sidecars(&external_root, &bytes, 289);
    let external_pack = external_root.join(layout.relative_pack);
    options.nav_pack = Some(external_pack);
    let external = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(bundled.nav_pack(), bundled_pack.as_path());
    assert_ne!(bundled.nav_pack(), external.nav_pack());
    assert_eq!(
        external.canlight().as_deref(),
        bundled.canlight().as_deref()
    );
    assert_eq!(external.reach().as_deref(), bundled.reach().as_deref());
    assert_eq!(external.reach().as_deref(), Some(&[0u64][..]));
    assert_eq!(external.canlight().as_deref(), Some(&[0u64][..]));
    assert_eq!(
        std::fs::read(bundled.nav_pack()).unwrap(),
        std::fs::read(external.nav_pack()).unwrap()
    );
}

fn write_world_json(engine: &Path, revision: u64, port: u64, members: &str) {
    write_world_json_with_web(engine, revision, port, None, members);
}

fn write_world_json_with_web(
    engine: &Path,
    revision: u64,
    port: u64,
    web_port: Option<u64>,
    members: &str,
) {
    let dir = engine.join("data/config");
    std::fs::create_dir_all(&dir).unwrap();
    let web = web_port
        .map(|port| format!(r#","web":{{"port":{port}}}"#))
        .unwrap_or_default();
    std::fs::write(
        dir.join("world.json"),
        format!(
            r#"{{"engine":{{"revision":{revision}}},"node":{{"port":{port},"members":{members}}}{web}}}"#
        ),
    )
    .unwrap();
}

#[test]
fn world_members_parses_true_false_and_rejects_invalid() {
    let (ok, _) = parse_profile_args(["--world-members", "true"]).unwrap();
    assert_eq!(ok.world_members, Some(true));
    let (ok, _) = parse_profile_args(["--world-members", "false"]).unwrap();
    assert_eq!(ok.world_members, Some(false));
    let err = parse_profile_args(["--world-members", "yes"]).unwrap_err();
    assert!(err.contains("--world-members"), "{err}");
}

#[test]
fn local_world_json_binds_only_when_revision_and_bool_match() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json(&engine, 274, 43594, "true");
    let env = fixture.env();
    let (options, _) = parse_profile_args([
        "--profile",
        "local-274",
        "--engine",
        engine.to_str().unwrap(),
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(selected.map_members());
    assert_eq!(
        selected.world_members().world_truth(),
        api::selected::Truth::True
    );
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::LocalWorldJson { .. }
        }
    ));

    write_world_json(&engine, 274, 43594, "false");
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(!selected.map_members());
    assert_eq!(
        selected.world_members().world_truth(),
        api::selected::Truth::False
    );
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known { members: false, .. }
    ));

    write_world_json(&engine, 289, 43594, "true");
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(!selected.map_members());
    assert_eq!(
        selected.world_members(),
        &host_play::WorldMembersFact::Unknown
    );
    // A bound profile that declares nothing is free-to-play for scripts.
    assert_eq!(
        selected.world_members().world_truth(),
        api::selected::Truth::False
    );

    // Engine ports are independent from forwarded connect ports.
    write_world_json(&engine, 274, 1, "true");
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known { members: true, .. }
    ));

    write_world_json(&engine, 274, 43594, "1");
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert_eq!(
        selected.world_members(),
        &host_play::WorldMembersFact::Unknown
    );
}

#[test]
fn public_profile_never_inherits_local_world_json() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json(&engine, 289, 443, "false");
    let env = fixture.env();
    let (options, _) = parse_profile_args([
        "--profile",
        "public-289",
        "--engine",
        engine.to_str().unwrap(),
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(selected.map_members());
    assert!(!matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            source: host_play::WorldMembersSource::LocalWorldJson { .. },
            ..
        }
    ));
}

#[test]
fn public_membership_requires_every_configured_world_to_be_rs2b2t() {
    use host_play::public_worlds::PublicWorlds;
    let resolve = |roster: &PublicWorlds, explicit: Option<bool>| {
        let fixture = Fixture::new();
        fixture.write_public_roster(roster.worlds.iter().cloned());
        let (mut options, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
        options.world_members = explicit;
        options.resolve_with_env(None, &fixture.env()).unwrap()
    };
    let bundled = PublicWorlds::default();
    for world in &bundled.worlds {
        let mut roster = bundled.clone();
        roster.worlds = vec![world.clone()];
        assert!(
            resolve(&roster, None).map_members(),
            "configured {} is a members host",
            world.host
        );
    }
    for host in ["w3.rs2b2t.com", "W42.RS2B2T.COM", "w001.rs2b2t.com"] {
        let mut roster = bundled.clone();
        roster.worlds.truncate(1);
        roster.worlds[0].host = host.into();
        roster.worlds[0].number = 17;
        roster.worlds[0].node_id = 52;
        let selected = resolve(&roster, None);
        assert!(
            selected.map_members(),
            "{host}: {:?}",
            selected.world_members()
        );
    }
    for (host, port) in [
        ("w1.rs2b2t.com", 444),
        ("rs2b2t.com.evil.net", 443),
        ("w1.rs2b2t.com.x", 443),
        ("w.rs2b2t.com", 443),
        ("w1a.rs2b2t.com", 443),
        ("w1.2.rs2b2t.com", 443),
        ("x1.rs2b2t.com", 443),
        ("w1.evilrs2b2t.com", 443),
    ] {
        let mut roster = bundled.clone();
        roster.worlds[0].host = host.into();
        roster.worlds[0].port = port;
        let selected = resolve(&roster, None);
        assert_eq!(
            selected.world_members(),
            &host_play::WorldMembersFact::Unknown,
            "{host}:{port}"
        );
    }
    for mixed in [false, true] {
        let mut roster = bundled.clone();
        let custom = roster.worlds.last_mut().unwrap();
        custom.host = "custom.example".into();
        if !mixed {
            roster.worlds.remove(0);
        }
        let selected = resolve(&roster, None);
        assert_eq!(
            selected.world_members(),
            &host_play::WorldMembersFact::Unknown
        );
        assert!(!selected.map_members());
    }
    let selected = resolve(&bundled, Some(false));
    assert!(!selected.map_members());
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: false,
            source: host_play::WorldMembersSource::ExplicitOverride
        }
    ));
}

/// White Wolf Mountain's pack wolves and sentries are vislevel 25 with
/// `check_nottoostrong=outside_wilderness`: they never hunt a player above
/// combat 50, but they do walk the back ridge. A known combat-51 account
/// therefore has a safe land route around the back; an unknown or low combat
/// state is honestly refused at the mountain.
#[test]
#[ignore = "requires NAV_TEST_PACK pointing to the shipped revision-289 nav pack"]
fn public_profile_routes_lumbridge_to_ardougne() {
    use api::snapshot::WorldTile;
    use host_play::walk_map::{ActionError, WalkActionOutcome, WalkRequest};
    use nav_planner::{
        router::{find_blocking_zones, FindOptions},
        world::NavWorld,
        world_state::WorldState,
    };
    struct Log(parking_lot::Mutex<Vec<String>>);
    impl api::hostlog::Sink for Log {
        fn record(&self, record: &api::hostlog::Record<'_>) {
            if record.slot == Some("members-route-regression") {
                assert_eq!(record.level, api::hostlog::Level::Info);
                self.0.lock().push(record.message.to_owned());
            }
        }
    }
    static LOG: Log = Log(parking_lot::Mutex::new(Vec::new()));
    assert!(api::hostlog::install_sink(&LOG));
    let fixture = Fixture::new();
    let (mut options, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
    let world = NavWorld::load_pack(Path::new(&std::env::var("NAV_TEST_PACK").unwrap())).unwrap();
    let zones = world.graph.zones.as_ref().expect("baked zones");
    let tile = |x, z| WorldTile { x, z, level: 0 };
    let lumbridge_ardougne = (tile(3220, 3211), tile(2661, 3301));
    let taverley_catherby = (tile(2895, 3450), tile(2809, 3440));
    let cases = [
        (None, Some(51), lumbridge_ardougne, true),
        (None, Some(51), taverley_catherby, true),
        (None, None, lumbridge_ardougne, false),
        (None, Some(50), lumbridge_ardougne, false),
        (None, Some(3), taverley_catherby, false),
        (Some(false), Some(51), lumbridge_ardougne, false),
    ];
    for (override_members, combat, (from, to), reachable) in cases {
        options.world_members = override_members;
        let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
        let mut state = WorldState::empty().with_map_members(selected.map_members());
        state.combat_level = combat;
        let result = WalkRequest {
            slot: Some("members-route-regression"),
            origin: Some(nav_planner::tile::Tile {
                x: from.x,
                z: from.z,
                level: from.level,
            }),
            destination: Some(nav_planner::tile::Tile {
                x: to.x,
                z: to.z,
                level: to.level,
            }),
            options: FindOptions::default(),
            map_members: selected.map_members(),
            members: selected.world_members(),
        }
        .run(|| {
            let admission = host_play::admission::Admission::manual(
                FindOptions::default(),
                Default::default(),
                0,
                host_play::WalkGlobals::default(),
            );
            host_play::arm_walk_on(
                &world,
                nav_planner::tile::Tile {
                    x: from.x,
                    z: from.z,
                    level: from.level,
                },
                nav_planner::tile::Tile {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                },
                FindOptions::default(),
                &state,
                &nav_planner::bank_fetch::BankRows::default(),
                admission,
                &host_play::WalkArms::default(),
                None,
            )
            .map(WalkActionOutcome::Walking)
            .map_err(|_| ActionError::NoPath)
        });
        let label = format!("{override_members:?} combat={combat:?} {from:?}->{to:?}");
        assert_eq!(result.is_ok(), reachable, "{label}: {result:?}");
        if let Ok(WalkActionOutcome::Walking(route)) = &result {
            let walked: Vec<_> = route
                .route
                .legs
                .iter()
                .flat_map(|leg| match leg {
                    nav_planner::router::Leg::Walk { tiles } => tiles.clone(),
                    nav_planner::router::Leg::Transport { edge } => vec![edge.at, edge.to],
                })
                .collect();
            assert!(
                walked
                    .iter()
                    .any(|t| t.x <= 2800 && t.z >= 3480 && t.z <= 3530)
                    && walked
                        .iter()
                        .any(|t| (2817..=2860).contains(&t.x) && t.z >= 3520),
                "{label}: the land route goes around the back of White Wolf Mountain"
            );
        } else if override_members.is_none() {
            let names: Vec<_> = find_blocking_zones(
                &world.collision,
                &world.graph,
                from,
                to,
                FindOptions::default(),
                &state,
                &[],
            )
            .expect("a zone-attributed refusal")
            .into_iter()
            .map(|key| zones.name(key))
            .collect();
            assert!(
                names.iter().any(|name| name == "white-wolf-mountain"),
                "{label}: {names:?}"
            );
        }
    }
    let logs = LOG.0.lock();
    assert_eq!(
        logs.len(),
        2 + 4 * 2,
        "one request line each plus one failed terminal per refusal"
    );
    assert!(logs[0].contains("map_members=true"));
    assert!(logs[0].contains("members_source=rs2b2t "));
    let members_off = logs.len() - 2;
    assert!(logs[members_off].contains("map_members=false"));
    assert!(logs[members_off].contains("members_source=explicit "));
    assert!(logs[members_off].contains("refused=NoPath"));
    assert!(logs[members_off + 1].contains("WalkTo outcome=aborted"));
    assert!(logs[members_off + 1].contains("reason=NoPath"));
    for line in logs.iter() {
        println!("{line}");
    }
}

#[test]
fn explicit_world_members_beats_local_file_and_declares_public() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json(&engine, 274, 43594, "true");
    let env = fixture.env();
    let (options, _) = parse_profile_args([
        "--profile",
        "local-274",
        "--engine",
        engine.to_str().unwrap(),
        "--world-members",
        "false",
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(!selected.map_members());
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: false,
            source: host_play::WorldMembersSource::ExplicitOverride
        }
    ));

    let (options, _) =
        parse_profile_args(["--profile", "public-289", "--world-members", "true"]).unwrap();
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(selected.map_members());
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::ExplicitOverride
        }
    ));
}

#[test]
fn missing_world_json_is_unknown_not_default_true() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("empty-engine");
    std::fs::create_dir_all(&engine).unwrap();
    let env = fixture.env();
    let (options, _) = parse_profile_args([
        "--profile",
        "local-274",
        "--engine",
        engine.to_str().unwrap(),
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &env).unwrap();
    assert!(!selected.map_members());
    assert_eq!(
        selected.world_members(),
        &host_play::WorldMembersFact::Unknown
    );
}

fn resolve_named_local_289(
    fixture: &Fixture,
    engine: &Path,
    game_port: u16,
    http_port: u16,
    extra: &[&str],
) -> Result<host_play::ProfileSelection, String> {
    let engine = engine.to_str().unwrap().to_owned();
    let cache = fixture.0.to_str().unwrap().to_owned();
    let nav = fixture.0.join("missing.navpack");
    let nav = nav.to_str().unwrap().to_owned();
    let game_port = game_port.to_string();
    let http_port = http_port.to_string();
    let mut args = vec![
        "--profile",
        "local-289",
        "--engine",
        &engine,
        "--host",
        "127.0.0.1",
        "--port",
        &game_port,
        "--asset-host",
        "127.0.0.1",
        "--http-port",
        &http_port,
        "--cache",
        &cache,
        "--nav-pack",
        &nav,
    ];
    args.extend_from_slice(extra);
    let (options, _) = parse_profile_args(args)?;
    options.resolve_with_env(None, &fixture.env())
}

#[test]
fn default_profiles_without_endpoint_flags_remain_supported() {
    let fixture = Fixture::new();
    let local = ProfileOptions::default()
        .resolve_with_env(None, &fixture.env())
        .unwrap();
    assert!(local.supported_server());
    let (public, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
    assert!(public
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .supported_server());
}

#[test]
fn named_local_matching_loopback_ports_attach_revision_facts() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json_with_web(&engine, 289, 45594, Some(1180), "true");
    let selected = resolve_named_local_289(&fixture, &engine, 45594, 1180, &[]).unwrap();
    assert!(selected.supported_server());
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::LocalWorldJson { .. }
        }
    ));
    if !api::test_support::game_data_available() {
        return;
    }
    let profile = selected.bind().unwrap();
    assert!(profile.game_data().is_some());
    assert!(matches!(
        profile.game_data_status(),
        host_play::GameDataStatus::Attached
    ));
}

#[test]
fn named_local_forwarded_ports_keep_facts_attached() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json_with_web(&engine, 289, 45594, Some(1180), "true");
    let forwarded = resolve_named_local_289(&fixture, &engine, 44594, 2180, &[]).unwrap();
    assert!(forwarded.supported_server());
    assert!(matches!(
        forwarded.world_members(),
        host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::LocalWorldJson { .. }
        }
    ));
    if !api::test_support::game_data_available() {
        return;
    }
    let profile = forwarded.bind().unwrap();
    assert!(profile.game_data().is_some());
    assert!(matches!(
        profile.game_data_status(),
        host_play::GameDataStatus::Attached
    ));

    write_world_json(&engine, 289, 45594, "true");
    let missing_web = resolve_named_local_289(&fixture, &engine, 44594, 2180, &[]).unwrap();
    assert!(matches!(
        missing_web.world_members(),
        host_play::WorldMembersFact::Known {
            source: host_play::WorldMembersSource::LocalWorldJson { .. },
            ..
        }
    ));
    assert!(missing_web.supported_server());
    assert!(missing_web.bind().unwrap().game_data().is_some());
}

#[test]
fn named_local_nonlocal_host_stays_fail_closed() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json_with_web(&engine, 289, 45594, Some(1180), "true");
    let error = parse_profile_args([
        "--profile",
        "local-289",
        "--engine",
        engine.to_str().unwrap(),
        "--host",
        "8.8.8.8",
        "--port",
        "45594",
        "--http-port",
        "1180",
    ])
    .unwrap()
    .0
    .resolve_with_env(None, &fixture.env())
    .unwrap_err();
    assert!(error.contains("non-loopback"), "{error}");
}

#[test]
fn public_endpoint_overrides_do_not_inherit_local_world_facts() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json_with_web(&engine, 289, 443, Some(443), "false");
    let (options, _) = parse_profile_args([
        "--profile",
        "public-289",
        "--engine",
        engine.to_str().unwrap(),
        "--host",
        "w1.rs2b2t.com",
        "--port",
        "443",
        "--asset-host",
        "w1.rs2b2t.com",
        "--http-port",
        "443",
        "--cache",
        fixture.0.to_str().unwrap(),
        "--nav-pack",
        fixture.0.join("missing.navpack").to_str().unwrap(),
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(selected.supported_server());
    assert_eq!(
        selected.world_members(),
        &host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::Rs2b2tWorlds,
        }
    );
    assert_eq!(
        selected.world_members().world_truth(),
        api::selected::Truth::True
    );
}

#[test]
fn operator_listed_custom_endpoint_remains_unsupported() {
    let fixture = Fixture::new();
    fixture.write_public_roster([host_play::public_worlds::PublicWorld {
        number: 13,
        host: "custom.example".into(),
        port: 443,
        node_id: 42,
    }]);
    let mut options = fixture.options(289);
    options.rs2b2t = true;
    // Preserve the selected WSS roster world's paired asset endpoint.
    options.asset_host = None;
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(!selected.supported_server());
    let profile = selected.bind().unwrap();
    assert!(profile.game_data().is_none());
    match profile.game_data_status() {
        host_play::GameDataStatus::Unsupported { reason } => assert!(
            reason.contains("custom.example") && reason.contains("bundled rs2b2t world"),
            "the rejection names the endpoint that is not bundled: {reason}"
        ),
        other => panic!("an unlisted public endpoint is unsupported, got {other:?}"),
    }
}

#[test]
fn runtime_asset_connection_falls_back_to_next_listed_world() {
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    fixture.write_public_roster([
        host_play::public_worlds::PublicWorld {
            number: 13,
            host: "127.0.0.1".into(),
            port: dead_port,
            node_id: 42,
        },
        host_play::public_worlds::PublicWorld {
            number: 14,
            host: "localhost".into(),
            port,
            node_id: 43,
        },
    ]);
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((_socket, _)) => return true,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("unexpected listener failure: {error}"),
            }
        }
    });
    let mut options = fixture.options(289);
    options.rs2b2t = true;
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    let error = selected.bind_runtime().unwrap_err();
    assert!(
        server.join().unwrap(),
        "the second asset world was not contacted: {error}"
    );
}

#[test]
fn missing_world_json_with_port_overrides_does_not_qualify() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("empty-engine");
    std::fs::create_dir_all(&engine).unwrap();
    let selected = resolve_named_local_289(&fixture, &engine, 45594, 1180, &[]).unwrap();
    assert!(!selected.supported_server());
    assert_eq!(
        selected.world_members(),
        &host_play::WorldMembersFact::Unknown
    );
    let profile = selected.bind().unwrap();
    assert!(profile.game_data().is_none());
    match profile.game_data_status() {
        host_play::GameDataStatus::Unsupported { reason } => assert!(
            reason.contains("world.json") && reason.contains("none usable"),
            "the rejection names the missing guarded world.json: {reason}"
        ),
        other => panic!("explicit endpoints without world.json are unsupported, got {other:?}"),
    }
}

#[test]
fn explicit_world_members_does_not_manufacture_override_trust() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    write_world_json_with_web(&engine, 289, 45594, Some(1180), "true");
    let selected =
        resolve_named_local_289(&fixture, &engine, 45594, 1180, &["--world-members", "true"])
            .unwrap();
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            members: true,
            source: host_play::WorldMembersSource::ExplicitOverride
        }
    ));
    assert!(!selected.supported_server());
    let profile = selected.bind().unwrap();
    assert!(profile.game_data().is_none());
    match profile.game_data_status() {
        host_play::GameDataStatus::Unsupported { reason } => assert!(
            reason.contains("ExplicitOverride"),
            "the rejection names the explicit membership override: {reason}"
        ),
        other => panic!("an explicit membership override is unsupported, got {other:?}"),
    }

    let (options, _) = parse_profile_args([
        "--profile",
        "local-289",
        "--engine",
        engine.to_str().unwrap(),
        "--world-members",
        "false",
    ])
    .unwrap();
    let no_endpoint = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(no_endpoint.supported_server());
}

#[test]
fn malformed_web_port_does_not_affect_fact_qualification() {
    let fixture = Fixture::new();
    let engine = fixture.0.join("engine");
    let dir = engine.join("data/config");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("world.json"),
        r#"{"engine":{"revision":289},"node":{"port":45594,"members":true},"web":{"port":"1180"}}"#,
    )
    .unwrap();
    let selected = resolve_named_local_289(&fixture, &engine, 45594, 1180, &[]).unwrap();
    assert!(matches!(
        selected.world_members(),
        host_play::WorldMembersFact::Known {
            source: host_play::WorldMembersSource::LocalWorldJson { .. },
            ..
        }
    ));
    assert!(selected.supported_server());
}

/// Bind-only: matching isolated 45594/1180 attaches the selected Lobster heal.
#[test]
fn named_local_matching_isolated_ports_attach_lobster_heal12() {
    let Some(engine) = api::test_support::local_asset_path(
        "ISOLATED_289_ENGINE_DIR",
        "",
        "isolated local-289 engine",
    ) else {
        return;
    };
    let Some(content) = api::test_support::local_asset_path(
        "ISOLATED_289_CONTENT_DIR",
        "",
        "isolated local-289 content",
    ) else {
        return;
    };
    let fixture = Fixture::new();
    let cache = engine.join("data/pack/client");
    let (options, _) = parse_profile_args([
        "--profile",
        "local-289",
        "--engine",
        engine.to_str().unwrap(),
        "--host",
        "127.0.0.1",
        "--port",
        "45594",
        "--asset-host",
        "127.0.0.1",
        "--http-port",
        "1180",
        "--cache",
        cache.to_str().unwrap(),
        "--content",
        content.to_str().unwrap(),
        "--nav-pack",
        fixture.0.join("missing.navpack").to_str().unwrap(),
        "--unpack",
        fixture.0.join("unpack").to_str().unwrap(),
    ])
    .unwrap();
    let selected = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(selected.supported_server());
    let profile = selected.bind().unwrap();
    let data = profile
        .game_data()
        .expect("matching isolated local-289 attaches revision facts");
    assert_eq!(data.fixed_food_heal("Lobster"), Some(12));
}

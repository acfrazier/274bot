use super::*;
use host_play::profile::ProfileEnvironment;
use host_play::{Play, SlotArm};
use nav::grid::StepGrid;
use nav::router::FindOptions;
use nav::world::NavWorld;
use script::IsolatedEnv;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait_script_state(play: &host_play::Play, name: &str, want: script::RunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while play.script_state(name) != want && Instant::now() < deadline {
        play.pump_script_lifecycle(name);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(play.script_state(name), want);
}

fn dummy_options() -> PlayOptions {
    PlayOptions {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    }
}

#[test]
fn mainland_seed_is_cold_login_only_and_opt_in() {
    let sent = Mutex::new(HashSet::new());

    assert!(
        !take_mainland_seed(&sent, "interactive", false, None),
        "ordinary interactive boot must not opt into mainland seeding"
    );
    assert!(
        take_mainland_seed(&sent, "live", true, Some(false)),
        "the enabled cold login seeds once"
    );
    assert!(
        !take_mainland_seed(&sent, "live", true, Some(false)),
        "later ready frames in the same world do not re-seed"
    );
    assert!(
        !take_mainland_seed(&sent, "relog", true, Some(true)),
        "an intentional reconnect must retain the scenario's seeded tile"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum RecordedOut {
    Enc(i32),
    P1(i32),
    P2(i32),
    P4(i32),
    Jstr(String),
}

#[derive(Default)]
struct RecordingOut(Vec<RecordedOut>);

impl api::prot::Out for RecordingOut {
    fn p1_enc(&mut self, opcode: i32) {
        self.0.push(RecordedOut::Enc(opcode));
    }

    fn p1(&mut self, value: i32) {
        self.0.push(RecordedOut::P1(value));
    }

    fn p2(&mut self, value: i32) {
        self.0.push(RecordedOut::P2(value));
    }

    fn p4(&mut self, value: i32) {
        self.0.push(RecordedOut::P4(value));
    }

    fn pjstr(&mut self, value: &str) {
        self.0.push(RecordedOut::Jstr(value.to_string()));
    }
}

#[derive(Default)]
struct RecordingDriver {
    out: RecordingOut,
}

impl api::interact::Driver for RecordingDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}

    fn do_action(&mut self, _slot: i32) -> bool {
        false
    }

    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        false
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.out
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

#[test]
fn mainland_production_gate_queues_once_without_rearming_host_or_reconnect() {
    use client::io::ClientProt;

    let mut options = dummy_options();
    options.mainland = true;
    let (enabled, host_options) = mainland_seed_options(&options);
    assert!(enabled, "TUI must retain the opt-in");
    assert!(
        !host_options.mainland,
        "host-play must not own or re-arm mainland seeding"
    );

    let sent = Mutex::new(HashSet::new());
    let mut driver = RecordingDriver::default();
    assert!(!seed_mainland_on_ready(
        &mut driver,
        &sent,
        "not-ready",
        enabled,
        false,
        Some(false),
    ));
    assert!(!seed_mainland_on_ready(
        &mut driver,
        &sent,
        "disabled",
        false,
        true,
        Some(false),
    ));
    assert!(!seed_mainland_on_ready(
        &mut driver,
        &sent,
        "reconnect",
        enabled,
        true,
        Some(true),
    ));
    assert!(driver.out.0.is_empty());

    assert!(seed_mainland_on_ready(
        &mut driver,
        &sent,
        "cold",
        enabled,
        true,
        Some(false),
    ));
    assert!(!seed_mainland_on_ready(
        &mut driver,
        &sent,
        "cold",
        enabled,
        true,
        Some(false),
    ));

    let tele = format!("tele {}", api::interact::OFF_ISLAND_TELE);
    assert_eq!(
        driver.out.0,
        vec![
            RecordedOut::Enc(ClientProt::CLIENT_CHEAT.id),
            RecordedOut::P1((tele.len() + 1) as i32),
            RecordedOut::Jstr(tele),
            RecordedOut::Enc(ClientProt::CLIENT_CHEAT.id),
            RecordedOut::P1(("setvar tutorial 1000".len() + 1) as i32),
            RecordedOut::Jstr("setvar tutorial 1000".into()),
        ]
    );
}

fn response_15_reconnect(c: &mut client::client::Client) {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    c.config.port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = [0; 2];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 14);
        stream.write_all(&[0; 17]).unwrap();
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 18);
        let mut login = vec![0; header[1] as usize];
        stream.read_exact(&mut login).unwrap();
        stream.write_all(&[15]).unwrap();
    });
    c.login("snapshot", "test", true).unwrap();
    server.join().unwrap();
}

type FrontendFixture = (
    Arc<Mutex<HashMap<String, client::client::ClientGens>>>,
    Arc<Mutex<HashMap<String, api::snapshot::GameSnapshot>>>,
    SlotTravellers,
    Arc<Mutex<NavStepLatch>>,
);

fn frontend_fixture() -> FrontendFixture {
    (
        Arc::new(Mutex::new(HashMap::new())),
        Arc::new(Mutex::new(HashMap::new())),
        Arc::new(Mutex::new(HashMap::new())),
        Arc::new(Mutex::new(HashMap::new())),
    )
}

#[test]
fn frontend_logout_clears_facts_armed_work_and_tick_latch() {
    let (gens, snapshots, travellers, latch) = frontend_fixture();
    let mut c = bank_fetch_fixtures::bank_client();
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));
    latch
        .lock()
        .unwrap()
        .insert("alice".into(), (c.gens.player, (3205, 3205, 0)));

    c.logout();
    assert!(publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    let snapshots = snapshots.lock().unwrap();
    assert!(!snapshots["alice"].ingame());
    assert!(snapshots["alice"].local_player().is_none());
    drop(snapshots);
    assert!(!travellers.lock().unwrap().contains_key("alice"));
    assert!(!latch.lock().unwrap().contains_key("alice"));
}

#[test]
fn frontend_response_15_replacement_waits_for_post_grant_player_packet() {
    let (gens, snapshots, travellers, latch) = frontend_fixture();
    let mut c = bank_fetch_fixtures::bank_client();
    publish_frontend_slot("alice", &c, &gens, &snapshots, &travellers, &latch);
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));

    response_15_reconnect(&mut c);
    assert!(publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    assert!(snapshots.lock().unwrap()["alice"].local_player().is_none());
    assert!(!travellers.lock().unwrap().contains_key("alice"));

    let mut player = client::io::Packet::new(vec![0xe0, 0x50, 0xc0, 0]);
    c.psize = 4;
    c.handle_packet(client::io::ServerProt::PLAYER_INFO, &mut player);
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    assert!(snapshots.lock().unwrap()["alice"].local_player().is_some());
}

#[test]
fn same_name_lifetime_resets_but_scene_change_and_guardian_hold_preserve_work() {
    let (gens, snapshots, travellers, latch) = frontend_fixture();
    let mut c = bank_fetch_fixtures::bank_client();
    publish_frontend_slot("alice", &c, &gens, &snapshots, &travellers, &latch);
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));
    latch
        .lock()
        .unwrap()
        .insert("alice".into(), (c.gens.player, (3205, 3205, 0)));

    c.scene_state = 1;
    c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    assert!(!WalkArm::may_follow(true));
    assert!(travellers.lock().unwrap().contains_key("alice"));
    assert!(latch.lock().unwrap().contains_key("alice"));

    assert!(reset_frontend_slot_lifetime(
        "alice",
        &gens,
        &snapshots,
        &travellers,
        &latch
    ));
    assert!(!gens.lock().unwrap().contains_key("alice"));
    assert!(!snapshots.lock().unwrap().contains_key("alice"));
    assert!(!travellers.lock().unwrap().contains_key("alice"));
    assert!(!latch.lock().unwrap().contains_key("alice"));

    let fresh = bank_fetch_fixtures::bank_client();
    publish_frontend_slot("alice", &fresh, &gens, &snapshots, &travellers, &latch);
    assert!(!travellers.lock().unwrap().contains_key("alice"));
}

fn checked_fixture(revision: u16) -> (PathBuf, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let root = std::env::temp_dir().join(format!(
        "274bot-tui-profile-{revision}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    for jag in [
        "title",
        "config",
        "interface",
        "media",
        "versionlist",
        "textures",
        "wordenc",
        "sounds",
    ] {
        std::fs::copy(fixture.join(jag), cache.join(jag)).unwrap();
    }
    let manifest = fixture.join(format!("manifest-{revision}.json"));
    (root, cache, manifest)
}

/// The map pane reads `TuiApp::world`. The session holds the pack on
/// `nav_world` after `Play` loads it; pump must copy that Arc so a
/// running script's loc list is not the only live world view.
#[test]
fn pump_copies_nav_world_onto_the_app() {
    let mut session = TuiSession::new(dummy_options());
    *session.nav_world.lock().unwrap() =
        Some(Arc::new(NavWorld::from_grid(&StepGrid::fixture_open_3x3())));
    let mut app = TuiApp::new("274bot headless");
    assert!(app.world.is_none(), "fresh app has no pack");
    session.pump(&mut app);
    assert!(
        app.world.is_some(),
        "pump copies the session nav world onto the map"
    );
}

#[test]
fn live_pass_is_stdout_and_exit_0() {
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        false,
        false,
    );
    assert_eq!(code, Some(0));
    assert!(announced);
    assert_eq!(
        lines,
        vec![ProofLine::Stdout(
            "PASS: live alcher {\"outcome\":\"PASS\"}".into()
        )]
    );
}

#[test]
fn live_fail_is_stderr_and_exit_1() {
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Failed("deadline".into())),
        "{\"outcome\":\"FAIL\"}",
        true,
        false,
    );
    assert_eq!(
        code,
        Some(1),
        "FAIL must still exit 1, including during soak"
    );
    assert!(!announced, "FAIL does not latch a PASS announcement");
    assert_eq!(
        lines,
        vec![
            ProofLine::Stderr("FAIL: live alcher {\"outcome\":\"FAIL\"}".into()),
            ProofLine::Stderr("FAIL: deadline".into()),
        ]
    );
}

#[test]
fn live_pass_is_held_once_across_soak_then_exits_0() {
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        true,
        false,
    );
    assert_eq!(code, None, "soak keeps pumping after the PASS line exists");
    assert_eq!(
        lines,
        vec![ProofLine::Stdout(
            "PASS: live alcher {\"outcome\":\"PASS\"}".into()
        )]
    );
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        true,
        announced,
    );
    assert_eq!(code, None);
    assert!(
        lines.is_empty(),
        "headed soak must not reprint PASS into the alt screen"
    );
    let (code, lines, _) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        false,
        announced,
    );
    assert_eq!(code, Some(0));
    assert!(
        lines.is_empty(),
        "the held PASS line is flushed after restore, not here"
    );
}

#[test]
fn live_running_emits_no_proof() {
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(scenario::RunnerStatus::Running { step: 1, total: 2 }),
        "",
        false,
        false,
    );
    assert_eq!(code, None);
    assert!(lines.is_empty());
    assert!(!announced);
}

#[test]
fn parse_args_from_prod_is_not_unknown() {
    let args = parse_args_from(["--prod"]).expect("prod is a known flag");
    assert!(args.profile.prod);
    assert!(args.live.is_none());
    let home = std::env::temp_dir().join(format!("274bot-tui-public-{}", std::process::id()));
    let selection = args
        .profile
        .resolve_with_env(
            Some(274),
            &ProfileEnvironment {
                home: Some(home.clone()),
                ..ProfileEnvironment::default()
            },
        )
        .unwrap();
    assert_eq!(selection.revision(), client::io::ClientRevision::R289);
    assert_eq!(selection.target(), client::BotTarget::Prod);
    assert_eq!(selection.public_worlds().unwrap().worlds.len(), 2);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn world_flag_selects_auto_default_and_rejects_invalid_number() {
    let args = parse_args_from(["--profile", "public-289", "--world", "2"]).unwrap();
    assert_eq!(args.world, Some(2));
    assert!(parse_args_from(["--world", "0"]).is_err());
    assert!(parse_args_from(["--world", "not-a-number"]).is_err());
}

#[test]
fn parse_args_from_accepts_revision_profile_and_ordered_overrides() {
    let args = parse_args_from([
        "--live",
        "script_bone_burier",
        "--profile",
        "local-289",
        "--revision",
        "289",
        "--port",
        "44595",
    ])
    .expect("shared profile flags parse before TUI flags");
    assert_eq!(args.live.as_deref(), Some("script_bone_burier"));
    assert_eq!(args.profile.profile.as_deref(), Some("local-289"));
    assert_eq!(args.profile.revision.as_deref(), Some("289"));
    assert_eq!(args.profile.port, Some(44595));
}

#[test]
fn frontend_parser_prepares_real_clients_for_both_fixture_manifests() {
    for revision in [274_u16, 289] {
        let (root, cache, manifest) = checked_fixture(revision);
        let args = parse_args_from([
            "--live".to_string(),
            "script_bone_burier".to_string(),
            "--profile".to_string(),
            format!("local-{revision}"),
            "--cache".to_string(),
            cache.display().to_string(),
            "--cache-manifest".to_string(),
            manifest.display().to_string(),
        ])
        .expect("frontend and shared flags parse in either order");
        let env = ProfileEnvironment {
            home: Some(root.clone()),
            working_dir: Some(root.clone()),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..ProfileEnvironment::default()
        };
        let selection = args.profile.resolve_with_env(None, &env).unwrap();
        assert_eq!(selection.game_host(), "127.0.0.1");
        let profile = selection.bind().unwrap();
        let template = SharedClientTemplate::load(profile).unwrap();
        let client = template.prepare_client(274_000_001, true).unwrap();
        drop(client);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn validate_startup_host_refuses_non_loopback_with_local_rsa() {
    assert!(
        host_play::validate_play_host("attacker.example", client::BotTarget::Local).is_err(),
        "non-loopback host must be refused with local RSA"
    );
    assert!(host_play::validate_play_host("127.0.0.1", client::BotTarget::Local).is_ok());
    // `tui-play` startup must delegate to the shared helper (not duplicate checks).
    assert_eq!(
        validate_startup_host("attacker.example").is_ok(),
        host_play::validate_play_host("attacker.example", client::bot_target()).is_ok(),
    );
    assert_eq!(
        validate_startup_host("127.0.0.1").is_ok(),
        host_play::validate_play_host("127.0.0.1", client::bot_target()).is_ok(),
    );
}

#[test]
#[cfg(unix)]
fn create_profile_upsert_error_returns_err() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!(
        "274bot-tui-create-profile-err-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault.vault");
    let mut session = TuiSession::new(dummy_options());
    session
        .start_play(Vault::create(&path, "bot").unwrap())
        .unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let err = session.create_profile("alice").unwrap_err();
    assert!(
        err.starts_with("profile:"),
        "upsert failure must return Err, got {err:?}"
    );
}

fn fake_rs2b0t_tree(dir: &Path) -> PathBuf {
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
ScriptRegistry.register({
  name: 'BoneBurier',
  description: 'Buries bones',
  category: 'Prayer',
  tags: ['bones'],
  create: () => new BoneBurier(),
});
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    root
}

#[test]
fn live_scenario_looks_up_v2_file_ids_and_keeps_v1() {
    let js = live_scenario("script_bone_burier_v2_js").expect("js");
    let ts = live_scenario("script_bone_burier_v2_ts").expect("ts");
    let v1 = live_scenario("script_bone_burier").expect("v1");
    assert_eq!(js.name, "bone_burier_v2_js");
    assert_eq!(js.settings.start_file, Some("bone_burier_v2.js"));
    assert_eq!(ts.name, "bone_burier_v2_ts");
    assert_eq!(ts.settings.start_file, Some("bone_burier_v2.ts"));
    assert_eq!(v1.settings.start_script, Some("BoneBurier"));
    assert_eq!(v1.settings.start_file, None);
    assert!(live_scenario("script_nope").is_err());
}

#[test]
fn live_prepare_bone_burier_selects_the_rs2b0t_card_without_starting() {
    let iso = IsolatedEnv::enter("tui-bone-live");
    let root = fake_rs2b0t_tree(&iso.dir);
    iso.set_rs2b0t(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(scenario::get("bone_burier").expect("registered"))
        .expect("prepare");
    let name = session.names.first().expect("minted name").clone();
    assert_ne!(name, "test", "live must not log in `test`");
    assert_eq!(
        session.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "BoneBurier".into()
        )),
        "prepare sets script_sel to the catalog card"
    );
    assert!(session.core.play().is_some(), "play started");
    assert!(
        session.core.fleet().contains(&name),
        "the minted driver is a loaded member"
    );
    assert_eq!(
        session
            .pending_script
            .lock()
            .unwrap()
            .as_ref()
            .map(|pending| pending.slot.as_str()),
        Some(name.as_str()),
        "preparation stages the selected card for StartScript"
    );
}

#[test]
fn live_prepare_bone_burier_v2_selects_each_example_by_identity() {
    let ts = script::live_example_path("bone_burier_v2.ts").expect("ts example");
    let js = script::live_example_path("bone_burier_v2.js").expect("js example");
    for (name, path) in [
        ("bone_burier_v2_ts", ts.as_path()),
        ("bone_burier_v2_js", js.as_path()),
    ] {
        let iso = IsolatedEnv::enter(&format!("tui-bone-v2-{name}"));
        let mut session = TuiSession::new(dummy_options());
        session.core.set_spawn_workers(false);
        session.js = script::JsLibrary::with_cache(
            iso.dir.join("js-scripts.json"),
            iso.dir.join("js-cache"),
        );
        session.js.load(&ts).expect("preload ts");
        session.js.load(&js).expect("preload js");
        session.script_settings.set_str(
            script::ScriptSource::File,
            "bone_burier_v2",
            "boneName",
            "stem",
        );
        let identity = script::file_identity(path);
        session.script_settings.set_str(
            script::ScriptSource::File,
            &identity,
            "boneName",
            "identity",
        );
        session
            .live_prepare_script(scenario::get(name).expect("registered"))
            .expect("prepare");
        assert_eq!(
            session.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::File,
                identity.clone()
            )),
            "{name} must select the canonical-path identity"
        );
        assert_ne!(
            session.script_sel,
            Some(script::ScriptSel::Loaded(
                script::ScriptSource::File,
                "bone_burier_v2".into()
            ))
        );
        let bag = session
            .pending_script
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|pending| pending.bag.clone())
            .expect("settings bag");
        assert_eq!(bag.get("boneName"), Some(&serde_json::json!("identity")));
        assert!(
            session
                .pending_script
                .lock()
                .unwrap()
                .as_ref()
                .expect("file start stashed")
                .loadouts
                .is_empty(),
            "native-v2 File starts keep ordinary operator loadout behavior"
        );
        assert_eq!(
            session.live_wait_script_stop,
            Some("confirmed loaded current-generation bank exhaustion")
        );
    }
}

#[test]
fn live_prepare_thiever_posts_guard_target_when_schema_empty() {
    let iso = IsolatedEnv::enter("tui-thiever-bag");
    let root = iso.dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("ThievingBot")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import ThievingBot from './ThievingBot/ThievingBot.js';
ScriptRegistry.register({ name: 'Thiever', create: () => new ThievingBot() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("ThievingBot/ThievingBot.ts"),
        "export default class ThievingBot extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    iso.set_rs2b0t(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(scenario::get("thiever").expect("registered"))
        .expect("prepare");
    let pending = session.pending_script.lock().unwrap();
    let pending = pending.as_ref().expect("catalog start stashed");
    let bag = pending
        .bag
        .clone()
        .expect("inject bag is posted even when the card schema is empty");
    assert_eq!(
        bag.get("target"),
        Some(&serde_json::json!("Guard")),
        "thiever inject must beat the Man fallback"
    );
    assert_eq!(
        pending.loadouts,
        vec![script::Loadout::new("Memory food").with_carry("Lobster", 1)],
        "production live preparation stages scenario loadouts for catalog Start"
    );
}

#[test]
fn first_browse_without_rs2b0t_opens_catalog_prompt() {
    let iso = IsolatedEnv::enter("tui-browse");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.script_browse_open = true;
    session.on_script_browse_open(&mut app);
    assert!(
        session.rs2b0t_catalog_open,
        "first browse opens folder picker"
    );
    assert!(
        !iso.home.join(".274bot/rs2b0t-path").exists(),
        "first browse must not write rs2b0t-path"
    );
}

#[test]
fn defer_rs2b0t_catalog_leaves_no_path_and_zero_catalog_cards() {
    let iso = IsolatedEnv::enter("tui-defer");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.script_browse_open = true;
    session.on_script_browse_open(&mut app);
    session.defer_rs2b0t_catalog(&mut app);
    assert!(
        script::rs2b0t_import_deferred_at(&iso.home.join(".274bot/rs2b0t-import")),
        "defer flag written"
    );
    assert!(
        !iso.home.join(".274bot/rs2b0t-path").exists(),
        "defer must not write rs2b0t-path"
    );
    assert!(
        session
            .js
            .cards()
            .iter()
            .all(|c| c.source != script::ScriptSource::Catalog),
        "zero Catalog cards after defer"
    );
}

#[test]
fn import_rs2b0t_catalog_persists_path_and_registers_cards() {
    let iso = IsolatedEnv::enter("tui-import");
    let root = fake_rs2b0t_tree(&iso.dir);
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    let n = session
        .import_rs2b0t_catalog(&mut app, &root)
        .expect("import");
    assert_eq!(n, 1);
    assert!(iso.home.join(".274bot/rs2b0t-path").is_file());
    let card = session
        .js
        .get(script::ScriptSource::Catalog, "BoneBurier")
        .expect("catalog card");
    assert_eq!(card.description, "Buries bones");
}

#[test]
fn create_profile_prod_password_is_not_username() {
    let pass = host_play::profile_password_for("alice", client::BotTarget::Prod);
    assert_ne!(pass, "alice");
    assert_eq!(
        host_play::profile_password_for("alice", client::BotTarget::Local),
        "alice"
    );
}

#[test]
fn pump_leaves_app_world_none_when_no_pack_loaded() {
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    session.pump(&mut app);
    assert!(
        app.world.is_none(),
        "no session pack stays the empty-state title"
    );
}

/// Pump the TUI's Start settle (the public observe path) until every
/// pending Start has settled. Start returns before V8 setup.
fn settle_starts(session: &mut TuiSession, app: &mut TuiApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !session.pending_starts.is_empty() && Instant::now() < deadline {
        session.core.poll();
        session.settle_script_starts(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        session.pending_starts.is_empty(),
        "script Start did not settle"
    );
}

#[test]
fn initial_runtime_failure_survives_success_and_refusals_in_tui_output() {
    let iso = IsolatedEnv::enter("tui-initial-load");
    let mut session = TuiSession::new(dummy_options());
    let mut play = run_with_io(&dummy_options(), vec![], |_| (None, None), |_, _, _| {});
    play.attach_arm("alice", SlotArm::new(7, false));
    play.attach_arm("bob", SlotArm::new(8, false));
    session.inject_play(play);
    let mut app = TuiApp::new("initial load proof");
    app.names = vec!["alice".into(), "bob".into(), "missing".into()];
    app.focused = Some(0);
    let path = iso.dir.join("retry.ts");
    let helper = iso.dir.join("gate.ts");
    std::fs::write(&helper, "export const fail = true;").unwrap();
    let src = "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('tui-initial-load');\nexport function tick(api) {}";
    std::fs::write(&path, src).unwrap();
    let card = session.js.load(&path).unwrap();
    let sel = script::ScriptSel::Loaded(card.source, path.to_string_lossy().into_owned());
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    // The failure reaches the operator once setup settles, as the
    // synchronous Start error used to.
    let shown = app.error.clone().unwrap_or_default();
    assert!(
        shown.starts_with("script: ") && shown.contains("tui-initial-load"),
        "{shown}"
    );
    let failure = session
        .js
        .load_failure(&card.identity_key())
        .unwrap()
        .clone();
    assert_eq!(failure.identity_key, card.identity_key());
    assert_eq!(failure.path, path);
    assert_eq!(failure.stage, script::LoadStage::RuntimeLoad);
    assert_eq!(failure.api_family, Some(script::ApiFamily::V2));
    assert_eq!(
        failure.fingerprint,
        script::raw_content_fingerprint(&path, src)
    );
    assert_eq!(
        session
            .core
            .play()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(0)
    );

    let good_path = iso.dir.join("good.ts");
    std::fs::write(
        &good_path,
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    let good = session.js.load(&good_path).unwrap();
    assert_eq!(good.api_family, script::ApiFamily::V1);
    app.focused = Some(1);
    session.script_start(
        &mut app,
        &script::ScriptSel::Loaded(good.source, good_path.to_string_lossy().into_owned()),
    );
    settle_starts(&mut session, &mut app);
    assert_eq!(
        session.core.play().unwrap().script_state("bob"),
        script::RunState::Running
    );
    let output = app.error.as_deref().unwrap();
    assert!(output.contains("tui-initial-load") && output.contains("runtime-load"));
    assert!(output.contains(&path.display().to_string()));
    session.script_start(&mut app, &sel); // active slot refuses before evaluating
    assert!(app
        .error
        .as_deref()
        .unwrap()
        .contains("script already active"));
    assert_eq!(
        session.js.load_failure(&card.identity_key()),
        Some(&failure)
    );
    app.focused = Some(2);
    session.script_start(&mut app, &sel);
    assert_eq!(app.error.as_deref(), Some("script: no slot: missing"));
    assert_eq!(
        session.js.load_failure(&card.identity_key()),
        Some(&failure)
    );

    std::fs::write(&helper, "export const fail = false;").unwrap();
    app.focused = Some(0);
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    assert!(session.js.load_failure(&card.identity_key()).is_none());
    assert_eq!(app.error, None);
    assert_eq!(
        session
            .core
            .play()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(1)
    );
    session.core.play().unwrap().script_stop("alice");
    session.core.play().unwrap().script_stop("bob");
}

/// Task 13 fix: the paint-as-chat toggle must not stick across a
/// Stop → new Start. A slot whose script has no paint (stopped, or
/// not painted yet) resets the toggle, so the fresh paint is visible
/// by default instead of hidden behind the game-chat toggle.
/// TR-TUI-001: dispatching ScriptPause toggles pause/resume like the
/// panel's `script_toggle_pause` (Resume when Paused, Pause when Running).
#[test]
fn script_pause_toggle_resumes_when_paused_and_pauses_when_running() {
    let mut play = run_with_io(&dummy_options(), vec![], |_| (None, None), |_, _, _| {});
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_load("alice", src, script::LoadShape::NativeTick, None, vec![])
        .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);

    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);

    dispatch(&mut session, &mut app, AppAction::ScriptPause);
    assert_eq!(
        session.core.play().unwrap().script_state("alice"),
        script::RunState::Paused,
        "Pause while Running"
    );

    dispatch(&mut session, &mut app, AppAction::ScriptPause);
    assert_eq!(
        session.core.play().unwrap().script_state("alice"),
        script::RunState::Running,
        "Resume while Paused"
    );
}

#[test]
fn paint_button_action_does_not_queue_a_wire_cmd() {
    let mut play = run_with_io(&dummy_options(), vec![], |_| (None, None), |_, _, _| {});
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    play.script_start_load("alice", src, script::LoadShape::NativeTick, None, vec![])
        .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.chat_data.script_paint = Some(std::sync::Arc::new(script::shim::ScriptPaint {
        title: Some("NatureCrafter".into()),
        accent: None,
        lines: vec!["status".into()],
        buttons: vec![script::shim::ScriptPaintButton {
            id: "gobank".into(),
            label: "Go bank".into(),
        }],
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }));
    dispatch(
        &mut session,
        &mut app,
        AppAction::Chat(ChatAction::PaintButton(0)),
    );
    assert_eq!(
        session.core.play().unwrap().script_state("alice"),
        script::RunState::Running,
        "paint click must not pause or stop"
    );
}

#[test]
fn pump_resets_the_paint_toggle_when_the_paint_is_gone() {
    let mut session = TuiSession::new(dummy_options());
    session.core.fleet_mut().add("test");
    session.core.select("test");
    let mut app = TuiApp::new("274bot headless");
    // The operator toggled to game chat while the old script painted.
    app.chat_data.show_game_chat = true;
    session.pump(&mut app);
    assert!(
        !app.chat_data.show_game_chat,
        "a stopped/not-yet-painted slot must fall back to showing paint by default"
    );
}

mod bank_fetch_fixtures {
    use std::sync::Arc;

    use api::snapshot::GameSnapshot;
    use api::snapshot::WorldTile;
    use client::client::{Client, ClientConfig, ClientPlayer};
    use client::config::if_type::ComponentType;
    use client::config::{Cache, IfType, IfTypeMut, LocType, ObjType};
    use client::io::ServerProt;
    use nav::pack::BankAccess;
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use nav::world::NavWorld;

    pub fn bank_client() -> Client {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream =
            client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
        std::mem::forget(listener);
        let mut c = host::prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.stream = Some(stream);
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.minusedlevel = 0;
        c.local_player = Some(ClientPlayer::at(5, 5));
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs.resize(3, ObjType::default());
            cache.objs[1].id = 1;
            cache.objs[1].name = "Bones".into();
            cache.objs[2].id = 2;
            cache.objs[2].name = "Lobster".into();
            cache.locs.extend(
                (0..(2214usize.saturating_sub(cache.locs.len()))).map(|_| LocType::default()),
            );
            cache.locs[2213].id = 2213;
            cache.locs[2213].name = "Bank booth".into();
            cache.locs[2213].op = vec![None, Some("Use-quickly".into()), None, None, None];
        }
        let booth_typecode = 0x4000_0000 + (2213 << 14) + 1 + (2 << 7);
        c.world
            .set_wall(0, 5, 6, 0, 0, 0, booth_typecode, 0, 0, 0, 0, 0);
        c.main_modal_id = 600;
        c.set_iface(
            600,
            IfType {
                id: 600,
                layer_id: 600,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![601]),
                ..Default::default()
            },
        );
        c.set_iface(
            601,
            IfType {
                id: 601,
                layer_id: 600,
                r#type: ComponentType::TYPE_INV,
                iop: [
                    Some("Withdraw 1".into()),
                    Some("Withdraw 5".into()),
                    Some("Withdraw 10".into()),
                    Some("Withdraw All".into()),
                    None,
                ],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]),
                link_obj_number: Some(vec![20, 0]),
                ..Default::default()
            },
        );
        c.side_modal_id = 700;
        c.set_iface(
            700,
            IfType {
                id: 700,
                layer_id: 700,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![701]),
                ..Default::default()
            },
        );
        c.set_iface(
            701,
            IfType {
                id: 701,
                layer_id: 700,
                r#type: ComponentType::TYPE_INV,
                iop: [Some("Deposit All".into()), None, None, None, None],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            701,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]),
                link_obj_number: Some(vec![3, 0]),
                ..Default::default()
            },
        );
        for prot in [
            ServerProt::IF_OPENMAIN,
            ServerProt::IF_OPENCHAT,
            ServerProt::UPDATE_INV_FULL,
            ServerProt::REBUILD_NORMAL,
            ServerProt::PLAYER_INFO,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    pub fn bank_fetch_client() -> Client {
        let mut c = bank_client();
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs[2].name = "Knife".into();
        }
        c.side_icon[3] = 500;
        c.set_iface(
            500,
            IfType {
                id: 500,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            500,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]),
                link_obj_number: Some(vec![3, 0]),
                ..Default::default()
            },
        );
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![3, 0]),
                link_obj_number: Some(vec![20, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::IF_OPENMAIN);
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        c
    }

    pub fn knife_nav_world(knife_id: i32) -> NavWorld {
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
        }
        let edge = TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 1,
                z: 2,
                level: 0,
            },
            to: WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            loc_id: 2882,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![knife_id],
            members_req: false,
            wildy_cap: None,
        };
        let mut graph = TransportGraph::default();
        graph.at.entry(edge.at).or_default().push(0);
        graph.edges.push(edge);
        let (walk, blocked) = nav::collision::pack_walk(&flags);
        NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 5,
                height: 5,
                walk,
                blocked,
                flags: None,
            },
            graph,
            vec![nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 0,
                    z: 4,
                    level: 0,
                },
                access: BankAccess::Booth { op: 2 },
            }],
        )
    }

    pub fn seed_bank_fetch_snapshot() -> GameSnapshot {
        let c = bank_fetch_client();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        snap
    }
}

/// TR-TUI-003: Walk-confirm must pass `allow_bank_fetch` from nav
/// settings so `arm_walk_on` can latch a BankBudget session.
#[test]
fn arm_walk_on_with_allow_bank_fetch_latches_bank_fetch() {
    use api::snapshot::WorldTile as SnapTile;
    use bank_fetch_fixtures::{knife_nav_world, seed_bank_fetch_snapshot};

    let mut session = TuiSession::new(dummy_options());
    *session.nav_world.lock().unwrap() = Some(Arc::new(knife_nav_world(2)));
    session
        .snapshots
        .lock()
        .unwrap()
        .insert("alice".into(), seed_bank_fetch_snapshot());
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.here = Some(SnapTile {
        x: 0,
        z: 0,
        level: 0,
    });
    app.nav.allow_bank_fetch = true;
    session.arm_walk_on(
        &mut app,
        Tile {
            x: 4,
            z: 4,
            level: 0,
        },
    );
    let latched = session
        .travellers
        .lock()
        .unwrap()
        .get("alice")
        .is_some_and(|a| a.lock().unwrap().bank_fetch.is_some());
    assert!(
        latched,
        "allow_bank_fetch must latch WalkArm.bank_fetch on the focused arm"
    );
}

#[test]
fn refused_no_origin_walk_does_not_arm_destination() {
    // Production `Play::map_walk` NoOrigin needs a bound ServerProfile nav
    // identity before `map_context` succeeds. TUI tests do not construct that
    // cheaply; this hits the same `app.here` refusal the production path uses
    // before `map_walk` (`arm_walk_without_host` when Play/profile are absent).
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.map_active = true;
    app.here = None;
    session.arm_walk_on(
        &mut app,
        Tile {
            x: 3222,
            z: 3218,
            level: 0,
        },
    );
    assert!(
        app.walk_dest.is_none(),
        "a refused Walk must not look armed: {:?}",
        app.walk_dest
    );
    let error = app.error.as_deref().unwrap_or("");
    assert!(
        error.contains("no observed player"),
        "refusal must be visible: {error:?}"
    );
}

#[test]
fn worker_panic_does_not_restore_tui_terminal() {
    let _iso = IsolatedEnv::enter("tui-worker-panic-hook");
    super::install_tui_panic_hook();
    crate::stderr_capture::capture();
    assert!(
        crate::stderr_capture::is_active(),
        "capture must start for the probe"
    );
    let joined = std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(|| panic!("caught worker panic"));
    })
    .join();
    assert!(
        joined.is_ok(),
        "worker panic must stay on the worker thread"
    );
    assert!(
        crate::stderr_capture::is_active(),
        "a caught worker panic must not restore fd 2 or the alternate screen"
    );
    crate::stderr_capture::restore();
}

/// Whole-branch fix: follow hook must pass minusedlevel, not plane 0.
#[test]
fn player_at_plane_one_follow_uses_level() {
    use api::snapshot::WorldTile;
    use bank_fetch_fixtures::bank_client;
    use host_play::{player_here_tile, step_walk_arm_bank_fetch, PendingBankFetch};
    use nav::bank_fetch::BankStep;
    use nav::router::Route;
    use std::collections::VecDeque;

    let mut c = bank_client();
    c.minusedlevel = 1;
    let here = player_here_tile(&c).expect("bank_client has local_player");
    assert_eq!(here.2, 1, "fixture player must be upstairs");
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&c);
    let final_route = Route {
        dest: WorldTile {
            x: 99,
            z: 99,
            level: 0,
        },
        legs: vec![],
        ticks: 0.0,
    };
    let pending = PendingBankFetch {
        steps: VecDeque::from([BankStep::Walk {
            x: here.0,
            z: here.1,
            level: 1,
        }]),
        dest: final_route.dest,
        opts: FindOptions::default(),
        final_route: final_route.clone(),
    };

    let mut arm = WalkArm {
        bank_fetch: Some(pending.clone()),
        ..Default::default()
    };
    step_walk_arm_bank_fetch(&mut c, &snap, &mut arm, None, Some(here), false);
    assert_eq!(
        arm.route.as_ref().map(|r| r.dest),
        Some(final_route.dest),
        "follow at (x,z,1) must complete the stand Walk on the player plane"
    );
    assert!(
        arm.bank_fetch.is_none(),
        "stand Walk must clear bank_fetch when here matches plane 1"
    );

    let mut arm_ground = WalkArm {
        bank_fetch: Some(pending),
        ..Default::default()
    };
    step_walk_arm_bank_fetch(
        &mut c,
        &snap,
        &mut arm_ground,
        None,
        Some((here.0, here.1, 0)),
        false,
    );
    assert!(
        arm_ground.route.is_none(),
        "ground-plane here must not complete an upstairs stand Walk"
    );
}

/// OPT-012: the follow tick must pump BankBudget before route follow.
#[test]
fn follow_tick_pumps_bank_budget_step() {
    use api::snapshot::WorldTile;
    use bank_fetch_fixtures::{bank_client, knife_nav_world};
    use host_play::PendingBankFetch;
    use nav::bank_fetch::BankStep;
    use nav::router::Route;
    use std::collections::VecDeque;

    let mut c = bank_client();
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&c);
    let world = Arc::new(knife_nav_world(2));
    let final_route = Route {
        dest: WorldTile {
            x: 4,
            z: 4,
            level: 0,
        },
        legs: vec![],
        ticks: 0.0,
    };
    let mut arm = WalkArm {
        bank_fetch: Some(PendingBankFetch {
            steps: VecDeque::from([
                BankStep::DepositAll,
                BankStep::Withdraw { id: 2, count: 1 },
                BankStep::Close,
            ]),
            dest: WorldTile {
                x: 4,
                z: 4,
                level: 0,
            },
            opts: FindOptions::default(),
            final_route: final_route.clone(),
        }),
        route: Some(Route {
            dest: WorldTile {
                x: 0,
                z: 4,
                level: 0,
            },
            legs: vec![],
            ticks: 0.0,
        }),
        ..Default::default()
    };
    let before = c.out.pos;
    step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arm,
        Some(world.as_ref()),
        (0, 4, 0),
        false,
    );
    assert!(
        c.out.pos > before,
        "BankBudget pump must drive deposit on the Driver (pos {before} → {})",
        c.out.pos
    );
}

fn empty_play() -> Play {
    run_with_io(&dummy_options(), vec![], |_| (None, None), |_, _, _| {})
}

fn two_arm_play() -> Play {
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(1, false));
    play.attach_arm("bob", SlotArm::new(2, false));
    play
}

fn pump_two_slots(session: &mut TuiSession) -> TuiApp {
    session.inject_play(two_arm_play());
    session.names = vec!["alice".into(), "bob".into()];
    let mut app = TuiApp::new("tui");
    app.names = session.names.clone();
    app.focused = Some(0);
    session.pump(&mut app);
    app
}

#[test]
fn tui_live_mode_does_not_show_or_persist_background_notice() {
    let iso = IsolatedEnv::enter("tui-live-ack");
    let mut session = TuiSession::new(dummy_options());
    session.persist_ui = false;
    let app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_none());
    let mut app = app;
    session.ack_background_bots(&mut app);
    assert!(!host_play::background_bots_acked());
    let _iso = iso;
}

#[test]
fn tui_notice_uses_shared_ack_file() {
    let iso = IsolatedEnv::enter("tui-shared-ack");
    let mut session = TuiSession::new(dummy_options());
    let app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_some());
    host_play::persist_background_bots_ack().unwrap();
    let mut app = TuiApp::new("tui");
    app.names = vec!["alice".into(), "bob".into()];
    app.focused = Some(0);
    session.expire_ack_cache();
    session.pump(&mut app);
    assert!(app.background_notice.is_none());
    let _iso = iso;
}

#[test]
fn tui_settings_map_bake_choice_is_the_panel_prefs_key() {
    let iso = IsolatedEnv::enter("tui-map-bake");
    let path = host_play::panel_ui_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"last_focus":"alice","map_bake":"always"}"#).unwrap();
    let mut session = TuiSession::new(dummy_options());
    assert_eq!(
        session.map_bake.choice(),
        frontend_core::MapBakeChoice::Always,
        "the panel's remembered choice is read at start"
    );
    let mut app = TuiApp::new("tui");
    app.map_bake = frontend_core::MapBakeChoice::Ask;
    app.map_bake_dirty = true;
    session.pump(&mut app);
    assert!(!app.map_bake_dirty);
    assert_eq!(session.map_bake.choice(), frontend_core::MapBakeChoice::Ask);
    assert_eq!(
        frontend_core::load_map_bake_choice(),
        frontend_core::MapBakeChoice::Ask
    );
    let prefs: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(prefs["last_focus"], "alice", "other prefs survive");
    let _iso = iso;
}

#[test]
fn tui_pump_reaps_finished_workers_logged_out_arms_still_count() {
    let iso = IsolatedEnv::enter("tui-reap-finished");
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(1, false));
    play.attach_finished_worker_for_test("bob", SlotArm::new(2, false));
    play.attach_arm("carol", SlotArm::new(3, false));
    play.statuses.lock().unwrap().extend([
        host_play::SlotStatus {
            username: "alice".into(),
            ingame: true,
            connected: true,
            ..host_play::SlotStatus::default()
        },
        host_play::SlotStatus {
            username: "bob".into(),
            worker_terminal: Some(host_play::WorkerTerminal::Failed),
            ..host_play::SlotStatus::default()
        },
        host_play::SlotStatus {
            username: "carol".into(),
            login_latched: true,
            ..host_play::SlotStatus::default()
        },
    ]);
    assert_eq!(
        play.background_bot_count(Some("alice")),
        2,
        "unreaped finished worker still has an arm"
    );
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    session.names = vec!["alice".into(), "bob".into(), "carol".into()];
    let mut app = TuiApp::new("tui");
    app.names = session.names.clone();
    app.focused = Some(0);
    session.pump(&mut app);
    let play = session.core.play().unwrap();
    assert!(
        play.arm("bob").is_none(),
        "pump must reap the finished worker"
    );
    assert!(
        play.arm("carol").is_some(),
        "logged-out active arms stay live"
    );
    assert_eq!(play.background_bot_count(Some("alice")), 1);
    assert!(app.background_notice.is_some());
    let _iso = iso;
}

#[test]
fn tui_failed_ack_persist_then_success_clears_only_that_error() {
    let iso = IsolatedEnv::enter("tui-ack-fail");
    let mut session = TuiSession::new(dummy_options());
    let mut app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_some());
    let parent = host_play::panel_ui_path().parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::write(&parent, b"not-a-dir").unwrap();
    app.error = None;
    session.ack_background_bots(&mut app);
    assert!(
        app.background_notice.is_some(),
        "persist failure must keep the notice"
    );
    assert!(
        app.error
            .as_deref()
            .is_some_and(|e| e.starts_with("background bots:")),
        "got {:?}",
        app.error
    );
    assert!(!host_play::background_bots_acked());
    std::fs::remove_file(&parent).unwrap();
    session.ack_background_bots(&mut app);
    assert!(app.background_notice.is_none());
    assert!(app.error.is_none(), "got {:?}", app.error);
    assert!(host_play::background_bots_acked());
    let _iso = iso;
}

#[test]
fn tui_successful_ack_preserves_unrelated_error() {
    let iso = IsolatedEnv::enter("tui-ack-unrelated");
    let mut session = TuiSession::new(dummy_options());
    let mut app = pump_two_slots(&mut session);
    app.error = Some("script: no focused profile".into());
    session.ack_background_bots(&mut app);
    assert!(app.background_notice.is_none());
    assert_eq!(app.error.as_deref(), Some("script: no focused profile"));
    assert!(host_play::background_bots_acked());
    let _iso = iso;
}

/// Vault with `alice` pinned to world 2 and auto-login on, in a temp dir.
fn lifecycle_vault(test: &str) -> Vault {
    let dir = std::env::temp_dir().join(format!("274bot-tui-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "bot").unwrap();
    vault
        .upsert(Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 7,
            settings: vault::ProfileSettings {
                auto_login: true,
                world: Some(2),
                ..vault::ProfileSettings::default()
            },
        })
        .unwrap();
    vault
}

#[test]
fn multibox_key_logs_a_loaded_logged_out_member_back_in() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut play = empty_play();
    let alice = SlotArm::new(7, true);
    alice.request_logout();
    alice.hold_logged_out();
    play.attach_arm("alice", Arc::clone(&alice));
    session.core.start(lifecycle_vault("multibox-rearm"), play);
    let mut app = TuiApp::new("tui");
    assert!(!alice.wants_login());

    dispatch(&mut session, &mut app, AppAction::SpawnAll);

    assert!(
        alice.wants_login(),
        "m must re-arm a member that is already loaded but logged out"
    );
    assert!(!alice.login_latched());
    assert!(session.core.fleet().contains("alice"));
}

#[test]
fn settings_popup_writes_only_guardian_fields() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let alice = SlotArm::new(7, true);
    session
        .core
        .start(lifecycle_vault("settings-fields"), empty_play());
    session
        .core
        .play_mut()
        .unwrap()
        .attach_arm("alice", Arc::clone(&alice));
    session.core.fleet_mut().add("alice");
    session.core.select("alice");
    let mut app = TuiApp::new("tui");
    // A stale popup draft: every non-guardian field at its default.
    app.settings = vault::ProfileSettings {
        random_events: false,
        lamp_skill: "Magic".into(),
        lamp_auto: true,
        ..vault::ProfileSettings::default()
    };
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());
    app.settings_dirty = true;

    session.pump(&mut app);
    assert!(
        alice.random_events.load(Ordering::Relaxed),
        "the arm waits for the durable write"
    );
    session.core.flush_writes();

    let saved = session.core.vault().unwrap().get("alice").unwrap().clone();
    assert!(!saved.settings.random_events);
    assert_eq!(saved.settings.lamp_skill, "Magic");
    assert!(saved.settings.lamp_auto);
    assert_eq!(saved.settings.world, Some(2), "world pin survives");
    assert!(saved.settings.auto_login, "auto-login survives");
    assert!(!alice.random_events.load(Ordering::Relaxed));
    assert_eq!(*alice.lamp_skill.lock().unwrap(), "Magic");
}

#[test]
fn removing_the_focused_member_focuses_its_neighbour() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let play = two_arm_play();
    play.statuses.lock().unwrap().push(host_play::SlotStatus {
        username: "bob".into(),
        connected: true,
        ..host_play::SlotStatus::default()
    });
    session
        .core
        .start(lifecycle_vault("remove-neighbour"), play);
    session.core.fleet_mut().add("alice");
    session.core.fleet_mut().add("bob");
    let bob = session.core.play().unwrap().arm("bob").unwrap();
    let mut app = TuiApp::new("tui");
    app.names = vec!["alice".into(), "bob".into()];
    app.focused = Some(1);
    dispatch(&mut session, &mut app, AppAction::Focus("bob".into()));

    dispatch(&mut session, &mut app, AppAction::Remove);
    session.pump(&mut app);

    assert_eq!(
        app.names,
        ["alice".to_string()],
        "the strip drops the member"
    );
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    let tab = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    );
    let action = app.on_key(tab);
    assert!(
        !matches!(&action, AppAction::Focus(name) if name == "bob"),
        "Tab must not reach a removed member still logging out: {action:?}"
    );
    assert_eq!(session.core.selected(), Some("alice"));
    assert_eq!(session.core.members(), ["alice".to_string()]);
    assert!(bob.wants_logout(), "a connected member logs out cleanly");
    assert!(
        !bob.stop.load(Ordering::Relaxed),
        "and is not stopped inline"
    );
}

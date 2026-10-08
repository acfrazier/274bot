use super::*;
use host_play::profile::ProfileEnvironment;
use host_play::{Play, SlotArm};
use nav::grid::StepGrid;
use nav::router::FindOptions;
use nav::world::NavWorld;
use script::IsolatedEnv;
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play as map_host;
#[path = "../../host-play/tests/support/map_fixture.rs"]
mod map_fixture;
#[path = "../../host-play/tests/support/upgrade_home.rs"]
mod upgrade_home;

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
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    }
}

fn walk_receipt_field<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.split_ascii_whitespace().find_map(|field| {
        let (key, value) = field.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn assert_walk_receipt_fields(
    message: &str,
    outcome: &str,
    destination: &str,
    at: &str,
    reason: &str,
    transport: &str,
) {
    for (name, expected) in [
        ("outcome", outcome),
        ("destination", destination),
        ("at", at),
        ("reason", reason),
        ("transport", transport),
    ] {
        assert_eq!(
            walk_receipt_field(message, name),
            Some(expected),
            "{name} in {message:?}"
        );
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
    cheats: Vec<String>,
}

impl api::interact::Driver for RecordingDriver {
    fn cheat_admission(&self) -> client::CheatAdmission {
        client::CheatAdmission::Granted
    }

    fn send_cheat(&mut self, cmd: &str) -> client::CheatSend {
        self.cheats.push(cmd.into());
        client::CheatSend::Sent
    }

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

    assert_eq!(
        driver.cheats,
        vec![
            format!("tele {}", api::interact::OFF_ISLAND_TELE),
            "setvar tutorial 1000".into()
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
fn parse_args_from_rs2b2t_selects_remote_profile() {
    let args = parse_args_from(["--rs2b2t"]).unwrap();
    assert!(args.profile.rs2b2t);
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
    assert_eq!(selection.profile_class(), host_play::ProfileClass::Remote);
    assert_eq!(selection.public_worlds().unwrap().worlds.len(), 2);
    std::fs::remove_dir_all(home).unwrap();
    assert!(parse_args_from(["--prod"]).is_err());
}

#[test]
fn world_flag_selects_auto_default_and_rejects_invalid_number() {
    let args = parse_args_from(["--profile", "public-289", "--world", "2"]).unwrap();
    assert_eq!(args.world, Some(2));
    assert!(parse_args_from(["--world", "0"]).is_err());
    assert!(parse_args_from(["--world", "not-a-number"]).is_err());
}

#[test]
fn the_passphrase_is_only_ever_read_from_stdin_never_an_argument() {
    assert!(!parse_args_from(["--user", "alice"]).unwrap().pass_stdin);
    assert!(parse_args_from(["--vault-pass-stdin"]).unwrap().pass_stdin);
    for spelling in [
        &["--vault-pass", "hunter2-hunter2"][..],
        &["--vault-pass=hunter2-hunter2"][..],
        &["--user", "alice", "--vault-pass", "hunter2-hunter2"][..],
    ] {
        let error = parse_args_from(spelling.iter().copied()).unwrap_err();
        assert!(error.contains("--vault-pass-stdin"), "{error}");
        assert!(
            !error.contains("hunter2"),
            "the value must not be echoed: {error}"
        );
    }
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
fn core_gate_flags_need_a_live_run_and_parse_the_environment_form() {
    let catalog = parse_args_from(["--live", "script_thiever", "--catalog-core"]).unwrap();
    assert!(catalog.catalog_core && !catalog.pair_core);
    let pair = parse_args_from(["--pair-core", "--live", "script_flax_runner"]).unwrap();
    assert!(pair.pair_core && !pair.catalog_core);
    assert!(parse_args_from(["--catalog-core"]).is_err());
    assert_eq!(live_core_from_env(Some("catalog")), Ok((true, false)));
    assert_eq!(live_core_from_env(Some("pair")), Ok((false, true)));
    assert_eq!(live_core_from_env(Some(" ")), Ok((false, false)));
    assert!(live_core_from_env(Some("core")).is_err());
}

/// A one-step scenario the synthetic client passes, so the runner reports
/// Passed before any shared witness has qualified.
fn passed_runner() -> scenario::ScenarioRunner {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
    let scenario = Scenario {
        name: "core_hold",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 5;
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 5 },
                budget_ticks: 5,
            },
        }],
        proof: Proof::Stat { id: 16, min: 5 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = client::client::Client::new(client::client::ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    });
    c.ingame = true;
    c.scene_state = 2;
    runner.tick(&mut c);
    c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
    runner.tick(&mut c);
    assert_eq!(runner.status(), scenario::RunnerStatus::Passed);
    runner
}

/// What `--catalog-core` prepare leaves: the Play's catalog witness armed for
/// the driven account and held by the run.
fn arm_catalog_witness(session: &mut TuiSession, play: &Play) {
    let catalog = play.catalog_core_watch();
    catalog.configure(host_play::catalog_core::CoreCase::Thiever, "alice");
    session.live_witnesses = Some(LiveWitnesses {
        catalog,
        pair: play.paired_core_watch(),
    });
}

#[test]
fn catalog_core_holds_live_pass_until_the_witness_qualifies_or_its_deadline_fails() {
    let play = empty_play();
    let mut session = TuiSession::new(dummy_options());
    arm_catalog_witness(&mut session, &play);
    session.inject_play(play);
    session.live_name = Some("core_hold".into());
    *session.scenario.lock().unwrap() = Some(passed_runner());
    session.live_core_deadline = Some(Instant::now() + Duration::from_secs(60));
    assert_eq!(
        session.live_status(),
        (None, Vec::new()),
        "a scenario-only PASS is not a catalog core PASS"
    );

    session.live_core_deadline = Some(Instant::now() - Duration::from_secs(1));
    let (code, lines) = session.live_status();
    assert_eq!(code, Some(1));
    let stderr = |needle: &str| {
        lines
            .iter()
            .any(|line| matches!(line, ProofLine::Stderr(text) if text.contains(needle)))
    };
    assert!(stderr("CATALOG_CORE: script_core_hold "), "{lines:?}");
    assert!(stderr("catalog core did not qualify"), "{lines:?}");
    assert!(
        !lines
            .iter()
            .any(|line| matches!(line, ProofLine::Stdout(text) if text.starts_with("PASS:"))),
        "{lines:?}"
    );
}

#[test]
fn only_the_witness_a_live_run_armed_decides_it() {
    // A catalog witness no live run armed, already failed at its Start.
    let play = empty_play();
    let unarmed = play.catalog_core_watch();
    unarmed.configure(host_play::catalog_core::CoreCase::Thiever, "alice");
    unarmed.observe(
        "alice",
        host_play::catalog_core::Observation::default(),
        false,
    );
    assert!(unarmed.begin_start("alice").is_err());
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);

    // Interactive tui-play polls every frame and has nothing to decide.
    assert_eq!(session.live_status(), (None, Vec::new()));

    // A plain `--live` run passes on its own proof, without a witness line.
    session.live_name = Some("plain".into());
    *session.scenario.lock().unwrap() = Some(passed_runner());
    let (code, lines) = session.live_status();
    assert_eq!(code, Some(0), "{lines:?}");
    assert!(
        lines.iter().all(|line| match line {
            ProofLine::Stdout(text) | ProofLine::Stderr(text) => !text.contains("CORE"),
        }),
        "{lines:?}"
    );
}

#[test]
fn paired_proof_refuses_a_live_run_without_the_pair_gate() {
    let _iso = IsolatedEnv::enter("tui-pair-refusal");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let error = session
        .live_prepare_script(scenario::get("nature_crafter_air").expect("registered"))
        .expect_err("a paired proof must not run outside the pair gate");
    assert!(error.contains("pair core gate"), "{error}");
    assert!(session.core.play().is_none(), "refused before any boot");
    assert!(session.pending_script.lock().unwrap().is_empty());
}

#[test]
fn pair_gate_stashes_both_slots_with_complementary_role_bags() {
    let iso = IsolatedEnv::enter("tui-pair-gate");
    let root = iso.dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("NatureCrafter")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import NatureCrafter from './NatureCrafter/NatureCrafter.js';
ScriptRegistry.register({ name: 'NatureCrafter', create: () => new NatureCrafter() });
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("NatureCrafter/NatureCrafter.ts"),
        "export default class NatureCrafter extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    iso.set_rs2b0t(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session.live_pair_core = true;
    session
        .live_prepare_script(scenario::get("nature_crafter_air").expect("registered"))
        .expect("prepare");
    let pending = session.pending_script.lock().unwrap();
    let slots = pending
        .iter()
        .map(|start| start.slot.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        slots,
        [session.names[0].as_str(), session.names[1].as_str()]
    );
    let modes = pending
        .iter()
        .map(|start| {
            start
                .bag
                .as_ref()
                .and_then(|bag| bag.get("mode"))
                .and_then(serde_json::Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(modes, [Some("Master"), Some("Runner")]);
    assert!(session
        .core
        .play()
        .expect("play")
        .paired_core_watch()
        .configured());
    assert!(session.live_core_deadline.is_some());
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
            engine_dir: Some(root.join("engine")),
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
fn plaintext_offhost_requires_explicit_opt_in() {
    assert!(host_play::validate_play_host("attacker.example", false).is_err());
    assert!(host_play::validate_play_host("127.0.0.1", false).is_ok());
    assert!(host_play::validate_play_host("attacker.example", true).is_ok());
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
        .start_play(Vault::create(&path, "test-passphrase-01").unwrap())
        .unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let err = session.create_profile("alice").unwrap_err();
    assert!(
        err.starts_with("profile:"),
        "upsert failure must return Err, got {err:?}"
    );
}

/// M-311: a fresh vault starts empty — interactive boot must not seed a
/// `test`/`test` profile. `--user` still creates the first profile, and an
/// existing vault keeps its profiles.
#[test]
fn fresh_vault_starts_empty_without_a_seeded_test_profile() {
    let iso = IsolatedEnv::enter("tui-fresh-vault-empty");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .start_play(Vault::create(&iso.dir.join("vault"), "test-passphrase-01").unwrap())
        .unwrap();
    // No `--user`: an honest error, and no `test` profile left behind.
    let err = session.bootstrap_interactive_profiles(&[]).unwrap_err();
    assert_eq!(
        err,
        "vault has no profiles (start tui-play with --user <name> to create one)"
    );
    assert!(session.names.is_empty(), "names: {:?}", session.names);
    assert!(
        session.core.vault().is_none_or(|v| v.get("test").is_none()),
        "fresh vault must not seed `test`"
    );
    // `--user` creates the first profile (and focuses it).
    assert_eq!(
        session
            .bootstrap_interactive_profiles(&["alice".to_string()])
            .unwrap(),
        "alice"
    );
    assert_eq!(session.names, vec!["alice".to_string()]);
    assert!(
        session
            .core
            .vault()
            .is_some_and(|v| v.get("test").is_none() && v.get("alice").is_some()),
        "only `--user` profiles exist"
    );
    // An existing vault keeps its profiles: no seeding, no wipe.
    assert_eq!(
        session.bootstrap_interactive_profiles(&[]).unwrap(),
        "alice"
    );
    assert_eq!(session.names, vec!["alice".to_string()]);
}

/// A 0.1.9.x home keeps its v10 pack at the default path. The TUI's play and
/// its WalkTo/scenario world are the packaged v11 bundle's one shared world,
/// and the old file is left alone; an explicit override to the old file still
/// starts, with no world.
#[test]
fn tui_upgraded_from_v10_home_plays_on_the_packaged_v11_world() {
    let home = upgrade_home::UpgradeHome::new();
    let start = |profile: Arc<host_play::ServerProfile>, vault: &str| {
        let template = SharedClientTemplate::load(profile).unwrap();
        let mut session =
            TuiSession::new_bound(Arc::clone(&template), host_play::InstancePermit::SkipLock);
        let vault = Vault::create(&home.root.join(vault), "test-passphrase-01").unwrap();
        session.start_play(vault).unwrap();
        (session, template)
    };

    let profile = home.bind(None);
    assert!(profile.nav_origin().is_bundled());
    let (session, template) = start(profile, "bundled.vault");
    let world = template.world().expect("the packaged world decodes");
    let played = session
        .core
        .play()
        .and_then(|play| play.world())
        .expect("the TUI play has the packaged world");
    assert!(Arc::ptr_eq(&played, &world));
    let routed = session.nav_world.lock().unwrap().clone();
    assert!(Arc::ptr_eq(&routed.expect("WalkTo world"), &world));
    assert!(home.old_pack_untouched());

    let overridden = home.bind(Some(home.old_pack.clone()));
    assert!(matches!(
        overridden.nav_availability(),
        host_play::profile::NavAvailability::Unavailable(message)
            if message.contains("rebuild it with nav-pack")
    ));
    let (session, _) = start(overridden, "override.vault");
    assert!(session.core.play().unwrap().world().is_none());
    assert!(session.nav_world.lock().unwrap().is_none());
    assert!(home.old_pack_untouched());
}

#[test]
fn tui_reach_layer_uses_shared_lazy_sidecar_and_releases_its_lease() {
    let home = upgrade_home::UpgradeHome::new();
    let profile = home.bind(None);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let mut session = TuiSession::new_bound(template, host_play::InstancePermit::SkipLock);
    *session.nav_world.lock().unwrap() = profile.world();
    let mut app = TuiApp::new("reach fixture");
    app.map_active = true;
    session.pump(&mut app);
    assert!(
        app.map_reach.is_none(),
        "map default never requests paint words"
    );
    app.map.layers.reach = true;
    session.pump(&mut app);
    let first = app.map_reach.clone().expect("layer requests bound sidecar");
    assert_eq!(&*first, &[0u64]);
    assert!(Arc::ptr_eq(&first, &profile.reach().unwrap()));
    let refs = Arc::strong_count(&first);
    for _ in 0..4 {
        dispatch(&mut session, &mut app, AppAction::MapClose);
        assert!(app.map_reach.is_none());
        app.map_active = true;
        session.pump(&mut app);
        assert!(Arc::ptr_eq(&first, app.map_reach.as_ref().unwrap()));
        assert_eq!(
            Arc::strong_count(&first),
            refs,
            "reopen must not retain another lease"
        );
    }
    dispatch(&mut session, &mut app, AppAction::MapClose);
    assert_eq!(Arc::strong_count(&first), refs - 1);
    *session.nav_world.lock().unwrap() = Some(Arc::new(nav::world::NavWorld::from_grid(
        &nav::grid::StepGrid::fixture_open_3x3(),
    )));
    app.map_active = true;
    session.pump(&mut app);
    assert!(
        app.map_reach.is_none(),
        "another world's paint mask must not be reused"
    );
}

fn fake_rs2b0t_tree(dir: &Path, include_jive_kq: bool) -> PathBuf {
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    let mut index = r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
ScriptRegistry.register({
  name: 'BoneBurier',
  description: 'Buries bones',
  category: 'Prayer',
  tags: ['bones'],
  create: () => new BoneBurier(),
});
"#
    .to_string();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    if include_jive_kq {
        std::fs::create_dir_all(scripts.join("JiveKQ")).unwrap();
        index.push_str(
            r#"
import JiveKQ from './JiveKQ/JiveKQ.js';
ScriptRegistry.register({
  name: 'JiveKQ',
  create: () => new JiveKQ(),
});
"#,
        );
        std::fs::write(
            scripts.join("JiveKQ/JiveKQ.ts"),
            "export default class JiveKQ extends LoopingBot { override loop() {} }",
        )
        .unwrap();
    }
    std::fs::write(scripts.join("index.ts"), index).unwrap();
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
    let root = fake_rs2b0t_tree(&iso.dir, false);
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
    let pending = session.pending_script.lock().unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|pending| pending.slot.as_str())
            .collect::<Vec<_>>(),
        [name.as_str()],
        "preparation stages the selected card for StartScript"
    );
}

#[test]
fn live_prepare_jive_kq_stashes_four_starts_with_the_same_roster() {
    let iso = IsolatedEnv::enter("tui-jive-kq-fleet");
    let root = fake_rs2b0t_tree(&iso.dir, true);
    iso.set_rs2b0t(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(scenario::get("jive_kq_four").expect("registered"))
        .expect("prepare four-player JiveKQ");

    assert_eq!(session.names.len(), 4, "JiveKQ mints four fleet profiles");
    let roster = session
        .names
        .iter()
        .map(|name| client::util::JString::to_screen_name(name))
        .collect::<Vec<_>>()
        .join(",");
    let starts = session.pending_script.lock().unwrap();
    assert_eq!(starts.len(), 4, "StartScript prepares all four isolates");
    let mut prepared_slots = starts
        .iter()
        .map(|start| start.slot.clone())
        .collect::<Vec<_>>();
    prepared_slots.sort();
    let mut fleet_slots = session.names.clone();
    fleet_slots.sort();
    assert_eq!(
        prepared_slots, fleet_slots,
        "each fleet profile has exactly one prepared start"
    );
    for start in starts.iter() {
        assert_eq!(
            start.bag.as_ref().and_then(|settings| settings.get("team")),
            Some(&serde_json::json!(roster)),
            "{} receives the complete shared roster",
            start.slot
        );
    }
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
        session.scripts.js = script::JsLibrary::with_cache(
            iso.dir.join("js-scripts.json"),
            iso.dir.join("js-cache"),
        );
        session.scripts.js.load(&ts).expect("preload ts");
        session.scripts.js.load(&js).expect("preload js");
        session.scripts.legacy.set_str(
            script::ScriptSource::File,
            "bone_burier_v2",
            "boneName",
            "stem",
        );
        let identity = script::file_identity(path);
        session.scripts.legacy.set_str(
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
        let pending = session.pending_script.lock().unwrap().clone();
        let [pending] = pending.as_slice() else {
            panic!("{name}: one File start stashed, got {}", pending.len());
        };
        let bag = pending.bag.clone().expect("settings bag");
        assert_eq!(bag.get("boneName"), Some(&serde_json::json!("identity")));
        assert!(
            pending.loadouts.is_empty(),
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
    let [pending] = pending.as_slice() else {
        panic!("one catalog start stashed, got {}", pending.len());
    };
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
            .scripts
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
    let root = fake_rs2b0t_tree(&iso.dir, false);
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    let n = session
        .import_rs2b0t_catalog(&mut app, &root)
        .expect("import");
    assert_eq!(n, 1);
    assert!(iso.home.join(".274bot/rs2b0t-path").is_file());
    let card = session
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "BoneBurier")
        .expect("catalog card");
    assert_eq!(card.description, "Buries bones");
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
    while session.scripts.starts_pending() && Instant::now() < deadline {
        session.core.poll();
        session.poll_scripts(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !session.scripts.starts_pending(),
        "script Start did not settle"
    );
}

#[test]
fn initial_runtime_failure_survives_success_and_refusals_in_tui_output() {
    let iso = IsolatedEnv::enter("tui-initial-load");
    let mut session = TuiSession::new(dummy_options());
    let mut vault = Vault::create(&iso.dir.join("vault"), "test-passphrase-01").unwrap();
    for (uid, name) in [(7, "alice"), (8, "bob")] {
        vault
            .upsert(vault::Profile {
                username: name.into(),
                password: "pw".into(),
                uid,
                settings: vault::ProfileSettings::default(),
            })
            .unwrap();
    }
    session.core.set_vault(Some(vault));
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
    let card = session.scripts.js.load(&path).unwrap();
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
        .scripts
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
    let good = session.scripts.js.load(&good_path).unwrap();
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
        session.scripts.js.load_failure(&card.identity_key()),
        Some(&failure)
    );
    app.focused = Some(2);
    session.script_start(&mut app, &sel);
    assert_eq!(app.error.as_deref(), Some("script: no slot: missing"));
    assert_eq!(
        session.scripts.js.load_failure(&card.identity_key()),
        Some(&failure)
    );

    std::fs::write(&helper, "export const fail = false;").unwrap();
    app.focused = Some(0);
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    assert!(session
        .scripts
        .js
        .load_failure(&card.identity_key())
        .is_none());
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
        // The withdraw grid's full inventory packet: the bank is loaded, as
        // the slot's bank memory needs before it observes the rows.
        let mut full = client::io::Packet::new(vec![2, 89, 2, 0, 3, 20, 0, 0, 0]);
        c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut full);
        c
    }

    pub fn knife_nav_world(knife_id: i32) -> NavWorld {
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
        }
        let edge = TransportEdge {
            takeoff: None,
            worn_all_req: Vec::new(),
            kind: TransportKind::Door,
            player_delta: None,
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
            consumed_req: vec![],
            item_returns: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![knife_id],
            members_req: false,
            wildy_cap: None,
            quest_gates: None,
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
    app.nav.set_danger_level(frontend_core::DangerLevel::Always);
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

/// A map walk the TUI arms shows on the member's fleet row (what the strip
/// and the status pane draw) until the route ends.
#[test]
fn a_map_walk_shows_the_member_running_on_its_fleet_row() {
    use api::snapshot::WorldTile as SnapTile;
    use bank_fetch_fixtures::knife_nav_world;

    let mut session = TuiSession::new(dummy_options());
    *session.nav_world.lock().unwrap() = Some(Arc::new(knife_nav_world(2)));
    session.core.fleet_mut().add("alice");
    session.core.select("alice");
    let mut app = TuiApp::new("274bot headless");
    app.nav.set_danger_level(frontend_core::DangerLevel::Always);
    session.pump(&mut app);
    assert!(!app.fleet[0].walking);

    app.here = Some(SnapTile {
        x: 0,
        z: 0,
        level: 0,
    });
    // West of the fixture's knife door: reachable on foot.
    session.arm_walk_on(
        &mut app,
        Tile {
            x: 1,
            z: 4,
            level: 0,
        },
    );
    assert_eq!(app.error, None);
    session.pump(&mut app);
    assert!(app.fleet[0].walking, "the armed walk shows on the row");
    assert!(app.fleet[0].busy());

    let arm = Arc::clone(session.travellers.lock().unwrap().get("alice").unwrap());
    arm.lock().unwrap().route = None;
    session.pump(&mut app);
    assert!(!app.fleet[0].walking, "an ended route clears the row");
}

#[test]
fn resetting_the_same_walk_slot_session_twice_logs_one_cancellation() {
    use api::snapshot::WorldTile;
    use frontend_core::log::{global, LogScope, LogView};
    use nav::router::Route;

    let log = global();
    let slot = "walk-receipt-session-ended";
    let destination = WorldTile {
        x: 8,
        z: 9,
        level: 0,
    };
    let arm = Arc::new(Mutex::new(WalkArm {
        route: Some(Arc::new(Route {
            dest: destination,
            legs: vec![],
            ticks: 0.0,
        })),
        ..Default::default()
    }));
    let travellers: SlotTravellers = Arc::new(Mutex::new(HashMap::from([(slot.to_owned(), arm)])));
    let tick_latch = Arc::new(Mutex::new(HashMap::from([(
        slot.to_owned(),
        (1, (8, 9, 0)),
    )])));

    assert!(reset_frontend_slot_session(slot, &travellers, &tick_latch));
    assert!(!reset_frontend_slot_session(slot, &travellers, &tick_latch));

    let mut receipts = LogView::new(LogScope::Slot(slot.into()));
    receipts.edit_filter(|filter| filter.text = "WalkTo outcome=".into());
    log.refresh(&mut receipts);
    assert_eq!(receipts.len(), 1, "{}", receipts.to_text());
    assert_walk_receipt_fields(
        receipts.rows()[0].message.as_ref(),
        "cancelled",
        "(8,9,0)",
        "unknown",
        "SessionEnded",
        "-",
    );
}

#[test]
fn production_walk_receipt_uses_snapped_destination() {
    use frontend_core::log::{global, LogScope, LogView};
    let log = global();
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let mut session = TuiSession::new(dummy_options());
    session.server_profile = Some(Arc::clone(fixture.template.profile()));
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    session.core.set_play(Some(fixture.play(origin)));
    let mut app = TuiApp::new("274bot headless");
    let name = "tui-walk-receipt";
    app.names = vec![name.into()];
    app.focused = Some(0);
    app.map_active = true;
    app.world = Some(Arc::clone(&world));
    session.bind_map_context(&mut app);
    let requested = Tile {
        x: 5,
        z: 2,
        level: 0,
    };
    app.select_requested(requested);
    let target = app.map_model.pending().unwrap().target.unwrap();
    assert_ne!(target, requested);
    app.here = None;
    session.arm_walk_on(&mut app, requested);
    assert!(app.walk_dest.is_none());
    let mut view = LogView::new(LogScope::Slot(name.into()));
    view.edit_filter(|filter| filter.text = "WalkTo ".into());
    log.refresh(&mut view);
    assert_eq!(view.len(), 2, "{}", view.to_text());
    let request = view
        .rows()
        .iter()
        .find(|row| row.message.starts_with("WalkTo origin="))
        .expect("request receipt");
    assert!(request.message.contains("refused=NoOrigin"), "{request:?}");
    assert!(
        request.message.contains(&format!(
            "destination=({},{},{})",
            target.x, target.z, target.level
        )),
        "{request:?}"
    );
    assert!(
        request.message.contains("members_source=unknown "),
        "{request:?}"
    );
    let terminal = view
        .rows()
        .iter()
        .find(|row| row.message.starts_with("WalkTo outcome="))
        .expect("terminal receipt");
    assert_walk_receipt_fields(
        terminal.message.as_ref(),
        "aborted",
        &format!("({},{},{})", target.x, target.z, target.level),
        "unknown",
        "NoOrigin",
        "-",
    );
}

#[test]
fn empty_group_walk_preserves_selection_and_reports_refusal() {
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.world = Some(Arc::new(bank_fetch_fixtures::knife_nav_world(2)));
    app.select_requested(Tile {
        x: 1,
        z: 1,
        level: 0,
    });
    app.walk_send.mode = crate::app::WalkSendMode::Group;
    assert_eq!(app.map_enter(), AppAction::None);
    assert_eq!(app.error.as_deref(), Some("no bots selected"));
    assert!(app.map_model.pending().is_some());
    app.error = None;
    session.map_walk_group(&mut app);
    assert_eq!(app.error.as_deref(), Some("no bots selected"));
    assert!(app.map_model.pending().is_some());
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
    let final_route = Arc::new(Route {
        dest: WorldTile {
            x: 99,
            z: 99,
            level: 0,
        },
        legs: vec![],
        ticks: 0.0,
    });
    let pending = PendingBankFetch {
        steps: VecDeque::from([BankStep::Walk {
            x: here.0,
            z: here.1,
            level: 1,
        }]),
        dest: final_route.dest,
        opts: FindOptions::default(),
        final_route: final_route.clone(),
        avoid: Vec::new(),
        progress: Default::default(),
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
    c.set_iface_mut(
        601,
        client::config::IfTypeMut {
            link_obj_type: Some(vec![3, 0]), // bank item ids are stored as id + 1
            link_obj_number: Some(vec![1, 0]),
            ..Default::default()
        },
    );
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&c);
    let world = Arc::new(knife_nav_world(2));
    let final_route = Arc::new(Route {
        dest: WorldTile {
            x: 4,
            z: 4,
            level: 0,
        },
        legs: vec![],
        ticks: 0.0,
    });
    let mut arm = WalkArm {
        bank_fetch: Some(PendingBankFetch {
            steps: VecDeque::from([BankStep::Withdraw { id: 2, count: 1 }, BankStep::Close]),
            dest: WorldTile {
                x: 4,
                z: 4,
                level: 0,
            },
            opts: FindOptions::default(),
            final_route: final_route.clone(),
            avoid: Vec::new(),
            progress: Default::default(),
        }),
        route: Some(Arc::new(Route {
            dest: WorldTile {
                x: 0,
                z: 4,
                level: 0,
            },
            legs: vec![],
            ticks: 0.0,
        })),
        ..Default::default()
    };
    let before = c.out.pos;
    host_play::step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arm,
        Some(world.as_ref()),
        (0, 4, 0),
        false,
        None,
    );
    assert!(
        c.out.pos > before,
        "BankBudget pump must drive the shortage withdrawal on the Driver (pos {before} → {})",
        c.out.pos
    );
}

#[test]
fn bank_stand_subroute_does_not_emit_operator_terminal_receipt() {
    use api::snapshot::WorldTile;
    use bank_fetch_fixtures::bank_client;
    use frontend_core::log::{global, LogScope, LogView};
    use host_play::PendingBankFetch;
    use nav::bank_fetch::BankStep;
    use nav::router::{Leg, Route};
    use std::collections::VecDeque;

    let log = global();
    let mut c = bank_client();
    let here = host_play::player_here_tile(&c).expect("fixture player");
    let at = WorldTile {
        x: here.0,
        z: here.1,
        level: here.2,
    };
    let stand = WorldTile { x: at.x + 1, ..at };
    let destination = WorldTile { x: at.x + 2, ..at };
    let final_route = Arc::new(Route {
        dest: destination,
        legs: vec![Leg::Walk {
            tiles: vec![stand, destination],
        }],
        ticks: 1.0,
    });
    let mut arm = WalkArm {
        bank_fetch: Some(PendingBankFetch {
            steps: VecDeque::from([BankStep::Walk {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            }]),
            dest: destination,
            opts: FindOptions::default(),
            final_route: final_route.clone(),
            avoid: Vec::new(),
            progress: Default::default(),
        }),
        route: Some(Arc::new(Route {
            dest: stand,
            legs: vec![Leg::Walk {
                tiles: vec![at, stand],
            }],
            ticks: 1.0,
        })),
        route_generation: 103,
        ..Default::default()
    };
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&c);
    assert!(!host_play::step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arm,
        None,
        here,
        false,
        Some("receipt-bank-root"),
    ));

    c.local_player = Some(client::client::ClientPlayer::at(6, 5));
    c.bump_gens(client::io::ServerProt::PLAYER_INFO);
    snap.rebuild(&c);
    assert!(
        !host_play::step_walk_arm_follow(
            &mut c,
            &snap,
            &mut arm,
            None,
            (stand.x, stand.z, stand.level),
            false,
            Some("receipt-bank-root"),
        ),
        "completing the internal stand route must not finish the operator route"
    );
    assert_eq!(
        arm.route.as_ref().map(|route| route.dest),
        Some(destination)
    );
    let mut receipts = LogView::new(LogScope::Slot("receipt-bank-root".into()));
    receipts.edit_filter(|filter| filter.text = "WalkTo outcome=".into());
    log.refresh(&mut receipts);
    assert_eq!(receipts.len(), 0, "{}", receipts.to_text());

    c.local_player = Some(client::client::ClientPlayer::at(7, 5));
    c.bump_gens(client::io::ServerProt::PLAYER_INFO);
    snap.rebuild(&c);
    assert!(host_play::step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arm,
        None,
        (destination.x, destination.z, destination.level),
        false,
        Some("receipt-bank-root"),
    ));
    log.refresh(&mut receipts);
    assert_eq!(receipts.len(), 1, "{}", receipts.to_text());
    let message = receipts.rows()[0].message.as_ref();
    let tile = format!(
        "({},{},{})",
        destination.x, destination.z, destination.level
    );
    assert_walk_receipt_fields(message, "arrived", &tile, &tile, "-", "-");
    assert_eq!(walk_receipt_field(message, "leg"), Some("-"), "{message:?}");
}

#[test]
fn operator_walk_logs_one_terminal_receipt_for_arrival_and_abort() {
    use api::snapshot::WorldTile;
    use bank_fetch_fixtures::bank_client;
    use frontend_core::log::{global, LogScope, LogView};
    use nav::router::{Leg, Route};
    use nav::transport::{TransportEdge, TransportKind};

    let log = global();

    let mut c = bank_client();
    let here = host_play::player_here_tile(&c).expect("fixture player");
    let at = WorldTile {
        x: here.0,
        z: here.1,
        level: here.2,
    };
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&c);

    let arrived_route = Route {
        dest: at,
        legs: vec![],
        ticks: 0.0,
    };
    let mut arrived = WalkArm {
        route: Some(Arc::new(arrived_route)),
        route_generation: 101,
        ..Default::default()
    };
    assert!(host_play::step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arrived,
        None,
        here,
        false,
        Some("receipt-arrived"),
    ));
    assert!(!host_play::step_walk_arm_follow(
        &mut c,
        &snap,
        &mut arrived,
        None,
        here,
        false,
        Some("receipt-arrived"),
    ));

    let destination = WorldTile {
        x: at.x + 10,
        z: at.z,
        level: at.level,
    };
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Boat,
        player_delta: None,
        at,
        to: destination,
        loc_id: 378,
        option: 1,
        ticks: 7,
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
    };
    let aborted_route = Route {
        dest: destination,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        ticks: 7.0,
    };
    let mut aborted = WalkArm {
        route: Some(Arc::new(aborted_route)),
        route_generation: 102,
        ..Default::default()
    };
    let mut finished = 0;
    for _ in 0..70 {
        finished += usize::from(host_play::step_walk_arm_follow(
            &mut c,
            &snap,
            &mut aborted,
            None,
            here,
            false,
            Some("receipt-aborted"),
        ));
    }
    assert_eq!(finished, 1, "the route has one terminal transition");

    let mut arrived = LogView::new(LogScope::Slot("receipt-arrived".into()));
    arrived.edit_filter(|filter| filter.text = "WalkTo outcome=".into());
    log.refresh(&mut arrived);
    let mut aborted = LogView::new(LogScope::Slot("receipt-aborted".into()));
    aborted.edit_filter(|filter| filter.text = "WalkTo outcome=".into());
    log.refresh(&mut aborted);
    assert_eq!(arrived.len(), 1, "{}", arrived.to_text());
    assert_eq!(aborted.len(), 1, "{}", aborted.to_text());
    let arrived_message = arrived.rows()[0].message.as_ref();
    let at = format!("({},{},{})", at.x, at.z, at.level);
    assert_walk_receipt_fields(arrived_message, "arrived", &at, &at, "-", "-");
    assert_eq!(
        walk_receipt_field(arrived_message, "leg"),
        Some("-"),
        "{arrived_message:?}"
    );
    let aborted_message = aborted.rows()[0].message.as_ref();
    assert_walk_receipt_fields(
        aborted_message,
        "aborted",
        &format!(
            "({},{},{})",
            destination.x, destination.z, destination.level
        ),
        &at,
        "Blocked",
        "Boat:378",
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
fn tui_manual_walk_pause_preference_projects_and_reports_persistence() {
    let iso = IsolatedEnv::enter("tui-manual-walk-pause");
    let path = host_play::panel_ui_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"capture":false,"nav":{"pause_script_on_manual_walk_abort":false,"show_special_areas":true}}"#,
    )
    .unwrap();
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.restore_preferences(path.clone());
    assert!(!app.pause_script_on_manual_walk_abort);

    app.pause_script_on_manual_walk_abort = true;
    app.pause_script_on_manual_walk_abort_dirty = true;
    session.pump(&mut app);
    let prefs: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(prefs["capture"], false);
    assert_eq!(prefs["nav"]["show_special_areas"], true);
    assert_eq!(prefs["nav"]["pause_script_on_manual_walk_abort"], true);
    assert!(!app.pause_script_on_manual_walk_abort_dirty);

    let blocker = iso.dir.join("not-a-directory");
    std::fs::write(&blocker, b"file").unwrap();
    app.restore_preferences(blocker.join("panel-ui.json"));
    app.pause_script_on_manual_walk_abort = false;
    app.pause_script_on_manual_walk_abort_dirty = true;
    session.pump(&mut app);
    assert!(
        app.error
            .as_deref()
            .is_some_and(|error| error.starts_with("settings: pause script on manual movement:")),
        "failed writes must reach the TUI error surface: {:?}",
        app.error
    );
}

#[test]
fn tui_quester_paths_failed_save_keeps_source_and_error_without_reloading() {
    let iso = IsolatedEnv::enter("tui-quester-paths-save-failure");
    let blocker = iso.dir.join("not-a-directory");
    std::fs::write(&blocker, b"file").unwrap();
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.restore_preferences(blocker.join("panel-ui.json"));
    let before = app.quester_paths.clone();
    app.quester_paths.enabled = true;
    app.quester_paths.folder = iso.dir.join("paths");
    app.quester_paths_dirty = true;

    session.project_quester_paths(&mut app);

    assert_eq!(app.quester_paths, before);
    assert!(!app.quester_paths_controller.is_running());
    assert!(app.quester_paths_controller.notice_is_error());
    assert!(app
        .quester_paths_controller
        .notice_text()
        .unwrap()
        .starts_with("Quest Paths settings were not saved:"));
}

#[test]
fn tui_restored_enabled_folder_loads_when_game_data_becomes_ready() {
    let iso = IsolatedEnv::enter("tui-quester-paths-startup");
    let folder = iso.dir.join("paths");
    std::fs::create_dir_all(&folder).unwrap();
    for (id, name) in [("cook", "Folder Cook"), ("fresh-draft", "Fresh Draft")] {
        let mut document: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../script/paths/289/cook.json"
        )))
        .unwrap();
        document["id"] = id.into();
        document["display_name"] = name.into();
        std::fs::write(
            folder.join(format!("{id}.json")),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
    }
    let prefs = iso.dir.join("panel-ui.json");
    std::fs::write(
        &prefs,
        serde_json::to_vec(&serde_json::json!({
            "quester_paths": {
                "enabled": true,
                "folder": folder.to_string_lossy()
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.restore_preferences(prefs.clone());
    assert!(app.quester_paths.enabled);
    let selected = api::selected::FamilyPreparation::run(|_| {
        api::game_data::for_revision(client::io::ClientRevision::R289)
            .expect("selected 289 game data")
    })
    .expect("spawn selected data worker")
    .join()
    .expect("selected data worker");

    session.project_quester_paths_with_game_data(&mut app, Some(Arc::clone(&selected)));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.quester_paths_controller.is_running() {
        assert!(
            Instant::now() < deadline,
            "startup Path reload did not finish"
        );
        session.project_quester_paths_with_game_data(&mut app, Some(Arc::clone(&selected)));
        std::thread::yield_now();
    }

    let registry = script::quester::registry::snapshot();
    assert!(registry.rows().iter().any(|row| {
        row.id == "cook" && row.source == script::quester::registry::PathSource::Folder
    }));
    assert!(registry.rows().iter().any(|row| {
        row.id == "fresh-draft" && row.source == script::quester::registry::PathSource::Draft
    }));
    let report = app
        .quester_paths_controller
        .notice_text()
        .expect("startup validation report");
    assert!(report.contains("2 folder documents"), "{report}");
    assert!(report.contains("0 validation errors"), "{report}");
    script::quester::registry::set_source(script::quester::registry::FolderSource::default());
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
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
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
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());
    // The popup binds the focused profile when it opens.
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    // A stale popup draft: every non-guardian field at its default.
    app.settings = vault::ProfileSettings {
        random_events: false,
        lamp_skill: "Magic".into(),
        lamp_auto: true,
        ..vault::ProfileSettings::default()
    };
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

    dispatch(&mut session, &mut app, AppAction::Remove("bob".into()));
    session.pump(&mut app);

    assert_eq!(
        app.names,
        ["alice".to_string()],
        "the fleet table drops the member"
    );
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    assert!(
        !matches!(
            app.run_command(crate::commands::Command::SelectNextBot),
            AppAction::Focus(name) if name == "bob"
        ),
        "selecting the next bot must not reach a removed member still logging out"
    );
    assert_eq!(session.core.selected(), Some("alice"));
    assert_eq!(session.core.members(), ["alice".to_string()]);
    assert!(bob.wants_logout(), "a connected member logs out cleanly");
    assert!(
        !bob.stop.load(Ordering::Relaxed),
        "and is not stopped inline"
    );
}

const LOOPING_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";
const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// A TUI session over a real vault and an empty play: members loaded
/// (arms attached, no worker threads), library in the isolated home.
fn tui_with_profiles(iso: &IsolatedEnv, names: &[&str]) -> (TuiSession, TuiApp) {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = Vault::create(&iso.dir.join("vault"), "test-passphrase-01").unwrap();
    for (i, name) in names.iter().enumerate() {
        vault
            .upsert(vault::Profile {
                username: (*name).into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: vault::ProfileSettings::default(),
            })
            .unwrap();
    }
    session.core.set_vault(Some(vault));
    session.inject_play(run_with_io(
        &dummy_options(),
        vec![],
        |_| (None, None),
        |_, _, _| {},
    ));
    session.scripts.js =
        script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    let mut surface = HeadlessSurface::new();
    for name in names {
        session.core.load(name, &mut surface);
    }
    let mut app = TuiApp::new("scripts");
    session.pump(&mut app);
    (session, app)
}

fn assign(session: &mut TuiSession, name: &str, card: &script::JsCard) {
    assert!(session
        .scripts
        .persist_assignment(&mut session.core, name, card.assignment()));
    session.core.flush_writes();
}

fn focus_member(session: &mut TuiSession, app: &mut TuiApp, name: &str) {
    session.focus(name);
    session.pump(app);
}

#[test]
fn restart_required_prompt_is_visible_at_standard_and_compact_sizes() {
    let iso = IsolatedEnv::enter("tui-restart-settings-prompt");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    session.core.play_mut().unwrap().bind_script_test_data(data);
    let id = script::CompiledId("Gatherer");
    assert!(session.scripts.persist_assignment(
        &mut session.core,
        "alice",
        script::compiled_assignment(id)
    ));
    session.core.flush_writes();
    session
        .scripts
        .set_compiled_setting(
            &mut session.core,
            "alice",
            id,
            "skill",
            serde_json::json!("Fishing"),
        )
        .unwrap();
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    session
        .scripts
        .start_profile(&mut session.core, "alice", None)
        .unwrap();
    settle_starts(&mut session, &mut app);
    wait_script_state(
        session.core.play().unwrap(),
        "alice",
        script::RunState::Running,
    );

    session
        .scripts
        .set_compiled_setting(
            &mut session.core,
            "alice",
            id,
            "skill",
            serde_json::json!("Mining"),
        )
        .unwrap();
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    let prompt = session.scripts.pending_restart_prompt().unwrap();
    let summary = prompt.summary();
    assert!(matches!(
        app.modal.as_ref(),
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(
                &confirm.kind,
                crate::overlay::ConfirmKind::RestartSettings(current) if current == prompt
            )
    ));

    for (width, height) in [(120, 40), (80, 24)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw_modal(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains(&summary), "{width}x{height}: {text}");
        assert!(
            text.contains("[Restart y]"),
            "{width}x{height}: missing restart action: {text}"
        );
        assert!(
            text.contains("[Later n]"),
            "{width}x{height}: missing Later action: {text}"
        );
    }
    session.scripts.dismiss_restart_prompt();
}

/// Live finding (TUI parity): the settings popup closed without explanation
/// when focus moved, dropping the draft. It must stay open and bound to the
/// profile it was opened for, and persist there — never to the newly
/// focused profile.
#[test]
fn settings_popup_stays_bound_to_its_profile_across_focus_change() {
    let iso = IsolatedEnv::enter("tui-settings-bind");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    session
        .core
        .play()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .extend(["alice", "bob"].map(|name| host_play::SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..host_play::SlotStatus::default()
        }));
    session.core.poll();
    session.core.set_memory_mode("alice", false).unwrap();
    session.core.flush_writes();
    focus_member(&mut session, &mut app, "alice");
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    assert!(app.settings_state.open, "settings opens");
    // Toggle random events off: the draft belongs to alice.
    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(!app.settings.random_events, "the toggle edits the draft");
    // Switch focus to bob: the popup stays open on alice's draft, titled
    // with the profile it is bound to.
    focus_member(&mut session, &mut app, "bob");
    assert!(
        !app.memory.unwrap().differs(),
        "the status pane follows focused bob"
    );
    assert!(
        app.settings_memory.unwrap().differs(),
        "the popup notice stays bound to alice"
    );
    assert!(
        app.settings_state.open,
        "a focus change never closes the form"
    );
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(
            cell_of(&screen, "settings — alice").is_some(),
            "{w}x{h}: the title names the bound profile"
        );
    }
    assert!(
        !app.settings.random_events,
        "a focus change never rewrites the buffers"
    );
    // Persisting writes alice's row, never bob's.
    session.pump(&mut app);
    session.core.flush_writes();
    let vault = session.core.vault().unwrap();
    assert!(
        !vault.get("alice").unwrap().settings.random_events,
        "the edit lands on the bound profile"
    );
    assert!(
        vault.get("bob").unwrap().settings.random_events,
        "bob untouched"
    );
}

fn settings_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

/// The terminal sizes the TUI is checked at.
const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

/// The whole TUI drawn at `w`×`h`.
fn screen(app: &mut TuiApp, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

/// The cell where `needle` starts on screen, matched one cell per char.
fn cell_of(screen: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    let area = screen.area;
    let chars: Vec<char> = needle.chars().collect();
    (area.top()..area.bottom()).find_map(|y| {
        (area.left()..area.right())
            .find(|&x| {
                chars.iter().enumerate().all(|(i, &c)| {
                    let cx = usize::from(x) + i;
                    cx < usize::from(area.right())
                        && screen[(cx as u16, y)]
                            .symbol()
                            .chars()
                            .eq(std::iter::once(c))
                })
            })
            .map(|x| (x, y))
    })
}

/// Open the settings popup on the focused profile, as `o` does.
fn open_settings(session: &mut TuiSession, app: &mut TuiApp) {
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(session, app, open);
    session.pump(app);
    assert!(app.settings_state.open, "settings opens");
}

/// `Saved <name>.` shows in the settings popup, in green under its rows,
/// only once the persist is durable.
#[test]
fn settings_popup_shows_saved_once_the_write_is_durable() {
    let iso = IsolatedEnv::enter("tui-settings-saved");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(cell_of(&screen, "settings — alice").is_some(), "{w}x{h}");
        assert!(
            cell_of(&screen, "Saved alice.").is_none(),
            "{w}x{h}: no Saved while the write is queued"
        );
    }

    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        let (left, top) = cell_of(&screen, "settings — alice").expect("the popup is drawn");
        let (x, y) = cell_of(&screen, "Saved alice.")
            .unwrap_or_else(|| panic!("{w}x{h}: Saved shows once the write is durable"));
        assert!(
            x == left && y > top,
            "{w}x{h}: inside the popup, under its rows"
        );
        assert_eq!(screen[(x, y)].fg, ratatui::style::Color::Green, "{w}x{h}");
    }
    let vault = session.core.vault().unwrap();
    assert!(!vault.get("alice").unwrap().settings.random_events);
    assert!(vault.get("bob").unwrap().settings.random_events);
}

/// Closing the settings popup before a persist completes: the popup opened
/// next, here on another profile, never shows that persist's `Saved`.
#[test]
fn a_persist_completing_after_close_shows_nothing_in_the_next_popup() {
    let iso = IsolatedEnv::enter("tui-settings-close-pending");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    session.pump(&mut app);
    assert!(!app.settings_state.open, "Esc closes the popup");
    focus_member(&mut session, &mut app, "bob");
    open_settings(&mut session, &mut app);

    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(cell_of(&screen, "settings — bob").is_some(), "{w}x{h}");
        assert!(
            cell_of(&screen, "Saved alice.").is_none(),
            "{w}x{h}: alice's persist never shows in bob's popup"
        );
    }
    let vault = session.core.vault().unwrap();
    assert!(
        !vault.get("alice").unwrap().settings.random_events,
        "the persist itself landed"
    );
}

/// A persist that fails after it was accepted shows in the popup it came
/// from, in red under the rows with "nothing was saved." under it, and on the
/// message line; the popup goes back to what is saved.
#[test]
#[cfg(unix)]
fn a_late_persist_failure_shows_in_the_popup_it_came_from() {
    use std::os::unix::fs::PermissionsExt;

    let iso = IsolatedEnv::enter("tui-settings-late-failure");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    assert!(!app.settings.random_events, "the popup shows the edit");

    std::fs::set_permissions(&iso.dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    std::fs::set_permissions(&iso.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        let (left, top) = cell_of(&screen, "settings — alice").expect("the popup is drawn");
        let (x, y) = cell_of(&screen, "random: ")
            .unwrap_or_else(|| panic!("{w}x{h}: the failure shows in the popup"));
        assert!(
            x == left && y > top,
            "{w}x{h}: inside the popup, under its rows"
        );
        assert_eq!(screen[(x, y)].fg, ratatui::style::Color::Red, "{w}x{h}");
        let (nx, ny) = cell_of(&screen, frontend_core::NOTHING_SAVED)
            .unwrap_or_else(|| panic!("{w}x{h}: nothing-was-saved shows"));
        assert!(nx == left && ny > y, "{w}x{h}: under the reason");
        assert!(
            cell_of(&screen, "msg: random:").is_some(),
            "{w}x{h}: and the message line reports it too"
        );
        assert!(
            cell_of(&screen, "random events: true").is_some(),
            "{w}x{h}: the popup is back on what is saved"
        );
    }
    assert!(
        session
            .core
            .vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .random_events,
        "nothing was saved"
    );

    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(
            cell_of(&screen, frontend_core::NOTHING_SAVED).is_none(),
            "{w}x{h}: the next edit clears the failure"
        );
    }
}

/// A persist that fails after its popup was closed never shows in the popup
/// opened next, here on another profile: the message line reports it.
#[test]
#[cfg(unix)]
fn a_persist_failing_after_close_reaches_only_the_message_line() {
    use std::os::unix::fs::PermissionsExt;

    let iso = IsolatedEnv::enter("tui-settings-late-closed");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    session.pump(&mut app);
    focus_member(&mut session, &mut app, "bob");
    open_settings(&mut session, &mut app);

    std::fs::set_permissions(&iso.dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    std::fs::set_permissions(&iso.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(cell_of(&screen, "settings — bob").is_some(), "{w}x{h}");
        assert!(
            cell_of(&screen, "msg: random:").is_some(),
            "{w}x{h}: the message line reports the failure"
        );
        assert!(
            cell_of(&screen, frontend_core::NOTHING_SAVED).is_none(),
            "{w}x{h}: alice's failure never shows in bob's popup"
        );
    }
}

/// A refused persist (the bound profile is gone) shows in the popup, in red
/// under its rows with "nothing was saved." under it, and on the message
/// line; editing the popup again clears it.
#[test]
fn a_refused_persist_shows_in_the_popup_until_the_next_edit() {
    let iso = IsolatedEnv::enter("tui-settings-refused");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    session.core.vault_mut().unwrap().remove("alice").unwrap();
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        let (left, top) = cell_of(&screen, "settings — alice").expect("the popup is drawn");
        let (x, y) = cell_of(&screen, "settings: random: no profile alice")
            .unwrap_or_else(|| panic!("{w}x{h}: the refusal shows"));
        assert!(x == left && y > top, "{w}x{h}: inside the popup");
        assert_eq!(screen[(x, y)].fg, ratatui::style::Color::Red, "{w}x{h}");
        assert_eq!(
            cell_of(&screen, frontend_core::NOTHING_SAVED),
            Some((left, y + 1)),
            "{w}x{h}: nothing-was-saved right under the reason"
        );
        assert!(
            cell_of(&screen, "msg: settings: random: no profile alice").is_some(),
            "{w}x{h}: and on the message line"
        );
    }

    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(
            cell_of(&screen, frontend_core::NOTHING_SAVED).is_none(),
            "{w}x{h}: editing the popup clears the refusal"
        );
    }
    let vault = session.core.vault().unwrap();
    assert!(vault.get("alice").is_none(), "nothing recreated");
    assert!(
        vault.get("bob").unwrap().settings.random_events,
        "bob untouched"
    );
}

fn press(session: &mut TuiSession, app: &mut TuiApp, code: crossterm::event::KeyCode) {
    let key = crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
    let action = session.params_key(app, key);
    dispatch(session, app, action);
}

/// Start all, Reload (warn, then Confirm) and Stop all from the script
/// pane go through the same coordinator as the panel: assigned members
/// start, the warning names both runs, Confirm replaces both, Stop all
/// stops both.
#[test]
fn tui_start_all_reload_and_stop_all_use_the_shared_coordinator() {
    let iso = IsolatedEnv::enter("tui-bulk-scripts");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let path = iso.dir.join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.js.load(&path).unwrap();
    assign(&mut session, "alice", &card);
    assign(&mut session, "bob", &card);

    dispatch(&mut session, &mut app, AppAction::ScriptStartAll);
    let click = app.error.clone().unwrap_or_default();
    assert_eq!(click, "Start all: started 1, queued 1, skipped 0");
    settle_starts(&mut session, &mut app);
    let generation = |session: &TuiSession, name: &str| {
        session.core.play().unwrap().script_runtime_generation(name)
    };
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play().unwrap(),
            name,
            script::RunState::Running,
        );
    }
    let before = (generation(&session, "alice"), generation(&session, "bob"));

    focus_member(&mut session, &mut app, "alice");
    assert_eq!(
        app.script_sel,
        frontend_core::scripts::sel_from_assignment(&card.assignment()),
        "the heading is alice's assignment"
    );
    std::fs::write(&path, format!("{LOOPING_TS}// changed\n")).unwrap();
    dispatch(&mut session, &mut app, AppAction::ScriptReload);
    let deadline = Instant::now() + Duration::from_secs(10);
    let outcome = loop {
        session.pump(&mut app);
        if let Some(outcome) = session.scripts.take_reload_outcome() {
            break outcome;
        }
        assert!(
            Instant::now() < deadline,
            "reload validation did not settle"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(outcome, frontend_core::scripts::ReloadOutcome::NeedsConfirm);
    assert!(app.reload_confirm, "Reload reads Confirm");
    let warning = app.error.clone().unwrap_or_default();
    assert!(
        warning.contains("will replace running bots") && warning.contains("alice, bob"),
        "{warning}"
    );
    assert_eq!(
        (generation(&session, "alice"), generation(&session, "bob")),
        before,
        "nothing is replaced before Confirm"
    );

    dispatch(&mut session, &mut app, AppAction::ScriptReload);
    assert!(matches!(
        session.scripts.take_reload_outcome(),
        Some(frontend_core::scripts::ReloadOutcome::Applied {
            restarted: 2,
            failed: 0,
            ..
        })
    ));
    settle_starts(&mut session, &mut app);
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play().unwrap(),
            name,
            script::RunState::Running,
        );
    }
    assert_ne!(generation(&session, "alice"), before.0);
    assert_ne!(generation(&session, "bob"), before.1);
    assert!(!app.reload_confirm);

    dispatch(&mut session, &mut app, AppAction::ScriptStopAll);
    for name in ["alice", "bob"] {
        wait_script_state(session.core.play().unwrap(), name, script::RunState::Idle);
    }
}

/// The TUI params popup edits the focused profile's own bag (not the
/// global store) and `a`/`y` apply it to the same-card member only.
#[test]
fn tui_params_edit_the_focused_profile_and_apply_to_all_skips_other_cards() {
    use crossterm::event::KeyCode;
    let iso = IsolatedEnv::enter("tui-apply-to-all");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob", "carol"]);
    let thiever_path = iso.dir.join("thiever.ts");
    std::fs::write(&thiever_path, THIEVER_TS).unwrap();
    let miner_path = iso.dir.join("miner.ts");
    std::fs::write(&miner_path, LOOPING_TS).unwrap();
    let thiever = session.scripts.js.load(&thiever_path).unwrap();
    let miner = session.scripts.js.load(&miner_path).unwrap();
    assign(&mut session, "alice", &thiever);
    assign(&mut session, "bob", &thiever);
    assign(&mut session, "carol", &miner);
    session
        .scripts
        .start_profile(&mut session.core, "bob", None)
        .unwrap();
    settle_starts(&mut session, &mut app);
    wait_script_state(
        session.core.play().unwrap(),
        "bob",
        script::RunState::Running,
    );

    focus_member(&mut session, &mut app, "alice");
    dispatch(&mut session, &mut app, AppAction::ScriptParams);
    assert!(app.params_state.open, "alice's thiever parameters open");
    press(&mut session, &mut app, KeyCode::Enter);
    while !app.params_state.scratch.is_empty() {
        press(&mut session, &mut app, KeyCode::Backspace);
    }
    for c in "Guard".chars() {
        press(&mut session, &mut app, KeyCode::Char(c));
    }
    press(&mut session, &mut app, KeyCode::Enter);
    session.core.flush_writes();
    press(&mut session, &mut app, KeyCode::Char('a'));
    press(&mut session, &mut app, KeyCode::Char('y'));
    session.core.flush_writes();
    session.pump(&mut app);

    let key = thiever.identity_key();
    let target = |session: &TuiSession, name: &str| {
        session
            .core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .and_then(|bag| bag.get("target").cloned())
    };
    let guard = Some(serde_json::json!("Guard"));
    assert_eq!(target(&session, "alice"), guard);
    assert_eq!(target(&session, "bob"), guard);
    assert_eq!(target(&session, "carol"), None, "other card untouched");
    assert!(
        session
            .scripts
            .legacy
            .overrides(script::ScriptSource::File, &thiever.name)
            .is_empty(),
        "the global store is not written"
    );
    let report = session.scripts.last_settings_sync().unwrap();
    assert_eq!(
        (report.saved, report.skipped.len(), report.delivered),
        (1, 1, 1)
    );
    assert_eq!(app.error.as_deref(), Some(report.summary()));
    session.core.play().unwrap().script_stop("bob");
}

/// Start with a prior load failure lists it on the strip; a newer front-end
/// error replaces it; the Start settling Ready leaves that error shown.
#[test]
fn tui_ready_start_keeps_a_newer_error_on_the_strip() {
    let iso = IsolatedEnv::enter("tui-ready-newer-error");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    std::fs::write(iso.dir.join("gate.ts"), "export const fail = true;").unwrap();
    let path = iso.dir.join("retry.ts");
    std::fs::write(
        &path,
        "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('tui-first-load');\nexport function tick(api) {}\n",
    )
    .unwrap();
    let card = session.scripts.js.load(&path).unwrap();
    let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
    focus_member(&mut session, &mut app, "alice");
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    std::fs::write(iso.dir.join("gate.ts"), "export const fail = false;").unwrap();

    session.script_start(&mut app, &sel);
    assert!(
        app.error
            .as_deref()
            .unwrap_or("")
            .contains("tui-first-load"),
        "{:?}",
        app.error
    );
    app.error = Some("map: no route".into());
    settle_starts(&mut session, &mut app);
    assert!(session
        .scripts
        .js
        .load_failure(&card.identity_key())
        .is_none());
    assert_eq!(app.error.as_deref(), Some("map: no route"));
    session.core.play().unwrap().script_stop("alice");
}

#[test]
fn tui_start_all_paces_admission_through_the_shared_coordinator() {
    let iso = IsolatedEnv::enter("tui-start-pace");
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    let path = iso.dir.join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.js.load(&path).unwrap();
    for name in names {
        assign(&mut session, name, &card);
    }

    dispatch(&mut session, &mut app, AppAction::ScriptStartAll);
    let starting = {
        let play = session.core.play().unwrap();
        names
            .iter()
            .filter(|name| play.script_state(name) == script::RunState::Starting)
            .count()
    };
    assert!(
        starting <= frontend_core::scripts::START_ADMIT_PER_FRAME,
        "TUI Start all dispatched {starting} in the click frame"
    );
    assert!(starting > 0);
    settle_starts(&mut session, &mut app);
    for name in names {
        wait_script_state(
            session.core.play().unwrap(),
            name,
            script::RunState::Running,
        );
        session.core.play().unwrap().script_stop(name);
    }
}

fn screen_text(app: &mut TuiApp, w: u16, h: u16) -> String {
    let buffer = screen(app, w, h);
    let area = buffer.area;
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The command palette's *Apply focused bot's settings to marked* copies the
/// focused bot's parameters to the marked same-card bots only: the frozen
/// scope is confirmed first (Esc drops it), an unmarked same-card bot keeps
/// its own, and the report names the marked bot that could not take it.
#[test]
fn palette_apply_settings_to_marked_copies_only_to_marked_same_card_bots() {
    let iso = IsolatedEnv::enter("tui-apply-marked");
    let (mut session, mut app) =
        tui_with_profiles(&iso, &["alice", "bob", "carol", "dave", "erin"]);
    let thiever = iso.dir.join("thiever.ts");
    std::fs::write(&thiever, THIEVER_TS).unwrap();
    let card = session.scripts.js.load(&thiever).unwrap();
    let looping = iso.dir.join("looping.ts");
    std::fs::write(&looping, LOOPING_TS).unwrap();
    let other = session.scripts.js.load(&looping).unwrap();
    for name in ["alice", "bob", "carol", "dave"] {
        assign(&mut session, name, &card);
    }
    assign(&mut session, "erin", &other);
    session
        .scripts
        .set_profile_setting(
            &mut session.core,
            "alice",
            card.source,
            &card.name,
            &card.path,
            "target",
            serde_json::json!("Guard"),
        )
        .unwrap();
    session.core.flush_writes();
    session.pump(&mut app);

    focus_member(&mut session, &mut app, "alice");
    app.script_sel = Some(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let command = crate::commands::Command::ScriptApplyMarked;
    assert_eq!(
        command.availability(&app),
        Err("mark fleet rows first (Space)")
    );
    // bob and carol take it; erin is marked but on another card; dave is not
    // marked.
    for name in ["bob", "carol", "erin"] {
        let id = session.core.profile_identity(name).unwrap();
        app.table.selection.set(id, true);
    }
    assert_eq!(command.availability(&app), Ok(()));

    // The palette lists the command over the marked scope, at both sizes.
    for (w, h) in SIZES {
        app.modal = None;
        let open = app.run_command(crate::commands::Command::Palette);
        dispatch(&mut session, &mut app, open);
        for c in "apply focused".chars() {
            app.on_key(settings_key(crossterm::event::KeyCode::Char(c)));
        }
        let shown = screen_text(&mut app, w, h);
        assert!(
            shown.contains("Apply focused bot's settings to marked"),
            "{w}x{h}: {shown}"
        );
    }

    // Running it freezes the scope and asks first; Esc drops it unwritten.
    let vault_file = iso.dir.join("vault");
    let rows_before = script_settings_rows(&session);
    let bytes_before = std::fs::read(&vault_file).unwrap();
    app.modal = None;
    let action = app.run_command(command);
    dispatch(&mut session, &mut app, action);
    let prompt = session.scripts.prepared_settings_sync().unwrap().prompt();
    assert!(
        prompt.contains("to 2 marked same-card bot(s)")
            && prompt.contains("erin (assigned another card)")
            && prompt.contains("1 unmarked bot(s) left unchanged"),
        "{prompt}"
    );
    let shown = screen_text(&mut app, 80, 24);
    assert!(
        shown.contains("marked same-card") && shown.contains("Apply y"),
        "{shown}"
    );
    let cancel = app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    dispatch(&mut session, &mut app, cancel);
    assert!(session.scripts.prepared_settings_sync().is_none());
    session.core.flush_writes();
    assert_eq!(
        script_settings_rows(&session),
        rows_before,
        "cancel changed a profile, the focused source included"
    );
    assert_eq!(
        std::fs::read(&vault_file).unwrap(),
        bytes_before,
        "cancel wrote the vault"
    );
    let key = card.identity_key();
    let saved = |session: &TuiSession, name: &str| {
        session
            .core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .cloned()
    };
    assert_eq!(saved(&session, "bob"), None, "cancel writes nothing");

    // Confirming applies it to the marked same-card bots only.
    let action = app.run_command(command);
    dispatch(&mut session, &mut app, action);
    let apply = app.on_key(settings_key(crossterm::event::KeyCode::Char('y')));
    dispatch(&mut session, &mut app, apply);
    session.core.flush_writes();
    session.pump(&mut app);
    let want: serde_json::Map<String, serde_json::Value> =
        std::iter::once(("target".to_string(), serde_json::json!("Guard"))).collect();
    assert_eq!(saved(&session, "bob"), Some(want.clone()));
    assert_eq!(saved(&session, "carol"), Some(want));
    assert_eq!(
        saved(&session, "dave"),
        None,
        "an unmarked bot is unchanged"
    );
    assert_eq!(saved(&session, "erin"), None, "another card is unchanged");
    let report = session.scripts.last_settings_sync().unwrap().summary();
    assert!(
        report.starts_with("Apply to marked ")
            && report.contains("saved 2")
            && report.contains("erin: skipped, assigned another card")
            && report.contains("1 unmarked unchanged"),
        "{report}"
    );
}

/// Opening *Apply focused bot's settings to marked* for a card newly assigned
/// to the focused bot (no per-card key in its profile yet, only a legacy
/// global override) and cancelling leaves the vault byte-identical, as the
/// dialog promises, while the frozen scope still carries the migrated bag.
#[test]
fn palette_apply_settings_to_marked_prepare_and_cancel_leave_a_fresh_source_unwritten() {
    let iso = IsolatedEnv::enter("tui-apply-marked-fresh");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let thiever = iso.dir.join("thiever.ts");
    std::fs::write(&thiever, THIEVER_TS).unwrap();
    let card = session.scripts.js.load(&thiever).unwrap();
    for name in ["alice", "bob"] {
        assign(&mut session, name, &card);
    }
    session
        .scripts
        .legacy
        .set_str(card.source, &card.name, "target", "Guard");
    focus_member(&mut session, &mut app, "alice");
    app.script_sel = Some(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let id = session.core.profile_identity("bob").unwrap();
    app.table.selection.set(id, true);

    let vault_file = iso.dir.join("vault");
    let rows_before = script_settings_rows(&session);
    assert!(
        rows_before
            .iter()
            .all(|(_, settings)| !settings.contains_key(&card.identity_key())),
        "a newly assigned card has no per-card key"
    );
    let bytes_before = std::fs::read(&vault_file).unwrap();

    let action = app.run_command(crate::commands::Command::ScriptApplyMarked);
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    session.pump(&mut app);
    let scope = session.scripts.prepared_settings_sync().unwrap();
    assert_eq!(
        scope.overrides.get("target"),
        Some(&serde_json::json!("Guard"))
    );
    assert_eq!(
        std::fs::read(&vault_file).unwrap(),
        bytes_before,
        "opening the confirmation wrote the vault"
    );

    let cancel = app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    dispatch(&mut session, &mut app, cancel);
    session.core.flush_writes();
    session.pump(&mut app);
    assert_eq!(script_settings_rows(&session), rows_before);
    assert_eq!(std::fs::read(&vault_file).unwrap(), bytes_before);
}

type SettingsRows = Vec<(
    String,
    std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>>,
)>;

/// Every profile's per-card settings map, by name.
fn script_settings_rows(session: &TuiSession) -> SettingsRows {
    session
        .core
        .vault()
        .unwrap()
        .profiles()
        .map(|row| (row.username.clone(), row.settings.script_settings.clone()))
        .collect()
}

#[test]
fn memory_relog_warns_before_cancelling_a_queued_start() {
    let iso = IsolatedEnv::enter("tui-relog-queued-start");
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    let path = iso.dir.join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.js.load(&path).unwrap();
    for name in names {
        assign(&mut session, name, &card);
    }
    dispatch(&mut session, &mut app, AppAction::ScriptStartAll);
    let queued = names
        .iter()
        .find(|name| session.scripts.start_queue_place(name).is_some())
        .expect("more Starts than one frame admits")
        .to_string();
    assert_eq!(
        session.core.play().unwrap().script_state(&queued),
        script::RunState::Idle,
        "the warning subject is queued, not already running"
    );

    dispatch(
        &mut session,
        &mut app,
        AppAction::MemoryRelog(queued.clone()),
    );

    assert!(
        matches!(
            &app.modal,
            Some(crate::overlay::Modal::Confirm(confirm))
                if confirm.kind == crate::overlay::ConfirmKind::MemoryRelog(queued.clone())
        ),
        "a queued Start must be disclosed before Relog-now cancels it"
    );
    assert!(
        !session
            .core
            .play()
            .unwrap()
            .arm(&queued)
            .unwrap()
            .login_latched(),
        "nothing relogs before confirmation"
    );
    session.scripts.stop_all(&mut session.core);
}

#[test]
fn settings_memory_row_persists_and_arms_the_handshake() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let alice = SlotArm::new(7, true);
    session
        .core
        .start(lifecycle_vault("mem-persist"), empty_play());
    session
        .core
        .play_mut()
        .unwrap()
        .attach_arm("alice", Arc::clone(&alice));
    session.core.fleet_mut().add("alice");
    session.core.select("alice");
    let mut app = TuiApp::new("tui");
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    // Flip the memory row draft; the binary persists it on the next pump.
    app.settings.lowmem = false;
    app.settings_dirty = true;
    session.pump(&mut app);

    assert_eq!(
        alice.lowmem_handshake(),
        Some(false),
        "the toggle arms the next handshake at once"
    );
    session.core.flush_writes();
    assert!(
        !session
            .core
            .vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .lowmem,
        "the setting is durable"
    );
}

#[test]
fn memory_relog_confirms_while_a_script_runs() {
    let mut play = run_with_io(&dummy_options(), vec![], |_| (None, None), |_, _, _| {});
    let alice = SlotArm::new(7, false);
    play.attach_arm("alice", Arc::clone(&alice));
    play.script_start_load(
        "alice",
        "export function tick(api) { api._n = (api._n||0)+1 }".to_string(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    let mut app = TuiApp::new("tui");
    app.names = vec!["alice".into()];
    app.focused = Some(0);

    dispatch(
        &mut session,
        &mut app,
        AppAction::MemoryRelog("alice".into()),
    );
    assert!(
        matches!(
            &app.modal,
            Some(crate::overlay::Modal::Confirm(c))
                if matches!(
                    c.kind,
                    crate::overlay::ConfirmKind::MemoryRelog(_)
                )
        ),
        "a running script forces the confirm, nothing relogs yet"
    );
    assert!(!alice.login_latched());

    dispatch(
        &mut session,
        &mut app,
        AppAction::MemoryRelogNow("alice".into()),
    );
    assert!(alice.login_latched(), "the confirmed relog logs out");
    assert!(alice.wants_logout());
}

use super::*;
use crate::bin::seam::Seam;

use frontend_core::scripts::{CardSource, CompiledCardId, Loadouts, ScriptChoice, ScriptRunState};
#[cfg(feature = "js")]
use frontend_core::scripts::{LoadShape, LoadStage};
use frontend_core::test_support::live::{
    passing_runner_for_test, runner_status_for_test, tick_scenario_runner_for_test,
};
#[cfg(feature = "js")]
use frontend_core::test_support::paint::TestPaint;
use frontend_core::test_support::scripts::{assign_compiled, reset_quester_registry};
#[cfg(feature = "js")]
use frontend_core::test_support::scripts::{
    assign_loaded, file_identity, live_example_path, raw_content_fingerprint, LoadedCardSnapshot,
};
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::tui::{
    arm_random_event_policy, play_script_state, quester_draft_path_ids, quester_folder_path_ids,
    remove_saved_profile,
};
#[cfg(feature = "js")]
use frontend_core::test_support::tui::{
    card_api_family, saved_script_bag, saved_script_settings, start_script_source,
};
use frontend_core::test_support::{FixtureProfile, IsolatedStores};
use host_play::profile::ProfileEnvironment;
use host_play::{Play, SlotArm, WalkArm};
use nav_planner::router::FindOptions;
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play as map_host;
#[path = "../../host-play/tests/support/map_fixture.rs"]
mod map_fixture;
#[path = "../../host-play/tests/support/upgrade_home.rs"]
mod upgrade_home;

/// Select `name` through the command seam, as the fleet's focus change does.
fn select(session: &mut TuiSession, name: &str) {
    session
        .submit(OperatorCommand::Select, Targets::Profile(name))
        .expect("select");
}

fn wait_script_state(play: &host_play::Play, name: &str, want: ScriptRunState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while play_script_state(play, name) != want && Instant::now() < deadline {
        play.pump_script_lifecycle(name);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(play_script_state(play, name), want);
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

fn publish_map_walk_snapshot(
    core: &mut frontend_core::OperatorSession<()>,
    name: &str,
    tile: Tile,
) {
    let world_tile = api::WorldTile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    };
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tile(world_tile);
    snapshot.seed_local_player(api::snapshot::LocalPlayerView {
        player: api::snapshot::PlayerView {
            index: 0,
            actor: api::snapshot::ActorView {
                name: Some(name.to_owned()),
                actions: Vec::new(),
                tile: world_tile,
                distance: 0,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 0,
                total_health: 0,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
            },
            network: world_tile,
            combat_level: 0,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    });
    snapshot.seed_equipment(vec![api::snapshot::ItemView {
        def: api::ItemDefView {
            id: 2,
            name: Some("knife".into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: api::snapshot::ItemContainer::Equipment,
        action_family: api::snapshot::ItemActionFamily::Component,
        slot: 0,
        count: 1,
        actions: Vec::new(),
        component_id: -1,
    }]);
    let state =
        nav_planner::WorldState::from_snapshot(&snapshot).with_map_members(core.map_members());
    core.publish_test_snapshot(name, snapshot, state);
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

    fn send_packet(&mut self, packet: &api::prot::Send) -> Result<(), api::prot::PacketError> {
        packet.write_for_revision(self.revision(), &mut self.out)
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

#[test]
fn mainland_production_gate_queues_once_without_rearming_host_or_reconnect() {
    let mut session = TuiSession::new(dummy_options());
    session.mainland = true;
    let enabled = session.mainland;
    assert!(enabled, "TUI must retain the opt-in");
    assert!(
        !session.launcher.play_config().mainland(),
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

/// The map pane reads `TuiApp::world`, projected from the core after `Play`
/// loads its pack.
#[test]
fn pump_copies_nav_world_onto_the_app() {
    let home = upgrade_home::UpgradeHome::new();
    let profile = home.bind(None);
    let template = SharedClientTemplate::load(profile).unwrap();
    let mut session = TuiSession::new_bound(template);
    let vault = VaultFixture::create(&home.root.join("map-world.vault"), "test-passphrase-01");
    session.start_play(vault).unwrap();
    let mut app = TuiApp::new("274bot headless");
    assert!(app.world.is_none(), "fresh app has no pack");
    session.pump(&mut app);
    assert!(
        app.world.is_some(),
        "pump copies the core's nav world onto the map"
    );
}

#[test]
fn live_pass_is_stdout_and_exit_0() {
    let (code, lines, announced) = live_proof(
        "alcher",
        Some(frontend_core::live::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        None,
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
        Some(frontend_core::live::RunnerStatus::Failed),
        "{\"outcome\":\"FAIL\"}",
        Some("deadline"),
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
        Some(frontend_core::live::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        None,
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
        Some(frontend_core::live::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        None,
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
        Some(frontend_core::live::RunnerStatus::Passed),
        "{\"outcome\":\"PASS\"}",
        None,
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
        Some(frontend_core::live::RunnerStatus::Running { step: 1, total: 2 }),
        "",
        None,
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
    assert!(args.profile.as_host().rs2b2t);
    assert!(args.live.is_none());
    let home = std::env::temp_dir().join(format!("274bot-tui-public-{}", std::process::id()));
    let selection = args
        .profile
        .as_host()
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
    let world_three = selection.public_worlds().unwrap().by_number(3).unwrap();
    assert_eq!(
        (
            world_three.host.as_str(),
            world_three.port,
            world_three.node_id
        ),
        ("w3.rs2b2t.com", 443, 12)
    );
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
    assert_eq!(args.profile.as_host().profile.as_deref(), Some("local-289"));
    assert_eq!(args.profile.as_host().revision.as_deref(), Some("289"));
    assert_eq!(args.profile.as_host().port, Some(44595));
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

/// Install a one-step scenario the synthetic client has passed, so the
/// runner reports Passed before any shared witness has qualified.
fn install_passed_runner(handle: &frontend_core::live::RunnerHandle) {
    let mut runner = passing_runner_for_test("core_hold", None, None);
    let mut c = client::client::Client::new(client::client::ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    });
    c.ingame = true;
    c.scene_state = 2;
    tick_scenario_runner_for_test(&mut runner, &mut c);
    c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
    tick_scenario_runner_for_test(&mut runner, &mut c);
    handle.install(runner);
    assert_eq!(
        runner_status_for_test(handle),
        Some(frontend_core::live::RunnerStatus::Passed)
    );
}

/// What `--catalog-core` prepare leaves: the Play's catalog witness armed for
/// the driven account and held by the run.
fn arm_catalog_witness(session: &mut TuiSession, play: &Play) {
    let catalog = play.catalog_core_watch();
    catalog.configure(host_play::catalog_core::CoreCase::Thiever, "alice");
    session
        .core
        .live_runner()
        .install_catalog_watch_for_test(Some(catalog));
}

#[test]
fn catalog_core_holds_live_pass_until_the_witness_qualifies_or_its_deadline_fails() {
    let play = empty_play();
    let mut session = TuiSession::new(dummy_options());
    arm_catalog_witness(&mut session, &play);
    session.inject_play(play);
    session.live_name = Some("core_hold".into());
    install_passed_runner(session.core.live_runner());
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
fn pair_fixture_setup_deadline_is_named_and_reports_split_elapsed_time() {
    let mut session = TuiSession::new(dummy_options());
    session.live_name = Some("flax_runner".into());
    let started = Instant::now() - Duration::from_secs(121);
    let deadline = started + Duration::from_secs(180);
    let runner = session.core.live_runner();
    runner
        .resolve_core("flax_runner", 2, false, true)
        .expect("resolve Flax pair gate");
    let pair = host_play::paired_core::PairWatch::default();
    pair.configure(host_play::paired_core::PairCase::Flax, "runner", "spinner");
    runner.install_pair_watch_for_test(Some(pair));
    runner.configure_pair_setup_timing(started, Some(deadline));

    let (code, lines) = session.live_status();
    assert_eq!(code, Some(1));
    assert!(
        lines.iter().any(
            |line| matches!(line, ProofLine::Stderr(text) if text.contains("FAIL: live flax_runner setup:"))
        ),
        "{lines:?}"
    );
    let timing = lines
        .iter()
        .find_map(|line| match line {
            ProofLine::Stderr(text) => text
                .strip_prefix("LIVE_TIMING: script_flax_runner ")
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok()),
            _ => None,
        })
        .expect("setup timeout must include its timing receipt");
    assert!(
        timing["setup_elapsed_ms"]
            .as_u64()
            .is_some_and(|ms| ms >= 120_000),
        "{timing}"
    );
    assert_eq!(timing["script_elapsed_ms"], serde_json::Value::Null);
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
    install_passed_runner(session.core.live_runner());
    let (code, lines) = session.live_status();
    assert_eq!(code, Some(0), "{lines:?}");
    assert!(
        lines.iter().all(|line| match line {
            ProofLine::Stdout(text) | ProofLine::Stderr(text) => !text.contains("CORE"),
        }),
        "{lines:?}"
    );
}

#[cfg(feature = "js")]
#[test]
fn paired_proof_refuses_a_live_run_without_the_pair_gate() {
    let _iso = IsolatedStores::enter("tui-pair-refusal");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let error = session
        .live_prepare_script(
            frontend_core::live::Scenario::get("nature_crafter_air").expect("registered"),
        )
        .expect_err("a paired proof must not run outside the pair gate");
    assert!(error.contains("pair core gate"), "{error}");
    assert!(
        session.core.play_for_test().is_none(),
        "refused before any boot"
    );
    assert!(session
        .core
        .live_runner()
        .pending_starts_for_test()
        .is_empty());
}

#[cfg(feature = "js")]
#[test]
fn pair_gate_stashes_both_slots_with_complementary_role_bags() {
    let iso = IsolatedStores::enter("tui-pair-gate");
    let root = iso.dir().join("rs2b0t");
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
    iso.set_catalog_root(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session.live_pair_core = true;
    session
        .live_prepare_script(
            frontend_core::live::Scenario::get("nature_crafter_air").expect("registered"),
        )
        .expect("prepare");
    let pending = session.core.live_runner().pending_starts_for_test();
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
        .play_for_test()
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
        let selection = args.profile.as_host().resolve_with_env(None, &env).unwrap();
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

/// M-311: a fresh vault starts empty — interactive boot must not seed a
/// `test`/`test` profile. An empty vault is not an error: the boot
/// opens the New profile form on it. `--user` creates nothing: an
/// unknown name stops with a message that names how to add it, and
/// leaves the vault empty.
#[test]
fn fresh_vault_starts_empty_without_a_seeded_test_profile() {
    let iso = IsolatedStores::enter("tui-fresh-vault-empty");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .start_play(VaultFixture::create(
            &iso.dir().join("vault"),
            "test-passphrase-01",
        ))
        .unwrap();
    // No `--user`: no focus and no error (the boot opens the New
    // profile form), and no `test` profile left behind.
    assert_eq!(session.bootstrap_interactive_profiles(&[]).unwrap(), None);
    assert!(session.names.is_empty(), "names: {:?}", session.names);
    assert!(
        session.core.profile("test").is_none(),
        "fresh vault must not seed `test`"
    );
    // `--user` with an unknown name: a clear message, nothing created.
    let err = session
        .bootstrap_interactive_profiles(&["alice".to_string()])
        .unwrap_err();
    assert_eq!(
        err,
        "no profile named alice in the vault: --user only selects existing \
         profiles and creates none (start tui-play without --user and add \
         it with New profile: press N in the Fleet pane, then type its \
         username and password)"
    );
    assert!(session.names.is_empty(), "names: {:?}", session.names);
    assert!(
        session.core.profile("alice").is_none() && session.core.profile("test").is_none(),
        "--user must not create a profile"
    );
}

/// A missing vault rejects `--user` before it can request a passphrase or
/// create the vault file.
#[test]
fn user_on_missing_vault_fails_without_prompt_or_vault_write() {
    let iso = IsolatedStores::enter("tui-user-missing-vault");
    // `run` resolves the vault from its own arguments and the process home,
    // not from the thread-pinned stores, so name the vault path explicitly.
    let vault_path = iso.dir().join("vault-test");
    assert!(!vault_path.exists(), "the test starts without a vault");
    let nav_dir = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .and_then(std::path::Path::parent)
        .expect("target debug directory")
        .join("nav/289");
    // The local profile resolves with any engine directory; `--user` on a
    // missing vault is refused before binding reads the engine key or
    // prepares the cache, so the test needs neither the host's `ENGINE_DIR`
    // nor `LOGIN_RSAN/E`, nor a running engine.
    let args = parse_args_from([
        "--profile".to_string(),
        "local-289".to_string(),
        "--engine".to_string(),
        iso.dir().join("engine").to_string_lossy().into_owned(),
        "--nav-pack".to_string(),
        nav_dir
            .join("274bot.navpack")
            .to_string_lossy()
            .into_owned(),
        "--nav-flags".to_string(),
        nav_dir
            .join("274bot.navflags")
            .to_string_lossy()
            .into_owned(),
        "--vault".to_string(),
        vault_path.to_string_lossy().into_owned(),
        "--user".to_string(),
        "ghost".to_string(),
    ])
    .unwrap();

    let error = run(&args, RunMode::Interactive).unwrap_err();
    assert_eq!(
        error,
        "no profile named ghost in the vault: --user only selects existing \
         profiles and creates none (start tui-play without --user and add \
         it with New profile: press N in the Fleet pane, then type its \
         username and password)"
    );
    assert!(
        !vault_path.exists(),
        "a rejected user must not create a vault"
    );
}

/// `--user` selects an existing profile as the boot focus, and an
/// existing vault keeps its profiles: no seeding, no wipe, nothing
/// created for a later unknown name either.
#[test]
fn user_selects_an_existing_profile_and_creates_none() {
    let iso = IsolatedStores::enter("tui-user-selects-existing");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    assert_eq!(
        session
            .bootstrap_interactive_profiles(&["bob".to_string()])
            .unwrap(),
        Some("bob".to_string())
    );
    assert_eq!(session.names, vec!["alice".to_string(), "bob".to_string()]);
    // No `--user`: the default request is resolved after saved order is reconciled.
    assert_eq!(session.bootstrap_interactive_profiles(&[]).unwrap(), None);
    // An unknown name after valid ones still creates nothing.
    assert!(session
        .bootstrap_interactive_profiles(&["alice".to_string(), "nosuch".to_string()])
        .is_err());
    assert!(session.core.profile("nosuch").is_none());
}

#[test]
fn interactive_landing_restores_preferences_before_loading_existing_profile() {
    let iso = IsolatedStores::enter("tui-landing-preferences");
    let path = frontend_core::scripts::paths::bot_home()
        .join(".274bot")
        .join("panel-ui.json");
    crate::test_support::write_private_fixture(
        &path,
        r#"{"nav":{"allow_teleports":true,"pause_script_on_manual_walk_abort":false}}"#,
    )
    .unwrap();
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    session.start_play(vault).unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();

    assert!(app.modal.is_none());
    assert!(
        app.focused.is_none(),
        "default focus waits for shared preparation"
    );
    assert!(app.walk_permissions.globals().allow_teleports());
    assert!(!app.pause_script_on_manual_walk_abort);
    assert_eq!(session.startup_focus, None);
    assert!(session.startup_default_pending);
    assert!(!session
        .core
        .commands()
        .reports()
        .any(|report| report.command() == OperatorCommand::Load));

    session.pump(&mut app);
    assert!(session.startup_focus.is_none());
    assert!(session
        .core
        .commands()
        .reports()
        .any(|report| report.command() == OperatorCommand::Load));
}

#[test]
fn removing_the_focused_bot_cancels_startup_focus() {
    let iso = IsolatedStores::enter("tui-remove-startup-focus");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session.preferences.reload().unwrap();
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    session.start_play(vault).unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    session.load_and_login("alice");
    session.remove(&mut app, "alice");
    assert!(session.core.selected().is_none());
    session.pump(&mut app);
    assert!(
        session.core.selected().is_none(),
        "startup must not reload the removed focus"
    );
    assert!(app.focused_name().is_none());
    assert!(session.startup_focus.is_none());
}

#[test]
fn operator_focus_before_preference_adoption_keeps_walk_target_and_boot_load() {
    for path in [
        "direct-same",
        "direct",
        "keyboard",
        "fleet",
        "first-profile",
    ] {
        let iso = IsolatedStores::enter(&format!("tui-startup-focus-{path}"));
        let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
        let mut session = TuiSession::new(dummy_options());
        session.core.set_spawn_workers(false);
        let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release, release_rx) = std::sync::mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let first = std::sync::atomic::AtomicBool::new(true);
        let service =
            host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
                host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
                Arc::new(move || {
                    if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        let _ = entered.send(());
                        let _ = release_rx.lock().unwrap().recv();
                    }
                }),
            );
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        session.core.bind_local_state(service);
        session.preferences = session.core.preferences().unwrap();
        let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
        vault.put(FixtureProfile::new("alice", "pw").uid(7));
        vault.put(FixtureProfile::new("bob", "pw").uid(8));
        session.start_play(vault).unwrap();
        let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
        let mut app = interactive_landing(&mut session, &args).unwrap();
        assert!(session.startup_default_pending);
        assert!(session.startup_focus.is_none());
        // Selection itself never loads a slot in the TUI. Bob is preloaded
        // for the alternate-focus paths while the boot request waits.
        session.load_and_login("bob");
        let expected_focus = if path == "direct-same" {
            "alice"
        } else {
            "bob"
        };
        let action = match path {
            "direct-same" => app.select_bot("alice"),
            "direct" => app.select_bot("bob"),
            "keyboard" => {
                // The deferred default has no initial selection. Two explicit
                // next-bot keys choose Alice, then Bob.
                let first = app.run_command(crate::commands::Command::SelectNextBot);
                dispatch(&mut session, &mut app, first);
                app.run_command(crate::commands::Command::SelectNextBot)
            }
            "fleet" => {
                session.pump(&mut app);
                app.table.cursor = app
                    .fleet_model
                    .shown()
                    .iter()
                    .position(|index| app.fleet_model.rows()[*index].name() == "bob")
                    .unwrap();
                app.on_key(settings_key(crossterm::event::KeyCode::Enter))
            }
            "first-profile" => {
                // A durable New profile save uses this same deferred gate.
                session.queue_startup_focus("alice");
                app.select_bot("bob")
            }
            _ => unreachable!(),
        };
        assert!(
            matches!(&action, AppAction::Focus(name) if name == expected_focus),
            "{path}: {action:?}"
        );
        dispatch(&mut session, &mut app, action);
        assert!(!session.preferences.ready());
        release.send(()).unwrap();
        session.preferences.reload().unwrap();
        session.pump(&mut app);
        assert!(
            session.core.members().iter().any(|name| name == "alice"),
            "{path}: the boot profile must still load after an early selection"
        );
        assert!(
            session
                .core
                .play_for_test()
                .unwrap()
                .arm("alice")
                .unwrap()
                .wants_login(),
            "{path}: the boot profile must retain its login intent"
        );
        session.arm_walk_on(
            &mut app,
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        );
        let report = session
            .core
            .commands()
            .reports()
            .find(|report| report.command() == OperatorCommand::WalkTo)
            .expect("WalkTo command report");
        let focus = session.core.profile_identity_or_synthetic(expected_focus);
        assert!(
            report
                .member(frontend_core::command::Subject::Profile(focus))
                .is_some(),
            "{path}: the command must target the operator's selection: {report:?}"
        );
        assert_eq!(session.core.selected(), Some(expected_focus), "{path}");
        assert_eq!(
            app.focused_name().as_deref(),
            Some(expected_focus),
            "{path}"
        );
        assert!(session.startup_focus.is_none(), "{path}");
    }
}

#[test]
fn fleet_login_selection_keeps_startup_load_without_overriding_focus() {
    let iso = IsolatedStores::enter("tui-fleet-login-startup-focus");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    // The default target is resolved only once the blocked preferences read completes.
    session.startup_default_pending = true;
    let mut app = new_app("fleet login", &session.preferences);
    app.names = session.names.clone();
    session
        .submit(OperatorCommand::Load, Targets::Profile("bob"))
        .unwrap();
    session.pump(&mut app);
    assert!(!session.preferences.ready());
    assert!(session.core.selected().is_none());

    session.plan_fleet_action(&mut app, frontend_core::FleetAction::Login, None);
    assert!(session
        .core
        .fleet_plan()
        .is_some_and(|plan| plan.needs_confirm()));
    session.run_fleet_plan(&mut app, frontend_core::FleetAction::Login);
    assert_eq!(session.core.selected(), Some("bob"));
    assert!(session
        .core
        .play_for_test()
        .unwrap()
        .arm("bob")
        .unwrap()
        .wants_login());

    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);
    assert!(session.core.members().iter().any(|name| name == "alice"));
    assert!(session.core.members().iter().any(|name| name == "bob"));
    let play = session.core.play_for_test().unwrap();
    for name in ["alice", "bob"] {
        assert!(play.arm(name).unwrap().wants_login(), "{name}");
    }
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(app.focused_name().as_deref(), Some("bob"));
}

#[test]
fn removing_a_nonfocused_startup_target_cancels_its_deferred_load() {
    let iso = IsolatedStores::enter("tui-remove-nonfocused-startup-target");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    let args = parse_args_from(["--user", "alice"]).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    assert_eq!(
        session.startup_focus,
        session.core.profile_identity("alice")
    );
    session.load_and_login("bob");
    // A core-side focus path leaves the startup request pending.
    select(&mut session, "bob");
    session.pump(&mut app);
    session.load_and_login("alice");
    session.pump(&mut app);
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(app.focused_name().as_deref(), Some("bob"));
    assert!(session.core.members().iter().any(|name| name == "alice"));

    let load_count = session
        .core
        .commands()
        .reports()
        .filter(|report| report.command() == OperatorCommand::Load)
        .count();
    let login_count = session
        .core
        .commands()
        .reports()
        .filter(|report| report.command() == OperatorCommand::Login)
        .count();
    session.remove(&mut app, "alice");
    session.pump(&mut app);
    assert_eq!(session.core.selected(), Some("bob"));
    assert!(!session.core.members().iter().any(|name| name == "alice"));
    assert_eq!(app.focused_name().as_deref(), Some("bob"));

    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);
    assert!(!session.core.members().iter().any(|name| name == "alice"));
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(app.focused_name().as_deref(), Some("bob"));
    assert_eq!(
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == OperatorCommand::Load)
            .count(),
        load_count,
        "the removed pending target must not be loaded again"
    );
    assert_eq!(
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == OperatorCommand::Login)
            .count(),
        login_count,
        "the removed pending target must not be logged in again"
    );
}

/// Type `text` into the open New profile overlay, dispatching each
/// key as the event loop does.
fn type_new_profile(session: &mut TuiSession, app: &mut TuiApp, text: &str) {
    for c in text.chars() {
        let action = app.on_key(settings_key(crossterm::event::KeyCode::Char(c)));
        dispatch(session, app, action);
    }
}

#[test]
fn new_profile_form_refuses_empty_password_without_login() {
    let iso = IsolatedStores::enter("tui-empty-profile-password");
    let path = iso.dir().join("vault");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .start_play(VaultFixture::create(&path, "test-passphrase-01"))
        .unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();

    type_new_profile(&mut session, &mut app, "alice");
    let tab = app.on_key(settings_key(crossterm::event::KeyCode::Tab));
    dispatch(&mut session, &mut app, tab);
    let enter = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, enter);

    assert_eq!(session.profile_editor.target(), Some(""));
    let Some(Modal::NewProfile(view)) = app.modal.as_ref() else {
        panic!("the refused save keeps the New profile form open");
    };
    assert_eq!(
        view.error.as_deref(),
        Some("credentials: password required")
    );
    assert!(session.core.profile("alice").is_none());
    assert!(
        !session
            .core
            .commands()
            .reports()
            .any(|report| report.command() == frontend_core::command::OperatorCommand::Login),
        "a refused save must not attempt a login"
    );
    session.core.flush_writes();
    assert!(
        VaultFixture::unlock(&path, "test-passphrase-01")
            .usernames()
            .is_empty(),
        "the rejected profile must not be written"
    );
}

/// An empty vault does not exit: `interactive_landing`, which `run` also
/// calls, opens the New profile form. This test types and saves a profile
/// through the shared form. It also checks Esc and `N` at both UI sizes.
#[test]
fn new_profile_form_saves_the_typed_password() {
    let iso = IsolatedStores::enter("tui-new-profile-form");
    let path = iso.dir().join("vault");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .start_play(VaultFixture::create(&path, "test-passphrase-01"))
        .unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    assert_eq!(session.profile_editor.target(), Some(""));
    // Esc closes the form and creates nothing.
    type_new_profile(&mut session, &mut app, "ghost");
    let esc = app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    dispatch(&mut session, &mut app, esc);
    assert!(app.modal.is_none(), "Esc closes the form");
    assert!(session.core.profile("ghost").is_none());
    assert_eq!(session.profile_editor.target(), None);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(
            cell_of(&screen, "no profiles: press N to add one").is_some(),
            "{w}x{h}: the empty Fleet row must name `N`"
        );
    }
    // Reopen through the Fleet pane's command, as `N` and the palette do.
    let open = app.run_command(crate::commands::Command::NewProfile);
    assert_eq!(open, AppAction::OpenNewProfile);
    dispatch(&mut session, &mut app, open);
    assert_eq!(session.profile_editor.target(), Some(""));
    type_new_profile(&mut session, &mut app, "alice");
    let tab = app.on_key(settings_key(crossterm::event::KeyCode::Tab));
    dispatch(&mut session, &mut app, tab);
    type_new_profile(&mut session, &mut app, "s3cret-typed");
    assert_eq!(session.profile_editor.draft().username(), "alice");
    assert_eq!(
        session.profile_editor.draft().password_for_test(),
        "s3cret-typed"
    );
    // Both sizes: the full labels fit, the password is only a mask.
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        for label in [
            "New profile",
            "Username: alice_",
            "Password: ************_",
            "[Save Enter]",
            "[Close Esc]",
        ] {
            assert!(cell_of(&screen, label).is_some(), "{w}x{h}: {label}");
        }
        assert!(
            cell_of(&screen, "s3cret-typed").is_none(),
            "{w}x{h}: the password is never shown"
        );
    }
    // Enter on the password field saves the form.
    let enter = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, enter);
    session.core.flush_writes();
    session.pump(&mut app);
    assert!(app.modal.is_none(), "a saved profile closes the form");
    assert_eq!(app.error, None, "the save was accepted");
    assert!(session.core.profile("alice").is_some(), "alice appears");
    // The vault file holds the typed password, not a generated one.
    let stored = VaultFixture::unlock(&path, "test-passphrase-01");
    assert_eq!(stored.usernames(), vec!["alice".to_string()]);
    assert_eq!(
        stored.profile("alice").unwrap().draft().password_for_test(),
        "s3cret-typed"
    );
}

#[test]
fn first_profile_save_waits_for_preferences_before_loading_and_logging_in_once() {
    let iso = IsolatedStores::enter("tui-first-profile-cold-preferences");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    session
        .start_play(VaultFixture::create(
            &iso.dir().join("vault"),
            "test-passphrase-01",
        ))
        .unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    type_new_profile(&mut session, &mut app, "alice");
    let tab = app.on_key(settings_key(crossterm::event::KeyCode::Tab));
    dispatch(&mut session, &mut app, tab);
    type_new_profile(&mut session, &mut app, "pw");
    let enter = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, enter);
    session.core.flush_writes();

    for _ in 0..4 {
        session.pump(&mut app);
        assert!(!session.preferences.ready());
        assert!(session.core.profile("alice").is_some());
        assert!(app.modal.is_none(), "the durable save closes the form");
        assert!(
            session.core.slots().is_empty(),
            "saving the first profile must not load or log in before preferences are ready"
        );
        assert!(!session.core.commands().reports().any(|report| matches!(
            report.command(),
            OperatorCommand::Load | OperatorCommand::Login
        )));
    }

    assert_eq!(
        session.startup_focus,
        session.core.profile_identity("alice"),
        "the saved profile is queued by UID"
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::Focus("alice".to_string()),
    );
    assert!(!session.preferences.ready());
    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);
    assert!(
        session.core.members().iter().any(|name| name == "alice"),
        "the new profile still loads after an early explicit focus"
    );
    assert!(session.preferences.ready());
    for _ in 0..4 {
        session.pump(&mut app);
    }
    assert_eq!(session.core.selected(), Some("alice"));
    assert!(session
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    for command in [OperatorCommand::Load, OperatorCommand::Login] {
        assert_eq!(
            session
                .core
                .commands()
                .reports()
                .filter(|report| report.command() == command)
                .count(),
            1,
            "{command:?} must be submitted exactly once"
        );
    }
}

#[test]
fn o1_new_profile_saved_during_boot_wait_uses_late_saved_order() {
    let iso = IsolatedStores::enter("tui-o1-new-profile-during-boot-wait");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let state = host_play::local_state::StateRoot::new(iso.dir().join("cold-state"));
    crate::test_support::write_private_fixture(
        state.preferences_file(),
        br#"{"profile_order":{"7":131072,"8":65536}}"#,
    )
    .unwrap();
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        state,
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();

    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    assert!(
        !session.core.commands().reports().any(|report| matches!(
            report.command(),
            OperatorCommand::Load | OperatorCommand::Login
        )),
        "interactive landing must queue startup work without loading synchronously"
    );

    let open = app.run_command(crate::commands::Command::NewProfile);
    dispatch(&mut session, &mut app, open);
    type_new_profile(&mut session, &mut app, "charlie");
    let tab = app.on_key(settings_key(crossterm::event::KeyCode::Tab));
    dispatch(&mut session, &mut app, tab);
    type_new_profile(&mut session, &mut app, "pw");
    let enter = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, enter);
    session.core.flush_writes();
    session.pump(&mut app);

    assert!(!session.preferences.ready());
    assert!(session.core.profile("charlie").is_some());
    assert!(session.core.slots().is_empty());
    assert!(
        !session.core.commands().reports().any(|report| matches!(
            report.command(),
            OperatorCommand::Load | OperatorCommand::Login
        )),
        "neither queued profile may load before preferences are ready"
    );
    assert!(app.modal.is_none(), "the durable save closes the form");

    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);

    assert_eq!(
        session.core.profile_order().names(),
        ["bob", "alice", "charlie"],
        "late preference adoption reconciles saved UID order before startup loads"
    );
    assert_eq!(
        session.core.members(),
        ["bob", "charlie"],
        "the actual loaded rows retain the boot and New profile targets in saved order"
    );
    for name in ["bob", "charlie"] {
        assert!(
            session
                .core
                .play_for_test()
                .unwrap()
                .arm(name)
                .unwrap()
                .wants_login(),
            "{name} retains the startup login intent"
        );
        let identity = session.core.profile_identity(name).unwrap();
        for command in [OperatorCommand::Load, OperatorCommand::Login] {
            assert_eq!(
                session
                    .core
                    .commands()
                    .reports()
                    .filter(|report| {
                        report.command() == command
                            && report
                                .member(frontend_core::command::Subject::Profile(identity))
                                .is_some()
                    })
                    .count(),
                1,
                "{command:?} is submitted once for {name}"
            );
        }
    }
    assert_eq!(session.core.selected(), Some("charlie"));
    assert_eq!(app.focused_name().as_deref(), Some("charlie"));
}

#[test]
fn o1_renamed_explicit_boot_profile_keeps_its_uid_during_preference_wait() {
    let iso = IsolatedStores::enter("tui-o1-renamed-boot-profile");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();

    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    session.start_play(vault).unwrap();
    let boot_identity = session.core.profile_identity("alice").unwrap();
    let args = parse_args_from(["--user", "alice"]).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();

    session
        .submit_raw_editor(
            &mut app,
            OperatorCommand::OpenProfileForm,
            Targets::Profile("alice"),
        )
        .unwrap();
    session.profile_editor.username_mut().clear();
    session.profile_editor.username_mut().push_str("alicia");
    session
        .submit_raw_editor(
            &mut app,
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Username),
            Targets::Application,
        )
        .unwrap();
    session
        .submit_raw_editor(
            &mut app,
            OperatorCommand::SaveProfile,
            Targets::Profile("alice"),
        )
        .unwrap();
    session.core.flush_writes();
    assert_eq!(session.profile_editor.settle(&mut session.core), ["alicia"]);
    assert!(session.core.profile("alice").is_none());
    assert_eq!(
        session.core.profile_identity("alicia"),
        Some(boot_identity),
        "rename preserves the queued profile identity"
    );
    assert!(!session.preferences.ready());

    session.pump(&mut app);
    assert!(!session.preferences.ready());
    assert!(
        !session.core.commands().reports().any(|report| matches!(
            report.command(),
            OperatorCommand::Load | OperatorCommand::Login
        )),
        "rename does not bypass the preference-readiness gate"
    );

    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);

    assert_eq!(session.core.members(), ["alicia"]);
    assert!(
        session
            .core
            .play_for_test()
            .unwrap()
            .arm("alicia")
            .unwrap()
            .wants_login(),
        "the renamed boot target retains its login intent"
    );
    for command in [OperatorCommand::Load, OperatorCommand::Login] {
        assert_eq!(
            session
                .core
                .commands()
                .reports()
                .filter(|report| {
                    report.command() == command
                        && report
                            .member(frontend_core::command::Subject::Profile(boot_identity))
                            .is_some()
                })
                .count(),
            1,
            "{command:?} is submitted once for the renamed boot UID"
        );
    }
    assert_eq!(session.core.selected(), Some("alicia"));
    assert_eq!(app.focused_name().as_deref(), Some("alicia"));
}

#[test]
fn o1_duplicate_startup_targets_load_each_unique_profile_once() {
    let iso = IsolatedStores::enter("tui-o1-duplicate-startup-targets");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session.preferences.reload().unwrap();

    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();

    for _ in 0..512 {
        for name in ["alice", "bob"] {
            let args = parse_args_from(["--user", name]).unwrap();
            let _ = interactive_landing(&mut session, &args).unwrap();
        }
    }
    session.pump(&mut app);

    assert_eq!(
        session.core.members(),
        ["alice", "bob"],
        "repeated requests retain each unique target once"
    );
    for name in ["alice", "bob"] {
        let identity = session.core.profile_identity(name).unwrap();
        assert!(
            session
                .core
                .play_for_test()
                .unwrap()
                .arm(name)
                .unwrap()
                .wants_login(),
            "{name} retains its login intent"
        );
        for command in [OperatorCommand::Load, OperatorCommand::Login] {
            assert_eq!(
                session
                    .core
                    .commands()
                    .reports()
                    .filter(|report| {
                        report.command() == command
                            && report
                                .member(frontend_core::command::Subject::Profile(identity))
                                .is_some()
                    })
                    .count(),
                1,
                "{command:?} is submitted once for {name} despite duplicate requests"
            );
        }
    }
    assert_eq!(session.core.selected(), Some("bob"));
}

/// Build the disposable vault/preferences state used by the headed O1 run.
/// Set `O1_HEADED_HOME` to an absolute empty directory; this test refuses to
/// overwrite anything and deliberately leaves the same vault for restarts.
#[test]
#[ignore = "writes a headed-test fixture only into the caller-selected empty O1_HEADED_HOME"]
fn prepare_startup_order_headed_fixture() {
    let home = std::path::PathBuf::from(
        std::env::var_os("O1_HEADED_HOME")
            .expect("set O1_HEADED_HOME to an absolute empty disposable directory"),
    );
    assert!(home.is_absolute(), "O1_HEADED_HOME must be absolute");
    if !home.exists() {
        std::fs::create_dir_all(&home).expect("create O1_HEADED_HOME");
    }
    assert!(
        std::fs::read_dir(&home)
            .expect("read O1_HEADED_HOME")
            .next()
            .is_none(),
        "refusing to write a headed fixture into nonempty O1_HEADED_HOME"
    );

    let state_dir = home.join(".274bot");
    std::fs::create_dir(&state_dir).expect("create fresh .274bot state directory");
    let vault_path = state_dir.join("vault-289");
    let preferences_path = state_dir.join("panel-ui.json");
    let mut vault = VaultFixture::create(&vault_path, "test-passphrase-01");
    for (username, uid) in [("o1xa", 1), ("o1xb", 2), ("o1xc", 3)] {
        vault.put(
            FixtureProfile::new(username, "test-passphrase-01")
                .uid(uid)
                .auto_login(false),
        );
    }
    crate::test_support::write_private_fixture(
        &preferences_path,
        br#"{"profile_order":{"3":65536,"1":131072,"2":196608},"last_focus":"o1xb"}"#,
    )
    .expect("write saved O1 profile order");

    println!("O1_HEADED_HOME={}", home.display());
    println!("vault={}", vault_path.display());
    println!("preferences={}", preferences_path.display());
    println!("vault_passphrase=test-passphrase-01");
    println!("account_password=test-passphrase-01");
    println!("profile_order=o1xc,o1xa,o1xb");
    println!("last_focus=o1xb");
}

#[test]
fn deleting_pending_startup_profile_cancels_load_and_focus() {
    let iso = IsolatedStores::enter("tui-delete-pending-startup-profile");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7));
    session.start_play(vault).unwrap();
    session.queue_startup_focus("alice");
    let mut app = new_app("deleted startup profile", &session.preferences);
    app.names = session.names.clone();

    let delete = session
        .submit(OperatorCommand::DeleteProfile, Targets::Profile("alice"))
        .unwrap();
    assert!(delete.operation().is_some());
    session.core.flush_writes();
    for _ in 0..4 {
        session.pump(&mut app);
        assert!(!session.preferences.ready());
    }
    assert!(session.core.profile("alice").is_none());
    assert!(session.core.members().is_empty());
    assert!(session.core.selected().is_none());
    let load_count = session
        .core
        .commands()
        .reports()
        .filter(|report| report.command() == OperatorCommand::Load)
        .count();
    let login_count = session
        .core
        .commands()
        .reports()
        .filter(|report| report.command() == OperatorCommand::Login)
        .count();

    release.send(()).unwrap();
    session.preferences.reload().unwrap();
    session.pump(&mut app);
    assert!(session.core.members().is_empty());
    assert!(session.core.selected().is_none());
    assert!(app.focused_name().is_none());
    assert!(
        session.error.is_none(),
        "the deleted profile was not retried"
    );
    assert_eq!(
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == OperatorCommand::Load)
            .count(),
        load_count,
        "the deleted startup profile must not be loaded"
    );
    assert_eq!(
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == OperatorCommand::Login)
            .count(),
        login_count,
        "the deleted startup profile must not be logged in"
    );
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
        let mut session = TuiSession::new_bound(Arc::clone(&template));
        let vault = VaultFixture::create(&home.root.join(vault), "test-passphrase-01");
        session.start_play(vault).unwrap();
        (session, template)
    };

    let profile = home.bind(None);
    assert!(profile.nav_origin().is_bundled());
    let (session, template) = start(profile, "bundled.vault");
    let world = template.world().expect("the packaged world decodes");
    let played = session
        .core
        .play_for_test()
        .and_then(|play| play.world())
        .expect("the TUI play has the packaged world");
    assert!(Arc::ptr_eq(&played, &world));
    let routed = session.core.nav_world();
    assert!(Arc::ptr_eq(&routed.expect("WalkTo world"), &world));
    assert!(home.old_pack_untouched());

    let overridden = home.bind(Some(home.old_pack.clone()));
    assert!(matches!(
        overridden.nav_availability(),
        host_play::profile::NavAvailability::Unavailable(message)
            if message.contains("rebuild it with nav-pack")
    ));
    let (session, _) = start(overridden, "override.vault");
    assert!(session.core.play_for_test().unwrap().world().is_none());
    assert!(session.core.nav_world().is_none());
    assert!(home.old_pack_untouched());
}

#[test]
fn tui_reach_layer_uses_shared_lazy_sidecar_and_releases_its_lease() {
    let home = upgrade_home::UpgradeHome::new();
    let profile = home.bind(None);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let mut session = TuiSession::new_bound(template);
    let vault = VaultFixture::create(&home.root.join("reach.vault"), "test-passphrase-01");
    session.start_play(vault).unwrap();
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
    session.core.set_play(None);
    app.map_active = true;
    session.pump(&mut app);
    assert!(
        app.map_reach.is_none(),
        "a missing core world must not retain its previous paint mask"
    );
}

#[cfg(feature = "js")]
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
    assert_eq!(js.view().name(), "bone_burier_v2_js");
    assert_eq!(js.view().start_file(), Some("bone_burier_v2.js"));
    assert_eq!(ts.view().name(), "bone_burier_v2_ts");
    assert_eq!(ts.view().start_file(), Some("bone_burier_v2.ts"));
    assert_eq!(v1.view().start_script(), Some("BoneBurier"));
    assert_eq!(v1.view().start_file(), None);
    assert!(live_scenario("script_nope").is_err());
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_bone_burier_selects_the_rs2b0t_card_without_starting() {
    let iso = IsolatedStores::enter("tui-bone-live");
    let root = fake_rs2b0t_tree(iso.dir(), false);
    iso.set_catalog_root(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(frontend_core::live::Scenario::get("bone_burier").expect("registered"))
        .expect("prepare");
    let name = session.names.first().expect("minted name").clone();
    assert_ne!(name, "test", "live must not log in `test`");
    assert_eq!(
        session.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
        "prepare sets script_sel to the catalog card"
    );
    assert!(session.core.play_for_test().is_some(), "play started");
    assert!(
        session.core.fleet().contains(&name),
        "the minted driver is a loaded member"
    );
    let pending = session.core.live_runner().pending_starts_for_test();
    assert_eq!(
        pending
            .iter()
            .map(|pending| pending.slot.as_str())
            .collect::<Vec<_>>(),
        [name.as_str()],
        "preparation stages the selected card for StartScript"
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_script_trade_loads_the_file_fixture_without_a_catalog_card() {
    // TradeBot is a checked-in file fixture, not a catalog card: prepare
    // must load it as File (the panel branch), never fail with
    // `no card (Catalog, TradeBot)`.
    let _iso = IsolatedStores::enter("tui-trade-live");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(
            frontend_core::live::Scenario::get("script_trade").expect("registered"),
        )
        .expect("prepare");
    assert_eq!(
        session.script_sel,
        Some(ScriptChoice::loaded(CardSource::File, "trade_bot")),
        "script_trade loads the in-tree TradeBot fixture as File"
    );
    assert_eq!(
        session.names.len(),
        2,
        "script_trade is a two-profile fleet"
    );
    let pending = session.core.live_runner().pending_starts_for_test();
    assert_eq!(
        pending
            .iter()
            .map(|pending| pending.slot.as_str())
            .collect::<Vec<_>>(),
        [session.names[0].as_str(), session.names[1].as_str()],
        "StartScript loads TradeBot on both fleet slots"
    );
    assert_eq!(
        pending[0]
            .bag
            .as_ref()
            .expect("driven partner inject")
            .get("partner"),
        Some(&serde_json::json!(frontend_core::live::screen_name(
            &session.names[1]
        ))),
        "driven partner is the companion as the game shows it"
    );
    assert_eq!(
        pending[1]
            .bag
            .as_ref()
            .expect("companion partner inject")
            .get("partner"),
        Some(&serde_json::json!(frontend_core::live::screen_name(
            &session.names[0]
        ))),
        "companion partner is the driven player as the game shows it"
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_jive_kq_stashes_four_starts_with_the_same_roster() {
    let iso = IsolatedStores::enter("tui-jive-kq-fleet");
    let root = fake_rs2b0t_tree(iso.dir(), true);
    iso.set_catalog_root(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(
            frontend_core::live::Scenario::get("jive_kq_four").expect("registered"),
        )
        .expect("prepare four-player JiveKQ");

    assert_eq!(session.names.len(), 4, "JiveKQ mints four fleet profiles");
    let roster = session
        .names
        .iter()
        .map(|name| client::util::JString::to_screen_name(name))
        .collect::<Vec<_>>()
        .join(",");
    let starts = session.core.live_runner().pending_starts_for_test();
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

#[cfg(feature = "js")]
#[test]
fn live_prepare_bone_burier_v2_selects_each_example_by_identity() {
    let ts = live_example_path("bone_burier_v2.ts").expect("ts example");
    let js = live_example_path("bone_burier_v2.js").expect("js example");
    for (name, path) in [
        ("bone_burier_v2_ts", ts.as_path()),
        ("bone_burier_v2_js", js.as_path()),
    ] {
        let iso = IsolatedStores::enter(&format!("tui-bone-v2-{name}"));
        let mut session = TuiSession::new(dummy_options());
        session.core.set_spawn_workers(false);
        session.scripts.use_library_at(
            iso.dir().join("js-scripts.json"),
            iso.dir().join("js-cache"),
        );
        session.scripts.load_card(&ts).expect("preload ts");
        session.scripts.load_card(&js).expect("preload js");
        session
            .scripts
            .set_legacy_str(CardSource::File, "bone_burier_v2", "boneName", "stem");
        let identity = file_identity(path);
        session
            .scripts
            .set_legacy_str(CardSource::File, &identity, "boneName", "identity");
        session
            .live_prepare_script(frontend_core::live::Scenario::get(name).expect("registered"))
            .expect("prepare");
        assert_eq!(
            session.script_sel,
            Some(ScriptChoice::loaded(CardSource::File, &identity)),
            "{name} must select the canonical-path identity"
        );
        assert_ne!(
            session.script_sel,
            Some(ScriptChoice::loaded(CardSource::File, "bone_burier_v2"))
        );
        let pending = session.core.live_runner().pending_starts_for_test();
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
            session.core.live_runner().wait_script_stop(),
            Some("confirmed loaded current-generation bank exhaustion")
        );
    }
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_thiever_posts_guard_target_when_schema_empty() {
    let iso = IsolatedStores::enter("tui-thiever-bag");
    let root = iso.dir().join("rs2b0t");
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
    iso.set_catalog_root(&root);
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    session
        .live_prepare_script(frontend_core::live::Scenario::get("thiever").expect("registered"))
        .expect("prepare");
    let pending = session.core.live_runner().pending_starts_for_test();
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
        vec![api::loadout::Loadout::new("Memory food").with_carry("Lobster", 1)],
        "production live preparation stages scenario loadouts for catalog Start"
    );
}

#[cfg(feature = "js")]
#[test]
fn first_browse_without_rs2b0t_opens_catalog_prompt() {
    let iso = IsolatedStores::enter("tui-browse");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.script_browse_open = true;
    session.on_script_browse_open(&mut app);
    assert!(
        session.rs2b0t_catalog_open,
        "first browse opens folder picker"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "first browse must not write rs2b0t-path"
    );
}

#[cfg(feature = "js")]
#[test]
fn defer_rs2b0t_catalog_leaves_no_path_and_zero_catalog_cards() {
    let iso = IsolatedStores::enter("tui-defer");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    app.script_browse_open = true;
    session.on_script_browse_open(&mut app);
    session.defer_rs2b0t_catalog(&mut app);
    assert!(
        frontend_core::scripts::paths::rs2b0t_import_deferred_at(
            &iso.home().join(".274bot/rs2b0t-import")
        ),
        "defer flag written"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "defer must not write rs2b0t-path"
    );
    assert!(
        !session.scripts.has_cards_from(CardSource::Catalog),
        "zero Catalog cards after defer"
    );
}

#[cfg(feature = "js")]
#[test]
fn import_rs2b0t_catalog_persists_path_and_registers_cards() {
    let iso = IsolatedStores::enter("tui-import");
    let root = fake_rs2b0t_tree(iso.dir(), false);
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    session
        .import_rs2b0t_catalog(&mut app, &root)
        .expect("import");
    assert!(session.scripts.catalog_work_pending());
    assert!(!iso.home().join(".274bot/rs2b0t-path").exists());
    settle_catalog_work(&mut session, &mut app);
    assert!(iso.home().join(".274bot/rs2b0t-path").is_file());
    let card = session
        .scripts
        .card(CardSource::Catalog, "BoneBurier")
        .expect("catalog card");
    assert_eq!(card.description(), "Buries bones");
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
/// The default boot target is chosen from saved order after shared startup
/// preparation, then loaded and logged in before a loaded script starts on it.
/// The same check as the panel's unlock focus (F4 rs2b2t live session:
/// `script: no slot: <name>` on the auto-logged-in bot).
#[cfg(feature = "js")]
#[test]
fn the_boot_focus_is_a_fleet_member_and_starts_a_loaded_script() {
    let iso = IsolatedStores::enter("tui-boot-focus");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    vault.put(FixtureProfile::new("alice", "pw").uid(7).auto_login(true));
    vault.put(FixtureProfile::new("bob", "pw").uid(8));
    session.start_play(vault).unwrap();
    let args = parse_args_from(std::iter::empty::<&str>()).unwrap();
    let mut app = interactive_landing(&mut session, &args).unwrap();
    assert!(session.startup_default_pending);
    session.pump(&mut app);
    assert_eq!(session.core.members(), ["alice".to_string()]);
    assert_eq!(session.core.selected(), Some("alice"));
    assert_eq!(app.focused_name().as_deref(), Some("alice"));
    let path = iso.dir().join("boot-focus.ts");
    std::fs::write(
        &path,
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    session.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(CardSource::File, &path.to_string_lossy());
    session.script_start(&mut app, &sel);
    assert_eq!(app.error, None, "a loaded script starts on the boot's bot");
}

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

#[cfg(feature = "js")]
/// Flush writes and pump until the latest fleet result has settled.
fn settle_fleet_result(session: &mut TuiSession, app: &mut TuiApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.core.flush_writes();
        session.pump(app);
        if session.core.fleet_result().is_some_and(|r| r.settled()) {
            return;
        }
        assert!(Instant::now() < deadline, "the fleet result did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(feature = "js")]
fn settle_catalog_work(session: &mut TuiSession, app: &mut TuiApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.scripts.catalog_work_pending() && Instant::now() < deadline {
        session.core.poll();
        session.poll_scripts(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    session.core.poll();
    session.poll_scripts(app);
    assert!(
        !session.scripts.catalog_work_pending(),
        "catalog work did not settle"
    );
}

fn settle_preferences(session: &mut TuiSession, app: &mut TuiApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.pump(app);
        if !session.core.preferences_pending() {
            // Projection precedes core polling, so deliver the settled snapshot next frame.
            session.pump(app);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the preference commands did not finish"
        );
        std::thread::yield_now();
    }
}

#[cfg(feature = "js")]
#[test]
fn initial_runtime_failure_survives_success_and_refusals_in_tui_output() {
    let iso = IsolatedStores::enter("tui-initial-load");
    let mut session = TuiSession::new(dummy_options());
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    for (uid, name) in [(7, "alice"), (8, "bob")] {
        vault.put(FixtureProfile::new(name, "pw").uid(uid));
    }
    install_vault(&mut session.core, vault);
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(7, false));
    play.attach_arm("bob", SlotArm::new(8, false));
    session.inject_play(play);
    session.core.fleet_mut().add("alice");
    session.core.fleet_mut().add("bob");
    let mut app = TuiApp::new("initial load proof");
    app.names = vec!["alice".into(), "bob".into(), "missing".into()];
    app.focused = Some(0);
    let path = iso.dir().join("retry.ts");
    let helper = iso.dir().join("gate.ts");
    std::fs::write(&helper, "export const fail = true;").unwrap();
    let src = "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('tui-initial-load');\nexport function tick(api) {}";
    std::fs::write(&path, src).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(CardSource::File, &path.to_string_lossy());
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    // The failure reaches the operator once setup settles, as the
    // synchronous Start error used to.
    let shown = app.error.clone().unwrap_or_default();
    assert!(
        shown.starts_with("script: ") && shown.contains("tui-initial-load"),
        "{shown}"
    );
    let failure = session.scripts.load_failure(card.identity_key()).unwrap();
    assert_eq!(failure.identity_key(), card.identity_key());
    assert_eq!(failure.path(), path.as_path());
    assert_eq!(failure.stage(), LoadStage::RuntimeLoad);
    let retained = failure.copyable_text();
    let retained_failure = |session: &TuiSession| {
        session
            .scripts
            .load_failure(card.identity_key())
            .map(|failure| failure.copyable_text())
    };
    assert_eq!(
        session
            .scripts
            .load_failure_api_family(card.identity_key())
            .as_deref(),
        Some("v2")
    );
    assert_eq!(
        session
            .scripts
            .load_failure_fingerprint(card.identity_key()),
        Some(raw_content_fingerprint(&path, src))
    );
    assert_eq!(
        session
            .core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(0)
    );

    let good_path = iso.dir().join("good.ts");
    std::fs::write(
        &good_path,
        "export default class T extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    let good = session.scripts.load_card(&good_path).unwrap();
    assert_eq!(
        card_api_family(&session.scripts, good.source(), good.identity_id()).as_deref(),
        Some("v1")
    );
    app.focused = Some(1);
    session.script_start(
        &mut app,
        &ScriptChoice::loaded(CardSource::File, &good_path.to_string_lossy()),
    );
    settle_starts(&mut session, &mut app);
    assert_eq!(session.core.script_state("bob"), ScriptRunState::Running);
    let output = app.error.as_deref().unwrap();
    assert!(output.contains("tui-initial-load") && output.contains("runtime-load"));
    assert!(output.contains(&path.display().to_string()));
    session.script_start(&mut app, &sel); // active slot refuses before evaluating
    assert!(app
        .error
        .as_deref()
        .unwrap()
        .contains("script already active"));
    assert_eq!(retained_failure(&session).as_ref(), Some(&retained));
    app.focused = Some(2);
    session.script_start(&mut app, &sel);
    assert_eq!(app.error.as_deref(), Some("script: missing is not loaded"));
    assert_eq!(retained_failure(&session).as_ref(), Some(&retained));

    std::fs::write(&helper, "export const fail = false;").unwrap();
    app.focused = Some(0);
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    assert!(session.scripts.load_failure(card.identity_key()).is_none());
    assert_eq!(app.error, None);
    assert_eq!(
        session
            .core
            .play_for_test()
            .unwrap()
            .script_runtime_generation("alice"),
        Some(1)
    );
    session.core.play_for_test().unwrap().script_stop("alice");
    session.core.play_for_test().unwrap().script_stop("bob");
}

/// Task 13 fix: the paint-as-chat toggle must not stick across a
/// Stop → new Start. A slot whose script has no paint (stopped, or
/// not painted yet) resets the toggle, so the fresh paint is visible
/// by default instead of hidden behind the game-chat toggle.
/// TR-TUI-001: dispatching ScriptPause toggles pause/resume like the
/// panel's `script_toggle_pause` (Resume when Paused, Pause when Running).
#[cfg(feature = "js")]
#[test]
fn script_pause_toggle_resumes_when_paused_and_pauses_when_running() {
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    start_script_source(&play, "alice", src, LoadShape::NativeTick).unwrap();
    wait_script_state(&play, "alice", ScriptRunState::Running);

    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);

    dispatch(&mut session, &mut app, AppAction::ScriptPause);
    assert_eq!(
        session.core.script_state("alice"),
        ScriptRunState::Paused,
        "Pause while Running"
    );

    dispatch(&mut session, &mut app, AppAction::ScriptPause);
    assert_eq!(
        session.core.script_state("alice"),
        ScriptRunState::Running,
        "Resume while Paused"
    );
}

#[cfg(feature = "js")]
#[test]
fn paint_button_action_does_not_queue_a_wire_cmd() {
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(7, false));
    let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
    start_script_source(&play, "alice", src, LoadShape::NativeTick).unwrap();
    wait_script_state(&play, "alice", ScriptRunState::Running);
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(play);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.chat_data.script_paint = Some(
        TestPaint::new(Some("NatureCrafter"), &["status"])
            .with_button("gobank", "Go bank")
            .frame(),
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::Chat(ChatAction::PaintButton(0)),
    );
    assert_eq!(
        session.core.script_state("alice"),
        ScriptRunState::Running,
        "paint click must not pause or stop"
    );
}

#[cfg(feature = "js")]
/// The frame the standard paint-controls helper records, published on
/// `name`'s status row as the slot's observe does after a tick.
fn publish_helper_paint(session: &mut TuiSession, name: &str, generation: u64) {
    TestPaint::new(Some("Helper"), &["status"])
        .with_host_controls()
        .with_generation(generation)
        .publish(session.core.play_for_test().expect("play"), name);
    session.core.poll();
}

#[cfg(feature = "js")]
fn running_play(names: &[&str]) -> Play {
    let mut play = empty_play();
    for (uid, name) in names.iter().enumerate() {
        play.attach_arm(name, SlotArm::new(uid as i32 + 1, false));
        let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
        start_script_source(&play, name, src, LoadShape::NativeTick).unwrap();
        wait_script_state(&play, name, ScriptRunState::Running);
    }
    play
}

/// Digits, the focused Enter/Space and a mouse click on the drawn row run
/// the helper's host controls through the same core activation as the
/// panel: Pause, the retained frame's Resume, and Stop.
#[cfg(feature = "js")]
#[test]
fn tui_keys_and_mouse_run_the_paint_host_controls() {
    use crate::layout::Screen;
    use crate::test_support::{ch, draw, find, key};
    use crossterm::event::KeyCode;
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(running_play(&["alice"]));
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.show_screen(Screen::Chat);
    publish_helper_paint(&mut session, "alice", 51);
    let refresh = |session: &TuiSession, app: &mut TuiApp| {
        app.chat_data.script_paint = session.core.script_paint("alice");
    };

    refresh(&session, &mut app);
    let rows = draw(&mut app, 120, 40);
    assert!(find(&rows, "[1] Pause").is_some(), "{rows:#?}");
    assert!(find(&rows, "[2] Stop").is_some(), "{rows:#?}");
    let action = app.on_key(ch('1'));
    assert_eq!(action, AppAction::Chat(ChatAction::PaintButton(0)));
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.script_state("alice"), ScriptRunState::Paused);

    // The retained frame offers Resume without another script tick.
    refresh(&session, &mut app);
    let rows = draw(&mut app, 120, 40);
    assert!(find(&rows, "[1] Resume").is_some(), "{rows:#?}");
    let action = app.on_key(key(KeyCode::Enter));
    assert_eq!(action, AppAction::Chat(ChatAction::PaintButton(0)));
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.script_state("alice"), ScriptRunState::Running);

    refresh(&session, &mut app);
    let rows = draw(&mut app, 120, 40);
    let (col, row) = find(&rows, "[1] Pause").expect("Pause row drawn");
    let action = app.on_click(col, row);
    assert_eq!(action, AppAction::Chat(ChatAction::PaintButton(0)));
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.script_state("alice"), ScriptRunState::Paused);

    refresh(&session, &mut app);
    assert_eq!(app.on_key(ch('j')), AppAction::None, "focus moves to Stop");
    let action = app.on_key(ch(' '));
    assert_eq!(action, AppAction::Chat(ChatAction::PaintButton(1)));
    dispatch(&mut session, &mut app, action);
    wait_script_state(
        session.core.play_for_test().expect("play"),
        "alice",
        ScriptRunState::Idle,
    );
    assert!(app.error.is_none(), "{:?}", app.error);
}

/// A control drawn for alice never controls bob after the focus moves.
#[cfg(feature = "js")]
#[test]
fn tui_paint_control_shown_for_one_profile_is_refused_for_another() {
    let mut session = TuiSession::new(dummy_options());
    session.inject_play(running_play(&["alice", "bob"]));
    publish_helper_paint(&mut session, "alice", 61);
    publish_helper_paint(&mut session, "bob", 62);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into(), "bob".into()];
    app.chat_data.script_paint = session.core.script_paint("alice");
    app.focused = Some(1);
    for index in [0, 1] {
        dispatch(
            &mut session,
            &mut app,
            AppAction::Chat(ChatAction::PaintButton(index)),
        );
        assert!(
            app.error
                .as_deref()
                .is_some_and(|error| error.contains("another profile")),
            "{:?}",
            app.error
        );
    }
    assert_eq!(session.core.script_state("alice"), ScriptRunState::Running);
    assert_eq!(session.core.script_state("bob"), ScriptRunState::Running);
}

#[test]
fn pump_resets_the_paint_toggle_when_the_paint_is_gone() {
    let mut session = TuiSession::new(dummy_options());
    session.core.fleet_mut().add("test");
    select(&mut session, "test");
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

    use api::snapshot::WorldTile;
    use client::client::{Client, ClientConfig, ClientPlayer};
    use client::config::if_type::ComponentType;
    use client::config::{Cache, IfType, IfTypeMut, LocType, ObjType};
    use client::io::ServerProt;
    use nav_planner::pack::BankAccess;
    use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};
    use nav_planner::world::NavWorld;

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

    pub fn knife_nav_world(knife_id: i32) -> NavWorld {
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
        }
        let edge = TransportEdge {
            takeoff: None,
            worn_all_req: Vec::new(),
            forbidden_held: Vec::new(),
            forbidden_worn: Vec::new(),
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
        let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
        NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
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
            vec![nav_planner::pack::BankStand {
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
}

#[test]
fn map_walk_options_keep_the_frontend_bank_fetch_permission() {
    let mut app = TuiApp::new("274bot headless");
    app.nav = frontend_core::WalkGlobals::new(false, false, true, true, true);
    app.nav = app
        .nav
        .with_danger_level(frontend_core::DangerLevel::Always);
    assert!(app.map_find_options().allow_bank_fetch);
}

/// A map walk the TUI arms shows on the member's fleet row.
#[test]
fn a_map_walk_shows_the_member_running_on_its_fleet_row() {
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let mut session = TuiSession::new_bound(Arc::clone(&fixture.template));
    session.core.fleet_mut().add("alice");
    select(&mut session, "alice");
    session.core.set_play(Some(fixture.play(origin)));
    publish_map_walk_snapshot(&mut session.core, "alice", origin);
    let mut app = TuiApp::new("274bot headless");
    app.map_active = true;
    session.pump(&mut app);
    assert!(!app.fleet_model.rows()[0].manual_walk());

    let destination = Tile {
        x: 1,
        z: 4,
        level: 0,
    };
    app.select_requested(destination);
    session.arm_walk_on(&mut app, destination);
    assert_eq!(app.error, None);
    session.pump(&mut app);
    assert!(
        app.fleet_model.rows()[0].manual_walk(),
        "the armed walk shows on the row"
    );
    assert!(app.fleet_model.rows()[0].busy());
    assert!(session.core.walk_route_armed("alice"));
}

#[test]
fn removing_a_walk_slot_twice_logs_one_cancellation() {
    use frontend_core::log::{global, LogScope, LogView};
    use host_play::SlotStatus;

    let log = global();
    let slot = "walk-receipt-session-ended";
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let mut session = TuiSession::new_bound(Arc::clone(&fixture.template));
    session.core.fleet_mut().add(slot);
    select(&mut session, slot);
    let mut play = fixture.play(Tile {
        x: 0,
        z: 1,
        level: 0,
    });
    play.attach_arm(slot, SlotArm::new(1, false));
    play.statuses.lock().unwrap().push(SlotStatus {
        username: slot.into(),
        ingame: true,
        tile_x: 0,
        tile_z: 1,
        tile_level: 0,
        ..Default::default()
    });
    play.focus(slot);
    session.core.set_play(Some(play));

    let mut app = TuiApp::new("274bot headless");
    app.names = vec![slot.into()];
    app.focused = Some(0);
    app.map_active = true;
    app.world = Some(Arc::clone(&world));
    session.bind_map_context(&mut app);
    let destination = Tile {
        x: 1,
        z: 4,
        level: 0,
    };
    app.select_requested(destination);
    session.arm_walk_on(&mut app, destination);
    assert!(session.core.walk_route_armed(slot));

    let mut walk_cleared = false;
    {
        let mut surface = TuiSurface {
            walk_cleared: &mut walk_cleared,
            map_desk: &mut session.map_desk,
            models: frontend_core::Models::none(),
            frame_hook: None,
        };
        for _ in 0..2 {
            session
                .core
                .submit(
                    OperatorCommand::Remove,
                    Targets::Profile(slot),
                    &mut surface,
                )
                .unwrap();
        }
    }
    assert!(walk_cleared);

    let mut receipts = LogView::new(LogScope::slot(slot));
    receipts.edit_filter(|filter| filter.set_text("WalkTo outcome="));
    log.refresh(&mut receipts);
    assert_eq!(receipts.len(), 1, "{}", receipts.to_text());
    let receipt = receipts.rows()[0].message();
    assert!(receipt.contains("outcome=cancelled"), "{receipt}");
    assert!(receipt.contains("reason=SessionEnded"), "{receipt}");
}

#[test]
fn map_walk_uses_core_slot_position_and_keeps_snapped_destination() {
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let mut session = TuiSession::new_bound(Arc::clone(&fixture.template));
    let name = "tui-walk-receipt";
    session.core.fleet_mut().add(name);
    select(&mut session, name);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let mut play = fixture.play(origin);
    play.attach_arm(name, SlotArm::new(1, false));
    play.statuses.lock().unwrap().push(host_play::SlotStatus {
        username: name.into(),
        ingame: true,
        tile_x: origin.x,
        tile_z: origin.z,
        tile_level: origin.level,
        ..Default::default()
    });
    play.focus(name);
    session.core.set_play(Some(play));
    publish_map_walk_snapshot(&mut session.core, name, origin);

    let mut app = TuiApp::new("274bot headless");
    app.names = vec![name.into()];
    app.focused = Some(0);
    app.map_active = true;
    app.world = session.core.nav_world();
    session.bind_map_context(&mut app);
    let requested = Tile {
        x: 5,
        z: 2,
        level: 0,
    };
    app.select_requested(requested);
    let target = app.map_model.pending().unwrap().reported_destination();
    assert_ne!(target, requested);
    app.here = None;
    session.arm_walk_on(&mut app, requested);
    assert_eq!(app.error, None);
    assert_eq!(app.walk_dest, Some(target));
    assert!(session.core.walk_route_armed(name));
}

#[test]
fn production_walk_uses_the_live_slot_origin_when_ui_position_is_missing() {
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let mut session = TuiSession::new_bound(Arc::clone(&fixture.template));
    let name = "alice";
    session.core.fleet_mut().add(name);
    select(&mut session, name);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    session.core.set_play(Some(fixture.play(origin)));
    publish_map_walk_snapshot(&mut session.core, name, origin);
    let mut app = TuiApp::new("274bot headless");
    app.names = vec![name.into()];
    app.focused = Some(0);
    app.map_active = true;
    app.world = session.core.nav_world();
    session.bind_map_context(&mut app);
    let requested = Tile {
        x: 5,
        z: 2,
        level: 0,
    };
    app.select_requested(requested);
    let target = app.map_model.pending().unwrap().reported_destination();
    assert_ne!(target, requested);
    app.here = None;
    session.arm_walk_on(&mut app, requested);
    assert_eq!(app.error, None);
    assert_eq!(app.walk_dest, Some(target));
    assert!(session.core.walk_route_armed(name));
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
    let refused = app.map_enter();
    assert_eq!(
        refused,
        AppAction::Refused(crate::app::Refusal {
            command: Some(OperatorCommand::GroupWalk),
            bot: None,
            reason: "no bots selected".into(),
            need: crate::commands::Unavailable::NoCheckedBots.need(),
        })
    );
    dispatch(&mut session, &mut app, refused);
    assert_eq!(app.error.as_deref(), Some("no bots selected"));
    assert!(app.map_model.pending().is_some());
    app.error = None;
    session.map_walk_group(&mut app);
    assert_eq!(app.error.as_deref(), Some("no bots selected"));
    assert!(app.map_model.pending().is_some());
}

#[test]
fn refused_no_origin_walk_does_not_arm_destination() {
    let world = Arc::new(bank_fetch_fixtures::knife_nav_world(2));
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    let mut session = TuiSession::new_bound(Arc::clone(&fixture.template));
    session.core.fleet_mut().add("alice");
    select(&mut session, "alice");
    session.core.set_play(Some(fixture.play(Tile {
        x: 0,
        z: 0,
        level: 0,
    })));
    let mut app = TuiApp::new("274bot headless");
    app.names = vec!["alice".into()];
    app.focused = Some(0);
    app.map_active = true;
    app.world = Some(Arc::clone(&world));
    session.bind_map_context(&mut app);
    let destination = Tile {
        x: 1,
        z: 4,
        level: 0,
    };
    app.select_requested(destination);
    session.arm_walk_on(&mut app, destination);
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
    let _iso = IsolatedStores::enter("tui-worker-panic-hook");
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
    use nav_planner::bank_fetch::BankStep;
    use nav_planner::router::Route;
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
    let pending = PendingBankFetch::for_test_with_route(
        VecDeque::from([BankStep::Walk {
            x: here.0,
            z: here.1,
            level: 1,
        }]),
        final_route.dest,
        FindOptions::default(),
        final_route.clone(),
    );

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
    use nav_planner::bank_fetch::BankStep;
    use nav_planner::router::Route;
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
        bank_fetch: Some(PendingBankFetch::for_test_with_route(
            VecDeque::from([BankStep::Withdraw { id: 2, count: 1 }, BankStep::Close]),
            WorldTile {
                x: 4,
                z: 4,
                level: 0,
            },
            FindOptions::default(),
            final_route.clone(),
        )),
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
    use nav_planner::bank_fetch::BankStep;
    use nav_planner::router::{Leg, Route};
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
        bank_fetch: Some(PendingBankFetch::for_test_with_route(
            VecDeque::from([BankStep::Walk {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            }]),
            destination,
            FindOptions::default(),
            final_route.clone(),
        )),
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
    let mut receipts = LogView::new(LogScope::slot("receipt-bank-root"));
    receipts.edit_filter(|filter| filter.set_text("WalkTo outcome="));
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
    let message = receipts.rows()[0].message();
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
    use nav_planner::router::{Leg, Route};
    use nav_planner::transport::{TransportEdge, TransportKind};

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
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
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

    let mut arrived = LogView::new(LogScope::slot("receipt-arrived"));
    arrived.edit_filter(|filter| filter.set_text("WalkTo outcome="));
    log.refresh(&mut arrived);
    let mut aborted = LogView::new(LogScope::slot("receipt-aborted"));
    aborted.edit_filter(|filter| filter.set_text("WalkTo outcome="));
    log.refresh(&mut aborted);
    assert_eq!(arrived.len(), 1, "{}", arrived.to_text());
    assert_eq!(aborted.len(), 1, "{}", aborted.to_text());
    let arrived_message = arrived.rows()[0].message();
    let at = format!("({},{},{})", at.x, at.z, at.level);
    assert_walk_receipt_fields(arrived_message, "arrived", &at, &at, "-", "-");
    assert_eq!(
        walk_receipt_field(arrived_message, "leg"),
        Some("-"),
        "{arrived_message:?}"
    );
    let aborted_message = aborted.rows()[0].message();
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
    frontend_core::test_support::empty_play(&dummy_options())
}

fn two_arm_play() -> Play {
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(1, false));
    play.attach_arm("bob", SlotArm::new(2, false));
    play
}

fn pump_two_slots(session: &mut TuiSession) -> TuiApp {
    session.preferences.reload().unwrap();
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
    let iso = IsolatedStores::enter("tui-live-ack");
    let mut session = TuiSession::new(dummy_options());
    session.persist_ui = false;
    let app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_none());
    let mut app = app;
    session.ack_background_bots(&mut app);
    assert!(!frontend_core::resources::background_bots_acked(
        &session.preferences
    ));
    let _iso = iso;
}

#[test]
fn tui_notice_uses_shared_ack_file() {
    let iso = IsolatedStores::enter("tui-shared-ack");
    let mut session = TuiSession::new(dummy_options());
    let mut app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_some());
    dispatch(&mut session, &mut app, AppAction::AckBackground);
    settle_preferences(&mut session, &mut app);

    session.expire_ack_cache();
    session.pump(&mut app);
    assert!(app.background_notice.is_none());
    let _iso = iso;
}

#[test]
fn tui_settings_map_bake_choice_is_the_panel_prefs_key() {
    let iso = IsolatedStores::enter("tui-map-bake");
    let path = frontend_core::scripts::paths::bot_home()
        .join(".274bot")
        .join("panel-ui.json");
    crate::test_support::write_private_fixture(
        &path,
        r#"{"last_focus":"alice","map_bake":"always"}"#,
    )
    .unwrap();
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    session.pump(&mut app);
    assert_eq!(
        session.map_desk.bake_gate().choice(),
        frontend_core::MapBakeChoice::Always,
        "the panel's remembered choice is read at start"
    );
    app.map_bake = frontend_core::MapBakeChoice::Ask;
    app.map_bake_dirty = true;
    settle_preferences(&mut session, &mut app);
    assert!(!app.map_bake_dirty);
    assert_eq!(
        session.map_desk.bake_gate().choice(),
        frontend_core::MapBakeChoice::Ask
    );
    assert_eq!(
        frontend_core::load_map_bake_choice(&session.preferences),
        frontend_core::MapBakeChoice::Ask
    );
    let prefs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session.preferences.path()).unwrap()).unwrap();
    assert_eq!(prefs["last_focus"], "alice", "other prefs survive");
    let _iso = iso;
}

/// T5.2 r3 round trip: the TUI's composition root saves a preference (the
/// Settings map-bake choice, through the pump) and a Loadouts edit (Enter
/// on the popup's name row) through the session's local-state service.
/// Another process's service on the same home reads both back; its own
/// edits reach the TUI's forms after they re-read, and the TUI's next saves
/// keep them.
#[test]
fn a_preference_and_a_loadout_edit_round_trip_through_the_service() {
    let iso = IsolatedStores::enter("tui-state-round-trip");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.map_bake = frontend_core::MapBakeChoice::Always;
    app.map_bake_dirty = true;
    settle_preferences(&mut session, &mut app);
    assert_eq!(app.error, None);
    app.loadouts_state = crate::loadouts::LoadoutsState {
        open: true,
        row: 1,
        name_scratch: "roundtrip".into(),
        carry_scratch: "Lobster:5".into(),
        ..Default::default()
    };
    let enter = settings_key(crossterm::event::KeyCode::Enter);
    assert!(session.loadouts_key(&mut app, enter));
    assert_eq!(session.loadouts.status(), "Saved.");

    // Another 274bot process on this home: its own service, same files.
    let root = host_play::local_state::StateRoot::new(
        frontend_core::scripts::paths::bot_home().join(".274bot"),
    );
    let peer = host_play::local_state::LocalStateService::open_process(root.clone());
    peer.preferences().reload().unwrap();
    assert_eq!(
        peer.preferences().value("map_bake"),
        Some(serde_json::json!("always"))
    );
    let mut peer_form = Loadouts::operator(&peer);
    assert_eq!(
        peer_form.get("roundtrip").unwrap().carry,
        vec![api::loadout::CarryEntry::new("Lobster", 5)]
    );
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.loadouts_file()).unwrap()).unwrap();
    assert_eq!(on_disk[0]["name"], "roundtrip", "{on_disk}");

    // The peer's edits reach the TUI through the TUI's service.
    peer.preferences()
        .set("peer_marker", serde_json::json!({"kept": 7}))
        .unwrap();
    peer_form.upsert_for_test(api::loadout::Loadout::new("peer-row"));
    peer_form.save_for_test().unwrap();
    session.preferences.reload().unwrap();
    assert_eq!(
        session.preferences.value("peer_marker"),
        Some(serde_json::json!({"kept": 7}))
    );
    session.loadouts.reload_for_test().unwrap();
    assert_eq!(session.loadouts.names(), ["roundtrip", "peer-row"]);

    // The TUI's next saves keep the peer's row and preference.
    session.loadouts.stage_selection(0);
    app.loadouts_state.row = 4;
    app.loadouts_state.name_scratch = "roundtrip".into();
    app.loadouts_state.carry_scratch = "Lobster:6".into();
    assert!(session.loadouts_key(&mut app, enter));
    assert_eq!(session.loadouts.status(), "Saved.");
    app.map_bake = frontend_core::MapBakeChoice::Ask;
    app.map_bake_dirty = true;
    settle_preferences(&mut session, &mut app);
    assert_eq!(app.error, None);
    let mut reread = Loadouts::operator(&peer);
    reread.reload_for_test().unwrap();
    assert_eq!(reread.names(), ["roundtrip", "peer-row"]);
    assert_eq!(reread.get("roundtrip").unwrap().carry[0].qty, 6);
    let prefs = peer.preferences();
    prefs.reload().unwrap();
    assert_eq!(
        prefs.value("peer_marker"),
        Some(serde_json::json!({"kept": 7}))
    );
    assert_eq!(prefs.value("map_bake"), Some(serde_json::json!("ask")));
    let _iso = iso;
}

/// A loadouts list on its own scratch state root, `rows` saved in it, and
/// the service behind it (another list over the same service reads what the
/// first one saved).
fn seeded_loadouts(
    label: &str,
    rows: Vec<api::loadout::Loadout>,
) -> (Loadouts, Arc<host_play::local_state::LocalStateService>) {
    let root = host_play::local_state::StateRoot::new(
        frontend_core::scripts::paths::bot_home().join(format!(".274bot-loadouts-{label}")),
    );
    let service = host_play::local_state::LocalStateService::open_process(root);
    let mut store = Loadouts::operator(&service);
    for row in rows {
        store.upsert_for_test(row);
    }
    store.save_for_test().unwrap();
    (store, service)
}

/// Enter on the popup's `[new]` row creates a loadout, writes it and shows
/// it selected.
#[test]
fn loadouts_popup_adds_a_preset_and_lists_names() {
    let _iso = IsolatedStores::enter("tui-loadouts-add");
    let mut session = TuiSession::new(dummy_options());
    let (store, _service) = seeded_loadouts(
        "add",
        vec![api::loadout::Loadout::new("fish").with_carry("net", 1)],
    );
    session.loadouts = frontend_core::scripts::LoadoutForm::over(store);
    let mut app = TuiApp::new("tui");
    app.loadouts_state = crate::loadouts::LoadoutsState {
        open: true,
        row: 5,
        ..Default::default()
    };
    app.loadouts_state.fill(&session.loadouts);
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Enter)));
    assert_eq!(session.loadouts.names().len(), 2);
    assert!(session
        .loadouts
        .names()
        .iter()
        .any(|name| name.starts_with("loadout")));
    assert_eq!(session.loadouts.status(), "Saved.");
    assert_eq!(session.loadouts.selected(), 1, "the new row is selected");
    assert_eq!(app.loadouts_state.name_scratch, session.loadouts.names()[1]);
}

/// A text row committed with Enter replaces the selected loadout where it
/// stands.
#[test]
fn apply_scratch_renames_in_place() {
    let _iso = IsolatedStores::enter("tui-loadouts-rename");
    let mut session = TuiSession::new(dummy_options());
    let (store, _service) = seeded_loadouts("rename", vec![api::loadout::Loadout::new("melee")]);
    session.loadouts = frontend_core::scripts::LoadoutForm::over(store);
    let mut app = TuiApp::new("tui");
    app.loadouts_state = crate::loadouts::LoadoutsState {
        open: true,
        row: 1,
        name_scratch: "melee2".into(),
        ..Default::default()
    };
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Enter)));
    assert_eq!(session.loadouts.loadouts().len(), 1);
    assert_eq!(session.loadouts.loadouts()[0].name, "melee2");
    assert_eq!(session.loadouts.selected(), 0);
    assert_eq!(session.loadouts.status(), "Saved.");
}

/// Rows are keyed by name in the store: renaming onto another row's name
/// is refused and both rows stay, in the form and in the file.
#[test]
fn renaming_onto_another_loadouts_name_is_refused_and_keeps_both() {
    let _iso = IsolatedStores::enter("tui-loadouts-collide");
    let mut session = TuiSession::new(dummy_options());
    let (store, service) = seeded_loadouts(
        "collide",
        vec![
            api::loadout::Loadout::new("melee").with_carry("Lobster", 8),
            api::loadout::Loadout::new("range"),
        ],
    );
    session.loadouts = frontend_core::scripts::LoadoutForm::over(store);
    session.loadouts.stage_selection(1);
    let mut app = TuiApp::new("tui");
    app.loadouts_state = crate::loadouts::LoadoutsState {
        open: true,
        row: 1,
        name_scratch: "melee".into(),
        ..Default::default()
    };
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Enter)));
    assert_eq!(
        session.loadouts.status(),
        "A loadout named \"melee\" already exists."
    );
    assert_eq!(session.loadouts.names(), ["melee", "range"]);
    assert!(!session.loadouts.store().is_dirty());
    let reread = Loadouts::operator(&service);
    assert_eq!(reread.names(), ["melee", "range"]);
    assert_eq!(
        reread.get("melee").unwrap().carry,
        vec![api::loadout::CarryEntry::new("Lobster", 8)]
    );
}

#[test]
fn invalid_empty_name_does_not_clobber_store() {
    let _iso = IsolatedStores::enter("tui-loadouts-empty-name");
    let mut session = TuiSession::new(dummy_options());
    let (store, _service) = seeded_loadouts(
        "empty-name",
        vec![api::loadout::Loadout::new("fish").with_carry("net", 1)],
    );
    session.loadouts = frontend_core::scripts::LoadoutForm::over(store);
    let mut app = TuiApp::new("tui");
    app.loadouts_state = crate::loadouts::LoadoutsState {
        open: true,
        row: 1,
        name_scratch: " ".into(),
        ..Default::default()
    };
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Enter)));
    assert_eq!(session.loadouts.loadouts()[0].name, "fish");
    assert_eq!(session.loadouts.status(), "Name is required.");
    assert_eq!(app.loadouts_state.name_scratch, " ", "the typed text stays");
}

/// Opening the popup selects the first loadout and reads its text; Enter on
/// the `selected` row steps through the loadouts and wraps.
#[test]
fn opening_the_loadouts_popup_selects_the_first_row_and_row_zero_cycles() {
    let _iso = IsolatedStores::enter("tui-loadouts-open");
    let mut session = TuiSession::new(dummy_options());
    let (store, _service) = seeded_loadouts(
        "open",
        vec![
            api::loadout::Loadout::new("a"),
            api::loadout::Loadout::new("b"),
        ],
    );
    session.loadouts = frontend_core::scripts::LoadoutForm::over(store);
    session.loadouts.stage_selection(1);
    let mut app = TuiApp::new("tui");
    app.loadouts_state.reopen();
    let enter = settings_key(crossterm::event::KeyCode::Enter);
    // The first look after the open selects the first row and fills the text.
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Down)));
    assert_eq!(session.loadouts.selected(), 0);
    assert_eq!(app.loadouts_state.name_scratch, "a");

    app.loadouts_state.row = 0;
    assert!(session.loadouts_key(&mut app, enter));
    assert_eq!(session.loadouts.selected(), 1);
    assert_eq!(app.loadouts_state.name_scratch, "b");
    assert!(session.loadouts_key(&mut app, enter));
    assert_eq!(session.loadouts.selected(), 0, "the selection wraps");
    assert_eq!(app.loadouts_state.name_scratch, "a");

    // Esc closes the popup; a closed popup takes no key.
    assert!(session.loadouts_key(&mut app, settings_key(crossterm::event::KeyCode::Esc)));
    assert!(!session.loadouts_key(&mut app, enter));
}

#[test]
fn tui_manual_walk_pause_preference_projects_and_reports_persistence() {
    let iso = IsolatedStores::enter("tui-manual-walk-pause");
    let path = frontend_core::scripts::paths::bot_home()
        .join(".274bot")
        .join("panel-ui.json");
    crate::test_support::write_private_fixture(
        &path,
        r#"{"capture":false,"nav":{"pause_script_on_manual_walk_abort":false,"show_special_areas":true}}"#,
    )
    .unwrap();
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    Seam::restore_preferences(&mut app, session.preferences.share());

    assert!(!app.pause_script_on_manual_walk_abort);

    app.pause_script_on_manual_walk_abort = true;
    app.pause_script_on_manual_walk_abort_dirty = true;
    settle_preferences(&mut session, &mut app);
    let prefs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session.preferences.path()).unwrap()).unwrap();
    assert_eq!(prefs["capture"], false);
    assert_eq!(prefs["nav"]["show_special_areas"], true);
    assert_eq!(prefs["nav"]["pause_script_on_manual_walk_abort"], true);
    assert!(!app.pause_script_on_manual_walk_abort_dirty);

    // A refused preferences file (larger than the bound) refuses writes:
    // the failure surfaces.
    std::fs::write(
        session.preferences.path(),
        vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
    )
    .unwrap();
    assert!(session.preferences.reload().is_err());
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
    let _iso = iso;
}

/// The settings popup's walk permissions reach the shared preferences
/// through `SetNavPreference` and `SetDangerLevel`: a grant, the danger
/// routing level (the middle level cycles to Always) and a notice
/// acknowledgement.
#[test]
fn walk_permission_changes_in_the_settings_popup_are_saved_through_the_commands() {
    let iso = IsolatedStores::enter("tui-walk-globals-commands");
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    Seam::restore_preferences(&mut app, session.preferences.share());

    app.settings_state.open = true;
    let enter = settings_key(crossterm::event::KeyCode::Enter);
    app.settings_state.row = 5;
    assert_eq!(app.on_key(enter), AppAction::None);
    app.settings_state.row = 8;
    assert_eq!(app.on_key(enter), AppAction::None);
    assert_eq!(
        app.on_key(settings_key(crossterm::event::KeyCode::Char('d'))),
        AppAction::None
    );
    settle_preferences(&mut session, &mut app);
    assert_eq!(app.error, None);
    let prefs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session.preferences.path()).unwrap()).unwrap();
    assert_eq!(prefs["nav"]["allow_teleports"], true);
    assert_eq!(prefs["nav"]["allow_danger_zones"], true);
    assert_eq!(prefs["nav"]["survivable_routing"], false);
    assert_eq!(prefs["nav"]["script_scope_notice_ack"], true);
    assert!(app.walk_permissions.globals().allow_teleports());
    assert_eq!(
        app.walk_permissions.danger_level(),
        frontend_core::DangerLevel::Always
    );
    assert!(app.walk_permissions.script_scope_notice_ack());
    let _iso = iso;
}

#[test]
fn tui_refresh_preserves_unsaved_checkbox_and_folder_draft() {
    let _iso = IsolatedStores::enter("tui-preference-draft-refresh");
    let session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    Seam::restore_preferences(&mut app, session.preferences.share());
    app.settings_state.open = true;
    app.settings_state.row = 5;
    assert_eq!(
        app.on_key(settings_key(crossterm::event::KeyCode::Enter)),
        AppAction::None,
    );
    assert!(app.nav.allow_teleports());
    app.settings_state.row = 13;
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    app.on_key(settings_key(crossterm::event::KeyCode::Char('x')));
    let folder = app.quester_paths.folder().to_path_buf();
    for _ in 0..4 {
        app.refresh_walk_permissions();
        assert!(
            app.nav.allow_teleports(),
            "an unchanged durable snapshot must not erase the checkbox draft"
        );
        assert_eq!(app.quester_paths.folder(), folder);
    }
    app.settings_state.open = false;
    app.settings_state.open = true;
    assert!(app.nav.allow_teleports());
    assert_eq!(app.quester_paths.folder(), folder);
}

#[test]
fn pending_preferences_survive_settings_reopen_and_keep_the_folder_draft() {
    let _iso = IsolatedStores::enter("tui-preference-pending-reopen");
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    Seam::restore_preferences(&mut app, session.preferences.share());
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    session
        .core
        .local_state_for_test()
        .unwrap()
        .set_write_stall_for_test(Arc::new(move || {
            let _ = entered.send(());
            let _ = release_rx.lock().unwrap().recv();
        }));
    app.settings_state.open = true;
    app.settings_state.row = 5;
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    session.pump(&mut app);
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(app.settings_state.preferences_pending);
    assert!(!app.walk_permissions.globals().allow_teleports());
    app.settings_state.row = 13;
    app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    app.on_key(settings_key(crossterm::event::KeyCode::Char('x')));
    let folder = app.quester_paths.folder().to_path_buf();
    for _ in 0..4 {
        session.pump(&mut app);
        assert!(app.nav.allow_teleports());
        assert_eq!(app.quester_paths.folder(), folder);
    }
    app.settings_state.open = false;
    session.pump(&mut app);
    app.settings_state.open = true;
    for (width, height) in [(120, 40), (80, 24)] {
        let shown = crate::test_support::text(&crate::test_support::draw(&mut app, width, height));
        assert!(shown.contains("Saving preferences"), "{shown}");
        if let Some(dir) = std::env::var_os("TUI_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            super::manual_click_live_tests::capture(
                &mut app,
                width,
                height,
                std::path::Path::new(&dir),
                &format!("{width}x{height}"),
                "preferences-pending-reopened",
                &serde_json::json!({
                    "preferences_pending": true,
                    "allow_teleports_draft": true,
                    "folder_draft": folder,
                }),
            )
            .unwrap();
        }
    }
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.core.preferences_pending() && Instant::now() < deadline {
        session.pump(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!session.core.preferences_pending());
    assert!(app.walk_permissions.globals().allow_teleports());
    assert_eq!(app.quester_paths.folder(), folder);
    assert!(!app.settings_state.preferences_pending);
    if let Some(dir) = std::env::var_os("TUI_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [(120, 40), (80, 24)] {
            super::manual_click_live_tests::capture(
                &mut app,
                width,
                height,
                std::path::Path::new(&dir),
                &format!("{width}x{height}"),
                "preferences-settled-draft-kept",
                &serde_json::json!({
                    "preferences_pending": false,
                    "allow_teleports_durable": true,
                    "folder_draft": folder,
                }),
            )
            .unwrap();
        }
    }
}

/// Quit goes through the `Quit` command: every loaded member's Quit
/// settles and the play session is closed before the loop ends.
#[test]
fn quit_closes_the_play_through_the_quit_command() {
    let iso = IsolatedStores::enter("tui-quit-command");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    assert!(session.core.has_play());
    dispatch(&mut session, &mut app, AppAction::Quit);
    assert!(app.quit, "the event loop ends");
    assert_eq!(app.error, None);
    assert!(!session.core.has_play(), "Quit closed the play session");
}

/// Nothing loaded is not an error when quitting.
#[test]
fn quit_with_nothing_loaded_is_not_an_error() {
    let iso = IsolatedStores::enter("tui-quit-nothing");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    dispatch(&mut session, &mut app, AppAction::Quit);
    assert!(app.quit);
    assert_eq!(app.error, None);
    let _iso = iso;
}

/// Log in marked and log out marked are the `Login` and `Logout` rows over
/// the marks: the strip shows the command's per-row report, and a member
/// that is not marked is not named.
#[test]
fn marked_login_and_logout_show_the_report_of_the_command_over_the_marks() {
    let iso = IsolatedStores::enter("tui-marked-login-logout-command");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let alice = session.core.profile_identity_or_synthetic("alice");
    app.fleet_model.set_mark_for_test(alice, true);

    dispatch(
        &mut session,
        &mut app,
        AppAction::PlanFleet(frontend_core::FleetAction::Login),
    );
    assert!(app.modal.is_none(), "a marked Log in is one step");
    let report = app.error.clone().expect("the login result is shown");
    assert!(report.starts_with("Log in: "), "{report}");
    assert!(report.contains("alice"), "{report}");
    assert!(!report.contains("bob"), "bob is not marked: {report}");

    dispatch(
        &mut session,
        &mut app,
        AppAction::PlanFleet(frontend_core::FleetAction::Logout),
    );
    let report = app.error.clone().expect("the logout result is shown");
    assert!(report.starts_with("Log out: "), "{report}");
    assert!(!report.contains("bob"), "bob is not marked: {report}");
}

/// Publish `name` as a bot in game on public world `world`, as its slot
/// does: into the host's shared status rows. The next pump projects it.
fn publish_on_world(session: &TuiSession, name: &str, world: u16) {
    let mut rows = session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap();
    rows.retain(|row| row.username != name);
    rows.push(host_play::SlotStatus {
        username: name.into(),
        world: Some(world),
        connected: true,
        ingame: true,
        scene_state: 2,
        ..host_play::SlotStatus::default()
    });
}

/// A session whose loaded `names` are in game on world 2 of a server with
/// worlds 1, 2 and 3, and the app after one pump.
fn tui_on_worlds(iso: &IsolatedStores, names: &[&str]) -> (TuiSession, TuiApp) {
    let (mut session, mut app) = tui_with_profiles(iso, names);
    session.core.set_public_worlds_for_test(&[1, 2, 3]);
    for name in names {
        publish_on_world(&session, name, 2);
    }
    session.pump(&mut app);
    (session, app)
}

/// Move the fleet cursor onto `name`'s row.
fn cursor_on(app: &mut TuiApp, name: &str) {
    let position = app
        .fleet_model
        .shown_rows()
        .position(|(_, row)| row.name() == name)
        .expect("the row is shown");
    app.table.sync(&app.fleet_model);
    app.table.cursor_to(position, &app.fleet_model);
}

/// The members of every Switch world command to `world` in the command
/// book, one list of subjects per command.
fn switch_world_members(
    session: &TuiSession,
    world: u16,
) -> Vec<Vec<frontend_core::command::Subject>> {
    session
        .core
        .commands()
        .reports()
        .filter(|report| report.command() == OperatorCommand::SwitchWorld(world))
        .map(|report| {
            report
                .members()
                .iter()
                .map(|member| member.subject())
                .collect()
        })
        .collect()
}

/// The world `name`'s vault row saves for its next login.
fn saved_world(session: &TuiSession, name: &str) -> Option<u16> {
    session
        .core
        .profile(name)
        .expect("a vault profile")
        .settings_draft()
        .world()
}

/// `W` opens the picker over the worlds core cached when the play started
/// (the server's list, nothing fetched) with the bot's own world ticked: the
/// cursor row's, not the selected bot's.
#[test]
fn w_opens_the_picker_over_the_cached_worlds_with_the_bots_own_world_ticked() {
    use crate::test_support::{ch, draw, key, text};
    let iso = IsolatedStores::enter("tui-world-picker-lists");
    let (mut session, mut app) = tui_on_worlds(&iso, &["alice", "bob"]);
    publish_on_world(&session, "bob", 3);
    session.pump(&mut app);
    assert_eq!(
        session
            .core
            .switch_world_choices()
            .expect("a public server")
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );

    let action = app.on_key(ch('W'));
    assert_eq!(action, AppAction::PickWorld(Some("alice".into())));
    dispatch(&mut session, &mut app, action);
    assert!(matches!(&app.modal, Some(Modal::WorldPicker(_))));
    let shown = text(&draw(&mut app, 120, 40));
    for want in [
        "Switch world of alice",
        "[ ] World 1",
        "[x] World 2",
        "[ ] World 3",
    ] {
        assert!(shown.contains(want), "{want}: {shown}");
    }
    assert_eq!(
        app.on_key(key(crossterm::event::KeyCode::Esc)),
        AppAction::None
    );

    // On bob's row the ticked world is his own, 3.
    cursor_on(&mut app, "bob");
    let action = app.on_key(ch('W'));
    assert_eq!(action, AppAction::PickWorld(Some("bob".into())));
    dispatch(&mut session, &mut app, action);
    let shown = text(&draw(&mut app, 120, 40));
    assert!(shown.contains("Switch world of bob"), "{shown}");
    assert!(shown.contains("[x] World 3"), "{shown}");
    assert!(!shown.contains("[x] World 2"), "{shown}");
    assert_eq!(app.error, None);
}

/// A server with one world (a local one) has no switching: `W` and the
/// palette entry say so on the message line and open nothing.
#[test]
fn a_one_world_server_offers_no_picker() {
    use crate::test_support::ch;
    let iso = IsolatedStores::enter("tui-world-picker-local");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    publish_on_world(&session, "alice", 2);
    session.pump(&mut app);
    assert!(session.core.switch_world_choices().is_none());

    let action = app.on_key(ch('W'));
    assert_eq!(action, AppAction::PickWorld(Some("alice".into())));
    dispatch(&mut session, &mut app, action);
    assert!(app.modal.is_none());
    assert_eq!(
        app.error.take().as_deref(),
        Some("Switch world: this server has one world")
    );
    let action = app.run_command(crate::commands::Command::PickWorldMarked);
    assert_eq!(action, AppAction::PickWorld(None));
    dispatch(&mut session, &mut app, action);
    assert!(app.modal.is_none());
    assert_eq!(
        app.error.take().as_deref(),
        Some("Switch world: this server has one world")
    );
    assert!(session.core.fleet_plan().is_none());
}

/// Picking the world the bot is already on submits nothing: no plan, no
/// command, no write, and no message.
#[test]
fn picking_the_bots_own_world_submits_nothing() {
    use crate::test_support::{ch, key};
    let iso = IsolatedStores::enter("tui-world-picker-same");
    let (mut session, mut app) = tui_on_worlds(&iso, &["alice"]);
    let commands = session.core.commands().reports().count();

    let action = app.on_key(ch('W'));
    dispatch(&mut session, &mut app, action);
    assert!(matches!(&app.modal, Some(Modal::WorldPicker(_))));
    // The cursor starts on the ticked world: Enter picks it.
    let action = app.on_key(key(crossterm::event::KeyCode::Enter));
    assert_eq!(action, AppAction::None);
    dispatch(&mut session, &mut app, action);

    assert!(app.modal.is_none());
    assert!(session.core.fleet_plan().is_none());
    assert_eq!(session.core.commands().reports().count(), commands);
    session.core.flush_writes();
    assert_eq!(saved_world(&session, "alice"), None);
    assert_eq!(app.error, None);
}

/// Picking another world stages the cursor's bot alone (a marked bot is not
/// added) and plans a Switch world for exactly that bot. The confirm
/// overlay shows core's prompt; nothing runs and nothing is saved before
/// Confirm; Cancel writes nothing; Confirm runs one Switch world command
/// over that bot only, and the message line shows its result.
#[test]
fn picking_another_world_plans_that_bot_and_runs_only_on_confirm() {
    use crate::test_support::{ch, draw, key, text};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-world-picker-plan");
    let (mut session, mut app) = tui_on_worlds(&iso, &["alice", "bob", "carol"]);
    // bob is marked, but a world marker's pick is about the cursor row.
    let bob = session.core.profile_identity("bob").unwrap();
    app.fleet_model.set_mark_for_test(bob, true);
    let alice = session.core.profile_identity("alice").unwrap();

    let pick_world_3 = |session: &mut TuiSession, app: &mut TuiApp| {
        let action = app.on_key(ch('W'));
        dispatch(session, app, action);
        // The highlight starts on World 2, alice's own: one Down is World 3.
        app.on_key(key(KeyCode::Down));
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(action, AppAction::PlanSwitchWorld(Some("alice".into()), 3));
        dispatch(session, app, action);
    };

    pick_world_3(&mut session, &mut app);
    let plan = session.core.fleet_plan().expect("the plan waits");
    assert_eq!(plan.action(), frontend_core::FleetAction::SwitchWorld(3));
    assert!(plan.needs_confirm(), "a Switch world plan always asks");
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice"]);
    let prompt = plan.prompt().to_owned();
    assert!(
        prompt.starts_with("Move 1 bot to world 3: alice."),
        "{prompt}"
    );
    assert!(!prompt.contains("bob"), "the mark did not join: {prompt}");
    assert!(
        matches!(
            &app.modal,
            Some(Modal::Confirm(confirm)) if matches!(
                &confirm.kind,
                ConfirmKind::FleetPlan {
                    action: frontend_core::FleetAction::SwitchWorld(3),
                    prompt: shown,
                } if shown == &prompt
            )
        ),
        "the confirm overlay shows core's prompt: {:?}",
        app.modal
    );
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("Switch world"), "{screen}");
    assert!(screen.contains("Move 1 bot to world 3: alice."), "{screen}");
    assert!(
        switch_world_members(&session, 3).is_empty(),
        "nothing ran before Confirm"
    );

    // Esc cancels: the plan goes and nothing was written.
    let action = app.on_key(key(KeyCode::Esc));
    assert_eq!(action, AppAction::CancelFleetPlan);
    dispatch(&mut session, &mut app, action);
    assert!(session.core.fleet_plan().is_none());
    session.core.flush_writes();
    assert!(switch_world_members(&session, 3).is_empty());
    assert_eq!(saved_world(&session, "alice"), None);

    // Again, and Enter confirms: one command, one member, alice.
    pick_world_3(&mut session, &mut app);
    let action = app.on_key(key(KeyCode::Enter));
    assert!(matches!(action, AppAction::RunFleetPlan(_)), "{action:?}");
    dispatch(&mut session, &mut app, action);
    assert!(session.core.fleet_plan().is_none(), "the plan ran");
    assert_eq!(
        switch_world_members(&session, 3),
        [[frontend_core::command::Subject::Profile(alice)]]
    );
    session.core.flush_writes();
    assert_eq!(
        saved_world(&session, "alice"),
        Some(3),
        "the world is saved before the relog"
    );
    assert_eq!(saved_world(&session, "bob"), None);
    assert_eq!(saved_world(&session, "carol"), None);
    // The relog has not finished: the one member is pending, and the
    // message line shows core's one result.
    let result = session.core.fleet_result().expect("one result");
    assert_eq!(result.action(), frontend_core::FleetAction::SwitchWorld(3));
    assert_eq!(result.counts().pending, 1, "{}", result.text());
    assert!(
        result.text().starts_with("Switch world: "),
        "{}",
        result.text()
    );
    assert_eq!(app.error.as_deref(), Some(result.text()));
}

/// The palette's Switch world of marked… opens the same picker with no
/// profile staged: the plan covers the marked bots and no other.
#[test]
fn the_marked_world_action_plans_over_the_marks() {
    use crate::test_support::{key, text};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-world-picker-marked");
    let (mut session, mut app) = tui_on_worlds(&iso, &["alice", "bob", "carol"]);
    let alice = session.core.profile_identity("alice").unwrap();
    let carol = session.core.profile_identity("carol").unwrap();
    app.fleet_model.set_mark_for_test(alice, true);
    app.fleet_model.set_mark_for_test(carol, true);

    let action = app.run_command(crate::commands::Command::PickWorldMarked);
    assert_eq!(action, AppAction::PickWorld(None));
    dispatch(&mut session, &mut app, action);
    let shown = text(&crate::test_support::draw(&mut app, 120, 40));
    assert!(shown.contains("Switch world of marked"), "{shown}");
    assert!(!shown.contains("[x] World"), "no world is ticked: {shown}");
    // World 1 is first; two Downs reach World 3.
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Down));
    let action = app.on_key(key(KeyCode::Enter));
    assert_eq!(action, AppAction::PlanSwitchWorld(None, 3));
    dispatch(&mut session, &mut app, action);

    let plan = session.core.fleet_plan().expect("the plan waits");
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice", "carol"]);
    assert!(
        plan.prompt()
            .starts_with("Move 2 marked bots to world 3: alice, carol."),
        "{}",
        plan.prompt()
    );
    assert!(switch_world_members(&session, 3).is_empty());
    let action = app.on_key(key(KeyCode::Enter));
    dispatch(&mut session, &mut app, action);
    let members = switch_world_members(&session, 3);
    assert_eq!(members.len(), 1, "one command");
    let mut subjects = members.into_iter().next().unwrap();
    subjects.sort_by_key(|subject| format!("{subject:?}"));
    let mut want = vec![
        frontend_core::command::Subject::Profile(alice),
        frontend_core::command::Subject::Profile(carol),
    ];
    want.sort_by_key(|subject| format!("{subject:?}"));
    assert_eq!(subjects, want, "the marked bots, and not bob");
}

/// A plan core refuses ("no bot to act on") shows its reason on the message
/// line, as every fleet plan refusal does; no confirm overlay opens and
/// nothing runs.
#[test]
fn a_refused_world_plan_shows_core_s_reason_on_the_message_line() {
    use crate::test_support::{ch, key};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-world-picker-refused");
    let (mut session, mut app) = tui_with_loaded(&iso, &["alice", "bob"], &["bob"]);
    session.core.set_public_worlds_for_test(&[1, 2, 3]);
    publish_on_world(&session, "bob", 2);
    session.pump(&mut app);
    // alice is a vault profile that is not loaded: there is no bot to move.
    cursor_on(&mut app, "alice");

    let action = app.on_key(ch('W'));
    dispatch(&mut session, &mut app, action);
    assert!(matches!(&app.modal, Some(Modal::WorldPicker(_))));
    let action = app.on_key(key(KeyCode::Enter));
    assert_eq!(action, AppAction::PlanSwitchWorld(Some("alice".into()), 1));
    dispatch(&mut session, &mut app, action);

    assert!(app.modal.is_none(), "a refusal opens no confirmation");
    assert_eq!(
        app.error.as_deref(),
        Some("Switch world: no bot to act on (alice: not loaded)")
    );
    assert!(session.core.fleet_plan().is_none());
    assert!(switch_world_members(&session, 1).is_empty());
}

/// Save log runs through `SaveLog`: the write happens off the UI thread
/// and the log pane's status follows it to the saved file.
#[test]
fn save_log_runs_through_the_command_and_the_pane_follows_the_write() {
    let iso = IsolatedStores::enter("tui-save-log-command");
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    dispatch(&mut session, &mut app, AppAction::SaveLog);
    assert_eq!(app.log.status.as_deref(), Some("saving…"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.log.status.as_deref() == Some("saving…") {
        assert!(Instant::now() < deadline, "the log save did not finish");
        app.log.refresh(None);
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = app.log.status.clone().expect("a save status");
    let saved = status
        .strip_prefix("saved ")
        .unwrap_or_else(|| panic!("the save failed: {status}"));
    assert!(std::path::Path::new(saved).is_file(), "{status}");
    let _iso = iso;
}

/// The session-file toggle is saved and applied by `SetSessionLog`, and the
/// log pane preserves its prior status while storage is pending.
#[test]
fn session_file_toggle_is_saved_and_applied_through_the_command() {
    let _lock = session_file_lock();
    let _iso = IsolatedStores::enter("tui-session-log-command");
    frontend_core::log_file::apply_session_log_for_test(false);
    let mut session = TuiSession::new(dummy_options());
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    dispatch(&mut session, &mut app, AppAction::SetSessionLog(false));
    settle_preferences(&mut session, &mut app);
    assert!(!frontend_core::log_file::session_log_setting(
        &session.preferences
    ));
    assert!(frontend_core::log_file::session_log_path().is_none());

    let (write_entered_tx, write_entered_rx) = std::sync::mpsc::sync_channel(2);
    let (write_release_tx, write_release_rx) = std::sync::mpsc::channel();
    let write_release_rx = std::sync::Mutex::new(write_release_rx);
    let stalled_writes = std::sync::atomic::AtomicUsize::new(0);
    session
        .core
        .local_state_for_test()
        .unwrap()
        .set_write_stall_for_test(Arc::new(move || {
            if stalled_writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                write_entered_tx
                    .send(())
                    .expect("the pending write has a test observer");
                write_release_rx
                    .lock()
                    .unwrap()
                    .recv()
                    .expect("the test releases the pending write");
            }
        }));

    let current_sink_status = || {
        frontend_core::log_file::session_log_path()
            .map(|path| format!("session file {}", path.display()))
            .unwrap_or_else(|| "session file off".to_owned())
    };
    for (on, was_on) in [(true, false), (false, true)] {
        let sink_status_before_settlement = current_sink_status();
        assert_eq!(
            frontend_core::log_file::session_log_setting(&session.preferences),
            was_on
        );
        assert_eq!(
            frontend_core::log_file::session_log_path().is_some(),
            was_on
        );

        app.log.status = Some(format!("prior status before setting session file {on}"));
        dispatch(&mut session, &mut app, AppAction::SetSessionLog(on));
        write_entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the preference write reached the held storage seam");
        assert!(session.core.preferences_pending());
        assert_eq!(
            app.log.status.as_deref(),
            Some("session file: saving…"),
            "the status line says the choice is saving while its write is pending"
        );
        assert_eq!(current_sink_status(), sink_status_before_settlement);
        assert_eq!(
            frontend_core::log_file::session_log_setting(&session.preferences),
            was_on
        );

        write_release_tx
            .send(())
            .expect("the preference writer is still held");
        settle_preferences(&mut session, &mut app);
        assert_eq!(
            frontend_core::log_file::session_log_setting(&session.preferences),
            on
        );
        assert_eq!(frontend_core::log_file::session_log_path().is_some(), on);
        let settled_deadline = Instant::now() + Duration::from_secs(10);
        while app.log.status.as_deref().is_some_and(|status| {
            status.starts_with("session file opening") || status.ends_with("saving…")
        }) {
            assert!(Instant::now() < settled_deadline, "the sink never settled");
            session.pump(&mut app);
            std::thread::yield_now();
        }
        let settled_status = current_sink_status();
        assert_eq!(app.log.status.as_deref(), Some(settled_status.as_str()));
    }
}

/// The process has one session file: the tests that open, fail or close it
/// run one at a time.
fn session_file_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Pump `session` until the Logs status line says `wanted`.
fn pump_until_status(session: &mut TuiSession, app: &mut TuiApp, wanted: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !app.log.status.as_deref().is_some_and(&wanted) {
        assert!(
            Instant::now() < deadline,
            "the Logs status never said it: {:?}",
            app.log.status
        );
        session.pump(app);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The failure leaves with the shared error timeout, from the setting and
/// from the Logs status line alike, and nothing replaces it.
fn expire_session_file_failure(session: &mut TuiSession, app: &mut TuiApp) {
    let after = Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT;
    {
        let (core, mut surface) = session.core_and_surface_for_preferences(app);
        core.poll_preferences_at(&mut surface, after);
    }
    session.project_setting_feedback(app);
    session.refresh_session_log_status(app);
    assert_eq!(
        app.log.status, None,
        "the failure left; nothing replaced it"
    );
    for _ in 0..3 {
        session.pump(app);
    }
    assert_eq!(app.log.status, None, "a later frame shows nothing either");
}

/// A session file that was on at startup (saved before this session read
/// its preferences, no toggle of this terminal behind it) and could not be
/// opened: the Logs status line shows the core's failure, and the failure
/// leaves with the shared error timeout instead of being replaced by a
/// permanent line.
#[test]
fn settings_feedback_tui_session_file_failure_from_a_choice_made_before_this_session_shows_and_expires(
) {
    let _lock = session_file_lock();
    frontend_core::log_file::apply_session_log_for_test(false);
    let iso = IsolatedStores::enter("tui-s3-sink-startup");
    // A file stands where the logs folder must go: the open cannot succeed.
    std::fs::create_dir_all(iso.home().join(".274bot")).unwrap();
    std::fs::write(iso.home().join(".274bot").join("logs"), b"not a folder").unwrap();
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    session.preferences.reload().unwrap();
    session
        .preferences
        .set(
            frontend_core::log_file::SESSION_LOG_KEY,
            serde_json::Value::Bool(true),
        )
        .unwrap();
    pump_until_status(&mut session, &mut app, |status| {
        status.contains("could not be opened or written")
    });
    assert!(
        matches!(
            app.setting_feedback(Setting::SessionLog),
            Some(SettingFeedback::Failed(text)) if Some(text.as_str()) == app.log.status.as_deref()
        ),
        "the Logs line shows the core's failure: {:?}",
        app.log.status
    );
    expire_session_file_failure(&mut session, &mut app);
    frontend_core::log_file::apply_session_log_for_test(false);
}

/// A session file that opened and wrote, then failed: the Logs status line
/// follows the sink past its earlier success, and the failure leaves with
/// the shared error timeout.
#[test]
fn settings_feedback_tui_session_file_failure_after_the_file_was_writing_shows_and_expires() {
    let _lock = session_file_lock();
    frontend_core::log_file::apply_session_log_for_test(false);
    let iso = IsolatedStores::enter("tui-s3-sink-after-writing");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    session.preferences.reload().unwrap();
    dispatch(&mut session, &mut app, AppAction::SetSessionLog(true));
    settle_preferences(&mut session, &mut app);
    pump_until_status(&mut session, &mut app, |status| {
        status.starts_with("session file ") && !status.contains("opening")
    });
    assert_eq!(
        frontend_core::log_file::session_file_phase(),
        frontend_core::log_file::SessionFilePhase::Writing
    );
    // Steady: a pump compares the stage and builds no text. The status
    // line's string is the one first built, frame after frame.
    let steady = app.log.status.as_ref().map(|status| status.as_ptr());
    assert!(steady.is_some());
    for _ in 0..200 {
        session.pump(&mut app);
    }
    assert_eq!(
        app.log.status.as_ref().map(|status| status.as_ptr()),
        steady
    );
    frontend_core::log_file::fail_session_sink_for_test("no space left on device");
    pump_until_status(&mut session, &mut app, |status| {
        status.contains("no space left on device")
    });
    expire_session_file_failure(&mut session, &mut app);
    frontend_core::log_file::apply_session_log_for_test(false);
}

/// A disk for one TUI session's service: every file write waits at its
/// start until released and fails while `failing` is set.
struct TuiDisk {
    release: std::sync::mpsc::Sender<()>,
    failing: Arc<std::sync::atomic::AtomicBool>,
}

impl TuiDisk {
    fn install(session: &TuiSession) -> Self {
        let service = session
            .core
            .local_state_for_test()
            .expect("the TUI binds the process state service");
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        service.set_write_stall_for_test(Arc::new(move || {
            let _ = gate.lock().unwrap().recv();
        }));
        let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let refusing = Arc::clone(&failing);
        service.set_write_fault_for_test(Arc::new(move || {
            refusing.load(std::sync::atomic::Ordering::SeqCst)
        }));
        Self { release, failing }
    }
}

/// The settings popup's "map bake" row (a remembered choice) at both sizes:
/// it keeps the saved value and says "saving…" while its write is held,
/// then shows the failure on its own row with the saved value again; the
/// retry is the action that resolves it and the saved value then shows.
#[test]
fn settings_feedback_tui_row_shows_saving_then_the_failure_then_clears_on_retry() {
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-s3-row-feedback");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    session.preferences.reload().unwrap();
    app.restore_preferences(session.preferences.share());
    open_settings(&mut session, &mut app);
    app.settings_state.row = 10;
    let disk = TuiDisk::install(&session);
    settings_press(&mut session, &mut app, KeyCode::Enter);
    session.pump(&mut app);
    assert!(session.core.preferences_pending());
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            words.contains("map bake: always — saving…"),
            "{width}x{height}: the row shows the choice and says it is saving: {words}"
        );
        assert_eq!(
            session.map_desk.bake_gate().choice(),
            frontend_core::MapBakeChoice::Ask,
            "{width}x{height}: the applied choice is still the saved one"
        );
    }

    // The disk fails the held write.
    drop(disk.release);
    let deadline = Instant::now() + Duration::from_secs(30);
    while session.core.preferences_pending() {
        session.pump(&mut app);
        assert!(Instant::now() < deadline, "the held write never ended");
        std::thread::yield_now();
    }
    session.pump(&mut app);
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            words.contains("map bake: ask") && words.contains("Not saved:"),
            "{width}x{height}: the failure shows on the row: {words}"
        );
        assert!(
            !words.contains("saving…"),
            "{width}x{height}: nothing is saving any more: {words}"
        );
    }
    assert_eq!(app.map_bake, frontend_core::MapBakeChoice::Ask);

    // The retry resolves the failure; the durable value then shows.
    disk.failing
        .store(false, std::sync::atomic::Ordering::SeqCst);
    settings_press(&mut session, &mut app, KeyCode::Enter);
    session.pump(&mut app);
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            !words.contains("Not saved:"),
            "{width}x{height}: the retry cleared the failure: {words}"
        );
    }
    settle_preferences(&mut session, &mut app);
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            words.contains("map bake: always") && !words.contains("saving…"),
            "{width}x{height}: the saved choice shows: {words}"
        );
    }
}

/// A real file-lock timeout leaves the saved grant unchanged. A second
/// operator action after the lock is released must reach durable storage.
#[test]
fn settings_feedback_tui_teleports_retry_after_file_lock_timeout_is_durable() {
    use crossterm::event::KeyCode;
    use frontend_core::{NavPreference, Setting, SettingStatus};

    let iso = IsolatedStores::enter("tui-s3-lock-timeout-retry");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    session.preferences.reload().unwrap();
    app.restore_preferences(session.preferences.share());
    settle_preferences(&mut session, &mut app);
    open_settings(&mut session, &mut app);
    app.settings_state.row = 5;
    let setting = Setting::Nav(NavPreference::AllowTeleports);
    let path = session.preferences.path();
    let lock_path = path.with_extension("json.lock");
    crate::test_support::write_private_fixture(&lock_path, b"").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();

    settings_press(&mut session, &mut app, KeyCode::Enter);
    session.pump(&mut app);
    assert!(matches!(
        session.core.setting_status(setting),
        SettingStatus::Pending
    ));
    settle_preferences(&mut session, &mut app);
    assert!(
        matches!(session.core.setting_status(setting), SettingStatus::Failed(text) if text.contains("busy")),
        "the real writer must time out while the file lock is held"
    );
    assert!(!app.walk_permissions.globals().allow_teleports());
    assert!(!app.nav.allow_teleports());

    drop(lock);
    settings_press(&mut session, &mut app, KeyCode::Enter);
    settle_preferences(&mut session, &mut app);
    assert!(matches!(
        session.core.setting_status(setting),
        SettingStatus::Saved
    ));
    assert!(app.walk_permissions.globals().allow_teleports());
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(words.contains("allow teleports: true"), "{words}");
        assert!(!words.contains("saving…"), "{words}");
        assert!(!words.contains("Not saved:"), "{words}");
    }
    let durable: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(durable["nav"]["allow_teleports"], true);
}

/// A failed row feedback leaves by itself after the error timeout.
#[test]
fn settings_feedback_tui_row_failure_expires_by_itself() {
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-s3-row-expiry");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    session.preferences.reload().unwrap();
    app.restore_preferences(session.preferences.share());
    open_settings(&mut session, &mut app);
    app.settings_state.row = 10;
    let disk = TuiDisk::install(&session);
    drop(disk.release);
    settings_press(&mut session, &mut app, KeyCode::Enter);
    let deadline = Instant::now() + Duration::from_secs(30);
    while session
        .core
        .setting_failure(frontend_core::Setting::MapBake)
        .is_none()
    {
        session.pump(&mut app);
        assert!(Instant::now() < deadline, "the failure never came");
        std::thread::yield_now();
    }
    session.pump(&mut app);
    assert!(screen_words(&mut app, 120, 40).contains("Not saved:"));
    let after = Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT;
    {
        let (core, mut surface) = session.core_and_surface_for_preferences(&mut app);
        core.poll_preferences_at(&mut surface, after);
    }
    session.project_setting_feedback(&mut app);
    let words = screen_words(&mut app, 120, 40);
    assert!(!words.contains("Not saved:"), "{words}");
    assert!(words.contains("map bake: ask"), "{words}");
}

/// A cold TUI (preferences not yet read) adopts every saved choice when the
/// read is released, and writes no default over the file meanwhile.
#[test]
fn settings_feedback_tui_cold_start_adopts_saved_choices_without_writing_defaults() {
    let iso = IsolatedStores::enter("tui-s3-cold");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();
    let state = iso.dir().join("cold-state");
    let saved: &[u8] = br#"{"map_bake":"always","nav":{"allow_teleports":true}}"#;
    let path = host_play::local_state::StateRoot::new(state.clone()).preferences_file();
    crate::test_support::write_private_fixture(&path, saved).unwrap();
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(state),
        Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    let mut app = new_app("tui", &session.preferences);
    for _ in 0..4 {
        session.pump(&mut app);
        assert!(!session.preferences.ready());
        assert_eq!(app.map_bake, frontend_core::MapBakeChoice::Ask);
        assert!(!app.nav.allow_teleports());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            saved,
            "no default is written"
        );
    }
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(app.nav.allow_teleports() && app.map_bake == frontend_core::MapBakeChoice::Always)
        && Instant::now() < deadline
    {
        session.pump(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.nav.allow_teleports());
    assert_eq!(app.map_bake, frontend_core::MapBakeChoice::Always);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        saved,
        "no default is written"
    );
}

/// Dismissing the Quester Paths status line is the `AcknowledgeNotice`
/// command: a refused reload leaves its reason as the notice, dismissing
/// clears it.
#[test]
fn dismissing_the_paths_notice_clears_it_through_the_acknowledge_command() {
    let iso = IsolatedStores::enter("tui-paths-notice-command");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();

    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.restore_preferences(session.preferences.share());
    dispatch(&mut session, &mut app, AppAction::ReloadPaths);
    assert!(
        app.quester_paths_controller.notice_is_error(),
        "no game data is loaded: {:?}",
        app.quester_paths_controller.notice_text()
    );
    dispatch(&mut session, &mut app, AppAction::DismissPathsNotice);
    assert_eq!(app.quester_paths_controller.notice_text(), None);
    let _iso = iso;
}

#[test]
fn tui_quester_paths_failed_save_keeps_source_and_error_without_reloading() {
    let iso = IsolatedStores::enter("tui-quester-paths-save-failure");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();

    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("tui");
    app.restore_preferences(session.preferences.share());
    // A refused preferences file (larger than the bound) refuses writes:
    // the save fails, the source stays, and no reload starts.
    std::fs::create_dir_all(session.preferences.path().parent().unwrap()).unwrap();
    std::fs::write(
        session.preferences.path(),
        vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
    )
    .unwrap();
    assert!(session.preferences.reload().is_err());
    let before_enabled = app.quester_paths.enabled();
    let before_folder = app.quester_paths.folder().to_path_buf();
    app.quester_paths.set_enabled(true);
    app.quester_paths.set_folder(iso.dir().join("paths"));
    app.quester_paths_dirty = true;

    session.project_quester_paths(&mut app);

    assert_eq!(app.quester_paths.enabled(), before_enabled);
    assert_eq!(app.quester_paths.folder(), before_folder);
    assert!(!app.quester_paths_controller.is_running());
    assert!(
        matches!(
            app.setting_feedback(frontend_core::Setting::QuesterPaths),
            Some(SettingFeedback::Failed(text)) if text.starts_with("Not saved:")
        ),
        "the failure shows at the Quest Paths setting: {:?}",
        app.setting_feedback(frontend_core::Setting::QuesterPaths)
    );
    let _iso = iso;
}

#[test]
fn tui_restored_enabled_folder_loads_when_game_data_becomes_ready() {
    let iso = IsolatedStores::enter("tui-quester-paths-startup");
    let _quester_paths_guard = crate::bin::seam::lock_quester_paths_test_state();

    let folder = iso.dir().join("paths");
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
    let prefs_path = frontend_core::scripts::paths::bot_home()
        .join(".274bot")
        .join("panel-ui.json");
    crate::test_support::write_private_fixture(
        &prefs_path,
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
    session.preferences.reload().unwrap();
    let mut app = TuiApp::new("tui");
    app.restore_preferences(session.preferences.share());
    assert!(app.quester_paths.enabled());
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

    assert!(quester_folder_path_ids().iter().any(|id| id == "cook"));
    assert!(quester_draft_path_ids()
        .iter()
        .any(|id| id == "fresh-draft"));
    let report = app
        .quester_paths_controller
        .notice_text()
        .expect("startup validation report");
    assert!(report.contains("2 folder documents"), "{report}");
    assert!(report.contains("0 validation errors"), "{report}");
    reset_quester_registry();
}

#[test]
fn tui_pump_reaps_finished_workers_logged_out_arms_still_count() {
    let iso = IsolatedStores::enter("tui-reap-finished");
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
    let play = session.core.play_for_test().unwrap();
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
    let iso = IsolatedStores::enter("tui-ack-fail");
    let mut session = TuiSession::new(dummy_options());
    let mut app = pump_two_slots(&mut session);
    assert!(app.background_notice.is_some());
    settle_preferences(&mut session, &mut app);
    // An unreadable preferences file refuses the store on every platform. (A
    // folder replaced by a file does not: a read through it fails as
    // `NotADirectory` on Unix but as `NotFound` on Windows, which the store
    // reads as "no file yet", and the write then fails later, not at once.)
    let path = session.preferences.path();
    crate::test_support::write_private_fixture(&path, b"{malformed").unwrap();
    assert!(session.preferences.reload().is_err());
    app.error = None;
    session.ack_background_bots(&mut app);
    assert!(
        app.background_notice.is_some(),
        "persist failure must keep the notice"
    );
    let notice = frontend_core::Setting::Notice(frontend_core::OperatorNotice::BackgroundBots);
    assert!(
        matches!(
            app.setting_feedback(notice),
            Some(SettingFeedback::Failed(text)) if text.starts_with("Not saved:")
        ),
        "the failure shows at the notice's setting: {:?}",
        app.setting_feedback(notice)
    );
    assert!(
        app.background_notice
            .as_deref()
            .is_some_and(|text| text.contains("Not saved:")),
        "the notice itself carries the failure: {:?}",
        app.background_notice
    );
    assert!(!frontend_core::resources::background_bots_acked(
        &session.preferences
    ));
    std::fs::remove_file(&path).unwrap();
    session.preferences.reload().unwrap();
    session.ack_background_bots(&mut app);
    settle_preferences(&mut session, &mut app);
    // The notice cache refreshes on its own one-second cadence.
    session.expire_ack_cache();
    session.pump(&mut app);
    assert!(app.background_notice.is_none());
    assert!(app.error.is_none(), "got {:?}", app.error);
    assert!(frontend_core::resources::background_bots_acked(
        &session.preferences
    ));
    let _iso = iso;
}

#[test]
fn tui_successful_ack_preserves_unrelated_error() {
    let iso = IsolatedStores::enter("tui-ack-unrelated");
    let mut session = TuiSession::new(dummy_options());
    let mut app = pump_two_slots(&mut session);
    app.error = Some("script: no focused profile".into());
    session.ack_background_bots(&mut app);
    settle_preferences(&mut session, &mut app);
    session.expire_ack_cache();
    session.pump(&mut app);
    assert!(app.background_notice.is_none());
    assert_eq!(app.error.as_deref(), Some("script: no focused profile"));
    assert!(frontend_core::resources::background_bots_acked(
        &session.preferences
    ));
    let _iso = iso;
}

/// Vault with `alice` pinned to world 2 and auto-login on, in a temp dir.
fn lifecycle_vault(test: &str) -> VaultFixture {
    let dir = std::env::temp_dir().join(format!("274bot-tui-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(
        FixtureProfile::new("alice", "pw")
            .uid(7)
            .auto_login(true)
            .world(Some(2)),
    );
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
    lifecycle_vault("multibox-rearm").start_session(&mut session.core, play);
    let mut app = TuiApp::new("tui");
    assert!(!alice.wants_login());

    // alice is a vault profile with a logged-out slot arm: mark it, then
    // Log in loads and re-arms it.
    session.pump(&mut app);
    dispatch(
        &mut session,
        &mut app,
        AppAction::MarkFleet(frontend_core::MarkOp::Shown),
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::PlanFleet(frontend_core::FleetAction::Login),
    );

    assert!(
        alice.wants_login(),
        "m must re-arm a member that is already loaded but logged out: {:?} {:?}",
        app.error,
        app.modal
    );
    assert!(!alice.login_latched());
    assert!(session.core.fleet().contains("alice"));
}

#[test]
fn load_all_keeps_focus_when_others_log_in() {
    let iso = IsolatedStores::enter("tui-loadall-keeps-focus");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    for (i, name) in ["alice", "bob", "carol", "dave"].iter().enumerate() {
        vault.put(
            FixtureProfile::new(name, "pw")
                .uid(1 + i as i32)
                .auto_login(*name != "alice"),
        );
    }
    install_vault(&mut session.core, vault);
    session.inject_play(empty_play());
    session
        .submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    select(&mut session, "alice");
    let mut app = TuiApp::new("tui");
    session.pump(&mut app);
    assert_eq!(app.focused_name().as_deref(), Some("alice"));

    dispatch(
        &mut session,
        &mut app,
        AppAction::MarkFleet(frontend_core::MarkOp::Shown),
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::PlanFleet(frontend_core::FleetAction::Login),
    );
    session.pump(&mut app);
    assert_eq!(
        session.core.members(),
        [
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string()
        ]
    );
    assert_eq!(session.core.selected(), Some("alice"));
    assert_eq!(app.focused_name().as_deref(), Some("alice"));

    for name in ["bob", "carol", "dave"] {
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        rows.retain(|row| row.username != name);
        rows.push(host_play::SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 1,
            ..host_play::SlotStatus::default()
        });
        drop(rows);
        session.pump(&mut app);
        assert_eq!(
            app.focused_name().as_deref(),
            Some("alice"),
            "{name} at scene_state==1 must not take TUI focus"
        );
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        rows.retain(|row| row.username != name);
        rows.push(host_play::SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..host_play::SlotStatus::default()
        });
        drop(rows);
        session.pump(&mut app);
        assert_eq!(
            app.focused_name().as_deref(),
            Some("alice"),
            "{name} logging in must not take TUI focus"
        );
        assert_eq!(session.core.selected(), Some("alice"));
    }
    let _iso = iso;
}

#[test]
fn load_all_picks_focus_when_none_was_focused() {
    let iso = IsolatedStores::enter("tui-loadall-picks-focus");
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    for (i, name) in ["bob", "carol", "dave"].iter().enumerate() {
        vault.put(
            FixtureProfile::new(name, "pw")
                .uid(1 + i as i32)
                .auto_login(true),
        );
    }
    install_vault(&mut session.core, vault);
    session.inject_play(empty_play());
    let mut app = TuiApp::new("tui");
    session.pump(&mut app);
    assert_eq!(session.core.selected(), None);
    assert_eq!(app.focused_name(), None);

    dispatch(
        &mut session,
        &mut app,
        AppAction::MarkFleet(frontend_core::MarkOp::Shown),
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::PlanFleet(frontend_core::FleetAction::Login),
    );
    session.pump(&mut app);
    let picked = frontend_core::pick_focus(session.core.members(), None);
    assert_eq!(picked.as_deref(), Some("bob"));
    assert_eq!(session.core.selected(), picked.as_deref());
    assert_eq!(app.focused_name(), picked);

    for name in ["carol", "dave"] {
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        rows.retain(|row| row.username != name);
        rows.push(host_play::SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 1,
            ..host_play::SlotStatus::default()
        });
        drop(rows);
        session.pump(&mut app);
        assert_eq!(
            app.focused_name(),
            picked,
            "{name} at scene_state==1 must not take TUI focus"
        );
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        rows.retain(|row| row.username != name);
        rows.push(host_play::SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..host_play::SlotStatus::default()
        });
        drop(rows);
        session.pump(&mut app);
        assert_eq!(
            app.focused_name(),
            picked,
            "{name} logging in must not take TUI focus"
        );
        assert_eq!(session.core.selected(), picked.as_deref());
    }
    let _iso = iso;
}

#[test]
fn settings_popup_persists_guardian_fields_and_live_policy() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let alice = SlotArm::new(7, true);
    lifecycle_vault("settings-fields").start_session(&mut session.core, empty_play());
    session
        .core
        .play_mut_for_test()
        .unwrap()
        .attach_arm("alice", Arc::clone(&alice));
    session.core.fleet_mut().add("alice");
    select(&mut session, "alice");
    session
        .core
        .stage_guardian(frontend_core::GuardianEdit::policy(
            "river troll",
            frontend_core::GuardianPolicy::Fight,
        ));
    session
        .core
        .submit_detached(OperatorCommand::SetGuardian, Targets::Profile("alice"))
        .unwrap()
        .operation()
        .expect("the policy write opened an operation");
    session.core.flush_writes();
    assert_eq!(
        arm_random_event_policy(&alice, "river troll"),
        frontend_core::GuardianPolicy::Fight
    );
    let mut app = TuiApp::new("tui");
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());
    // The popup binds the focused profile when it opens.
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    // A stale popup draft restores the supported Flee policy and changes the
    // other Guardian fields without replacing unrelated profile settings.
    app.settings = frontend_core::SettingsDraft::default();
    *app.settings.random_events_mut() = false;
    *app.settings.lamp_skill_mut() = "Magic".into();
    *app.settings.lamp_auto_mut() = true;
    app.settings_dirty = true;

    session.pump(&mut app);
    assert!(
        alice.random_events.load(Ordering::Relaxed),
        "the arm waits for the durable write"
    );
    session.core.flush_writes();

    let saved = session.core.profile("alice").unwrap();
    let saved_settings = saved.settings_draft();
    assert!(!saved.random_events());
    assert_eq!(saved_settings.lamp_skill(), "Magic");
    assert!(saved_settings.lamp_auto());
    assert_eq!(saved_settings.world(), Some(2), "world pin survives");
    assert!(saved.auto_login(), "auto-login survives");
    assert_eq!(
        saved.random_event_policy("river troll"),
        frontend_core::GuardianPolicy::Flee
    );
    assert_eq!(
        arm_random_event_policy(&alice, "river troll"),
        frontend_core::GuardianPolicy::Flee
    );
    assert!(!alice.random_events.load(Ordering::Relaxed));
    assert_eq!(*alice.lamp_skill.lock().unwrap(), "Magic");
}

#[test]
fn settings_popup_shows_unavailable_policy_refusal() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    lifecycle_vault("settings-policy-refusal").start_session(&mut session.core, empty_play());
    session.core.fleet_mut().add("alice");
    select(&mut session, "alice");
    let mut app = TuiApp::new("tui");
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());

    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    app.settings_state.row = 4;
    let before = session.core.refusals();
    let refused = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, refused);

    assert_eq!(
        app.error.as_deref(),
        Some("Fight and Ignore are unavailable; the Guardian treats saved choices as Flee.")
    );
    assert_eq!(
        session.core.refusals(),
        before + 1,
        "the refused policy is logged at the command boundary"
    );
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
    lifecycle_vault("remove-neighbour").start_session(&mut session.core, play);
    session.core.fleet_mut().add("alice");
    session.core.fleet_mut().add("bob");
    let bob = session.core.play_for_test().unwrap().arm("bob").unwrap();
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

#[cfg(feature = "js")]
const LOOPING_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";
#[cfg(feature = "js")]
const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// A TUI session over a real vault and an empty play: members loaded
/// (arms attached, no worker threads), library in the isolated home.
fn tui_with_profiles(iso: &IsolatedStores, names: &[&str]) -> (TuiSession, TuiApp) {
    tui_with_loaded(iso, names, names)
}

/// [`tui_with_profiles`] with only `loaded` of the vault's `names` loaded:
/// the others are vault profiles that are not loaded.
fn tui_with_loaded(iso: &IsolatedStores, names: &[&str], loaded: &[&str]) -> (TuiSession, TuiApp) {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let mut vault = VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01");
    for (i, name) in names.iter().enumerate() {
        vault.put(FixtureProfile::new(name, "pw").uid(1 + i as i32));
    }
    install_vault(&mut session.core, vault);
    session.inject_play(empty_play());
    #[cfg(feature = "js")]
    session.scripts.use_library_at(
        iso.dir().join("js-scripts.json"),
        iso.dir().join("js-cache"),
    );
    for name in loaded {
        session
            .submit(OperatorCommand::Load, Targets::Profile(name))
            .expect("load");
    }
    let mut app = TuiApp::new("scripts");
    session.pump(&mut app);
    (session, app)
}

#[cfg(feature = "js")]
fn assign(session: &mut TuiSession, name: &str, card: &LoadedCardSnapshot) {
    assert!(assign_loaded(
        &mut session.scripts,
        &mut session.core,
        name,
        card.source(),
        card.identity_id()
    ));
    session.core.flush_writes();
}
fn save_compiled_setting(
    session: &mut TuiSession,
    app: &mut TuiApp,
    name: &str,
    card_id: CompiledCardId,
    field: &str,
    value: serde_json::Value,
) {
    let selection = ScriptChoice::compiled(card_id);
    session
        .scripts
        .stage_setting(Some(name), &selection, field, value);
    session
        .submit_with(
            app,
            OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Native),
            Targets::Profile(name),
        )
        .unwrap();
}

fn start_assigned_script(session: &mut TuiSession, app: &mut TuiApp, name: &str) {
    session
        .submit_with(app, OperatorCommand::StartScript, Targets::Profile(name))
        .unwrap();
}

fn focus_member(session: &mut TuiSession, app: &mut TuiApp, name: &str) {
    session.focus(name);
    session.pump(app);
}

/// Plan `action` as the keys do, and answer the confirm overlay with Enter
/// when the plan asks first.
fn run_fleet_action(
    session: &mut TuiSession,
    app: &mut TuiApp,
    action: frontend_core::FleetAction,
) {
    session.pump(app);
    dispatch(session, app, AppAction::PlanFleet(action));
    if matches!(
        &app.modal,
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(confirm.kind, crate::overlay::ConfirmKind::FleetPlan { .. })
    ) {
        let confirm = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
        dispatch(session, app, confirm);
    }
}

#[test]
fn restart_required_prompt_is_visible_at_standard_and_compact_sizes() {
    let iso = IsolatedStores::enter("tui-restart-settings-prompt");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    session
        .core
        .play_mut_for_test()
        .unwrap()
        .bind_script_test_data(data);
    let card_id = CompiledCardId::lookup("Gatherer").unwrap();
    assert!(assign_compiled(
        &mut session.scripts,
        &mut session.core,
        "alice",
        card_id
    ));
    session.core.flush_writes();
    save_compiled_setting(
        &mut session,
        &mut app,
        "alice",
        card_id,
        "skill",
        serde_json::json!("Fishing"),
    );
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    start_assigned_script(&mut session, &mut app, "alice");
    settle_starts(&mut session, &mut app);
    wait_script_state(
        session.core.play_for_test().unwrap(),
        "alice",
        ScriptRunState::Running,
    );

    save_compiled_setting(
        &mut session,
        &mut app,
        "alice",
        card_id,
        "skill",
        serde_json::json!("Mining"),
    );
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    let prompt = session.scripts.pending_restart_prompt().unwrap();
    let summary = prompt.summary();
    let op = prompt.operation();
    assert!(matches!(
        app.modal.as_ref(),
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(
                &confirm.kind,
                crate::overlay::ConfirmKind::RestartSettings { op: shown_op, summary: shown, .. }
                    if *shown_op == op && shown == &summary
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
    session
        .submit(OperatorCommand::DeferSettingsRestart, Targets::Application)
        .unwrap();
}

#[test]
fn restart_settings_modal_redraws_when_targets_shrink_under_the_same_op() {
    let iso = IsolatedStores::enter("tui-restart-settings-stale-summary");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    session
        .core
        .play_mut_for_test()
        .unwrap()
        .bind_script_test_data(data);
    let card_id = CompiledCardId::lookup("Gatherer").unwrap();
    for name in ["alice", "bob"] {
        assert!(assign_compiled(
            &mut session.scripts,
            &mut session.core,
            name,
            card_id
        ));
    }
    session.core.flush_writes();
    for name in ["alice", "bob"] {
        save_compiled_setting(
            &mut session,
            &mut app,
            name,
            card_id,
            "skill",
            serde_json::json!("Fishing"),
        );
    }
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    for name in ["alice", "bob"] {
        start_assigned_script(&mut session, &mut app, name);
    }
    settle_starts(&mut session, &mut app);
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
    }
    for name in ["alice", "bob"] {
        save_compiled_setting(
            &mut session,
            &mut app,
            name,
            card_id,
            "skill",
            serde_json::json!("Mining"),
        );
    }
    session.core.flush_writes();
    session.poll_scripts(&mut app);
    let prompt = session.scripts.pending_restart_prompt().unwrap();
    let op = prompt.operation();
    let summary = prompt.summary();
    assert_eq!(
        prompt.profiles().map(String::as_str).collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    assert!(
        summary.contains("alice") && summary.contains("bob"),
        "{summary}"
    );
    assert!(matches!(
        app.modal.as_ref(),
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(
                &confirm.kind,
                crate::overlay::ConfirmKind::RestartSettings { op: shown_op, summary: shown, .. }
                    if *shown_op == op && shown == &summary
            )
    ));

    session.core.play_for_test().unwrap().script_stop("alice");
    wait_script_state(
        session.core.play_for_test().unwrap(),
        "alice",
        ScriptRunState::Idle,
    );
    session.poll_scripts(&mut app);

    let prompt = session
        .scripts
        .pending_restart_prompt()
        .expect("bob remains");
    assert_eq!(prompt.operation(), op);
    assert_eq!(
        prompt.profiles().map(String::as_str).collect::<Vec<_>>(),
        ["bob"]
    );
    let summary = prompt.summary();
    assert!(
        !summary.contains("alice"),
        "live prompt dropped alice: {summary}"
    );
    assert!(summary.contains("bob"), "{summary}");
    assert!(matches!(
        app.modal.as_ref(),
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(
                &confirm.kind,
                crate::overlay::ConfirmKind::RestartSettings { op: shown_op, summary: shown, .. }
                    if *shown_op == op && shown == &summary
            )
    ));

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw_modal(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!text.contains("alice"), "modal still names alice: {text}");
    assert!(text.contains("bob"), "{text}");
    session
        .submit(OperatorCommand::DeferSettingsRestart, Targets::Application)
        .unwrap();
}

/// Live finding (TUI parity): the settings popup closed without explanation
/// when focus moved, dropping the draft. It must stay open and bound to the
/// profile it was opened for, and persist there — never to the newly
/// focused profile.
#[test]
fn settings_popup_stays_bound_to_its_profile_across_focus_change() {
    let iso = IsolatedStores::enter("tui-settings-bind");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    session
        .core
        .play_for_test()
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
    session
        .core
        .submit_detached(
            OperatorCommand::SetMemoryMode(api::lifecycle::MemoryMode::High),
            Targets::Profile("alice"),
        )
        .unwrap();
    session.core.flush_writes();
    focus_member(&mut session, &mut app, "alice");
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    assert!(app.settings_state.open, "settings opens");
    // Toggle random events off: the draft belongs to alice.
    let edit = app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    dispatch(&mut session, &mut app, edit);

    assert!(!app.settings.random_events(), "the toggle edits the draft");
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
        !app.settings.random_events(),
        "a focus change never rewrites the buffers"
    );
    // Persisting writes alice's row, never bob's.
    session.pump(&mut app);
    session.core.flush_writes();
    assert!(
        !session.core.profile("alice").unwrap().random_events(),
        "the edit lands on the bound profile"
    );
    assert!(
        session.core.profile("bob").unwrap().random_events(),
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
fn settings_press(session: &mut TuiSession, app: &mut TuiApp, code: crossterm::event::KeyCode) {
    let action = app.on_key(settings_key(code));
    dispatch(session, app, action);
}

/// `Saved <name>.` shows in the settings popup, in green under its rows,
/// only once the persist is durable.
#[test]
fn settings_popup_shows_saved_once_the_write_is_durable() {
    let iso = IsolatedStores::enter("tui-settings-saved");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
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
    assert!(!session.core.profile("alice").unwrap().random_events());
    assert!(session.core.profile("bob").unwrap().random_events());
}

/// Closing the settings popup before a persist completes: the popup opened
/// next, here on another profile, never shows that persist's `Saved`.
#[test]
fn a_persist_completing_after_close_shows_nothing_in_the_next_popup() {
    let iso = IsolatedStores::enter("tui-settings-close-pending");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
    session.pump(&mut app);
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Esc);
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
    assert!(
        !session.core.profile("alice").unwrap().random_events(),
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

    let iso = IsolatedStores::enter("tui-settings-late-failure");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
    session.pump(&mut app);
    assert!(!app.settings.random_events(), "the popup shows the edit");

    std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
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
        session.core.profile("alice").unwrap().random_events(),
        "nothing was saved"
    );

    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
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

    let iso = IsolatedStores::enter("tui-settings-late-closed");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    let gate = session.core.write_gate();
    let held = gate.lock().unwrap();
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
    session.pump(&mut app);
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Esc);
    session.pump(&mut app);
    focus_member(&mut session, &mut app, "bob");
    open_settings(&mut session, &mut app);

    std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
    drop(held);
    session.core.flush_writes();
    session.pump(&mut app);
    std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
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
    let iso = IsolatedStores::enter("tui-settings-refused");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    focus_member(&mut session, &mut app, "alice");
    open_settings(&mut session, &mut app);
    remove_saved_profile(&mut session.core, "alice");
    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
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

    settings_press(&mut session, &mut app, crossterm::event::KeyCode::Enter);
    for (w, h) in SIZES {
        let screen = screen(&mut app, w, h);
        assert!(
            cell_of(&screen, frontend_core::NOTHING_SAVED).is_none(),
            "{w}x{h}: editing the popup clears the refusal"
        );
    }
    assert!(session.core.profile("alice").is_none(), "nothing recreated");
    assert!(
        session.core.profile("bob").unwrap().random_events(),
        "bob untouched"
    );
}

#[cfg(feature = "js")]
fn press(session: &mut TuiSession, app: &mut TuiApp, code: crossterm::event::KeyCode) {
    let key = crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
    let action = session.params_key(app, key);
    dispatch(session, app, action);
}

/// Start fleet, Reload (warn, then Confirm) and Stop fleet from the script
/// pane go through the same coordinator as the panel: assigned members
/// start, the warning names both runs, Confirm replaces both, Stop fleet
/// stops both.
#[cfg(feature = "js")]
#[test]
fn tui_start_all_reload_and_stop_all_use_the_shared_coordinator() {
    let iso = IsolatedStores::enter("tui-bulk-scripts");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let path = iso.dir().join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    assign(&mut session, "alice", &card);
    assign(&mut session, "bob", &card);

    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Start);
    let click = app.error.clone().unwrap_or_default();
    assert!(click.starts_with("Start: "), "the result line: {click}");
    assert!(
        !click.contains("Start selected"),
        "a planned run puts no Scripts tally on the banner: {click}"
    );
    settle_starts(&mut session, &mut app);
    settle_fleet_result(&mut session, &mut app);
    let settled = app.error.clone().unwrap_or_default();
    assert!(
        settled.starts_with("Start: done 2:")
            && settled.contains("alice")
            && settled.contains("bob"),
        "the one result line names both bots once settled: {settled}"
    );
    let generation = |session: &TuiSession, name: &str| {
        session
            .core
            .play_for_test()
            .unwrap()
            .script_runtime_generation(name)
    };
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
    }
    let before = (generation(&session, "alice"), generation(&session, "bob"));

    focus_member(&mut session, &mut app, "alice");
    assert_eq!(
        app.script_sel,
        Some(ScriptChoice::loaded(CardSource::File, card.identity_id(),)),
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let outcome = loop {
        session.pump(&mut app);
        if let Some(outcome) = session.scripts.take_reload_outcome() {
            break outcome;
        }
        assert!(
            Instant::now() < deadline,
            "confirmed reload preparation did not settle"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(matches!(outcome.applied_counts(), Some((2, _, 0))));
    settle_starts(&mut session, &mut app);
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
    }
    assert_ne!(generation(&session, "alice"), before.0);
    assert_ne!(generation(&session, "bob"), before.1);
    assert!(!app.reload_confirm);

    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Stop);
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Idle,
        );
    }
}

/// The TUI params popup edits the focused profile's own bag (not the
/// global store) and `a`/`y` apply it to the same-card member only.
#[cfg(feature = "js")]
#[test]
fn tui_params_edit_the_focused_profile_and_apply_to_all_skips_other_cards() {
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-apply-to-all");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob", "carol"]);
    let thiever_path = iso.dir().join("thiever.ts");
    std::fs::write(&thiever_path, THIEVER_TS).unwrap();
    let miner_path = iso.dir().join("miner.ts");
    std::fs::write(&miner_path, LOOPING_TS).unwrap();
    let thiever = session.scripts.load_card(&thiever_path).unwrap();
    let miner = session.scripts.load_card(&miner_path).unwrap();
    assign(&mut session, "alice", &thiever);
    assign(&mut session, "bob", &thiever);
    assign(&mut session, "carol", &miner);
    start_assigned_script(&mut session, &mut app, "bob");
    settle_starts(&mut session, &mut app);
    wait_script_state(
        session.core.play_for_test().unwrap(),
        "bob",
        ScriptRunState::Running,
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
        saved_script_bag(&session.core, name, key).and_then(|bag| bag.get("target").cloned())
    };
    let guard = Some(serde_json::json!("Guard"));
    assert_eq!(target(&session, "alice"), guard);
    assert_eq!(target(&session, "bob"), guard);
    assert_eq!(target(&session, "carol"), None, "other card untouched");
    assert!(
        session
            .scripts
            .legacy_overrides(CardSource::File, thiever.name())
            .is_empty(),
        "the global store is not written"
    );
    let report = session.scripts.last_settings_sync().unwrap();
    assert_eq!(
        (report.saved(), report.skipped().len(), report.delivered()),
        (1, 1, 1)
    );
    assert_eq!(app.error.as_deref(), Some(report.summary()));
    session.core.play_for_test().unwrap().script_stop("bob");
}

#[cfg(feature = "js")]
#[test]
#[cfg(unix)]
fn failed_typed_parameter_saves_restore_the_value_and_show_the_field_error() {
    use crossterm::event::KeyCode;
    use std::os::unix::fs::PermissionsExt;

    for (tag, definition, old, new) in [
        (
            "picker",
            "{ type: 'string', default: 'Range', label: 'Cook on', options: ['Range', 'Fire'] }",
            serde_json::json!("Range"),
            serde_json::json!("Fire"),
        ),
        (
            "checkbox",
            "{ type: 'boolean', default: true, label: 'Cook on' }",
            serde_json::json!(true),
            serde_json::json!(false),
        ),
    ] {
        let iso = IsolatedStores::enter(&format!("tui-typed-failure-{tag}"));
        let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
        let path = iso.dir().join("settings.ts");
        std::fs::write(
            &path,
            format!(
                "export const SETTINGS = {{ value: {definition} }};\nexport default class T extends LoopingBot {{ override loop() {{}} }}\n"
            ),
        ).unwrap();
        let card = session.scripts.load_card(&path).unwrap();
        assign(&mut session, "alice", &card);
        focus_member(&mut session, &mut app, "alice");
        dispatch(&mut session, &mut app, AppAction::ScriptParams);
        assert_eq!(app.params_bag.get("value"), Some(&old));
        let gate = session.core.write_gate();
        let held = gate.lock().unwrap();
        press(&mut session, &mut app, KeyCode::Char(' '));
        assert_eq!(app.params_bag.get("value"), Some(&new));
        std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
        drop(held);
        session.core.flush_writes();
        session.pump(&mut app);
        std::fs::set_permissions(iso.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            app.params_bag.get("value"),
            Some(&old),
            "{tag}: durable value"
        );
        let error = session
            .scripts
            .parameter_text_error_for(Some("alice"), &app.params_card().unwrap(), "value")
            .expect("keep the writer error")
            .to_owned();
        assert!(!error.is_empty());
        for (width, height) in [(80, 24), (120, 40)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            headed_frame(&mut session, &mut app, &mut terminal, &mut Vec::new()).unwrap();
            let rendered = terminal.backend().buffer();
            let popup = crate::script_params::ParamsPane::popup_rect(rendered.area);
            let mut visible = String::new();
            for y in popup.top() + 1..popup.bottom() - 1 {
                for x in popup.left() + 1..popup.right() - 1 {
                    visible.push_str(rendered[(x, y)].symbol());
                }
                visible.push('\n');
            }
            let compact = visible
                .chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>();
            let field_error = format!("Cook on: {error}")
                .chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>();
            assert!(
                compact.contains(&field_error),
                "{tag} {width}x{height}: the failed control shows its full error\n{visible}\nexpected {field_error}"
            );
        }
    }
}

/// Start with a prior load failure lists it on the strip; a newer front-end
/// error replaces it; the Start settling Ready leaves that error shown.
#[cfg(feature = "js")]
#[test]
fn tui_ready_start_keeps_a_newer_error_on_the_strip() {
    let iso = IsolatedStores::enter("tui-ready-newer-error");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    std::fs::write(iso.dir().join("gate.ts"), "export const fail = true;").unwrap();
    let path = iso.dir().join("retry.ts");
    std::fs::write(
        &path,
        "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('tui-first-load');\nexport function tick(api) {}\n",
    )
    .unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    let sel = ScriptChoice::loaded(CardSource::File, card.identity_id());
    focus_member(&mut session, &mut app, "alice");
    session.script_start(&mut app, &sel);
    settle_starts(&mut session, &mut app);
    std::fs::write(iso.dir().join("gate.ts"), "export const fail = false;").unwrap();

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
    assert!(session.scripts.load_failure(card.identity_key()).is_none());
    assert_eq!(app.error.as_deref(), Some("map: no route"));
    session.core.play_for_test().unwrap().script_stop("alice");
}

#[cfg(feature = "js")]
#[test]
fn tui_start_all_paces_admission_through_the_shared_coordinator() {
    let iso = IsolatedStores::enter("tui-start-pace");
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    let path = iso.dir().join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    for name in names {
        assign(&mut session, name, &card);
    }

    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Start);
    let starting = names
        .iter()
        .filter(|name| session.core.script_state(name) == ScriptRunState::Starting)
        .count();
    assert!(
        starting <= frontend_core::scripts::START_ADMIT_PER_FRAME,
        "TUI Start fleet dispatched {starting} in the click frame"
    );
    assert!(starting > 0 || session.scripts.starts_pending());
    settle_starts(&mut session, &mut app);
    for name in names {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
        session.core.play_for_test().unwrap().script_stop(name);
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

#[cfg(feature = "js")]
/// The screen's words at `w`x`h`: borders and padding collapse to single
/// spaces, so a prompt that wraps across rows still reads as one line.
fn screen_words(app: &mut TuiApp, w: u16, h: u16) -> String {
    screen_text(app, w, h)
        .chars()
        .map(|c| {
            if c.is_whitespace() || "│─┌┐└┘├┤┬┴┼".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command palette's *Apply focused bot's settings to marked* copies the
/// focused bot's parameters to the marked same-card bots only: the frozen
/// scope is confirmed first (Esc drops it), an unmarked same-card bot keeps
/// its own, and the report names the marked bot that could not take it. The
/// card lives under a long path so the confirm text wraps at both sizes
/// (the Windows nightly condition on every OS); the screen check reads the
/// words without requiring one unbroken row.
#[cfg(feature = "js")]
#[test]
fn palette_apply_settings_to_marked_copies_only_to_marked_same_card_bots() {
    let iso = IsolatedStores::enter("tui-apply-marked");
    let (mut session, mut app) =
        tui_with_profiles(&iso, &["alice", "bob", "carol", "dave", "erin"]);
    // One long word, like the Windows nightly temporary path: the prompt is
    // hundreds of cells, so it wraps to several rows at both sizes on every
    // OS. The over-long path starts on a fresh row; this padding puts
    // `2 marked bots` across the wrap at 80x24 and at 120x40 (the popup is
    // 80/90 wide with a 2-cell border).
    let deep = iso.dir().join(["nested-0123456789"; 8].join("/"));
    let deep = deep
        .join("n".repeat(11))
        .join("n".repeat(225))
        .join("n".repeat(224));
    std::fs::create_dir_all(&deep).unwrap();
    let thiever = deep.join("thiever.ts");
    std::fs::write(&thiever, THIEVER_TS).unwrap();
    let card = session.scripts.load_card(&thiever).unwrap();
    let looping = iso.dir().join("looping.ts");
    std::fs::write(&looping, LOOPING_TS).unwrap();
    let other = session.scripts.load_card(&looping).unwrap();
    for name in ["alice", "bob", "carol", "dave"] {
        assign(&mut session, name, &card);
    }
    assign(&mut session, "erin", &other);
    let selection = ScriptChoice::loaded(CardSource::File, card.identity_id());
    session.scripts.stage_setting(
        Some("alice"),
        &selection,
        "target",
        serde_json::json!("Guard"),
    );
    session
        .submit_with(
            &mut app,
            OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Script),
            Targets::Profile("alice"),
        )
        .unwrap();
    session.core.flush_writes();
    session.pump(&mut app);

    focus_member(&mut session, &mut app, "alice");
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, card.identity_id()));
    let command = crate::commands::Command::Fleet(frontend_core::FleetAction::CopySettings);
    assert_eq!(
        command.availability(&app),
        Err(crate::commands::Unavailable::NoFleetCard),
        "each bot's own card cannot be copied"
    );
    // The fleet card follows the focused bot: alice's card.
    dispatch(
        &mut session,
        &mut app,
        AppAction::PickFleetCard(frontend_core::FleetCard::FOCUSED),
    );
    assert_eq!(command.availability(&app), Ok(()));
    // bob and carol take it; erin is marked but on another card; dave is not
    // marked.
    for name in ["bob", "carol", "erin"] {
        let id = session.core.profile_identity(name).unwrap();
        app.fleet_model.set_mark_for_test(id, true);
    }

    // The palette lists the command over the marked scope, at both sizes.
    for (w, h) in SIZES {
        app.modal = None;
        let open = app.run_command(crate::commands::Command::Palette);
        dispatch(&mut session, &mut app, open);
        for c in "copy settings".chars() {
            app.on_key(settings_key(crossterm::event::KeyCode::Char(c)));
        }
        let shown = screen_text(&mut app, w, h);
        assert!(shown.contains("Copy settings"), "{w}x{h}: {shown}");
    }

    // Running it freezes the scope and asks first; Esc drops it unwritten.
    let vault_file = iso.dir().join("vault");
    let rows_before = script_settings_rows(&session);
    let bytes_before = std::fs::read(&vault_file).unwrap();
    app.modal = None;
    let action = app.run_command(command);
    dispatch(&mut session, &mut app, action);
    let prompt = session
        .core
        .fleet_plan()
        .expect("a copy always waits for confirmation")
        .prompt()
        .to_owned();
    assert!(
        prompt.contains("to 2 marked bots: bob, carol") && prompt.contains("erin"),
        "{prompt}"
    );
    // The words are all shown even when the long path wraps them across
    // rows: borders and padding collapse, so no unbroken row is required.
    for (w, h) in SIZES {
        let shown = screen_words(&mut app, w, h);
        assert!(
            shown.contains("Copy alice's")
                && shown.contains("2 marked bots")
                && shown.contains("bob, carol")
                && shown.contains("erin: assigned another card")
                && shown.contains("Run y")
                && shown.contains("Cancel n"),
            "{w}x{h}: {shown}"
        );
    }
    let cancel = app.on_key(settings_key(crossterm::event::KeyCode::Esc));
    dispatch(&mut session, &mut app, cancel);
    assert!(session.core.fleet_plan().is_none(), "Esc leaves no plan");
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
    let saved = |session: &TuiSession, name: &str| saved_script_bag(&session.core, name, key);
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
            && report.contains("erin: skipped, assigned another card"),
        "{report}"
    );
    let result = session
        .core
        .fleet_result()
        .expect("the copy's result view")
        .text()
        .to_owned();
    assert!(
        result.starts_with("Copy settings to: ") && result.contains("erin"),
        "{result}"
    );
}

/// Opening *Apply focused bot's settings to marked* for a card newly assigned
/// to the focused bot (no per-card key in its profile yet, only a legacy
/// global override) and cancelling leaves the vault byte-identical, as the
/// dialog promises, while the frozen scope still carries the migrated bag.
#[cfg(feature = "js")]
#[test]
fn palette_apply_settings_to_marked_prepare_and_cancel_leave_a_fresh_source_unwritten() {
    let iso = IsolatedStores::enter("tui-apply-marked-fresh");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let thiever = iso.dir().join("thiever.ts");
    std::fs::write(&thiever, THIEVER_TS).unwrap();
    let card = session.scripts.load_card(&thiever).unwrap();
    for name in ["alice", "bob"] {
        assign(&mut session, name, &card);
    }
    session
        .scripts
        .set_legacy_str(card.source(), card.name(), "target", "Guard");
    focus_member(&mut session, &mut app, "alice");
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, card.identity_id()));
    let id = session.core.profile_identity("bob").unwrap();
    app.fleet_model.set_mark_for_test(id, true);

    let vault_file = iso.dir().join("vault");
    let rows_before = script_settings_rows(&session);
    assert!(
        rows_before
            .iter()
            .all(|(_, settings)| !settings.contains_key(card.identity_key())),
        "a newly assigned card has no per-card key"
    );
    let bytes_before = std::fs::read(&vault_file).unwrap();

    dispatch(
        &mut session,
        &mut app,
        AppAction::PickFleetCard(frontend_core::FleetCard::FOCUSED),
    );
    let action = app.run_command(crate::commands::Command::Fleet(
        frontend_core::FleetAction::CopySettings,
    ));
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    session.pump(&mut app);
    let scope = session.scripts.prepared_settings_sync().unwrap();
    assert_eq!(
        scope.overrides().get("target"),
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

#[cfg(feature = "js")]
type SettingsRows = Vec<(
    String,
    std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>>,
)>;

/// Every profile's per-card settings map, by name.
#[cfg(feature = "js")]
fn script_settings_rows(session: &TuiSession) -> SettingsRows {
    saved_script_settings(&session.core)
}

#[cfg(feature = "js")]
#[test]
fn memory_relog_warns_before_cancelling_a_queued_start() {
    let iso = IsolatedStores::enter("tui-relog-queued-start");
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    let path = iso.dir().join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    for name in names {
        assign(&mut session, name, &card);
    }
    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Start);
    let queued = names
        .iter()
        .find(|name| session.scripts.start_queue_place(name).is_some())
        .expect("more Starts than one frame admits")
        .to_string();
    assert_eq!(
        session.core.script_state(&queued),
        ScriptRunState::Idle,
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
            .play_for_test()
            .unwrap()
            .arm(&queued)
            .unwrap()
            .login_latched(),
        "nothing relogs before confirmation"
    );
    session
        .submit(OperatorCommand::StopScript, Targets::Everyone)
        .unwrap();
}

/// Confirming Relog-now on a Start that is still queued cancels the queued
/// Start, and the original fleet Start command's member for that bot
/// settles cancelled (`ScopeInvalidated`: the relog retires the session it
/// was issued under): it never starts later.
#[cfg(feature = "js")]
#[test]
fn confirmed_relog_settles_the_queued_start_member_and_it_never_starts() {
    use api::identity::{Cancellation, Settlement};
    use frontend_core::command::Subject;
    let iso = IsolatedStores::enter("tui-relog-queued-start-confirmed");
    let names = ["p0", "p1", "p2", "p3", "p4"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    let path = iso.dir().join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    for name in names {
        assign(&mut session, name, &card);
    }
    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Start);
    let start = session
        .core
        .fleet_result()
        .expect("the Start result")
        .request()
        .expect("the Start opened a command record");
    let queued = names
        .iter()
        .find(|name| session.scripts.start_queue_place(name).is_some())
        .expect("more Starts than one frame admits")
        .to_string();
    assert_eq!(session.core.script_state(&queued), ScriptRunState::Idle);

    dispatch(
        &mut session,
        &mut app,
        AppAction::MemoryRelog(queued.clone()),
    );
    assert!(matches!(
        &app.modal,
        Some(crate::overlay::Modal::Confirm(confirm))
            if confirm.kind == crate::overlay::ConfirmKind::MemoryRelog(queued.clone())
    ));
    // The overlay's own confirm path.
    press_key(
        &mut session,
        &mut app,
        crate::test_support::key(crossterm::event::KeyCode::Enter),
    );
    assert!(app.modal.is_none());

    let identity = session.core.profile_identity_or_synthetic(&queued);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.core.flush_writes();
        session.pump(&mut app);
        let member = session
            .core
            .commands()
            .report(start)
            .and_then(|report| report.member(Subject::Profile(identity)));
        if let Some(terminal) = member.and_then(|member| member.terminal()) {
            assert_eq!(
                terminal,
                // Relog-now in the same frame retires the session scope the
                // queued Start was issued under, before the coordinator's
                // next poll would settle it as NotLoggedIn.
                &Settlement::Cancelled(Cancellation::ScopeInvalidated),
                "the queued member of the original Start (detail {:?})",
                member.and_then(|member| member.detail())
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the queued Start member never settled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        session.scripts.start_queue_place(&queued).is_none(),
        "the queued Start is gone"
    );
    for _ in 0..20 {
        session.core.flush_writes();
        session.pump(&mut app);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(
            session.core.script_state(&queued),
            ScriptRunState::Idle,
            "the cancelled Start never starts"
        );
    }
    session
        .submit(OperatorCommand::StopScript, Targets::Everyone)
        .unwrap();
}

#[test]
fn settings_memory_row_persists_and_arms_the_handshake() {
    let mut session = TuiSession::new(dummy_options());
    session.core.set_spawn_workers(false);
    let alice = SlotArm::new(7, true);
    lifecycle_vault("mem-persist").start_session(&mut session.core, empty_play());
    session
        .core
        .play_mut_for_test()
        .unwrap()
        .attach_arm("alice", Arc::clone(&alice));
    session.core.fleet_mut().add("alice");
    select(&mut session, "alice");
    let mut app = TuiApp::new("tui");
    session.names = vec!["alice".into()];
    session.last_focused = Some("alice".into());
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    // Flip the memory row draft; the binary persists it on the next pump.
    *app.settings.lowmem_mut() = false;
    app.settings_dirty = true;
    session.pump(&mut app);

    assert_eq!(
        alice.lowmem_handshake(),
        Some(false),
        "the toggle arms the next handshake at once"
    );
    session.core.flush_writes();
    assert!(
        !session.core.profile("alice").unwrap().lowmem(),
        "the setting is durable"
    );
}

#[cfg(feature = "js")]
#[test]
fn memory_relog_confirms_while_a_script_runs() {
    let mut play = empty_play();
    let alice = SlotArm::new(7, false);
    play.attach_arm("alice", Arc::clone(&alice));
    start_script_source(
        &play,
        "alice",
        "export function tick(api) { api._n = (api._n||0)+1 }".to_string(),
        LoadShape::NativeTick,
    )
    .unwrap();
    wait_script_state(&play, "alice", ScriptRunState::Running);
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
    assert!(
        !alice.login_latched(),
        "RelogNow does not make a Logout latch"
    );
    assert!(
        !alice.wants_logout(),
        "the fixture has no owner session to leave"
    );
    assert!(
        alice.wants_login(),
        "the confirmed offline relog arms Login"
    );
}

/// Review R3-H3: the import overlay built from a real preview of an export
/// whose password is repeated as the account's tab holds no copy of the
/// password, so its derived `Debug` shows none. (The redaction registry is
/// process-wide: this test's password is its own.)
#[cfg(feature = "js")]
#[test]
fn import_overlay_debug_holds_no_export_password() {
    const SECRET: &str = "r4t-tui-pw-60234";
    let iso = IsolatedStores::enter("tui-import-redacted");
    let (mut session, mut app) = tui_with_profiles(&iso, &[]);
    let path = iso
        .dir()
        .join(frontend_core::rs2b0t_import::PROFILE_FILE_NAME);
    std::fs::write(
        &path,
        serde_json::json!({
            "kind": frontend_core::rs2b0t_import::PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [{ "username": "rvr4probe", "password": SECRET, "tab": SECRET }],
            "tabs": [],
            "activeTab": "Main",
            "storage": { "rvr4probe": {} },
        })
        .to_string(),
    )
    .unwrap();
    let open = app.run_command(crate::commands::Command::ImportProfiles);
    dispatch(&mut session, &mut app, open);
    dispatch(
        &mut session,
        &mut app,
        AppAction::ProfileImport(crate::profile_import::ImportAction::Preview(
            path.display().to_string(),
        )),
    );
    let Some(crate::overlay::Modal::ProfileImport(view)) = app.modal.as_ref() else {
        panic!("the import overlay is not open");
    };
    assert_eq!(
        view.stage,
        frontend_core::rs2b0t_import::Stage::Preview,
        "{:?}",
        view.error
    );
    assert!(
        view.lines
            .iter()
            .any(|line| line.contains("not imported: tab")),
        "{:?}",
        view.lines
    );
    let shown = format!("{view:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[cfg(feature = "js")]
#[test]
fn profile_import_preview_waits_for_first_catalog_scan() {
    let iso = IsolatedStores::enter("tui-import-catalog-pending");
    let root = fake_rs2b0t_tree(iso.dir(), false);
    iso.set_catalog_root(&root);
    let mut session = TuiSession::new(dummy_options());
    install_vault(
        &mut session.core,
        VaultFixture::create(&iso.dir().join("vault"), "test-passphrase-01"),
    );
    let mut app = TuiApp::new("274bot headless");
    let path = iso.dir().join("profiles.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "kind": frontend_core::rs2b0t_import::PROFILE_FILE_KIND,
            "v": 1,
            "profiles": [],
            "tabs": [],
            "activeTab": "Main",
            "storage": {},
        })
        .to_string(),
    )
    .unwrap();

    session.profile_import(&mut app, ImportAction::Preview(path.display().to_string()));
    assert!(session.scripts.catalog_work_pending());
    assert!(session.profile_import_preview_pending);
    assert_eq!(
        session.profile_import.stage(),
        frontend_core::rs2b0t_import::Stage::Path
    );

    settle_catalog_work(&mut session, &mut app);
    assert!(!session.profile_import_preview_pending);
    assert_eq!(
        session.profile_import.stage(),
        frontend_core::rs2b0t_import::Stage::Preview
    );
}

/// One headless fleet session end to end: type a filter, mark the shown
/// rows, then run a marked bulk action (Assign) and read its report.
#[cfg(feature = "js")]
#[test]
fn fleet_session_filters_marks_shown_and_assigns_marked() {
    let iso = IsolatedStores::enter("tui-fleet-session");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob", "carol", "bobby"]);
    let looping = iso.dir().join("looping.ts");
    std::fs::write(&looping, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&looping).unwrap();
    app.script_sel = Some(ScriptChoice::loaded(CardSource::File, card.identity_id()));
    session.pump(&mut app);

    // Filter by a typed term: only bob and bobby stay shown.
    let open = app.run_command(crate::commands::Command::FilterFleet);
    dispatch(&mut session, &mut app, open);
    for c in "bob".chars() {
        let typed = app.on_key(settings_key(crossterm::event::KeyCode::Char(c)));
        dispatch(&mut session, &mut app, typed);
    }
    let done = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, done);
    let shown: Vec<&str> = app
        .fleet_model
        .shown_rows()
        .map(|(_, row)| row.name())
        .collect();
    assert_eq!(shown, ["bob", "bobby"]);
    for (w, h) in SIZES {
        let text = screen_text(&mut app, w, h);
        assert!(text.contains("2/4 shown"), "{w}x{h}: {text}");
    }

    // Mark all shown marks exactly the filtered rows.
    let mark = app.run_command(crate::commands::Command::MarkAllShown);
    dispatch(&mut session, &mut app, mark);
    assert_eq!(app.fleet_model.marks().len(), 2);
    for name in ["bob", "bobby"] {
        let id = session.core.profile_identity(name).unwrap();
        assert!(app.fleet_model.marks().contains(id), "{name} is marked");
    }
    let alice = session.core.profile_identity("alice").unwrap();
    assert!(!app.fleet_model.marks().contains(alice));

    // Pin the card, then Assign it to the marked bots: marked and safe, so
    // core's plan needs no confirmation and runs at once.
    let card_choice = ScriptChoice::loaded(CardSource::File, card.identity_id());
    dispatch(
        &mut session,
        &mut app,
        AppAction::PickFleetCard(frontend_core::FleetCard::pinned(card_choice)),
    );
    let action = app.run_command(crate::commands::Command::Fleet(
        frontend_core::FleetAction::Assign,
    ));
    dispatch(&mut session, &mut app, action);
    assert!(app.modal.is_none(), "a marked Assign is one step");
    settle_fleet_result(&mut session, &mut app);
    let assigned =
        |name: &str| frontend_core::Scripts::test_assignment(&session.core, name).map(|a| a.key());
    assert_eq!(assigned("bob"), Some(card.identity_key().to_owned()));
    assert_eq!(assigned("bobby"), Some(card.identity_key().to_owned()));
    assert_eq!(assigned("alice"), None, "the filtered-out bot is untouched");
    assert_eq!(assigned("carol"), None, "the filtered-out bot is untouched");
    let report = app.error.clone().expect("the result is shown");
    assert!(
        report.starts_with("Assign: ") && report.contains("bob, bobby"),
        "{report}"
    );
}

fn alt_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::ALT)
}

fn saved_order(session: &TuiSession) -> Vec<String> {
    session.core.profile_order().names().to_vec()
}

/// The names of the fleet table's drawn rows, top to bottom. Only the
/// table's own columns are read, so the detail pane never adds a name.
fn drawn_fleet_names(app: &mut TuiApp, names: &[&str]) -> Vec<String> {
    let rows = crate::test_support::draw(app, 120, 40);
    let region = app.regions.fleet_rows;
    (region.y..region.y + region.height)
        .filter_map(|y| {
            let line: String = rows[usize::from(y)]
                .chars()
                .skip(usize::from(region.x))
                .take(usize::from(region.width))
                .collect();
            names
                .iter()
                .find(|name| line.contains(**name))
                .map(|name| (*name).to_owned())
        })
        .collect()
}

/// The Load order has no row in the settings popup: its saved state shows on
/// the message line, at both sizes. A reorder whose write is held says
/// "Load order: saving…" there (the table already shows the new order), and
/// when the write fails the line holds the failure and the table the stored
/// order.
#[test]
fn settings_feedback_tui_load_order_says_saving_then_the_failure_on_the_message_line() {
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-s3-load-order");
    let names = ["alice", "bob", "carol"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    let disk = TuiDisk::install(&session);
    let action = app.on_key(alt_key(KeyCode::Down));
    dispatch(&mut session, &mut app, action);
    session.pump(&mut app);
    assert!(session.core.preferences_pending());
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            words.contains("msg: Load order: saving…"),
            "{width}x{height}: the message line says the order is saving: {words}"
        );
    }

    // The disk fails the held write: the stored order shows again, with the
    // failure on the line.
    drop(disk.release);
    let deadline = Instant::now() + Duration::from_secs(30);
    while session.core.preferences_pending() {
        session.pump(&mut app);
        assert!(Instant::now() < deadline, "the held write never ended");
        std::thread::yield_now();
    }
    session.pump(&mut app);
    for (width, height) in SIZES {
        let words = screen_words(&mut app, width, height);
        assert!(
            words.contains("msg: Not saved:"),
            "{width}x{height}: the failure shows: {words}"
        );
        assert!(
            !words.contains("Load order: saving…"),
            "{width}x{height}: nothing is saving any more: {words}"
        );
    }
    assert_eq!(saved_order(&session), ["alice", "bob", "carol"]);
}

/// Alt+Down and Alt+Up through the real key path change the saved Load
/// order and the drawn fleet table; the cursor stays on the moved profile;
/// Alt+Up on the top row does nothing.
#[test]
fn alt_keys_reorder_the_saved_load_order_and_the_drawn_table() {
    let iso = IsolatedStores::enter("tui-reorder-keys");
    let names = ["alice", "bob", "carol", "dave"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    assert_eq!(app.key_focus, crate::layout::Pane::Fleet);
    assert_eq!(saved_order(&session), ["alice", "bob", "carol", "dave"]);
    let alice = session.core.profile_identity("alice").unwrap();
    let bob = session.core.profile_identity("bob").unwrap();

    // Alt+Down on alice: after the next shown row.
    let action = app.on_key(alt_key(crossterm::event::KeyCode::Down));
    assert_eq!(
        action,
        AppAction::ReorderProfiles(frontend_core::ReorderOp::Move {
            moving: alice,
            to: frontend_core::Placement::After(bob),
        })
    );
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    assert_eq!(saved_order(&session), ["bob", "alice", "carol", "dave"]);
    assert_eq!(
        drawn_fleet_names(&mut app, &names),
        ["bob", "alice", "carol", "dave"],
        "the drawn table follows the saved order"
    );
    assert_eq!(
        app.fleet_model
            .identity(app.table.cursor_row(&app.fleet_model).unwrap()),
        Some(alice),
        "the cursor stays on the moved profile"
    );
    assert_eq!(app.error, None);

    // Alt+Up on alice: back before bob, and the top row now holds alice.
    let action = app.on_key(alt_key(crossterm::event::KeyCode::Up));
    assert_eq!(
        action,
        AppAction::ReorderProfiles(frontend_core::ReorderOp::Move {
            moving: alice,
            to: frontend_core::Placement::Before(bob),
        })
    );
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    assert_eq!(saved_order(&session), ["alice", "bob", "carol", "dave"]);
    assert_eq!(
        drawn_fleet_names(&mut app, &names),
        ["alice", "bob", "carol", "dave"]
    );

    // Alt+Up on the top row is refused through the real dispatcher: one
    // refusal at the command boundary, and nothing moves.
    let refusals = session.core.refusals();
    let up = app.on_key(alt_key(crossterm::event::KeyCode::Up));
    assert!(
        matches!(&up, AppAction::Refused(refusal)
            if refusal.command == Some(OperatorCommand::ReorderProfiles)),
        "{up:?}"
    );
    dispatch(&mut session, &mut app, up);
    assert_eq!(session.core.refusals(), refusals + 1);
    assert_eq!(saved_order(&session), ["alice", "bob", "carol", "dave"]);
}

/// A filter hides the middle profile: Alt+Down on the upper shown row lands
/// after the lower shown row, and the hidden profile keeps its relative
/// place among the others.
#[test]
fn alt_down_with_a_hidden_profile_between_lands_after_the_lower_shown_row() {
    let iso = IsolatedStores::enter("tui-reorder-hidden");
    let names = ["alpha-zq", "bob", "carol-zq", "dave"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    assert_eq!(
        saved_order(&session),
        ["alpha-zq", "bob", "carol-zq", "dave"]
    );
    let open = app.run_command(crate::commands::Command::FilterFleet);
    dispatch(&mut session, &mut app, open);
    for c in "zq".chars() {
        let typed = app.on_key(settings_key(crossterm::event::KeyCode::Char(c)));
        dispatch(&mut session, &mut app, typed);
    }
    let done = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
    dispatch(&mut session, &mut app, done);
    let shown: Vec<&str> = app
        .fleet_model
        .shown_rows()
        .map(|(_, row)| row.name())
        .collect();
    assert_eq!(shown, ["alpha-zq", "carol-zq"]);

    let action = app.on_key(alt_key(crossterm::event::KeyCode::Down));
    let moving = session.core.profile_identity("alpha-zq").unwrap();
    let target = session.core.profile_identity("carol-zq").unwrap();
    assert_eq!(
        action,
        AppAction::ReorderProfiles(frontend_core::ReorderOp::Move {
            moving,
            to: frontend_core::Placement::After(target),
        })
    );
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    assert_eq!(
        saved_order(&session),
        ["bob", "carol-zq", "alpha-zq", "dave"],
        "alpha-zq is after carol-zq; bob and dave keep their relative order"
    );
    let shown: Vec<&str> = app
        .fleet_model
        .shown_rows()
        .map(|(_, row)| row.name())
        .collect();
    assert_eq!(shown, ["carol-zq", "alpha-zq"]);
}

/// Play `moves` (palette commands, or Alt keys) from alice's row on four
/// profiles and return the saved order after each.
fn reorder_run(label: &str, via_keys: bool) -> Vec<Vec<String>> {
    use crate::commands::Command;
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter(label);
    let names = ["alice", "bob", "carol", "dave"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    let mut seen = Vec::new();
    for down in [true, true, true, false] {
        let action = match (via_keys, down) {
            (true, true) => app.on_key(alt_key(KeyCode::Down)),
            (true, false) => app.on_key(alt_key(KeyCode::Up)),
            (false, true) => app.run_command(Command::MoveProfileDown),
            (false, false) => app.run_command(Command::MoveProfileUp),
        };
        dispatch(&mut session, &mut app, action);
        session.core.flush_writes();
        seen.push(saved_order(&session));
    }
    seen
}

/// The palette commands give the same saved order as the keys.
#[test]
fn move_profile_commands_match_the_alt_keys() {
    let keys = reorder_run("tui-reorder-parity-keys", true);
    let palette = reorder_run("tui-reorder-parity-palette", false);
    assert_eq!(keys, palette);
    assert_eq!(
        keys.last().unwrap(),
        &["bob", "carol", "alice", "dave"],
        "down three rows, then up one"
    );
}

/// Move marked to top puts the marked profiles first, in their saved
/// relative order.
#[test]
fn move_marked_to_top_leads_the_saved_order_with_the_marked_profiles() {
    let iso = IsolatedStores::enter("tui-reorder-marked");
    let names = ["alice", "bob", "carol", "dave"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    let before = app.run_command(crate::commands::Command::MoveMarkedToTop);
    assert!(
        matches!(&before, AppAction::Refused(refusal)
            if refusal.command == Some(OperatorCommand::ReorderProfiles)),
        "nothing is marked: {before:?}"
    );
    for name in ["dave", "carol"] {
        dispatch(
            &mut session,
            &mut app,
            AppAction::SetMark(name.into(), true),
        );
    }
    let action = app.run_command(crate::commands::Command::MoveMarkedToTop);
    assert_eq!(
        action,
        AppAction::ReorderProfiles(frontend_core::ReorderOp::MarkedToTop)
    );
    dispatch(&mut session, &mut app, action);
    session.core.flush_writes();
    assert_eq!(saved_order(&session), ["carol", "dave", "alice", "bob"]);
    assert_eq!(
        drawn_fleet_names(&mut app, &names),
        ["carol", "dave", "alice", "bob"]
    );
}

/// Outside Load order the keys and the palette commands are unavailable
/// and submit nothing; a reorder that reaches the core anyway is refused
/// and the saved order stays.
#[test]
fn reorder_is_unavailable_outside_load_order() {
    use crate::commands::Command;
    let iso = IsolatedStores::enter("tui-reorder-sorted");
    let names = ["alice", "bob", "carol", "dave"];
    let (mut session, mut app) = tui_with_profiles(&iso, &names);
    drawn_fleet_names(&mut app, &names);
    dispatch(
        &mut session,
        &mut app,
        AppAction::SetMark("bob".into(), true),
    );
    app.fleet_model
        .set_order_for_test(frontend_core::FleetOrder::Name);
    for command in [
        Command::MoveProfileUp,
        Command::MoveProfileDown,
        Command::MoveMarkedToTop,
    ] {
        assert_eq!(
            command.availability(&app),
            Err(crate::commands::Unavailable::NotLoadOrder),
            "{command:?}"
        );
        assert!(
            matches!(&app.run_command(command), AppAction::Refused(refusal)
                if refusal.command == Some(OperatorCommand::ReorderProfiles)),
            "{command:?}"
        );
    }
    for code in [
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyCode::Down,
    ] {
        assert!(matches!(
            &app.on_key(alt_key(code)),
            AppAction::Refused(refusal) if refusal.command == Some(OperatorCommand::ReorderProfiles)
        ));
    }
    assert_eq!(saved_order(&session), ["alice", "bob", "carol", "dave"]);

    // The core refuses the same op when the model does not show Load order.
    let alice = session.core.profile_identity("alice").unwrap();
    let bob = session.core.profile_identity("bob").unwrap();
    app.error = None;
    dispatch(
        &mut session,
        &mut app,
        AppAction::ReorderProfiles(frontend_core::ReorderOp::Move {
            moving: alice,
            to: frontend_core::Placement::After(bob),
        }),
    );
    session.core.flush_writes();
    assert_eq!(saved_order(&session), ["alice", "bob", "carol", "dave"]);
    let error = app.error.clone().expect("the refusal is shown");
    assert!(error.contains("choose Load order to reorder"), "{error}");
}

/// One key through the real path: `on_key`, then the dispatcher.
fn press_key(session: &mut TuiSession, app: &mut TuiApp, key: crossterm::event::KeyEvent) {
    let action = app.on_key(key);
    dispatch(session, app, action);
}

#[cfg(feature = "js")]
/// Open the fleet-card picker with `c`, walk to `card` with Down and pick
/// it with Enter.
fn pick_fleet_card_with_keys(session: &mut TuiSession, app: &mut TuiApp, card: &ScriptChoice) {
    use crate::test_support::{ch, key};
    press_key(session, app, ch('c'));
    let want = frontend_core::FleetCard::pinned(card.duplicate());
    for _ in 0..200 {
        let Some(crate::overlay::Modal::FleetCard(picker)) = &app.modal else {
            panic!("the picker is open");
        };
        if picker.highlighted().as_ref() == Some(&want) {
            break;
        }
        press_key(session, app, key(crossterm::event::KeyCode::Down));
    }
    press_key(session, app, key(crossterm::event::KeyCode::Enter));
}

/// `c` + navigation + Enter pins a card; Esc keeps the card; neither
/// touches the Script tab's Browse pick, the focused bot's pending
/// heading, an assignment or a run. The header names the card and
/// follows the focus for the Focused card.
#[cfg(feature = "js")]
#[test]
fn c_picks_a_fleet_card_with_keys_and_esc_keeps_it_without_touching_the_browse_pick() {
    use crate::test_support::{ch, key};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-fleet-card-pick");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let thiever_path = iso.dir().join("thiever.ts");
    std::fs::write(&thiever_path, THIEVER_TS).unwrap();
    let thiever = session.scripts.load_card(&thiever_path).unwrap();
    assign(&mut session, "alice", &thiever);
    focus_member(&mut session, &mut app, "alice");

    // Registry cards have short labels that fit the header.
    let compiled: Vec<ScriptChoice> = app
        .script_card_choices
        .iter()
        .filter(|choice| choice.compiled_id().is_some())
        .map(ScriptChoice::duplicate)
        .collect();
    assert!(compiled.len() >= 2, "the registry lists compiled cards");
    // alice's heading is a pending Browse pick of the first one.
    let pending = compiled[0].duplicate();
    app.script_sel = Some(pending.duplicate());
    app.browse_changed = true;
    session.pump(&mut app);
    let heading = |session: &TuiSession, name: &str| session.scripts.heading(&session.core, name);
    assert_eq!(heading(&session, "alice"), Some(pending.duplicate()));
    let assignment = |session: &TuiSession, name: &str| {
        frontend_core::Scripts::test_assignment(&session.core, name).map(|a| a.key())
    };
    let browse_before = session
        .scripts
        .browse_selection()
        .map(ScriptChoice::duplicate);

    // Another registry card is pinned.
    let target = compiled[1].duplicate();
    pick_fleet_card_with_keys(&mut session, &mut app, &target);
    session.pump(&mut app);
    assert!(app.modal.is_none());
    assert_eq!(
        app.fleet_model.card(),
        &frontend_core::FleetCard::pinned(target.duplicate()),
        "Enter pinned the card"
    );
    let shown = screen_text(&mut app, 120, 40);
    assert!(
        shown.contains(&format!("card: {}", target.label())),
        "the header names the pinned card: {shown}"
    );
    assert!(shown.contains("sort: Load order"), "{shown}");
    let unchanged = |session: &TuiSession, app: &TuiApp| {
        assert_eq!(
            heading(session, "alice"),
            Some(pending.duplicate()),
            "the focused bot's pending heading"
        );
        assert_eq!(
            session
                .scripts
                .browse_selection()
                .map(ScriptChoice::duplicate),
            browse_before.as_ref().map(ScriptChoice::duplicate),
            "the Browse pick"
        );
        assert_eq!(
            app.script_sel.as_ref(),
            Some(&pending),
            "the Script tab pick"
        );
        assert_eq!(
            assignment(session, "alice"),
            Some(thiever.identity_key().to_owned()),
            "alice keeps her assignment"
        );
        assert_eq!(assignment(session, "bob"), None);
        for name in ["alice", "bob"] {
            assert_eq!(session.core.script_state(name), ScriptRunState::Idle);
        }
        assert!(session.core.fleet_plan().is_none(), "picking plans nothing");
        assert!(session.core.fleet_result().is_none(), "and runs nothing");
    };
    unchanged(&session, &app);

    // Esc keeps the pinned card.
    press_key(&mut session, &mut app, ch('c'));
    press_key(&mut session, &mut app, key(KeyCode::Down));
    press_key(&mut session, &mut app, key(KeyCode::Esc));
    session.pump(&mut app);
    assert!(app.modal.is_none());
    assert_eq!(
        app.fleet_model.card(),
        &frontend_core::FleetCard::pinned(target.duplicate()),
        "Esc left the card"
    );
    unchanged(&session, &app);

    // Focused follows the focus: alice's pending pick, then bob's none.
    press_key(&mut session, &mut app, ch('c'));
    press_key(&mut session, &mut app, key(KeyCode::Home));
    press_key(&mut session, &mut app, key(KeyCode::Down));
    press_key(&mut session, &mut app, key(KeyCode::Enter));
    session.pump(&mut app);
    assert_eq!(app.fleet_model.card(), &frontend_core::FleetCard::FOCUSED);
    let shown = screen_text(&mut app, 120, 40);
    // The 34-cell pane wraps the long label at its spaces.
    assert!(shown.contains("card: Focused bot's card:"), "{shown}");
    assert!(
        shown.contains(pending.label()) && shown.contains("(pending)"),
        "{shown}"
    );
    focus_member(&mut session, &mut app, "bob");
    let shown = screen_text(&mut app, 120, 40);
    assert!(
        shown.contains("card: Focused bot's card: none"),
        "the label follows the focus each loop: {shown}"
    );
}

/// `a` and `A` mark and unmark only the shown rows: a mark the filter hides
/// stays. A status mark from the palette marks the rows in that status.
#[test]
fn a_and_capital_a_mark_only_shown_rows_and_a_status_mark_picks_the_right_rows() {
    use crate::test_support::{ch, ctrl, key};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-fleet-marks");
    let (mut session, mut app) = tui_with_loaded(
        &iso,
        &["alice", "bob", "carol", "bobby", "zed"],
        &["alice", "bob", "carol", "bobby"],
    );
    let id = |session: &TuiSession, name: &str| session.core.profile_identity(name).unwrap();
    let marked = |session: &TuiSession, app: &TuiApp| {
        ["alice", "bob", "carol", "bobby", "zed"]
            .into_iter()
            .filter(|name| app.fleet_model.marks().contains(id(session, name)))
            .collect::<Vec<_>>()
    };

    // alice is marked, then hidden by a filter that shows bob and bobby.
    press_key(&mut session, &mut app, ch(' '));
    assert_eq!(marked(&session, &app), ["alice"]);
    press_key(&mut session, &mut app, ch('/'));
    for c in "bob".chars() {
        press_key(&mut session, &mut app, ch(c));
    }
    press_key(&mut session, &mut app, key(KeyCode::Enter));
    let shown: Vec<&str> = app
        .fleet_model
        .shown_rows()
        .map(|(_, row)| row.name())
        .collect();
    assert_eq!(shown, ["bob", "bobby"]);

    press_key(&mut session, &mut app, ch('a'));
    assert_eq!(marked(&session, &app), ["alice", "bob", "bobby"]);
    let text = screen_text(&mut app, 120, 40);
    assert!(text.contains("3 marked"), "the header counts them: {text}");
    assert_eq!(app.fleet_model.counts().hidden_marked(), 1);

    press_key(&mut session, &mut app, ch('A'));
    assert_eq!(
        marked(&session, &app),
        ["alice"],
        "A unmarks the shown rows only; the hidden mark stays"
    );

    // Clear the filter (Esc) and mark by status from the palette.
    press_key(&mut session, &mut app, key(KeyCode::Esc));
    assert_eq!(app.fleet_model.shown().len(), 5);
    let palette_run = |session: &mut TuiSession, app: &mut TuiApp, query: &str| {
        press_key(session, app, ctrl('p'));
        for c in query.chars() {
            press_key(session, app, ch(c));
        }
        press_key(session, app, key(KeyCode::Enter));
    };
    palette_run(&mut session, &mut app, "status: not loaded");
    assert_eq!(
        marked(&session, &app),
        ["alice", "zed"],
        "only the profile that is not loaded joins the marks"
    );
    palette_run(&mut session, &mut app, "status: logged out");
    assert_eq!(
        marked(&session, &app),
        ["alice", "bob", "carol", "bobby", "zed"],
        "the loaded bots that are not logged in join too"
    );
    palette_run(&mut session, &mut app, "status: running");
    assert_eq!(marked(&session, &app).len(), 5, "no bot runs a script");
}

/// Stop with nothing marked: with no script to stop core refuses and the
/// message line says why; with two running scripts the overlay shows
/// core's prompt as it is, Esc runs nothing and leaves no plan, and
/// confirming runs the Stop and shows its settled result on the message
/// line.
#[cfg(feature = "js")]
#[test]
fn e_with_nothing_marked_shows_the_core_prompt_and_confirm_runs_it() {
    use crate::test_support::{ch, key};
    use crossterm::event::KeyCode;
    let iso = IsolatedStores::enter("tui-fleet-stop-all");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let path = iso.dir().join("shared.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    assign(&mut session, "alice", &card);
    assign(&mut session, "bob", &card);
    press_key(&mut session, &mut app, key(KeyCode::F(5)));
    assert_eq!(app.screen, crate::layout::Screen::Script);

    // Nothing runs: a refusal with its reason, no overlay, no plan.
    press_key(&mut session, &mut app, ch('E'));
    let refusal = app.error.clone().expect("the refusal shows");
    assert!(refusal.starts_with("Stop: no bot to act on"), "{refusal}");
    assert!(app.modal.is_none());
    assert!(session.core.fleet_plan().is_none());

    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Start);
    settle_starts(&mut session, &mut app);
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
    }
    let started = session
        .core
        .fleet_result()
        .expect("the Start result")
        .request();

    // E: two shown bots run a script, so core asks first with its prompt.
    // The loop pumps every frame: the fleet rows now show both running.
    session.pump(&mut app);
    press_key(&mut session, &mut app, ch('E'));
    let prompt = session
        .core
        .fleet_plan()
        .expect("the plan waits for the answer")
        .prompt()
        .to_owned();
    assert!(
        prompt.starts_with("Stop 2 shown bots: alice, bob.")
            && prompt.contains("This stops the running script of alice, bob"),
        "{prompt}"
    );
    assert!(matches!(
        &app.modal,
        Some(crate::overlay::Modal::Confirm(confirm))
            if matches!(&confirm.kind, crate::overlay::ConfirmKind::FleetPlan { prompt: shown, .. } if *shown == prompt)
    ));
    let flat = |text: String| text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (w, h) in SIZES {
        let shown = flat(screen_text(&mut app, w, h));
        assert!(
            shown.contains(&prompt),
            "{w}x{h}: the prompt verbatim: {shown}"
        );
    }

    // Esc runs nothing and leaves no plan.
    press_key(&mut session, &mut app, key(KeyCode::Esc));
    assert!(app.modal.is_none());
    assert!(session.core.fleet_plan().is_none(), "Esc leaves no plan");
    assert_eq!(
        session.core.fleet_result().unwrap().request(),
        started,
        "nothing ran"
    );
    for name in ["alice", "bob"] {
        assert_eq!(session.core.script_state(name), ScriptRunState::Running);
    }

    // Confirm runs it: the scripts stop and the message line shows the
    // settled result, once.
    press_key(&mut session, &mut app, ch('E'));
    assert!(app.modal.is_some());
    press_key(&mut session, &mut app, ch('y'));
    assert!(session.core.fleet_plan().is_none(), "the plan was taken");
    for name in ["alice", "bob"] {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Idle,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.pump(&mut app);
        if session.core.fleet_result().is_some_and(|r| r.settled()) {
            break;
        }
        assert!(Instant::now() < deadline, "the Stop result did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = session.core.fleet_result().unwrap();
    assert_ne!(result.request(), started);
    assert_eq!(result.action(), frontend_core::FleetAction::Stop);
    let text = result.text().to_owned();
    assert!(
        text.starts_with("Stop: done 2: alice, bob"),
        "the one result: {text}"
    );
    assert_eq!(app.error.as_deref(), Some(text.as_str()));
    let rows = crate::test_support::draw(&mut app, 120, 40);
    assert!(
        rows.iter().any(|row| row.contains(&format!("msg: {text}"))),
        "the message line shows it: {}",
        crate::test_support::text(&rows)
    );
}

/// A marked Assign with a pinned card needs no confirmation and assigns
/// only the marked profiles. A plan made first does not take a profile
/// marked after it: drive it plan, then mark, then confirm.
#[cfg(feature = "js")]
#[test]
fn a_marked_assign_runs_at_once_and_a_late_mark_never_joins_the_plan() {
    let iso = IsolatedStores::enter("tui-fleet-assign-plan");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob", "carol", "dave"]);
    let path = iso.dir().join("looping.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    let pinned = ScriptChoice::loaded(CardSource::File, card.identity_id());
    dispatch(
        &mut session,
        &mut app,
        AppAction::PickFleetCard(frontend_core::FleetCard::pinned(pinned)),
    );
    let assigned = |session: &mut TuiSession, name: &str| {
        session.core.flush_writes();
        frontend_core::Scripts::test_assignment(&session.core, name).map(|a| a.key())
    };
    let key = Some(card.identity_key().to_owned());
    for name in ["alice", "bob"] {
        let id = session.core.profile_identity(name).unwrap();
        app.fleet_model.set_mark_for_test(id, true);
    }
    app.names = vec!["alice".into(), "bob".into(), "carol".into(), "dave".into()];
    app.focused = Some(0);
    assert!(app.script_sel.is_none(), "alice has no card yet");

    // Marked and safe: no overlay, no waiting plan, only the marked.
    let action = app.run_command(crate::commands::Command::Fleet(
        frontend_core::FleetAction::Assign,
    ));
    dispatch(&mut session, &mut app, action);
    assert!(app.modal.is_none(), "a marked Assign runs at once");
    assert!(session.core.fleet_plan().is_none());
    assert_eq!(
        app.script_sel
            .as_ref()
            .and_then(ScriptChoice::loaded_card)
            .map(|(_, id)| id.to_owned()),
        Some(card.identity_id().to_owned()),
        "the focused bot's Script heading follows its new assignment"
    );
    assert_eq!(assigned(&mut session, "alice"), key);
    assert_eq!(assigned(&mut session, "bob"), key);
    assert_eq!(assigned(&mut session, "carol"), None);
    assert_eq!(assigned(&mut session, "dave"), None);
    settle_fleet_result(&mut session, &mut app);
    let result = app.error.clone().expect("the result shows");
    assert!(
        result.starts_with("Assign: ") && result.contains("alice, bob"),
        "{result}"
    );

    // Plan for carol only, then mark dave, then confirm.
    dispatch(&mut session, &mut app, AppAction::ClearMarks);
    let carol = session.core.profile_identity("carol").unwrap();
    app.fleet_model.set_mark_for_test(carol, true);
    session
        .submit_with(
            &mut app,
            OperatorCommand::PlanFleetAction(frontend_core::FleetAction::Assign),
            Targets::Application,
        )
        .expect("the plan is made");
    assert!(session.core.fleet_plan().is_some());
    dispatch(
        &mut session,
        &mut app,
        AppAction::SetMark("dave".into(), true),
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::RunFleetPlan(frontend_core::FleetAction::Assign),
    );
    assert_eq!(assigned(&mut session, "carol"), key, "the frozen target");
    assert_eq!(
        assigned(&mut session, "dave"),
        None,
        "a mark made after the plan never joins it"
    );
}

/// Review F3 r2-2: two emptied plans with the same skip text each show
/// their result, even when another message replaced the first between them
/// (an emptied plan has no command record to tell the results apart).
#[test]
fn a_second_identical_emptied_plan_shows_its_result_again() {
    let iso = IsolatedStores::enter("tui-emptied-plan-twice");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    let alice = session
        .core
        .vault_mut()
        .and_then(|vault| vault.get("alice"))
        .cloned()
        .expect("alice");
    let emptied_login = |session: &mut TuiSession, app: &mut TuiApp| {
        session.pump(app);
        dispatch(
            session,
            app,
            AppAction::PlanFleet(frontend_core::FleetAction::Login),
        );
        assert!(
            matches!(
                &app.modal,
                Some(crate::overlay::Modal::Confirm(confirm))
                    if matches!(confirm.kind, crate::overlay::ConfirmKind::FleetPlan { .. })
            ),
            "a no-marks Log in asks first"
        );
        // The plan's only target is gone before Confirm.
        assert!(session
            .core
            .vault_mut()
            .expect("vault")
            .remove("alice")
            .expect("remove"));
        let confirm = app.on_key(settings_key(crossterm::event::KeyCode::Enter));
        dispatch(session, app, confirm);
    };
    let skipped = "Log in: skipped 1: alice (profile no longer exists)";
    emptied_login(&mut session, &mut app);
    assert_eq!(app.error.as_deref(), Some(skipped));
    // Another action replaces the message line.
    app.error = Some("map: no tile selected".into());
    session
        .core
        .vault_mut()
        .expect("vault")
        .upsert(alice)
        .expect("alice again");
    emptied_login(&mut session, &mut app);
    assert_eq!(
        app.error.as_deref(),
        Some(skipped),
        "the second emptied plan's result shows"
    );
}

/// PANEL-STATE-GAP-1023, acceptance 7 (TUI half): the same nine bots as the
/// panel's test. The Fleet pane draws the label the core projects, Stop
/// retires the three script walks, and the Stop decision comes from the same
/// facts the label does.
#[cfg(feature = "js")]
#[test]
fn tui_fleet_pane_and_stop_agree_with_the_core_after_nine_bots_stop() {
    use crate::fleet::{FleetState, FleetTable};
    use crate::layout::SizeClass;
    const NINE: [&str; 9] = [
        "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan",
    ];
    let iso = IsolatedStores::enter("tui-state-gap");
    let (mut session, mut app) = tui_with_profiles(&iso, &NINE);
    {
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        for name in NINE {
            rows.retain(|row| row.username != name);
            rows.push(host_play::SlotStatus {
                username: name.into(),
                connected: true,
                ingame: true,
                scene_state: 2,
                ..host_play::SlotStatus::default()
            });
        }
    }
    let path = iso.dir().join("loop.ts");
    std::fs::write(&path, LOOPING_TS).unwrap();
    let card = session.scripts.load_card(&path).unwrap();
    for name in NINE {
        assign(&mut session, name, &card);
    }
    for name in NINE {
        start_assigned_script(&mut session, &mut app, name);
    }
    settle_starts(&mut session, &mut app);
    for name in NINE {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Running,
        );
    }
    {
        let mut rows = session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        for row in rows
            .iter_mut()
            .filter(|row| NINE[..3].contains(&row.username.as_str()))
        {
            (row.walk_x, row.walk_z, row.walk_level) = (2550, 3348, 0);
        }
    }
    for name in NINE {
        session
            .submit_with(
                &mut app,
                OperatorCommand::SetMark(true),
                Targets::Profile(name),
            )
            .expect("mark");
    }
    session.pump(&mut app);
    assert_eq!(app.fleet_model.counts().marked(), 9);

    // What the Fleet pane draws for each bot, one text line per row.
    let drawn = |app: &TuiApp| -> Vec<String> {
        let area = ratatui::layout::Rect::new(0, 0, 80, 20);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        FleetTable {
            model: &app.fleet_model,
            restart_badges: &std::collections::HashSet::new(),
            state: &mut FleetState::default(),
            selected: None,
            keys: false,
            class: SizeClass::Standard,
            header_parts: &mut Vec::new(),
        }
        .render(area, &mut buf, 0);
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .filter(|line| NINE.iter().any(|name| line.contains(name)))
            .collect()
    };
    let state_of = |line: &str| {
        line.split_whitespace()
            .find(|word| ["running", "idle"].contains(word))
            .map(str::to_owned)
    };
    let lines = drawn(&app);
    assert_eq!(lines.len(), 9, "{lines:?}");
    assert!(
        lines
            .iter()
            .all(|line| state_of(line).as_deref() == Some("running")),
        "{lines:?}"
    );

    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Stop);
    for name in NINE {
        wait_script_state(
            session.core.play_for_test().unwrap(),
            name,
            ScriptRunState::Idle,
        );
    }
    settle_fleet_result(&mut session, &mut app);
    session.pump(&mut app);
    let settled = app.error.clone().unwrap_or_default();
    assert!(settled.starts_with("Stop: done 9"), "{settled}");

    let lines = drawn(&app);
    assert_eq!(lines.len(), 9, "{lines:?}");
    assert!(
        lines
            .iter()
            .all(|line| state_of(line).as_deref() == Some("idle")),
        "the TUI drew a stale walking label: {lines:?}"
    );
    for row in app.fleet_model.rows() {
        assert_eq!(
            (
                row.brief(),
                row.script().is_live(),
                row.script_walk(),
                row.connected()
            ),
            ("idle", false, false, true),
            "{}",
            row.name()
        );
        let bot = session
            .core
            .bot_projection(Some(&session.scripts), row.name())
            .expect("a loaded bot");
        assert_eq!(bot.label(), row.brief());
        assert_eq!(
            bot.stop_eligibility(),
            frontend_core::StopEligibility::NoScript
        );
    }
    for status in session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .iter()
    {
        assert_eq!(status.script_walk(), None, "{}", status.username);
    }

    // Stop again: nothing to stop, and the refusal names the same fact.
    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Stop);
    let refusal = app.error.clone().unwrap_or_default();
    assert!(
        refusal.contains("no bot to act on") && refusal.contains("alice: no script running"),
        "{refusal}"
    );
    assert_eq!(
        session.core.refusals(),
        1,
        "the repeated Stop is the one refusal the command boundary logged"
    );
    session.pump(&mut app);
    session.expire_error(
        &mut app,
        Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT + Duration::from_secs(1),
    );
    assert_eq!(app.error, None, "the refusal message expired");
}

/// The message line stays until an action resolves it, a newer message
/// replaces it, or the shared timeout passes: the same rule as the panel's
/// banner (`frontend_core::ERROR_MESSAGE_TIMEOUT`).
#[test]
fn the_message_line_expires_unless_replaced() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;
    let second = Duration::from_secs(1);
    let mut session = TuiSession::new(dummy_options());
    let mut app = TuiApp::new("274bot headless");
    let t0 = Instant::now();

    app.error = Some("first".into());
    session.expire_error(&mut app, t0);
    session.expire_error(&mut app, t0 + TIMEOUT - second);
    assert_eq!(app.error.as_deref(), Some("first"));
    session.expire_error(&mut app, t0 + TIMEOUT);
    assert_eq!(app.error, None);

    app.error = Some("older".into());
    session.expire_error(&mut app, t0);
    app.error = Some("newer".into());
    session.expire_error(&mut app, t0 + TIMEOUT - second);
    session.expire_error(&mut app, t0 + TIMEOUT + second);
    assert_eq!(app.error.as_deref(), Some("newer"), "the timer restarted");
    session.expire_error(&mut app, t0 + TIMEOUT * 2 - second);
    assert_eq!(app.error, None);
}

#[test]
fn retained_preference_failure_expires_without_reappearing_and_clears_on_recovery() {
    let iso = IsolatedStores::enter("tui-preference-error-expiry");
    for expire in [false, true] {
        let state = iso.dir().join(if expire { "expired" } else { "recovered" });
        std::fs::create_dir_all(&state).unwrap();
        let path = state.join("panel-ui.json");
        std::fs::write(&path, b"not json").unwrap();
        let service = host_play::local_state::LocalStateService::open_process(
            host_play::local_state::StateRoot::new(state),
        );
        let mut session = TuiSession::new(dummy_options());
        session.core.bind_local_state(service);
        session.preferences = session.core.preferences().unwrap();
        let mut app = TuiApp::new("274bot headless");
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.error.is_none() && Instant::now() < deadline {
            session.pump(&mut app);
            std::thread::sleep(Duration::from_millis(5));
        }
        let failure = session.core.preference_error().unwrap().to_owned();
        assert_eq!(app.error.as_deref(), Some(failure.as_str()));
        let captures = std::env::var_os("TUI_CAPTURE_DIR").map(PathBuf::from);
        if let Some(dir) = captures.as_deref() {
            std::fs::create_dir_all(dir).unwrap();
        }
        capture_message_line(
            captures.as_deref(),
            &session,
            &mut app,
            if expire {
                "preferences-before-timeout"
            } else {
                "preferences-before-recovery"
            },
            Some(&failure),
            &failure,
        );

        if expire {
            let now = Instant::now();
            session.expire_error(&mut app, now);
            session.expire_error(&mut app, now + frontend_core::ERROR_MESSAGE_TIMEOUT);
            assert_eq!(app.error, None);
            for _ in 0..4 {
                session.pump(&mut app);
                assert_eq!(app.error, None, "the retained failure must not reappear");
            }
            capture_message_line(
                captures.as_deref(),
                &session,
                &mut app,
                "preferences-expired-without-reappearing",
                None,
                &failure,
            );
            app.error = Some("newer message".into());
        }

        std::fs::write(&path, b"{}").unwrap();
        session.preferences.reload().unwrap();
        session.pump(&mut app);
        session.pump(&mut app);
        assert!(session.core.preference_error().is_none());
        assert_eq!(
            app.error.as_deref(),
            expire.then_some("newer message"),
            "recovery clears only the message for that failure"
        );
        capture_message_line(
            captures.as_deref(),
            &session,
            &mut app,
            if expire {
                "preferences-recovered-newer-message-kept"
            } else {
                "preferences-recovered-error-cleared"
            },
            expire.then_some("newer message"),
            &failure,
        );
    }
}

/// PANEL-STATE-GAP-1023: the terminal app's own refusals (Start with nothing
/// focused, nothing browsed, no bots checked) log one `info` line at the
/// shared command boundary, naming the command, the bots and the reason, and
/// the action that resolves each one clears the message line at once.
#[test]
fn a_refusal_before_submission_logs_once_and_the_resolving_action_clears_it() {
    use crate::app::Refusal;
    use frontend_core::log::{global, LogScope, LogView};
    let iso = IsolatedStores::enter("tui-resolve-refusal");
    let mut session = TuiSession::new(dummy_options());
    session.persist_ui = false;
    session.inject_play(two_arm_play());
    session.names = vec!["alice".into(), "bob".into()];
    let mut app = TuiApp::new("tui");
    app.names = session.names.clone();
    let sel = ScriptChoice::loaded(CardSource::Catalog, "BoneBurier");
    let before = session.core.refusals();

    // Start with nothing focused: refused at the boundary, once.
    dispatch(
        &mut session,
        &mut app,
        AppAction::ScriptStart(sel.duplicate()),
    );
    assert_eq!(app.error.as_deref(), Some("script: no focused profile"));
    assert_eq!(session.core.refusals(), before + 1);
    let mut view = LogView::new(LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused: StartScript on no bots"));
    global().refresh(&mut view);
    assert!(
        view.rows()
            .iter()
            .any(|row| row.message().ends_with("script: no focused profile")),
        "{}",
        view.to_text()
    );
    // Focusing a bot resolves it.
    app.focused = Some(0);
    dispatch(&mut session, &mut app, AppAction::Focus("alice".into()));
    assert_eq!(app.error, None, "focusing a bot resolved the message");

    // The key for Start with no card browsed refuses the same way; picking
    // a card resolves it.
    dispatch(
        &mut session,
        &mut app,
        AppAction::Refused(Refusal {
            command: Some(OperatorCommand::StartScript),
            bot: Some("alice".into()),
            reason: "script: browse to pick one first".into(),
            need: crate::commands::Unavailable::NoScriptPicked.need(),
        }),
    );
    assert_eq!(
        app.error.as_deref(),
        Some("script: browse to pick one first")
    );
    assert_eq!(session.core.refusals(), before + 2);
    session.pump(&mut app);
    assert!(app.error.is_some(), "nothing was picked yet");
    app.script_sel = Some(sel);
    session.pump(&mut app);
    assert_eq!(app.error, None, "picking a card resolved the message");
    let _iso = iso;
}

/// Draw the app at 120x40 and 80x24 and check its message line (`msg: …`,
/// one row, cut at the width): it holds the start of `shown`, or, with
/// `None`, is empty (`msg: —`) and holds no `gone`. The log pane may still
/// show the refusal's line. With `dir`, each frame is also written there by
/// the terminal app's capture harness (`manual_click_live_tests::capture`:
/// text and JSON cell receipts). Returns the frames drawn.
fn capture_message_line(
    dir: Option<&std::path::Path>,
    session: &TuiSession,
    app: &mut TuiApp,
    label: &str,
    shown: Option<&str>,
    gone: &str,
) -> usize {
    let sizes = [(120, 40), (80, 24)];
    for (cols, rows) in sizes {
        let text = crate::test_support::text(&crate::test_support::draw(app, cols, rows));
        let head = |message: &str| {
            let start: String = message.chars().take(40).collect();
            format!("msg: {start}")
        };
        match shown {
            Some(message) => assert!(
                text.contains(&head(message)),
                "{label} {cols}x{rows}:\n{text}"
            ),
            None => assert!(
                text.contains("msg: —") && !text.contains(&head(gone)),
                "{label} {cols}x{rows}:\n{text}"
            ),
        }
        if let Some(dir) = dir {
            let proof = serde_json::json!({
                "label": label,
                "message": app.error,
                "refusals": session.core.refusals(),
            });
            super::manual_click_live_tests::capture(
                app,
                cols,
                rows,
                dir,
                &format!("{cols}x{rows}"),
                label,
                &proof,
            )
            .unwrap();
        }
    }
    sizes.len()
}

/// PANEL-STATE-GAP-1023 (R2-M1): the message line at 120x40 and 80x24 for
/// the changed messages. A refusal before submission and a refusal at
/// submission show their reason, stay through 14 seconds and are gone at
/// the shared timeout; a refusal that waits for a focus clears at once when
/// a bot is focused. With `TUI_CAPTURE_DIR` set, every frame is also written
/// there by the terminal app's capture harness
/// (`manual_click_live_tests::capture`: text and JSON cell receipts).
#[test]
fn the_message_line_shows_and_expires_refusals_at_both_sizes() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;
    let iso = IsolatedStores::enter("tui-message-captures");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let dir = std::env::var_os("TUI_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = dir.as_deref() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let second = Duration::from_secs(1);
    let mut frames = 0;
    let mut shot =
        |session: &TuiSession, app: &mut TuiApp, label: &str, shown: Option<&str>, gone: &str| {
            frames += capture_message_line(dir.as_deref(), session, app, label, shown, gone);
        };
    let before = session.core.refusals();

    // A refusal before submission: Start with no bot focused.
    let unfocused = "script: no focused profile";
    dispatch(
        &mut session,
        &mut app,
        AppAction::ScriptStart(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
    );
    assert_eq!(app.error.as_deref(), Some(unfocused));
    assert_eq!(session.core.refusals(), before + 1);
    let t0 = Instant::now();
    session.expire_error(&mut app, t0);
    shot(
        &session,
        &mut app,
        "01-refused-before-submit-0s",
        Some(unfocused),
        unfocused,
    );
    session.expire_error(&mut app, t0 + TIMEOUT - second);
    shot(
        &session,
        &mut app,
        "02-refused-before-submit-14s",
        Some(unfocused),
        unfocused,
    );
    session.expire_error(&mut app, t0 + TIMEOUT);
    assert_eq!(app.error, None);
    shot(
        &session,
        &mut app,
        "03-refused-before-submit-15s-expired",
        None,
        unfocused,
    );

    // A refusal at submission: Stop with no script running.
    run_fleet_action(&mut session, &mut app, frontend_core::FleetAction::Stop);
    let stop = app.error.clone().unwrap_or_default();
    assert!(stop.contains("no script running"), "{stop}");
    assert_eq!(session.core.refusals(), before + 2);
    assert!(stop.contains("no bot to act on"), "{stop}");
    let shown = stop.as_str();
    let t1 = Instant::now();
    session.expire_error(&mut app, t1);
    shot(
        &session,
        &mut app,
        "04-refused-at-submit-0s",
        Some(shown),
        shown,
    );
    session.expire_error(&mut app, t1 + TIMEOUT - second);
    shot(
        &session,
        &mut app,
        "05-refused-at-submit-14s",
        Some(shown),
        shown,
    );
    session.expire_error(&mut app, t1 + TIMEOUT);
    assert_eq!(app.error, None);
    shot(
        &session,
        &mut app,
        "06-refused-at-submit-15s-expired",
        None,
        shown,
    );

    // The resolving action clears a refusal before its timeout.
    dispatch(
        &mut session,
        &mut app,
        AppAction::ScriptStart(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
    );
    assert_eq!(app.error.as_deref(), Some(unfocused));
    shot(
        &session,
        &mut app,
        "07-refused-waiting-for-focus",
        Some(unfocused),
        unfocused,
    );
    app.focused = Some(0);
    dispatch(&mut session, &mut app, AppAction::Focus("alice".into()));
    assert_eq!(app.error, None, "focusing a bot resolved the message");
    shot(&session, &mut app, "08-resolved-by-focus", None, unfocused);

    // A key whose command is unavailable: refused at the boundary too.
    let keyed = "Stop script: no script running";
    let action = app.run_command(crate::commands::Command::ScriptStop);
    dispatch(&mut session, &mut app, action);
    assert_eq!(app.error.as_deref(), Some(keyed));
    assert_eq!(session.core.refusals(), before + 4);
    let t2 = Instant::now();
    session.expire_error(&mut app, t2);
    shot(
        &session,
        &mut app,
        "09-refused-by-key-0s",
        Some(keyed),
        keyed,
    );
    session.expire_error(&mut app, t2 + TIMEOUT);
    shot(
        &session,
        &mut app,
        "10-refused-by-key-15s-expired",
        None,
        keyed,
    );
    assert_eq!(frames, 20);
    let _iso = iso;
}

/// The `p` toggle needs a script paint to toggle: with none the command is
/// unavailable and its refusal says what is missing. The key must not toggle
/// game chat with nothing to toggle.
#[test]
fn toggle_game_chat_refuses_without_a_script_paint() {
    let mut app = crate::test_support::fleet_app(&["alice"]);
    app.chat_data.script_paint = None;
    assert_eq!(
        crate::commands::Command::ToggleGameChat.availability(&app),
        Err(crate::commands::Unavailable::NoScriptPaint)
    );
    assert_eq!(
        crate::commands::Unavailable::NoScriptPaint.text(),
        "no script paint to toggle"
    );
}

/// Whether the process log holds a refusal line containing `line`. The log
/// redacts every registered password (the short ones other tests register
/// too, such as "test"), wherever it appears: so a line that holds a path is
/// compared after the same redaction, whatever the temporary folder is called.
fn refusal_logged(line: &str) -> bool {
    use frontend_core::log::{global, LogScope, LogView};
    let mut view = LogView::new(LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused: "));
    global().refresh(&mut view);
    let wanted = api::hostlog::redact(line);
    view.rows().iter().any(|row| {
        row.message().starts_with("command refused: ")
            && api::hostlog::redact(row.message()).contains(&*wanted)
    })
}

/// PANEL-STATE-GAP-1023 (F2): a command the terminal app refuses before it
/// submits logs one `info` line at the shared command boundary, naming the
/// command, the bot and the reason: a key whose command is unavailable, the
/// same command from the palette, Enter on a profile that is not loaded, and
/// a Confirm with no plan waiting.
#[test]
fn the_terminal_apps_own_refusals_log_at_the_command_boundary() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let iso = IsolatedStores::enter("tui-own-refusals");
    let (mut session, mut app) = tui_with_loaded(&iso, &["psgalice", "psgzed"], &["psgalice"]);
    app.focused = app
        .fleet_model
        .rows()
        .iter()
        .position(|row| row.name() == "psgalice");
    dispatch(&mut session, &mut app, AppAction::Focus("psgalice".into()));
    let before = session.core.refusals();

    // A key (the Script pane's Stop) with no script running.
    let action = app.run_command(crate::commands::Command::ScriptStop);
    assert!(
        matches!(&action, AppAction::Refused(refusal) if refusal.command == Some(OperatorCommand::StopScript)),
        "{action:?}"
    );
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.refusals(), before + 1);
    assert!(refusal_logged(
        "command refused: StopScript on psgalice: Stop script: no script running"
    ));

    // The same command from the palette: its note stays, and it logs.
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for c in "stop script".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(&app.modal, Some(crate::overlay::Modal::Palette(palette)) if palette.note.is_some()),
        "the palette keeps its note"
    );
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.refusals(), before + 2);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // Enter on a profile that is not loaded.
    let zed = app
        .fleet_model
        .rows()
        .iter()
        .position(|row| row.name() == "psgzed")
        .expect("the unloaded row is listed");
    let action = app.focus_row(zed);
    dispatch(&mut session, &mut app, action);
    assert_eq!(session.core.refusals(), before + 3);
    assert!(refusal_logged(
        "command refused: Select on psgzed: psgzed is not loaded"
    ));
    assert_eq!(app.focused_name().as_deref(), Some("psgalice"));

    // Confirm with no plan waiting.
    dispatch(
        &mut session,
        &mut app,
        AppAction::RunFleetPlan(frontend_core::FleetAction::Stop),
    );
    assert_eq!(app.error.as_deref(), Some("fleet: no plan is waiting"));
    assert_eq!(session.core.refusals(), before + 4);
    assert!(refusal_logged(
        "command refused: PlanFleetAction(Stop) on no bots: fleet: no plan is waiting"
    ));
    let _iso = iso;
}

/// PANEL-STATE-GAP-1023 (R3-M1): a command the gate refuses waits for the
/// precondition its reason names, whichever way it was asked for. Log in
/// with no bot focused, from its Overview key, the palette and a context
/// menu: the refusal logs once, stays while no bot is focused, and focusing
/// `alice` through her Fleet row clears it at once, before its timeout. A
/// command that only changes the screen waits the same way and logs
/// nothing: Parameters with no script picked clears when a card is picked.
/// With `TUI_CAPTURE_DIR` set, the key case is captured at 120x40 and 80x24
/// before and after the focus.
#[test]
fn a_gate_refusal_clears_when_its_precondition_is_supplied() {
    use crate::commands::Command;
    use crate::overlay::{ContextItem, ContextMenu, Modal};
    use crate::test_support::{ch, key};
    use crossterm::event::KeyCode;

    let refused = "Log in: no bot selected (Fleet: Enter or click a row)";
    let dir = std::env::var_os("TUI_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = dir.as_deref() {
        std::fs::create_dir_all(dir).unwrap();
    }
    for path in ["key", "palette", "menu"] {
        let iso = IsolatedStores::enter(&format!("tui-gate-focus-{path}"));
        let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
        assert_eq!(app.focused_name(), None);
        let action = match path {
            "key" => {
                let shown = app.show_screen(crate::layout::Screen::Overview);
                dispatch(&mut session, &mut app, shown);
                app.on_key(ch('i'))
            }
            "palette" => {
                let opened = app.run_command(Command::Palette);
                dispatch(&mut session, &mut app, opened);
                for c in "Log in".chars() {
                    app.on_key(ch(c));
                }
                app.on_key(key(KeyCode::Enter))
            }
            _ => {
                app.modal = Some(Modal::Context(ContextMenu {
                    title: "Overview".into(),
                    items: vec![ContextItem::Run(Command::Login)],
                    cursor: 0,
                    at: (0, 0),
                }));
                app.on_key(key(KeyCode::Enter))
            }
        };
        let before = session.core.refusals();
        dispatch(&mut session, &mut app, action);
        assert_eq!(session.core.refusals(), before + 1, "{path}: one refusal");
        assert_eq!(app.error.as_deref(), Some(refused), "{path}");
        assert!(refusal_logged(&format!(
            "command refused: Login on no bots: {refused}"
        )));
        session.expire_error(&mut app, Instant::now());
        session.pump(&mut app);
        assert_eq!(app.error.as_deref(), Some(refused), "{path}: no bot yet");
        if path == "key" {
            capture_message_line(
                dir.as_deref(),
                &session,
                &mut app,
                "11-log-in-refused-no-focus",
                Some(refused),
                refused,
            );
        }

        let alice = app
            .fleet_model
            .rows()
            .iter()
            .position(|row| row.name() == "alice")
            .unwrap();
        let focus = app.focus_row(alice);
        dispatch(&mut session, &mut app, focus);
        session.pump(&mut app);
        assert_eq!(app.focused_name().as_deref(), Some("alice"));
        assert_eq!(
            app.error, None,
            "{path}: focusing alice resolved the refusal before its timeout"
        );
        if path == "key" {
            capture_message_line(
                dir.as_deref(),
                &session,
                &mut app,
                "12-log-in-resolved-by-focus",
                None,
                refused,
            );
        }
        let _iso = iso;
    }

    let iso = IsolatedStores::enter("tui-gate-params-card");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    let before = session.core.refusals();
    let action = app.run_command(Command::ScriptParams);
    dispatch(&mut session, &mut app, action);
    let shown = app.error.clone().expect("the refusal shows");
    assert!(shown.ends_with(": pick a script first"), "{shown}");
    assert_eq!(
        session.core.refusals(),
        before,
        "a screen change logs nothing"
    );
    session.pump(&mut app);
    assert_eq!(app.error.as_deref(), Some(shown.as_str()), "no card yet");
    app.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier"));
    session.pump(&mut app);
    assert_eq!(app.error, None, "picking a card resolved the message");
    let _iso = iso;
}

/// PANEL-STATE-GAP-1023 (F2 census): a Guardian edit in the settings popup
/// opened with no bot focused has no profile to save on. It is refused at
/// the command boundary (before, it was dropped without a word), and
/// focusing a bot, which binds the popup, clears the message.
#[test]
fn a_settings_edit_with_no_bound_profile_is_refused() {
    let iso = IsolatedStores::enter("tui-settings-no-profile");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    assert_eq!(app.focused_name(), None);
    let open = app.run_command(crate::commands::Command::Settings);
    dispatch(&mut session, &mut app, open);
    session.pump(&mut app);
    assert_eq!(app.settings_profile, None, "no bot to bind");
    let before = session.core.refusals();
    app.settings_dirty = true;
    session.pump(&mut app);
    let refused = "settings: no bot selected (Fleet: Enter or click a row)";
    assert_eq!(session.core.refusals(), before + 1);
    assert_eq!(app.error.as_deref(), Some(refused));
    assert!(refusal_logged(&format!(
        "command refused: SetGuardian on no bots: {refused}"
    )));

    let alice = app
        .fleet_model
        .rows()
        .iter()
        .position(|row| row.name() == "alice")
        .unwrap();
    let focus = app.focus_row(alice);
    dispatch(&mut session, &mut app, focus);
    session.pump(&mut app);
    assert_eq!(app.settings_profile.as_deref(), Some("alice"));
    assert_eq!(app.error, None, "focusing a bot resolved the message");
    let _iso = iso;
}

/// Edit `name`'s published host row, as its slot worker would.
fn publish_status(session: &TuiSession, name: &str, edit: impl FnOnce(&mut host_play::SlotStatus)) {
    let play = session.core.play_for_test().unwrap();
    let mut rows = play.statuses.lock().unwrap();
    match rows.iter_mut().find(|row| row.username == name) {
        Some(row) => edit(row),
        None => {
            let mut row = host_play::SlotStatus {
                username: name.into(),
                ..host_play::SlotStatus::default()
            };
            edit(&mut row);
            rows.push(row);
        }
    }
}

/// LOGOUT-RETRY-1025: the `[Log out u]` button is dim for a bot that never
/// logged in and lit for a bot whose refused login is retrying; its key
/// refuses the first and parks the second, and Fleet Log out takes the
/// second the same way. At 120x40 and 80x24, with `TUI_CAPTURE_DIR` set the
/// frames go to `<dir>/logout-retry`.
#[test]
fn logout_retry_reaches_a_retrying_login_by_key_and_fleet_and_refuses_a_never_logged_in_bot() {
    use crate::commands::Command;
    use crate::test_support::ch;
    let refused = "Error connecting to server. (connect: Connection refused)";
    let iso = IsolatedStores::enter("tui-logout-retry");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let dir = std::env::var_os("TUI_CAPTURE_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join("logout-retry"));
    if let Some(dir) = dir.as_deref() {
        std::fs::create_dir_all(dir).unwrap();
    }
    // The `[Log out u]` button's first cell is lit or dim, at both sizes.
    let logout_button = |app: &mut TuiApp, label: &str, lit: bool| {
        for (cols, rows) in [(120, 40), (80, 24)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let mut found = None;
            for y in 0..rows {
                let line: String = (0..cols).map(|x| buffer[(x, y)].symbol()).collect();
                if let Some(at) = line.find("[Log out u]") {
                    let x = line[..at].chars().count() as u16;
                    found = Some(
                        buffer[(x, y)]
                            .modifier
                            .contains(ratatui::style::Modifier::DIM),
                    );
                }
            }
            let dim = found.unwrap_or_else(|| panic!("{label} {cols}x{rows}: no [Log out u]"));
            assert_eq!(!dim, lit, "{label} {cols}x{rows}");
            if let Some(dir) = dir.as_deref() {
                let proof = serde_json::json!({
                    "label": label,
                    "log_out_button_lit": !dim,
                    "message": app.error,
                });
                super::manual_click_live_tests::capture(
                    app,
                    cols,
                    rows,
                    dir,
                    &format!("{cols}x{rows}"),
                    label,
                    &proof,
                )
                .unwrap();
            }
        }
    };
    let arm =
        |session: &TuiSession, name: &str| session.core.play_for_test().unwrap().arm(name).unwrap();

    // alice never logged in: Log out is dim and refuses with one reason.
    app.focused = Some(0);
    dispatch(&mut session, &mut app, AppAction::Focus("alice".into()));
    app.key_focus = crate::layout::Pane::Detail;
    session.pump(&mut app);
    assert!(Command::Logout.availability(&app).is_err());
    logout_button(&mut app, "01-never-logged-in", false);
    let before = session.core.refusals();
    press_key(&mut session, &mut app, ch('u'));
    assert_eq!(session.core.refusals(), before + 1);
    assert_eq!(app.error.as_deref(), Some("Log out: not logged in"));
    assert!(
        !arm(&session, "alice").login_latched(),
        "nothing was latched"
    );

    // bob's refused login keeps retrying: Log out is lit and parks him.
    app.focused = Some(1);
    dispatch(&mut session, &mut app, AppAction::Focus("bob".into()));
    app.key_focus = crate::layout::Pane::Detail;
    session
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .expect("login");
    publish_status(&session, "bob", |r| {
        r.startup_phase = host_play::StartupPhase::Error;
        r.startup_phase_started = Instant::now();
        r.error = Some(refused.into());
    });
    session.core.poll();
    session.pump(&mut app);
    assert!(arm(&session, "bob").wants_login(), "the slot would retry");
    assert_eq!(Command::Logout.availability(&app), Ok(()));
    // The refusal waited for a bot Log out can act on: bob's retry is one.
    session.clear_resolved_error(&mut app);
    assert_eq!(app.error, None, "the not-logged-in refusal resolved");
    logout_button(&mut app, "02-retrying", true);
    press_key(&mut session, &mut app, ch('u'));
    assert!(arm(&session, "bob").login_latched(), "bob is parked");
    assert!(
        !arm(&session, "bob").wants_login(),
        "no further login attempt"
    );
    session.core.poll();
    session.pump(&mut app);
    assert!(Command::Logout.availability(&app).is_err());
    logout_button(&mut app, "03-parked", false);

    // Fleet Log out (`U`) reaches a retrying bot through the shared plan.
    session
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .expect("login");
    publish_status(&session, "bob", |r| {
        r.startup_phase = host_play::StartupPhase::Error;
        r.startup_phase_started = Instant::now();
        r.error = Some(refused.into());
    });
    session.core.poll();
    session.pump(&mut app);
    assert!(arm(&session, "bob").wants_login());
    app.key_focus = crate::layout::Pane::Fleet;
    session
        .submit_with(
            &mut app,
            OperatorCommand::PlanFleetAction(frontend_core::FleetAction::Logout),
            Targets::Application,
        )
        .expect("the plan is made");
    let plan = session
        .core
        .fleet_plan()
        .expect("bob has a login to cancel");
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["bob"]);
    assert!(
        plan.prompt().contains("alice: not logged in"),
        "{}",
        plan.prompt()
    );
    dispatch(
        &mut session,
        &mut app,
        AppAction::RunFleetPlan(frontend_core::FleetAction::Logout),
    );
    session.core.poll();
    assert!(arm(&session, "bob").login_latched(), "bob is parked again");
    assert!(!arm(&session, "bob").wants_login());
    assert!(!arm(&session, "alice").login_latched());
    let _iso = iso;
}

/// LOGIN-NOTICE-1024: bob's refused login at 120x40 and 80x24. The
/// notice stays in red in the status pane through the retry's Connecting,
/// a queue wait and a parked logged out, with the live state line above
/// it; it is gone at the shared 15 s timeout and the error is then history
/// (`last error`). With `TUI_CAPTURE_DIR` set, every frame is written to
/// `<dir>/login-notice` by the capture harness (text and JSON cells).
#[test]
fn a_login_notice_stands_through_retry_states_and_expires_at_both_sizes() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;
    let refused = "Error connecting to server. (connect: Connection refused)";
    let notice = format!("login {refused}");
    let iso = IsolatedStores::enter("tui-login-notice");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice", "bob"]);
    let dir = std::env::var_os("TUI_CAPTURE_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join("login-notice"));
    if let Some(dir) = dir.as_deref() {
        std::fs::create_dir_all(dir).unwrap();
    }
    app.focused = Some(1);
    dispatch(&mut session, &mut app, AppAction::Focus("bob".into()));
    app.key_focus = crate::layout::Pane::Detail;
    session
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .expect("login");
    let mut frames = 0;
    let mut shot = |session: &mut TuiSession, app: &mut TuiApp, label: &str, state: &str| {
        session.pump(app);
        let detail = app.focused_detail();
        let shown = detail.and_then(|detail| detail.row().login_notice().map(str::to_owned));
        let apart = detail.is_some_and(|detail| detail.login_notice_line().is_some());
        // Heads short enough for the narrowest status pane: longer text
        // wraps there.
        let head = |line: &str| line.chars().take(26).collect::<String>();
        for (cols, rows) in [(120, 40), (80, 24)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let lines: Vec<String> = (0..rows)
                .map(|y| (0..cols).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let text = lines.join("\n");
            assert!(
                text.contains(&head(&format!("state: {state}"))),
                "{label} {cols}x{rows}:\n{text}"
            );
            match &shown {
                Some(_) => assert!(
                    text.contains(&head(&notice)) && !text.contains("last error"),
                    "{label} {cols}x{rows}:\n{text}"
                ),
                None => assert!(
                    text.contains(&head(&format!("last error: {refused}"))),
                    "{label} {cols}x{rows}:\n{text}"
                ),
            }
            // A notice on its own line is drawn in red in the status pane:
            // a row that starts at the state line's column (the log pane
            // may show the same words, also in red).
            let column = lines
                .iter()
                .find_map(|line| line.find("state: ").map(|at| line[..at].chars().count()))
                .expect("the status pane");
            let red = lines.iter().enumerate().any(|(y, line)| {
                let rest: String = line.chars().skip(column).collect();
                rest.starts_with(&head(&notice))
                    && buffer[(column as u16, y as u16)].fg == ratatui::style::Color::Red
            });
            assert_eq!(red, apart, "{label} {cols}x{rows}:\n{text}");
            if let Some(dir) = dir.as_deref() {
                let proof = serde_json::json!({
                    "label": label,
                    "state": state,
                    "login_notice": shown,
                });
                super::manual_click_live_tests::capture(
                    app,
                    cols,
                    rows,
                    dir,
                    &format!("{cols}x{rows}"),
                    label,
                    &proof,
                )
                .unwrap();
            }
            frames += 1;
        }
        shown
    };

    let t0 = Instant::now();
    publish_status(&session, "bob", |r| {
        r.startup_phase = host_play::StartupPhase::Error;
        r.startup_phase_started = t0;
        r.error = Some(refused.into());
    });
    session.core.poll_at(t0);
    assert_eq!(
        shot(&mut session, &mut app, "01-refused-0s", &notice).as_deref(),
        Some(notice.as_str())
    );

    publish_status(&session, "bob", |r| {
        r.startup_phase = host_play::StartupPhase::Connecting;
        r.startup_phase_started = Instant::now();
        r.error = None;
    });
    session.core.poll_at(t0 + Duration::from_secs(1));
    assert!(shot(
        &mut session,
        &mut app,
        "02-retry-connecting-1s",
        "logging in…"
    )
    .is_some());

    publish_status(&session, "bob", |r| {
        r.startup_phase = host_play::StartupPhase::Queueing;
        r.startup_phase_started = Instant::now();
        (r.queue_position, r.queue_total) = (2, 3);
    });
    session.core.poll_at(t0 + Duration::from_secs(2));
    assert!(shot(
        &mut session,
        &mut app,
        "03-queued-2s",
        "waiting in login queue (2/3)"
    )
    .is_some());

    session
        .submit(OperatorCommand::Logout, Targets::Profile("bob"))
        .expect("logout");
    let parked = "logged out (Log in to connect)";
    session.core.poll_at(t0 + Duration::from_secs(3));
    assert!(shot(&mut session, &mut app, "04-logged-out-3s", parked).is_some());

    session.core.poll_at(t0 + TIMEOUT - Duration::from_secs(1));
    assert!(shot(&mut session, &mut app, "05-logged-out-14s", parked).is_some());

    session.core.poll_at(t0 + TIMEOUT);
    assert_eq!(
        shot(&mut session, &mut app, "06-expired-15s", parked),
        None,
        "the notice is gone at the shared timeout"
    );
    assert_eq!(frames, 12);
    let _iso = iso;
}

#[test]
fn unavailable_quester_skip_shows_and_expires_the_tui_refusal() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;

    let iso = IsolatedStores::enter("tui-quester-skip-refusal");
    let (mut session, mut app) = tui_with_profiles(&iso, &["alice"]);
    assert_eq!(
        app.show_screen(crate::layout::Screen::Script),
        AppAction::None
    );
    let before = session.core.refusals();
    let action = app.on_key(settings_key(KeyCode::Char('k')));
    assert!(matches!(
        &action,
        AppAction::Refused(refusal)
            if refusal.command == Some(OperatorCommand::SkipQuest)
    ));
    dispatch(&mut session, &mut app, action);

    let message = "script: no active Quester queue";
    assert_eq!(app.error.as_deref(), Some(message));
    assert_eq!(session.core.refusals(), before + 1);
    assert!(refusal_logged("SkipQuest on alice"));
    for (width, height) in [(120, 40), (80, 24)] {
        let shown = screen_words(&mut app, width, height);
        assert!(shown.contains(message), "{width}x{height}: {shown}");
    }

    let now = Instant::now();
    session.expire_error(&mut app, now);
    session.expire_error(&mut app, now + TIMEOUT - Duration::from_secs(1));
    assert_eq!(app.error.as_deref(), Some(message));
    session.expire_error(&mut app, now + TIMEOUT);
    assert_eq!(app.error, None, "the refusal expires at the shared timeout");
}

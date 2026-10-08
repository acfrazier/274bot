use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::{
    apply_only_render_selected, apply_ui_scale, boot_failure_is_fatal, boot_for,
    chooser_should_open_popup, clamp_hop_label_px, debug_caption, drive_startup,
    edit_parameters_enabled, finish_panel_run, game_window_flags, hold_script_terminal_shot,
    live_exit_code, live_null_tick, live_script_tick, live_smoke_tick, live_stress_tick,
    loading_text, logout_enabled, manual_shot_label, parse_args, parse_live_args, progress_channel,
    request_clean_stop_capture, request_native_failure_capture, runner_config,
    script_failure_scenario, smoke_settled, smoke_should_fire, startup_progress,
    status_value_visible, Boot, LiveBoot, LiveHarness, LiveNull, LiveScript, LiveSmoke, LiveStress,
    PanelState, ProfilePrepareJob, ProgressPhase, RunMode, ShotStatus, SoakCapture,
    StartupPreparation, BASE_WINDOW_H, BASE_WINDOW_W, LIVE_USAGE, NAV_FULL_SHOT_DRAIN,
    SMOKE_DEADLINE, SMOKE_SETTLE,
};
use crate::log_pane::log_follow_bottom;
use crate::test_support::TestDir;
use crate::theme::{
    applet_offset, fit_applet, game_window_title, native_applet, panel_split_ratio, PANEL_WIDTH,
};
use crate::window::RedrawMode;
use client::io::Packet;
use dear_imgui_rs::{BackendFlags, ConfigFlags, Id, WindowFlags};
use host_play::profile::ProfileEnvironment;
use host_play::SharedClientTemplate;

#[test]
#[ignore]
fn panel_exit_status_child() {
    match std::env::var("PANEL_EXIT_CHILD").as_deref() {
        Ok("fail") => {
            let (mut session, live, first_failure) = failed_live_script();
            let mut live = LiveHarness::Script(live);
            assert_eq!(live_exit_code(&live, Some(&first_failure)), Some(1));

            let repeated_failure = live.tick(&mut session, &[], &ShotStatus::Missing, None, 0);
            assert!(repeated_failure.is_none());
            assert_eq!(live_exit_code(&live, repeated_failure.as_deref()), Some(1));
            let result = finish_panel_run(Ok(()), live.failure().map(str::to_owned));
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
        Ok("pass") => {
            let null = LiveHarness::Null(LiveNull {
                started: Instant::now(),
                saw_scene2: true,
                passed: true,
            });
            let stress = LiveHarness::Stress(LiveStress {
                started: Instant::now(),
                last_announced: 50,
                passed: true,
                name: "stress50",
                host: "127.0.0.1".into(),
                port: 0,
            });
            assert_eq!(live_exit_code(&null, None), Some(0));
            assert_eq!(live_exit_code(&stress, None), Some(0));
            let live = LiveHarness::Script(LiveScript {
                name: "script_exit_pass".into(),
                passed: true,
                failed: None,
                last_step: None,
                drain_started: None,
                soak: false,
                soak_until: None,
                announced_pass: true,
                native_failure_capture_requested: false,
                clean_stop_capture_requested: false,
                core_deadline: None,
                soak_capture: SoakCapture::NotNeeded,
            });
            std::process::exit(live_exit_code(&live, None).expect("PASS exits immediately"));
        }
        other => panic!("unknown child mode {other:?}"),
    }
}

#[test]
fn latched_live_failure_exits_the_process_nonzero() {
    let output = live_exit_child("fail");
    assert_eq!(
        output.status.code(),
        Some(1),
        "FAIL child stdout:\n{}\nFAIL child stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("FAIL: live script_live_failure"),
        "the child drove the real live-harness failure path"
    );
}

#[test]
fn live_pass_exits_the_process_successfully() {
    let output = live_exit_child("pass");
    assert_eq!(
        output.status.code(),
        Some(0),
        "PASS child stdout:\n{}\nPASS child stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn normal_interactive_window_close_remains_successful() {
    assert!(finish_panel_run(Ok(()), None).is_ok());
}

fn live_exit_child(mode: &str) -> std::process::Output {
    std::process::Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "app::tests::panel_exit_status_child",
        ])
        .env("PANEL_EXIT_CHILD", mode)
        .output()
        .expect("run live exit child")
}

fn failed_live_script() -> (crate::session::Session, LiveScript, String) {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};

    let fail_scenario = Scenario {
        name: "live_failure_exit",
        seed: Seed {
            profiles: vec![("alice", "password")],
            mainland: false,
        },
        steps: vec![Step {
            name: "unmet proof after scripted action",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 999 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 999 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(fail_scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut client = script_client();
    runner.tick(&mut client);
    assert!(matches!(runner.status(), scenario::RunnerStatus::Failed(_)));

    let mut session = crate::session::Session::new();
    *session.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_live_failure".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    let message = live_script_tick(&mut live, &mut session, &ShotStatus::Missing, None)
        .expect("a failed scenario must FAIL the live harness");
    (session, live, message)
}

#[test]
fn empty_walk_queue_and_modal_values_are_not_status_rows() {
    for value in ["", "—", "-1"] {
        assert!(
            !status_value_visible("walk", value),
            "walk placeholder {value:?} is hidden"
        );
        assert!(
            !status_value_visible("queue", value),
            "queue placeholder {value:?} is hidden"
        );
        assert!(
            !status_value_visible("modals", value),
            "modal placeholder {value:?} is hidden"
        );
    }
    assert!(status_value_visible("walk", "2659 3292 0"));
    assert!(status_value_visible("queue", "1 of 2"));
    assert!(status_value_visible("modals", "0"));
    assert!(status_value_visible("state", "idle"));
}

#[test]
fn status_rows_render_only_meaningful_values_for_the_selected_phase() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut logged_out = frontend_core::SlotDetail::default();
    logged_out.row.phase = frontend_core::Phase::LoggedOut;
    logged_out.state = "logged out".into();
    logged_out.modal = 7;
    let hidden = draw_status_detail(&logged_out, "—", "lowmem");
    assert!(
        !hidden.contains("walk"),
        "empty walk row is visible: {hidden}"
    );
    assert!(
        !hidden.contains("queue"),
        "empty queue row is visible: {hidden}"
    );
    assert!(
        !hidden.contains("modals"),
        "logged-out modal row is visible: {hidden}"
    );
    assert!(hidden.contains("state") && hidden.contains("logged out"));

    let mut ready = frontend_core::SlotDetail::default();
    ready.row.phase = frontend_core::Phase::Ready;
    ready.state = "ingame scene 2".into();
    ready.modal = 0;
    let shown = draw_status_detail(&ready, "2659 3292 0", "lowmem");
    assert!(shown.contains("walk") && shown.contains("2659 3292 0"));
    assert!(shown.contains("modals") && shown.contains("0"));
}

/// On a 1024×768 desktop the app's client area is about 1008×580 and the
/// docked Profiles window runs past its right edge. The last row's Edit and
/// ✕, and the form's Save, Cancel and Close, all lie wholly on screen and in
/// the part of their window that shows: no list or window scrolling, and a
/// click there lands.
#[test]
fn profiles_controls_remain_reachable_when_the_dock_runs_past_1024() {
    let _guard = crate::test_support::imgui_context_guard();
    let display = [1008.0, 580.0];
    let mut ui = ProfilesUi::with_geometry(
        "profiles-narrow-controls",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
        display,
        [790.0, 0.0],
        [PANEL_WIDTH, 580.0],
    );
    ui.click(At::List, "Edit##edit-alice");
    for (at, label) in [
        (At::List, "Edit##edit-alice"),
        (At::List, "✕##alice"),
        (At::List, "Edit##edit-bob"),
        (At::List, "✕##bob"),
        (At::Form, "Save"),
        (At::Form, "Cancel"),
        (At::Window, "Close"),
    ] {
        let (item, visible) = ui.item_rect(at, label);
        for (name, area) in [("display", [[0.0, 0.0], display]), ("window", visible)] {
            assert!(
                item[0][0] >= area[0][0]
                    && item[0][1] >= area[0][1]
                    && item[1][0] <= area[1][0]
                    && item[1][1] <= area[1][1],
                "{label} at {item:?} is not wholly shown in the {name}'s {area:?}"
            );
        }
    }
    let (bob, _) = ui.item_rect(At::List, "Edit##edit-bob");
    let opened = ui.click_at(rect_center(bob));
    assert!(opened.has("Editing bob"), "{}", opened.text);
}

#[test]
fn catalog_assignment_without_catalog_has_a_distinct_prefs_hint() {
    let mut session = crate::session::Session::new();
    session.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "Sherlock".into(),
    ));
    assert_eq!(
        super::script_prefs_disabled_hint(&session),
        Some("script catalog unavailable")
    );
}

#[test]
fn parameters_rail_does_not_call_an_unloaded_catalog_card_parameterless() {
    let mut session = crate::session::Session::new();
    session.script_sel = Some(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "Sherlock".into(),
    ));
    assert_eq!(
        super::parameters_rail_placeholder(&session),
        Some("(script catalog unavailable)"),
        "0.1.9 printed '(no parameters)' for a card whose catalog was never read"
    );
    session.script_sel = None;
    assert_eq!(
        super::parameters_rail_placeholder(&session),
        Some("(no script selected)")
    );
}

#[test]
fn logout_is_enabled_only_for_a_connected_queued_or_guard_parked_focus() {
    assert!(!logout_enabled(false, true, true, false, false));
    assert!(!logout_enabled(true, false, true, false, false));
    assert!(!logout_enabled(true, true, false, false, false));
    assert!(logout_enabled(true, true, true, false, false));
    assert!(logout_enabled(true, true, false, true, false));
    assert!(logout_enabled(true, true, false, false, true));
}

fn checked_fixture(revision: u16) -> (TestDir, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let root = TestDir::new(&format!("profile-{revision}"));
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

const JAG_SLOTS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

fn identity_packs() -> Vec<(String, Vec<u8>)> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let mut tables: Vec<(String, Vec<u8>)> = Vec::new();
    for prefix in ["model", "anim", "midi", "map"] {
        tables.push((format!("{prefix}_version"), vec![0, 1]));
        tables.push((format!("{prefix}_crc"), 1u32.to_be_bytes().to_vec()));
        tables.push((format!("{prefix}_index"), vec![0]));
    }
    let refs: Vec<(&str, &[u8])> = tables
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let versionlist = client::io::synthetic_jag(&refs);
    JAG_SLOTS
        .iter()
        .map(|name| {
            let bytes = if *name == "versionlist" {
                versionlist.clone()
            } else if *name == "config" || *name == "interface" {
                std::fs::read(fixture.join(name)).unwrap()
            } else {
                client::io::synthetic_jag(&[("data", b"content".as_slice())])
            };
            ((*name).to_string(), bytes)
        })
        .collect()
}
fn crc_body(packs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (name, bytes) in packs {
        let slot = JAG_SLOTS
            .iter()
            .position(|candidate| candidate == name)
            .unwrap()
            + 1;
        checksums[slot] = Packet::getcrc(bytes, 0, bytes.len());
    }
    let mut body = Packet::alloc(0);
    for &checksum in &checksums {
        body.p4(checksum);
    }
    let mut hash = 1234i32;
    for &checksum in &checksums {
        hash = hash.wrapping_shl(1).wrapping_add(checksum);
    }
    body.p4(hash);
    body.data()[..body.pos].to_vec()
}

fn plant_snapshot(unpack: &std::path::Path, revision: u16, packs: &[(String, Vec<u8>)]) {
    struct FixtureEntries;
    impl client::unpack::EntrySource for FixtureEntries {
        fn fetch_entries(
            &mut self,
            _archive: i32,
            files: &[i32],
        ) -> Result<Vec<(i32, Vec<u8>)>, String> {
            Ok(files.iter().map(|&file| (file, b"body".to_vec())).collect())
        }
    }
    let versionlist = &packs
        .iter()
        .find(|(name, _)| name == "versionlist")
        .unwrap()
        .1;
    let transfer = nav::manifest::hash_bytes(versionlist);
    let negotiated = nav::manifest::hash_bytes(&crc_body(packs)[..36]);
    let input = unpack.join("fixture-input");
    std::fs::create_dir_all(&input).unwrap();
    for (name, bytes) in packs {
        std::fs::write(input.join(name), bytes).unwrap();
    }
    let out = unpack
        .join(format!("revision-{revision}"))
        .join(negotiated)
        .join(transfer);
    // Use the same verified retained publisher fixture as host runtime_bind.
    // Legacy size-only markers are not ordinary-launch asset candidates.
    client::unpack::fetch_snapshot(
        &input.to_string_lossy(),
        &out.to_string_lossy(),
        &mut FixtureEntries,
    )
    .unwrap();
}

/// Mock update server: `/crc` matching the fixture packs, plus pack GETs if
/// a runtime refresh asks. Detached so unit tests never need a live engine.
fn serve_fixture_crc(packs: Vec<(String, Vec<u8>)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let body = crc_body(&packs);
        while Instant::now() < deadline {
            let (mut sock, _) = match listener.accept() {
                Ok(conn) => {
                    // Accepted sockets inherit O_NONBLOCK from the listener on macOS/BSD.
                    let _ = conn.0.set_nonblocking(false);
                    conn
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let path = String::from_utf8_lossy(&request)
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let payload = if path == "/crc" {
                body.clone()
            } else {
                packs
                    .iter()
                    .find(|(name, _)| path.starts_with(&format!("/{name}")))
                    .map(|(_, bytes)| bytes.clone())
                    .unwrap_or_default()
            };
            let response = [
                b"HTTP/1.0 200 OK\r\nContent-Length: ".as_slice(),
                payload.len().to_string().as_bytes(),
                b"\r\n\r\n",
                &payload,
            ]
            .concat();
            let _ = sock.write_all(&response);
        }
    });
    port
}

fn ephemeral_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn runtime_checked_fixture(revision: u16) -> (TestDir, PathBuf, PathBuf, PathBuf, u16) {
    let (root, cache, _) = checked_fixture(revision);
    let packs = identity_packs();
    for (name, bytes) in &packs {
        std::fs::write(cache.join(name), bytes).unwrap();
    }
    let manifest_path = root.join("cache-manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(revision, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let unpack = root.join("unpack");
    plant_snapshot(&unpack, revision, &packs);
    let port = serve_fixture_crc(packs);
    (root, cache, manifest_path, unpack, port)
}

#[test]
fn frontend_parser_prepares_real_clients_for_both_fixture_manifests() {
    for revision in [274_u16, 289] {
        let (root, cache, manifest) = checked_fixture(revision);
        // OnDemand hubs are keyed by (game_host, game_port); parallel panel
        // tests must not share the default local ports with different caches.
        let game_port = ephemeral_port();
        let asset_port = ephemeral_port();
        let args = parse_args(
            [
                "--smoke".to_string(),
                "--profile".to_string(),
                format!("local-{revision}"),
                "--cache".to_string(),
                cache.display().to_string(),
                "--cache-manifest".to_string(),
                manifest.display().to_string(),
                "--port".to_string(),
                game_port.to_string(),
                "--http-port".to_string(),
                asset_port.to_string(),
            ],
            None,
        )
        .expect("frontend and shared flags parse in either order");
        let env = ProfileEnvironment {
            home: Some(root.to_path_buf()),
            working_dir: Some(root.to_path_buf()),
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
    }
}

#[test]
fn boot_is_deferred_and_maps_live_smoke_and_vault_pass() {
    // Every slot-spawning path is deferred (returns a Boot to run on
    // the first frame after GPU init), never executed eagerly: mapping
    // a mode to a Boot must not unlock or call live_prepare — those
    // run only in `boot_execute`, after the first frame presents and
    // `on_gpu_init` injected the panel's device.
    assert!(matches!(
        boot_for(&RunMode::Live("null_raster".into()), None),
        Some(Boot::Live(LiveBoot::NullRaster))
    ));
    assert!(matches!(
        boot_for(&RunMode::Live("stress50".into()), None),
        Some(Boot::Live(LiveBoot::Stress50))
    ));
    assert!(matches!(
        boot_for(&RunMode::Live("stress50_full".into()), None),
        Some(Boot::Live(LiveBoot::Stress50Full))
    ));
    match boot_for(&RunMode::Live("script_walk".into()), Some("pass".into())) {
        Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "script_walk"),
        other => panic!("script_<name> must map to a deferred Script boot, got {other:?}"),
    }
    match boot_for(&RunMode::Live("nav_full".into()), None) {
        Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "nav_full"),
        other => panic!("nav_full must map to a deferred Script boot, got {other:?}"),
    }
    match boot_for(&RunMode::Interactive, Some("hunter2".into())) {
        Some(Boot::Unlock { pass }) => assert_eq!(pass, "hunter2"),
        other => panic!("a piped passphrase must map to a deferred Unlock boot, got {other:?}"),
    }
    assert!(matches!(
        boot_for(&RunMode::Smoke, None),
        Some(Boot::Live(LiveBoot::Smoke))
    ));
    assert!(
        boot_for(&RunMode::Interactive, None).is_none(),
        "no boot with no live arg and no pass"
    );
    assert!(!boot_failure_is_fatal(&Boot::Unlock {
        pass: "secret".into()
    }));
    assert!(boot_failure_is_fatal(&Boot::Live(LiveBoot::Smoke)));
}

#[test]
fn the_passphrase_is_only_ever_read_from_stdin_never_an_argument() {
    assert!(!parse_args(["--smoke"], None).unwrap().vault_pass_stdin);
    assert!(
        parse_args(["--vault-pass-stdin"], None)
            .unwrap()
            .vault_pass_stdin
    );
    for spelling in [
        &["--vault-pass", "hunter2-hunter2"][..],
        &["--vault-pass=hunter2-hunter2"][..],
        &["--smoke", "--vault-pass", "hunter2-hunter2"][..],
    ] {
        let (code, message) = parse_args(spelling.iter().copied(), None).unwrap_err();
        assert_eq!(code, 2);
        assert!(message.contains("--vault-pass-stdin"), "{message}");
        assert!(
            !message.contains("hunter2"),
            "the value is never echoed: {message}"
        );
    }
}

fn prepared_startup(boot: Boot) -> (PanelState, StartupPreparation, TestDir) {
    let (root, cache, manifest, unpack, port) = runtime_checked_fixture(274);
    let options = host_play::ProfileOptions {
        profile: Some("local-274".into()),
        cache_dir: Some(cache),
        cache_manifest: Some(manifest),
        unpack_dir: Some(unpack),
        port: Some(ephemeral_port()),
        http_port: Some(port),
        vault_path: Some(root.join("startup.vault")),
        ..host_play::ProfileOptions::default()
    };
    let env = ProfileEnvironment {
        home: Some(root.to_path_buf()),
        working_dir: Some(root.to_path_buf()),
        engine_dir: Some(root.join("engine")),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    };
    let template = options
        .resolve_with_env(None, &env)
        .unwrap()
        .prepare_template()
        .unwrap();
    let mut state = PanelState::default();
    state
        .session
        .configure_profile_with_env(options, env)
        .unwrap();
    let generation = state.session.profile_generation();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender.send(Ok(template)).unwrap();
    let (_, progress) = progress_channel(host_play::progress::ProfileProgress::steps(
        host_play::progress::ProfileProgressStage::LoadingGameData,
        4,
        4,
    ));
    let startup = StartupPreparation {
        prepare: Some(ProfilePrepareJob {
            generation,
            receiver,
            progress,
        }),
        validate: None,
        pending_boot: Some(boot),
        failed_generation: None,
    };
    (state, startup, root)
}

#[test]
fn normal_unlock_waits_for_worker_validation_then_uses_prepared_profile() {
    let (mut state, mut startup, _root) = prepared_startup(Boot::Unlock {
        pass: "prepared-pass".into(),
    });
    drive_startup(&mut state, &mut startup);
    let mut validation = startup.validate.take().expect("final validation worker");
    let result = validation
        .receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("final validation completion");
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender.send(result).unwrap();
    validation.receiver = receiver;
    startup.validate = Some(validation);
    drive_startup(&mut state, &mut startup);
    assert!(state.session.profile_bound());
    assert!(state.session.core.vault().is_some());
    assert!(state.session.core.play().is_some());
    assert!(state.session.core.slots().is_empty());
    assert!(startup_progress(&startup, state.session.profile_generation()).is_none());
}

#[test]
fn live_boot_stays_deferred_while_final_validation_is_in_flight() {
    let (mut state, mut startup, _root) = prepared_startup(Boot::Live(LiveBoot::Smoke));
    drive_startup(&mut state, &mut startup);
    assert!(state.session.profile_bound());
    let validation = startup.validate.take().expect("final validation worker");
    assert!(state.session.profile_preparing());
    assert!(state.session.core.vault().is_none());
    assert!(state.session.core.play().is_none());
    assert!(state.session.core.slots().is_empty());
    assert!(validation
        .receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .is_ok());
    assert!(matches!(validation.boot, Boot::Live(LiveBoot::Smoke)));
}

#[test]
fn startup_progress_is_latest_only_and_generation_scoped() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let (observer, progress) = progress_channel(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        0,
        8,
    ));
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        3,
        8,
    ));
    observer.report(ProfileProgress::bytes(
        ProfileProgressStage::CheckingNavigationFiles,
        1024,
        4096,
    ));
    let (_sender, receiver) = std::sync::mpsc::sync_channel(1);
    let mut startup = StartupPreparation::new(None);
    startup.prepare = Some(ProfilePrepareJob {
        generation: 7,
        receiver,
        progress,
    });

    let current = startup_progress(&startup, 7).expect("matching generation progress");
    assert_eq!(current.phase, ProgressPhase::Preparing);
    assert_eq!(current.progress.completed, 1024);
    assert_eq!(current.progress.total, 4096);
    assert!(
        startup_progress(&startup, 8).is_none(),
        "stale work is hidden"
    );
    startup.prepare = None;
    assert!(
        startup_progress(&startup, 7).is_none(),
        "completion clears it"
    );
}

#[test]
fn preparation_failure_clears_progress_with_partial_session_state_absent() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let (root, cache, manifest) = checked_fixture(274);
    let mut state = PanelState::default();
    state
        .session
        .configure_profile_with_env(
            host_play::ProfileOptions {
                profile: Some("local-274".into()),
                cache_dir: Some(cache),
                cache_manifest: Some(manifest),
                ..host_play::ProfileOptions::default()
            },
            ProfileEnvironment {
                home: Some(root.to_path_buf()),
                working_dir: Some(root.to_path_buf()),
                ..ProfileEnvironment::default()
            },
        )
        .unwrap();
    let generation = state.session.profile_generation();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender
        .send(Err("fixture preparation failed".into()))
        .unwrap();
    let (_, progress) = progress_channel(ProfileProgress::steps(
        ProfileProgressStage::CheckingGameFiles,
        3,
        8,
    ));
    let mut startup = StartupPreparation {
        prepare: Some(ProfilePrepareJob {
            generation,
            receiver,
            progress,
        }),
        validate: None,
        pending_boot: Some(Boot::Unlock {
            pass: "not-used".into(),
        }),
        failed_generation: None,
    };

    drive_startup(&mut state, &mut startup);

    assert!(startup_progress(&startup, generation).is_none());
    assert!(!state.session.profile_preparing());
    assert!(!state.session.profile_bound());
    assert!(state.session.core.vault().is_none());
    assert!(state.session.core.play().is_none());
    assert!(state.session.core.slots().is_empty());
    assert_eq!(
        state.session.error.as_deref(),
        Some("fixture preparation failed")
    );
}

#[test]
fn loading_text_fits_the_rail_and_never_rounds_partial_work_to_complete() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let partial = loading_text(
        ProgressPhase::FinalChecks,
        &ProfileProgress::bytes(ProfileProgressStage::CheckingNavigationFiles, 999, 1000),
    );

    assert!(partial.description.starts_with("Final checks"));
    assert_eq!(partial.filled.len() + partial.empty.len(), 20);
    assert_eq!(partial.percent, 99);
    assert!(partial.caption.contains("bytes checked"));
}

#[test]
fn loading_text_names_bundled_decode_and_custom_verify_passes() {
    use host_play::progress::{ProfileProgress, ProfileProgressStage};

    let bundled = loading_text(
        ProgressPhase::Preparing,
        &ProfileProgress::steps(ProfileProgressStage::PreparingNavigation, 1, 1),
    );
    assert_eq!(bundled.description, "Loading navigation");
    assert_eq!(bundled.percent, 100);

    let custom = loading_text(
        ProgressPhase::Preparing,
        &ProfileProgress::bytes(ProfileProgressStage::CheckingNavigationFiles, 10, 100),
    );
    assert_eq!(custom.description, "Verifying custom navigation");
    assert_eq!(custom.percent, 10);
    assert!(custom.caption.contains("bytes checked"));
}

#[test]
fn log_follow_bottom_sticks_at_end_and_releases_when_scrolled_up() {
    assert!(log_follow_bottom(0.0, 0.0), "empty / first frame follows");
    assert!(log_follow_bottom(99.0, 100.0), "within 1 px of the bottom");
    assert!(log_follow_bottom(100.0, 100.0));
    assert!(!log_follow_bottom(50.0, 100.0), "scrolled up stays put");
}

#[test]
fn legacy_log_detached_is_ignored_without_losing_other_preferences() {
    let dir = TestDir::new("legacy-log-ui-state");
    let path = dir.join("panel-ui.json");
    std::fs::write(
        &path,
        r#"{"last_focus":"alice","collapsed":{"alice":{"status":false}},"capture":false,"log_detached":true}"#,
    )
    .unwrap();

    let loaded = crate::ui_state::load_at(&path);
    assert_eq!(loaded.last_focus.as_deref(), Some("alice"));
    assert!(!loaded.collapsed["alice"]["status"]);
    assert!(!loaded.capture);

    crate::ui_state::save(&loaded);
    let session = crate::session::Session::new();
    assert!(!session.log_window_open);
    assert_eq!(session.ui.last_focus.as_deref(), Some("alice"));
    assert!(!session.ui.collapsed["alice"]["status"]);
    assert!(!session.ui.capture);
}

#[test]
fn chooser_should_open_popup_table() {
    // First open: rising edge opens the popup and latches prev.
    assert_eq!(chooser_should_open_popup(true, false), (true, true));
    // Already open: no re-open while want stays true.
    assert_eq!(chooser_should_open_popup(true, true), (false, true));
    // Esc closed it: want drops to false and prev must fall so a later
    // `+ add bot` is a fresh rising edge.
    assert_eq!(chooser_should_open_popup(false, true), (false, false));
    assert_eq!(chooser_should_open_popup(false, false), (false, false));
}

#[test]
fn chooser_reopens_after_a_close() {
    let mut prev = false;
    let (open, np) = chooser_should_open_popup(true, prev);
    assert!(open, "first + add opens the chooser");
    prev = np;
    let (open, np) = chooser_should_open_popup(true, prev);
    assert!(!open, "already open: no reopen");
    prev = np;
    let (open, np) = chooser_should_open_popup(false, prev);
    assert!(!open);
    prev = np;
    assert!(!prev, "prev must track the close so + add can reopen");
    let (open, _np) = chooser_should_open_popup(true, prev);
    assert!(open, "the next + add bot reopens the chooser");
}

#[test]
fn edit_parameters_enabled_for_operator_bag() {
    assert!(edit_parameters_enabled(), "parameter editors ship in 0.1.6");
}

#[test]
fn loadout_combo_lists_store_names() {
    use script::{resolve_setting_options, Loadout, LoadoutsStore, SettingDef};

    let dir = TestDir::new("loadout-combo");
    let mut store = LoadoutsStore::at(dir.join("loadouts.json"));
    store.upsert(Loadout::new("guard"));
    store.upsert(Loadout::new("stall"));
    let def = SettingDef {
        id: "loadout".into(),
        ty: "string".into(),
        default: None,
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: Some("loadouts".into()),
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    };
    assert_eq!(
        resolve_setting_options(&def, &store, None),
        vec!["guard".to_string(), "stall".to_string()]
    );
}

#[test]
fn debug_captions_fit_the_strip() {
    assert_eq!(debug_caption("DebugPanel"), "Panel");
    assert_eq!(debug_caption("Lumbridge"), "Lumb");
    assert_eq!(debug_caption("maxme"), "maxme");
    assert_eq!(debug_caption("Teles"), "Teles");
    assert_eq!(debug_caption("TutSkip"), "TutSkip");
}

#[test]
fn hop_label_px_writes_clamp_to_8_28() {
    // The settings UI is the only writer of `hop_label_px`; it must
    // hold the NavSettings 8..=28 field invariant.
    assert_eq!(clamp_hop_label_px(0), 8);
    assert_eq!(clamp_hop_label_px(8), 8);
    assert_eq!(clamp_hop_label_px(11), 11);
    assert_eq!(clamp_hop_label_px(28), 28);
    assert_eq!(clamp_hop_label_px(100), 28);
}

#[test]
fn apply_only_render_selected_warns_before_unchecking() {
    // Checking "only render selected" on is immediate, no dialog.
    assert_eq!(apply_only_render_selected(false, true), (true, false));
    // Unchecking does not apply: keeps the safe default, opens the
    // warning instead.
    assert_eq!(apply_only_render_selected(true, false), (true, true));
    // No-op rows: the box already matches the flag.
    assert_eq!(apply_only_render_selected(true, true), (true, false));
    assert_eq!(apply_only_render_selected(false, false), (false, false));
}

#[test]
fn runner_config_docks_without_viewports() {
    let c = runner_config();
    assert!(c.docking.enable);
    assert!(
        !c.docking.auto_dockspace,
        "we own the game-left / panel-right split"
    );
    assert!(
        c.docking
            .dockspace_flags
            .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR),
        "single-bot hides the game/panel tab strip"
    );
    assert!(
        c.docking
            .dockspace_flags
            .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE),
        "dock splitters stay off — only grid uses OS-window resize"
    );
    assert!(c.ini_filename.is_none(), "no imgui.ini to restore");
    let flags = c.io_config_flags.expect("flags");
    assert!(flags.contains(ConfigFlags::DOCKING_ENABLE));
    assert!(!flags.contains(ConfigFlags::VIEWPORTS_ENABLE));
    assert!(matches!(c.redraw, RedrawMode::WaitUntil { fps } if (fps - 50.0).abs() < 0.01));
    assert_eq!(c.window_size, (BASE_WINDOW_W as f64, BASE_WINDOW_H as f64));
}

/// The panel's OS window for the M-003 rail-fit tests: a
/// [`FakeFrame`](crate::test_support::FakeFrame) on a fixed work area,
/// fitted through the production glue.
struct FittedFrame {
    frame: crate::test_support::FakeFrame,
    work: crate::window::WorkArea,
    fits: std::sync::atomic::AtomicU32,
}

impl FittedFrame {
    fn fits(&self) -> u32 {
        self.fits.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl super::OsWindow for FittedFrame {
    fn set_title(&self, _title: &str) {}
    fn request_redraw(&self) {}
    fn fit_to_work_area(&self, need_logical: (f64, f64)) {
        self.fits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::window::fit_window_in(&self.frame, Some(self.work), need_logical);
    }
}

fn fitted_frame(work: (f64, f64), inner: (u32, u32), pos: (i32, i32)) -> Arc<FittedFrame> {
    Arc::new(FittedFrame {
        frame: crate::test_support::FakeFrame::new(1.0, (16, 39), inner, pos),
        work: crate::window::WorkArea {
            origin: (0.0, 0.0),
            size: work,
        },
        fits: std::sync::atomic::AtomicU32::new(0),
    })
}

/// One headless panel frame through `dock_host`, with ImGui's display size
/// taken from the window as winit's `Resized` would set it.
fn dock_host_frame(ctx: &mut dear_imgui_rs::Context, state: &mut PanelState, os: &FittedFrame) {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new(os.frame.display_size(), 1.0 / 60.0)
            .renderer_has_textures(),
    );
    super::dock_host(ctx.frame(), state, "274bot");
    ctx.render();
}

fn dock_host_context() -> dear_imgui_rs::Context {
    let mut ctx = dear_imgui_rs::Context::create();
    ctx.io_mut()
        .set_backend_flags(BackendFlags::RENDERER_HAS_TEXTURES);
    assert!(crate::window::add_panel_font(&mut ctx));
    let _ = ctx.font_atlas_mut().build();
    apply_ui_scale(ctx.style_mut(), 1.0);
    ctx.io_mut().set_config_flags(ConfigFlags::DOCKING_ENABLE);
    ctx
}

/// M-003 on the Windows 1366x768 guest (48 px taskbar, 16x39 chrome):
/// opening MultiBox on a centered 1120 window fits the frame into the work
/// area once. The width stays short of the 1384 need, and later frames must
/// not fit again, or the operator could not drag the window (N1).
#[test]
fn rail_grow_fits_a_small_screen_once_and_the_window_stays_where_dragged() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    let os = fitted_frame((1366.0, 720.0), (1120, 580), (115, 50));
    let mut state = PanelState {
        os_window: Some(os.clone()),
        ..PanelState::default()
    };
    state.session.multibox = true;

    dock_host_frame(&mut ctx, &mut state, &os);
    assert_eq!(os.fits(), 1, "opening the rail fits the window");
    assert_eq!(
        os.frame.outer_rect(),
        (0, 50, 1366, 669),
        "clamped to the work width and moved off the right edge"
    );

    for _ in 0..3 {
        dock_host_frame(&mut ctx, &mut state, &os);
    }
    os.frame.drag_to((300, 40));
    for _ in 0..3 {
        dock_host_frame(&mut ctx, &mut state, &os);
    }
    assert_eq!(os.fits(), 1, "a clamped need is not re-fitted per frame");
    assert_eq!(os.frame.outer_rect().0, 300, "the dragged window stays put");

    state.session.multibox = false;
    dock_host_frame(&mut ctx, &mut state, &os);
    assert_eq!(os.fits(), 2, "closing the rail is a new need");
    assert_eq!(os.frame.inner(), (1120, 580));
}

/// Where the rail need fits the screen, a window the operator shrinks below
/// it while MultiBox is open grows back, as before M-003.
#[test]
fn rail_need_that_fits_regrows_a_window_shrunk_below_it() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    let os = fitted_frame((1920.0, 1040.0), (1120, 580), (400, 200));
    let mut state = PanelState {
        os_window: Some(os.clone()),
        ..PanelState::default()
    };
    state.session.multibox = true;
    dock_host_frame(&mut ctx, &mut state, &os);
    dock_host_frame(&mut ctx, &mut state, &os);
    assert_eq!(os.frame.inner(), (1384, 580));

    os.frame.user_resize((1200, 580));
    dock_host_frame(&mut ctx, &mut state, &os);
    assert_eq!(os.frame.inner(), (1384, 580));
    assert_eq!(os.fits(), 2);
}

#[test]
fn single_bot_window_class_hides_tab_bar() {
    let c = super::game_window_class();
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING));
    assert!(!c.docking_always_tab_bar);
}

#[test]
fn rail_window_class_shows_tab_x() {
    let c = super::rail_window_class();
    assert!(
        !c.dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR),
        "hidden tab strip has no X"
    );
    assert!(c.docking_always_tab_bar, "tab X needs a visible tab bar");
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
}

#[test]
fn chooser_docks_to_panel_never_rail_or_game() {
    let panel = Id::from(20u32);
    assert_eq!(super::chooser_dock_id(Some(panel)), Some(panel));
    assert_eq!(super::chooser_dock_id(None), None);
}

fn log_tab_frame(
    ctx: &mut dear_imgui_rs::Context,
    session: &mut crate::session::Session,
) -> ([f32; 2], Option<[f32; 2]>) {
    const DISPLAY_SIZE: [f32; 2] = [1440.0, 900.0];
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new(DISPLAY_SIZE, 1.0 / 60.0).renderer_has_textures(),
    );
    let mut close_button = None;
    let log_button = {
        let ui = ctx.frame();
        let (min, max) = ui
            .window("Log tab harness")
            .position([20.0, 600.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                super::title_row(ui, session);
                (ui.item_rect_min(), ui.item_rect_max())
            })
            .expect("Log button harness was drawn");
        if session.log_window_open {
            super::log_window_with_body(ui, session, None, |ui, session| {
                let pos = ui.window_pos();
                let size = ui.window_size();
                let frame_padding = ui.clone_style().frame_padding();
                let font_size = ui.current_font_size();
                close_button = Some([
                    pos[0] + size[0] - frame_padding[0] - font_size * 0.5,
                    pos[1] + frame_padding[1] + font_size * 0.5,
                ]);
                crate::log_pane::log_body(ui, session, true);
            });
        }
        [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]
    };
    ctx.render();
    (log_button, close_button)
}

#[test]
fn log_button_opens_and_window_x_closes_without_hiding_inline_log() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut session = crate::session::Session::new();
    let mut ctx = dock_host_context();
    ctx.io_mut()
        .set_config_flags(dear_imgui_rs::ConfigFlags::empty());
    ctx.set_ini_filename::<std::path::PathBuf>(None)
        .expect("disable ImGui settings persistence");
    let drawn = DrawnText::default();
    ctx.set_clipboard_backend(drawn.clone());

    let (log_button, _) = log_tab_frame(&mut ctx, &mut session);
    ctx.io_mut().add_mouse_pos_event(log_button);
    log_tab_frame(&mut ctx, &mut session);
    ctx.io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
    log_tab_frame(&mut ctx, &mut session);
    ctx.io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
    let (_, close_button) = log_tab_frame(&mut ctx, &mut session);
    assert!(session.log_window_open, "the panel button opens Log");

    let close_button = close_button.expect("the opened Log window has a close button");
    ctx.io_mut().add_mouse_pos_event(close_button);
    log_tab_frame(&mut ctx, &mut session);
    ctx.io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
    log_tab_frame(&mut ctx, &mut session);
    ctx.io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
    log_tab_frame(&mut ctx, &mut session);
    assert!(
        !session.log_window_open,
        "the window's X closes the Log view"
    );

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([1440.0, 900.0], 1.0 / 60.0)
            .renderer_has_textures(),
    );
    {
        let ui = ctx.frame();
        ui.log_to_clipboard(0u32);
        ui.window("Inline log harness")
            .build(|| super::log_section(ui, &mut session, true));
        ui.log_finish();
    }
    ctx.render();
    assert!(
        drawn.0.take().contains("[ Copy ]"),
        "closing Log leaves the inline log section drawn"
    );
}

#[test]
fn log_tab_is_registered_with_panel_dock_and_can_undock() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    let os = fitted_frame((1920.0, 1080.0), (1120, 580), (400, 200));
    let mut state = PanelState {
        os_window: Some(os.clone()),
        ..PanelState::default()
    };
    dock_host_frame(&mut ctx, &mut state, &os);
    dock_host_frame(&mut ctx, &mut state, &os);
    let panel = state.panel_dock_node.expect("panel dock node was built");
    state.session.log_window_open = true;

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new(os.frame.display_size(), 1.0 / 60.0)
            .renderer_has_textures(),
    );
    let mut docked = false;
    let mut dock = None;
    {
        let ui = ctx.frame();
        super::dock_host(ui, &mut state, "274bot");
        super::log_window_with_body(ui, &mut state.session, Some(panel), |ui, session| {
            docked = ui.is_window_docked();
            dock = Some(ui.get_window_dock_id());
            crate::log_pane::log_body(ui, session, true);
        });
    }
    ctx.render();
    assert!(docked, "Log opens as a docked panel tab");
    assert_eq!(dock, Some(panel));
    assert!(super::PANEL_TAB_WINDOWS.contains(&"Log"));

    let class = super::panel_window_class();
    assert!(class.docking_always_tab_bar, "the tab has a close X");
    assert!(
        !class
            .dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING),
        "Log can be dragged out to float"
    );
}

#[test]
fn drawn_log_window_suppresses_the_inline_log_body() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut session = crate::session::Session::new();
    session.log_window_open = true;
    let mut ctx = dock_host_context();
    ctx.io_mut().set_config_flags(ConfigFlags::empty());
    ctx.set_ini_filename::<std::path::PathBuf>(None)
        .expect("disable ImGui settings persistence");
    let drawn = DrawnText::default();
    ctx.set_clipboard_backend(drawn.clone());
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([1440.0, 900.0], 1.0 / 60.0)
            .renderer_has_textures(),
    );
    let mut window_body_drawn = false;
    {
        let ui = ctx.frame();
        ui.log_to_clipboard(0u32);
        super::log_window_with_body(ui, &mut session, None, |ui, session| {
            window_body_drawn = true;
            crate::log_pane::log_body(ui, session, true);
        });
        ui.window("Inline log harness")
            .build(|| super::log_section(ui, &mut session, true));
        ui.log_finish();
    }
    ctx.render();

    let text = drawn.0.take();
    assert!(window_body_drawn, "the visible Log window drew its body");
    assert!(session.log_window_body_drawn);
    assert!(text.contains("Log is open in its tab"));
    assert_eq!(
        text.matches("[ Copy ]").count(),
        1,
        "only the Log window submitted its body"
    );
}

#[test]
fn unselected_docked_log_window_leaves_the_inline_body_visible() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    ctx.set_ini_filename::<std::path::PathBuf>(None)
        .expect("disable ImGui settings persistence");
    let drawn = DrawnText::default();
    ctx.set_clipboard_backend(drawn.clone());
    let os = fitted_frame((1920.0, 1080.0), (1120, 580), (400, 200));
    let mut state = PanelState {
        os_window: Some(os.clone()),
        ..PanelState::default()
    };
    dock_host_frame(&mut ctx, &mut state, &os);
    dock_host_frame(&mut ctx, &mut state, &os);
    let panel = state.panel_dock_node.expect("panel dock node was built");
    let panel_title = format!(
        "{}###{}",
        state.session.app_title(),
        crate::theme::PANEL_WINDOW
    );
    state.session.log_window_open = true;

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new(os.frame.display_size(), 1.0 / 60.0)
            .renderer_has_textures(),
    );
    {
        let ui = ctx.frame();
        super::dock_host(ui, &mut state, "274bot");
        ui.set_next_window_class(&super::panel_window_class());
        ui.set_next_window_dock_id_with_cond(panel, dear_imgui_rs::Condition::Always);
        ui.window(panel_title.as_str()).build(|| {});
        ui.set_window_focus(Some(panel_title.as_str()));
        super::log_window_with_body(ui, &mut state.session, Some(panel), |ui, session| {
            crate::log_pane::log_body(ui, session, true);
        });
        ui.set_window_focus(Some(panel_title.as_str()));
    }
    ctx.render();

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new(os.frame.display_size(), 1.0 / 60.0)
            .renderer_has_textures(),
    );
    let mut window_body_drawn = false;
    {
        let ui = ctx.frame();
        ui.log_to_clipboard(0u32);
        super::dock_host(ui, &mut state, "274bot");
        ui.set_window_focus(Some(panel_title.as_str()));
        super::log_window_with_body(ui, &mut state.session, Some(panel), |ui, session| {
            window_body_drawn = true;
            crate::log_pane::log_body(ui, session, true);
        });
        ui.set_next_window_class(&super::panel_window_class());
        ui.set_next_window_dock_id_with_cond(panel, dear_imgui_rs::Condition::Always);
        ui.window(panel_title.as_str())
            .build(|| super::log_section(ui, &mut state.session, true));
        ui.log_finish();
    }
    ctx.render();

    let text = drawn.0.take();
    assert!(state.session.log_window_open);
    assert!(
        !window_body_drawn,
        "an open but unselected dock tab does not draw its body"
    );
    assert!(!state.session.log_window_body_drawn);
    assert!(
        text.contains("[ Copy ]"),
        "the inline log body remains visible"
    );
    assert!(!text.contains("Log is open in its tab"));
}

#[test]
fn panel_window_class_allows_config_tabs() {
    let c = super::panel_window_class();
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(c
        .dock_node_flags_override_set
        .contains(dear_imgui_rs::DockNodeFlags::NO_DOCKING_SPLIT));
    assert!(
        !c.dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING),
        "configs must be able to tab onto 274bot and undock later"
    );
    assert!(
        c.docking_allow_unclassed,
        "General/Nav are unclassed and must merge with the panel"
    );
    assert!(
        c.docking_always_tab_bar,
        "panel tab bar is the drop target for configs"
    );
}

#[test]
fn dockspace_does_not_lock_undock_on_every_node() {
    let f = super::dock_flags();
    assert!(f.contains(dear_imgui_rs::DockNodeFlags::NO_DOCKING_SPLIT));
    assert!(f.contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
    assert!(
        !f.contains(dear_imgui_rs::DockNodeFlags::NO_UNDOCKING),
        "NO_UNDOCKING on the dockspace would lock the panel against config tabs"
    );
}

#[test]
fn apply_ui_scale_scales_padding_for_retina() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dear_imgui_rs::Context::create();
    let before = ctx.style().window_padding();
    apply_ui_scale(ctx.style_mut(), 2.0);
    let after = ctx.style().window_padding();
    assert_eq!(before, [8.0, 8.0]);
    assert_eq!(after, [16.0, 16.0]);
}

#[test]
fn amber_palette_preserves_scaled_scrollbars_across_monitor_changes() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    super::amber_style(&mut ctx);
    let base_style = ctx.style().clone();
    for (scale, size, rounding) in [(2.0, 12.0, 6.0), (1.5, 9.0, 4.0), (1.0, 6.0, 3.0)] {
        *ctx.style_mut() = base_style.clone();
        apply_ui_scale(ctx.style_mut(), scale);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([800.0, 600.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let _ui = ctx.frame();
        crate::theme::apply_amber_current(&crate::theme::ChromeColors::default());
        ctx.render();
        assert_eq!(
            ctx.style().scrollbar_size(),
            size,
            "scrollbar width at {scale}×"
        );
        assert_eq!(
            ctx.style().scrollbar_rounding(),
            rounding,
            "scrollbar rounding at {scale}×",
        );
    }
}

#[test]
fn fit_applet_keeps_aspect_and_does_not_dpi_double() {
    assert_eq!(native_applet(), [765.0, 503.0]);
    assert_eq!(fit_applet([765.0, 503.0]), [765.0, 503.0]);
    let off = applet_offset([1200.0, 700.0], [765.0, 503.0]);
    assert!(
        (off[0] - (1200.0 - 765.0) * 0.5).abs() < 0.01,
        "centred in the pane"
    );
    assert!((off[1] - (700.0 - 503.0) * 0.5).abs() < 0.01);
    // Grid cells downscale; the non-grid Game blit stays native_applet.
    assert_eq!(fit_applet([382.5, 251.5]), [382.5, 251.5]);
    let wide = fit_applet([2000.0, 503.0]);
    assert!((wide[1] - 503.0).abs() < 0.01);
    assert!(wide[0] <= 2000.0);
}

#[test]
fn game_window_flags_have_no_scrollbar() {
    let f = game_window_flags();
    assert!(f.contains(WindowFlags::NO_SCROLLBAR));
    assert!(f.contains(WindowFlags::NO_SCROLL_WITH_MOUSE));
    assert!(f.contains(WindowFlags::NO_RESIZE));
    assert!(!f.contains(WindowFlags::HORIZONTAL_SCROLLBAR));
}

#[test]
fn game_window_title_is_the_profile_name() {
    assert_eq!(game_window_title(Some("test")), "test");
    assert_eq!(game_window_title(None), "Game");
    assert_eq!(game_window_title(Some("")), "Game");
}

#[test]
fn panel_split_keeps_its_logical_width_at_every_scale() {
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        for base_width in [1120.0, 2000.0] {
            let width = base_width * scale;
            let ratio = panel_split_ratio(width, scale);
            assert!(
                (ratio * width - PANEL_WIDTH * scale).abs() < 0.01,
                "panel stays {} physical px at {scale}× in a {width}px window, got {}",
                PANEL_WIDTH * scale,
                ratio * width
            );
        }
    }
}

#[test]
fn parse_live_args_none_without_flag_or_env() {
    assert_eq!(
        parse_live_args([] as [&str; 0], None),
        Ok(RunMode::Interactive)
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("")),
        Ok(RunMode::Interactive)
    );
}

#[test]
fn parse_args_accepts_session_nav_paint_choice() {
    let parsed = parse_args(["--nav-paints", "on", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.nav_paints, Some(true));
    assert_eq!(parsed.mode, RunMode::Live("script_rock_crab".into()));

    let parsed = parse_args(["--nav-paints", "off", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.nav_paints, Some(false));
}

#[test]
fn parse_args_accepts_session_memory_choice_and_rejects_conflicts() {
    let parsed = parse_args(["--lowmem", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, Some(true));
    let parsed = parse_args(["--highmem", "--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, Some(false));
    let parsed = parse_args(["--live", "script_rock_crab"], None).unwrap();
    assert_eq!(parsed.memory_override, None);
    assert!(matches!(
        parse_args(["--lowmem", "--highmem"], None),
        Err((2, message)) if message.contains("conflict")
    ));
}

#[test]
fn parse_args_rejects_malformed_session_nav_paint_choice() {
    assert!(matches!(
        parse_args(["--nav-paints", "maybe"], None),
        Err((2, message)) if message == "panel-play: --nav-paints expects on or off, got maybe"
    ));
    assert!(matches!(
        parse_args(["--nav-paints"], None),
        Err((2, message)) if message == "panel-play: --nav-paints needs on or off"
    ));
}

#[test]
fn parse_live_args_env_and_flag_null_raster() {
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("null_raster")),
        Ok(RunMode::Live("null_raster".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "null_raster"], None),
        Ok(RunMode::Live("null_raster".into()))
    );
}

#[test]
fn parse_live_args_env_and_flag_stress50() {
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("stress50")),
        Ok(RunMode::Live("stress50".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "stress50"], None),
        Ok(RunMode::Live("stress50".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "stress50_full"], None),
        Ok(RunMode::Live("stress50_full".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("stress50_full")),
        Ok(RunMode::Live("stress50_full".into()))
    );
}

#[test]
fn parse_live_args_accepts_nav_full() {
    assert_eq!(
        parse_live_args(["--live", "nav_full"], None),
        Ok(RunMode::Live("nav_full".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("nav_full")),
        Ok(RunMode::Live("nav_full".into()))
    );
}

#[test]
fn parse_live_args_accepts_runtime_catalog_scenarios_without_building_them() {
    assert_eq!(
        parse_live_args(["--live", "script_quester_path"], None),
        Ok(RunMode::Live("script_quester_path".into()))
    );
}

#[test]
fn parse_live_args_flag_wins_over_env() {
    assert_eq!(
        parse_live_args(["--live", "null_raster"], Some("other")),
        Ok(RunMode::Live("null_raster".into()))
    );
}

#[test]
fn parse_live_args_smoke_flag_maps_to_smoke_mode() {
    assert_eq!(parse_live_args(["--smoke"], None), Ok(RunMode::Smoke));
    // A stray BOT_LIVE env does not demote --smoke.
    assert_eq!(
        parse_live_args(["--smoke"], Some("stress50")),
        Ok(RunMode::Smoke)
    );
    // --smoke wins over --live when both are passed.
    assert_eq!(
        parse_live_args(["--smoke", "--live", "null_raster"], None),
        Ok(RunMode::Smoke)
    );
}

#[test]
fn parse_live_args_unknown_name_is_usage_exit_2() {
    assert_eq!(
        parse_live_args(["--live", "nope"], None),
        Err((2, LIVE_USAGE.into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("other")),
        Err((2, LIVE_USAGE.into()))
    );
}

#[test]
fn parse_live_args_rejects_removed_prod_flag() {
    assert_eq!(
        parse_live_args(["--prod"], None),
        Err((2, "panel-play: unknown --prod".into()))
    );
}

#[test]
fn parse_live_args_unknown_flag_and_missing_name() {
    assert_eq!(
        parse_live_args(["--wat"], None),
        Err((2, "panel-play: unknown --wat".into()))
    );
    assert_eq!(
        parse_live_args(["--live"], None),
        Err((2, "panel-play: --live needs a name".into()))
    );
}

#[test]
fn parse_live_args_help_is_usage_exit_0() {
    assert_eq!(
        parse_live_args(["--help"], None),
        Err((0, LIVE_USAGE.into()))
    );
    assert_eq!(parse_live_args(["-h"], None), Err((0, LIVE_USAGE.into())));
}

#[test]
fn parse_live_args_script_walk_accepted() {
    assert_eq!(
        parse_live_args(["--live", "script_walk"], None),
        Ok(RunMode::Live("script_walk".into()))
    );
    assert_eq!(
        parse_live_args([] as [&str; 0], Some("script_walk")),
        Ok(RunMode::Live("script_walk".into()))
    );
    // The smoke scenario is a registered scenario, so the script_
    // harness can also drive it manually.
    assert_eq!(
        parse_live_args(["--live", "script_render_smoke"], None),
        Ok(RunMode::Live("script_render_smoke".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_nav_routes"], None),
        Ok(RunMode::Live("script_nav_routes".into()))
    );
    // The courtyard paint-path scenario is a registered scenario, so
    // the script_ harness drives it like script_nav_routes.
    assert_eq!(
        parse_live_args(["--live", "script_nav_paint_path"], None),
        Ok(RunMode::Live("script_nav_paint_path".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier_v2_js"], None),
        Ok(RunMode::Live("script_bone_burier_v2_js".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier_v2_ts"], None),
        Ok(RunMode::Live("script_bone_burier_v2_ts".into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_bone_burier"], None),
        Ok(RunMode::Live("script_bone_burier".into()))
    );
}

#[test]
fn parse_live_args_unknown_script_rejected() {
    assert_eq!(
        parse_live_args(["--live", "script_nope"], None),
        Err((2, LIVE_USAGE.into()))
    );
    assert_eq!(
        parse_live_args(["--live", "script_"], None),
        Err((2, LIVE_USAGE.into()))
    );
}

#[test]
fn parse_args_prepare_fixture_is_offline_mode() {
    let parsed = parse_args(["--prepare-fixture", "thiever"], None).unwrap();
    assert_eq!(parsed.mode, RunMode::PrepareFixture("thiever".into()));
    assert!(!parsed.run_prepared);
}

#[test]
fn parse_args_run_prepared_requires_script_live() {
    let parsed = parse_args(
        [
            "--live",
            "script_thiever",
            "--run-prepared",
            "--fixture-path",
            "/tmp/t.json",
        ],
        None,
    )
    .unwrap();
    assert_eq!(parsed.mode, RunMode::Live("script_thiever".into()));
    assert!(parsed.run_prepared);
    assert_eq!(
        parsed.fixture_path.as_deref(),
        Some(std::path::Path::new("/tmp/t.json"))
    );
    assert!(parse_args(["--run-prepared"], None).is_err());
    assert!(parse_args(
        ["--prepare-fixture", "thiever", "--live", "script_thiever"],
        None
    )
    .is_err());
    assert!(parse_args(["--prepare-fixture", "thiever", "--run-prepared"], None).is_err());
    assert!(parse_args(["--prepare-fixture", "nope"], None).is_err());
    let ts = parse_args(["--prepare-fixture", "bone_burier_v2_ts"], None).unwrap();
    assert_eq!(ts.mode, RunMode::PrepareFixture("bone_burier_v2_ts".into()));
    let js = parse_args(["--prepare-fixture", "bone_burier_v2_js"], None).unwrap();
    assert_eq!(js.mode, RunMode::PrepareFixture("bone_burier_v2_js".into()));
    assert_eq!(
        scenario::fixture_preset_for("bone_burier_v2_ts").unwrap(),
        "bone_burier_v2"
    );
    assert_eq!(scenario::fixture_preset_for("thiever").unwrap(), "thiever");
    assert!(scenario::fixture_preset_for("bone_burier").is_err());
}

#[test]
fn parse_live_args_accepts_external_loader_without_scenario_catalog() {
    assert_eq!(
        parse_live_args(["--live", "script_external_loader"], None),
        Ok(RunMode::Live("script_external_loader".into()))
    );
    assert!(scenario::get("external_loader").is_none());
}

#[test]
fn parse_args_external_ts_refuses_relative_and_keeps_ordinary_watchers_off() {
    let err = parse_args(["--external-ts", "ExampleBot.ts"], None).unwrap_err();
    assert_eq!(err.0, 2);
    assert!(err.1.contains("relative"), "{}", err.1);
    let args = parse_args(["--live", "script_bone_burier"], None).unwrap();
    assert!(!args.external_core);
    assert!(args.external_ts.is_none());
    assert!(!args.catalog_core);
    assert!(!args.pair_core);
}

#[test]
fn pair_watch_cli_cases_resolve_two_prepared_actors_and_shared_start() {
    use host_play::paired_core::{
        AirObservation, FlaxObservation, PairCase, PairWatch, StartBarrier, AIR_RUINS, FLAX_FIELD,
        FLAX_MEET, MULE_TRADE_CAP, TRADE_CAP,
    };
    use scenario::StepKind;

    for (cli, case, card, mode_a, mode_b) in [
        (
            "script_nature_crafter_air",
            PairCase::Air,
            "NatureCrafter",
            "Master",
            "Runner",
        ),
        (
            "script_mule_crafter_air",
            PairCase::Mule,
            "MuleCrafter",
            "Crafter",
            "Mule",
        ),
        (
            "script_flax_runner",
            PairCase::Flax,
            "FlaxRunner",
            "Runner",
            "Spinner",
        ),
    ] {
        assert_eq!(
            parse_live_args(["--live", cli], None),
            Ok(RunMode::Live(cli.into())),
            "{cli} must resolve as a headed pair_watch live name"
        );
        let name = cli.strip_prefix("script_").unwrap();
        assert_eq!(PairCase::parse(name).unwrap(), case);
        let scenario = scenario::get(name).unwrap();
        assert_eq!(scenario.seed.profiles.len(), 2);
        assert_ne!(scenario.seed.profiles[0].0, scenario.seed.profiles[1].0);
        assert_eq!(scenario.settings.start_script, Some(card));
        assert_eq!(scenario.companions.len(), 1);
        assert_eq!(scenario.companions[0].profile, 1);
        assert!(scenario
            .steps
            .iter()
            .any(|step| matches!(step.kind, StepKind::StartScript)));

        let a = "alice";
        let b = "bob";
        let bag_a = host_play::paired_core::pair_settings(case, &[], 0, a, b).unwrap();
        let bag_b = host_play::paired_core::pair_settings(case, &[], 1, b, a).unwrap();
        assert_eq!(bag_a.get("mode").and_then(|v| v.as_str()), Some(mode_a));
        assert_eq!(bag_b.get("mode").and_then(|v| v.as_str()), Some(mode_b));
        let screen_a = client::util::JString::to_screen_name(a);
        let screen_b = client::util::JString::to_screen_name(b);
        assert_eq!(
            bag_a.get("partner").and_then(|v| v.as_str()),
            Some(screen_b.as_str())
        );
        assert_eq!(
            bag_b.get("partner").and_then(|v| v.as_str()),
            Some(screen_a.as_str())
        );

        let watch = PairWatch::default();
        watch.configure(case, a, b);
        match case {
            PairCase::Air | PairCase::Mule => {
                let first = AirObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(a.into()),
                    tile: Some(AIR_RUINS),
                    air_talisman: 1,
                    essence_unnoted: if case == PairCase::Mule {
                        MULE_TRADE_CAP
                    } else {
                        0
                    },
                    ..AirObservation::default()
                };
                let second = AirObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(b.into()),
                    tile: Some(AIR_RUINS),
                    air_talisman: 0,
                    essence_unnoted: if case == PairCase::Mule {
                        MULE_TRADE_CAP
                    } else {
                        TRADE_CAP
                    },
                    ..AirObservation::default()
                };
                watch.observe_air(a, first, false);
                watch.observe_air(b, second, false);
                assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
                watch.begin_shared_start(a, b).unwrap();
            }
            PairCase::Flax => {
                let runner = FlaxObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(a.into()),
                    tile: Some(FLAX_FIELD),
                    crafting: 1,
                    ..FlaxObservation::default()
                };
                let mut spinner = FlaxObservation {
                    ingame: true,
                    scene_state: 2,
                    inventory_tab_available: true,
                    player: Some(b.into()),
                    tile: Some(FLAX_MEET),
                    crafting: 1,
                    ..FlaxObservation::default()
                };
                watch.observe_flax(a, runner, false);
                watch.observe_flax(b, spinner.clone(), false);
                assert_eq!(
                    watch.barrier(),
                    StartBarrier::Wait,
                    "Crafting 1 cannot spin flax"
                );
                spinner.crafting = 10;
                watch.observe_flax(b, spinner, false);
                assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
                watch.begin_shared_start(a, b).unwrap();
            }
            PairCase::Duel => unreachable!("this table is Air/Mule/Flax role bags"),
        }
    }

    let cli = "script_duel_arena";
    assert_eq!(
        parse_live_args(["--live", cli], None),
        Ok(RunMode::Live(cli.into())),
        "{cli} must resolve as a headed pair_watch live name"
    );
    assert_eq!(PairCase::parse("duel_arena").unwrap(), PairCase::Duel);
    let scenario = scenario::get("duel_arena").unwrap();
    assert_eq!(scenario.seed.profiles.len(), 2);
    assert_ne!(scenario.seed.profiles[0].0, scenario.seed.profiles[1].0);
    assert_eq!(
        scenario.settings.start_script,
        Some(PairCase::Duel.card_name())
    );
    assert_eq!(scenario.companions.len(), 1);
    assert!(scenario
        .steps
        .iter()
        .any(|step| matches!(step.kind, StepKind::StartScript)));
    let bag_a =
        host_play::paired_core::pair_settings(PairCase::Duel, &[], 0, "alice", "bob").unwrap();
    let bag_b =
        host_play::paired_core::pair_settings(PairCase::Duel, &[], 1, "bob", "alice").unwrap();
    assert!(bag_a.get("partner").is_none());
    assert!(bag_b.get("partner").is_none());
    let watch = PairWatch::default();
    watch.configure(PairCase::Duel, "alice", "bob");
    let first = host_play::paired_core::DuelObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some("alice".into()),
        tile: Some(host_play::paired_core::DUEL_CHALLENGE_ANCHOR),
        tick: 0,
        attack_xp: 0,
        strength_xp: 0,
        defence_xp: 0,
        hitpoints_xp: 0,
        in_combat: false,
        in_challenge_area: true,
        in_fight_pen: false,
        main_modal: -1,
        duel_offer_open: false,
        duel_confirm_open: false,
        duel_win_open: false,
        duel_partner: None,
        waiting_for_other: false,
        weapon_equipped: true,
        peer_visible: true,
    };
    let second = host_play::paired_core::DuelObservation {
        player: Some("bob".into()),
        ..first.clone()
    };
    watch.observe_duel("alice", first, false);
    watch.observe_duel("bob", second, false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth, "{cli}");
    watch.begin_shared_start("alice", "bob").unwrap();
}

#[test]
fn smoke_should_fire_table() {
    // Fires exactly once, only while armed, only at `ingame && scene 2`.
    let cases: &[(&str, bool, bool, bool, i32, bool)] = &[
        ("disarmed never fires", false, false, true, 2, false),
        ("fires at scene 2", true, false, true, 2, true),
        ("not before scene 2", true, false, true, 1, false),
        ("not while logged out", true, false, false, 2, false),
        ("not after scene 1", true, false, true, 0, false),
        ("not twice", true, true, true, 2, false),
        ("not after exit", true, true, true, 1, false),
    ];
    for (name, armed, fired, ingame, scene, expect) in cases {
        assert_eq!(
            smoke_should_fire(*armed, *fired, *ingame, *scene),
            *expect,
            "{name}"
        );
    }
}

#[test]
fn smoke_settled_table() {
    // The scene2 shot request reaches the render readback only once
    // the focused slot has held scene 2 for the full settle window and
    // is still ingame (the 1 fps renderer needs wall-clock time to
    // rasterize the world; an early capture is the title screen).
    let t0 = Instant::now();
    let cases: &[(&str, Option<Instant>, Instant, bool, bool)] = &[
        ("no scene 2 yet", None, t0, true, false),
        (
            "before the settle window",
            Some(t0),
            t0 + Duration::from_secs(1),
            true,
            false,
        ),
        (
            "at the settle boundary",
            Some(t0),
            t0 + SMOKE_SETTLE,
            true,
            true,
        ),
        (
            "after the settle window",
            Some(t0),
            t0 + Duration::from_secs(5),
            true,
            true,
        ),
        (
            "settled but logged out",
            Some(t0),
            t0 + Duration::from_secs(5),
            false,
            false,
        ),
    ];
    for (name, at, now, ingame, expect) in cases {
        assert_eq!(smoke_settled(*at, *now, *ingame), *expect, "{name}");
    }
}

#[test]
fn manual_shot_label_is_stamped_and_stays_normalized() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_616_000);
    let label = manual_shot_label(now);
    assert_eq!(label, "manual-2026-08-25T00-00-00");
    // The label is already in the 377 safe alphabet: the file name
    // normalizes to itself (the write path applies `safe_label`).
    assert_eq!(scenario::shot::safe_label(&label), label);
}

#[test]
fn manual_shot_without_a_focused_live_snapshot_is_not_enqueued() {
    let mut state = PanelState::default();
    super::enqueue_manual_shot(&mut state);
    assert!(state.shot_state.lock().unwrap().requests.is_empty());
}

#[test]
fn manual_shot_rejects_an_empty_published_snapshot() {
    let mut state = PanelState::default();
    state.session.set_focus_for_test("alice");
    state.session.nav_states.lock().unwrap().insert(
        "alice".into(),
        (
            api::snapshot::GameSnapshot::new(),
            nav::WorldState::default(),
        ),
    );
    super::enqueue_manual_shot(&mut state);
    assert!(state.shot_state.lock().unwrap().requests.is_empty());
}

/// A synthetic client that has already seeded: ingame, scene 2, a
/// mainland build base, and bumped family gens (same trick as the
/// scenario crate's own tests — no live server).
fn script_client() -> client::client::Client {
    let mut c = host::prepare_client(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        1,
        std::sync::Arc::new(client::config::Cache::default()),
        std::sync::Arc::new(vec![]),
        Vec::new(),
    );
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(client::dash3d::ClientPlayer::at(20, 20));
    for prot in [
        client::io::ServerProt::PLAYER_INFO,
        client::io::ServerProt::REBUILD_NORMAL,
        client::io::ServerProt::UPDATE_STAT,
    ] {
        c.bump_gens(prot);
    }
    c
}

#[test]
fn live_script_tick_holds_pass_until_clean_script_stop() {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};

    let mut s = crate::session::Session::new();
    let pass = Scenario {
        name: "bone_stop",
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
        settings: ScenarioSettings {
            wait_script_stop: Some("confirmed loaded current-generation bank exhaustion"),
            ..Default::default()
        },
    };
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = script_client();
    runner.tick(&mut c);
    c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
    runner.tick(&mut c);
    assert_eq!(runner.status(), scenario::RunnerStatus::Passed);
    assert_eq!(
        runner.wait_script_stop(),
        Some("confirmed loaded current-generation bank exhaustion")
    );
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None
    );
    assert!(!live.passed, "game-state PASS must wait for Idle + reason");
    s.live_script_stop_wait_started = Some(Instant::now() - Duration::from_secs(60));
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("clean-stop wait times out");
    assert!(error.contains("clean stop reason"), "{error}");
    assert!(live.failed.is_some());
}

#[test]
fn clean_stop_rearms_written_shot_once_and_can_complete() {
    let mut session = crate::session::Session::new();
    session.set_focus_for_test("alice");
    let client = script_client();
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    assert!(snapshot.ingame() && snapshot.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let label = "bone-stop";
    shots.lock().unwrap().mark_written(label);
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: Some(Instant::now() - NAV_FULL_SHOT_DRAIN),
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };

    request_clean_stop_capture(&mut live, &session, Some(&shots), Some(label))
        .expect("scene-2 snapshot can re-arm the original Written shot");
    assert!(
        live.clean_stop_capture_requested,
        "clean-stop capture latches after the original Written shot"
    );
    assert!(
        live.drain_started.is_none(),
        "re-arm resets only the capture drain window"
    );
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Requested,
        "the original Written label must be re-armed exactly once"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Written,
            Some(&shots),
        ),
        Ok(true),
        "the stale pre-Idle Written status must not complete before the new shot"
    );

    shots.lock().unwrap().mark_written(label);
    request_clean_stop_capture(&mut live, &session, Some(&shots), Some(label))
        .expect("latched clean-stop capture must not re-arm");
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Written,
        "a later clean-stop tick must not clear Written"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Requested,
            Some(&shots),
        ),
        Ok(false),
        "PASS can complete after the clean-stop capture writes"
    );
    assert!(live.failed.is_none());
}

#[test]
fn clean_stop_missing_scene_cannot_reuse_prior_written_capture() {
    let mut session = crate::session::Session::new();
    session.set_focus_for_test("alice");
    let mut client = script_client();
    client.scene_state = 1;
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("bone-stop");
    let mut live = LiveScript {
        name: "script_bone_stop".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    let error = request_clean_stop_capture(&mut live, &session, Some(&shots), Some("bone-stop"))
        .expect_err("missing scene-2 must fail closed");
    assert!(error.contains("scene-2"), "{error}");
    assert!(live.clean_stop_capture_requested);
    assert!(matches!(
        shots.lock().unwrap().status("bone-stop"),
        ShotStatus::Failed(ref failed) if failed.contains("scene-2")
    ));
    assert!(shots.lock().unwrap().requests.is_empty());
}

/// Headed contract: PASS latches `passed` (the caller exits 0),
/// FAIL returns the message the caller turns into exit 1.
#[test]
fn live_script_tick_latches_pass_reports_soak_readbacks_and_fail() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };

    let mut s = crate::session::Session::new();
    // A runnable micro-scenario: the send sets run energy, the arm
    // waits for it, the proof asserts it.
    let pass = Scenario {
        name: "t",
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
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), RunnerStatus::Passed);
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None,
        "PASS latches; the caller exits 0"
    );
    assert!(live.passed);

    // Dedicated catalog_watch: scenario PASS alone waits for the shared
    // core, and an unqualified core becomes FAIL at its own deadline.
    let watch = host_play::catalog_core::CoreWatch::default();
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    s.install_catalog_core_watch(Some(watch.clone()));
    live.passed = false;
    live.announced_pass = false;
    live.core_deadline = Some(Instant::now() + Duration::from_secs(60));
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None),
        None
    );
    assert!(!live.passed, "scenario-only PASS is not catalog core PASS");
    live.core_deadline = Some(Instant::now() - Duration::from_secs(1));
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("unqualified core times out as FAIL");
    assert!(error.contains("catalog core did not qualify"), "{error}");

    live.failed = None;
    live.native_failure_capture_requested = false;
    live.core_deadline = Some(Instant::now() + Duration::from_secs(60));
    s.scenario
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_terminal_shot("core-pass");
    s.set_focus_for_test("catalogtest");
    let mut terminal_snapshot = api::snapshot::GameSnapshot::new();
    terminal_snapshot.rebuild(&script_client());
    s.nav_states.lock().unwrap().insert(
        "catalogtest".into(),
        (terminal_snapshot, nav::WorldState::default()),
    );
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("core-pass");
    let mut baseline = host_play::catalog_core::Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..host_play::catalog_core::Observation::default()
    };
    baseline.levels.insert("thieving".into(), 50);
    baseline.levels.insert("hitpoints".into(), 50);
    baseline.effective_levels.insert("thieving".into(), 50);
    baseline.effective_levels.insert("hitpoints".into(), 50);
    baseline.items.insert("Lobster".into(), 10);
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    baseline.xp.insert("thieving".into(), 1);
    baseline.items.insert("Coins".into(), 1);
    watch.observe("catalogtest", baseline, false);
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert!(!live.passed, "qualified core waits for its current capture");
    assert_eq!(
        shots.lock().unwrap().status("core-pass"),
        ShotStatus::Requested,
        "the scenario's earlier XP capture cannot discharge the terminal core proof"
    );
    shots.lock().unwrap().mark_written("core-pass");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, Some(&shots),),
        None
    );
    assert!(
        live.passed,
        "full shared core and its capture permit headed PASS"
    );
    watch.clear();

    live.passed = false;
    live.announced_pass = false;
    live.soak = true;
    live.soak_until = Some(Instant::now() + Duration::from_secs(60));
    live.soak_capture = SoakCapture::NotNeeded;
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None,
        "BUDGET_S soak prints PASS but does not latch exit"
    );
    assert!(!live.passed, "window stays open after proof PASS");
    assert!(live.announced_pass);
    assert!(matches!(
        live.soak_capture,
        SoakCapture::PostPass { ref label, .. } if label == "core-pass-postpass"
    ));
    assert_eq!(
        shots.lock().unwrap().status("core-pass-postpass"),
        ShotStatus::Requested,
        "PASS must enqueue a fresh post-pass checkpoint"
    );
    shots.lock().unwrap().mark_written("core-pass-postpass");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert_eq!(live.soak_capture, SoakCapture::WaitingForFinal);
    live.soak_until = Some(Instant::now() - Duration::from_secs(1));
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert!(matches!(
        live.soak_capture,
        SoakCapture::Final { ref label, .. } if label == "core-pass-soak-final"
    ));
    assert!(!live.passed, "final readback must precede exit 0");
    assert_eq!(
        shots.lock().unwrap().status("core-pass-soak-final"),
        ShotStatus::Requested,
        "deadline must enqueue a distinct final readback"
    );
    shots.lock().unwrap().mark_written("core-pass-soak-final");
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    assert_eq!(live.soak_capture, SoakCapture::Complete);
    assert!(live.passed, "exit 0 only after the final readback writes");

    // A failed post-pass readback is a real harness failure, not a
    // successful soak with missing evidence.
    s.scenario
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_terminal_shot("fail-soak");
    shots.lock().unwrap().mark_written("fail-soak");
    let mut failed_soak = LiveScript {
        name: "script_fail_soak".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: true,
        soak_until: Some(Instant::now() + Duration::from_secs(60)),
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut failed_soak, &mut s, &ShotStatus::Written, Some(&shots),),
        None
    );
    shots.lock().unwrap().fail_labels(
        &["fail-soak-postpass".to_string()],
        "synthetic readback failure",
    );
    let error = live_script_tick(&mut failed_soak, &mut s, &ShotStatus::Written, Some(&shots))
        .expect("failed post-pass readback must fail the soak");
    assert!(error.contains("soak checkpoint") && error.contains("synthetic"));
    assert!(failed_soak.failed.is_some());

    // FAIL: a never-satisfiable arm within a 1-tick budget.
    let fail_scenario = Scenario {
        name: "f",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "never",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 999 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 999 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(fail_scenario);
    runner.set_scene_settle(Duration::ZERO);
    {
        let mut c = script_client();
        runner.tick(&mut c);
    }
    assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_f".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    // No terminal shot armed: the FAIL returns immediately.
    let msg = live_script_tick(&mut live, &mut s, &ShotStatus::Missing, None)
        .expect("FAIL returns the message");
    assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
    assert!(live.failed.is_some());
}

/// A terminal-shot FAIL holds the exit until the shot writes (or the
/// drain lapses): the first frame returns `None` (the request landed
/// after `pump_shots` ran), the write frame returns the message.
#[test]
fn live_script_tick_holds_a_terminal_shot_fail_until_the_shot_writes() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };
    let mut s = crate::session::Session::new();
    let fail_scenario = Scenario {
        name: "f",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "never",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat { id: 16, min: 999 },
                budget_ticks: 1,
            },
        }],
        proof: Proof::Stat { id: 16, min: 999 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(fail_scenario);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot("t-fail");
    {
        let mut c = script_client();
        runner.tick(&mut c);
    }
    assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "nav_full".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None),
        None,
        "the first FAIL frame holds so the shot can land"
    );
    assert!(live.drain_started.is_some());
    // The shot writes on a later frame: the FAIL is returned.
    let msg = live_script_tick(&mut live, &mut s, &ShotStatus::Written, None)
        .expect("FAIL after the shot writes");
    assert!(live.failed.is_some());
    assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
}

/// A terminal-shot PASS holds the exit until the shot writes, then
/// latches `passed` so the caller exits 0.
#[test]
fn live_script_tick_holds_a_terminal_shot_pass_until_the_shot_writes() {
    use scenario::{
        Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
    };
    let mut s = crate::session::Session::new();
    let pass = Scenario {
        name: "t",
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
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot("t-pass");
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), RunnerStatus::Passed);
    *s.scenario.lock().unwrap() = Some(runner);
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None),
        None,
        "the first PASS frame holds so the shot can land"
    );
    assert!(!live.passed);
    assert!(live.drain_started.is_some());
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, None),
        None
    );
    assert!(live.passed, "PASS after the shot writes; caller exits 0");

    live.passed = false;
    live.failed = None;
    live.announced_pass = false;
    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Requested, None)
        .expect("PASS with a missing terminal shot must fail after the drain bound");
    assert!(error.contains("terminal shot"), "error: {error}");
    assert!(error.contains("not written"), "error: {error}");
    assert!(live.failed.is_some(), "the missing capture latches failure");
}

#[test]
fn terminal_shot_drain_accepts_a_write_from_an_earlier_core_pending_frame() {
    let mut live = LiveScript {
        name: "script_t".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        super::hold_terminal_shot(
            &mut live,
            Some("t-pass"),
            &crate::window::ShotStatus::Written,
        ),
        Ok(false)
    );
    assert!(live.drain_started.is_none());
}

#[test]
fn native_failure_rearms_written_shot_and_waits_for_current_capture() {
    let mut session = crate::session::Session::new();
    session.set_focus_for_test("alice");
    let client = script_client();
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    assert!(snapshot.ingame() && snapshot.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let label = "thiever";
    shots.lock().unwrap().mark_written(label);
    let mut live = LiveScript {
        name: "script_thiever".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };

    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    request_native_failure_capture(&mut live, &session, Some(&shots), Some(label));
    assert!(live.native_failure_capture_requested);
    assert!(live.drain_started.is_none(), "re-arm resets the old drain");
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Requested,
        "a prior Written capture must be re-armed"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Written,
            Some(&shots),
        ),
        Ok(true),
        "the stale pre-tick Written status must not exit before the new shot"
    );
    assert!(live.drain_started.is_some());

    shots.lock().unwrap().mark_written(label);
    request_native_failure_capture(&mut live, &session, Some(&shots), Some(label));
    assert_eq!(
        shots.lock().unwrap().status(label),
        ShotStatus::Written,
        "a later failure tick must not re-arm the current capture"
    );
    assert_eq!(
        hold_script_terminal_shot(
            &mut live,
            &session,
            Some(label),
            &ShotStatus::Requested,
            Some(&shots),
        ),
        Ok(false),
        "exit is released only after the current capture is written"
    );
}

#[test]
fn native_failure_missing_scene_cannot_reuse_prior_written_capture() {
    let mut session = crate::session::Session::new();
    session.set_focus_for_test("alice");
    let mut client = script_client();
    client.scene_state = 1;
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(&client);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snapshot, nav::WorldState::default()));
    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().mark_written("thiever");
    super::enqueue_current_terminal_shot(&session, &shots, "thiever");
    assert!(matches!(
        shots.lock().unwrap().status("thiever"),
        ShotStatus::Failed(error) if error.contains("scene-2")
    ));
    assert!(shots.lock().unwrap().requests.is_empty());
}

#[test]
fn native_failure_receipt_uses_inner_scenario_identity() {
    let evidence = scenario::Evidence {
        scenario: "thiever".into(),
        outcome: "PASS",
        predicate: "stat(16)>=0".into(),
        ticks: 1,
        elapsed_ms: 2,
        message: None,
        tile: None,
        inv: Vec::new(),
        stat: None,
        chat: Vec::new(),
        receipt: None,
        protect_window: None,
        scene: 2,
    };
    assert_eq!(
        script_failure_scenario("script_thiever", Some(&evidence)),
        "thiever"
    );
    assert_eq!(script_failure_scenario("script_thiever", None), "thiever");
}

#[test]
fn pair_terminal_decision_requests_scene2_snapshots_for_both_actors() {
    use std::collections::HashSet;

    let mut s = crate::session::Session::new();
    let scenario = scenario::get("nature_crafter_air").expect("paired cell");
    let mut runner = scenario::ScenarioRunner::new(scenario);
    runner.set_live_names(&["alice".into(), "bob".into()]);
    *s.scenario.lock().unwrap() = Some(runner);

    let watch = host_play::paired_core::PairWatch::default();
    watch.configure(host_play::paired_core::PairCase::Air, "alice", "bob");
    s.install_paired_core_watch(Some(watch));

    let mut alice_client = script_client();
    let mut alice_player = client::dash3d::ClientPlayer::at(20, 20);
    alice_player.name = Some("NatureMaster".into());
    alice_client.local_player = Some(alice_player);
    let mut snap_a = api::snapshot::GameSnapshot::new();
    snap_a.rebuild(&alice_client);

    let mut bob_client = script_client();
    bob_client.local_player = None;
    let mut snap_b = api::snapshot::GameSnapshot::new();
    snap_b.rebuild(&bob_client);

    assert!(snap_a.ingame() && snap_a.scene_state() == 2);
    assert!(snap_b.ingame() && snap_b.scene_state() == 2);
    s.nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snap_a, nav::WorldState::default()));
    s.nav_states
        .lock()
        .unwrap()
        .insert("bob".into(), (snap_b, nav::WorldState::default()));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    let mut live = LiveScript {
        name: "script_nature_crafter_air".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: Some(Instant::now() - Duration::from_secs(1)),
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Missing, Some(&shots)),
        None,
        "pair terminal FAIL holds until both actor shots are requested"
    );
    let guard = shots.lock().unwrap();
    assert_eq!(guard.requests.len(), 2, "both actors must be captured");
    let mut actors = HashSet::new();
    let mut payloads = HashSet::new();
    for request in &guard.requests {
        let label = &request.label;
        let json = &request.snapshot_json;
        let value: serde_json::Value = serde_json::from_str(json).expect("snapshot json");
        assert_eq!(value.get("ingame"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(value.get("scene_state"), Some(&serde_json::json!(2)));
        let actor = value
            .get("actor")
            .and_then(|v| v.as_str())
            .expect("actor label metadata is the profile identity");
        assert_eq!(
            request.actor.as_deref(),
            Some(actor),
            "queued capture must be bound to the selected actor, not a label clone"
        );
        assert!(
            label.contains(actor),
            "label {label} must name actor {actor}"
        );
        match actor {
            "alice" => {
                assert_eq!(
                    value.pointer("/player/player/actor/name"),
                    Some(&serde_json::json!("NatureMaster")),
                    "observed player name must be retained"
                );
                assert!(
                    value
                        .get("player")
                        .and_then(|player| player.get("name"))
                        .is_none(),
                    "actor metadata must not overwrite or synthesize player.name"
                );
            }
            "bob" => {
                assert_eq!(
                    value.get("player"),
                    Some(&serde_json::Value::Null),
                    "missing-player stays missing"
                );
            }
            other => panic!("unexpected actor {other}"),
        }
        actors.insert(actor.to_string());
        payloads.insert(json.clone());
    }
    assert_eq!(
        actors.len(),
        2,
        "paired headed snapshots must identify two distinct actors"
    );
    assert_eq!(
        payloads.len(),
        2,
        "paired headed snapshots must retain distinct observed payloads"
    );
}

fn dummy_slot() -> crate::session::SlotIo {
    crate::session::SlotIo {
        input: host::SlotInput::new(),
        pixels: host::FrameBuf::new(),
    }
}

fn insert_named_scene2(session: &crate::session::Session, name: &str, player: Option<&str>) {
    let mut client = script_client();
    match player {
        Some(player_name) => {
            let mut player = client::dash3d::ClientPlayer::at(20, 20);
            player.name = Some(player_name.into());
            client.local_player = Some(player);
        }
        None => client.local_player = None,
    }
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.ingame() && snap.scene_state() == 2);
    session
        .nav_states
        .lock()
        .unwrap()
        .insert(name.into(), (snap, nav::WorldState::default()));
}

fn sidecar_actor_and_scene(json: &str) -> (String, i64) {
    let value: serde_json::Value = serde_json::from_str(json).expect("sidecar json");
    (
        value
            .get("actor")
            .and_then(|v| v.as_str())
            .expect("actor")
            .to_string(),
        value
            .get("scene_state")
            .and_then(|v| v.as_i64())
            .expect("scene_state"),
    )
}

#[test]
fn pump_shots_does_not_promote_two_pair_actors_from_an_unready_buffer() {
    let mut state = PanelState::default();
    state.session.core.insert_slot_io("alice", dummy_slot());
    state.session.core.insert_slot_io("bob", dummy_slot());
    state.session.set_focus_for_test("alice");
    insert_named_scene2(&state.session, "alice", Some("NatureMaster"));
    insert_named_scene2(&state.session, "bob", None);
    {
        let mut shots = state.shot_state.lock().unwrap();
        shots.enqueue_for_actor(
            "air-alice".into(),
            "{\"actor\":\"alice\"}".into(),
            "alice".into(),
        );
        shots.enqueue_for_actor("air-bob".into(), "{\"actor\":\"bob\"}".into(), "bob".into());
    }

    assert_eq!(super::pump_shots(&mut state), 0);
    {
        let shots = state.shot_state.lock().unwrap();
        assert!(
            shots.wanted.is_empty(),
            "unready selected buffer must not capture"
        );
        assert_eq!(shots.requests.len(), 2);
        assert_eq!(shots.status("air-alice"), ShotStatus::Requested);
        assert_eq!(shots.status("air-bob"), ShotStatus::Requested);
    }

    state.last_upload = Some(("alice".into(), 1));
    assert_eq!(super::pump_shots(&mut state), 0);
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "air-alice");
        assert_eq!(
            sidecar_actor_and_scene(&shots.wanted[0].1),
            ("alice".into(), 2),
            "promoted sidecar must be the current scene2 snapshot, not the enqueue-time clone"
        );
        assert_eq!(shots.requests.len(), 1);
        assert_eq!(shots.requests[0].actor.as_deref(), Some("bob"));
    }

    state.shot_state.lock().unwrap().wanted.clear();
    state.shot_state.lock().unwrap().mark_written("air-alice");
    state.last_upload = Some(("alice".into(), 1));
    super::pump_shots(&mut state);
    assert_eq!(
        state.session.focused_name().as_deref(),
        Some("bob"),
        "next queued actor is selected only after the previous capture writes"
    );
    assert!(
        state.shot_state.lock().unwrap().wanted.is_empty(),
        "bob is focused but not yet presented"
    );

    state.last_upload = Some(("bob".into(), 2));
    super::pump_shots(&mut state);
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "air-bob");
        assert_eq!(
            sidecar_actor_and_scene(&shots.wanted[0].1),
            ("bob".into(), 2)
        );
        assert!(shots.requests.is_empty());
    }
}

#[test]
fn pump_shots_fails_a_missing_pair_actor_without_deadlocking() {
    let mut state = PanelState::default();
    state.shot_state.lock().unwrap().enqueue_for_actor(
        "air-ghost".into(),
        "{\"actor\":\"ghost\"}".into(),
        "ghost".into(),
    );
    assert_eq!(super::pump_shots(&mut state), 0);
    let shots = state.shot_state.lock().unwrap();
    match shots.status("air-ghost") {
        ShotStatus::Failed(error) => {
            assert!(
                error.contains("not a live slot"),
                "missing actor must fail closed: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(shots.requests.is_empty());
    assert!(shots.wanted.is_empty());
}

#[test]
fn pump_shots_does_not_promote_a_presented_actor_that_left_scene2() {
    let mut state = PanelState::default();
    state.session.core.insert_slot_io("alice", dummy_slot());
    state.session.set_focus_for_test("alice");
    state.last_upload = Some(("alice".into(), 1));
    state.session.nav_states.lock().unwrap().insert(
        "alice".into(),
        (
            api::snapshot::GameSnapshot::new(),
            nav::WorldState::default(),
        ),
    );
    state.shot_state.lock().unwrap().enqueue_for_actor(
        "air-alice".into(),
        "{\"actor\":\"alice\",\"ingame\":true,\"scene_state\":2}".into(),
        "alice".into(),
    );
    assert_eq!(super::pump_shots(&mut state), 0);
    let shots = state.shot_state.lock().unwrap();
    assert!(
        shots.wanted.is_empty(),
        "stale scene2 sidecar must not prove a currently offworld actor"
    );
    assert_eq!(shots.status("air-alice"), ShotStatus::Requested);
    assert_eq!(
        shots.requests[0].snapshot_json,
        "{\"actor\":\"alice\",\"ingame\":true,\"scene_state\":2}"
    );
}

fn actor_snapshot_serialization_count() -> u64 {
    super::ACTOR_SNAPSHOT_SERIALIZATIONS.with(|count| count.get())
}

#[test]
fn pump_shots_does_not_serialize_sidecar_without_a_pending_actor_capture() {
    let mut state = PanelState::default();
    state.session.core.insert_slot_io("alice", dummy_slot());
    state.session.set_focus_for_test("alice");
    insert_named_scene2(&state.session, "alice", Some("NatureMaster"));
    state.last_upload = Some(("alice".into(), 1));

    let before = actor_snapshot_serialization_count();
    assert_eq!(super::pump_shots(&mut state), 0);
    assert_eq!(
        actor_snapshot_serialization_count(),
        before,
        "ordinary focused scene2 must not serialize a pair sidecar with no actor-tagged request"
    );
    assert!(state.shot_state.lock().unwrap().wanted.is_empty());
    assert!(state.shot_state.lock().unwrap().requests.is_empty());

    state
        .shot_state
        .lock()
        .unwrap()
        .enqueue("manual".into(), "{\"untagged\":true}".into());
    let before = actor_snapshot_serialization_count();
    assert_eq!(super::pump_shots(&mut state), 0);
    assert_eq!(
        actor_snapshot_serialization_count(),
        before,
        "untagged promote must not require pair sidecar serialization"
    );
    {
        let shots = state.shot_state.lock().unwrap();
        assert_eq!(shots.wanted.len(), 1);
        assert_eq!(shots.wanted[0].0, "manual");
        assert_eq!(shots.wanted[0].1, "{\"untagged\":true}");
        assert!(shots.requests.is_empty());
    }
}

#[test]
fn pair_deadline_fails_remaining_queued_actors() {
    let s = crate::session::Session::new();
    let scenario = scenario::get("nature_crafter_air").expect("paired cell");
    let mut runner = scenario::ScenarioRunner::new(scenario);
    runner.set_live_names(&["alice".into(), "bob".into()]);
    *s.scenario.lock().unwrap() = Some(runner);
    let watch = host_play::paired_core::PairWatch::default();
    watch.configure(host_play::paired_core::PairCase::Air, "alice", "bob");
    s.install_paired_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots.lock().unwrap().enqueue_for_actor(
        "nature_crafter_air-alice".into(),
        "{}".into(),
        "alice".into(),
    );
    shots.lock().unwrap().enqueue_for_actor(
        "nature_crafter_air-bob".into(),
        "{}".into(),
        "bob".into(),
    );
    let mut live = LiveScript {
        name: "script_nature_crafter_air".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: Some(Instant::now() - NAV_FULL_SHOT_DRAIN),
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    let error = super::hold_script_terminal_shot(
        &mut live,
        &s,
        Some("nature_crafter_air"),
        &ShotStatus::Missing,
        Some(&shots),
    )
    .expect_err("drain lapse is FAIL");
    assert!(error.contains("not written"), "{error}");
    let guard = shots.lock().unwrap();
    assert!(matches!(
        guard.status("nature_crafter_air-alice"),
        ShotStatus::Failed(_)
    ));
    assert!(matches!(
        guard.status("nature_crafter_air-bob"),
        ShotStatus::Failed(_)
    ));
    assert!(guard.requests.is_empty());
    assert!(guard.wanted.is_empty());
}

#[test]
fn record_presented_upload_ignores_a_failed_take_after_focus_switch() {
    let mut last = Some(("alice".into(), 3));
    super::record_presented_upload(&mut last, "bob".into(), 4, false);
    assert_eq!(last, Some(("alice".into(), 3)));
    super::record_presented_upload(&mut last, "bob".into(), 4, true);
    assert_eq!(last, Some(("bob".into(), 4)));
}

fn passed_prereq_runner() -> scenario::ScenarioRunner {
    use scenario::{Proof, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
    let pass = Scenario {
        name: "t",
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
    let mut runner = ScenarioRunner::new(pass);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_terminal_shot(host_play::external_loader::PREREQ_SHOT);
    {
        let mut c = script_client();
        runner.tick(&mut c);
        c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
    }
    assert_eq!(runner.status(), scenario::RunnerStatus::Passed);
    runner
}

fn drive_external_watch_to_capture(watch: &host_play::external_loader::ExternalWatch) {
    use host_play::external_loader::{BONES_COUNT, FROZEN_SHA256, NOTHING_CHANGED, SCRIPT_NAME};
    use std::path::Path;
    let t0 = Instant::now();
    watch.configure(
        "alice",
        PathBuf::from("/tmp/ExampleBot.ts"),
        FROZEN_SHA256.into(),
    );
    watch.note_scene(true, 2);
    watch.note_inventory(t0, "alice", BONES_COUNT, 0);
    watch.note_prereq_passed();
    watch.note_load(
        1,
        SCRIPT_NAME,
        Path::new("/tmp/ExampleBot.ts"),
        "file:/tmp/ExampleBot.ts",
        "compiled-a",
        true,
        false,
    );
    watch.begin_start(t0).unwrap();
    let lines: Vec<String> = (1..=10)
        .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
        .collect();
    watch.note_logs(t0, "alice", &lines);
    watch.note_inventory(t0, "alice", 12, 45);
    let stop = t0 + Duration::from_millis(8);
    watch.request_stop(stop);
    watch.note_logs(
        stop,
        "alice",
        &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
    );
    watch.note_stop(stop + Duration::from_millis(40), true, false);
    watch.note_reload_unchanged(NOTHING_CHANGED);
    watch.note_reload_changed(
        1,
        true,
        false,
        Path::new("/tmp/ExampleBot.ts"),
        "file:/tmp/ExampleBot.ts",
        "sha-before",
        "sha-after",
        "compiled-a",
        "compiled-b",
        true,
        false,
    );
}

fn alice_scene2_session() -> crate::session::Session {
    let s = crate::session::Session::new();
    *s.scenario.lock().unwrap() = Some(passed_prereq_runner());
    let mut client = script_client();
    let mut player = client::dash3d::ClientPlayer::at(20, 20);
    player.name = Some("Alice".into());
    client.local_player = Some(player);
    let mut snap = api::snapshot::GameSnapshot::new();
    snap.rebuild(&client);
    assert!(snap.ingame() && snap.scene_state() == 2);
    s.nav_states
        .lock()
        .unwrap()
        .insert("alice".into(), (snap, nav::WorldState::default()));
    s
}

#[test]
fn earlier_fixture_written_cannot_discharge_external_terminal_hold() {
    let mut s = alice_scene2_session();
    let watch = host_play::external_loader::ExternalWatch::default();
    drive_external_watch_to_capture(&watch);
    s.install_external_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::PREREQ_SHOT);

    let mut live = LiveScript {
        name: "script_external_loader".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None,
        "prereq Written must not latch PASS before the post-run shot"
    );
    assert!(!live.passed);
    assert_eq!(
        shots
            .lock()
            .unwrap()
            .status(host_play::external_loader::TERMINAL_SHOT),
        ShotStatus::Requested
    );

    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::TERMINAL_SHOT);
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None
    );
    assert!(
        live.passed,
        "PASS only after the external terminal shot writes"
    );
}

#[test]
fn missing_external_terminal_shot_fails_after_drain() {
    let mut s = alice_scene2_session();
    let watch = host_play::external_loader::ExternalWatch::default();
    drive_external_watch_to_capture(&watch);
    s.install_external_core_watch(Some(watch));

    let shots = std::sync::Mutex::new(crate::window::ShotState::default());
    shots
        .lock()
        .unwrap()
        .mark_written(host_play::external_loader::PREREQ_SHOT);

    let mut live = LiveScript {
        name: "script_external_loader".into(),
        passed: false,
        failed: None,
        last_step: None,
        drain_started: None,
        soak: false,
        soak_until: None,
        announced_pass: false,
        native_failure_capture_requested: false,
        clean_stop_capture_requested: false,
        core_deadline: None,
        soak_capture: SoakCapture::NotNeeded,
    };
    assert_eq!(
        live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots)),
        None
    );
    live.drain_started = Some(Instant::now() - NAV_FULL_SHOT_DRAIN);
    let error = live_script_tick(&mut live, &mut s, &ShotStatus::Written, Some(&shots))
        .expect("missing external shot is FAIL");
    assert!(error.contains("external_loader terminal"), "{error}");
    assert!(error.contains("not written"), "{error}");
    assert!(live.failed.is_some());
    assert!(!live.passed);
}

#[test]
fn pump_shots_marks_completion_only_after_the_png_and_snapshot_pair_write() {
    let dir = TestDir::new("shot-pump");

    let mut state = PanelState {
        shot_dir: Some(dir.to_path_buf()),
        ..PanelState::default()
    };
    state
        .shot_state
        .lock()
        .unwrap()
        .done
        .push(crate::window::ShotCapture {
            label: "gnome_chop".into(),
            snapshot_json: "{\"scene\":2}".into(),
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            #[cfg(feature = "render-diagnostics")]
            pixel_roi: None,
        });

    assert_eq!(super::pump_shots(&mut state), 1);
    assert_eq!(
        state.shot_state.lock().unwrap().status("gnome_chop"),
        ShotStatus::Written
    );
    let mut extensions = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    extensions.sort();
    assert_eq!(extensions, ["json", "png"]);
}

fn smoke_at(started: Instant) -> LiveSmoke {
    LiveSmoke {
        started,
        last_step: None,
        failed: None,
        saw_scene2_at: None,
        passed: false,
    }
}

#[test]
fn live_smoke_tick_passes_when_the_shot_is_written() {
    let mut s = crate::session::Session::new();
    let mut live = smoke_at(Instant::now());
    let statuses = [st("test", true, 2)];
    // A written shot (pump_shots drained `done`) exits the smoke: the
    // tick latches passed and the caller turns it into exit 0.
    assert_eq!(live_smoke_tick(&mut live, &mut s, &statuses, 1), None);
    assert!(live.passed, "the written scene2 shot passes the smoke");
}

#[test]
fn live_smoke_tick_latches_scene2_once_and_reports_a_missing_write() {
    let mut s = crate::session::Session::new();
    s.set_focus_for_test("test");
    let mut live = smoke_at(Instant::now());
    // Before scene 2 nothing latches.
    assert_eq!(
        live_smoke_tick(&mut live, &mut s, &[st("test", true, 1)], 0),
        None
    );
    assert!(!live.saw_scene2_at.is_some());
    // Scene 2 on the focused slot latches exactly once (the pure
    // trigger), and a late deadline names the missing write.
    let scene2 = [st("test", true, 2)];
    assert_eq!(live_smoke_tick(&mut live, &mut s, &scene2, 0), None);
    assert!(live.saw_scene2_at.is_some());
    live.started = Instant::now() - SMOKE_DEADLINE;
    let err = live_smoke_tick(&mut live, &mut s, &scene2, 0).expect("deadline");
    assert!(err.contains("never written within 300s"), "err: {err}");
}

#[test]
fn live_smoke_tick_deadline_reports_scene2_never_reached() {
    let mut s = crate::session::Session::new();
    s.set_focus_for_test("test");
    let mut live = smoke_at(Instant::now() - SMOKE_DEADLINE);
    let err = live_smoke_tick(&mut live, &mut s, &[st("test", false, 0)], 0).expect("deadline");
    assert!(
        err.contains("never reached scene 2 within 300s"),
        "err: {err}"
    );
    assert!(live.failed.is_some(), "the deadline latches the failure");
}

fn st(name: &str, ingame: bool, scene: i32) -> host_play::SlotStatus {
    host_play::SlotStatus {
        username: name.into(),
        ingame,
        scene_state: scene,
        ..Default::default()
    }
}

fn live_at(started: Instant) -> LiveNull {
    LiveNull {
        started,
        saw_scene2: false,
        passed: false,
    }
}

#[test]
fn live_null_tick_waits_until_two_scene2() {
    let mut live = live_at(Instant::now());
    let statuses = [st("test", true, 1), st("test2", false, 0)];
    assert_eq!(live_null_tick(&mut live, &statuses), None);
    assert!(!live.saw_scene2);
}

#[test]
fn live_null_tick_timeout_before_scene2() {
    let mut live = live_at(Instant::now() - Duration::from_secs(120));
    let statuses = [st("test", true, 2)];
    let err = live_null_tick(&mut live, &statuses).expect("timeout");
    assert!(err.contains("1/2"), "{err}");
    assert!(err.contains("120s"), "{err}");
}

#[test]
fn live_null_tick_passes_at_scene2_without_freeze() {
    let mut live = live_at(Instant::now());
    let scene2 = [st("test", true, 2), st("test2", true, 2)];
    assert_eq!(live_null_tick(&mut live, &scene2), None);
    assert!(live.passed);
    assert!(live.saw_scene2);
    assert_eq!(live_null_tick(&mut live, &scene2), None, "stay passed");
}

fn stress_at(started: Instant) -> LiveStress {
    LiveStress {
        started,
        last_announced: 0,
        passed: false,
        name: "stress50",
        host: "127.0.0.1".into(),
        port: 43594,
    }
}

fn ready_n(n: usize) -> Vec<host_play::SlotStatus> {
    (0..n).map(|i| st(&format!("s{i:02}"), true, 2)).collect()
}

#[test]
fn live_stress_tick_announces_1_10_50_and_stays_passed() {
    let mut live = stress_at(Instant::now());
    assert_eq!(live_stress_tick(&mut live, &[]), None);
    assert_eq!(live.last_announced, 0);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(1)), None);
    assert_eq!(live.last_announced, 1);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(10)), None);
    assert_eq!(live.last_announced, 10);
    assert!(!live.passed);

    assert_eq!(live_stress_tick(&mut live, &ready_n(50)), None);
    assert_eq!(live.last_announced, 50);
    assert!(live.passed);
    assert_eq!(
        live_stress_tick(&mut live, &ready_n(50)),
        None,
        "stay passed"
    );
    assert!(live.passed);
}

#[test]
fn live_stress_tick_timeout_before_50() {
    let mut live = stress_at(Instant::now() - Duration::from_secs(600));
    let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
    assert_eq!(err, "live stress50: 1/50 up after 600s");
    assert!(!live.passed);
    assert_eq!(live.last_announced, 1);
}

#[test]
fn live_stress_tick_full_name_in_timeout() {
    let mut live = LiveStress {
        started: Instant::now() - Duration::from_secs(600),
        last_announced: 0,
        passed: false,
        name: "stress50_full",
        host: "127.0.0.1".into(),
        port: 43594,
    };
    let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
    assert_eq!(err, "live stress50_full: 1/50 up after 600s");
}

#[test]
fn live_stress_tick_counts_full_clients_up() {
    let mut live = stress_at(Instant::now());
    // Every member is a full Client: "up" requires scene 2, so loading
    // slots do not count toward the 50.
    let mut rows = vec![st("s00", true, 2)];
    for i in 1..50 {
        rows.push(st(&format!("s{i:02}"), true, 1));
    }
    assert_eq!(live_stress_tick(&mut live, &rows), None);
    assert!(!live.passed, "49 loading Clients are not 50 up");
    assert_eq!(live.last_announced, 1);
    for r in rows.iter_mut() {
        r.scene_state = 2;
    }
    assert_eq!(live_stress_tick(&mut live, &rows), None);
    assert!(live.passed, "50 scene-2 Clients pass");
    assert_eq!(live.last_announced, 50);
}

#[test]
fn panel_heading_toggle_hides_status_section() {
    use crate::ui_state::{panel_section_visible, set_panel_section_visible, PanelUiState};
    let mut ui = PanelUiState::default();
    assert!(panel_section_visible(&ui, "status"));
    set_panel_section_visible(&mut ui, "status", false);
    assert!(!panel_section_visible(&ui, "status"));
}

#[test]
fn parameters_and_script_prefs_share_show_parameters_rail() {
    use crate::ui_state::{panel_section_visible, set_panel_section_visible, PanelUiState};
    let mut ui = PanelUiState::default();
    assert!(!ui.show_parameters_rail);
    set_panel_section_visible(&mut ui, "parameters", true);
    assert!(ui.show_parameters_rail);
    assert!(panel_section_visible(&ui, "parameters"));
    ui.show_parameters_rail = false;
    assert!(!panel_section_visible(&ui, "parameters"));
}

/// File-loaded cards have empty description/tags. A trailing
/// SetCursorScreenPos after the badge used to EndChild past CursorMaxPos
/// and abort ErrorCheckUsingSetCursorPosToExtendParentBoundaries
/// (panel-play SIGABRT on Browse, window `##scard-File-trade_bot`).
#[test]
fn browse_file_card_without_desc_does_not_assert_on_endchild() {
    let _guard = crate::test_support::imgui_context_guard();
    let iso = script::IsolatedEnv::enter("browse-scard-assert");
    let path = iso.dir.join("trade_bot.js");
    std::fs::write(
        &path,
        "export default class TradeBot extends LoopingBot { loop() {} }\n",
    )
    .unwrap();
    let mut s = crate::session::Session::new();
    s.scripts.js =
        script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.load_js(&path);
    assert_eq!(s.error, None, "load: {:?}", s.error);
    assert_eq!(
        s.scripts.js.cards()[0].description,
        "",
        "File cards have no registry description — this is the abort path"
    );
    s.script_browse_open = true;
    let mut ctx = dear_imgui_rs::Context::create();
    // AUTO_RESIZE_Y measures on frame 1 and applies on frame 2.
    for _ in 0..3 {
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            super::browse_window(ui, &mut s);
        }
        ctx.render();
    }
}

fn chooser_frame(ctx: &mut dear_imgui_rs::Context, session: &mut crate::session::Session) {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    {
        let ui = ctx.frame();
        super::chooser_window(ui, session, None);
    }
    ctx.render();
}

/// A profile editor text field is focused and being typed in when the
/// operator opens another profile: the switch waits for the Discard / Keep
/// prompt, and after discarding the next keystroke must not write the
/// previous profile's text into the new one's field.
#[test]
fn switching_the_edit_target_resets_a_focused_text_field() {
    let _guard = crate::test_support::imgui_context_guard();
    let dir = TestDir::new("chooser-switch-focused");
    let mut session = crate::session::Session::new();
    let mut vault = vault::Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    for (name, pass, uid) in [("Hans", "hpass", 43), ("aindniK", "kpass", 42)] {
        vault
            .upsert(vault::Profile {
                username: name.into(),
                password: pass.into(),
                uid,
                settings: vault::ProfileSettings::default(),
            })
            .unwrap();
    }
    session.core.set_vault(Some(vault));
    session.begin_edit_profile(Some("Hans"));
    let mut ctx = dear_imgui_rs::Context::create();
    chooser_frame(&mut ctx, &mut session);
    chooser_frame(&mut ctx, &mut session);
    // Tab into the form until one of its text fields is being edited.
    for _ in 0..40 {
        if ctx.io().want_text_input() {
            break;
        }
        ctx.io_mut().add_key_event(dear_imgui_rs::Key::Tab, true);
        chooser_frame(&mut ctx, &mut session);
        ctx.io_mut().add_key_event(dear_imgui_rs::Key::Tab, false);
        chooser_frame(&mut ctx, &mut session);
    }
    ctx.io_mut().add_input_character('X');
    chooser_frame(&mut ctx, &mut session);
    assert!(
        session.cred_user.contains('X') || session.cred_pass.contains('X'),
        "typing reaches a credentials field"
    );

    // The dirty draft holds the switch for the prompt.
    session.begin_edit_profile(Some("aindniK"));
    assert_eq!(
        session
            .pending_edit_switch
            .as_ref()
            .map(|switch| switch.target.as_str()),
        Some("aindniK")
    );
    assert!(
        session.cred_user.contains('X') || session.cred_pass.contains('X'),
        "the unconfirmed switch rewrites nothing"
    );
    chooser_frame(&mut ctx, &mut session);
    session.confirm_pending_edit_switch();
    chooser_frame(&mut ctx, &mut session);
    ctx.io_mut().add_input_character('Y');
    chooser_frame(&mut ctx, &mut session);
    chooser_frame(&mut ctx, &mut session);
    assert_eq!(
        (session.cred_user.as_str(), session.cred_pass.as_str()),
        ("aindniK", "kpass")
    );
}

/// Every text item one ImGui frame drew, read back from ImGui's own text
/// log (`LogToClipboard` finishes into this clipboard backend): one line
/// per row, same-row items joined by a space, buttons as `[ label ]`.
#[derive(Clone, Default)]
struct DrawnText(std::rc::Rc<std::cell::RefCell<String>>);

impl dear_imgui_rs::ClipboardBackend for DrawnText {
    fn get(&mut self) -> Option<String> {
        None
    }

    fn set(&mut self, value: &str) {
        value.clone_into(&mut self.0.borrow_mut());
    }
}
fn draw_status_detail(detail: &frontend_core::SlotDetail, walk: &str, mem: &str) -> String {
    let mut ctx = dear_imgui_rs::Context::create();
    let drawn = DrawnText::default();
    ctx.set_clipboard_backend(drawn.clone());
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([500.0, 400.0], 1.0 / 60.0).renderer_has_textures(),
    );
    {
        let ui = ctx.frame();
        ui.log_to_clipboard(0u32);
        ui.window("status")
            .build(|| super::status_detail_rows(ui, detail, walk, mem));
        ui.log_finish();
    }
    ctx.render();
    drawn.0.take()
}

#[test]
fn memory_notice_gets_its_own_readable_popup_row() {
    let _guard = crate::test_support::imgui_context_guard();
    let dir = TestDir::new("memory-popup-layout");
    let mut vault = vault::Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    vault
        .upsert(vault::Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 1,
            settings: vault::ProfileSettings::default(),
        })
        .unwrap();
    let play = host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let mut session = crate::session::Session::new();
    session.core.set_spawn_workers(false);
    session.core.start(vault, play);
    assert!(session.load("alice"));
    session.select("alice");
    session
        .core
        .play()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(host_play::SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..host_play::SlotStatus::default()
        });
    session.core.poll();

    let mut ctx = dear_imgui_rs::Context::create();
    let drawn = DrawnText::default();
    ctx.set_clipboard_backend(drawn.clone());
    let mut offer_rect = None;
    for frame in 0..8 {
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([500.0, 400.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            ui.log_to_clipboard(0u32);
            ui.window("memory-layout")
                .size([330.0, 350.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    if frame == 0 {
                        ui.open_popup(super::MEM_POPUP);
                    }
                    if frame == 2 {
                        ui.with_bound_context(|| {
                            use dear_imgui_rs::sys;
                            // SAFETY: the bound context owns the popup drawn
                            // by the previous frame; activate its real mode button.
                            unsafe {
                                let parent = sys::igFindWindowByName(c"memory-layout".as_ptr());
                                let popup_id = sys::igGetIDWithSeed_Str(
                                    c"mem-pick".as_ptr(),
                                    std::ptr::null(),
                                    (*parent).ID,
                                );
                                let popup =
                                    std::ffi::CString::new(format!("##Popup_{popup_id:08x}"))
                                        .unwrap();
                                let window = sys::igFindWindowByName(popup.as_ptr());
                                let id = sys::igGetIDWithSeed_Str(
                                    c"highmem".as_ptr(),
                                    std::ptr::null(),
                                    (*window).ID,
                                );
                                sys::igActivateItemByID(id);
                            }
                        });
                    }
                    super::mem_popup(ui, &mut session);
                    if frame >= 5 {
                        offer_rect = Some(ui.with_bound_context(|| {
                            use dear_imgui_rs::sys;
                            // SAFETY: this frame owns the bound context and the
                            // parent window is currently drawing.
                            let popup_id = unsafe {
                                let parent = sys::igFindWindowByName(c"memory-layout".as_ptr());
                                assert!(!parent.is_null());
                                sys::igGetIDWithSeed_Str(
                                    c"mem-pick".as_ptr(),
                                    std::ptr::null(),
                                    (*parent).ID,
                                )
                            };
                            let popup =
                                std::ffi::CString::new(format!("##Popup_{popup_id:08x}")).unwrap();
                            // SAFETY: this frame owns the bound context, and the popup
                            // has just drawn; ImGui keeps its window alive.
                            unsafe {
                                let window = sys::igFindWindowByName(popup.as_ptr());
                                assert!(!window.is_null());
                                let id = sys::igGetIDWithSeed_Str(
                                    c"Relog now".as_ptr(),
                                    std::ptr::null(),
                                    (*window).ID,
                                );
                                sys::igSetFocusID(id, window);
                                focused_item_rect(window)
                            }
                        }));
                    }
                });
            ui.log_finish();
        }
        ctx.render();
    }
    let text = drawn.0.take();
    assert!(
        text.contains("Relog now"),
        "the real in-popup toggle must offer Relog now: {text:?}"
    );
    let (offer, clip) = offer_rect.unwrap();
    assert!(
        offer[0][0] >= clip[0][0] && offer[0][1] >= clip[0][1]
            && offer[1][0] <= clip[1][0] && offer[1][1] <= clip[1][1],
        "Relog now must be visible without scrolling after a logged-in toggle: offer={offer:?}, clip={clip:?}"
    );

    // Closing the picker must not hide the offer in the ordinary bot status.
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([500.0, 400.0], 1.0 / 60.0).renderer_has_textures(),
    );
    {
        let ui = ctx.frame();
        ui.log_to_clipboard(0u32);
        ui.window("memory-status")
            .build(|| super::status_section(ui, &mut session));
        ui.log_finish();
    }
    ctx.render();
    let text = drawn.0.take();
    assert!(
        text.contains("Relog now"),
        "the pending-mode offer must survive leaving the picker: {text:?}"
    );
}

/// What one frame of the Profiles window showed.
struct Shown {
    text: String,
    colours: Vec<u32>,
}

impl Shown {
    fn has(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }

    /// The `n` lines drawn right above the Save button.
    fn above_save(&self, n: usize) -> Vec<&str> {
        let lines: Vec<&str> = self.text.lines().collect();
        let save = lines
            .iter()
            .position(|line| line.contains("[ Save ]"))
            .unwrap_or_else(|| panic!("Save is drawn: {}", self.text));
        lines[save.saturating_sub(n)..save].to_vec()
    }

    /// How many vertices drew in `colour` (ImGui packs RGBA little-endian).
    /// Text outside the window's visible area draws none.
    fn in_colour(&self, colour: [f32; 4]) -> usize {
        let packed = u32::from_le_bytes(colour.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        self.colours.iter().filter(|&&c| c == packed).count()
    }
}

/// Where a Profiles item's id lives.
#[derive(Clone, Copy)]
enum At {
    /// The window's own scope (Close).
    Window,
    /// The edit form's per-target scope (Save, Cancel).
    Form,
    /// The profile list (a row's name, its Edit).
    List,
    /// The Discard / Keep editing prompt.
    SwitchPrompt,
}

/// The Profiles item `label`, whose id lives `at` (`form` is the edit
/// form's id scope): the window that lays it out, and its id. Call with the
/// frame's context bound.
fn profiles_item(
    at: At,
    form: usize,
    label: &str,
) -> (
    *mut dear_imgui_rs::sys::ImGuiWindow,
    dear_imgui_rs::sys::ImGuiID,
) {
    use dear_imgui_rs::sys;
    use std::ffi::CString;

    // SAFETY: the caller binds the frame's context; every name is a
    // NUL-terminated copy that outlives its call, and a window is read only
    // after the lookup found it.
    unsafe {
        let window = |name: &str| {
            let name = CString::new(name).unwrap();
            let window = sys::igFindWindowByName(name.as_ptr());
            assert!(!window.is_null(), "{name:?} was drawn");
            window
        };
        let id = |label: &str, seed: sys::ImGuiID| {
            let label = CString::new(label).unwrap();
            sys::igGetIDWithSeed_Str(label.as_ptr(), std::ptr::null(), seed)
        };
        let profiles = window("Profiles");
        let profiles_id = (*profiles).ID;
        let (window, seed) = match at {
            At::Window => (profiles, profiles_id),
            At::Form => (profiles, sys::igGetIDWithSeed_Int(form as i32, profiles_id)),
            At::List => {
                let list = window(&format!(
                    "Profiles/##profiles-list_{:08X}",
                    id("##profiles-list", profiles_id)
                ));
                (list, (*list).ID)
            }
            At::SwitchPrompt => {
                let prompt = window(&format!(
                    "##Popup_{:08x}",
                    id(super::PROFILE_EDIT_SWITCH_POPUP, profiles_id)
                ));
                (prompt, (*prompt).ID)
            }
        };
        (window, id(label, seed))
    }
}

/// Queue ImGui's own activation of the Profiles item `label`, whose id lives
/// `at` (`form` is the edit form's id scope): it is pressed on the next
/// frame exactly as a click presses it, and never while it is disabled.
/// Call with the frame's context bound.
fn activate_profiles_item(at: At, form: usize, label: &str) {
    let (_, id) = profiles_item(at, form, label);
    // SAFETY: the caller binds the frame's context.
    unsafe { dear_imgui_rs::sys::igActivateItemByID(id) };
}

/// Give the Profiles item `label` ImGui's keyboard focus, so laying it out
/// this frame records where it went. Returns the window to read that back
/// from with [`focused_item_rect`]. Call with the frame's context bound.
fn focus_profiles_item(at: At, form: usize, label: &str) -> *mut dear_imgui_rs::sys::ImGuiWindow {
    let (window, id) = profiles_item(at, form, label);
    // SAFETY: the caller binds the frame's context and `window` is the live
    // window the lookup found.
    unsafe { dear_imgui_rs::sys::igSetFocusID(id, window) };
    window
}

/// The focused item's laid-out rectangle and the visible part of `window`
/// holding it, both as `[min, max]` on screen. Call with the frame's
/// context bound, after the window drew.
fn focused_item_rect(
    window: *mut dear_imgui_rs::sys::ImGuiWindow,
) -> ([[f32; 2]; 2], [[f32; 2]; 2]) {
    use dear_imgui_rs::sys;

    let corners = |r: sys::ImRect_c| [[r.Min.x, r.Min.y], [r.Max.x, r.Max.y]];
    // SAFETY: the caller binds the frame's context; `window` came from
    // `focus_profiles_item` this frame and ImGui keeps windows alive.
    unsafe {
        (
            corners(sys::igWindowRectRelToAbs(window, (*window).NavRectRel[0])),
            corners((*window).InnerClipRect),
        )
    }
}

fn rect_center(rect: [[f32; 2]; 2]) -> [f32; 2] {
    [
        (rect[0][0] + rect[1][0]) * 0.5,
        (rect[0][1] + rect[1][1]) * 0.5,
    ]
}

/// Pin the Profiles window to a chosen dock geometry. Call with the frame's
/// context bound.
fn pin_profiles_geometry_at(pos: [f32; 2], size: [f32; 2]) {
    use dear_imgui_rs::sys;

    let always = sys::ImGuiCond_Always;
    // SAFETY: the caller binds the frame's context; the name is a static
    // NUL-terminated string, and ImGui ignores a window it has not created.
    unsafe {
        sys::igSetWindowPos_Str(
            c"Profiles".as_ptr(),
            sys::ImVec2_c {
                x: pos[0],
                y: pos[1],
            },
            always,
        );
        sys::igSetWindowSize_Str(
            c"Profiles".as_ptr(),
            sys::ImVec2_c {
                x: size[0],
                y: size[1],
            },
            always,
        );
    }
}

/// The laid-out `(pos, size)` of the Profiles popup `popup` (the unsaved-
/// edits or the delete prompt). Call with the frame's context bound, after
/// the Profiles window drew.
fn profiles_popup_rect(popup: &str) -> ([f32; 2], [f32; 2]) {
    use dear_imgui_rs::sys;
    use std::ffi::CString;

    // SAFETY: the caller binds the frame's context; every name is a
    // NUL-terminated copy that outlives its call, and a window is read only
    // after the lookup found it.
    unsafe {
        let profiles = sys::igFindWindowByName(c"Profiles".as_ptr());
        assert!(!profiles.is_null(), "Profiles was drawn");
        let label = CString::new(popup).unwrap();
        let id = sys::igGetIDWithSeed_Str(label.as_ptr(), std::ptr::null(), (*profiles).ID);
        let name = CString::new(format!("##Popup_{id:08x}")).unwrap();
        let window = sys::igFindWindowByName(name.as_ptr());
        assert!(!window.is_null(), "{popup:?} was drawn");
        (
            [(*window).Pos.x, (*window).Pos.y],
            [(*window).Size.x, (*window).Size.y],
        )
    }
}

fn switch_prompt_width() -> f32 {
    profiles_popup_rect(super::PROFILE_EDIT_SWITCH_POPUP).1[0]
}

/// The Profiles window over a real vault, driven through real ImGui frames
/// the way the app runs them (the session pump, then the window) at the
/// default docked tab geometry: what it showed is read back from ImGui's
/// text log and vertex colours, and clicks go through ImGui's own item
/// activation.
struct ProfilesUi {
    ctx: dear_imgui_rs::Context,
    drawn: DrawnText,
    session: crate::session::Session,
    dir: TestDir,
    display_size: [f32; 2],
    window_pos: [f32; 2],
    window_size: [f32; 2],
}

impl ProfilesUi {
    /// Profiles open over a fresh vault of `(username, password, uid)`.
    fn new(label: &str, profiles: &[(&str, &str, i32)]) -> Self {
        Self::with_geometry(
            label,
            profiles,
            [900.0, 700.0],
            [0.0, 0.0],
            [PANEL_WIDTH, BASE_WINDOW_H],
        )
    }

    fn with_geometry(
        label: &str,
        profiles: &[(&str, &str, i32)],
        display_size: [f32; 2],
        window_pos: [f32; 2],
        window_size: [f32; 2],
    ) -> Self {
        let dir = TestDir::new(label);
        let mut vault = vault::Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
        for &(username, password, uid) in profiles {
            vault
                .upsert(vault::Profile {
                    username: username.into(),
                    password: password.into(),
                    uid,
                    settings: vault::ProfileSettings::default(),
                })
                .unwrap();
        }
        let mut session = crate::session::Session::new();
        session.core.set_vault(Some(vault));
        session.wall.chooser_open = true;
        let mut ctx = dear_imgui_rs::Context::create();
        let drawn = DrawnText::default();
        ctx.set_clipboard_backend(drawn.clone());
        let mut ui = Self {
            ctx,
            drawn,
            session,
            dir,
            display_size,
            window_pos,
            window_size,
        };
        ui.frame();
        ui
    }

    fn frame(&mut self) -> Shown {
        self.draw(None)
    }

    /// Click `label` (its id lives `at`): ImGui presses it on the next
    /// frame; the frame after that shows the result.
    fn click(&mut self, at: At, label: &str) -> Shown {
        self.draw(Some((at, label)));
        self.frame();
        self.frame()
    }

    fn draw(&mut self, click: Option<(At, &str)>) -> Shown {
        self.session.pump_status();
        self.ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new(self.display_size, 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let form = self.session.chooser_form;
        let window_pos = self.window_pos;
        let window_size = self.window_size;
        {
            let ui = self.ctx.frame();
            ui.with_bound_context(|| pin_profiles_geometry_at(window_pos, window_size));
            if let Some((at, label)) = click {
                ui.with_bound_context(|| activate_profiles_item(at, form, label));
            }
            ui.log_to_clipboard(0u32);
            super::chooser_window(ui, &mut self.session, None);
            ui.log_finish();
        }
        let colours = self
            .ctx
            .render()
            .draw_lists()
            .flat_map(|list| list.vtx_buffer().iter().map(|vertex| vertex.col))
            .collect();
        Shown {
            text: self.drawn.0.take(),
            colours,
        }
    }

    /// One frame; returns the unsaved-edits prompt's laid-out width and
    /// the width its message needs on one line.
    fn switch_prompt_width(&mut self) -> (f32, f32) {
        self.session.pump_status();
        self.ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new(self.display_size, 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let window_pos = self.window_pos;
        let window_size = self.window_size;
        let width = {
            let ui = self.ctx.frame();
            ui.with_bound_context(|| pin_profiles_geometry_at(window_pos, window_size));
            super::chooser_window(ui, &mut self.session, None);
            let prompt = &self.session.pending_edit_switch.as_ref().unwrap().prompt;
            (
                ui.with_bound_context(switch_prompt_width),
                ui.current_font()
                    .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, prompt)[0],
            )
        };
        self.ctx.render();
        width
    }

    /// One frame; returns the Profiles popup `popup`'s laid-out
    /// `(pos, size)`.
    fn popup_rect(&mut self, popup: &str) -> ([f32; 2], [f32; 2]) {
        self.session.pump_status();
        self.ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new(self.display_size, 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let window_pos = self.window_pos;
        let window_size = self.window_size;
        let rect = {
            let ui = self.ctx.frame();
            ui.with_bound_context(|| pin_profiles_geometry_at(window_pos, window_size));
            super::chooser_window(ui, &mut self.session, None);
            ui.with_bound_context(|| profiles_popup_rect(popup))
        };
        self.ctx.render();
        rect
    }

    /// One frame; returns where the Profiles item `label` (its id lives
    /// `at`) was laid out on screen and the visible part of the window
    /// holding it, both as `[min, max]`.
    fn item_rect(&mut self, at: At, label: &str) -> ([[f32; 2]; 2], [[f32; 2]; 2]) {
        self.session.pump_status();
        self.ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new(self.display_size, 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let form = self.session.chooser_form;
        let window_pos = self.window_pos;
        let window_size = self.window_size;
        let rects = {
            let ui = self.ctx.frame();
            ui.with_bound_context(|| pin_profiles_geometry_at(window_pos, window_size));
            let window = ui.with_bound_context(|| focus_profiles_item(at, form, label));
            super::chooser_window(ui, &mut self.session, None);
            ui.with_bound_context(|| focused_item_rect(window))
        };
        self.ctx.render();
        rects
    }

    /// A left click with the pointer at `pos`, as the mouse makes it; the
    /// last frame shows the result.
    fn click_at(&mut self, pos: [f32; 2]) -> Shown {
        self.ctx.io_mut().add_mouse_pos_event(pos);
        self.frame();
        self.ctx
            .io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        self.frame();
        self.ctx
            .io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
        self.frame();
        self.frame()
    }

    /// Type `c` into the edit form, as the keyboard would: Tab to one of
    /// its text fields, then the character.
    fn type_char(&mut self, c: char) -> Shown {
        for _ in 0..40 {
            if self.ctx.io().want_text_input() {
                break;
            }
            self.ctx
                .io_mut()
                .add_key_event(dear_imgui_rs::Key::Tab, true);
            self.frame();
            self.ctx
                .io_mut()
                .add_key_event(dear_imgui_rs::Key::Tab, false);
            self.frame();
        }
        assert!(
            self.ctx.io().want_text_input(),
            "a form text field has the keyboard"
        );
        self.ctx.io_mut().add_input_character(c);
        self.frame();
        self.frame()
    }

    /// The main panel's banner line (hidden behind Profiles in the default
    /// layout) as it draws now. Its callers are the unix-only tests.
    #[cfg(unix)]
    fn banner(&mut self) -> String {
        self.ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = self.ctx.frame();
            ui.log_to_clipboard(0u32);
            ui.window("banner")
                .build(|| super::banner(ui, &self.session, None));
            ui.log_finish();
        }
        self.ctx.render();
        self.drawn.0.take()
    }

    /// Let every queued profile write finish; the next frame's pump settles
    /// it.
    fn finish_writes(&mut self) {
        self.session.core.flush_writes();
    }

    /// The vault on disk as `(username, uid, password)`, by username.
    fn disk(&self) -> Vec<(String, i32, String)> {
        let vault = vault::Vault::unlock(&self.dir.join("vault"), "test-passphrase-01").unwrap();
        let mut rows: Vec<_> = vault
            .profiles()
            .map(|p| (p.username.clone(), p.uid, p.password.to_string()))
            .collect();
        rows.sort();
        rows
    }

    #[cfg(unix)]
    fn writable(&self, writable: bool) {
        use std::os::unix::fs::PermissionsExt;

        let mode = if writable { 0o700 } else { 0o500 };
        std::fs::set_permissions(&*self.dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn row(username: &str, uid: i32, password: &str) -> (String, i32, String) {
    (username.into(), uid, password.into())
}

/// A refused Save (renaming onto another profile's username) shows its
/// reason and "nothing was saved." right above Save, the reason in the error
/// colour, with Profiles still open on the form. Nothing is written, and
/// typing in the form clears it.
#[test]
fn a_refused_save_shows_above_save_until_the_form_is_edited() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-refused",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_user = "bob".into();
    let unsaved = ui.frame();

    let refused = ui.click(At::Form, "Save");
    assert!(refused.has("Editing alice"), "{}", refused.text);
    assert_eq!(
        refused.above_save(2),
        [
            "credentials: a profile named bob already exists",
            frontend_core::NOTHING_SAVED,
        ],
        "{}",
        refused.text
    );
    assert!(
        refused.in_colour(super::ERROR) > unsaved.in_colour(super::ERROR),
        "the reason draws in the error colour"
    );
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "nothing was written"
    );

    let typed = ui.type_char('x');
    assert!(typed.has("Editing alice"), "{}", typed.text);
    assert!(
        !typed.has("already exists") && !typed.has(frontend_core::NOTHING_SAVED),
        "typing clears the refusal: {}",
        typed.text
    );
}

/// `Saved <name>.` shows right above Save, in green, only once the write is
/// durable, and on the form the Save came from; Profiles stays open.
#[test]
fn saved_shows_only_once_the_write_is_durable() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-saved", &[("alice", "apass", 42)]);
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();

    let queued = ui.click(At::Form, "Save");
    assert!(queued.has("Editing alice"), "{}", queued.text);
    assert!(
        !queued.has("Saved"),
        "no Saved while the write is queued: {}",
        queued.text
    );
    assert_eq!(queued.in_colour(super::GREEN), 0);

    drop(held);
    ui.finish_writes();
    let saved = ui.frame();
    assert!(saved.has("Editing alice"), "{}", saved.text);
    assert_eq!(saved.above_save(1), ["Saved alice."], "{}", saved.text);
    assert!(saved.in_colour(super::GREEN) > 0, "Saved draws in green");
    assert_eq!(ui.disk(), [row("alice", 42, "newpass")]);
}

/// Opening another profile's form while a save is still being written asks
/// first: the save can still fail, and the draft is all that is left of it.
/// Discard opens the other form; the save lands, but its `Saved` never
/// shows there.
#[test]
fn a_save_completing_after_a_switch_shows_nothing_on_the_new_form() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-switch-pending",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    let asked = ui.click(At::List, "Edit##edit-bob");
    assert!(
        asked.has("Editing alice") && asked.has("[ Discard ]") && asked.has("Saving alice"),
        "an unsettled save asks before the form is left: {}",
        asked.text
    );
    let switched = ui.click(At::SwitchPrompt, "Discard");
    assert!(switched.has("Editing bob"), "{}", switched.text);

    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    assert!(shown.has("Editing bob"), "{}", shown.text);
    assert!(
        !shown.has("Saved"),
        "alice's save never shows on bob's form: {}",
        shown.text
    );
    assert_eq!(shown.in_colour(super::GREEN), 0);
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "newpass"), row("bob", 43, "bpass")],
        "the save itself landed"
    );
}

/// Closing Profiles while a save is still being written asks first, from
/// Close and from the window's ✕ alike; Keep editing leaves the form and its
/// draft alone. Discard closes; opening the same profile again never shows
/// the earlier save's `Saved`.
#[test]
fn a_save_completing_after_close_shows_nothing_on_a_reopened_form() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-close-pending", &[("alice", "apass", 42)]);
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");

    let x = ui.click(At::Window, "#CLOSE");
    assert!(
        x.has("Editing alice") && x.has("[ Discard ]") && x.has("close Profiles"),
        "the ✕ asks while the save is unsettled: {}",
        x.text
    );
    let kept = ui.click(At::SwitchPrompt, "Keep editing");
    assert!(kept.has("Editing alice"), "{}", kept.text);
    assert_eq!(ui.session.cred_pass, "newpass", "keeping drops nothing");

    let asked = ui.click(At::Window, "Close");
    assert!(
        asked.has("Editing alice") && asked.has("[ Discard ]"),
        "Close asks too: {}",
        asked.text
    );
    let closed = ui.click(At::SwitchPrompt, "Discard");
    assert!(closed.text.trim().is_empty(), "Discard shut Profiles");

    // The panel's Profiles button, then Edit on the same profile.
    ui.session.wall.chooser_open = true;
    ui.frame();
    let reopened = ui.click(At::List, "Edit##edit-alice");
    assert!(reopened.has("Editing alice"), "{}", reopened.text);

    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert!(
        !shown.has("Saved"),
        "the closed form's save never shows on the reopened one: {}",
        shown.text
    );
    assert_eq!(ui.disk(), [row("alice", 42, "newpass")]);
}

/// With no save unsettled, Close and the ✕ close Profiles at once, as
/// before: the protection lasts only until the write settles.
#[test]
fn close_needs_no_prompt_once_the_save_settled() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-close-settled", &[("alice", "apass", 42)]);
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    ui.click(At::Form, "Save");
    ui.finish_writes();
    let saved = ui.frame();
    assert_eq!(saved.above_save(1), ["Saved alice."], "{}", saved.text);

    let closed = ui.click(At::Window, "#CLOSE");
    assert!(closed.text.trim().is_empty(), "the ✕ shut Profiles");
}

/// A save whose write fails after it was accepted shows in the form it came
/// from, as well as on the banner: the form keeps its draft and its target,
/// and Save retries.
#[test]
#[cfg(unix)]
fn a_late_write_failure_shows_in_the_form_it_came_from() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-late-failure",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    ui.click(At::List, "Edit##edit-bob");
    ui.click(At::SwitchPrompt, "Keep editing");

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert!(shown.has("credentials:"), "the reason: {}", shown.text);
    assert_eq!(
        shown.above_save(1),
        [frontend_core::NOTHING_SAVED],
        "{}",
        shown.text
    );
    assert!(shown.in_colour(super::ERROR) > 0, "in the error colour");
    assert_eq!(ui.session.cred_pass, "newpass", "the draft is kept");
    assert!(
        ui.banner().contains("credentials:"),
        "and the banner reports it too"
    );
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "nothing was saved"
    );

    // The form kept its draft and its target, so Save retries the write.
    ui.click(At::Form, "Save");
    ui.finish_writes();
    let saved = ui.frame();
    assert_eq!(saved.above_save(1), ["Saved alice."], "{}", saved.text);
    assert!(
        !saved.has("credentials:"),
        "the earlier failure is gone: {}",
        saved.text
    );
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "newpass"), row("bob", 43, "bpass")],
        "the retry landed"
    );
}

/// A failure that arrives after the operator discarded the form it came from
/// shows in no form: the banner reports it, and the form showing now stays
/// clean.
#[test]
#[cfg(unix)]
fn a_late_failure_after_discarding_the_form_reaches_only_the_banner() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-late-discarded",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    ui.click(At::List, "Edit##edit-bob");
    ui.click(At::SwitchPrompt, "Discard");

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing bob"), "{}", shown.text);
    assert!(
        !shown.has("credentials:") && !shown.has(frontend_core::NOTHING_SAVED),
        "the failure never shows in bob's form: {}",
        shown.text
    );
    let banner = ui.banner();
    assert!(
        banner.contains("credentials:"),
        "the banner reports it: {banner:?}"
    );
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "nothing was saved"
    );
}

/// A failure after Profiles was closed and the same profile reopened shows
/// only on the banner, not in the new form of the same target.
#[test]
#[cfg(unix)]
fn a_late_failure_after_close_and_reopen_reaches_only_the_banner() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-late-reopened", &[("alice", "apass", 42)]);
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    ui.click(At::Window, "Close");
    ui.click(At::SwitchPrompt, "Discard");
    ui.session.wall.chooser_open = true;
    ui.frame();
    ui.click(At::List, "Edit##edit-alice");

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert!(
        !shown.has("credentials:") && !shown.has(frontend_core::NOTHING_SAVED),
        "the earlier form's failure never shows on the reopened one: {}",
        shown.text
    );
    assert!(ui.banner().contains("credentials:"));
    assert_eq!(ui.disk(), [row("alice", 42, "apass")]);
}

/// Deleting the profile whose save is still queued asks first: the commit
/// can still fail, which puts the profile back and would leave the typed
/// draft nowhere. Keep editing deletes nothing; when the save then fails the
/// form shows the failure on its draft, and Save retries.
#[test]
#[cfg(unix)]
fn deleting_the_target_while_its_save_is_pending_keeps_the_draft_on_failure() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-delete-pending",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");

    // The delete confirm's body.
    assert!(!ui.session.delete_profile("alice"), "the delete waits");
    let asked = ui.frame();
    assert!(
        asked.has("Editing alice")
            && asked.has("[ Discard ]")
            && asked.has("[ Keep editing ]")
            && asked.has("delete alice"),
        "an unsettled save asks before its profile is deleted: {}",
        asked.text
    );
    let kept = ui.click(At::SwitchPrompt, "Keep editing");
    assert!(kept.has("Editing alice"), "{}", kept.text);
    assert!(!kept.has("[ Discard ]"), "the prompt closed: {}", kept.text);
    assert_eq!(ui.session.cred_pass, "newpass", "keeping drops nothing");

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert!(shown.has("credentials:"), "the reason: {}", shown.text);
    assert_eq!(
        shown.above_save(1),
        [frontend_core::NOTHING_SAVED],
        "{}",
        shown.text
    );
    assert_eq!(ui.session.cred_pass, "newpass", "the draft is still there");
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "nothing was deleted or saved"
    );

    ui.click(At::Form, "Save");
    ui.finish_writes();
    let saved = ui.frame();
    assert_eq!(saved.above_save(1), ["Saved alice."], "{}", saved.text);
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "newpass"), row("bob", 43, "bpass")],
        "the retry landed"
    );
}

/// Discard on that prompt is the operator's choice to lose the draft: the
/// form and the profile go, and when the commit then fails the profile is
/// back as it is on disk with only the banner reporting it.
#[test]
#[cfg(unix)]
fn discarding_a_pending_save_to_delete_its_profile_reports_only_on_the_banner() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-delete-discard",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    assert!(!ui.session.delete_profile("alice"));
    ui.frame();
    let deleted = ui.click(At::SwitchPrompt, "Discard");
    assert!(!deleted.has("Editing"), "the form went: {}", deleted.text);

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(
        !shown.has("credentials:") && !shown.has(frontend_core::NOTHING_SAVED),
        "{}",
        shown.text
    );
    assert!(ui.banner().contains("chooser:"), "the delete's failure");
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "neither the save nor the delete landed"
    );
    assert!(
        ui.session.core.vault().unwrap().get("alice").is_some(),
        "alice is back in the list"
    );
}

/// Turning MultiBox off closes Profiles and its form, so while the form's
/// save is queued it asks first and MultiBox stays on; Keep editing leaves
/// the draft, and a later failure shows on it.
#[test]
#[cfg(unix)]
fn multibox_off_while_a_save_is_pending_asks_and_keeps_the_draft_on_failure() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-multibox-pending", &[("alice", "apass", 42)]);
    ui.session.set_multibox(true);
    ui.session.wall.chooser_open = true;
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");

    assert!(!ui.session.set_multibox(false), "the toggle waits");
    assert!(ui.session.multibox, "MultiBox is still on");
    let asked = ui.frame();
    assert!(
        asked.has("Editing alice")
            && asked.has("[ Discard ]")
            && asked.has("[ Keep editing ]")
            && asked.has("turn MultiBox off"),
        "{}",
        asked.text
    );
    let kept = ui.click(At::SwitchPrompt, "Keep editing");
    assert!(kept.has("Editing alice") && ui.session.multibox);

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert!(shown.has("credentials:"), "the reason: {}", shown.text);
    assert_eq!(ui.session.cred_pass, "newpass", "the draft is kept");
    assert_eq!(ui.disk(), [row("alice", 42, "apass")]);

    // Nothing is pending any more: the toggle applies at once.
    assert!(ui.session.set_multibox(false));
    assert!(!ui.session.multibox);
    assert!(ui.session.chooser_edit.is_none());
}

/// Discard on the MultiBox prompt turns it off, form and all.
#[test]
fn discarding_a_pending_save_turns_multibox_off() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-multibox-discard", &[("alice", "apass", 42)]);
    ui.session.set_multibox(true);
    ui.session.wall.chooser_open = true;
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "newpass".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    assert!(!ui.session.set_multibox(false));
    ui.frame();

    ui.click(At::SwitchPrompt, "Discard");
    assert!(!ui.session.multibox);
    assert!(ui.session.chooser_edit.is_none());
    drop(held);
}

/// A Discard / Keep editing prompt that is already open when the save it
/// waited on settles ends with it: it must not say the save "may fail" over
/// the result, which shows on the form instead (a failure with its reason,
/// a success as `Saved`).
#[test]
#[cfg(unix)]
fn an_open_prompt_ends_when_its_save_settles() {
    for fails in [true, false] {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ui = ProfilesUi::new(
            &format!("profiles-prompt-settles-{fails}"),
            &[("alice", "apass", 42)],
        );
        ui.click(At::List, "Edit##edit-alice");
        ui.session.cred_pass = "newpass".into();
        let gate = ui.session.core.write_gate();
        let held = gate.lock().unwrap();
        ui.click(At::Form, "Save");
        let asked = ui.click(At::Window, "#CLOSE");
        assert!(
            asked.has("[ Discard ]") && asked.has("may fail"),
            "{}",
            asked.text
        );

        ui.writable(!fails);
        drop(held);
        ui.finish_writes();
        let shown = ui.frame();
        ui.writable(true);
        assert!(
            !shown.has("may fail") && !shown.has("[ Discard ]") && !shown.has("[ Keep editing ]"),
            "the prompt is gone once the save settled (fails={fails}): {}",
            shown.text
        );
        assert!(shown.has("Editing alice"), "{}", shown.text);
        if fails {
            assert!(shown.has("credentials:"), "{}", shown.text);
            assert_eq!(ui.session.cred_pass, "newpass", "the draft is kept");
        } else {
            assert_eq!(shown.above_save(1), ["Saved alice."], "{}", shown.text);
        }
        // The ✕ now closes as usual when nothing is pending.
        if !fails {
            let closed = ui.click(At::Window, "#CLOSE");
            assert!(closed.text.trim().is_empty(), "{}", closed.text);
        }
    }
}

/// A second Save while the first is still being written is refused, not
/// queued: when the first then fails, the form shows the failure and the
/// second attempt never wrote anything.
#[test]
#[cfg(unix)]
fn a_refused_second_save_is_not_queued_behind_a_failing_one() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-second-refused",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_user = "carol".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    ui.session.cred_user = "dave".into();
    assert!(
        !ui.session.save_credentials(),
        "the second attempt is refused while the first is unsettled"
    );

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let failed = ui.frame();
    ui.writable(true);
    assert!(failed.has("Editing alice"), "{}", failed.text);
    assert_eq!(
        failed.above_save(1),
        [frontend_core::NOTHING_SAVED],
        "{}",
        failed.text
    );
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")],
        "neither attempt wrote anything"
    );

    // The form still holds the second draft, so Save now writes that one.
    ui.click(At::Form, "Save");
    ui.finish_writes();
    let saved = ui.frame();
    assert_eq!(saved.above_save(1), ["Saved dave."], "{}", saved.text);
    assert_eq!(
        ui.disk(),
        [row("bob", 43, "bpass"), row("dave", 42, "apass")]
    );
}

/// A save a later save of the same profile superseded in the writer's queue
/// (the operator discarded its form and saved from the next one): the
/// commit fails, and the form showing, the later save's, shows the failure.
#[test]
#[cfg(unix)]
fn a_failure_of_a_superseding_save_shows_in_the_form_that_superseded() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-superseded",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "first".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    ui.click(At::Form, "Save");
    ui.click(At::List, "Edit##edit-bob");
    ui.click(At::SwitchPrompt, "Discard");
    // Alice again, from a new form that loads the staged row.
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "second".into();
    ui.click(At::Form, "Save");

    ui.writable(false);
    drop(held);
    ui.finish_writes();
    let shown = ui.frame();
    ui.writable(true);
    assert!(shown.has("Editing alice"), "{}", shown.text);
    assert_eq!(
        shown.above_save(1),
        [frontend_core::NOTHING_SAVED],
        "{}",
        shown.text
    );
    assert_eq!(ui.session.cred_pass, "second", "the draft is kept");
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")]
    );
}

/// A rename whose write fails leaves the form on the old profile, so Save
/// retries the same rename: the profile keeps its uid under the new name
/// and no second profile appears.
#[test]
#[cfg(unix)]
fn a_failed_rename_retries_as_the_same_rename() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-rename-retry",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_user = "carol".into();

    ui.writable(false);
    ui.click(At::Form, "Save");
    ui.finish_writes();
    let failed = ui.frame();
    ui.writable(true);
    assert!(
        failed.has("Editing alice"),
        "the failed rename leaves the form on alice: {}",
        failed.text
    );
    assert!(!failed.has("Saved"), "{}", failed.text);
    assert!(ui.banner().contains("credentials:"));
    assert_eq!(
        ui.disk(),
        [row("alice", 42, "apass"), row("bob", 43, "bpass")]
    );

    ui.click(At::Form, "Save");
    ui.finish_writes();
    let saved = ui.frame();
    assert!(saved.has("Editing carol"), "{}", saved.text);
    assert_eq!(saved.above_save(1), ["Saved carol."], "{}", saved.text);
    assert_eq!(
        ui.disk(),
        [row("bob", 43, "bpass"), row("carol", 42, "apass")],
        "alice renamed in place, no second profile"
    );
}

/// While a rename is being written the form keeps its old target, so Save
/// is disabled until it settles: a second Save (after typing yet another
/// name) is not taken, and no second profile appears.
#[test]
fn save_waits_for_the_forms_save_in_flight() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new("profiles-save-once", &[("alice", "apass", 42)]);
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_user = "carol".into();
    let gate = ui.session.core.write_gate();
    let held = gate.lock().unwrap();
    let queued = ui.click(At::Form, "Save");
    assert!(
        queued.has("Editing alice"),
        "the form follows the rename only once it is durable: {}",
        queued.text
    );
    ui.session.cred_user = "dave".into();
    ui.click(At::Form, "Save");

    drop(held);
    ui.finish_writes();
    let saved = ui.frame();
    assert!(saved.has("Editing carol"), "{}", saved.text);
    assert_eq!(saved.above_save(1), ["Saved carol."], "{}", saved.text);
    assert_eq!(
        ui.disk(),
        [row("carol", 42, "apass")],
        "one rename, no second profile"
    );
}

/// Picking a profile row only moves focus: Profiles stays open with the
/// edit form on its profile and its unsaved draft.
#[test]
fn picking_a_row_keeps_profiles_open_on_the_form() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-pick",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "typed".into();

    let picked = ui.click(At::List, "bob");
    assert_eq!(ui.session.focused_name().as_deref(), Some("bob"));
    assert!(
        picked.has("Editing alice"),
        "Profiles stays open on the form: {}",
        picked.text
    );
    assert_eq!(ui.session.cred_pass, "typed", "the draft is kept");
}

/// Opening another profile over unsaved edits asks first: Keep editing
/// leaves the form and its draft alone, Discard opens the other profile.
#[test]
fn opening_another_profile_over_unsaved_edits_asks_first() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = ProfilesUi::new(
        "profiles-switch-prompt",
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
    );
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "typed".into();

    let asked = ui.click(At::List, "Edit##edit-bob");
    assert!(asked.has("Editing alice"), "{}", asked.text);
    assert!(
        asked.has("[ Discard ]") && asked.has("[ Keep editing ]"),
        "{}",
        asked.text
    );
    // Readable, not a one-character sliver: a short message fits on one
    // line (the 0.1.9.1 RC drew the prompt about 20 px wide).
    let (width, message) = ui.switch_prompt_width();
    assert!(width >= message, "prompt width {width} < message {message}");
    let kept = ui.click(At::SwitchPrompt, "Keep editing");
    assert!(kept.has("Editing alice"), "{}", kept.text);
    assert!(!kept.has("[ Discard ]"), "{}", kept.text);
    assert_eq!(ui.session.cred_pass, "typed", "keeping drops nothing");

    ui.click(At::List, "Edit##edit-bob");
    let discarded = ui.click(At::SwitchPrompt, "Discard");
    assert!(discarded.has("Editing bob"), "{}", discarded.text);
    assert_eq!(ui.session.cred_pass, "bpass", "bob's own row loads");
}

/// A Profiles prompt opened from a control at the display's right edge:
/// ImGui places it inside the display, and it keeps that place and width
/// however long it stays open, so both its buttons stay on screen.
fn assert_prompt_holds_on_screen(ui: &mut ProfilesUi, popup: &str) {
    let display = ui.display_size;
    let rects: Vec<_> = (0..60).map(|_| ui.popup_rect(popup)).collect();
    let (pos, size) = rects[2];
    for (frame, &rect) in rects.iter().enumerate().skip(2) {
        assert_eq!(
            rect,
            (pos, size),
            "{popup:?} at frame {frame} is {rect:?}, was {:?}",
            (pos, size)
        );
    }
    assert!(
        pos[0] >= 0.0
            && pos[1] >= 0.0
            && pos[0] + size[0] <= display[0]
            && pos[1] + size[1] <= display[1],
        "{popup:?} at {pos:?} {size:?} leaves the {display:?} display"
    );
    assert!(
        size[0] <= 2.0 * super::DIALOG_W,
        "{popup:?} stays a dialog, not the window's width: {size:?}"
    );
}

/// Profiles docked flush against the right edge of a 1024×768 display.
fn far_right_profiles(label: &str) -> ProfilesUi {
    ProfilesUi::with_geometry(
        label,
        &[("alice", "apass", 42), ("bob", "bpass", 43)],
        [1024.0, 768.0],
        [1024.0 - PANEL_WIDTH, 0.0],
        [PANEL_WIDTH, 768.0],
    )
}

/// The unsaved-edits prompt keeps one width however long it stays open.
/// Its buttons are sized for the gap between them, and any change to the
/// popup's position or width inside its own frame feeds back into that
/// size: a mismatched gap made it 2 px wider every frame, and moving it
/// left inside the popup made it about 5 px wider every frame, until it
/// covered the window.
#[test]
fn the_leave_prompt_keeps_a_stable_width_so_both_buttons_stay_on_screen() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = far_right_profiles("profiles-prompt-width");
    ui.click(At::List, "Edit##edit-alice");
    ui.session.cred_pass = "typed".into();
    let (edit, _) = ui.item_rect(At::List, "Edit##edit-bob");
    let asked = ui.click_at(rect_center(edit));
    assert!(
        asked.has("[ Discard ]") && asked.has("[ Keep editing ]"),
        "{}",
        asked.text
    );
    assert_prompt_holds_on_screen(&mut ui, super::PROFILE_EDIT_SWITCH_POPUP);
}

/// The delete prompt, opened from a row's ✕ at the display's right edge,
/// holds its width and stays on screen the same way.
#[test]
fn the_delete_prompt_keeps_a_stable_width_so_both_buttons_stay_on_screen() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ui = far_right_profiles("profiles-delete-prompt-width");
    let (delete, _) = ui.item_rect(At::List, "✕##bob");
    let asked = ui.click_at(rect_center(delete));
    assert!(
        asked.has("Remove bob") && asked.has("[ Cancel ]"),
        "{}",
        asked.text
    );
    assert_prompt_holds_on_screen(&mut ui, super::PROFILE_DELETE_POPUP);
}

#[test]
fn unlock_sends_the_passphrase_exactly_as_typed() {
    let mut session = crate::session::Session::new();
    session.pass_scratch.push_str("  spaced pass ");
    super::submit_vault_pass(&mut session, true);
    let sent = session.take_requested_unlock().expect("unlock requested");
    assert_eq!(sent.as_str(), "  spaced pass ");

    // Create still needs something besides spaces.
    session.pass_scratch.push_str("   ");
    super::submit_vault_pass(&mut session, false);
    assert!(session.take_requested_unlock().is_none());
}

#[test]
fn typed_unlock_opens_terminal_and_trimmed_panel_vaults() {
    use crate::session::Session;
    let dir = TestDir::new("vault-typed-unlock");

    // Made in the terminal with the spaces kept: only the typed text opens it.
    let terminal = dir.join("terminal");
    drop(vault::Vault::create(&terminal, " pad ").unwrap());
    assert!(Session::open_vault_typed(&terminal, " pad ").is_ok());

    // Made by the 0.1.9.1 panel, which stored the trimmed text.
    let legacy = dir.join("legacy");
    drop(vault::Vault::create(&legacy, "pad").unwrap());
    assert!(Session::open_vault_typed(&legacy, "  pad ").is_ok());

    assert!(matches!(
        Session::open_vault_typed(&legacy, " nope "),
        Err(vault::VaultError::WrongPassphrase)
    ));
}

/// Content width of the docked Nav config window at scale 1: the width the
/// review's screenshot clipped the Danger routing button at.
const NAV_ROUTING_CONTENT_W: f32 = 330.0;
/// Row of the Danger routing drop-down in the Routing group: after the scope
/// note, the three checkboxes, the bank-fetch scope, and the drop-down's label.
const DANGER_ROUTING_COMBO_ROW: usize = 6;

/// One headless frame of the Routing group in a window whose content area is
/// [`NAV_ROUTING_CONTENT_W`] wide. Returns the group as drawn and the content
/// region's left and right edges.
fn nav_routing_frame(
    ctx: &mut dear_imgui_rs::Context,
    nav: &mut crate::nav_settings::NavSettings,
) -> (super::TestRouting, [f32; 2]) {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([800.0, 600.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let ui = ctx.frame();
    let pad = ui.clone_style().window_padding()[0];
    let mut drawn = None;
    ui.window("##nav-routing")
        .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
        .size(
            [NAV_ROUTING_CONTENT_W + 2.0 * pad, 500.0],
            dear_imgui_rs::Condition::Always,
        )
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::NO_SCROLLBAR,
        )
        .build(|| {
            let left = ui.cursor_screen_pos()[0];
            let right = left + ui.content_region_avail()[0];
            drawn = Some((super::draw_test_nav_routing(ui, nav), [left, right]));
        });
    ctx.render();
    drawn.expect("the routing window draws its body")
}

/// [`nav_routing_frame`] with the pointer at `point`, the left button held
/// when `pressed`.
fn nav_routing_pointer_frame(
    ctx: &mut dear_imgui_rs::Context,
    nav: &mut crate::nav_settings::NavSettings,
    point: [f32; 2],
    pressed: bool,
) -> (super::TestRouting, [f32; 2]) {
    let io = ctx.io_mut();
    io.add_mouse_pos_event(point);
    io.add_mouse_button_event(dear_imgui_rs::MouseButton::Left, pressed);
    nav_routing_frame(ctx, nav)
}

/// Centre of an item rect, where a pointer click lands.
fn rect_centre([min_x, min_y, max_x, max_y]: [f32; 4]) -> [f32; 2] {
    [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0]
}

/// At the docked Nav config width every Routing row stays inside the content
/// region at each Danger routing level. The drop-down spans the width, so its
/// level names never clip; the old plain button ran past the edge at "availabl".
#[test]
fn nav_routing_rows_fit_the_docked_width_at_every_danger_level() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    super::amber_style(&mut ctx);
    for level in frontend_core::walk_permissions::DANGER_ROUTING_LEVELS {
        let mut nav = crate::nav_settings::NavSettings::default();
        nav.set_danger_level(level);
        let (routing, [left, right]) = nav_routing_frame(&mut ctx, &mut nav);
        assert!(
            ((right - left) - NAV_ROUTING_CONTENT_W).abs() < 0.5,
            "the test window's content is the docked width, got {}",
            right - left
        );
        // The scope note, three checkboxes, the bank-fetch scope, the drop-down's
        // label and the drop-down, plus the note under a held level and the
        // Always warning.
        let notes = usize::from(frontend_core::walk_permissions::danger_routing_held(level))
            + usize::from(level == frontend_core::DangerLevel::Always);
        assert_eq!(routing.items.len(), 7 + notes, "routing rows at {level:?}");
        for (index, item) in routing.items.iter().enumerate() {
            let [min_x, _, max_x, _] = *item;
            assert!(
                min_x >= left - 0.5 && max_x <= right + 0.5,
                "routing row {index} at {level:?} spans {min_x}..{max_x}, past the content region {left}..{right}"
            );
        }
    }
}

/// Picking each Danger routing entry in the open drop-down stores that level
/// and reports a change, so the Nav config saves the way the old button did.
#[test]
fn nav_danger_routing_drop_down_stores_each_picked_level() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dock_host_context();
    super::amber_style(&mut ctx);
    let levels = frontend_core::walk_permissions::DANGER_ROUTING_LEVELS;
    for (index, target) in levels.into_iter().enumerate() {
        let mut nav = crate::nav_settings::NavSettings::default();
        nav.set_danger_level(levels[(index + 1) % levels.len()]);
        let (routing, _) = nav_routing_frame(&mut ctx, &mut nav);
        let combo = rect_centre(routing.items[DANGER_ROUTING_COMBO_ROW]);

        // Hover the drop-down, then press and release on it to open the popup.
        for pressed in [false, true, false] {
            nav_routing_pointer_frame(&mut ctx, &mut nav, combo, pressed);
        }
        // The popup lays out its entries a frame or two after it opens.
        let mut choices = Vec::new();
        for _ in 0..8 {
            let (routing, _) = nav_routing_pointer_frame(&mut ctx, &mut nav, combo, false);
            choices = routing.choices;
            if choices.len() == levels.len() {
                break;
            }
        }
        assert_eq!(
            choices.len(),
            levels.len(),
            "the open drop-down lists every level"
        );

        // Move onto the entry, then press and release on it.
        let entry = rect_centre(choices[index]);
        let mut changed = false;
        for pressed in [false, false, true, false] {
            let (routing, _) = nav_routing_pointer_frame(&mut ctx, &mut nav, entry, pressed);
            changed |= routing.changed;
        }
        assert!(changed, "picking {target:?} reports a change");
        assert_eq!(nav.danger_level(), target, "the picked entry is stored");
    }
}

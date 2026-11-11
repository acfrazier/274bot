//! Real-content Running+Blocked Gatherer through the shared panel/TUI projection.
//! Only the minted account's tutorial state, pack, skill and position are seeded.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use api::identity::Settlement;
use api::interact::{cheat, tele_args};
use api::snapshot::GameSnapshot;
use frontend_core::command::Completion;
use frontend_core::scripts::{ScriptRunState, StatusView};
use frontend_core::views::{script_status_label, script_status_reason, script_status_rows};
use frontend_core::{
    fleet_row_matches, HeadlessSurface, OperatorCommand, OperatorSession, Targets,
};
use host::Pump;
use host_play::{InstancePermit, ProfileOptions, SharedClientTemplate};
use scenario::{Proof, RunnerStatus, ScenarioRunner, Step, StepKind, Wait};
use serde_json::json;
use vault::{Profile, ProfileSettings, RasterMode, Vault};

fn submit_and_wait(
    session: &mut OperatorSession<()>,
    surface: &mut HeadlessSurface<fn(&str)>,
    command: OperatorCommand,
    account: &str,
    deadline: Instant,
) -> Result<(), String> {
    let request = session
        .submit(command, Targets::Profile(account), surface)
        .map_err(|error| format!("submit {command:?}: {error:?}"))?
        .request();
    loop {
        session.poll();
        let report = session
            .commands()
            .report(request)
            .ok_or_else(|| format!("{command:?} report disappeared for {account}"))?;
        let [member] = report.members() else {
            return Err(format!(
                "{command:?} targeted {} members instead of only {account}",
                report.members().len()
            ));
        };
        match member.terminal() {
            Some(Settlement::Completed(Completion::Done)) => return Ok(()),
            Some(terminal) => {
                return Err(format!(
                    "{command:?} did not complete for {account}: {terminal:?}"
                ));
            }
            None => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("{command:?} deadline for {account}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_script_state(
    session: &mut OperatorSession<()>,
    account: &str,
    expected: script::RunState,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        session.poll();
        let play = session
            .play_for_test()
            .ok_or("OperatorSession lost its Play while awaiting a script state")?;
        play.pump_script_lifecycle(account);
        if play.script_state(account) == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "script state remained {:?} instead of {expected:?}",
                play.script_state(account)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_state(play: &host_play::Play, account: &str) -> ScriptRunState {
    match play.script_state(account) {
        script::RunState::Idle => ScriptRunState::Idle,
        script::RunState::Starting => ScriptRunState::Starting,
        script::RunState::Running => ScriptRunState::Running,
        script::RunState::Paused => ScriptRunState::Paused,
        script::RunState::Stopping => ScriptRunState::Stopping,
        script::RunState::Error => ScriptRunState::Error,
    }
}

fn status_view(status: &script::native::ScriptStatus) -> StatusView<'_> {
    StatusView::from_script(status)
}

struct LiveState {
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    capture: Option<(PathBuf, serde_json::Value)>,
    capture_result: Option<Result<PathBuf, String>>,
}

fn required_path(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required")));
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

fn capture(
    client: &mut client::client::Client,
    directory: &std::path::Path,
    mut receipt: serde_json::Value,
) -> Result<PathBuf, String> {
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("expected real Client CPU framebuffer".into());
    };
    let rgba: Vec<u8> = pixels
        .pixels
        .iter()
        .flat_map(|pixel| {
            [
                ((pixel >> 16) & 255) as u8,
                ((pixel >> 8) & 255) as u8,
                (pixel & 255) as u8,
                255,
            ]
        })
        .collect();
    receipt["frame"] = json!({"ingame": client.ingame, "scene_state": client.scene_state});
    scenario::shot::write_shot(
        directory,
        "01-running-blocked",
        &rgba,
        pixels.width as u32,
        pixels.height as u32,
        &serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, BLOCKED_* paths and Engine L at 47594/3080"]
fn gatherer_parked_status_is_shared_and_controllable() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let _home = script::IsolatedEnv::enter("blocked-run-live");
    let root = required_path("BLOCKED_EVIDENCE_DIR");
    std::fs::create_dir_all(&root).unwrap();
    let names = host_play::mint_live_names(1);
    let account = names[0].clone();
    let password = host_play::mint_live_entries(&names)[0].1.clone();
    let run_stamp = scenario::shot::stamp_utc(SystemTime::now());
    let directory = root.join(format!("gatherer-running-blocked_{account}_{run_stamp}"));
    std::fs::create_dir_all(&directory).unwrap();
    let vault_path = directory.join("vault");
    let mut vault = Vault::create(&vault_path, "blocked-run-live-pass-01").unwrap();
    vault
        .upsert(Profile {
            username: account.clone(),
            password: password.into(),
            uid: 1,
            settings: ProfileSettings {
                auto_login: true,
                raster: RasterMode::Cpu,
                ..Default::default()
            },
        })
        .unwrap();
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(host_play::live_ports::game_port()),
        http_port: Some(host_play::live_ports::http_port()),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        nav_pack: Some(required_path("BLOCKED_NAV_PACK")),
        engine_dir: Some(required_path("BLOCKED_ENGINE_DIR")),
        catalog_root: Some(required_path("BLOCKED_CATALOG_ROOT")),
        vault_path: Some(vault_path),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .unwrap()
    .bind()
    .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    // Reuse the registered Gatherer's tutorial skip/relog sequence, then
    // replace only its gathering fixture with the real Lumbridge fishing site.
    let mut prep = scenario::get("gatherer").unwrap();
    let seed_end = prep
        .steps
        .iter()
        .position(|step| step.name == "prepare Woodcutting 1 and a real bronze axe before Start")
        .unwrap();
    prep.steps.truncate(seed_end);
    prep.steps.push(Step {
        name: "seed a fly fishing rod but no feathers at the real Lumbridge site",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, "setstat fishing 20").is_sent()
                    && cheat(client, "give fly_fishing_rod 1").is_sent()
                    && cheat(client, &tele_args(0, 3238, 3252)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::ItemId { id: 309, count: 1 },
            budget_ticks: 120,
        },
    });
    prep.steps.push(Step {
        name: "observe the real content position before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: 3238,
                z: 3252,
                level: 0,
            },
            budget_ticks: 120,
        },
    });
    prep.proof = Proof::ItemId { id: 309, count: 1 };
    prep.settings.start_script = None;
    let mut runner = ScenarioRunner::with_world(prep, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(LiveState {
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        capture: None,
        capture_result: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let play = host_play::run_with_template(
        template,
        true,
        vec![],
        |_| (None, None),
        move |client, name, frame| {
            if name != frame_account {
                return;
            }
            let mut state = frame_state.lock().unwrap();
            let drain = state.pump.drain_client(client);
            host::publish_snapshot(&mut state.snapshot, client, drain);
            if !matches!(
                state.runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                state.runner.tick_with_hold(client, frame.hold);
            }
            if let Some((directory, receipt)) = state.capture.take() {
                state.capture_result = Some(capture(client, &directory, receipt));
            }
        },
    )
    .unwrap();
    state.lock().unwrap().runner.set_obj_names(play.obj_names());
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.declare_headless_render();
    session.start_for_test(vault, play);
    let mut surface = HeadlessSurface::new();
    let deadline = Instant::now() + Duration::from_secs(240);
    let result = (|| -> Result<(), String> {
        submit_and_wait(
            &mut session,
            &mut surface,
            OperatorCommand::Load,
            &account,
            deadline,
        )?;
        submit_and_wait(
            &mut session,
            &mut surface,
            OperatorCommand::Select,
            &account,
            deadline,
        )?;
        assert_eq!(session.selected(), Some(account.as_str()));
        loop {
            let relog_login = state.lock().unwrap().runner.take_relog_login();
            if relog_login {
                if let Some(arm) = session.play_for_test().and_then(|play| play.arm(&account)) {
                    arm.arm_explicit_login();
                }
            }
            session.poll();
            match state.lock().unwrap().runner.status() {
                RunnerStatus::Passed => break,
                RunnerStatus::Failed(reason) => return Err(reason),
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err("preparation deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let bag = serde_json::from_value(json!({
            "skill": "Fishing", "fishingMethod": "fishing.freshfish.op1",
            "location": "Start", "radius": 12, "disposition": "Power",
            "allowTeleports": false, "allowWilderness": false
        }))
        .unwrap();
        session
            .play_for_test()
            .ok_or("OperatorSession lost its Play before Gatherer start")?
            .script_start_handle()
            .start_compiled(&account, script::CompiledId("Gatherer"), bag)?;
        let blocked = loop {
            session.poll();
            let play = session
                .play_for_test()
                .ok_or("OperatorSession lost its Play while awaiting Gatherer status")?;
            if play.script_state(&account) == script::RunState::Running {
                if let Some(status) = play.script_native_status(&account) {
                    if status.phase == script::native::NativePhase::Blocked
                        && status.failure.is_none()
                    {
                        break status;
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err("running blocked outcome deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        session.poll();
        let play = session
            .play_for_test()
            .ok_or("OperatorSession lost its Play after Gatherer parked")?;
        assert_eq!(play.script_state(&account), script::RunState::Running);
        assert!(play.script_native_run(&account).is_some());
        let reason = script_status_reason(status_view(&blocked))
            .expect("a parked Gatherer publishes its block reason")
            .full();
        assert!(
            reason.to_ascii_lowercase().contains("feather"),
            "unexpected supply block reason: {reason}"
        );
        let core_label =
            script_status_label(run_state(play, &account), Some(status_view(&blocked)));
        assert_eq!(core_label, "blocked");
        let status_rows = script_status_rows(status_view(&blocked)).collect::<Vec<_>>();
        assert!(
            status_rows
                .iter()
                .any(|(label, value)| *label == "Bank seen (at status update)" && !value.is_empty()),
            "the real Gatherer status exposes its point-in-time bank observation"
        );
        let projected = {
            let view = session.fleet_view();
            let row = view.row(&account).expect("loaded account has a fleet row");
            assert_eq!(row.script(), ScriptRunState::Running);
            assert!(row.parked());
            assert_eq!(row.brief(), "blocked");
            row.snapshot()
        };
        assert!(fleet_row_matches(&account, Some(&projected), "blocked"));
        {
            let view = session.fleet_view();
            let phase = view
                .detail()
                .and_then(|detail| detail.script_phase())
                .expect("selected account detail retains the parked script phase");
            assert_eq!(phase.label(), "blocked");
            assert_eq!(phase.phase(), Some("blocked"));
            assert_eq!(phase.reason().map(|line| line.full()), Some(reason));
        }
        let observation = state.lock().unwrap();
        let snapshot = &observation.snapshot;
        assert!(snapshot.ingame() && snapshot.scene_state() == 2);
        assert!(snapshot.inventory().iter().any(|row| row.def.id == 309));
        assert!(!snapshot.inventory().iter().any(|row| row.def.id == 314));
        let receipt = json!({
            "request": "BLOCKED-RUN-3", "account": account,
            "outcome": "PASS", "run_state": "Running",
            "native_phase": "Blocked", "native_failure": null,
            "native_run_retained": true,
            "frontend_core_label": core_label,
            "fleet_brief": projected.brief(),
            "fleet_parked": projected.parked(),
            "fleet_filter_matches_blocked": true,
            "panel_tui_reason": reason,
            "status_rows": status_rows,
            "tile": snapshot.tile(), "scene_state": snapshot.scene_state(),
            "inventory": snapshot.inventory().iter().map(|row| json!({"id":row.def.id,"count":row.count})).collect::<Vec<_>>(),
            "fixture": "own tutorial state, Fishing 20, rod and position only; no feathers, resource spawn or XP injection"
        });
        drop(observation);
        state.lock().unwrap().capture = Some((directory.clone(), receipt));
        loop {
            session.poll();
            if let Some(result) = state.lock().unwrap().capture_result.take() {
                let path = result?;
                println!("running-blocked capture={}", path.display());
                break;
            }
            if Instant::now() >= deadline {
                return Err("capture deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        submit_and_wait(
            &mut session,
            &mut surface,
            OperatorCommand::PauseScript,
            &account,
            deadline,
        )?;
        wait_script_state(&mut session, &account, script::RunState::Paused, deadline)?;
        let paused = session
            .play_for_test()
            .and_then(|play| play.script_native_status(&account))
            .ok_or("Pause removed the parked script status")?;
        assert_eq!(
            script_status_label(
                run_state(
                    session.play_for_test().expect("Play remains attached"),
                    &account
                ),
                Some(status_view(&paused))
            ),
            "paused"
        );
        let paused_row = session
            .fleet_view()
            .row(&account)
            .expect("paused account remains projected")
            .snapshot();
        assert_eq!(paused_row.script(), ScriptRunState::Paused);
        assert!(!paused_row.parked());
        assert_ne!(paused_row.brief(), "blocked");

        submit_and_wait(
            &mut session,
            &mut surface,
            OperatorCommand::ResumeScript,
            &account,
            deadline,
        )?;
        wait_script_state(&mut session, &account, script::RunState::Running, deadline)?;
        let resumed = loop {
            session.poll();
            let row = session
                .fleet_view()
                .row(&account)
                .expect("resumed account remains projected");
            if row.parked() {
                break row.snapshot();
            }
            if Instant::now() >= deadline {
                return Err("resumed blocked fleet projection deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(resumed.brief(), "blocked");
        assert_eq!(resumed.script(), ScriptRunState::Running);
        let resumed_status = session
            .play_for_test()
            .and_then(|play| play.script_native_status(&account))
            .ok_or("Resume removed the parked script status")?;
        assert!(resumed_status.failure.is_none());
        assert_eq!(resumed_status.phase, script::native::NativePhase::Blocked);

        submit_and_wait(
            &mut session,
            &mut surface,
            OperatorCommand::StopScript,
            &account,
            deadline,
        )?;
        wait_script_state(&mut session, &account, script::RunState::Idle, deadline)?;
        let stopped_play = session
            .play_for_test()
            .ok_or("OperatorSession lost its Play after Stop")?;
        assert!(stopped_play.script_native_run(&account).is_none());
        let stopped = session
            .fleet_view()
            .row(&account)
            .expect("stopped account remains projected")
            .snapshot();
        assert_eq!(stopped.script(), ScriptRunState::Idle);
        assert!(!stopped.parked());
        assert_ne!(stopped.brief(), "blocked");
        Ok(())
    })();
    drop(session);
    result.unwrap();
}

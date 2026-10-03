//! Real-content blocked Gatherer through the shared panel/TUI presentation.
//! Only the minted account's tutorial state, pack, skill and position are seeded.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use api::interact::{cheat, tele_args};
use api::snapshot::GameSnapshot;
use frontend_core::log::{global, LogScope, LogView};
use frontend_core::views::{script_status_label, script_status_reason};
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use scenario::{Proof, RunnerStatus, ScenarioRunner, Step, StepKind, Wait};
use serde_json::json;

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
        "01-stopped-blocked",
        &rgba,
        pixels.width as u32,
        pixels.height as u32,
        &serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, BLOCKED_* paths and local 289 engine"]
fn gatherer_blocked_stops_with_shared_status() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let _home = script::IsolatedEnv::enter("blocked-stop-live");
    let root = required_path("BLOCKED_EVIDENCE_DIR");
    std::fs::create_dir_all(&root).unwrap();
    global();
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        nav_pack: Some(required_path("BLOCKED_NAV_PACK")),
        engine_dir: Some(required_path("BLOCKED_ENGINE_DIR")),
        catalog_root: Some(required_path("BLOCKED_CATALOG_ROOT")),
        vault_path: Some(root.join("unused-vault")),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .unwrap()
    .bind()
    .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let names = host_play::mint_live_names(1);
    let account = names[0].clone();
    let password = host_play::mint_live_entries(&names)[0].1.clone();
    let directory = root.join(format!(
        "blocked-stop_bait_{}_{}",
        account,
        scenario::shot::stamp_utc(SystemTime::now())
    ));
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
    let mut play = host_play::run_with_template(
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
    play.try_spawn_slot(
        vault::Profile {
            username: account.clone(),
            password: password.into(),
            uid: 1,
            settings: vault::ProfileSettings {
                auto_login: true,
                ..Default::default()
            },
        },
        None,
        None,
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(240);
    let result = (|| -> Result<(), String> {
        loop {
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
        play.script_start_handle()
            .start_compiled(&account, script::CompiledId("Gatherer"), bag)?;
        let blocked = loop {
            if let Some(status) = play.script_native_status(&account) {
                if status.failure.is_some() {
                    break status;
                }
            }
            if Instant::now() >= deadline {
                return Err("blocked outcome deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let failure = blocked.failure.as_ref().unwrap();
        assert_eq!(failure.code.as_ref(), "supply-missing");
        assert_eq!(play.script_state(&account), script::RunState::Idle);
        assert!(play.script_native_run(&account).is_none());
        assert_eq!(
            script_status_label(play.script_state(&account), Some(&blocked)),
            "stopped (blocked)"
        );
        assert_eq!(
            script_status_reason(&blocked),
            Some(failure.message.as_ref())
        );
        let mut log = LogView::new(LogScope::Slot(account.clone()));
        global().refresh(&mut log);
        let reason_logged = log.rows().iter().any(|row| {
            row.source == api::hostlog::Source::Script
                && row.message.contains(failure.message.as_ref())
        });
        let observation = state.lock().unwrap();
        let snapshot = &observation.snapshot;
        assert!(snapshot.ingame() && snapshot.scene_state() == 2);
        assert!(snapshot.inventory().iter().any(|row| row.def.id == 309));
        assert!(!snapshot.inventory().iter().any(|row| row.def.id == 314));
        let receipt = json!({
            "request": "BLOCKED-STOP-1", "account": account,
            "outcome": "PASS", "run_state": "Idle", "native_run": null,
            "frontend_core_label": script_status_label(play.script_state(&account), Some(&blocked)),
            "panel_tui_reason": script_status_reason(&blocked),
            "failure_code": failure.code, "log": log.to_text(),
            "reason_logged": reason_logged,
            "tile": snapshot.tile(), "scene_state": snapshot.scene_state(),
            "inventory": snapshot.inventory().iter().map(|row| json!({"id":row.def.id,"count":row.count})).collect::<Vec<_>>(),
            "fixture": "own tutorial state, Fishing 20, rod and position only; no feathers, resource spawn or XP injection"
        });
        drop(observation);
        state.lock().unwrap().capture = Some((directory.clone(), receipt));
        loop {
            if let Some(result) = state.lock().unwrap().capture_result.take() {
                let path = result?;
                println!("blocked-stop capture={}", path.display());
                break;
            }
            if Instant::now() >= deadline {
                return Err("capture deadline".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })();
    drop(play);
    result.unwrap();
}

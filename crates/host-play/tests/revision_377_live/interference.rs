//! The same real nettle sting in transit and inside an explicitly measured window.
use super::*;
use scenario::{
    CombatExpectation, Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step,
    StepKind, Wait,
};
use std::io::Write;
use std::process::Command;

// Selected 377 content: nettles.loc and maps/m49_52.jm2 (0 1 15).
// Picking without gloves runs damage_self(2); it changes only the picker.
const NETTLE_ID: i32 = 1181;
const NETTLE_TILE: (i32, i32, i32) = (3137, 3343, 0);
const START_TILE: (i32, i32, i32) = (3136, 3343, 0);
const WINDOW: &str = "self-nettle-sting";
const CHILD_MODE: &str = "PARITY_1I_WINDOW";

fn hitpoints(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == 3)
        .map(|stat| stat.effective)
}

fn combat_quiet(snapshot: &GameSnapshot) -> bool {
    snapshot.ingame()
        && snapshot.scene_state() == 2
        && snapshot.local_player().is_some_and(|local| {
            !local.player.actor.in_combat && local.player.actor.target.is_none()
        })
        && !snapshot.taking_damage()
}

fn state(snapshot: &GameSnapshot) -> Value {
    json!({"tick":snapshot.tick(),"tile":snapshot.tile(),"hitpoints":hitpoints(snapshot),
        "taking_damage":snapshot.taking_damage(),
        "actor":snapshot.local_player().map(|local| &local.player.actor)})
}

fn steps(inside: bool) -> Vec<Step> {
    let begin = || Step {
        name: "Begin measured nettle window",
        kind: StepKind::BeginMeasuredWindow {
            name: WINDOW,
            expectation: CombatExpectation::Quiet,
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 2,
        },
    };
    let mut steps = Vec::new();
    if inside {
        steps.push(begin());
    }
    steps.push(Step {
        name: "normal Pick Nettles without gloves: own account sting",
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                let Some(nettle) = snapshot.locs().iter().find(|loc| {
                    loc.id == NETTLE_ID && (loc.tile.x, loc.tile.z, loc.tile.level) == NETTLE_TILE
                }) else {
                    return false;
                };
                matches!(
                    Interactions::new(snapshot, client)
                        .interact(OpTarget::Loc(nettle), ActionSpec::Label("Pick".into())),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::StatAtMost { id: 3, max: 8 },
            budget_ticks: 12,
        },
    });
    if !inside {
        steps.push(Step {
            name: "disclosed transit sting expires before Begin",
            kind: StepKind::Await {
                evidence: "own sting observed and combat now quiet",
                ready: Box::new(|snapshot| {
                    hitpoints(snapshot) == Some(8) && combat_quiet(snapshot)
                }),
                summary: None,
            },
            wait: Wait {
                arm: Proof::IngameScene2,
                budget_ticks: 24,
            },
        });
        steps.push(begin());
    }
    steps.push(Step {
        name: "End measured nettle window",
        kind: StepKind::EndMeasuredWindow { name: WINDOW },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 2,
        },
    });
    steps
}

fn clean_logout(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last_attempt = None;
    while client.ingame {
        let started = Instant::now();
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        if !snapshot.ingame() {
            return Ok(());
        }
        // Match the live-map shutdown rule: the server may refuse logout
        // in combat; another request waits sixteen observed game ticks.
        if last_attempt.is_none_or(|tick| snapshot.tick().wrapping_sub(tick) >= 16) {
            let interfaces = std::sync::Arc::clone(&client.ifaces);
            if !interact::logout(client, interfaces.as_slice()) {
                return Err("clean logout interface unavailable".into());
            }
            last_attempt = Some(snapshot.tick());
        }
        if Instant::now() >= deadline {
            return Err("clean logout was not observed within thirty seconds".into());
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}

fn run_case(inside: bool, directory: &Path, mode: &str) -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let name = host_play::mint_live_names(1).remove(0);
    let mut log = fs::File::create(directory.join(format!("{mode}-events.jsonl")))
        .map_err(|error| error.to_string())?;
    let mut client = bound
        .template
        .prepare_client(fresh_uid(150 + i32::from(inside)), false)?;
    client.set_draw(false);
    client.maininit();
    assert_377_client(&client, false)?;
    if client.error_loading {
        return Err(format!(
            "fixture asset load: {}",
            client.last_progress_message
        ));
    }
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let result = (|| {
        if !interact::login(
            &mut client,
            &name,
            &host_play::profile_password(&name),
            false,
        ) {
            return Err("fixture login refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture login",
            Duration::from_secs(90),
            |state| state.ingame() && state.attached() && state.scene_state() == 2,
        )?;
        writeln!(
            log,
            "{}",
            json!({"phase":"fixture","account":name,"action":"api::interact::mainland_hop",
            "scope":"own fresh tutorial progress and position","state":state(&snapshot)})
        )
        .map_err(|error| error.to_string())?;
        interact::mainland_hop(&mut client);
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture mainland",
            Duration::from_secs(60),
            |state| {
                state.ingame() && state.scene_state() == 2 && state.tile() == Some((3220, 3220, 0))
            },
        )?;
        let interfaces = std::sync::Arc::clone(&client.ifaces);
        if !interact::logout(&mut client, interfaces.as_slice()) {
            return Err("fixture clean logout refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture logout",
            Duration::from_secs(60),
            |state| !state.ingame(),
        )?;
        if !interact::login(
            &mut client,
            &name,
            &host_play::profile_password(&name),
            false,
        ) {
            return Err("fixture relog refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture relog",
            Duration::from_secs(90),
            |state| {
                state.ingame() && state.scene_state() == 2 && state.tile() == Some((3220, 3220, 0))
            },
        )?;
        let command = format!(
            "tele {},{},{},{},{}",
            START_TILE.2,
            START_TILE.0 >> 6,
            START_TILE.1 >> 6,
            START_TILE.0 & 63,
            START_TILE.1 & 63
        );
        writeln!(log, "{}", json!({"phase":"fixture","account":name,"action":command,"scope":"own fresh account position"})).map_err(|error| error.to_string())?;
        if !interact::cheat(&mut client, &command).is_sent() {
            return Err("fixture own teleport refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture target and HP",
            Duration::from_secs(60),
            |state| {
                state.tile() == Some(START_TILE)
                    && state.scene_state() == 2
                    && hitpoints(state) == Some(10)
                    && state.locs().iter().any(|loc| {
                        loc.id == NETTLE_ID
                            && (loc.tile.x, loc.tile.z, loc.tile.level) == NETTLE_TILE
                    })
            },
        )?;
        writeln!(log, "{}", json!({"phase":"first-step-ready","account":name,"mode":mode,
            "disturbance":"normal Pick Nettles without gloves; own 2 HP damage","state":state(&snapshot)})).map_err(|error| error.to_string())?;
        log.flush().map_err(|error| error.to_string())?;
        let mut runner = ScenarioRunner::with_world(
            Scenario {
                name: "interference_377",
                seed: Seed {
                    profiles: vec![("interference", "unused")],
                    mainland: false,
                },
                steps: steps(inside),
                proof: Proof::StatAtMost { id: 3, max: 8 },
                companions: vec![],
                settings: ScenarioSettings {
                    require_mainland_base: false,
                    ..ScenarioSettings::default()
                },
            },
            None,
        );
        runner.set_live_names(std::slice::from_ref(&name));
        runner.set_scene_settle(Duration::ZERO);
        loop {
            let started = Instant::now();
            client.mainloop();
            host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
            runner.tick(&mut client);
            if matches!(
                runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                let evidence = runner.evidence().ok_or("terminal runner has no evidence")?;
                fs::write(
                    directory.join(format!("{mode}-runner.json")),
                    evidence.to_json(),
                )
                .map_err(|error| error.to_string())?;
                println!(
                    "{}",
                    json!({"cell":"interference_377","mode":mode,"account":name,"evidence":evidence,
                    "runner_exit_code":evidence.exit_code()})
                );
                return if evidence.is_pass() {
                    Ok(())
                } else {
                    Err(format!(
                        "{}: {}",
                        evidence.outcome,
                        evidence.message.as_deref().unwrap_or("")
                    ))
                };
            }
            if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    })();
    let cleanup = clean_logout(&mut client, &mut snapshot, &mut pump);
    fs::write(directory.join(format!("{mode}-cleanup.json")), json!({
        "success":cleanup.is_ok(),"logout_observed":!client.ingame,"error":cleanup.as_ref().err(),
    }).to_string()).map_err(|error| error.to_string())?;
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(error),
        (Err(original), Err(error)) => {
            Err(format!("{original}; additionally cleanup failed: {error}"))
        }
    }
}

#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, fresh prefix and PARITY_1I_EVIDENCE_DIR"]
fn measured_windows_377() -> Result<(), String> {
    let directory = required_path("PARITY_1I_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let inside = match mode.as_str() {
            "outside" => false,
            "inside" => true,
            _ => return Err("unknown measured-window child mode".into()),
        };
        let result = run_case(inside, &directory, &mode);
        if let Err(error) = &result {
            fs::write(
                directory.join(format!("{mode}-child-error.json")),
                json!({"mode":mode,"error":error}).to_string(),
            )
            .map_err(|error| error.to_string())?;
        }
        return result;
    }
    for mode in ["outside", "inside"] {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let output = Command::new(&executable)
            .args([
                "--ignored",
                "--exact",
                "interference::measured_windows_377",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_MODE, mode)
            .output()
            .map_err(|error| error.to_string())?;
        fs::write(directory.join(format!("{mode}-stdout.log")), &output.stdout)
            .map_err(|error| error.to_string())?;
        fs::write(directory.join(format!("{mode}-stderr.log")), &output.stderr)
            .map_err(|error| error.to_string())?;
        let runner = directory.join(format!("{mode}-runner.json"));
        let evidence: Result<Value, String> = fs::read(&runner)
            .map_err(|error| {
                format!("{mode} fixture failure before first tested step or receipt: {error}")
            })
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()));
        let cleanup: Result<Value, String> =
            fs::read(directory.join(format!("{mode}-cleanup.json")))
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
                });
        let exit_code = output.status.code();
        fs::write(directory.join(format!("{mode}-receipt.json")), serde_json::to_vec_pretty(&json!({
            "cell":"interference_377","mode":mode,"head":std::env::var("PARITY_1I_HEAD").ok(),
            "command":[executable.to_string_lossy(),"--ignored","--exact","interference::measured_windows_377","--nocapture","--test-threads=1"],
            "child_environment":{CHILD_MODE:mode},"exit_code":exit_code,"evidence":evidence.as_ref().ok(),
            "fixture_error":evidence.as_ref().err(),"cleanup":cleanup.as_ref().ok(),"cleanup_error":cleanup.as_ref().err(),
        })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        let evidence = evidence?;
        let cleanup = cleanup?;
        if cleanup["success"] != true || cleanup["logout_observed"] != true {
            return Err(format!(
                "{mode}: child did not complete clean logout: {cleanup}"
            ));
        }
        let expected = if mode == "outside" { "PASS" } else { "INVALID" };
        if evidence["outcome"] != expected
            || exit_code != Some(if mode == "outside" { 0 } else { 101 })
        {
            return Err(format!("{mode}: expected {expected} and corresponding child exit; got outcome={} exit={exit_code:?}", evidence["outcome"]));
        }
        let events = evidence["interference"]["events"]
            .as_array()
            .ok_or("missing typed interference events")?;
        let classification = if mode == "outside" {
            "transit"
        } else {
            "interference"
        };
        if !events.iter().any(|event| {
            event["classification"] == classification
                && matches!(
                    event["reason"].as_str(),
                    Some("hitpoints-decreased" | "fresh-hitmark" | "combat-active")
                )
        }) {
            return Err(format!(
                "{mode}: receipt does not disclose the actual nettle disturbance"
            ));
        }
        println!(
            "{}",
            json!({"cell":"interference_377","mode":mode,"outcome":expected,"exit_code":exit_code})
        );
    }
    Ok(())
}

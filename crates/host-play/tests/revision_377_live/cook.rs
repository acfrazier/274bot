//! Runs the registered scenario through the host snapshot pump and 377 client.
use super::*;
use scenario::{RunnerStatus, ScenarioRunner};
use std::io::Write;

fn sample(client: &client::client::Client, snapshot: &GameSnapshot) -> Value {
    json!({
        "tick": snapshot.tick(), "tile": snapshot.tile(),
        "scene": snapshot.scene_state(),
        "varps": snapshot.varps().iter().filter(|row| [29,101,202,203,695].contains(&row.index)).map(|row| (row.index,row.value)).collect::<Vec<_>>(),
        "cooking": snapshot.stats().iter().filter(|row| row.index == 7).collect::<Vec<_>>(),
        "inventory": snapshot.inventory(),
        "chat": snapshot.chat_lines(), "dialogue": snapshot.chat_modal_texts(),
        "options": snapshot.chat_options(), "modals": snapshot.modals(),
        "main_modal_texts": snapshot.main_modal_texts(),
        "quest_rows": snapshot.quest_statuses(),
        "quest_point_display": {"transmitted_total": client.var.get(101), "reward_text": snapshot.main_modal_texts().iter().filter(|text| text.contains("Quest Point")).collect::<Vec<_>>()},
        "locs": snapshot.locs(), "ground": snapshot.ground_items(),
    })
}

#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, live prefix and ROUTE_A_EVIDENCE_DIR"]
fn cook_377_route_a() -> Result<(), String> {
    let bound = load_bound_377()?;
    identity_record(&bound);
    let directory = required_path("ROUTE_A_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let username = host_play::mint_live_names(1).remove(0);
    let mut log =
        fs::File::create(directory.join(format!("{username}.jsonl"))).map_err(|e| e.to_string())?;
    let mut client = bound.template.prepare_client(fresh_uid(80), false)?;
    client.draw = false;
    client.maininit();
    assert_377_client(&client, false)?;
    if client.error_loading {
        return Err("377 asset initialization failed".into());
    }
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let result = (|| {
        if !interact::login(
            &mut client,
            &username,
            &host_play::profile_password(&username),
            false,
        ) {
            return Err(format!(
                "377 login refused: {} {}",
                client.login_mes1, client.login_mes2
            ));
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "cook-login",
            Duration::from_secs(90),
            |s| s.ingame() && s.scene_state() == 2 && s.tile().is_some(),
        )?;
        writeln!(log, "{}", json!({
            "account": username,
            "planned_seeds": [
                {"label": "SEED tutorial bypass", "command": "setvar tutorial 1000", "scope": "own account"},
                {"label": "SEED mainland position", "command": "tele 0,50,50,20,20", "scope": "own account"}
            ],
            "before": sample(&client, &snapshot)
        })).map_err(|e| e.to_string())?;
        // As in the TS mainlandAccount fixture, relog cleanly so quest tabs unlock.
        interact::mainland_hop(&mut client);
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "cook-mainland",
            Duration::from_secs(60),
            |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some((3220, 3220, 0)),
        )?;
        let ifaces = std::sync::Arc::clone(&client.ifaces);
        if !interact::logout(&mut client, ifaces.as_slice()) {
            return Err("Cook mainland fixture: clean logout interface unavailable".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "cook-clean-logout",
            Duration::from_secs(60),
            |s| !s.ingame(),
        )?;
        if !interact::login(
            &mut client,
            &username,
            &host_play::profile_password(&username),
            false,
        ) {
            return Err("Cook mainland fixture: clean relog refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "cook-relogin",
            Duration::from_secs(90),
            |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some((3220, 3220, 0)),
        )?;
        writeln!(log, "{}", json!({"account":username, "fixture":"clean mainland relog; no additional seeds", "state":sample(&client, &snapshot)})).map_err(|e| e.to_string())?;
        let cell = scenario::get("cook_377").ok_or("Cook scenario not registered")?;
        let mut runner = ScenarioRunner::with_world(cell, None);
        runner.set_live_names(std::slice::from_ref(&username));
        let deadline = Instant::now() + Duration::from_secs(1000);
        let mut last_step = None;
        let mut last_dialogue = String::new();
        loop {
            let started = Instant::now();
            client.mainloop();
            host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
            runner.tick(&mut client);
            let step = runner.record_step();
            let dialogue = format!(
                "{:?}{:?}",
                snapshot.chat_modal_texts(),
                snapshot.chat_options()
            );
            if last_step != Some(step.0) || last_dialogue != dialogue {
                writeln!(
                    log,
                    "{}",
                    json!({"step": step.0,"name":step.2,"state":sample(&client, &snapshot)})
                )
                .map_err(|e| e.to_string())?;
                log.flush().map_err(|e| e.to_string())?;
                println!(
                    "{}",
                    json!({"account":username,"step":step.0,"name":step.2,"tick":snapshot.tick(),"tile":snapshot.tile(),"dialogue":snapshot.chat_modal_texts(),"options":snapshot.chat_options()})
                );
                last_step = Some(step.0);
                last_dialogue = dialogue;
            }
            let outcome = match runner.status() {
                RunnerStatus::Passed => Some(Ok(())),
                RunnerStatus::Failed(reason) => Some(Err(format!("{}: {reason}", step.2))),
                _ if !snapshot.ingame() || Instant::now() >= deadline => {
                    Some(Err(format!("{}: disconnected or deadline", step.2)))
                }
                _ => None,
            };
            if let Some(result) = outcome {
                writeln!(log,"{}",json!({"outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),"account":username,"state":sample(&client, &snapshot)})).map_err(|e| e.to_string())?;
                println!(
                    "{}",
                    json!({"test":"cook_377_route_a","account":username,"outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err()})
                );
                return result;
            }
            if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    })();
    client.logout();
    result
}

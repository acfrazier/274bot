//! Real 377 fletching Make-X panel, prompt, consumption and XP witness.
use super::*;
use scenario::{RunnerStatus, ScenarioRunner};
use std::io::Write;

const LOG_ID: i32 = 1511;
const KNIFE_ID: i32 = 946;
const SHAFT_ID: i32 = 52;
const FLETCHING_STAT: i32 = 9;

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn fletching_xp(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == FLETCHING_STAT)
        .map(|stat| stat.xp)
}

fn sample(snapshot: &GameSnapshot) -> Value {
    json!({
        "tick": snapshot.tick(),
        "scene_state": snapshot.scene_state(),
        "count_dialog_open": snapshot.count_dialog_open(),
        "fletching": snapshot.stats().iter().find(|stat| stat.index == FLETCHING_STAT),
        "inventory": snapshot.inventory().iter().map(|item| json!({
            "id": item.def.id,
            "name": item.def.name.as_deref(),
            "count": item.count,
        })).collect::<Vec<_>>(),
        "make_products": snapshot.make_products().iter().map(|product| json!({
            "id": product.object_id,
            "name": product.name,
            "buttons": product.buttons.iter().map(|button| json!({
                "quantity": button.quantity,
                "component_id": button.component_id,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "main_make": snapshot.main_make().iter().map(|item| json!({
            "id": item.def.id,
            "name": item.def.name.as_deref(),
        })).collect::<Vec<_>>(),
    })
}

fn write_sample(log: &mut fs::File, phase: &str, snapshot: &GameSnapshot) -> Result<(), String> {
    writeln!(log, "{}", json!({"phase":phase,"state":sample(snapshot)}))
        .map_err(|error| error.to_string())?;
    log.flush().map_err(|error| error.to_string())
}

#[test]
#[ignore = "requires canonical 377 resources, LIVE=1, live prefix and PARITY_3B_EVIDENCE_DIR"]
fn fletching_make_x_377() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let directory = required_path("PARITY_3B_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let username = host_play::mint_live_names(1).remove(0);
    let mut log = fs::File::create(directory.join(format!("{username}.jsonl")))
        .map_err(|error| error.to_string())?;
    let mut client = bound.template.prepare_client(fresh_uid(90), false)?;
    client.draw = false;
    client.maininit();
    assert_377_client(&client, false)?;
    if client.error_loading {
        return Err(format!(
            "377 asset initialization failed: {}",
            client.last_progress_message
        ));
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
            "fletching-login",
            Duration::from_secs(90),
            |state| state.ingame() && state.attached() && state.scene_state() == 2,
        )?;

        let fixture: [(&str, &str, fn(&GameSnapshot) -> bool); 3] = [
            ("setstat fletching 5", "fletching-stat-seeded", |state| {
                state
                    .stats()
                    .iter()
                    .any(|stat| stat.index == FLETCHING_STAT && stat.base >= 5)
            }),
            ("give knife 1", "knife-seeded", |state| {
                item_count(state, KNIFE_ID) == 1
            }),
            ("give logs 2", "logs-seeded", |state| {
                item_count(state, LOG_ID) == 2
            }),
        ];
        for (command, phase, ready) in fixture {
            writeln!(
                log,
                "{}",
                json!({
                    "phase":"fixture-command",
                    "command":command,
                    "scope":"fresh account's own stats/materials",
                    "state":sample(&snapshot),
                })
            )
            .map_err(|error| error.to_string())?;
            log.flush().map_err(|error| error.to_string())?;
            if !interact::cheat(&mut client, command).is_sent() {
                return Err(format!("own-account fixture command refused: {command}"));
            }
            wait_for(
                &mut client,
                &mut snapshot,
                &mut pump,
                phase,
                Duration::from_secs(30),
                ready,
            )?;
            write_sample(&mut log, phase, &snapshot)?;
        }

        let knife = snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == KNIFE_ID)
            .ok_or("seeded knife is not in inventory")?;
        let logs = snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == LOG_ID)
            .ok_or("seeded logs are not in inventory")?;
        accepted(
            Interactions::new(&snapshot, &mut client).use_item_on(knife, OpTarget::Item(logs)),
            "open fletching Make panel with own knife/logs",
        )?;
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "make-panel-open",
            Duration::from_secs(30),
            |state| !state.make_products().is_empty() || !state.main_make().is_empty(),
        )?;
        if snapshot.make_products().is_empty() && snapshot.main_make().is_empty() {
            return Err("Make panel readiness ended without a published product view".into());
        }
        write_sample(&mut log, "make-panel-open", &snapshot)?;

        let before_shafts = item_count(&snapshot, SHAFT_ID);
        let before_logs = item_count(&snapshot, LOG_ID);
        let before_xp = fletching_xp(&snapshot).ok_or("Fletching XP is not published")?;
        let scenario = scenario::get("fletching_make_x")
            .ok_or("fletching_make_x scenario is not registered")?;
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_live_names(std::slice::from_ref(&username));
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut last_step = None;
        let mut last_prompt = None;
        loop {
            let started = Instant::now();
            client.mainloop();
            host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
            runner.tick(&mut client);
            let step = runner.record_step();
            let prompt = snapshot.count_dialog_open();
            if last_step != Some(step.0) || last_prompt != Some(prompt) {
                write_sample(&mut log, step.2, &snapshot)?;
                last_step = Some(step.0);
                last_prompt = Some(prompt);
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
                write_sample(
                    &mut log,
                    if result.is_ok() { "PASS" } else { "FAIL" },
                    &snapshot,
                )?;
                let after_xp = fletching_xp(&snapshot).ok_or("Fletching XP disappeared")?;
                let product_delta = item_count(&snapshot, SHAFT_ID) - before_shafts;
                let material_delta = before_logs - item_count(&snapshot, LOG_ID);
                let xp_delta = after_xp - before_xp;
                writeln!(
                    log,
                    "{}",
                    json!({
                        "outcome":if result.is_ok(){"PASS"}else{"FAIL"},
                        "account":username,
                        "revision":REVISION,
                        "fixture_commands":["setstat fletching 5","give knife 1","give logs 2"],
                        "output_seeded":false,
                        "product_delta":product_delta,
                        "material_consumed":material_delta,
                        "xp_delta":xp_delta,
                        "expected":{"arrow_shafts":30,"logs_consumed":2,"fletching_xp":100},
                    })
                )
                .map_err(|error| error.to_string())?;
                log.flush().map_err(|error| error.to_string())?;
                if let Err(error) = result {
                    return Err(error);
                }
                if product_delta != 30 || material_delta != 2 || xp_delta != 100 {
                    return Err(format!(
                        "Make-X result delta mismatch: shafts={product_delta}, logs={material_delta}, xp={xp_delta}"
                    ));
                }
                println!(
                    "{}",
                    json!({
                        "test":"revision_377_live",
                        "cell":"fletching_make_x_377",
                        "outcome":"PASS",
                        "account":username,
                        "product_delta":product_delta,
                        "material_consumed":material_delta,
                        "xp_delta":xp_delta,
                        "receipt": directory.join(format!("{username}.jsonl")).display().to_string(),
                    })
                );
                return Ok(());
            }
            if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    })();
    if client.ingame {
        client.logout();
    }
    result
}

//! Cache-defined varbits with independently observed, post-action base values.
use super::*;
use api::snapshot::{varbit_value, VarbitObservation, VarbitUnavailable, VarbitValue};
use scenario::proof::{ObservedVarpWitness, Proof};
use scenario::{
    RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use std::io::Write;

// quest_haunted.rs2:277-285 toggles this bit through the real lever's Pull action.
// all.varbit:8756-8759 defines ernestlever_a; quest_haunted.varp:5-7 transmits its base.
const KNOWN_ID: i32 = 1788;
const BASE_ID: i32 = 33;
const LEVER_ID: i32 = 146;
const LEVER_TILE: (i32, i32, i32) = (3108, 9745, 0);
const STAND: (i32, i32, i32) = (3107, 9745, 0);
// all.varbit:4881-4884 and all.varp:1129 define a private, untransmitted base.
const PRIVATE_ID: i32 = 997;
const PRIVATE_BASE: i32 = 562;

fn observation_scenario(
    id: i32,
    value: i32,
    observed_base_varp: Option<ObservedVarpWitness>,
) -> Scenario {
    let proof = Proof::VarbitEquals {
        id,
        value,
        observed_base_varp,
    };
    Scenario {
        name: "varbit_377_live",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "observe the selected cache varbit",
            kind: StepKind::ObserveVarbit { id },
            wait: Wait {
                arm: proof,
                budget_ticks: 2,
            },
        }],
        proof,
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

fn sample(snapshot: &GameSnapshot, client: &Client) -> Value {
    json!({
        "tick": snapshot.tick(),
        "session": client.gens.session,
        "varp_generation": client.gens.varp,
        "tile": snapshot.tile(),
        "bases": ([BASE_ID, PRIVATE_BASE].map(|id| json!({
            "id": id,
            "value": snapshot.varps().iter().find(|row| row.index == id).map(|row| row.value),
        }))),
    })
}

fn write_record(log: &mut fs::File, phase: &str, detail: Value) -> Result<(), String> {
    writeln!(log, "{}", json!({"phase": phase, "detail": detail}))
        .and_then(|()| log.flush())
        .map_err(|error| error.to_string())
}

fn candidate(
    snapshot: &GameSnapshot,
    client: &Client,
    id: i32,
    base: i32,
    bits: (i32, i32),
) -> Result<VarbitValue, String> {
    let definition = client
        .cache
        .varbits
        .get(usize::try_from(id).map_err(|error| error.to_string())?)
        .ok_or_else(|| format!("missing expected varbit {id}"))?;
    if (definition.basevar, definition.startbit, definition.endbit) != (base, bits.0, bits.1) {
        return Err(format!("unexpected cache definition for varbit {id}"));
    }
    varbit_value(snapshot, &client.cache, id).map_err(|error| format!("{error:?}"))
}

fn observer_check(
    client: &mut Client,
    account: &str,
    id: i32,
    value: i32,
    witness: Option<ObservedVarpWitness>,
    outcome: &str,
    reason: Option<&str>,
) -> Result<Value, String> {
    let mut runner = ScenarioRunner::with_world(observation_scenario(id, value, witness), None);
    runner.set_live_names(&[account.to_owned()]);
    runner.set_scene_settle(Duration::ZERO);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let sent_before = client.out.pos;
        runner.tick(client);
        if client.out.pos != sent_before {
            return Err("ObserveVarbit sent client output".into());
        }
        if matches!(
            runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "varbit observer did not finish: {:?}",
                runner.status()
            ));
        }
        client.mainloop();
        std::thread::sleep(Duration::from_millis(20));
    }
    let receipt = runner
        .evidence()
        .ok_or("observer has no terminal receipt")?;
    if receipt.outcome != outcome
        || reason.is_some_and(|reason| {
            !receipt
                .message
                .as_deref()
                .is_some_and(|message| message.contains(reason))
        })
    {
        return Err(format!(
            "observer expected {outcome}/{reason:?}, got {}/{:?}",
            receipt.outcome, receipt.message
        ));
    }
    Ok(json!({
        "receipt": receipt,
        "witness": witness.map(|witness| json!({
            "varp_id": witness.varp_id,
            "value": witness.value,
        })),
        "zero_observer_output": true,
    }))
}

fn pull_lever(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    expected: i32,
    log: &mut fs::File,
) -> Result<ObservedVarpWitness, String> {
    let before_tick = snapshot.tick();
    let before_generation = client.gens.varp;
    let before_session = client.gens.session;
    let before = candidate(snapshot, client, KNOWN_ID, BASE_ID, (1, 1))?;
    let VarbitObservation::Unknown { base_value, .. } = before.observation;
    if base_value == expected {
        return Err("lever action cannot witness an unchanged base value".into());
    }
    write_record(log, "action-begin", sample(snapshot, client))?;
    let lever = snapshot
        .locs()
        .iter()
        .find(|loc| loc.id == LEVER_ID && (loc.tile.x, loc.tile.z, loc.tile.level) == LEVER_TILE)
        .ok_or("real lever A is absent from the selected scene")?;
    accepted(
        Interactions::new(snapshot, client)
            .interact(OpTarget::Loc(lever), ActionSpec::Label("Pull".into())),
        "pull real lever A",
    )?;
    wait_for(
        client,
        snapshot,
        pump,
        "lever-base-varp-receive",
        Duration::from_secs(30),
        |state| {
            state.tick() > before_tick
                && state
                    .varps()
                    .iter()
                    .any(|row| row.index == BASE_ID && row.value == expected)
        },
    )?;
    if client.gens.session != before_session || client.gens.varp == before_generation {
        return Err("lever base value lacks a same-session post-action varp publication".into());
    }
    write_record(log, "action-result", sample(snapshot, client))?;
    Ok(ObservedVarpWitness {
        varp_id: BASE_ID,
        value: expected,
    })
}

#[test]
#[ignore = "requires selected 377 resources, LIVE=1, own prefix and PARITY_VARBIT_EVIDENCE_DIR"]
fn varbit_377_known_and_missing_are_read_only() -> Result<(), String> {
    let directory = required_path("PARITY_VARBIT_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut log = fs::File::create(directory.join("varbit-witness.jsonl"))
        .map_err(|error| error.to_string())?;
    let mut identity = serde_json::Map::new();
    for (key, variable) in [
        ("host_head", "PARITY_VARBIT_HEAD"),
        ("engine_head", "PARITY_VARBIT_ENGINE"),
        ("content_head", "PARITY_VARBIT_CONTENT"),
        ("client_pin", "PARITY_VARBIT_CLIENT"),
        ("served_crc_sha256", "PARITY_VARBIT_CRC"),
    ] {
        identity.insert(
            key.into(),
            Value::String(std::env::var(variable).map_err(|error| format!("{variable}: {error}"))?),
        );
    }
    write_record(&mut log, "identity", Value::Object(identity))?;
    let bound = load_bound_377()?;
    resource_record(&bound);
    let username = host_play::mint_live_names(1).remove(0);
    let mut client = bound.template.prepare_client(fresh_uid(90), false)?;
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
            return Err("377 login refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-login",
            Duration::from_secs(90),
            |state| state.ingame() && state.scene_state() == 2,
        )?;
        write_record(
            &mut log,
            "seed-mainland",
            json!({
                "helper": "api::interact::mainland_hop",
                "scope": "own tutorial progress and position",
            }),
        )?;
        interact::mainland_hop(&mut client);
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-mainland",
            Duration::from_secs(60),
            |state| {
                state.ingame() && state.scene_state() == 2 && state.tile() == Some((3220, 3220, 0))
            },
        )?;
        let ifaces = Arc::clone(&client.ifaces);
        if !interact::logout(&mut client, ifaces.as_slice()) {
            return Err("mainland fixture logout unavailable".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-fixture-logout",
            Duration::from_secs(60),
            |state| !state.ingame(),
        )?;
        if !interact::login(
            &mut client,
            &username,
            &host_play::profile_password(&username),
            false,
        ) {
            return Err("mainland fixture relog refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-fixture-relogin",
            Duration::from_secs(90),
            |state| state.ingame() && state.scene_state() == 2 && state.tile().is_some(),
        )?;
        write_record(
            &mut log,
            "seed-position",
            json!({
                "command": "tele 3107 9745 0", "scope": "own position",
            }),
        )?;
        if !interact::cheat(&mut client, "tele 3107 9745 0").is_sent() {
            return Err("lever fixture teleport refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-lever-fixture",
            Duration::from_secs(60),
            |state| state.scene_state() == 2 && state.tile() == Some(STAND),
        )?;
        let initial = candidate(&snapshot, &client, KNOWN_ID, BASE_ID, (1, 1))?;
        if initial.value != 0 {
            return Err(format!("fresh account's lever A is not up: {initial:?}"));
        }
        let private = candidate(&snapshot, &client, PRIVATE_ID, PRIVATE_BASE, (0, 3))?;
        let missing_id =
            i32::try_from(client.cache.varbits.len()).map_err(|error| error.to_string())?;
        if varbit_value(&snapshot, &client.cache, missing_id)
            != Err(VarbitUnavailable::MissingDefinition { id: missing_id })
        {
            return Err("missing varbit id did not return MissingDefinition".into());
        }
        let missing = observer_check(
            &mut client,
            &username,
            missing_id,
            0,
            None,
            "UNAVAILABLE",
            Some("MissingDefinition"),
        )?;
        let never_observed = observer_check(
            &mut client,
            &username,
            PRIVATE_ID,
            private.value,
            None,
            "UNAVAILABLE",
            Some("UnobservedBaseVarp"),
        )?;
        write_record(
            &mut log,
            "missing-cases",
            json!({
                "definition": missing, "unobserved_base": never_observed,
                "private_candidate": format!("{private:?}"), "private_id": PRIVATE_ID,
                "private_base": PRIVATE_BASE,
            }),
        )?;

        // The first ordinary Pull supplies the observed BEFORE value for the second Pull.
        let before_witness = pull_lever(&mut client, &mut snapshot, &mut pump, 2, &mut log)?;
        let before = candidate(&snapshot, &client, KNOWN_ID, BASE_ID, (1, 1))?;
        if before.value != 1 {
            return Err(format!("first Pull did not set lever A: {before:?}"));
        }
        let before_check = observer_check(
            &mut client,
            &username,
            KNOWN_ID,
            1,
            Some(before_witness),
            "PASS",
            None,
        )?;
        let after_witness = pull_lever(&mut client, &mut snapshot, &mut pump, 0, &mut log)?;
        let after = candidate(&snapshot, &client, KNOWN_ID, BASE_ID, (1, 1))?;
        if after.value != 0 {
            return Err(format!("second Pull did not reset lever A: {after:?}"));
        }
        let after_check = observer_check(
            &mut client,
            &username,
            KNOWN_ID,
            0,
            Some(after_witness),
            "PASS",
            None,
        )?;
        let no_witness = observer_check(
            &mut client,
            &username,
            KNOWN_ID,
            0,
            None,
            "UNAVAILABLE",
            Some("UnobservedBaseVarp"),
        )?;
        let stale_witness = observer_check(
            &mut client,
            &username,
            KNOWN_ID,
            0,
            Some(before_witness),
            "FAIL",
            None,
        )?;
        let wrong_index = observer_check(
            &mut client,
            &username,
            KNOWN_ID,
            0,
            Some(ObservedVarpWitness {
                varp_id: PRIVATE_BASE,
                value: after_witness.value,
            }),
            "FAIL",
            None,
        )?;
        write_record(
            &mut log,
            "known-cases",
            json!({
                "id": KNOWN_ID, "base_varp": BASE_ID, "start_bit": 1, "end_bit": 1,
                "before": before_check, "after": after_check, "no_witness": no_witness,
                "stale_witness": stale_witness, "wrong_index": wrong_index,
            }),
        )?;
        let ifaces = Arc::clone(&client.ifaces);
        if !interact::logout(&mut client, ifaces.as_slice()) {
            return Err("terminal clean logout unavailable".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-clean-logout",
            Duration::from_secs(60),
            |state| !state.ingame(),
        )?;
        Ok(())
    })();
    write_record(
        &mut log,
        "verdict",
        json!({
            "verdict": if result.is_ok() { "PASS" } else { "FAIL" },
            "error": result.as_ref().err(),
            "account": username,
            "known": "normal Pull: base 33, bit 1, observed 1 before and 0 after",
            "missing_definition": "UNAVAILABLE/MissingDefinition",
            "defined_unobserved_base": "UNAVAILABLE/UnobservedBaseVarp",
        }),
    )?;
    client.logout();
    result
}

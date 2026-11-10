//! Exercises the cache-backed varbit observer against the live 377 snapshot.
use super::*;
use api::snapshot::{varbit_value, VarbitObservation};
use scenario::proof::{ObservedVarpWitness, Proof};
use scenario::{
    RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};

// Content marks `dueloptions` transmitted in
// `content/scripts/minigames/game_duelarena/configs/duelarena.varp:9-11`.
// The canon-1108 cache metadata disagrees, so the cell supplies a witness from
// the live same-session base-varp readback instead of trusting that metadata.
// `duelwornoptions` covers bits 14..=27 in
// `content/scripts/_unpack/377/all.varbit:3126-3129`.
// `content/pack/varp.pack:287` and `content/pack/varbit.pack:643` map ids 286 and 642.
const KNOWN_VARBIT_ID: usize = 642;
const BASE_VARP_ID: i32 = 286;
const BASE_VARP_VALUE: i32 = 1 << 14;
const KNOWN_VARBIT_VALUE: i32 = 1;
const SET_VARP_COMMAND: &str = "setvar dueloptions 16384";

fn observation_scenario(
    id: i32,
    value: i32,
    observed_base_varp: Option<ObservedVarpWitness>,
) -> Scenario {
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
                arm: Proof::VarbitEquals {
                    id,
                    value,
                    observed_base_varp,
                },
                budget_ticks: 2,
            },
        }],
        proof: Proof::VarbitEquals {
            id,
            value,
            observed_base_varp,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
#[ignore = "requires canonical 377 cache/snapshot/nav resources, LIVE=1 and an owned BOT_LIVE_NAME_PREFIX"]
fn varbit_377_known_and_missing_are_read_only() -> Result<(), String> {
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
            return Err(format!(
                "377 login refused: {} {}",
                client.login_mes1, client.login_mes2
            ));
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "varbit-login",
            Duration::from_secs(90),
            |state| {
                state.ingame()
                    && state.scene_state() == 2
                    && state.tile().is_some()
                    && !state.varps().is_empty()
            },
        )?;

        let (
            known_id,
            base_varp,
            start_bit,
            end_bit,
            raw_value,
            known_value,
            known_observation,
            missing_id,
        ) = {
            if !interact::cheat(&mut client, SET_VARP_COMMAND).is_sent() {
                return Err(format!("setvar command was refused: {SET_VARP_COMMAND}"));
            }
            let initial_base_value = snapshot
                .varps()
                .iter()
                .find(|row| row.index == BASE_VARP_ID)
                .map(|row| row.value)
                .ok_or("fresh account has no dueloptions base-varp snapshot row")?;
            if initial_base_value == BASE_VARP_VALUE {
                return Err("fresh account already has the seeded dueloptions value".into());
            }
            wait_for(
                &mut client,
                &mut snapshot,
                &mut pump,
                "varbit-setvar",
                Duration::from_secs(30),
                |state| {
                    state
                        .varps()
                        .iter()
                        .any(|row| row.index == BASE_VARP_ID && row.value == BASE_VARP_VALUE)
                },
            )?;

            let cache = client.cache.as_ref();
            let known_id = i32::try_from(KNOWN_VARBIT_ID)
                .map_err(|_| "known varbit id exceeds the supported range")?;
            let definition = cache
                .varbits
                .get(KNOWN_VARBIT_ID)
                .ok_or("377 cache is missing duelwornoptions varbit 642")?;
            if definition.basevar != BASE_VARP_ID
                || definition.startbit != 14
                || definition.endbit != 27
            {
                return Err(format!(
                    "duelwornoptions cache definition mismatch: basevar {} bits {}..={}",
                    definition.basevar, definition.startbit, definition.endbit
                ));
            }
            let raw_value = snapshot
                .varps()
                .iter()
                .find(|row| row.index == BASE_VARP_ID)
                .map(|row| row.value)
                .ok_or("server did not observe the dueloptions base varp")?;
            let known_result = varbit_value(&snapshot, cache, known_id).map_err(|error| {
                format!("duelwornoptions was unavailable after setvar: {error:?}")
            })?;
            let known_value = known_result.value;
            let known_observation = known_result.observation;
            if known_observation
                != (VarbitObservation::Unknown {
                    base_varp: BASE_VARP_ID,
                    base_value: raw_value,
                })
            {
                return Err(format!(
                    "duelwornoptions observation marker mismatch: {known_observation:?}"
                ));
            }
            if known_value != KNOWN_VARBIT_VALUE {
                return Err(format!(
                    "duelwornoptions expected {KNOWN_VARBIT_VALUE}, observed {known_value} from base {raw_value}"
                ));
            }
            let missing_id = i32::try_from(cache.varbits.len())
                .map_err(|_| "377 varbit table exceeds the supported id range")?;
            (
                known_id,
                definition.basevar,
                definition.startbit,
                definition.endbit,
                raw_value,
                known_value,
                known_observation,
                missing_id,
            )
        };

        let sent_before = client.out.pos;
        let witness = Some(ObservedVarpWitness {
            varp_id: base_varp,
            value: raw_value,
        });
        let mut known =
            ScenarioRunner::with_world(observation_scenario(known_id, known_value, witness), None);
        known.set_live_names(std::slice::from_ref(&username));
        known.set_scene_settle(Duration::ZERO);
        known.tick(&mut client);
        if known.status() != RunnerStatus::Passed {
            return Err(format!(
                "known varbit runner did not pass: {:?}",
                known.status()
            ));
        }
        if client.out.pos != sent_before {
            return Err("known varbit observation sent client output".into());
        }
        drop(known);

        let mut missing =
            ScenarioRunner::with_world(observation_scenario(missing_id, 0, None), None);
        missing.set_live_names(std::slice::from_ref(&username));
        missing.set_scene_settle(Duration::ZERO);
        missing.tick(&mut client);
        let missing_message = match missing.status() {
            RunnerStatus::Failed(message) if message.contains("UNAVAILABLE") => message,
            status => return Err(format!("missing varbit was not unavailable: {status:?}")),
        };
        if !missing_message.contains("MissingDefinition") {
            return Err(format!(
                "missing varbit reason was not explicit: {missing_message}"
            ));
        }
        if client.out.pos != sent_before {
            return Err("missing varbit observation sent client output".into());
        }
        println!(
            "{}",
            json!({
                "test": "varbit_377_known_and_missing_are_read_only",
                "account": username,
                "seed_command": SET_VARP_COMMAND,
                "content_evidence": {
                    "varp": "content/scripts/minigames/game_duelarena/configs/duelarena.varp:9-11",
                    "varbit": "content/scripts/_unpack/377/all.varbit:3126-3129",
                },
                "known": {
                    "id": known_id,
                    "base_varp": base_varp,
                    "start_bit": start_bit,
                    "end_bit": end_bit,
                    "base_value": raw_value,
                    "value": known_value,
                    "observation": format!("{known_observation:?}"),
                    "observed_base_varp_witness": {
                        "varp_id": base_varp,
                        "value": raw_value
                    },
                    "runner": "PASS"
                },
                "missing": {
                    "id": missing_id,
                    "runner": "UNAVAILABLE",
                    "reason": missing_message
                },
                "zero_output_sends": client.out.pos == sent_before
            })
        );
        Ok(())
    })();
    client.logout();
    result
}

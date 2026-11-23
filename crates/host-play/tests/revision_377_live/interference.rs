//! The same real nettle sting in transit and inside an explicitly measured window.
use super::*;
use host_play::evidence_writer::record_scenario_sources;
use scenario::{
    CombatExpectation, Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step,
    StepKind, Wait,
};
use std::io::Write;
use std::process::Command;
use std::sync::Mutex;

// Selected 377 content: nettles.loc and maps/m49_52.jm2 (0 1 15).
// Picking without gloves runs damage_self(2); it changes only the picker.
const NETTLE_ID: i32 = 1181;
const NETTLE_TILE: (i32, i32, i32) = (3137, 3343, 0);
const START_TILE: (i32, i32, i32) = (3136, 3343, 0);
// Leave the manor's aggressive NPCs through normal scene walking. This
// destination is south of the nettles, not a teleport or a world-state edit.
const ESCAPE_TILE: (i32, i32, i32) = (3140, 3320, 0);
// The level-2 Giant spider spawns on this route (maps/m48_52.jm2:10653;
// all.npc:1790-1819). Its cowardly hunt (all.hunt:37-45) skips players above
// twice its level here (Npc.ts:1017-1019). Defence 8 reaches combat level 5
// on a fresh account without changing its 10 HP (Player.ts:1433-1438).
const MIN_NETTLE_COMBAT_LEVEL: i32 = 5;
const NETTLE_COMBAT_SEED: &str = "setstat defence 8";
const QUIET_TICKS: u32 = 4;
const WINDOW: &str = "self-nettle-sting";
const CHILD_MODE: &str = "PARITY_1I_WINDOW";

fn hitpoints(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == 3)
        .map(|stat| stat.effective)
}

fn combat_level(snapshot: &GameSnapshot) -> Option<i32> {
    snapshot
        .local_player()
        .map(|local| local.player.combat_level)
}

fn nettle_stats_ready(level: Option<i32>, hp: Option<i32>) -> bool {
    level.is_some_and(|level| level >= MIN_NETTLE_COMBAT_LEVEL) && hp == Some(10)
}

fn combat_quiet(snapshot: &GameSnapshot) -> bool {
    snapshot.ingame()
        && snapshot.scene_state() == 2
        && snapshot.local_player().is_some_and(|local| {
            !local.player.actor.in_combat && local.player.actor.target.is_none()
        })
        && !snapshot.taking_damage()
}

#[derive(Default)]
struct QuietInterval {
    first: Option<u32>,
    last: Option<u32>,
}

impl QuietInterval {
    fn observe(&mut self, tick: u32, quiet: bool) {
        if !quiet {
            self.first = None;
        }
        if self.last == Some(tick) {
            return;
        }
        if quiet && (self.first.is_none() || self.last != Some(tick.wrapping_sub(1))) {
            self.first = Some(tick);
        }
        self.last = Some(tick);
    }

    fn elapsed(&self) -> u32 {
        self.first
            .zip(self.last)
            .map_or(0, |(first, last)| last.wrapping_sub(first))
    }

    fn ready(&self) -> bool {
        self.first.is_some() && self.elapsed() >= QUIET_TICKS
    }
}

#[derive(Default)]
struct EscapeWitness {
    from: Option<(i32, i32, i32)>,
    requested_tick: Option<u32>,
    arrived_tick: Option<u32>,
    quiet: QuietInterval,
}

impl EscapeWitness {
    fn send(&mut self, client: &mut Client, snapshot: &GameSnapshot) -> bool {
        let (x, z, level) = ESCAPE_TILE;
        if !matches!(
            Interactions::new(snapshot, client).walk(WorldTile { x, z, level }),
            SendResult::Sent { .. }
        ) {
            return false;
        }
        self.from = snapshot.tile();
        self.requested_tick = Some(snapshot.tick());
        true
    }

    fn observe(&mut self, snapshot: &GameSnapshot) {
        let arrived = snapshot.tile() == Some(ESCAPE_TILE);
        if arrived {
            self.arrived_tick.get_or_insert(snapshot.tick());
        }
        self.quiet
            .observe(snapshot.tick(), arrived && combat_quiet(snapshot));
    }

    fn receipt(&self) -> Value {
        json!({
            "action":self.requested_tick.map(|_| "normal Interactions::walk"),
            "from":self.from,"to":ESCAPE_TILE,"requested_tick":self.requested_tick,
            "arrived_tick":self.arrived_tick,
            "quiet":{"first_tick":self.quiet.first,"last_tick":self.quiet.last,
                "elapsed_ticks":self.quiet.elapsed(),"required_ticks":QUIET_TICKS,
                "ready":self.quiet.ready()},
        })
    }
}

fn state(snapshot: &GameSnapshot) -> Value {
    json!({"tick":snapshot.tick(),"tile":snapshot.tile(),"hitpoints":hitpoints(snapshot),
        "combat_level":combat_level(snapshot),
        "taking_damage":snapshot.taking_damage(),"hitmarks":snapshot.hitmarks(),
        "actor":snapshot.local_player().map(|local| &local.player.actor)})
}

fn steps(inside: bool, escape: Arc<Mutex<EscapeWitness>>) -> Vec<Step> {
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
        let walk_escape = Arc::clone(&escape);
        steps.push(Step {
            name: "normal walk south out of the nettle NPCs' reach",
            kind: StepKind::Perform {
                send: Box::new(move |client, snapshot| {
                    hitpoints(snapshot).is_some_and(|hp| hp > 0 && hp <= 8)
                        && walk_escape.lock().unwrap().send(client, snapshot)
                }),
            },
            wait: Wait {
                arm: Proof::ArrivedNear {
                    x: ESCAPE_TILE.0,
                    z: ESCAPE_TILE.1,
                    level: ESCAPE_TILE.2,
                    radius: 0,
                },
                budget_ticks: 64,
            },
        });
        steps.push(Step {
            name: "measured quiet interval at the escape tile before Begin",
            kind: StepKind::Await {
                evidence: "own sting observed; escaped and quiet for four full game ticks",
                ready: Box::new(move |snapshot| {
                    hitpoints(snapshot).is_some_and(|hp| hp > 0 && hp <= 8)
                        && escape.lock().unwrap().quiet.ready()
                }),
                summary: None,
            },
            wait: Wait {
                arm: Proof::IngameScene2,
                budget_ticks: 32,
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
    escape: &mut EscapeWitness,
) -> Result<(), String> {
    let near_nettles = snapshot.tile().is_some_and(|(x, z, level)| {
        level == START_TILE.2 && x.abs_diff(START_TILE.0) <= 32 && z.abs_diff(START_TILE.1) <= 32
    });
    if client.ingame
        && near_nettles
        && snapshot.tile() != Some(ESCAPE_TILE)
        && !escape.send(client, snapshot)
    {
        return Err("cleanup normal escape walk was refused".into());
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut last_attempt = None;
    while client.ingame {
        let started = Instant::now();
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        escape.observe(snapshot);
        if !snapshot.ingame() {
            return Ok(());
        }
        // Match the live-map shutdown rule: the server may refuse logout
        // in combat; another request waits sixteen observed game ticks.
        if (!near_nettles || escape.quiet.ready())
            && combat_quiet(snapshot)
            && last_attempt.is_none_or(|tick| snapshot.tick().wrapping_sub(tick) >= 16)
        {
            let interfaces = std::sync::Arc::clone(&client.ifaces);
            if !interact::logout(client, interfaces.as_slice()) {
                return Err("clean logout interface unavailable".into());
            }
            last_attempt = Some(snapshot.tick());
        }
        if Instant::now() >= deadline {
            return Err(
                "clean escape, quiet interval and logout were not observed within ninety seconds"
                    .into(),
            );
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
    let transit_escape = Arc::new(Mutex::new(EscapeWitness::default()));
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
        writeln!(
            log,
            "{}",
            json!({"phase":"fixture","account":name,"action":NETTLE_COMBAT_SEED,
            "scope":"own fresh account defence"})
        )
        .map_err(|error| error.to_string())?;
        if !interact::cheat(&mut client, NETTLE_COMBAT_SEED).is_sent() {
            return Err("fixture own defence seed refused".into());
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nettle fixture combat level before teleport",
            Duration::from_secs(60),
            |state| nettle_stats_ready(combat_level(state), hitpoints(state)),
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
            "nettle fixture target and combat stats",
            Duration::from_secs(60),
            |state| {
                state.tile() == Some(START_TILE)
                    && state.scene_state() == 2
                    && nettle_stats_ready(combat_level(state), hitpoints(state))
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
                steps: steps(inside, Arc::clone(&transit_escape)),
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
        record_scenario_sources(&mut runner, Some(client.revision().as_i32()));
        loop {
            let started = Instant::now();
            client.mainloop();
            host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
            transit_escape.lock().unwrap().observe(&snapshot);
            runner.tick(&mut client);
            if matches!(
                runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                let evidence = runner.evidence().ok_or("terminal runner has no evidence")?;
                // Keep the runner's terminal sample unchanged. Its stat packet
                // can arrive after the fresh hitmark that already made it INVALID.
                client.mainloop();
                host::publish_snapshot(&mut snapshot, &client, pump.drain_client(&client));
                let next = state(&snapshot);
                fs::write(
                    directory.join(format!("{mode}-next-snapshot.json")),
                    next.to_string(),
                )
                .map_err(|error| error.to_string())?;
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
    let mut cleanup_escape = EscapeWitness::default();
    let cleanup = clean_logout(&mut client, &mut snapshot, &mut pump, &mut cleanup_escape);
    fs::write(
        directory.join(format!("{mode}-escape.json")),
        json!({
            "before_begin":transit_escape.lock().unwrap().receipt(),
            "cleanup":cleanup_escape.receipt(),
        })
        .to_string(),
    )
    .map_err(|error| error.to_string())?;
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
        let executable = crate::test_executable::executable().map_err(|error| error.to_string())?;
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
        let escape: Result<Value, String> = fs::read(directory.join(format!("{mode}-escape.json")))
            .map_err(|error| error.to_string())
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()));
        fs::write(directory.join(format!("{mode}-receipt.json")), serde_json::to_vec_pretty(&json!({
            "cell":"interference_377","mode":mode,"head":std::env::var("PARITY_1I_HEAD").ok(),
            "command":[executable.to_string_lossy(),"--ignored","--exact","interference::measured_windows_377","--nocapture","--test-threads=1"],
            "child_environment":{CHILD_MODE:mode},"exit_code":exit_code,"evidence":evidence.as_ref().ok(),
            "fixture_error":evidence.as_ref().err(),"cleanup":cleanup.as_ref().ok(),"cleanup_error":cleanup.as_ref().err(),
            "escape":escape.as_ref().ok(),"escape_error":escape.as_ref().err(),
        })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        let evidence = evidence?;
        let cleanup = cleanup?;
        let escape = escape?;
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
        if mode == "outside" {
            let before = &escape["before_begin"];
            let start = &evidence["interference"]["windows"][0]["start"];
            let quiet_last = before["quiet"]["last_tick"].as_u64();
            if before["action"] != "normal Interactions::walk"
                || before["to"] != json!(ESCAPE_TILE)
                || before["arrived_tick"].as_u64().is_none()
                || before["quiet"]["ready"] != true
                || before["quiet"]["elapsed_ticks"].as_u64().unwrap_or(0) < u64::from(QUIET_TICKS)
                || start["tile"] != json!(ESCAPE_TILE)
                || !quiet_last
                    .zip(start["tick"].as_u64())
                    .is_some_and(|(last, begin)| last <= begin)
            {
                return Err(format!(
                    "outside: missing normal escape and pre-Begin quiet proof: {escape}"
                ));
            }
        }
        let events = evidence["interference"]["events"]
            .as_array()
            .ok_or("missing typed interference events")?;
        let classification = if mode == "outside" {
            "transit"
        } else {
            "interference"
        };
        let next: Value = serde_json::from_slice(
            &fs::read(directory.join(format!("{mode}-next-snapshot.json")))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let observations = events
            .iter()
            .map(|event| nettle_disturbance(event, &next, classification))
            .collect::<Vec<_>>();
        let fixture = if observations.iter().any(Result::is_ok) {
            Ok(())
        } else if observations.contains(&Err(DisturbanceFixture::UnrelatedNpc)) {
            Err(DisturbanceFixture::UnrelatedNpc)
        } else {
            Err(DisturbanceFixture::NettleNotObserved)
        };
        fs::write(
            directory.join(format!("{mode}-classification.json")),
            json!({"outcome":if fixture.is_ok() { expected } else { "INVALID" },
                "fixture_reason":fixture.as_ref().err(),"next_snapshot":next})
            .to_string(),
        )
        .map_err(|error| error.to_string())?;
        fixture.map_err(|reason| format!("{mode}: INVALID fixture {reason:?}"))?;
        println!(
            "{}",
            json!({"cell":"interference_377","mode":mode,"outcome":expected,"exit_code":exit_code})
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
enum DisturbanceFixture {
    UnrelatedNpc,
    NettleNotObserved,
}

fn nettle_disturbance(
    event: &Value,
    next: &Value,
    classification: &str,
) -> Result<(), DisturbanceFixture> {
    if event["targeting_npcs"]
        .as_array()
        .is_some_and(|npcs| !npcs.is_empty())
    {
        return Err(DisturbanceFixture::UnrelatedNpc);
    }
    let previous = &event["previous"];
    let sample = &event["sample"];
    let before = previous["hitpoints"].as_i64();
    let after = sample["hitpoints"].as_i64();
    let stat_damage = before.zip(after).is_some_and(|(a, b)| a - b == 2);
    // Snapshot ticks are observer-local; hitmark loop cycles share the client clock.
    let next_damage = sample["hitpoints"] == previous["hitpoints"]
        && sample["tile"] == next["tile"]
        && sample["hitmarks"]["loop_cycle"]
            .as_i64()
            .zip(next["hitmarks"]["loop_cycle"].as_i64())
            .is_some_and(|(a, b)| b == a + 1)
        && before
            .zip(next["hitpoints"].as_i64())
            .is_some_and(|(a, b)| a - b == 2);
    let fresh_sting = sample["hitmarks"]["marks"].as_array().is_some_and(|marks| {
        marks.iter().any(|mark| {
            mark["value"] == 2
                && mark["cycle"]
                    .as_i64()
                    .zip(previous["hitmark_cycle"].as_i64())
                    .is_some_and(|(new, old)| new > old)
                && mark["cycle"]
                    .as_i64()
                    .zip(sample["hitmarks"]["loop_cycle"].as_i64())
                    .is_some_and(|(expires, cycle)| expires > cycle)
        })
    });
    if event["classification"] == classification
        && previous["session"] == sample["session"]
        && sample["tile"] == json!(START_TILE)
        && ((event["reason"] == "hitpoints-decreased" && stat_damage)
            || (event["reason"] == "fresh-hitmark" && fresh_sting && next_damage))
    {
        Ok(())
    } else {
        Err(DisturbanceFixture::NettleNotObserved)
    }
}

#[test]
fn nettle_fixture_requires_spider_safe_combat_level_and_unchanged_hp() {
    for level in [None, Some(3), Some(4)] {
        assert!(!nettle_stats_ready(level, Some(10)));
    }
    assert!(nettle_stats_ready(Some(5), Some(10)));
    for hp in [None, Some(8), Some(11)] {
        assert!(!nettle_stats_ready(Some(5), hp));
    }
}

#[test]
fn recorded_fresh_hitmark_can_precede_the_stat_packet() {
    // INT-QUEUE-1117: the runner stopped at cycle 207, HP still 10.
    let event = json!({
        "classification":"interference","reason":"fresh-hitmark","targeting_npcs":[],
        "previous":{"session":2,"tick":8,"hitpoints":10,"hitmark_cycle":0},
        "sample":{"session":2,"tick":9,"tile":START_TILE,"hitpoints":10,
            "hitmark_cycle":277,"hitmarks":{"loop_cycle":207,
                "marks":[{"value":2,"kind":1,"cycle":277}]}}
    });
    // The runner and diagnostic GameSnapshots have independent tick counters.
    let next = json!({"tick":5,"tile":START_TILE,"hitpoints":8,
        "hitmarks":{"loop_cycle":208}});
    assert_eq!(nettle_disturbance(&event, &next, "interference"), Ok(()));
    let mut moved_next = next.clone();
    moved_next["tile"] = json!([START_TILE.0 + 1, START_TILE.1, START_TILE.2]);
    assert_eq!(
        nettle_disturbance(&event, &moved_next, "interference"),
        Err(DisturbanceFixture::NettleNotObserved)
    );
    let mut other_session = event.clone();
    other_session["previous"]["session"] = json!(1);
    assert_eq!(
        nettle_disturbance(&other_session, &next, "interference"),
        Err(DisturbanceFixture::NettleNotObserved)
    );
    let mut npc = event.clone();
    npc["targeting_npcs"] = json!([4]);
    assert_eq!(
        nettle_disturbance(&npc, &next, "interference"),
        Err(DisturbanceFixture::UnrelatedNpc)
    );
    let mut no_damage = next.clone();
    no_damage["hitpoints"] = json!(10);
    assert_eq!(
        nettle_disturbance(&event, &no_damage, "interference"),
        Err(DisturbanceFixture::NettleNotObserved)
    );
    let mut stat_first = event.clone();
    stat_first["reason"] = json!("hitpoints-decreased");
    stat_first["sample"]["hitpoints"] = json!(8);
    assert_eq!(
        nettle_disturbance(&stat_first, &Value::Null, "interference"),
        Ok(())
    );
    let mut extra_hit = next.clone();
    extra_hit["hitpoints"] = json!(6);
    assert_eq!(
        nettle_disturbance(&event, &extra_hit, "interference"),
        Err(DisturbanceFixture::NettleNotObserved)
    );
    for cycle in [207, 209] {
        let mut not_next = next.clone();
        not_next["hitmarks"]["loop_cycle"] = json!(cycle);
        assert_eq!(
            nettle_disturbance(&event, &not_next, "interference"),
            Err(DisturbanceFixture::NettleNotObserved)
        );
    }
    let mut stale = event.clone();
    stale["sample"]["hitmarks"]["marks"][0]["cycle"] = json!(0);
    assert_eq!(
        nettle_disturbance(&stale, &next, "interference"),
        Err(DisturbanceFixture::NettleNotObserved)
    );
}

#[test]
fn quiet_interval_requires_consecutive_game_ticks_and_resets_on_combat() {
    let mut quiet = QuietInterval::default();
    for _ in 0..20 {
        quiet.observe(10, true);
    }
    assert_eq!(quiet.elapsed(), 0, "client cycles are not game ticks");
    for tick in 11..=13 {
        quiet.observe(tick, true);
        assert!(!quiet.ready());
    }
    quiet.observe(14, false);
    assert!(!quiet.ready(), "a fresh disturbance resets the interval");
    for tick in 15..=19 {
        quiet.observe(tick, true);
    }
    assert!(quiet.ready());
    quiet.observe(21, true);
    assert!(!quiet.ready(), "unobserved game ticks are not quiet proof");
}

#[test]
fn transit_escape_precedes_begin_but_inside_sting_remains_measured() {
    let outside = steps(false, Arc::new(Mutex::new(EscapeWitness::default())));
    assert!(matches!(outside[0].kind, StepKind::Perform { .. }));
    assert!(matches!(outside[1].kind, StepKind::Perform { .. }));
    assert!(matches!(outside[2].kind, StepKind::Await { .. }));
    assert!(matches!(
        outside[3].kind,
        StepKind::BeginMeasuredWindow { .. }
    ));
    let inside = steps(true, Arc::new(Mutex::new(EscapeWitness::default())));
    assert!(matches!(
        inside[0].kind,
        StepKind::BeginMeasuredWindow { .. }
    ));
    assert!(matches!(inside[1].kind, StepKind::Perform { .. }));
    assert!(matches!(inside[2].kind, StepKind::EndMeasuredWindow { .. }));
}

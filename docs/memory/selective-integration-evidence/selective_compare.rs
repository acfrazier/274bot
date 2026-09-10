//! Temporary common main/selected driver; campaign evidence, never production.
use host_play::{Play, PlayOptions, ScriptStartHandle};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Actor {
    observes: u64,
    generation: Option<u64>,
    updates: u64,
    last_update: Option<Instant>,
    xp: i32,
    runner: Option<scenario::ScenarioRunner>,
    started: bool,
    start_error: Option<String>,
}
struct Card {
    js: String,
    shape: script::LoadShape,
    bag: serde_json::Map<String, Value>,
    siblings: Vec<(String, String)>,
}
type Actors = BTreeMap<String, Arc<Mutex<Actor>>>;
fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}
fn emit(start: Instant, phase: &str, value: Value) {
    println!(
        "COMPARE {}",
        json!({"pid":std::process::id(),"elapsed_s":start.elapsed().as_secs_f64(),"phase":phase,"data":value})
    );
}
fn rows(play: &Play, actors: &Actors) -> Vec<Value> {
    actors.iter().map(|(name, row)| {
        let a = row.lock().unwrap();
        json!({"name":name,"observes":a.observes,"player_updates":a.updates,"xp":a.xp,
            "player_update_age_s":a.last_update.map(|t| t.elapsed().as_secs_f64()),
            "script_state":format!("{:?}",play.script_state(name)),"script_error":play.script_last_error(name),
            "start_error":a.start_error,"runner":a.runner.as_ref().map(|r|format!("{:?}",r.status())),
            "evidence":a.runner.as_ref().and_then(|r|r.evidence()).map(|e|e.to_json())})
    }).collect()
}
fn ready(play: &Play, actors: &Actors) -> bool {
    let statuses = play.statuses();
    let names: BTreeSet<_> = statuses.iter().map(|s| s.username.as_str()).collect();
    statuses.len() == actors.len()
        && names == actors.keys().map(String::as_str).collect()
        && statuses
            .iter()
            .all(|s| s.ingame && s.scene_state == 2 && s.error.is_none())
}
fn exercise(play: &mut Play, actors: &Actors, active: bool, start: Instant) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !ready(play, actors) {
        if Instant::now() >= deadline {
            return Err(format!(
                "not all {} actors ready within 180s: {:?}",
                actors.len(),
                play.statuses()
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
    emit(
        start,
        "ready",
        json!({"n":actors.len(),"actors":rows(play,actors)}),
    );
    let workers = client::io::OnDemand::live_workers_for("127.0.0.1", 43594);
    if workers != 1 {
        return Err(format!("expected 1 OnDemand worker, got {workers}"));
    }
    if let Some(tcp) = host_play::count_tcp_to("127.0.0.1", 43594) {
        if tcp > actors.len() + 1 {
            return Err(format!("TCP count {tcp} exceeds N+1"));
        }
    }
    if active {
        for (name, row) in actors {
            let mut runner = scenario::ScenarioRunner::with_world(
                scenario::get("thiever").unwrap(),
                play.world(),
            );
            runner.set_live_names(&[name.clone()]);
            runner.set_obj_names(play.obj_names());
            row.lock().unwrap().runner = Some(runner);
        }
        // Existing scenario deadlines/watch predicates are unchanged.
        // Also bound failure if a disconnected worker ceases delivering callbacks.
        let proof_deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if Instant::now() >= proof_deadline {
                return Err("not all actors proved Thiever progress within 180s".into());
            }
            let mut all_passed = true;
            for (name, row) in actors {
                let a = row.lock().unwrap();
                if let Some(error) = &a.start_error {
                    return Err(format!("{name} start: {error}"));
                }
                if let Some(error) = play.script_last_error(name) {
                    return Err(format!("{name} script: {error}"));
                }
                match a.runner.as_ref().unwrap().status() {
                    scenario::RunnerStatus::Passed => {}
                    scenario::RunnerStatus::Failed(error) => {
                        return Err(format!("{name} proof: {error}"))
                    }
                    _ => all_passed = false,
                }
            }
            if all_passed {
                break;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    if !ready(play, actors) {
        return Err("actor readiness lost before observation".into());
    }
    let before: BTreeMap<_, _> = actors
        .iter()
        .map(|(name, row)| {
            let a = row.lock().unwrap();
            (name.clone(), (a.updates, a.xp))
        })
        .collect();
    emit(
        start,
        "observation_start",
        json!({"n":actors.len(),"active":active,"actors":rows(play,actors)}),
    );
    let observe_start = Instant::now();
    while observe_start.elapsed() < Duration::from_secs(60) {
        if !ready(play, actors) {
            return Err("actor readiness lost during observation".into());
        }
        for (name, row) in actors {
            let a = row.lock().unwrap();
            if a.last_update
                .is_none_or(|t| t.elapsed() > Duration::from_secs(10))
            {
                return Err(format!("{name}: no player update within 10s"));
            }
            if active {
                if play.script_state(name) != script::RunState::Running {
                    return Err(format!("{name}: script is not Running"));
                }
                if let Some(error) = play.script_last_error(name) {
                    return Err(format!("{name}: {error}"));
                }
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    emit(
        start,
        "observation_end",
        json!({"n":actors.len(),"active":active,"duration_s":observe_start.elapsed().as_secs_f64(),"actors":rows(play,actors)}),
    );
    for (name, row) in actors {
        let a = row.lock().unwrap();
        if a.updates <= before[name].0 {
            return Err(format!("{name}: no player-update progress"));
        }
        if active && a.xp <= before[name].1 {
            return Err(format!("{name}: no thieving XP gain during observation"));
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1 and a local engine; campaign comparison only"]
fn common_session() {
    assert_eq!(required("LIVE"), "1");
    assert_eq!(required("BOT_TARGET"), "local");
    let n: usize = required("COMPARE_N").parse().expect("COMPARE_N integer");
    assert!(matches!(n, 1 | 32), "COMPARE_N must be 1 or 32");
    let active = match required("COMPARE_WORKLOAD").as_str() {
        "thiever" => true,
        "idle" => false,
        _ => panic!("unknown workload"),
    };
    let dir = PathBuf::from(required("COMPARE_RUN_DIR"));
    assert_eq!(
        PathBuf::from(required("HOME")),
        dir.join("home"),
        "HOME must be this run's disposable home"
    );
    std::fs::create_dir_all(dir.join("home/.274bot")).unwrap();
    let loadouts = json!([{"name":"Food","worn":[],"carry":["Lobster"]}]);
    std::fs::write(
        dir.join("home/.274bot/loadouts.json"),
        serde_json::to_vec_pretty(&loadouts).unwrap(),
    )
    .unwrap();
    let _pack = required("NAV_PACK");
    let card = if active {
        let root = PathBuf::from(required("RS2B0T"));
        let mut library = script::JsLibrary::new(dir.join("scripts.json"));
        library
            .register_rs2b0t(&root, &dir.join("rs2b0t-path"))
            .unwrap();
        library
            .ensure_js(script::ScriptSource::Catalog, "Thiever")
            .unwrap();
        let card = library
            .get(script::ScriptSource::Catalog, "Thiever")
            .unwrap()
            .clone();
        let scenario = scenario::get("thiever").unwrap();
        let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject);
        let bag = script::settings_store::merge_bag(
            &card.settings_schema,
            &serde_json::Map::new(),
            inject.as_ref(),
        );
        let siblings = script::resolve_sibling_modules(
            &card.path,
            &card.origin,
            library.cache(),
            script::CacheMeta {
                kind: card.kind,
                source: card.source,
                shape: None,
            },
        )
        .unwrap();
        Some(Card {
            js: card.js,
            shape: card.shape,
            bag,
            siblings,
        })
    } else {
        None
    };
    let names = host_play::mint_live_names(n);
    let actors: Arc<Actors> = Arc::new(
        names
            .iter()
            .map(|name| (name.clone(), Arc::new(Mutex::new(Actor::default()))))
            .collect(),
    );
    assert_eq!(actors.len(), n);
    let callback_actors = actors.clone();
    let handle: Arc<Mutex<Option<ScriptStartHandle>>> = Arc::new(Mutex::new(None));
    let callback_handle = handle.clone();
    let options = PlayOptions {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: required("COMPARE_CACHE"),
        lowmem: true,
        mainland: active,
    };
    let start = Instant::now();
    let mut play = host_play::run_with_io(
        &options,
        vec![],
        |_| (None, None),
        move |c, name, hold| {
            c.set_draw(false);
            let mut a = callback_actors[name].lock().unwrap();
            a.observes += 1;
            if a.generation != Some(c.gens.player) {
                if a.generation.is_some() {
                    a.updates += 1;
                }
                a.generation = Some(c.gens.player);
                a.last_update = Some(Instant::now());
            }
            a.xp = c.stat_xp[17];
            if a.runner.as_ref().is_some_and(|r| r.on_start_script()) && !a.started {
                let card = card.as_ref().unwrap();
                match callback_handle
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .start_load(
                        name,
                        card.js.clone(),
                        card.shape,
                        Some(card.bag.clone()),
                        card.siblings.clone(),
                    ) {
                    Ok(()) => a.started = true,
                    Err(error) => {
                        a.start_error = Some(error);
                        return;
                    }
                }
            }
            if let Some(runner) = a.runner.as_mut() {
                runner.tick_with_hold(c, hold);
            }
        },
    );
    *handle.lock().unwrap() = Some(play.script_start_handle());
    let mut vault =
        vault::Vault::create(&dir.join("vault"), &host_play::live_vault_passphrase()).unwrap();
    for (i, (username, password)) in host_play::mint_live_entries(&names).into_iter().enumerate() {
        let profile = vault::Profile {
            username,
            password,
            uid: 274_000_200 + i as i32,
            settings: vault::ProfileSettings::default(),
        };
        vault.upsert(profile.clone()).unwrap();
        play.spawn_slot(profile, None, None, None);
    }
    emit(
        start,
        "spawned",
        json!({"n":n,"active":active,"names":names,"loadouts":loadouts}),
    );
    let outcome = exercise(&mut play, &actors, active, start);
    if outcome.is_err() {
        emit(
            start,
            "failure",
            json!({"error":outcome.as_ref().err(),"actors":rows(&play,&actors)}),
        );
    }
    for name in &names {
        play.script_stop(name);
        play.stop_slot(name);
    }
    if !play.statuses().is_empty() {
        eprintln!("FAIL: slots retained after Stop");
        std::process::exit(1);
    }
    emit(start, "stopped", json!({"n":0}));
    if let Err(error) = outcome {
        eprintln!("FAIL: common_session: {error}");
        std::process::exit(1);
    }
    println!("PASS: common_session n={n} active={active}");
}

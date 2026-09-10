//! Controlled step-5 world/guardian cells.
//!
//! This deliberately uses `SharedClientTemplate::prepare_client` rather than
//! the production slot launcher: it binds the selected local profile and
//! template while preserving the revision-289 operation gate. Root runs one
//! explicit cell per process with LIVE=1.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use host::{Guardian, Pump};
use host_play::{ProfileOptions, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use vault::ProfileSettings;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn selected() -> (Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>) {
    let revision = required("WORLD_REVISION");
    assert!(matches!(revision.as_str(), "274" | "289"));
    let nav_pack = PathBuf::from(required("WORLD_NAV_PACK"));
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if std::env::var("WORLD_REVISION").as_deref() == Ok("289") {
            44594
        } else {
            43594
        }),
        http_port: Some(if std::env::var("WORLD_REVISION").as_deref() == Ok("289") {
            1080
        } else {
            80
        }),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("WORLD_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("profile resolve")
        .bind()
        .expect("profile bind");
    let template =
        SharedClientTemplate::load(Arc::clone(&profile)).expect("selected template/world load");
    assert!(
        template.world().is_some(),
        "selected template.world() is required"
    );
    println!(
        "{{\"revision\":{},\"profile\":{:?},\"cache_id\":{:?},\"nav_pack\":{:?}}}",
        profile.revision().as_i32(),
        profile.label(),
        profile.cache_id(),
        profile.nav_pack()
    );
    (profile, template)
}

fn run_nav(
    case: &str,
    template: Arc<SharedClientTemplate>,
    profile: Arc<host_play::ServerProfile>,
) {
    let scenario = scenario::get(case).expect("registered navigation scenario");
    let n = scenario.seed.profiles.len();
    let names: Vec<String> = (0..n)
        .map(|i| format!("w{}{}", profile.revision().as_i32(), i))
        .collect();
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(&names);
    runner.set_scene_settle(Duration::ZERO);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let mut clients = Vec::new();
    let mut snapshots = Vec::new();
    let mut pumps = Vec::new();
    let mut guardians = Vec::new();
    let settings = ProfileSettings::default();
    for (i, name) in names.iter().enumerate() {
        let mut c = template
            .prepare_client(274000000 + i as i32, true)
            .expect("prepare selected client");
        c.draw = false;
        c.maininit();
        assert!(
            interact::login(&mut c, name, name, false),
            "login failed: {} {}",
            c.login_mes1,
            c.login_mes2
        );
        clients.push(c);
        snapshots.push(GameSnapshot::new());
        pumps.push(Pump::new());
        guardians.push(Guardian::new());
    }
    let deadline = Instant::now() + runner.deadline().max(Duration::from_secs(180));
    loop {
        for i in 0..clients.len() {
            clients[i].mainloop();
            let drained = pumps[i].drain_client(&clients[i]);
            host::publish_snapshot(&mut snapshots[i], &clients[i], drained);
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let _ = guardians[i].tick(&mut clients[i], &snapshots[i], &settings, now_ms, None);
            if i == 0 {
                runner.tick(&mut clients[i]);
            }
        }
        match runner.status() {
            RunnerStatus::Passed => {
                println!("PASS: {case}: {:?}", runner.evidence());
                break;
            }
            RunnerStatus::Failed(msg) => panic!("{case}: {msg}; evidence={:?}", runner.evidence()),
            _ => assert!(
                Instant::now() < deadline,
                "{case}: runner timeout; evidence={:?}",
                runner.evidence()
            ),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for c in &mut clients {
        if c.ingame {
            c.logout();
        }
    }
}

fn run_guardian(template: Arc<SharedClientTemplate>, profile: Arc<host_play::ServerProfile>) {
    let name = format!("g{}", profile.revision().as_i32());
    let mut client = template
        .prepare_client(289000001, true)
        .expect("prepare selected client");
    client.draw = false;
    client.maininit();
    assert!(interact::login(&mut client, &name, &name, false));
    let settings = ProfileSettings {
        random_events: true,
        lamp_auto: true,
        lamp_skill: "strength".into(),
        ..ProfileSettings::default()
    };
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let mut guardian = Guardian::new();
    assert!(
        interact::cheat(&mut client, "give lamp"),
        "local give command refused"
    );
    let end = Instant::now() + Duration::from_secs(30);
    let mut saw_lamp = false;
    let mut saw_hold = false;
    let mut saw_resolution = false;
    while Instant::now() < end {
        client.mainloop();
        let drained = pump.drain_client(&client);
        host::publish_snapshot(&mut snapshot, &client, drained);
        saw_lamp |= snapshot.inventory().iter().any(|item| {
            item.def
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("lamp"))
        });
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let status = guardian.tick(&mut client, &snapshot, &settings, now_ms, None);
        saw_hold |= status.hold && status.ours;
        saw_resolution |= saw_hold
            && !status.hold
            && !snapshot.inventory().iter().any(|item| {
                item.def
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("lamp"))
            });
        if saw_lamp && saw_hold && saw_resolution {
            println!("PASS: guardian_lamp: hold/resolution observed");
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("guardian_lamp: expected inventory lamp, host hold, and consumption (lamp={saw_lamp}, hold={saw_hold}, resolved={saw_resolution})");
}

#[test]
#[ignore = "requires LIVE=1, WORLD_REVISION, WORLD_CASE, WORLD_NAV_PACK and local engine"]
fn world_boundary_live() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    let case = required("WORLD_CASE");
    assert!(matches!(
        case.as_str(),
        "nav_full" | "nav_door" | "guardian_lamp"
    ));
    let (profile, template) = selected();
    match case.as_str() {
        "nav_full" | "nav_door" => run_nav(&case, template, profile),
        "guardian_lamp" => run_guardian(template, profile),
        _ => unreachable!(),
    }
}

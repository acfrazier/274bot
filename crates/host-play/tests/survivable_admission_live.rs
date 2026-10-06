//! S2b advisory admission on real revision-289 content. No poison-clear
//! observer, preflight movement or survivability claim is manufactured here.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host::Pump;
use host_play::admission::{self, Admission, PoisonState, RiskInput};
use host_play::walk_map::{ActionError, ActionKind, FocusToken, MapContext, MapModel};
use host_play::{ProfileOptions, SharedClientTemplate, SlotArm, WalkArms, WalkGlobals};
use nav::map::identity::Digest;
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use script::combat::risk::{UnknownWhy, Verdict};
use script::native::{RiskPolicy, WalkBit, WalkOptions, WalkPermissions, WalkRefusal};
use serde_json::{json, Value};

struct NavLogs(Mutex<Vec<(String, Option<u32>, String)>>);

static NAV_LOGS: NavLogs = NavLogs(Mutex::new(Vec::new()));
static INSTALL_LOGS: std::sync::Once = std::sync::Once::new();

impl api::hostlog::Sink for NavLogs {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if matches!(record.source, api::hostlog::Source::Nav) {
            self.0.lock().expect("nav logs").push((
                record.slot.unwrap_or("?").to_owned(),
                record.tick,
                record.message.to_owned(),
            ));
        }
    }
}

fn install_nav_logs() {
    INSTALL_LOGS.call_once(|| assert!(api::hostlog::install_sink(&NAV_LOGS)));
}

fn nav_logs(slot: &str) -> Vec<Value> {
    NAV_LOGS
        .0
        .lock()
        .expect("nav logs")
        .iter()
        .filter(|(name, _, _)| name == slot)
        .map(|(slot, tick, message)| json!({"slot": slot, "tick": tick, "message": message}))
        .collect()
}

fn observed_hitmarks(client: &Client) -> Value {
    client.local_player.as_ref().map_or(Value::Null, |player| {
        json!({"client_cycle": client.loop_cycle,
            "values": player.entity.damage_values, "types": player.entity.damage_types,
            "expires_client_cycle": player.entity.damage_cycles})
    })
}

fn tile(tile: WorldTile) -> Tile {
    Tile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn pump_once(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
    if snapshot.ingame() && snapshot.attached() {
        let input = admission::capture(snapshot, false, PoisonState::default(), false);
        if input.hp_max > 0 {
            assert_ne!(
                input.hp,
                0,
                "death fails the S2b live cell at {:?}",
                snapshot.tile()
            );
        }
    }
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    ready: impl Fn(&GameSnapshot) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "live readiness deadline at {:?}, scene={}, inventory_size={}, captured={:?}",
            snapshot.tile(),
            snapshot.scene_state(),
            snapshot.inventory_size(),
            admission::capture(snapshot, false, PoisonState::default(), false)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn mainland_relog_runner() -> ScenarioRunner {
    // A real clean relog posts the mainland inventory/side-tab observations.
    // It neither walks nor supplies poison-clear evidence.
    let proof = Proof::SideTabAvailable { index: 3 };
    ScenarioRunner::with_world(
        Scenario {
            name: "survivable_admission_tutorial_relog",
            seed: Seed {
                profiles: Vec::new(),
                mainland: false,
            },
            steps: vec![Step {
                name: "clean relog after tutorial skip",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: proof,
                    budget_ticks: 600,
                },
            }],
            proof,
            companions: Vec::new(),
            settings: ScenarioSettings {
                deadline: Duration::from_secs(90),
                require_mainland_base: false,
                ..ScenarioSettings::default()
            },
        },
        None,
    )
}

// Pick a short actual endpoint crossing using only the selected pack's real
// geometry and collision. The copied input's position is a fixture coordinate,
// not live evidence. The receipt is assessed again from the actual seeded scene.
fn endpoint_fixture(
    world: &NavWorld,
    state: &WorldState,
    input: RiskInput,
) -> (WorldTile, WorldTile) {
    let zones = world.graph.zones.as_ref().expect("selected pack has zones");
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected revision-289 facts");
    let ice_warrior = selected
        .npc_by_config("icewarrior")
        .expect("selected Ice warrior identity")
        .id;
    for zone in zones.zones().iter().filter(|zone| {
        zones.kinds()[usize::from(zone.kind)].npc_id == ice_warrior
            && (3000..3080).contains(&zone.spawn_x)
            && zone.spawn_z > 9500
    }) {
        let level = i32::from(zone.level);
        let pairs = [
            (zone.min_x - 3, zone.spawn_z, zone.min_x + 1, zone.spawn_z),
            (zone.max_x + 3, zone.spawn_z, zone.max_x - 1, zone.spawn_z),
            (zone.spawn_x, zone.min_z - 3, zone.spawn_x, zone.min_z + 1),
            (zone.spawn_x, zone.max_z + 3, zone.spawn_x, zone.max_z - 1),
        ];
        for (x, z, dx, dz) in pairs {
            let origin = WorldTile { x, z, level };
            let destination = WorldTile {
                x: dx,
                z: dz,
                level,
            };
            if !world.collision.standable(origin)
                || !world.collision.standable(destination)
                || zones.at(origin).next().is_some()
                || zones.at(destination).next().is_none()
            {
                continue;
            }
            let options = FindOptions::default();
            let Ok(route) = nav::router::find_with(
                &world.collision,
                &world.graph,
                origin,
                destination,
                options,
                state,
            ) else {
                continue;
            };
            if route.ticks > 20.0 {
                continue;
            }
            let context = Admission::manual(
                options,
                RiskInput {
                    pos: origin,
                    ..input
                },
                0,
                WalkGlobals::default(),
            );
            let assessment = admission::assess(&route, world, &context);
            if assessment.verdict == Verdict::Unknown(UnknownWhy::Poison)
                && !assessment.plan.intervals.is_empty()
            {
                return (origin, destination);
            }
        }
    }
    panic!("selected real pack has no short ungranted Ice warrior endpoint crossing");
}

fn capture(client: &mut Client, directory: &Path, label: &str, receipt: &Value) -> PathBuf {
    let was_draw = client.draw;
    client.set_draw(true);
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let output = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = output else {
        panic!("BOT_CPU=1 must produce the real client CPU framebuffer");
    };
    let rgba: Vec<u8> = pixels
        .pixels
        .iter()
        .flat_map(|pixel| {
            [
                ((pixel >> 16) & 0xff) as u8,
                ((pixel >> 8) & 0xff) as u8,
                (pixel & 0xff) as u8,
                u8::MAX,
            ]
        })
        .collect();
    scenario::shot::write_shot(
        directory,
        label,
        &rgba,
        pixels.width as u32,
        pixels.height as u32,
        &serde_json::to_string_pretty(receipt).expect("serialize receipt"),
    )
    .expect("write PNG and observed-scene sidecar")
}

fn selected_template() -> (Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>) {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(PathBuf::from(
            std::env::var_os("WORLD_ENGINE_DIR").expect("engine path"),
        )),
        cache_dir: Some(PathBuf::from(
            std::env::var_os("BOT_CACHE_DIR").expect("per-run cache clone"),
        )),
        unpack_dir: Some(PathBuf::from(
            std::env::var_os("BOT_UNPACK_DIR").expect("per-run unpack path"),
        )),
        nav_pack: std::env::var_os("WORLD_NAV_PACK").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .expect("resolve isolated local profile")
        .bind()
        .expect("bind profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("load real content");
    (profile, template)
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, WORLD_ENGINE_DIR, BOT_CACHE_DIR (per-run clone), BOT_UNPACK_DIR, LIVE_EVIDENCE_DIR; Engine A 44594/1080"]
fn real_unknown_poison_explicit_forbid_then_opt_in_and_default_escape() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    install_nav_logs();
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("owned account prefix");
    assert!(!prefix.is_empty() && prefix.len() <= 4);
    let directory =
        PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("evidence directory"));
    std::fs::create_dir_all(&directory).expect("create evidence directory");
    let (profile, template) = selected_template();
    let world = template.world().expect("selected actual nav world");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let name = format!("{prefix}{}", serial % 1_000_000_000);
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .expect("prepare actual client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, &name, &name, false));
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.ingame() && s.attached() && s.scene_state() == 2
    });
    // Account-only initial state. No NPC, collision, map or shared content edits.
    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.chat_lines()
            .iter()
            .any(|line| line.text.contains("get tutorial: 1000"))
    });
    let mut relogger = mainland_relog_runner();
    let mut login_sent = false;
    let relog_deadline = Instant::now() + Duration::from_secs(90);
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        if !snapshot.ingame() && !login_sent {
            assert!(interact::login(&mut client, &name, &name, false));
            login_sent = true;
        }
        relogger.tick(&mut client);
        match relogger.status() {
            RunnerStatus::Passed => break,
            RunnerStatus::Failed(error) => panic!("tutorial relog failed: {error}"),
            RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
        }
        assert!(
            Instant::now() < relog_deadline,
            "manual clean relog deadline"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for command in ["setstat hitpoints 99", "setstat defence 99"] {
        assert!(interact::cheat(&mut client, command).is_sent());
    }
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        let input = admission::capture(s, profile.map_members(), PoisonState::default(), false);
        s.scene_state() == 2 && input.hp == 99 && !input.missing_facts
    });
    let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let input = admission::capture(&snapshot, state.map_members, PoisonState::default(), false);
    let (origin, destination) = endpoint_fixture(&world, &state, input);
    interact::seed_at(&mut client, origin.level, origin.x, origin.z);
    wait_for(&mut client, &mut snapshot, &mut pump, |s| {
        s.scene_state() == 2 && s.tile() == Some((origin.x, origin.z, origin.level))
    });
    let identity = profile.nav_identity().expect("bound navigation identity");
    let context = MapContext {
        focus: Some(FocusToken::capture(&SlotArm::new(1, false))),
        nav: Digest::from_hex(&identity.nav_sha256).expect("nav digest"),
        overlay: None,
        generation: 1,
    };
    let mut model = MapModel::default();
    model.bind(context);
    assert_eq!(
        model.select_tile(&world, tile(destination)),
        Some(tile(destination))
    );
    let arms = WalkArms::default();
    let state = WorldState::from_snapshot(&snapshot).with_map_members(profile.map_members());
    let input = admission::capture(&snapshot, state.map_members, PoisonState::default(), false);
    let strict_options = FindOptions::default();
    let strict = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(tile(origin)),
            strict_options,
        )
        .expect("strict map command");
    // The held default must retain the router's endpoint completion. Assess it
    // honestly, but do not follow it before exercising the explicit refusal.
    let default_walk = strict
        .walk_on(
            &world,
            &context,
            &name,
            &state,
            &[],
            Admission::manual(strict_options, input, 1, WalkGlobals::default()),
            &arms,
        )
        .expect("held default preserves endpoint admission");
    assert_eq!(
        default_walk.assessment.verdict,
        Verdict::Unknown(UnknownWhy::Poison)
    );
    let arm = Arc::clone(&arms.lock().expect("arms")[&name]);
    assert!(arm.lock().expect("arm").route.is_some());
    assert_eq!(snapshot.tile(), Some((origin.x, origin.z, origin.level)));
    assert_eq!(
        model.select_tile(&world, tile(destination)),
        Some(tile(destination))
    );
    let strict = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(tile(origin)),
            strict_options,
        )
        .expect("explicit forbid map command");
    // This caller explicitly requests Forbid. Resolve its enforcement from the
    // same global/tri-state contract as native requests; this is not default UI.
    let forbid = WalkOptions {
        allow_danger_zones: WalkBit::Forbid,
        ..WalkOptions::default()
    };
    let globals = WalkGlobals::default();
    let strict_admission = Admission {
        policy: globals.risk_policy(WalkPermissions::default(), forbid),
        enforce: globals.enforces_risk(WalkPermissions::default(), forbid),
        ..Admission::manual(strict_options, input, 2, globals)
    };
    assert_eq!(strict_admission.policy, RiskPolicy::Avoid);
    let refusal = strict
        .walk_on(
            &world,
            &context,
            &name,
            &state,
            &[],
            strict_admission,
            &arms,
        )
        .expect_err("explicit Forbid endpoint must refuse Unknown poison");
    let ActionError::RiskRefused {
        refusal: WalkRefusal::Unknown(UnknownWhy::Poison),
        detail,
    } = refusal
    else {
        panic!("expected visible Unknown(Poison), got {refusal:?}");
    };
    let assessment = Arc::clone(
        arm.lock()
            .expect("arm")
            .last_assessment
            .as_ref()
            .expect("retained refusal assessment"),
    );
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
    assert!(!assessment.plan.intervals.is_empty());
    assert!(arm.lock().expect("arm").route.is_none());
    assert_eq!(snapshot.tile(), Some((origin.x, origin.z, origin.level)));
    assert_eq!(detail, assessment.reason.as_ref());
    let mut receipt = json!({"cell": "S2b-real-admission", "account": name,
        "engine": "127.0.0.1:44594", "nav": identity.nav_sha256,
        "origin": [origin.x, origin.z, origin.level], "destination": [destination.x, destination.z, destination.level],
        "refused": {"verdict": format!("{:?}", assessment.verdict), "reason": detail,
            "route_armed": false, "crossings": assessment.plan.intervals.len(), "hp": input.hp,
            "poison": format!("{:?}", input.poison)},
        "held_default": {"admitted": true, "route_armed": true,
            "verdict": format!("{:?}", default_walk.assessment.verdict),
            "movement_before_explicit_forbid": false},
        "refusal_policy": "explicit per-walk Forbid (not default/inherited admission)",
        "poison_scope": "S2b poison remains Unknown for the entire session; only explicit enforcing crossings refuse, held defaults are unchanged.",
        "deaths": 0});
    receipt["refused_png"] = json!(capture(
        &mut client,
        &directory,
        "endpoint-refused",
        &receipt
    ));
    // The actual map checkbox is a request-local all-zone Allow; globals remain
    // at the held middle default. Its assessment remains honestly Unknown.
    assert_eq!(
        model.select_tile(&world, tile(destination)),
        Some(tile(destination))
    );
    let allow_options = FindOptions {
        zones: nav::zones::ZoneExempt::all(),
        ..strict_options
    };
    let command = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(tile(origin)),
            allow_options,
        )
        .expect("opt-in map command");
    let walked = command
        .walk_on(
            &world,
            &context,
            &name,
            &state,
            &[],
            Admission::manual(allow_options, input, 2, WalkGlobals::default()),
            &arms,
        )
        .expect("explicit opt-in proceeds");
    let assessment = walked.assessment;
    assert_eq!(assessment.verdict, Verdict::Unknown(UnknownWhy::Poison));
    receipt["opt_in"] = json!({"verdict": format!("{:?}", assessment.verdict),
        "reason": assessment.reason, "crossings": assessment.plan.intervals.len(), "ticks": walked.route.ticks});
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_tick = None;
    let mut path = Vec::new();
    let mut hitmarks = Vec::new();
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        if Some(snapshot.tick()) != last_tick {
            last_tick = Some(snapshot.tick());
            hitmarks.push(json!({"tick": snapshot.tick(), "hitmarks": observed_hitmarks(&client)}));
            if let Some(here) = snapshot.tile() {
                if path.last() != Some(&here) {
                    path.push(here);
                }
                host_play::step_walk_arm_follow(
                    &mut client,
                    &snapshot,
                    &mut arm.lock().expect("arm"),
                    Some(&world),
                    here,
                    state.map_members,
                    Some(&name),
                );
                if arm.lock().expect("arm").route.is_none() {
                    break;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "real opt-in crossing deadline at {:?}",
            snapshot.tile()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    receipt["hitmarks"] = json!(hitmarks);
    receipt["hostlog"] = json!(nav_logs(&name));
    assert!(
        receipt["hostlog"]
            .as_array()
            .expect("nav logs")
            .iter()
            .any(|line| {
                line["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("risk=Unknown(Poison)"))
            }),
        "real risk hostlog line must be recorded: {receipt}"
    );
    let final_input =
        admission::capture(&snapshot, state.map_members, PoisonState::default(), false);
    receipt["observed_path"] = json!(path);
    receipt["final_tile"] = json!(snapshot.tile());
    receipt["final_hp"] = json!(final_input.hp);
    receipt["arrived"] =
        json!(snapshot.tile() == Some((destination.x, destination.z, destination.level)));
    receipt["arrival_png"] = json!(capture(
        &mut client,
        &directory,
        "explicit-opt-in-arrived",
        &receipt
    ));
    // Added live cell: start inside the actual endpoint zone and leave it using
    // untouched defaults. No synthetic Clear, preflight or explicit grant.
    assert!(world
        .graph
        .zones
        .as_ref()
        .unwrap()
        .at(destination)
        .next()
        .is_some());
    assert_eq!(model.select_tile(&world, tile(origin)), Some(tile(origin)));
    let command = model
        .confirm(
            ActionKind::Walk,
            &context,
            Some(tile(destination)),
            strict_options,
        )
        .expect("default escape map command");
    let escape_input =
        admission::capture(&snapshot, state.map_members, PoisonState::default(), false);
    let escaped = command
        .walk_on(
            &world,
            &context,
            &name,
            &state,
            &[],
            Admission::manual(strict_options, escape_input, 3, WalkGlobals::default()),
            &arms,
        )
        .expect("default walk out of a real zone is admitted");
    assert!(escaped.assessment.plan.complete);
    assert!(escaped.assessment.plan.crossings.is_empty());
    assert!(escaped
        .assessment
        .plan
        .intervals
        .iter()
        .any(|row| row.escaping()));
    receipt["default_escape"] = json!({"admitted": true, "explicit_grant": false,
        "origin": [destination.x, destination.z, destination.level],
        "destination": [origin.x, origin.z, origin.level],
        "entering_crossings": 0, "escape_intervals": escaped.assessment.plan.intervals.len(),
        "verdict": format!("{:?}", escaped.assessment.verdict),
        "poison": format!("{:?}", escape_input.poison)});
    let escape_deadline = Instant::now() + Duration::from_secs(60);
    let mut last_tick = None;
    let mut escape_path = Vec::new();
    loop {
        pump_once(&mut client, &mut snapshot, &mut pump);
        if Some(snapshot.tick()) != last_tick {
            last_tick = Some(snapshot.tick());
            if let Some(here) = snapshot.tile() {
                if escape_path.last() != Some(&here) {
                    escape_path.push(here);
                }
                host_play::step_walk_arm_follow(
                    &mut client,
                    &snapshot,
                    &mut arm.lock().expect("arm"),
                    Some(&world),
                    here,
                    state.map_members,
                    Some(&name),
                );
                if arm.lock().expect("arm").route.is_none() {
                    break;
                }
            }
        }
        assert!(
            Instant::now() < escape_deadline,
            "default escape deadline at {:?}",
            snapshot.tile()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let escape_final =
        admission::capture(&snapshot, state.map_members, PoisonState::default(), false);
    assert_eq!(snapshot.tile(), Some((origin.x, origin.z, origin.level)));
    assert!(escape_path.len() > 1);
    assert!(escape_final.hp > 0);
    receipt["default_escape"]["arrived"] = json!(true);
    receipt["default_escape"]["observed_path"] = json!(escape_path);
    receipt["default_escape"]["final_hp"] = json!(escape_final.hp);
    receipt["default_escape"]["arrival_png"] = json!(capture(
        &mut client,
        &directory,
        "default-real-zone-escape-arrived",
        &receipt,
    ));
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("receipt JSON"),
    )
    .expect("write final evidence");
    client.logout();
    assert_eq!(
        receipt["arrived"], true,
        "real common-host follow must reach the exact crossing endpoint: {receipt}"
    );
    assert!(final_input.hp > 0);
    assert!(
        path.len() > 1,
        "real movement must be observed, not just routing"
    );
}

fn compat_corridor_fixture(
    world: &NavWorld,
    state: &WorldState,
    input: RiskInput,
) -> (WorldTile, WorldTile) {
    // Canonical raw Ice Dungeon ladder -> blurite-side geometry is also
    // exercised by nav/tests/ice_dungeon_real_pack.rs. No movement preflight.
    let origin = WorldTile {
        x: 3008,
        z: 9551,
        level: 0,
    };
    let destination = WorldTile {
        x: 3060,
        z: 9580,
        level: 0,
    };
    assert!(world.collision.standable(origin));
    assert!(world.collision.standable(destination));
    let options = FindOptions {
        zones: nav::zones::ZoneExempt::all(),
        ..FindOptions::default()
    };
    let route = nav::router::find_with(
        &world.collision,
        &world.graph,
        origin,
        destination,
        options,
        state,
    )
    .expect("real Ice Dungeon walking corridor");
    assert!(route
        .legs
        .iter()
        .all(|leg| matches!(leg, nav::router::Leg::Walk { .. })));
    let assessment = admission::assess(
        &route,
        world,
        &Admission::manual(
            options,
            RiskInput {
                pos: origin,
                ..input
            },
            0,
            WalkGlobals::default(),
        ),
    );
    assert!(
        !assessment.plan.intervals.is_empty(),
        "real corridor must cross danger"
    );
    (origin, destination)
}

struct CompatLive {
    snapshot: GameSnapshot,
    world: Arc<NavWorld>,
    members: bool,
    initial_hp: u8,
    base_hp: u8,
    phase: u8,
    relogger: ScenarioRunner,
    endpoints: Option<(WorldTile, WorldTile)>,
    baseline: Option<Value>,
    frames: Vec<Value>,
    ready: bool,
    failure: Option<String>,
    shot: Option<(PathBuf, Value)>,
    captured: bool,
}

impl CompatLive {
    fn frame(&mut self, client: &mut Client) {
        self.snapshot.rebuild(client);
        let snapshot = &self.snapshot;
        if self.phase == 2 {
            self.relogger.tick(client);
            match self.relogger.status() {
                RunnerStatus::Passed => {
                    for command in [
                        format!("setstat hitpoints {}", self.base_hp),
                        "setstat defence 99".into(),
                        "give lobster 6".into(),
                    ] {
                        assert!(interact::cheat(client, &command).is_sent());
                    }
                    self.phase = 3;
                }
                RunnerStatus::Failed(error) => {
                    self.failure = Some(format!("tutorial relog failed: {error}"));
                    return;
                }
                RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
            }
        }
        if !snapshot.ingame() || !snapshot.attached() || snapshot.scene_state() != 2 {
            return;
        }
        let input = admission::capture(snapshot, self.members, PoisonState::default(), false);
        if input.hp_max > 0 && input.hp == 0 {
            self.failure = Some(format!("compat live death at {:?}", snapshot.tile()));
            return;
        }
        let food = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.name.as_deref() == Some("Lobster"))
            .map(|item| item.count)
            .sum::<i32>();
        match self.phase {
            0 => {
                interact::mainland_hop(client);
                assert!(interact::cheat(client, "getvar tutorial").is_sent());
                self.phase = 1;
            }
            1 if snapshot
                .chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000")) =>
            {
                self.phase = 2;
            }
            3 if input.hp == self.base_hp && food == 6 && !input.missing_facts => {
                // Real account-only debug damage. No client/snapshot HP replacement.
                assert!(interact::cheat(
                    client,
                    &format!("~hit {}", self.base_hp - self.initial_hp)
                )
                .is_sent());
                self.phase = 4;
            }
            4 if input.hp == self.initial_hp => {
                let state = WorldState::from_snapshot(snapshot).with_map_members(self.members);
                let endpoints = compat_corridor_fixture(&self.world, &state, input);
                interact::seed_at(client, endpoints.0.level, endpoints.0.x, endpoints.0.z);
                self.endpoints = Some(endpoints);
                self.phase = 5;
            }
            5 if self.endpoints.is_some_and(|(origin, _)| {
                snapshot.tile() == Some((origin.x, origin.z, origin.level))
            }) =>
            {
                self.baseline = Some(json!({"hp": input.hp, "hp_max": input.hp_max,
                    "food": food, "tile": snapshot.tile()}));
                self.ready = true;
                self.phase = 6;
            }
            6 => {
                let value = json!({"tick": snapshot.tick(), "tile": snapshot.tile(),
                    "hp": input.hp, "food": food, "hitmarks": observed_hitmarks(client)});
                if self.frames.last() != Some(&value) {
                    self.frames.push(value);
                }
            }
            _ => {}
        }
        if let Some((directory, receipt)) = self.shot.take() {
            capture(client, &directory, "compat-v1-arrived", &receipt);
            self.captured = true;
        }
    }
}

fn compat_source(destination: WorldTile) -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
import {{ Sustain }} from '../../api/sustain/Sustain.js';
import {{ Inventory }} from '../../api/inventory/Inventory.js';
import {{ Skills }} from '../../api/skills/Skills.js';
export default class S2bCompat extends LoopingBot {{
    async loop() {{
        if (globalThis.__manual_started) return;
        globalThis.__manual_started = true;
        globalThis.__manual_sustain_eats = 0;
        Sustain.set(async () => {{
            const hp = Skills.effective('hitpoints');
            const max = Skills.level('hitpoints');
            const food = Inventory.first('Lobster');
            if (food && (max - hp >= 12 || hp < 20)) {{
                if (food.interact('Eat')) globalThis.__manual_sustain_eats++;
            }}
        }});
        globalThis.__manual_walk_result = await Traversal.walkTo(
            {{ x: {}, z: {}, level: {} }}, {{ arriveWithin: 0 }}
        );
        globalThis.__manual_terminal_count = 1;
    }}
}}
"#,
        destination.x, destination.z, destination.level
    )
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, WORLD_ENGINE_DIR, per-run BOT_CACHE_DIR/BOT_UNPACK_DIR and LIVE_EVIDENCE_DIR; Engine A 44594/1080"]
fn real_compat_v1_crossing_and_own_sustain_at_45_and_14_hp() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    install_nav_logs();
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("owned account prefix");
    assert!(!prefix.is_empty() && prefix.len() <= 4);
    let directory =
        PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("evidence directory"));
    let (profile, template) = selected_template();
    for (initial_hp, base_hp) in [(45, 57), (14, 45)] {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        let name = format!("{prefix}{}", serial % 1_000_000_000);
        let state = Arc::new(Mutex::new(CompatLive {
            snapshot: GameSnapshot::new(),
            world: template.world().expect("actual nav world"),
            members: profile.map_members(),
            initial_hp,
            base_hp,
            phase: 0,
            relogger: mainland_relog_runner(),
            endpoints: None,
            baseline: None,
            frames: Vec::new(),
            ready: false,
            failure: None,
            shot: None,
            captured: false,
        }));
        let frame_state = Arc::clone(&state);
        let mut play = host_play::run_with_template(
            Arc::clone(&template),
            true,
            vec![],
            |_| (None, None),
            move |client, _, _| frame_state.lock().expect("fixture state").frame(client),
        )
        .expect("start actual host");
        play.try_spawn_slot(
            vault::Profile {
                username: name.clone(),
                password: name.clone().into(),
                uid: (serial % 1_000_000_000) as i32,
                settings: vault::ProfileSettings::default(),
            },
            None,
            None,
            None,
        )
        .expect("spawn owned live account");
        let deadline = Instant::now() + Duration::from_secs(90);
        let destination = loop {
            {
                let state = state.lock().expect("fixture state");
                assert!(state.failure.is_none(), "{:?}", state.failure);
                if state.ready {
                    break state.endpoints.expect("seeded endpoints").1;
                }
            }
            if Instant::now() >= deadline {
                let state = state.lock().expect("fixture state");
                panic!(
                    "compat setup deadline: phase={}, scene={}, inventory_size={}, captured={:?}",
                    state.phase,
                    state.snapshot.scene_state(),
                    state.snapshot.inventory_size(),
                    admission::capture(
                        &state.snapshot,
                        state.members,
                        PoisonState::default(),
                        false
                    )
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        play.script_start_load(
            &name,
            compat_source(destination),
            script::LoadShape::CompatClass,
            None,
            vec![],
        )
        .expect("start real v1 class");
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut seen_risk = None;
        let terminal = loop {
            {
                let state = state.lock().expect("fixture state");
                assert!(state.failure.is_none(), "{:?}", state.failure);
            }
            assert!(
                play.script_last_error(&name).is_none(),
                "{:?}",
                play.script_last_error(&name)
            );
            let probe = play.manual_click_live_probe(&name);
            if !probe["nav"].is_null() {
                assert_eq!(probe["nav"]["guard_active"], false, "{probe}");
                assert_eq!(probe["nav"]["off_debt"], false, "{probe}");
                assert!(probe["nav"]["escape"].is_null(), "{probe}");
                if !probe["nav"]["admission"].is_null() {
                    assert_eq!(probe["nav"]["admission"]["policy"], "Proceed", "{probe}");
                    for permission in ["prayer", "food", "escape"] {
                        assert_eq!(probe["nav"]["admission"][permission], false, "{probe}");
                    }
                }
                if !probe["nav"]["assessment"].is_null() {
                    assert_eq!(probe["nav"]["compat_v1"], true, "{probe}");
                    assert!(probe["nav"]["assessment"]["verdict"].is_string(), "{probe}");
                    assert!(
                        probe["nav"]["assessment"]["crossings"]
                            .as_u64()
                            .is_some_and(|crossings| crossings > 0),
                        "{probe}"
                    );
                    seen_risk = Some(probe["nav"]["assessment"].clone());
                }
            }
            assert!(
                play.script_walk_risk(&name).is_none(),
                "v1 assessment must remain log-only"
            );
            if probe["script"]["terminal_count"] == 1 {
                break probe;
            }
            assert!(
                Instant::now() < deadline,
                "compat crossing deadline: {probe}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(terminal["script"]["walk_result"], true, "{terminal}");
        assert!(
            terminal["script"]["sustain_eats"]
                .as_u64()
                .is_some_and(|count| count > 0),
            "{terminal}"
        );
        let cell_dir = directory.join(format!("compat-v1-hp{initial_hp}"));
        std::fs::create_dir_all(&cell_dir).expect("create compat evidence");
        let receipt = {
            let state = state.lock().expect("fixture state");
            let input = admission::capture(
                &state.snapshot,
                state.members,
                PoisonState::default(),
                false,
            );
            let food = state
                .snapshot
                .inventory()
                .iter()
                .filter(|item| item.def.name.as_deref() == Some("Lobster"))
                .map(|item| item.count)
                .sum::<i32>();
            assert!(
                food < 6 && input.hp > 0,
                "the real script hook must consume food without death"
            );
            assert!(
                state
                    .frames
                    .windows(2)
                    .any(|frames| frames[1]["hp"].as_u64() > frames[0]["hp"].as_u64()),
                "the real script hook must produce an observed HP rise"
            );
            assert!(
                state.snapshot.tile().is_some_and(|here| {
                    here.2 == destination.level
                        && (here.0 - destination.x)
                            .abs()
                            .max((here.1 - destination.z).abs())
                            <= 2
                }),
                "v1's legacy Reach(2) completion must be honored"
            );
            let logs = nav_logs(&name);
            assert!(
                logs.iter().any(
                    |line| line["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("risk=")
                            && message.contains("compat_v1=true"))
                ),
                "compat risk assessment must be in the real hostlog: {logs:?}"
            );
            assert!(
                !logs.iter().any(|line| line["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("escape="))),
                "compat must not run an escape: {logs:?}"
            );
            json!({"request": "SURVIVABLE-NAV-S2B", "account": name,
                "engine": "127.0.0.1:44594", "nav": profile.nav_identity().expect("nav identity").nav_sha256,
                "baseline": state.baseline, "assessment": seen_risk.expect("real assessment"),
                "terminal": terminal, "observed_frames": state.frames, "hostlog": logs,
                "final_hp": input.hp, "final_food": food, "final_tile": state.snapshot.tile()})
        };
        std::fs::write(
            cell_dir.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).expect("JSON"),
        )
        .expect("write compat evidence");
        state.lock().expect("fixture state").shot = Some((cell_dir, receipt));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !state.lock().expect("fixture state").captured {
            assert!(Instant::now() < deadline, "compat PNG deadline");
            std::thread::sleep(Duration::from_millis(20));
        }
        play.stop_slot(&name);
    }
}

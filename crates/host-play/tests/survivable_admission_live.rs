//! Compat v1 keeps its own sustain and legacy walk result on real content.
//! Probability admission and live poison holds are tested in the host module.
#![cfg(feature = "js")]
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host_play::admission::{self, Admission, RiskInput};
use host_play::{ProfileOptions, SharedClientTemplate, WalkGlobals};
use nav_planner::router::FindOptions;
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
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
        port: Some(host_play::live_ports::game_port()),
        http_port: Some(host_play::live_ports::http_port()),
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

#[cfg(feature = "js")]
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
        zones: nav_planner::zones::ZoneExempt::all(),
        ..FindOptions::default()
    };
    let route = nav_planner::router::find_with(
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
        .all(|leg| matches!(leg, nav_planner::router::Leg::Walk { .. })));
    let mut risk_input = input;
    risk_input.pos = origin;
    let assessment = admission::assess(
        &route,
        world,
        &Admission::manual(
            WalkGlobals::default().manual_decision(options.zones.is_all()),
            risk_input,
            0,
        ),
    );
    assert!(
        !assessment.plan.intervals.is_empty(),
        "real corridor must cross danger"
    );
    (origin, destination)
}

#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
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
        let input = admission::capture(snapshot, self.members, None, false, None);
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
            3 if input.hp == self.base_hp && food == 6 && !input.missing_facts() => {
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

#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, WORLD_ENGINE_DIR, per-run BOT_CACHE_DIR/BOT_UNPACK_DIR and LIVE_EVIDENCE_DIR; a local 289 engine at WORLD_GAME_PORT/WORLD_HTTP_PORT (Engine A 44594/1080 by default)"]
fn real_compat_v1_crossing_and_own_sustain_at_45_and_14_hp() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    if api::test_support::local_asset_path(
        "WORLD_ENGINE_DIR",
        "",
        "real 289 engine content for survivability admission",
    )
    .is_none()
    {
        return;
    }
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
        let name = host_play::mint_live_names(1)
            .pop()
            .expect("one live account");
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
                let mut state = state.lock().expect("fixture state");
                assert!(state.failure.is_none(), "{:?}", state.failure);
                if state.relogger.take_relog_login() {
                    if let Some(arm) = play.arm(&name) {
                        arm.arm_explicit_login();
                    }
                }
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
                    admission::capture(&state.snapshot, state.members, None, false, None)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let source = compat_source(destination);
        play.script_start_load(
            &name,
            source,
            script_js::LoadShape::CompatClass,
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
        let receipt =
            {
                let state = state.lock().expect("fixture state");
                let input = admission::capture(&state.snapshot, state.members, None, false, None);
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
                    logs.iter().any(|line| line["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("risk=")
                            && message.contains("compat_v1=true"))),
                    "compat risk assessment must be in the real hostlog: {logs:?}"
                );
                assert!(
                    !logs.iter().any(|line| line["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("escape="))),
                    "compat must not run an escape: {logs:?}"
                );
                json!({"request": "SURVIVABLE-NAV-S2B", "account": name,
                "engine": format!("127.0.0.1:{}", host_play::live_ports::game_port()),
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

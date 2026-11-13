//! Engine A proof that compat InspectRoute and Traversal use fresh Hero's Quest evidence.
#![cfg(all(
    feature = "js",
    feature = "live-harness",
    feature = "live-probe",
    feature = "test-support"
))]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::SelectedGameData;
use api::interact::{self, Interactions};
use api::selected::QuestGate;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host_play::{ProfileOptions, SharedClientTemplate};
use nav_planner::transport::TransportKind;
use nav_planner::world::NavWorld;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use serde_json::{json, Value};
use vault::Profile;

const HEROES_GUILD_OUTSIDE: WorldTile = WorldTile {
    x: 2899,
    z: 3510,
    level: 0,
};
const HEROES_GUILD_INSIDE: WorldTile = WorldTile {
    x: 2905,
    z: 3510,
    level: 0,
};

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
}

fn quest_seed_commands(
    selected: &SelectedGameData,
) -> Result<(Vec<String>, Vec<Value>), String> {
    let quests = selected
        .try_quests()
        .ok_or_else(|| "selected quest catalog was not prepared".to_owned())?;
    let mut commands = Vec::new();
    let mut receipt = Vec::new();
    for (quest_id, stage) in [
        ("zanaris", 6),
        ("dragon", 10),
        ("arthur", 7),
        ("fortress", 4),
        ("hero", 15),
    ] {
        let facts = quests
            .quest(quest_id)
            .map_err(|error| format!("selected quest {quest_id}: {error:?}"))?;
        if facts.complete != stage {
            return Err(format!(
                "selected quest {quest_id} completes at {}, expected seed stage {stage}",
                facts.complete
            ));
        }
        let varp = facts.varp.0.as_ref();
        commands.push(format!("setvar {varp} {stage}"));
        receipt.push(json!({"quest_id": quest_id, "varp": varp, "stage": stage}));
    }
    Ok((commands, receipt))
}

fn hero_entrance_gate(world: &NavWorld) -> Result<String, String> {
    let edge = world.graph.edges.iter().find(|edge| {
        matches!(edge.kind, TransportKind::Door)
            && [edge.at, edge.to].into_iter().any(|tile| {
                tile.level == 0
                    && (2898..=2906).contains(&tile.x)
                    && (3508..=3512).contains(&tile.z)
            })
            && edge.quest_gates.as_ref().is_some_and(|gates| {
                gates.gates().iter().any(|gate| match gate {
                    QuestGate::Window(window) => {
                        window.quest.0.as_ref() == "hero"
                            && window.signal.0.as_ref() == "heroquest"
                            && window.values.min == Some(15)
                    }
                    _ => false,
                })
            })
    });
    edge.map(|edge| format!("{edge:?}")).ok_or_else(|| {
        "WORLD_NAV_PACK has no Heroes Guild entrance door with the Hero's Quest stage window"
            .into()
    })
}

fn selected_template() -> Result<
    (Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>),
    String,
> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(host_play::live_ports::game_port()),
        http_port: Some(host_play::live_ports::http_port()),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(required_path("BOT_UNPACK_DIR")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        ..ProfileOptions::default()
    };
    let profile = options
        .resolve(None)
        .map_err(|error| format!("resolve Engine A profile: {error:?}"))?
        .bind_runtime()
        .map_err(|error| format!("bind Engine A runtime profile: {error:?}"))?;
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .map_err(|error| format!("load Engine A client template: {error:?}"))?;
    Ok((profile, template))
}

fn cold_relog_runner() -> ScenarioRunner {
    let proof = Proof::SideTabAvailable { index: 3 };
    ScenarioRunner::with_world(
        Scenario {
            name: "nav_quest_evidence_cold_relog",
            seed: Seed {
                profiles: Vec::new(),
                mainland: false,
            },
            steps: vec![Step {
                name: "relog after Hero's Quest progress seeds",
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

fn fresh_hero_row(snapshot: &GameSnapshot, hero_display: &str) -> Option<Value> {
    if !snapshot.quest_statuses_available() {
        return None;
    }
    let (index, row) = snapshot
        .quest_statuses()
        .iter()
        .enumerate()
        .find(|(_, row)| row.name.trim().eq_ignore_ascii_case(hero_display))?;
    if row.colour != nav_planner::world_state::QUEST_COMPLETE_COLOUR
        || !snapshot.quest_green_since_fresh_login(index)
    {
        return None;
    }
    Some(json!({
        "tick": snapshot.tick(),
        "component_id": row.component_id,
        "name": row.name,
        "colour": row.colour,
        "green_since_fresh_login": true,
    }))
}

struct QuestEvidenceLive {
    snapshot: GameSnapshot,
    hero_display: String,
    phase: u8,
    seed_commands: Vec<String>,
    relogger: ScenarioRunner,
    seeded: bool,
    quest_tab_open: bool,
    ready: bool,
    failure: Option<String>,
    fresh_hero: Option<Value>,
    frames: Vec<Value>,
}

impl QuestEvidenceLive {
    fn frame(&mut self, client: &mut Client) {
        self.snapshot.rebuild(client);
        if self.phase == 2 {
            self.relogger.tick(client);
            match self.relogger.status() {
                RunnerStatus::Passed => self.phase = 3,
                RunnerStatus::Failed(error) => {
                    self.failure = Some(format!("cold relog failed: {error}"));
                    return;
                }
                RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
            }
        }
        if !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return;
        }
        match self.phase {
            0 => {
                interact::mainland_hop(client);
                self.phase = 1;
            }
            1 => {
                if !self.seeded {
                    for command in &self.seed_commands {
                        if !interact::cheat(client, command).is_sent() {
                            self.failure =
                                Some(format!("quest seed command was refused: {command}"));
                            return;
                        }
                    }
                    self.seeded = true;
                }
                if !self.quest_tab_open {
                    let available = self
                        .snapshot
                        .side_tabs()
                        .iter()
                        .any(|tab| tab.index == 2 && tab.available);
                    if !available {
                        return;
                    }
                    if self.snapshot.active_side_tab() != 2
                        && !Interactions::new(&self.snapshot, client)
                            .click_side_tab(2)
                            .is_sent()
                    {
                        self.failure = Some("quest side tab could not be opened".into());
                        return;
                    }
                    self.quest_tab_open = true;
                }
                if self.snapshot.quest_statuses_available() {
                    self.phase = 2;
                }
            }
            2 => {}
            3 => {
                if let Some(fresh_hero) = fresh_hero_row(&self.snapshot, &self.hero_display) {
                    self.fresh_hero = Some(fresh_hero);
                    let tele = interact::tele_args(
                        HEROES_GUILD_OUTSIDE.level,
                        HEROES_GUILD_OUTSIDE.x,
                        HEROES_GUILD_OUTSIDE.z,
                    );
                    if !interact::cheat(client, &tele).is_sent() {
                        self.failure = Some("Heroes Guild outside teleport was refused".into());
                        return;
                    }
                    self.phase = 4;
                }
            }
            4 if self.snapshot.tile()
                == Some((
                    HEROES_GUILD_OUTSIDE.x,
                    HEROES_GUILD_OUTSIDE.z,
                    HEROES_GUILD_OUTSIDE.level,
                ))
                && fresh_hero_row(&self.snapshot, &self.hero_display).is_some() =>
            {
                self.ready = true;
                self.phase = 5;
            }
            5 => {
                let frame = json!({"tick": self.snapshot.tick(), "tile": self.snapshot.tile()});
                if self.frames.last() != Some(&frame) {
                    self.frames.push(frame);
                }
            }
            _ => {}
        }
    }
}

const COMPAT_SOURCE: &str = r#"
import { reader } from '../../adapter/ClientAdapter.js';
import { WalkExecutor } from '../../event/webwalk/WalkExecutor.js';
import { Traversal } from '../../api/walking/Traversal.js';
globalThis.__manual_started = false;
globalThis.__manual_walk_result = null;
globalThis.__manual_terminal_count = 0;
export default class QuestEvidenceProbe extends LoopingBot {
    async loop() {
        if (globalThis.__manual_started) return;
        globalThis.__manual_started = true;
        const origin = reader.worldTile();
        if (!origin || origin.x !== 2899 || origin.z !== 3510 || origin.level !== 0) {
            throw new Error(`unexpected Heroes Guild origin: ${JSON.stringify(origin)}`);
        }
        const inspectRoute = await WalkExecutor.probeDest(
            { x: 2905, z: 3510, level: 0 }, 120000
        );
        if (!inspectRoute.ok || inspectRoute.terminal?.x !== 2905
            || inspectRoute.terminal?.z !== 3510 || inspectRoute.terminal?.level !== 0) {
            throw new Error(`InspectRoute did not prove the Hero gate: ${JSON.stringify(inspectRoute)}`);
        }
        const walked = await Traversal.walkTo(
            { x: 2905, z: 3510, level: 0 }, { radius: 0, timeoutMs: 180000 }
        );
        if (!walked) throw new Error('Traversal.walkTo did not cross the Heroes Guild entrance');
        const finalTile = reader.worldTile();
        if (!finalTile || finalTile.x !== 2905 || finalTile.z !== 3510 || finalTile.level !== 0) {
            throw new Error(`Traversal ended away from the Guild interior: ${JSON.stringify(finalTile)}`);
        }
        globalThis.__manual_walk_result = {
            origin,
            inspect_route: inspectRoute,
            traversal_walked: walked,
            final_tile: finalTile,
        };
        globalThis.__manual_terminal_count = 1;
    }
}
"#;

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, WORLD_ENGINE_DIR, WORLD_NAV_PACK, per-run BOT_CACHE_DIR/BOT_UNPACK_DIR, LIVE_EVIDENCE_DIR, and local Engine A at WORLD_GAME_PORT/WORLD_HTTP_PORT (44594/1080 by default)"]
fn real_engine_a_compat_inspect_route_and_traversal_cross_heroes_guild() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    if !api::test_support::game_data_available() {
        return;
    }
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("owned account prefix");
    assert!(!prefix.is_empty() && prefix.len() <= 4);
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR");
    let (profile, template) = selected_template().expect("resolve real Engine A content");
    let selected = profile
        .game_data()
        .expect("selected Engine A game data");
    let hero_display = selected
        .try_quests()
        .expect("selected quest catalog")
        .quest("hero")
        .expect("Hero's Quest facts")
        .display
        .to_string();
    let (seed_commands, seed_receipt) = quest_seed_commands(&selected)
        .expect("resolve only the Hero's own prerequisite stages from content");
    let world = template.world().expect("actual Heroes Guild nav world");
    let entrance_gate = hero_entrance_gate(&world).expect("actual Hero's Quest entrance gate");

    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let account = host_play::mint_live_names(1)
        .pop()
        .expect("one owned live account");
    let state = Arc::new(Mutex::new(QuestEvidenceLive {
        snapshot: GameSnapshot::new(),
        hero_display,
        phase: 0,
        seed_commands,
        relogger: cold_relog_runner(),
        seeded: false,
        quest_tab_open: false,
        ready: false,
        failure: None,
        fresh_hero: None,
        frames: Vec::new(),
    }));
    let frame_state = Arc::clone(&state);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, _, _| {
            frame_state
                .lock()
                .expect("quest evidence fixture")
                .frame(client)
        },
    )
    .expect("start actual Engine A host");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: (serial % 1_000_000_000) as i32,
            settings: vault::ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn owned Engine A account");

    let setup_deadline = Instant::now() + Duration::from_secs(180);
    let fresh_hero = loop {
        {
            let mut fixture = state.lock().expect("quest evidence fixture");
            let failure = fixture.failure.clone();
            assert!(failure.is_none(), "{failure:?}");
            if fixture.relogger.take_relog_login() {
                if let Some(arm) = play.arm(&account) {
                    arm.arm_explicit_login();
                }
            }
            if fixture.ready {
                break fixture.fresh_hero.clone().expect("fresh Hero row receipt");
            }
        }
        let (phase, tile, quest_statuses) = {
            let fixture = state.lock().expect("quest evidence fixture");
            (
                fixture.phase,
                fixture.snapshot.tile(),
                fixture.snapshot.quest_statuses().to_vec(),
            )
        };
        assert!(
            Instant::now() < setup_deadline,
            "quest evidence setup timed out: phase={phase}, snapshot={tile:?}, quest_statuses={quest_statuses:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };

    play.script_start_load(
        &account,
        COMPAT_SOURCE.to_owned(),
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start compat InspectRoute/Traversal script");
    let travel_deadline = Instant::now() + Duration::from_secs(240);
    let terminal = loop {
        let failure = state
            .lock()
            .expect("quest evidence fixture")
            .failure
            .clone();
        assert!(failure.is_none(), "{failure:?}");
        let script_error = play.script_last_error(&account);
        assert!(script_error.is_none(), "{script_error:?}");
        let probe = play.manual_click_live_probe(&account);
        if probe["script"]["terminal_count"].as_u64() == Some(1) {
            break probe;
        }
        assert!(
            Instant::now() < travel_deadline,
            "Heroes Guild traversal timed out: {probe}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let walk_result = &terminal["script"]["walk_result"];
    assert_eq!(walk_result["inspect_route"]["ok"], true, "{terminal}");
    assert_eq!(
        walk_result["inspect_route"]["terminal"],
        json!({
            "x": HEROES_GUILD_INSIDE.x,
            "z": HEROES_GUILD_INSIDE.z,
            "level": HEROES_GUILD_INSIDE.level
        }),
        "{terminal}"
    );
    assert_eq!(walk_result["traversal_walked"], true, "{terminal}");
    assert_eq!(
        walk_result["final_tile"],
        json!({
            "x": HEROES_GUILD_INSIDE.x,
            "z": HEROES_GUILD_INSIDE.z,
            "level": HEROES_GUILD_INSIDE.level
        }),
        "{terminal}"
    );
    let final_tile = state
        .lock()
        .expect("quest evidence fixture")
        .snapshot
        .tile();
    assert_eq!(
        final_tile,
        Some((HEROES_GUILD_INSIDE.x, HEROES_GUILD_INSIDE.z, HEROES_GUILD_INSIDE.level)),
        "the live client must be inside the Guild"
    );

    std::fs::create_dir_all(&evidence_dir).expect("create live evidence directory");
    let receipt_path = evidence_dir.join(format!("nav-quest-evidence-{account}-{serial}.json"));
    let fixture = state.lock().expect("quest evidence fixture");
    let receipt = json!({
        "test": "NAV-0291 compat InspectRoute and Traversal",
        "engine": format!("127.0.0.1:{}", host_play::live_ports::game_port()),
        "account": account,
        "seeded_quest_stages": seed_receipt,
        "fresh_cold_login_hero_row": fresh_hero,
        "actual_heroes_guild_quest_gate_edge": entrance_gate,
        "origin": HEROES_GUILD_OUTSIDE,
        "destination": HEROES_GUILD_INSIDE,
        "inspect_route_and_walk": walk_result,
        "observed_tiles": fixture.frames,
        "final_tile": final_tile,
    });
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize live receipt"),
    )
    .expect("write live receipt");
    println!("NAV_QUEST_EVIDENCE {receipt}");
    drop(fixture);
    play.stop_slot(&account);
}

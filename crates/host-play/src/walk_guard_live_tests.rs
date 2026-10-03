//! W1 protected crossing of a zoned route under a staged ranged attacker.
//!
//! `LIVE=1` with throwaway HOME, copied cache, explicit engine/nav, prefix `wg`.
use super::combat_proof::{self, CaptureRegistration, CombatCapture};
use super::{run_with_template, ProfileOptions, ScriptStartHandle};
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
use host::{FrameBuf, Pump};
use scenario::{Proof, RunnerStatus, Scenario, ScenarioRunner, Step, StepKind, Wait};
use script::combat::HitOnset;
use script::quester::compile::{compile_path, CompiledPath};
use script::quester::runner::Quester;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vault::{Profile, ProfileSettings};

const EVIDENCE_DIR: &str = "/Volumes/dev-scratch/274bot-evidence/WALK-GUARD";
const MISSILES_VARP: i32 = 96;
const ZONE: &str = "death-plateau-throwers";

fn path_json(dest: WorldTile) -> String {
    format!(
        r#"{{
  "schema": 2,
  "id": "imp",
  "display_name": "Walk Guard Throwers",
  "required": [],
  "tested_stats": null,
  "partner": null,
  "quest": {{
    "members": false,
    "quest_points": 1,
    "requirements": [],
    "items": [],
    "acquire": {{}},
    "bank": "nearest",
    "coin_float": 0,
    "loadouts": {{}},
    "areas": {{}},
    "tools": [],
    "owns_inventory": true
  }},
  "roles": [
    {{
      "role": null,
      "progress_binding": "journal:imp",
      "progress": {{
        "colour": {{ "not_started": "imp:0", "in_progress": "imp:0", "complete": "imp:2" }},
        "rules": [],
        "flags": [],
        "monotonic": false
      }},
      "prelude": [],
      "sequences": [
        {{
          "stage": "imp:0",
          "required": [],
          "terminal": false,
          "recovery_entry": null,
          "steps": [
            {{
              "id": "cross-throwers",
              "kind": "walk",
              "version": 1,
              "args": {{
                "tile": [{}, {}, {}],
                "source": "W1 protected crossing of death-plateau-throwers",
                "radius": 1,
                "cross": ["death-plateau-throwers"],
                "guard": "protect"
              }},
              "skip_if": {{ "Fact": {{ "kind": "near", "version": 1, "args": {{ "tile": [{}, {}, {}], "radius": 1 }} }} }},
              "settle": {{ "Fact": {{ "kind": "near", "version": 1, "args": {{ "tile": [{}, {}, {}], "radius": 1 }} }} }}
            }}
          ]
        }},
        {{ "stage": "imp:2", "required": [], "terminal": true, "recovery_entry": null, "steps": [] }}
      ]
    }}
  ]
}}"#,
        dest.x, dest.z, dest.level, dest.x, dest.z, dest.level, dest.x, dest.z, dest.level
    )
}

fn throwers_route_context(
    world: &nav::world::NavWorld,
) -> (nav::router::FindOptions, nav::WorldState) {
    let table = world.graph.zones.as_ref().expect("baked zone table");
    let key = table.resolve(ZONE).expect("death-plateau-throwers");
    let zones = nav::zones::ZoneExempt::named(&[key]).expect("zone exempt");
    let opts = nav::router::FindOptions {
        zones,
        ..nav::router::FindOptions::default()
    };
    let state = nav::WorldState::empty().with_map_members(true);
    (opts, state)
}

fn pick_crossing(world: &nav::world::NavWorld) -> (WorldTile, WorldTile) {
    let (opts, state) = throwers_route_context(world);
    let mut starts = Vec::new();
    let mut dests = Vec::new();
    for z in 3590..=3608 {
        for x in (2800..2843).rev() {
            let tile = WorldTile { x, z, level: 0 };
            if world.collision.standable(tile) {
                starts.push(tile);
                break;
            }
        }
        for x in 2879..=2910 {
            let tile = WorldTile { x, z, level: 0 };
            if world.collision.standable(tile) {
                dests.push(tile);
                break;
            }
        }
    }
    for start in &starts {
        for dest in &dests {
            if nav::router::find_with(&world.collision, &world.graph, *start, *dest, opts, &state)
                .is_ok()
            {
                return (*start, *dest);
            }
        }
    }
    panic!("no standable protected crossing of {ZONE}");
}

#[test]
fn death_plateau_throwers_has_a_standable_protected_crossing() {
    let pack =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/nav/289/274bot.navpack");
    let pack = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .unwrap_or(pack);
    if !pack.exists() {
        panic!("W1 pack missing at {}", pack.display());
    }
    let world = nav::world::NavWorld::load_pack(&pack).expect("load baked 289 pack");
    let (start, dest) = pick_crossing(&world);
    assert_ne!(start, dest, "crossing must move");
    assert_eq!(start.level, 0);
    assert_eq!(dest.level, 0);
    assert!(start.x < 2843, "start west of the thrower zone");
    assert!(dest.x > 2878, "dest east of the thrower zone");
}

struct ThrowawayHome {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl ThrowawayHome {
    fn enter() -> Result<Self, String> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = PathBuf::from(EVIDENCE_DIR)
            .join("homes")
            .join(format!("w1-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create throwaway HOME {}: {error}", path.display()))?;
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", &path);
        Ok(Self { path, previous })
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn profile_options(home: &Path) -> Result<ProfileOptions, String> {
    let engine_dir = std::env::var_os("BOT_ENGINE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "W1 live proof requires BOT_ENGINE_DIR".to_owned())?;
    let nav_pack = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .or_else(|| {
            let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/nav/289/274bot.navpack");
            candidate.exists().then_some(candidate)
        });
    let source = std::env::var_os("BOT_COMBAT_CACHE_SNAPSHOT")
        .or_else(|| std::env::var_os("BOT_CACHE_SNAPSHOT"))
        .map(PathBuf::from)
        .ok_or_else(|| "W1 live proof requires BOT_COMBAT_CACHE_SNAPSHOT".to_owned())?;
    let version = source
        .file_name()
        .ok_or_else(|| "cache snapshot has no version directory".to_owned())?;
    let cache = home.join("unpack").join(version);
    std::fs::create_dir_all(&cache).map_err(|error| format!("create cache copy: {error}"))?;
    for entry in
        std::fs::read_dir(&source).map_err(|error| format!("read retained cache: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read retained cache entry: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("cache entry type: {error}"))?
            .is_file()
        {
            continue;
        }
        std::fs::copy(entry.path(), cache.join(entry.file_name()))
            .map_err(|error| format!("copy retained cache: {error}"))?;
    }
    Ok(ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(45_594),
        http_port: Some(2_080),
        vault_path: Some(home.join("vault")),
        cache_dir: Some(cache),
        unpack_dir: Some(home.join("unpack")),
        nav_pack,
        engine_dir: Some(engine_dir),
        ..ProfileOptions::default()
    })
}

fn cheat_step(name: &'static str, command: String, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| {
                matches!(
                    api::interact::cheat(client, &command),
                    client::CheatSend::Sent
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: 120,
        },
    }
}

fn missiles_on(snapshot: &GameSnapshot) -> bool {
    snapshot
        .varps()
        .iter()
        .find(|row| row.index == MISSILES_VARP)
        .is_some_and(|row| row.value == 1)
}

fn arrived_at(snapshot: &GameSnapshot, dest: WorldTile) -> bool {
    snapshot.tile().is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs().max((z - dest.z).abs()) <= 1
    })
}

fn prayers_active(snapshot: &GameSnapshot) -> bool {
    snapshot
        .varps()
        .iter()
        .any(|row| (83..=97).contains(&row.index) && row.value == 1)
}

fn chebyshev(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

/// NPC → player ranged queue, design §5.4 / §10.6: `floor((32 + 5d) / 30)`.
fn npc_ranged_queue_ticks(distance: i32) -> u32 {
    let distance = distance.max(0);
    (32 + 5 * distance) as u32 / 30
}

fn send_cheat(client: &mut client::client::Client, command: &str) -> bool {
    matches!(
        api::interact::cheat(client, command),
        client::CheatSend::Sent
    )
}

fn reconstructed_launch_tick(tick: u32, loop_cycle: Option<i32>, t1: i32) -> u32 {
    let Some(cycle) = loop_cycle else {
        return tick;
    };
    if t1 < 0 {
        return tick;
    }
    let age = cycle.saturating_sub(t1).max(0) as u32 / 30;
    tick.saturating_sub(age)
}

#[derive(Clone, Copy)]
struct LaunchRecord {
    tick: u32,
    src: WorldTile,
    here: WorldTile,
    distance: i32,
    t1: i32,
    t2: i32,
    impact_tick: Option<u32>,
    observed_delay: Option<u32>,
}

impl LaunchRecord {
    fn identity(self) -> (i32, i32, i32, i32, i32) {
        (self.src.x, self.src.z, self.src.level, self.t1, self.t2)
    }

    fn expected_delay(self) -> u32 {
        npc_ranged_queue_ticks(self.distance)
    }

    fn json(self) -> serde_json::Value {
        json!({
            "tick": self.tick,
            "src": [self.src.x, self.src.z, self.src.level],
            "here": [self.here.x, self.here.z, self.here.level],
            "distance": self.distance,
            "t1": self.t1,
            "t2": self.t2,
            "impact_tick": self.impact_tick,
            "observed_delay": self.observed_delay,
        })
    }
}

fn in_crossing(launch: &LaunchRecord, off_tick: Option<u32>) -> bool {
    off_tick.is_none_or(|off| launch.tick < off)
}

/// Count-only gate that accepted the contradictory W1 receipt (two identities
/// and two distances with any delay). Kept for the regression that proves the
/// delay rule rejects that receipt.
fn count_only_queue_gate(launches: &[LaunchRecord]) -> bool {
    let measured: Vec<&LaunchRecord> = launches
        .iter()
        .filter(|launch| launch.observed_delay.is_some())
        .collect();
    let identities: std::collections::HashSet<(i32, i32, i32, i32, i32)> =
        measured.iter().map(|launch| launch.identity()).collect();
    let distances: std::collections::BTreeSet<i32> =
        measured.iter().map(|launch| launch.distance).collect();
    identities.len() >= 2 && distances.len() >= 2
}

fn queue_mismatch(launches: &[LaunchRecord]) -> Option<String> {
    launches.iter().find_map(|launch| {
        let delay = launch.observed_delay?;
        let expected = launch.expected_delay();
        (delay != expected).then(|| {
            format!(
                "NPC ranged queue delay {delay} at distance {} != {expected}",
                launch.distance
            )
        })
    })
}

fn queue_gate(launches: &[LaunchRecord], off_tick: Option<u32>) -> bool {
    if launches.iter().any(|launch| {
        launch
            .observed_delay
            .is_some_and(|delay| delay != launch.expected_delay())
    }) {
        return false;
    }
    let measured: Vec<&LaunchRecord> = launches
        .iter()
        .filter(|launch| launch.observed_delay.is_some() && in_crossing(launch, off_tick))
        .collect();
    let identities: std::collections::HashSet<(i32, i32, i32, i32, i32)> =
        measured.iter().map(|launch| launch.identity()).collect();
    let distances: std::collections::BTreeSet<i32> =
        measured.iter().map(|launch| launch.distance).collect();
    identities.len() >= 2 && distances.len() >= 2
}

fn attribute_due_impact(launches: &mut [LaunchRecord], tick: u32) {
    let due: Vec<usize> = launches
        .iter()
        .enumerate()
        .filter(|(_, launch)| {
            launch.impact_tick.is_none()
                && launch.tick <= tick
                && tick.saturating_sub(launch.tick) == launch.expected_delay()
        })
        .map(|(index, _)| index)
        .collect();
    if due.len() != 1 {
        return;
    }
    let launch = &mut launches[due[0]];
    launch.impact_tick = Some(tick);
    launch.observed_delay = Some(tick.saturating_sub(launch.tick));
}

fn w1_ready(
    capture: &CombatCapture,
    launched: bool,
    protected: bool,
    at_dest: bool,
    prayers_off: bool,
) -> bool {
    capture.started
        && launched
        && protected
        && at_dest
        && prayers_off
        && capture.invalid_reason.is_none()
}

struct LiveState {
    account: String,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    start_context: Option<(ScriptStartHandle, Arc<api::named_banks::NamedBankFacts>)>,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    path: Arc<CompiledPath>,
    capture: Arc<Mutex<CombatCapture>>,
    start: WorldTile,
    dest: WorldTile,
    started: bool,
    launch_tick: Option<u32>,
    protect_tick: Option<u32>,
    arrived: bool,
    last_tile: Option<(i32, i32, i32)>,
    attacker_staged: bool,
    prayers_off_after_arrival: bool,
    off_tick: Option<u32>,
    launches: Vec<LaunchRecord>,
    seen_launches: std::collections::HashSet<(i32, i32, i32, i32, i32)>,
    hit_onset: HitOnset,
    second_attacker_staged: bool,
    tiles_by_tick: std::collections::VecDeque<(u32, WorldTile)>,
}

impl LiveState {
    fn remember_tile(&mut self, tick: u32, tile: WorldTile) {
        if let Some((last_tick, last_tile)) = self.tiles_by_tick.back_mut() {
            if *last_tick == tick {
                *last_tile = tile;
                return;
            }
        }
        if self.tiles_by_tick.len() == 64 {
            self.tiles_by_tick.pop_front();
        }
        self.tiles_by_tick.push_back((tick, tile));
    }

    fn tile_at(&self, tick: u32) -> Option<WorldTile> {
        self.tiles_by_tick
            .iter()
            .rev()
            .find(|(at, _)| *at <= tick)
            .map(|(_, tile)| *tile)
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.last_tile = self.snapshot.tile();
        let tick = self.snapshot.tick();
        if let Some((x, z, level)) = self.last_tile {
            self.remember_tile(tick, WorldTile { x, z, level });
        }
        if self.runner.on_start_script() && !self.started {
            combat_proof::record_start_baseline(&self.account, &self.snapshot);
            let Some((handle, banks)) = self.start_context.as_ref() else {
                combat_proof::mark_invalid(
                    &self.account,
                    "StartScript reached before native preparation",
                );
                return;
            };
            let machine = Quester::new(
                RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                Arc::clone(&self.path),
                Arc::clone(&self.selected),
                Arc::clone(&self.quests),
                Arc::clone(banks),
            );
            match handle.start_test_script(
                &self.account,
                Box::new(machine),
                Some(Arc::clone(&self.selected)),
            ) {
                Ok(_) => {
                    self.started = true;
                    self.capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .started = true;
                    self.runner.observe_script_running();
                }
                Err(error) => {
                    combat_proof::mark_invalid(
                        &self.account,
                        format!("install compiled Quester failed: {error}"),
                    );
                    return;
                }
            }
        }
        if self.started
            && !self.attacker_staged
            && send_cheat(client, "npcadd death_troll_thrower1")
        {
            self.attacker_staged = true;
        }
        if self.attacker_staged
            && !self.second_attacker_staged
            && self
                .launches
                .first()
                .is_some_and(|launch| self.snapshot.tick() >= launch.tick.saturating_add(9))
            && send_cheat(client, "npcadd death_troll_thrower2")
        {
            self.second_attacker_staged = true;
        }
        let already_off = self.off_tick.is_some();
        if self.launch_tick.is_none() {
            self.launch_tick = self
                .launches
                .iter()
                .find(|launch| chebyshev(launch.src, self.start) > 4)
                .map(|launch| launch.tick);
        }
        if self.started && missiles_on(&self.snapshot) && self.protect_tick.is_none() {
            self.protect_tick = Some(self.snapshot.tick());
        }
        if arrived_at(&self.snapshot, self.dest) {
            self.arrived = true;
        }
        if self.arrived && !prayers_active(&self.snapshot) {
            if self.off_tick.is_none() {
                self.off_tick = Some(self.snapshot.tick());
            }
            self.prayers_off_after_arrival = true;
        }
        let me = self.snapshot.self_slot() as usize;
        let loop_cycle = self.snapshot.hitmarks().map(|hitmarks| hitmarks.loop_cycle);
        if self.started && !already_off {
            if let Some((x, z, level)) = self.last_tile {
                let here = WorldTile { x, z, level };
                for projectile in self.snapshot.projectiles() {
                    if !projectile.target.is_some_and(|target| {
                        target.kind == ActorKind::Player && target.index == me
                    }) {
                        continue;
                    }
                    let key = (
                        projectile.src.x,
                        projectile.src.z,
                        projectile.src.level,
                        projectile.t1,
                        projectile.t2,
                    );
                    if !self.seen_launches.insert(key) {
                        continue;
                    }
                    let launch_tick = reconstructed_launch_tick(tick, loop_cycle, projectile.t1);
                    let here = self.tile_at(launch_tick).unwrap_or(here);
                    self.launches.push(LaunchRecord {
                        tick: launch_tick,
                        src: projectile.src,
                        here,
                        distance: chebyshev(projectile.src, here),
                        t1: projectile.t1,
                        t2: projectile.t2,
                        impact_tick: None,
                        observed_delay: None,
                    });
                }
            }
        }
        if self.started {
            if let Some(hitmarks) = self.snapshot.hitmarks() {
                for _ in self.hit_onset.observe(&hitmarks.marks, hitmarks.loop_cycle) {
                    attribute_due_impact(&mut self.launches, tick);
                }
            }
        }
        if matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            return;
        }
        self.runner.tick_with_hold(client, hold);
    }
}

fn scenario_for(capture: Arc<Mutex<CombatCapture>>, start: WorldTile) -> Scenario {
    let mut scenario =
        scenario::quester_stage("walk_guard_throwers", "Imp Catcher", "imp", 0, &[], start);
    let stand_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "stand at the quest start")
        .expect("stand step");
    let relog_index = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .expect("relog step");
    let relog = scenario.steps.remove(relog_index);
    scenario.steps.insert(stand_index, relog);
    let extra = [cheat_step(
        "seed Prayer 43",
        "setstat prayer 43".to_owned(),
        Proof::Stat { id: 5, min: 43 },
    )];
    scenario
        .steps
        .splice(stand_index + 1..stand_index + 1, extra);
    let start_index = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("StartScript");
    scenario.steps.truncate(start_index + 1);
    let ready_capture = Arc::clone(&capture);
    scenario.steps.push(Step {
        name: "wait for the protected crossing",
        kind: StepKind::Await {
            evidence: "W1 protect and arrival",
            ready: Box::new(move |_| {
                let capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                capture.invalid_reason.is_some() || capture.started
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 0, min: 1 },
            budget_ticks: u32::MAX,
        },
    });
    scenario.proof = Proof::Stat { id: 0, min: 1 };
    scenario.settings.deadline = Duration::from_secs(240);
    scenario.settings.nav.engine_speed_ms = None;
    scenario
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_walk_guard_w1_protected_crossing() {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "W1 requires LIVE=1"
    );
    std::env::set_var("BOT_LIVE_NAME_PREFIX", "wg");
    std::fs::create_dir_all(EVIDENCE_DIR).expect("create WALK-GUARD evidence directory");
    let home = ThrowawayHome::enter().expect("create isolated HOME");
    let options = profile_options(&home.path).expect("W1 live prerequisites");
    let template = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare isolated local-289 template");
    let world = template.world().expect("selected navigation world");
    let (start, dest) = pick_crossing(world.as_ref());
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    let path = compile_path(path_json(dest).as_bytes(), &selected, &quests)
        .unwrap_or_else(|error| panic!("compile W1 path: {error:?}"));
    let names = super::mint_live_names(1);
    let account = names.first().expect("mint W1 account").clone();
    let password = super::mint_live_entries(&names)
        .into_iter()
        .find(|(name, _)| name == &account)
        .map(|(_, password)| password)
        .expect("mint local W1 password");
    let capture = Arc::new(Mutex::new(CombatCapture::default()));
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let scenario = scenario_for(Arc::clone(&capture), start);
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_map_members(true);
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(LiveState {
        account: account.clone(),
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_context: None,
        selected: Arc::clone(&selected),
        quests,
        path,
        capture: Arc::clone(&capture),
        start,
        dest,
        started: false,
        launch_tick: None,
        protect_tick: None,
        arrived: false,
        last_tile: None,
        attacker_staged: false,
        prayers_off_after_arrival: false,
        off_tick: None,
        launches: Vec::new(),
        seen_launches: std::collections::HashSet::new(),
        hit_onset: HitOnset::new(),
        second_attacker_staged: false,
        tiles_by_tick: std::collections::VecDeque::new(),
    }));
    let frame_state = Arc::clone(&state);
    let frame_buffer = FrameBuf::new();
    let frame_account = account.clone();
    let profile = Profile {
        username: account.clone(),
        password: password.into(),
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("W1 clock")
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    let mut play = run_with_template(
        Arc::clone(&template),
        true,
        vec![profile],
        move |_| (None, Some(Arc::clone(&frame_buffer))),
        move |client, username, hold| {
            if username == frame_account {
                client.set_draw(true);
                frame_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .frame(client, hold.hold);
            }
        },
    )
    .expect("start local W1 Play");
    {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.runner.set_obj_names(play.obj_names());
        state.start_context = Some((play.script_start_handle(), play.named_banks()));
    }
    play.focus(&account);
    let deadline = Instant::now() + Duration::from_secs(240);
    let (outcome, error) = loop {
        let snapshot = state.lock().unwrap_or_else(|e| e.into_inner());
        let launched = snapshot.launch_tick.is_some();
        let protected = snapshot.protect_tick.is_some();
        let at_dest = snapshot.arrived;
        let prayers_off = snapshot.prayers_off_after_arrival;
        let launch_tick = snapshot.launch_tick;
        let protect_tick = snapshot.protect_tick;
        let mismatch = queue_mismatch(&snapshot.launches);
        let queue_measured = queue_gate(&snapshot.launches, snapshot.off_tick);
        drop(snapshot);
        if let Some(reason) = mismatch {
            break ("FAIL", Some(reason));
        }
        let capture_snapshot = capture.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reason) = capture_snapshot.invalid_reason.clone() {
            break ("INVALID", Some(reason));
        }
        if capture_snapshot.actions.iter().any(|action| {
            action["request"]["op"] == json!("npc")
                && action["request"]["action"] == json!("Attack")
        }) {
            break ("FAIL", Some("W1 emitted Attack".into()));
        }
        if w1_ready(&capture_snapshot, launched, protected, at_dest, prayers_off) && queue_measured
        {
            if let (Some(launch), Some(protect)) = (launch_tick, protect_tick) {
                let delta = protect.abs_diff(launch);
                if delta > 2 {
                    break (
                        "FAIL",
                        Some(format!(
                            "Protect from Missiles {protect} was not within 2 ticks of launch {launch}"
                        )),
                    );
                }
            }
            if !capture_snapshot
                .actions
                .iter()
                .any(|action| action["kind"] == json!("guard"))
            {
                break (
                    "FAIL",
                    Some("W1 receipt is missing walk-guard clicks".into()),
                );
            }
            break ("PASS", None);
        }
        drop(capture_snapshot);
        if Instant::now() >= deadline {
            break ("FAIL", Some("W1 live proof exceeded 240s".into()));
        }
        if matches!(
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .runner
                .status(),
            RunnerStatus::Failed(_)
        ) {
            break ("FAIL", Some("scenario runner failed".into()));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let snapshot = state.lock().unwrap_or_else(|e| e.into_inner());
    let capture_snapshot = capture.lock().unwrap_or_else(|e| e.into_inner());
    let prayer = snapshot
        .snapshot
        .stats()
        .iter()
        .find(|row| row.index == 5)
        .map(|row| json!({ "base": row.base, "effective": row.effective }));
    let receipt = json!({
        "proof": "WALK-GUARD",
        "case": "W1",
        "outcome": outcome,
        "error": error,
        "account": account,
        "zone": ZONE,
        "started": snapshot.started,
        "launch_tick": snapshot.launch_tick,
        "protect_tick": snapshot.protect_tick,
        "arrived": snapshot.arrived,
        "prayers_off_after_arrival": snapshot.prayers_off_after_arrival,
        "off_tick": snapshot.off_tick,
        "last_tile": snapshot.last_tile,
        "start": [start.x, start.z, start.level],
        "dest": [dest.x, dest.z, dest.level],
        "attacker_staged": snapshot.attacker_staged,
        "prayer": prayer,
        "runner": format!("{:?}", snapshot.runner.status()),
        "launches": snapshot
            .launches
            .iter()
            .copied()
            .map(LaunchRecord::json)
            .collect::<Vec<_>>(),
        "second_attacker_staged": snapshot.second_attacker_staged,
        "guard_clicks": capture_snapshot
            .actions
            .iter()
            .filter(|action| action["kind"] == json!("guard"))
            .cloned()
            .collect::<Vec<_>>(),
        "action_count": capture_snapshot.actions.len(),
        "action_kinds": capture_snapshot
            .actions
            .iter()
            .map(|action| action["kind"].clone())
            .collect::<Vec<_>>(),
        "last_status": capture_snapshot.statuses.last(),
    });
    let path = PathBuf::from(EVIDENCE_DIR).join(format!("W1-{account}-receipt.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).expect("write W1 receipt");
    assert_eq!(outcome, "PASS", "W1 {}", error.unwrap_or_default());
}

#[allow(clippy::too_many_arguments)]
fn launch_row(
    tick: u32,
    src: [i32; 3],
    here: [i32; 3],
    distance: i32,
    t1: i32,
    t2: i32,
    impact_tick: Option<u32>,
    observed_delay: Option<u32>,
) -> LaunchRecord {
    LaunchRecord {
        tick,
        src: WorldTile {
            x: src[0],
            z: src[1],
            level: src[2],
        },
        here: WorldTile {
            x: here[0],
            z: here[1],
            level: here[2],
        },
        distance,
        t1,
        t2,
        impact_tick,
        observed_delay,
    }
}

/// The R3-F1 receipt: two identities, distances 4 and 6, but the distance-6
/// delay is 1 instead of `floor((32 + 5*6)/30) == 2`, and that row is after
/// prayers off at tick 43.
fn contradictory_w1_launches() -> Vec<LaunchRecord> {
    vec![
        launch_row(
            20,
            [2839, 3602, 0],
            [2843, 3606, 0],
            4,
            364,
            381,
            Some(21),
            Some(1),
        ),
        launch_row(
            31,
            [2860, 3609, 0],
            [2864, 3609, 0],
            4,
            627,
            644,
            Some(32),
            Some(1),
        ),
        launch_row(
            56,
            [2880, 3602, 0],
            [2880, 3596, 0],
            6,
            1219,
            1240,
            Some(57),
            Some(1),
        ),
    ]
}

#[test]
fn npc_ranged_queue_ticks_matches_the_named_formula() {
    assert_eq!(npc_ranged_queue_ticks(4), 1);
    assert_eq!(npc_ranged_queue_ticks(6), 2);
    assert_eq!(npc_ranged_queue_ticks(8), 2);
}

#[test]
fn count_only_queue_gate_accepts_the_contradictory_w1_receipt() {
    let launches = contradictory_w1_launches();
    assert!(
        count_only_queue_gate(&launches),
        "the old count-only gate must still describe the receipt it wrongly passed"
    );
    assert_eq!(
        queue_mismatch(&launches).as_deref(),
        Some("NPC ranged queue delay 1 at distance 6 != 2")
    );
}

#[test]
fn npc_ranged_queue_gate_rejects_a_mismatched_delay_the_count_only_gate_accepted() {
    let launches = contradictory_w1_launches();
    assert!(count_only_queue_gate(&launches));
    assert!(
        !queue_gate(&launches, Some(43)),
        "a distance-6 delay of 1 must not pass floor((32 + 5d)/30)"
    );
}

#[test]
fn npc_ranged_queue_gate_requires_two_in_crossing_distances_that_match_the_rule() {
    let matching = vec![
        launch_row(
            20,
            [2839, 3602, 0],
            [2843, 3606, 0],
            4,
            364,
            381,
            Some(21),
            Some(1),
        ),
        launch_row(
            31,
            [2855, 3607, 0],
            [2855, 3601, 0],
            6,
            627,
            657,
            Some(33),
            Some(2),
        ),
    ];
    assert!(queue_gate(&matching, Some(43)));

    let after_off = vec![
        launch_row(
            20,
            [2839, 3602, 0],
            [2843, 3606, 0],
            4,
            364,
            381,
            Some(21),
            Some(1),
        ),
        launch_row(
            56,
            [2880, 3602, 0],
            [2880, 3596, 0],
            6,
            1219,
            1240,
            Some(58),
            Some(2),
        ),
    ];
    assert!(
        !queue_gate(&after_off, Some(43)),
        "a second distance after prayers are off is not an in-crossing proof"
    );
}

#[test]
fn due_tick_attribution_does_not_attach_an_unrelated_hitmark_to_the_oldest_launch() {
    let mut launches = vec![launch_row(
        20,
        [2880, 3602, 0],
        [2880, 3596, 0],
        6,
        1219,
        1240,
        None,
        None,
    )];
    attribute_due_impact(&mut launches, 21);
    assert!(
        launches[0].impact_tick.is_none(),
        "a delay-1 hitmark is not the distance-6 rock"
    );
    attribute_due_impact(&mut launches, 22);
    assert_eq!(launches[0].impact_tick, Some(22));
    assert_eq!(launches[0].observed_delay, Some(2));
}

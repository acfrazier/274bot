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

fn evidence_dir() -> Result<PathBuf, String> {
    std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "W1 live proof requires LIVE_EVIDENCE_DIR".to_owned())
}
const MISSILES_VARP: i32 = 96;
const MISSILES_BUTTON: i32 = 5622;
const PRAYER_STABLE_TICKS: u32 = 3;
const STOP_OFF_BUDGET_TICKS: u32 = 4;
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

fn default_nav_pack_path() -> PathBuf {
    if let Some(path) = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .filter(|path| path.exists())
    {
        return path;
    }
    let profile_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .join(option_env!("PROFILE").unwrap_or("debug"))
        });
    profile_dir.join("nav/289/274bot.navpack")
}

#[test]
fn death_plateau_throwers_has_a_standable_protected_crossing() {
    let pack = default_nav_pack_path();
    if !pack.exists() {
        panic!("W1 pack missing at {}", pack.display());
    }
    let world = nav::world::NavWorld::load_pack(&pack).expect("load baked 289 pack");
    let (start, dest) = pick_crossing(&world);
    let attacker_stage = thrower_stage_tile(&world, start, dest);
    assert!(world.collision.standable(attacker_stage));
    assert!((3..=5).contains(&chebyshev(start, attacker_stage)));
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
        let path = evidence_dir()?
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
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
        .is_some_and(|tile| tile.level == dest.level && chebyshev(tile, dest) <= 1)
}

fn thrower_stage_tile(
    world: &nav::world::NavWorld,
    start: WorldTile,
    dest: WorldTile,
) -> WorldTile {
    let direction = (dest.x - start.x).signum();
    for distance in 3..=5 {
        for lateral in [0, 1, -1, 2, -2] {
            let tile = WorldTile {
                x: start.x + direction * distance,
                z: start.z + lateral,
                level: start.level,
            };
            if world.collision.standable(tile) {
                return tile;
            }
        }
    }
    panic!("no standable thrower stage tile at least three tiles ahead of {start:?}");
}

fn teleport_command(tile: WorldTile) -> String {
    format!(
        "tele {},{},{},{},{}",
        tile.level,
        tile.x.div_euclid(64),
        tile.z.div_euclid(64),
        tile.x.rem_euclid(64),
        tile.z.rem_euclid(64)
    )
}

fn final_w1_snapshot(snapshot: &GameSnapshot) -> serde_json::Value {
    let mut facts = combat_proof::snapshot_facts(snapshot, Some(u64::from(snapshot.tick())));
    facts["network_tile"] = facts["tile"].take();
    facts["tile"] = facts
        .pointer("/local_player/tile")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    facts
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrayerPhase {
    /// Older per-tick receipts did not distinguish crossing from cleanup.
    Unspecified,
    Crossing,
    Cleanup,
}

impl PrayerPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "observed",
            Self::Crossing => "crossing",
            Self::Cleanup => "cleanup",
        }
    }
    fn order(self) -> u8 {
        match self {
            Self::Crossing => 0,
            Self::Unspecified => 1,
            Self::Cleanup => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrayerObservation {
    tick: u32,
    phase: PrayerPhase,
    missiles_on: Option<bool>,
    prayers_active: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GuardClick {
    tick: u32,
    component_id: Option<i32>,
    missiles_on_before: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum W1Mode {
    Crossing,
    StopMidCrossing,
}

struct W1GateInput<'a> {
    started: bool,
    launches: &'a [LaunchRecord],
    protect_tick: Option<u32>,
    arrived: bool,
    arrival_tick: Option<u32>,
    prayers_off_after_arrival: bool,
    off_tick: Option<u32>,
    prayer_ticks: &'a [PrayerObservation],
    guard_clicks: &'a [GuardClick],
    attack_emitted: Option<bool>,
    current_tick: u32,
    legacy_sparse: bool,
    stop_requested: bool,
    stop_tick: Option<u32>,
    stop_arrived_before_stop: bool,
    stop_off_tick: Option<u32>,
    stop_error: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
enum W1GateDecision {
    Pending,
    Pass,
    Fail(String),
}

fn gate_fail(reason: impl Into<String>) -> W1GateDecision {
    W1GateDecision::Fail(reason.into())
}

fn exact_prayer_observation(
    observations: &[PrayerObservation],
    tick: u32,
) -> Option<PrayerObservation> {
    observations
        .iter()
        .rev()
        .find(|row| row.tick == tick)
        .copied()
}

fn crossing_prayer_observation(
    observations: &[PrayerObservation],
    tick: u32,
) -> Option<PrayerObservation> {
    observations
        .iter()
        .rev()
        .find(|row| {
            row.tick == tick
                && matches!(row.phase, PrayerPhase::Crossing | PrayerPhase::Unspecified)
        })
        .copied()
}

fn crossing_observations_all_on(observations: &[PrayerObservation], tick: u32) -> Option<bool> {
    let mut rows = observations.iter().filter(|row| {
        row.tick == tick && matches!(row.phase, PrayerPhase::Crossing | PrayerPhase::Unspecified)
    });
    let first = rows.next()?;
    Some(
        std::iter::once(first)
            .chain(rows)
            .all(|row| row.missiles_on == Some(true)),
    )
}

/// Reclassify only a same-tick false sample as cleanup when the off-click's
/// captured pre-click state proves Missiles was still on at that tick.
fn normalize_same_tick_cleanup(
    observations: &mut [PrayerObservation],
    off_tick: Option<u32>,
    click_crossing_observations: &[PrayerObservation],
) {
    let Some(off_tick) = off_tick else {
        return;
    };
    if !click_crossing_observations
        .iter()
        .any(|row| row.tick == off_tick && row.missiles_on == Some(true))
    {
        return;
    }
    for observation in observations.iter_mut().filter(|row| {
        row.tick == off_tick
            && row.phase == PrayerPhase::Unspecified
            && row.missiles_on == Some(false)
    }) {
        observation.phase = PrayerPhase::Cleanup;
    }
}

fn cleanup_prayer_observation(
    observations: &[PrayerObservation],
    tick: u32,
) -> Option<PrayerObservation> {
    observations
        .iter()
        .rev()
        .find(|row| {
            row.tick == tick && matches!(row.phase, PrayerPhase::Cleanup | PrayerPhase::Unspecified)
        })
        .copied()
}

fn missiles_on_at(input: &W1GateInput<'_>, tick: u32) -> Option<bool> {
    let observation = if input.legacy_sparse {
        exact_prayer_observation(input.prayer_ticks, tick)
    } else {
        crossing_prayer_observation(input.prayer_ticks, tick)
    };
    if let Some(on) = observation.and_then(|row| row.missiles_on) {
        return Some(on);
    }
    if !input.legacy_sparse {
        return None;
    }
    let latest = input
        .prayer_ticks
        .iter()
        .filter(|row| row.tick <= tick && row.missiles_on.is_some())
        .max_by_key(|row| row.tick)?;
    if input
        .guard_clicks
        .iter()
        .any(|click| click.tick > latest.tick && click.tick <= tick)
    {
        return None;
    }
    latest.missiles_on
}

fn prayer_active_at(input: &W1GateInput<'_>, tick: u32) -> Option<bool> {
    cleanup_prayer_observation(input.prayer_ticks, tick)?.prayers_active
}

fn require_crossing_guard_clicks(
    input: &W1GateInput<'_>,
    first_launch: u32,
    arrival_tick: u32,
    off_tick: u32,
) -> Result<(), String> {
    if input.guard_clicks.len() != 2 {
        return Err(format!(
            "W1 requires exactly two guard clicks (one enable and one off-click), observed {}",
            input.guard_clicks.len()
        ));
    }

    let mut enables = 0;
    let mut off_clicks = 0;
    for click in input.guard_clicks {
        if click.component_id != Some(MISSILES_BUTTON) {
            return Err("non-Missiles guard click during the crossing".into());
        }
        match click.missiles_on_before {
            Some(false) if (first_launch..=arrival_tick).contains(&click.tick) => {
                enables += 1;
            }
            Some(true) if (arrival_tick..=off_tick).contains(&click.tick) => {
                off_clicks += 1;
            }
            Some(false) => return Err("Missiles enable click was outside the crossing".into()),
            Some(true) => return Err("Missiles off-click was outside arrival cleanup".into()),
            None => return Err("guard click has no pre-click Missiles observation".into()),
        }
    }
    if enables != 1 || off_clicks != 1 {
        return Err(format!(
            "W1 requires one Missiles enable and one off-click, observed {enables} enables and {off_clicks} off-clicks"
        ));
    }
    Ok(())
}

fn stable_off_decision(
    input: &W1GateInput<'_>,
    off_tick: u32,
    current_tick: u32,
) -> W1GateDecision {
    if input.legacy_sparse {
        if !input.prayers_off_after_arrival {
            return gate_fail("legacy receipt does not record prayers off");
        }
        if input.guard_clicks.iter().any(|click| click.tick > off_tick) {
            return gate_fail("guard prayer click followed the legacy off observation");
        }
        if input
            .prayer_ticks
            .iter()
            .any(|row| row.tick > off_tick && row.prayers_active == Some(true))
        {
            return gate_fail("a prayer returned after the legacy off observation");
        }
        return W1GateDecision::Pass;
    }

    let stable_through = off_tick.saturating_add(PRAYER_STABLE_TICKS);
    for tick in off_tick..=stable_through {
        if tick > current_tick {
            return W1GateDecision::Pending;
        }
        let Some(active) = prayer_active_at(input, tick) else {
            return gate_fail(format!("missing prayer observation at tick {tick}"));
        };
        if active {
            return gate_fail(format!("a prayer returned after off_tick at tick {tick}"));
        }
    }
    W1GateDecision::Pass
}

fn evaluate_w1_gate(mode: W1Mode, input: &W1GateInput<'_>) -> W1GateDecision {
    if input.attack_emitted == Some(true) {
        return gate_fail("W1 emitted Attack");
    }
    if input.attack_emitted.is_none() {
        return gate_fail("W1 receipt does not establish the no-Attack proof");
    }
    if let Some(reason) = queue_mismatch(input.launches) {
        return gate_fail(reason);
    }
    let Some(first_index) = input
        .launches
        .iter()
        .enumerate()
        .min_by_key(|(_, launch)| launch.tick)
        .map(|(index, _)| index)
    else {
        return W1GateDecision::Pending;
    };
    let first_launch = input.launches[first_index].tick;
    let Some(protect_tick) = input.protect_tick else {
        return W1GateDecision::Pending;
    };
    if !protect_within_two_ticks(first_launch, protect_tick) {
        return gate_fail(format!(
            "Protect from Missiles {protect_tick} was not within 2 ticks of launch {first_launch}"
        ));
    }

    match mode {
        W1Mode::Crossing => evaluate_crossing_gate(input, first_index, first_launch),
        W1Mode::StopMidCrossing => evaluate_stop_gate(input, first_launch),
    }
}

fn evaluate_crossing_gate(
    input: &W1GateInput<'_>,
    first_index: usize,
    first_launch: u32,
) -> W1GateDecision {
    let arrival_bound = input.arrival_tick.or(input.off_tick);
    let click_bound = arrival_bound.unwrap_or(input.current_tick);
    if input.guard_clicks.iter().any(|click| {
        click.tick >= first_launch
            && click.tick <= click_bound
            && click.component_id != Some(MISSILES_BUTTON)
    }) {
        return gate_fail("non-Missiles guard click during the crossing");
    }

    for (index, launch) in input.launches.iter().enumerate() {
        if index == first_index
            || launch.tick <= first_launch
            || arrival_bound.is_some_and(|arrival| launch.tick > arrival)
            || input.off_tick.is_some_and(|off| launch.tick >= off)
        {
            continue;
        }
        match missiles_on_at(input, launch.tick) {
            Some(true) => {}
            Some(false) => {
                return gate_fail(format!(
                    "Missiles was off at in-crossing launch tick {}",
                    launch.tick
                ));
            }
            None => {
                return gate_fail(format!(
                    "no observable Missiles state at in-crossing launch tick {}",
                    launch.tick
                ));
            }
        }
    }

    if !input.started || !input.arrived || !input.prayers_off_after_arrival {
        return W1GateDecision::Pending;
    }
    let Some(off_tick) = input.off_tick else {
        return W1GateDecision::Pending;
    };
    if !queue_gate(input.launches, Some(off_tick)) {
        return W1GateDecision::Pending;
    }
    let Some(arrival_tick) = arrival_bound else {
        return W1GateDecision::Pending;
    };
    let Some(protect_tick) = input.protect_tick else {
        return W1GateDecision::Pending;
    };
    if protect_tick > arrival_tick {
        return gate_fail("Protect from Missiles was first observed after arrival");
    }
    if let Err(reason) = require_crossing_guard_clicks(input, first_launch, arrival_tick, off_tick)
    {
        return gate_fail(reason);
    }
    if !input.legacy_sparse {
        for tick in protect_tick..=arrival_tick {
            if tick > input.current_tick {
                return W1GateDecision::Pending;
            }
            match crossing_observations_all_on(input.prayer_ticks, tick) {
                Some(true) => {}
                Some(false) => {
                    return gate_fail(format!("Missiles was off during crossing at tick {tick}"));
                }
                None => {
                    return gate_fail(format!(
                        "missing crossing prayer observation at tick {tick}"
                    ));
                }
            }
        }
        for tick in first_launch..=off_tick.saturating_add(PRAYER_STABLE_TICKS) {
            if tick > input.current_tick {
                return W1GateDecision::Pending;
            }
            if exact_prayer_observation(input.prayer_ticks, tick).is_none() {
                return gate_fail(format!(
                    "missing per-tick prayer observation from launch through cleanup at tick {tick}"
                ));
            }
        }
    }
    stable_off_decision(input, off_tick, input.current_tick)
}

fn evaluate_stop_gate(input: &W1GateInput<'_>, first_launch: u32) -> W1GateDecision {
    if let Some(error) = input.stop_error {
        return gate_fail(format!("Stop mid-crossing failed: {error}"));
    }
    let Some(stop_tick) = input.stop_tick else {
        if input.arrived {
            return gate_fail("the route arrived before Stop was requested");
        }
        return W1GateDecision::Pending;
    };
    if !input.stop_requested || !input.stop_arrived_before_stop || stop_tick < first_launch {
        return gate_fail("Stop was not requested mid-crossing before arrival");
    }
    if missiles_on_at(input, stop_tick) != Some(true) {
        return gate_fail("Stop was not requested after Missiles was observed on");
    }
    let Some(off_tick) = input.stop_off_tick else {
        if input.current_tick > stop_tick.saturating_add(STOP_OFF_BUDGET_TICKS) {
            return gate_fail("prayers did not turn off within four ticks of Stop");
        }
        return W1GateDecision::Pending;
    };
    if off_tick > stop_tick.saturating_add(STOP_OFF_BUDGET_TICKS) {
        return gate_fail("prayers did not turn off within four ticks of Stop");
    }
    stable_off_decision(input, off_tick, input.current_tick)
}

struct ParsedW1Receipt {
    started: bool,
    account: Option<String>,
    recorded_outcome: Option<String>,
    launches: Vec<LaunchRecord>,
    protect_tick: Option<u32>,
    arrived: bool,
    arrival_tick: Option<u32>,
    prayers_off_after_arrival: bool,
    off_tick: Option<u32>,
    prayer_ticks: Vec<PrayerObservation>,
    guard_clicks: Vec<GuardClick>,
    attack_emitted: Option<bool>,
    current_tick: u32,
    legacy_sparse: bool,
}

impl ParsedW1Receipt {
    fn gate_input(&self) -> W1GateInput<'_> {
        W1GateInput {
            started: self.started,
            launches: &self.launches,
            protect_tick: self.protect_tick,
            arrived: self.arrived,
            arrival_tick: self.arrival_tick,
            prayers_off_after_arrival: self.prayers_off_after_arrival,
            off_tick: self.off_tick,
            prayer_ticks: &self.prayer_ticks,
            guard_clicks: &self.guard_clicks,
            attack_emitted: self.attack_emitted,
            current_tick: self.current_tick,
            legacy_sparse: self.legacy_sparse,
            stop_requested: false,
            stop_tick: None,
            stop_arrived_before_stop: false,
            stop_off_tick: None,
            stop_error: None,
        }
    }
}

fn parse_receipt_tick(value: &serde_json::Value, field: &str) -> Result<u32, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|tick| u32::try_from(tick).ok())
        .ok_or_else(|| format!("receipt has no valid {field}"))
}

fn parse_tile_array(value: &serde_json::Value) -> Option<WorldTile> {
    let values = value.as_array()?;
    Some(WorldTile {
        x: i32::try_from(values.first()?.as_i64()?).ok()?,
        z: i32::try_from(values.get(1)?.as_i64()?).ok()?,
        level: i32::try_from(values.get(2)?.as_i64()?).ok()?,
    })
}

fn parse_launch_receipt(value: &serde_json::Value) -> Result<LaunchRecord, String> {
    let tile = |name: &str| {
        value
            .get(name)
            .and_then(parse_tile_array)
            .ok_or_else(|| format!("launch receipt has no valid {name}"))
    };
    let integer = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_i64)
            .and_then(|number| i32::try_from(number).ok())
            .ok_or_else(|| format!("launch receipt has no valid {name}"))
    };
    Ok(LaunchRecord {
        tick: parse_receipt_tick(value, "tick")?,
        src: tile("src")?,
        here: tile("here")?,
        distance: integer("distance")?,
        t1: integer("t1")?,
        t2: integer("t2")?,
        impact_tick: value
            .get("impact_tick")
            .and_then(serde_json::Value::as_u64)
            .and_then(|tick| u32::try_from(tick).ok()),
        observed_delay: value
            .get("observed_delay")
            .and_then(serde_json::Value::as_u64)
            .and_then(|delay| u32::try_from(delay).ok()),
    })
}

fn prayer_observation_from_snapshot(
    snapshot: &serde_json::Value,
    tick: u32,
) -> Option<PrayerObservation> {
    let varps = snapshot.get("prayer_varps")?.as_array()?;
    let read = |index: i32| {
        varps
            .iter()
            .find(|row| row.get("index").and_then(serde_json::Value::as_i64) == Some(index as i64))
            .and_then(|row| row.get("value"))
            .and_then(serde_json::Value::as_i64)
            .map(|value| value == 1)
    };
    let missiles_on = read(MISSILES_VARP);
    let prayer_values: [Option<bool>; 15] = std::array::from_fn(|offset| read(83 + offset as i32));
    let prayers_active = if prayer_values.contains(&Some(true)) {
        Some(true)
    } else if prayer_values.iter().all(Option::is_some) {
        Some(false)
    } else {
        None
    };
    Some(PrayerObservation {
        tick,
        phase: PrayerPhase::Unspecified,
        missiles_on,
        prayers_active,
    })
}

fn parse_prayer_phase(value: &serde_json::Value) -> Result<PrayerPhase, String> {
    match value.get("phase").and_then(serde_json::Value::as_str) {
        None | Some("observed") => Ok(PrayerPhase::Unspecified),
        Some("crossing") => Ok(PrayerPhase::Crossing),
        Some("cleanup") => Ok(PrayerPhase::Cleanup),
        Some(phase) => Err(format!("unknown prayer observation phase {phase}")),
    }
}

fn receipt_snapshot_at_dest(snapshot: &serde_json::Value, dest: WorldTile) -> bool {
    snapshot
        .get("tile")
        .and_then(parse_tile_array)
        .is_some_and(|tile| tile.level == dest.level && chebyshev(tile, dest) <= 1)
}

fn parse_w1_receipt(value: &serde_json::Value) -> Result<ParsedW1Receipt, String> {
    let launches = value
        .get("launches")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "receipt has no launches array".to_owned())?
        .iter()
        .map(parse_launch_receipt)
        .collect::<Result<Vec<_>, _>>()?;
    let dest = value.get("dest").and_then(parse_tile_array);
    let guard_rows = value
        .get("guard_clicks")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "receipt has no guard_clicks array".to_owned())?;
    let mut guard_clicks = Vec::with_capacity(guard_rows.len());
    let mut legacy_observations = Vec::with_capacity(guard_rows.len());
    let mut click_crossing_observations = Vec::with_capacity(guard_rows.len());
    for row in guard_rows {
        if row.get("accepted").and_then(serde_json::Value::as_bool) == Some(false) {
            continue;
        }
        let tick = parse_receipt_tick(row, "tick")?;
        let component_id = row
            .get("component_id")
            .and_then(serde_json::Value::as_i64)
            .and_then(|id| i32::try_from(id).ok());
        let click_observation = row
            .get("snapshot")
            .and_then(|snapshot| prayer_observation_from_snapshot(snapshot, tick));
        guard_clicks.push(GuardClick {
            tick,
            component_id,
            missiles_on_before: click_observation.and_then(|observation| observation.missiles_on),
        });
        if let Some(observation) = click_observation {
            legacy_observations.push(observation);
            if component_id == Some(MISSILES_BUTTON)
                && observation.missiles_on == Some(true)
                && row.get("snapshot").is_some_and(|snapshot| {
                    dest.is_some_and(|dest| receipt_snapshot_at_dest(snapshot, dest))
                })
            {
                let mut crossing = observation;
                crossing.phase = PrayerPhase::Crossing;
                click_crossing_observations.push(crossing);
            }
        }
    }

    let protect_tick = value
        .get("protect_tick")
        .and_then(serde_json::Value::as_u64)
        .and_then(|tick| u32::try_from(tick).ok());
    let off_tick = value
        .get("off_tick")
        .and_then(serde_json::Value::as_u64)
        .and_then(|tick| u32::try_from(tick).ok());
    let prayers_off_after_arrival = value["prayers_off_after_arrival"]
        .as_bool()
        .unwrap_or(false);
    let per_tick_rows = value
        .get("prayer_ticks")
        .and_then(serde_json::Value::as_array);
    let legacy_sparse = per_tick_rows.is_none_or(Vec::is_empty);
    let mut prayer_ticks = if let Some(rows) = per_tick_rows {
        rows.iter()
            .map(|row| {
                Ok(PrayerObservation {
                    tick: parse_receipt_tick(row, "tick")?,
                    phase: parse_prayer_phase(row)?,
                    missiles_on: row.get("missiles_on").and_then(serde_json::Value::as_bool),
                    prayers_active: row
                        .get("prayers_active")
                        .and_then(serde_json::Value::as_bool),
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        Vec::new()
    };
    if legacy_sparse {
        prayer_ticks.extend(legacy_observations);
        if let Some(tick) = protect_tick {
            prayer_ticks.push(PrayerObservation {
                tick,
                phase: PrayerPhase::Crossing,
                missiles_on: Some(true),
                prayers_active: Some(true),
            });
        }
        if prayers_off_after_arrival {
            if let Some(tick) = off_tick {
                prayer_ticks.push(PrayerObservation {
                    tick,
                    phase: PrayerPhase::Cleanup,
                    missiles_on: Some(false),
                    prayers_active: Some(false),
                });
            }
        }
    }
    if !legacy_sparse {
        normalize_same_tick_cleanup(&mut prayer_ticks, off_tick, &click_crossing_observations);
    }
    prayer_ticks.extend(click_crossing_observations);
    prayer_ticks.sort_by_key(|row| {
        // Sparse protect_tick is the post-enable observation, later than
        // that tick's pre-click snapshot. Cleanup still follows both.
        let phase_order = match (legacy_sparse, row.phase) {
            (true, PrayerPhase::Unspecified) => 0,
            (true, PrayerPhase::Crossing) => 1,
            (_, phase) => phase.order(),
        };
        (row.tick, phase_order)
    });

    let arrival_tick = value
        .get("arrival_tick")
        .and_then(serde_json::Value::as_u64)
        .and_then(|tick| u32::try_from(tick).ok())
        .or_else(|| {
            let dest = dest?;
            guard_rows.iter().find_map(|row| {
                let accepted = row.get("accepted").and_then(serde_json::Value::as_bool);
                if accepted == Some(false) {
                    return None;
                }
                let tick = row
                    .get("tick")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|tick| u32::try_from(tick).ok())?;
                let snapshot = row.get("snapshot")?;
                receipt_snapshot_at_dest(snapshot, dest).then_some(tick)
            })
        })
        .or(off_tick);

    let attack_emitted = value
        .get("attack_emitted")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            value
                .get("actions")
                .and_then(serde_json::Value::as_array)
                .map(|actions| {
                    actions.iter().any(|action| {
                        action["request"]["op"] == json!("npc")
                            && action["request"]["action"] == json!("Attack")
                    })
                })
        })
        .or_else(|| {
            value
                .get("action_kinds")
                .and_then(serde_json::Value::as_array)
                .map(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| !matches!(kind.as_str(), Some("walk" | "guard")))
                })
        });
    let current_tick = value
        .get("current_tick")
        .and_then(serde_json::Value::as_u64)
        .and_then(|tick| u32::try_from(tick).ok())
        .or_else(|| prayer_ticks.last().map(|row| row.tick))
        .or(off_tick)
        .unwrap_or_default();
    Ok(ParsedW1Receipt {
        started: value["started"].as_bool().unwrap_or(false),
        account: value["account"].as_str().map(str::to_owned),
        recorded_outcome: value["outcome"].as_str().map(str::to_owned),
        launches,
        protect_tick,
        arrived: value["arrived"].as_bool().unwrap_or(false),
        arrival_tick,
        prayers_off_after_arrival,
        off_tick,
        prayer_ticks,
        guard_clicks,
        attack_emitted,
        current_tick,
        legacy_sparse,
    })
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

fn earliest_launch_tick(launches: &[LaunchRecord]) -> Option<u32> {
    launches.iter().map(|launch| launch.tick).min()
}

fn protect_within_two_ticks(launch_tick: u32, protect_tick: u32) -> bool {
    protect_tick.abs_diff(launch_tick) <= 2
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
    let pending: Vec<usize> = launches
        .iter()
        .enumerate()
        .filter(|(_, launch)| launch.impact_tick.is_none() && launch.tick <= tick)
        .map(|(index, _)| index)
        .collect();
    let due: Vec<usize> = pending
        .iter()
        .copied()
        .filter(|&index| {
            tick.saturating_sub(launches[index].tick) == launches[index].expected_delay()
        })
        .collect();
    let index = match (due.len(), pending.len()) {
        (1, _) => due[0],
        (0, 1) => pending[0],
        _ => return,
    };
    let launch = &mut launches[index];
    launch.impact_tick = Some(tick);
    launch.observed_delay = Some(tick.saturating_sub(launch.tick));
}

fn guard_click_from_action(action: &serde_json::Value) -> GuardClick {
    let tick = action["tick"].as_u64().unwrap_or_default() as u32;
    let observation = action
        .get("snapshot")
        .and_then(|snapshot| prayer_observation_from_snapshot(snapshot, tick));
    GuardClick {
        tick,
        component_id: action["component_id"].as_i64().map(|id| id as i32),
        missiles_on_before: observation.and_then(|row| row.missiles_on),
    }
}

fn append_guard_crossing_observations(
    prayer_ticks: &mut Vec<PrayerObservation>,
    actions: &[serde_json::Value],
    dest: WorldTile,
    off_tick: Option<u32>,
) {
    let mut click_crossing_observations = Vec::new();
    for action in actions
        .iter()
        .filter(|action| action["kind"] == json!("guard"))
    {
        let click = guard_click_from_action(action);
        if click.component_id != Some(MISSILES_BUTTON)
            || click.missiles_on_before != Some(true)
            || !action
                .get("snapshot")
                .is_some_and(|snapshot| receipt_snapshot_at_dest(snapshot, dest))
        {
            continue;
        }
        if let Some(mut observation) = action
            .get("snapshot")
            .and_then(|snapshot| prayer_observation_from_snapshot(snapshot, click.tick))
        {
            observation.phase = PrayerPhase::Crossing;
            click_crossing_observations.push(observation);
        }
    }
    normalize_same_tick_cleanup(prayer_ticks, off_tick, &click_crossing_observations);
    for observation in click_crossing_observations {
        if !prayer_ticks.contains(&observation) {
            prayer_ticks.push(observation);
        }
    }
    prayer_ticks.sort_by_key(|row| (row.tick, row.phase.order()));
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
    attacker_stage: WorldTile,
    launch_tick: Option<u32>,
    protect_tick: Option<u32>,
    started: bool,
    arrived: bool,
    arrival_tick: Option<u32>,
    last_tile: Option<(i32, i32, i32)>,
    attacker_staged: bool,
    prayers_off_after_arrival: bool,
    off_tick: Option<u32>,
    prayer_ticks: Vec<PrayerObservation>,
    stop_requested: bool,
    stop_tick: Option<u32>,
    stop_arrived_before_stop: bool,
    stop_off_tick: Option<u32>,
    stop_error: Option<String>,
    launches: Vec<LaunchRecord>,
    seen_launches: std::collections::HashSet<(i32, i32, i32, i32, i32)>,
    hit_onset: HitOnset,
    second_attacker_staged: bool,
    tiles_by_tick: std::collections::VecDeque<(u32, WorldTile)>,
}

impl LiveState {
    fn remember_prayer_observation(&mut self, tick: u32, phase: PrayerPhase) {
        let observation = PrayerObservation {
            tick,
            phase,
            missiles_on: Some(missiles_on(&self.snapshot)),
            prayers_active: Some(prayers_active(&self.snapshot)),
        };
        if let Some(existing) = self
            .prayer_ticks
            .iter_mut()
            .rev()
            .find(|row| row.tick == tick && row.phase == phase)
        {
            *existing = observation;
        } else {
            self.prayer_ticks.push(observation);
        }
    }

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
        self.last_tile = self.snapshot.local_player().map(|player| {
            let tile = player.player.actor.tile;
            (tile.x, tile.z, tile.level)
        });
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
        if self.started && !self.attacker_staged {
            let stage_command = teleport_command(self.attacker_stage);
            let start_command = teleport_command(self.start);
            if send_cheat(client, &stage_command)
                && send_cheat(client, "npcadd death_troll_thrower1")
                && send_cheat(client, &start_command)
            {
                self.attacker_staged = true;
            } else {
                combat_proof::mark_invalid(
                    &self.account,
                    "could not stage the first ranged thrower away from the route start",
                );
                return;
            }
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
        self.launch_tick = earliest_launch_tick(&self.launches);
        if self.started && missiles_on(&self.snapshot) && self.protect_tick.is_none() {
            self.protect_tick = Some(self.snapshot.tick());
        }
        if arrived_at(&self.snapshot, self.dest) {
            self.arrival_tick.get_or_insert(tick);
            self.arrived = true;
        }
        if self.arrived && !prayers_active(&self.snapshot) {
            self.off_tick.get_or_insert(tick);
            self.prayers_off_after_arrival = true;
        }
        if self.stop_requested && self.stop_off_tick.is_none() && !prayers_active(&self.snapshot) {
            self.stop_off_tick = Some(tick);
        }
        if self.started {
            let phase = if !prayers_active(&self.snapshot) && (self.arrived || self.stop_requested)
            {
                PrayerPhase::Cleanup
            } else {
                PrayerPhase::Crossing
            };
            self.remember_prayer_observation(tick, phase);
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
    run_live_w1(W1Mode::Crossing);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_walk_guard_w1_stop_mid_crossing() {
    run_live_w1(W1Mode::StopMidCrossing);
}

fn run_live_w1(mode: W1Mode) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "W1 requires LIVE=1"
    );
    std::env::set_var("BOT_LIVE_NAME_PREFIX", "wg");
    let evidence = evidence_dir().expect("W1 live prerequisites");
    std::fs::create_dir_all(&evidence).expect("create W1 lifecycle evidence directory");
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
        attacker_stage: thrower_stage_tile(world.as_ref(), start, dest),
        launch_tick: None,
        protect_tick: None,
        started: false,
        arrived: false,
        arrival_tick: None,
        last_tile: None,
        attacker_staged: false,
        prayers_off_after_arrival: false,
        off_tick: None,
        prayer_ticks: Vec::new(),
        stop_requested: false,
        stop_tick: None,
        stop_arrived_before_stop: false,
        stop_off_tick: None,
        stop_error: None,
        launches: Vec::new(),
        seen_launches: std::collections::HashSet::new(),
        hit_onset: HitOnset::new(),
        second_attacker_staged: false,
        tiles_by_tick: std::collections::VecDeque::new(),
    }));
    let frame_state = Arc::clone(&state);
    let frame_buffer = FrameBuf::new();
    let shot_buffer = Arc::clone(&frame_buffer);
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
        let stop_candidate = {
            let current = state.lock().unwrap_or_else(|e| e.into_inner());
            if mode == W1Mode::StopMidCrossing
                && current.started
                && current.launch_tick.is_some()
                && !current.arrived
                && !current.stop_requested
                && current.stop_error.is_none()
                && missiles_on(&current.snapshot)
            {
                current.start_context.as_ref().map(|(handle, _)| {
                    (
                        handle.clone(),
                        current.account.clone(),
                        current.snapshot.tick(),
                    )
                })
            } else {
                None
            }
        };
        if let Some((handle, name, observed_tick)) = stop_candidate {
            let result = handle.stop(&name);
            let mut current = state.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(()) => {
                    current.stop_requested = true;
                    current.stop_tick = Some(observed_tick);
                    current.stop_arrived_before_stop =
                        !current.arrived && current.arrival_tick.is_none();
                }
                Err(error) => current.stop_error = Some(error),
            }
        }

        let current = state.lock().unwrap_or_else(|e| e.into_inner());
        let capture_snapshot = capture.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reason) = capture_snapshot.invalid_reason.clone() {
            break ("INVALID", Some(reason));
        }
        let guard_clicks: Vec<GuardClick> = capture_snapshot
            .actions
            .iter()
            .filter(|action| action["kind"] == json!("guard"))
            .map(guard_click_from_action)
            .collect();
        let mut prayer_ticks = current.prayer_ticks.clone();
        append_guard_crossing_observations(
            &mut prayer_ticks,
            &capture_snapshot.actions,
            current.dest,
            current.off_tick,
        );
        let attack_emitted = capture_snapshot.actions.iter().any(|action| {
            action["request"]["op"] == json!("npc")
                && action["request"]["action"] == json!("Attack")
        });
        let gate_input = W1GateInput {
            started: current.started,
            launches: &current.launches,
            protect_tick: current.protect_tick,
            arrived: current.arrived,
            arrival_tick: current.arrival_tick,
            prayers_off_after_arrival: current.prayers_off_after_arrival,
            off_tick: current.off_tick,
            prayer_ticks: &prayer_ticks,
            guard_clicks: &guard_clicks,
            attack_emitted: Some(attack_emitted),
            current_tick: current.snapshot.tick(),
            legacy_sparse: false,
            stop_requested: current.stop_requested,
            stop_tick: current.stop_tick,
            stop_arrived_before_stop: current.stop_arrived_before_stop,
            stop_off_tick: current.stop_off_tick,
            stop_error: current.stop_error.as_deref(),
        };
        let decision = evaluate_w1_gate(mode, &gate_input);
        let runner_failed = matches!(current.runner.status(), RunnerStatus::Failed(_))
            && !(mode == W1Mode::StopMidCrossing && current.stop_requested);
        drop(capture_snapshot);
        drop(current);
        match decision {
            W1GateDecision::Pass => break ("PASS", None),
            W1GateDecision::Fail(reason) => break ("FAIL", Some(reason)),
            W1GateDecision::Pending => {}
        }
        if Instant::now() >= deadline {
            break ("FAIL", Some("W1 live proof exceeded 240s".into()));
        }
        if runner_failed {
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
    let observation_start = snapshot.launch_tick.unwrap_or_default();
    let observation_end = snapshot
        .off_tick
        .or(snapshot.stop_off_tick)
        .map(|tick| tick.saturating_add(PRAYER_STABLE_TICKS))
        .unwrap_or_else(|| snapshot.snapshot.tick());
    let mut observed_prayer_ticks = snapshot.prayer_ticks.clone();
    append_guard_crossing_observations(
        &mut observed_prayer_ticks,
        &capture_snapshot.actions,
        snapshot.dest,
        snapshot.off_tick.or(snapshot.stop_off_tick),
    );
    let prayer_ticks: Vec<serde_json::Value> = observed_prayer_ticks
        .iter()
        .filter(|row| row.tick >= observation_start && row.tick <= observation_end)
        .map(|row| {
            json!({
                "tick": row.tick,
                "phase": row.phase.as_str(),
                "missiles_on": row.missiles_on,
                "prayers_active": row.prayers_active,
            })
        })
        .collect();
    let attack_emitted = capture_snapshot.actions.iter().any(|action| {
        action["request"]["op"] == json!("npc") && action["request"]["action"] == json!("Attack")
    });
    let mode_name = match mode {
        W1Mode::Crossing => "crossing",
        W1Mode::StopMidCrossing => "stop_mid_crossing",
    };
    let receipt = json!({
        "proof": "WALK-GUARD",
        "case": "W1",
        "mode": mode_name,
        "outcome": outcome,
        "error": error,
        "account": account,
        "zone": ZONE,
        "started": snapshot.started,
        "launch_tick": snapshot.launch_tick,
        "protect_tick": snapshot.protect_tick,
        "arrived": snapshot.arrived,
        "arrival_tick": snapshot.arrival_tick,
        "prayers_off_after_arrival": snapshot.prayers_off_after_arrival,
        "off_tick": snapshot.off_tick,
        "prayer_ticks": prayer_ticks,
        "stop_requested": snapshot.stop_requested,
        "stop_tick": snapshot.stop_tick,
        "stop_arrived_before_stop": snapshot.stop_arrived_before_stop,
        "stop_off_tick": snapshot.stop_off_tick,
        "stop_error": snapshot.stop_error,
        "attack_emitted": attack_emitted,
        "current_tick": snapshot.snapshot.tick(),
        "last_tile": snapshot.last_tile,
        "start": [start.x, start.z, start.level],
        "dest": [dest.x, dest.z, dest.level],
        "attacker_staged": snapshot.attacker_staged,
        "attacker_stage": [
            snapshot.attacker_stage.x,
            snapshot.attacker_stage.z,
            snapshot.attacker_stage.level
        ],
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
        "final_snapshot": final_w1_snapshot(&snapshot.snapshot),
    });
    let path = evidence.join(format!("W1-{account}-receipt.json"));
    let pixels = shot_buffer.snapshot();
    assert_eq!(pixels.len(), 765 * 503, "W1 requires a real rendered frame");
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            0xff,
        ]);
    }
    let capture_dir = evidence.join(format!(
        "WALK-GUARD-LIFECYCLE-1_R2_W1_{mode_name}_{account}_{}",
        scenario::shot::stamp_utc(SystemTime::now())
    ));
    let capture_path = scenario::shot::write_shot(
        &capture_dir,
        if outcome == "PASS" {
            "01-final-protect-off"
        } else {
            "FAIL-final-protect-off"
        },
        &rgba,
        765,
        503,
        &serde_json::to_string_pretty(&receipt).expect("encode W1 shot receipt"),
    )
    .expect("write actual W1 final PNG and matching JSON");
    println!("W1 final capture: {}", capture_path.display());
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
fn npc_ranged_queue_distance_one_has_one_tick_delay() {
    let launch = launch_row(
        20,
        [2845, 3602, 0],
        [2844, 3602, 0],
        1,
        364,
        381,
        Some(21),
        Some(1),
    );
    assert_eq!(npc_ranged_queue_ticks(1), 1);
    assert_eq!(queue_mismatch(&[launch]), None);
}

#[test]
fn npc_ranged_queue_gate_rejects_a_mismatched_delay() {
    let launches = contradictory_w1_launches();
    assert_eq!(
        queue_mismatch(&launches).as_deref(),
        Some("NPC ranged queue delay 1 at distance 6 != 2")
    );
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
    let mut launches = vec![
        launch_row(
            20,
            [2880, 3602, 0],
            [2880, 3596, 0],
            6,
            1219,
            1240,
            None,
            None,
        ),
        launch_row(
            20,
            [2860, 3609, 0],
            [2864, 3609, 0],
            4,
            627,
            644,
            None,
            None,
        ),
    ];
    attribute_due_impact(&mut launches, 24);
    assert!(
        launches[0].impact_tick.is_none() && launches[1].impact_tick.is_none(),
        "two pending launches with no unique due tick stay unmatched"
    );
    attribute_due_impact(&mut launches, 21);
    assert_eq!(launches[1].impact_tick, Some(21));
    assert_eq!(launches[1].observed_delay, Some(1));
    assert!(launches[0].impact_tick.is_none());
    attribute_due_impact(&mut launches, 22);
    assert_eq!(launches[0].impact_tick, Some(22));
    assert_eq!(launches[0].observed_delay, Some(2));
}

#[test]
fn wrong_delay_onset_fails_the_proof_even_after_two_matching_rows() {
    let mut launches = vec![
        launch_row(
            19,
            [2838, 3602, 0],
            [2843, 3606, 0],
            5,
            366,
            384,
            Some(20),
            Some(1),
        ),
        launch_row(
            30,
            [2860, 3609, 0],
            [2864, 3609, 0],
            4,
            628,
            645,
            Some(31),
            Some(1),
        ),
        launch_row(
            33,
            [2868, 3609, 0],
            [2872, 3609, 0],
            4,
            700,
            717,
            None,
            None,
        ),
    ];
    assert!(
        queue_gate(&launches, Some(43)),
        "two matching in-crossing distances still pass before the wrong onset"
    );
    attribute_due_impact(&mut launches, 35);
    assert_eq!(launches[2].impact_tick, Some(35));
    assert_eq!(launches[2].observed_delay, Some(2));
    assert_eq!(
        queue_mismatch(&launches).as_deref(),
        Some("NPC ranged queue delay 2 at distance 4 != 1")
    );
    assert!(
        !queue_gate(&launches, Some(43)),
        "a later wrong delay must not be hidden by earlier matching rows"
    );
}

#[test]
fn first_launch_tick_does_not_skip_a_source_near_the_route_start() {
    let start = WorldTile {
        x: 2838,
        z: 3601,
        level: 0,
    };
    let launches = vec![
        launch_row(
            19,
            [2838, 3602, 0],
            [2843, 3606, 0],
            5,
            366,
            384,
            Some(20),
            Some(1),
        ),
        launch_row(
            30,
            [2860, 3609, 0],
            [2864, 3609, 0],
            4,
            628,
            645,
            Some(31),
            Some(1),
        ),
    ];
    assert_eq!(earliest_launch_tick(&launches), Some(19));
    assert!(
        chebyshev(launches[0].src, start) <= 4,
        "the first recorded launch is next to the route start"
    );
    assert!(
        !protect_within_two_ticks(earliest_launch_tick(&launches).unwrap(), 32),
        "Missiles at tick 32 is a 13-tick delay from the real first launch"
    );
    assert!(
        protect_within_two_ticks(30, 32),
        "selecting the later launch would hide the 13-tick delay"
    );
}

fn lifecycle_launches() -> Vec<LaunchRecord> {
    vec![
        launch_row(
            10,
            [2839, 3602, 0],
            [2843, 3606, 0],
            4,
            400,
            417,
            Some(11),
            Some(1),
        ),
        launch_row(
            15,
            [2855, 3607, 0],
            [2855, 3601, 0],
            6,
            550,
            580,
            Some(17),
            Some(2),
        ),
    ]
}

fn lifecycle_prayer_observations(return_tick: Option<u32>) -> Vec<PrayerObservation> {
    (10..=23)
        .map(|tick| {
            let crossing = tick <= 19;
            let active = tick < 20 || return_tick == Some(tick);
            PrayerObservation {
                tick,
                phase: if crossing {
                    PrayerPhase::Crossing
                } else {
                    PrayerPhase::Cleanup
                },
                missiles_on: Some(crossing),
                prayers_active: Some(active),
            }
        })
        .collect()
}

fn lifecycle_guard_clicks() -> [GuardClick; 2] {
    [
        GuardClick {
            tick: 10,
            component_id: Some(MISSILES_BUTTON),
            missiles_on_before: Some(false),
        },
        GuardClick {
            tick: 20,
            component_id: Some(MISSILES_BUTTON),
            missiles_on_before: Some(true),
        },
    ]
}

fn lifecycle_gate_input<'a>(
    launches: &'a [LaunchRecord],
    prayer_ticks: &'a [PrayerObservation],
    guard_clicks: &'a [GuardClick],
) -> W1GateInput<'a> {
    W1GateInput {
        started: true,
        launches,
        protect_tick: Some(10),
        arrived: true,
        arrival_tick: Some(19),
        prayers_off_after_arrival: true,
        off_tick: Some(20),
        prayer_ticks,
        guard_clicks,
        attack_emitted: Some(false),
        current_tick: 23,
        legacy_sparse: false,
        stop_requested: false,
        stop_tick: None,
        stop_arrived_before_stop: false,
        stop_off_tick: None,
        stop_error: None,
    }
}

#[test]
fn legacy_sparse_protect_confirmation_follows_the_same_tick_enable_snapshot() {
    let launches = lifecycle_launches();
    let receipt = json!({
        "started": true,
        "protect_tick": 10,
        "arrived": true,
        "arrival_tick": 19,
        "prayers_off_after_arrival": true,
        "off_tick": 20,
        "attack_emitted": false,
        "launches": launches.iter().copied().map(LaunchRecord::json).collect::<Vec<_>>(),
        "guard_clicks": [
            {
                "tick": 10,
                "component_id": MISSILES_BUTTON,
                "snapshot": {"prayer_varps": [{"index": MISSILES_VARP, "value": 0}]}
            },
            {
                "tick": 20,
                "component_id": MISSILES_BUTTON,
                "snapshot": {"prayer_varps": [{"index": MISSILES_VARP, "value": 1}]}
            }
        ]
    });
    let parsed = parse_w1_receipt(&receipt).unwrap();
    let input = parsed.gate_input();
    assert!(input.legacy_sparse);
    assert_eq!(missiles_on_at(&input, 10), Some(true));
    assert_eq!(missiles_on_at(&input, 15), Some(true));
    assert_eq!(
        evaluate_w1_gate(W1Mode::Crossing, &input),
        W1GateDecision::Pass
    );
}

#[test]
fn w1_gate_rejects_a_prayer_return_after_the_first_off_tick() {
    let launches = lifecycle_launches();
    let prayer_ticks = lifecycle_prayer_observations(Some(22));
    let guard_clicks = lifecycle_guard_clicks();
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("prayer returned")),
        "a prayer return at tick 22 must fail the shared live gate: {decision:?}"
    );
}

#[test]
fn w1_gate_rejects_a_later_launch_without_missiles() {
    let launches = lifecycle_launches();
    let mut prayer_ticks = lifecycle_prayer_observations(None);
    prayer_ticks[5].missiles_on = Some(false);
    prayer_ticks[5].prayers_active = Some(true);
    let guard_clicks = lifecycle_guard_clicks();
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("Missiles was off")),
        "Missiles must be on for the second in-crossing launch: {decision:?}"
    );
}

#[test]
fn w1_gate_rejects_a_mid_crossing_missiles_lapse_between_launches() {
    let launches = lifecycle_launches();
    let mut prayer_ticks = lifecycle_prayer_observations(None);
    for row in prayer_ticks
        .iter_mut()
        .filter(|row| (12..=13).contains(&row.tick))
    {
        row.missiles_on = Some(false);
        row.prayers_active = Some(false);
    }
    let guard_clicks = lifecycle_guard_clicks();
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("Missiles was off during crossing at tick 12")),
        "every crossing tick must keep Missiles on, even between launches: {decision:?}"
    );
}

#[test]
fn w1_gate_rejects_a_missing_crossing_prayer_observation() {
    let launches = lifecycle_launches();
    let mut prayer_ticks = lifecycle_prayer_observations(None);
    prayer_ticks.retain(|row| row.tick != 12);
    let guard_clicks = lifecycle_guard_clicks();
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("missing crossing prayer observation at tick 12")),
        "a missing crossing tick is not an on observation: {decision:?}"
    );
}

#[test]
fn w1_gate_rejects_an_extra_guard_click() {
    let launches = lifecycle_launches();
    let prayer_ticks = lifecycle_prayer_observations(None);
    let pair = lifecycle_guard_clicks();
    let guard_clicks = [
        pair[0],
        GuardClick {
            tick: 18,
            component_id: Some(MISSILES_BUTTON),
            missiles_on_before: Some(true),
        },
        pair[1],
    ];
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("exactly two guard clicks")),
        "only one enable and one off-click are allowed: {decision:?}"
    );
}

#[test]
fn w1_gate_accepts_arrival_and_cleanup_in_distinct_same_tick_phases() {
    let launches = lifecycle_launches();
    let mut prayer_ticks = lifecycle_prayer_observations(None);
    prayer_ticks.retain(|row| row.tick != 20);
    prayer_ticks.push(PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Crossing,
        missiles_on: Some(true),
        prayers_active: Some(true),
    });
    prayer_ticks.push(PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Cleanup,
        missiles_on: Some(false),
        prayers_active: Some(false),
    });
    let guard_clicks = lifecycle_guard_clicks();
    let mut input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    input.arrival_tick = Some(20);
    assert_eq!(
        evaluate_w1_gate(W1Mode::Crossing, &input),
        W1GateDecision::Pass
    );
}

#[test]
fn w1_gate_rejects_any_off_observation_in_a_crossing_phase() {
    let launches = lifecycle_launches();
    let mut prayer_ticks = lifecycle_prayer_observations(None);
    prayer_ticks.push(PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Crossing,
        missiles_on: Some(true),
        prayers_active: Some(true),
    });
    prayer_ticks.push(PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Crossing,
        missiles_on: Some(false),
        prayers_active: Some(false),
    });
    let guard_clicks = lifecycle_guard_clicks();
    let mut input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    input.arrival_tick = Some(20);
    assert!(
        matches!(
            evaluate_w1_gate(W1Mode::Crossing, &input),
            W1GateDecision::Fail(reason) if reason.contains("Missiles was off during crossing at tick 20")
        ),
        "every crossing-phase row, not just one selected row, must show Missiles on"
    );
}

#[test]
fn w1_gate_rejects_a_non_missiles_guard_click_during_the_crossing() {
    let launches = lifecycle_launches();
    let prayer_ticks = lifecycle_prayer_observations(None);
    let pair = lifecycle_guard_clicks();
    let guard_clicks = [
        pair[0],
        GuardClick {
            tick: 18,
            component_id: Some(5623),
            missiles_on_before: Some(true),
        },
        pair[1],
    ];
    let input = lifecycle_gate_input(&launches, &prayer_ticks, &guard_clicks);
    let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
    assert!(
        matches!(&decision, W1GateDecision::Fail(reason) if reason.contains("non-Missiles")),
        "a non-Missiles guard click between first launch and arrival must fail: {decision:?}"
    );
}

#[test]
fn off_click_snapshot_adds_crossing_evidence_at_its_observed_tick() {
    let dest = WorldTile {
        x: 2880,
        z: 3596,
        level: 0,
    };
    let mut prayer_ticks = vec![PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Unspecified,
        missiles_on: Some(false),
        prayers_active: Some(false),
    }];
    let actions = [json!({
        "kind": "guard",
        "tick": 20,
        "component_id": MISSILES_BUTTON,
        "snapshot": {
            "tile": [dest.x, dest.z, dest.level],
            "prayer_varps": [{"index": MISSILES_VARP, "value": 1}],
            "local_player": {"tile": {"x": dest.x, "z": dest.z + 3, "level": dest.level}}
        }
    })];
    append_guard_crossing_observations(&mut prayer_ticks, &actions, dest, Some(20));
    assert_eq!(prayer_ticks.len(), 2, "no tick is synthesized");
    assert_eq!(crossing_observations_all_on(&prayer_ticks, 20), Some(true));
    assert_eq!(
        cleanup_prayer_observation(&prayer_ticks, 20).and_then(|row| row.missiles_on),
        Some(false)
    );
    assert_eq!(prayer_ticks[0].phase, PrayerPhase::Crossing);
    assert_eq!(prayer_ticks[1].phase, PrayerPhase::Cleanup);
}

#[test]
fn off_click_snapshot_away_from_destination_is_not_crossing_evidence() {
    let dest = WorldTile {
        x: 2880,
        z: 3596,
        level: 0,
    };
    let mut prayer_ticks = vec![PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Cleanup,
        missiles_on: Some(false),
        prayers_active: Some(false),
    }];
    let actions = [json!({
        "kind": "guard",
        "tick": 20,
        "component_id": MISSILES_BUTTON,
        "snapshot": {
            "tile": [2842, 3605, 0],
            "prayer_varps": [{"index": MISSILES_VARP, "value": 1}],
            "local_player": {"tile": {"x": dest.x, "z": dest.z, "level": dest.level}}
        }
    })];
    append_guard_crossing_observations(&mut prayer_ticks, &actions, dest, Some(20));
    assert_eq!(prayer_ticks.len(), 1);
    assert_eq!(crossing_observations_all_on(&prayer_ticks, 20), None);
}

#[test]
fn retained_same_tick_off_row_is_cleanup_only_with_pre_click_on_evidence() {
    let click_crossing = [PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Crossing,
        missiles_on: Some(true),
        prayers_active: Some(true),
    }];
    let mut per_tick = [PrayerObservation {
        tick: 20,
        phase: PrayerPhase::Unspecified,
        missiles_on: Some(false),
        prayers_active: Some(false),
    }];
    normalize_same_tick_cleanup(&mut per_tick, Some(20), &click_crossing);
    let observations = [per_tick[0], click_crossing[0]];
    assert_eq!(observations.len(), 2, "the receipt tick count is unchanged");
    assert_eq!(observations[0].phase, PrayerPhase::Cleanup);
    assert_eq!(crossing_observations_all_on(&observations, 20), Some(true));
}

#[test]
fn receipt_arrival_uses_host_network_tile_while_rendered_actor_lags() {
    let dest = WorldTile {
        x: 2880,
        z: 3596,
        level: 0,
    };
    let route_head_only = json!({
        "tile": [dest.x, dest.z, dest.level],
        "local_player": {"tile": {"x": 2842, "z": 3605, "level": 0}}
    });
    assert!(receipt_snapshot_at_dest(&route_head_only, dest));
    let actor_at_dest = json!({
        "tile": [2842, 3605, 0],
        "local_player": {"tile": {"x": dest.x, "z": dest.z, "level": dest.level}}
    });
    assert!(!receipt_snapshot_at_dest(&actor_at_dest, dest));
}

#[test]
#[ignore = "offline replay reads retained W1 receipts and writes evidence"]
fn replay_walk_guard_w1_retained_receipts() {
    let Some(root) = std::env::var_os("LIVE_EVIDENCE_DIR") else {
        eprintln!("skip: replay_walk_guard_w1_retained_receipts requires LIVE_EVIDENCE_DIR");
        return;
    };
    let root = PathBuf::from(root);
    let cases = [
        (root.join("W1-wgkiu3hcw6_0-receipt.json"), false),
        (root.join("W1-wg9l6smxhw_0-receipt.json"), true),
        (root.join("W1-wgefe0xaj8_0-receipt.json"), true),
    ];
    std::fs::create_dir_all(&root).expect("create W1 lifecycle evidence directory");
    let mut table = Vec::with_capacity(cases.len());
    for (path, expected_pass) in cases {
        let path_label = path.display().to_string();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                eprintln!("skip: retained W1 receipt {path_label} is absent");
                return;
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("parse retained W1 receipt {path_label}: {error}"));
        let parsed = parse_w1_receipt(&value)
            .unwrap_or_else(|error| panic!("decode retained W1 receipt {path_label}: {error}"));
        let input = parsed.gate_input();
        let decision = evaluate_w1_gate(W1Mode::Crossing, &input);
        let (replay, reason) = match decision {
            W1GateDecision::Pass => ("PASS", None),
            W1GateDecision::Fail(reason) => ("FAIL", Some(reason)),
            W1GateDecision::Pending => ("PENDING", None),
        };
        assert_eq!(
            replay == "PASS",
            expected_pass,
            "{} replayed as {replay}: {}",
            parsed.account.as_deref().unwrap_or(path_label.as_str()),
            reason.as_deref().unwrap_or("no gate detail")
        );
        table.push(json!({
            "receipt": path_label,
            "account": parsed.account,
            "recorded_outcome": parsed.recorded_outcome,
            "replayed_outcome": replay,
            "reason": reason,
            "evidence_mode": if parsed.legacy_sparse { "legacy-sparse" } else { "per-tick" },
            "observed_prayer_rows": parsed.prayer_ticks.iter().map(|row| json!({
                "phase": row.phase.as_str(),
                "tick": row.tick,
                "missiles_on": row.missiles_on,
                "prayers_active": row.prayers_active,
            })).collect::<Vec<_>>(),
            "legacy_limit": parsed.legacy_sparse.then_some(
                "No per-tick observations are present. Prayer facts are read from guard-click snapshots and the receipt's protect/off observations; a known hold is carried only across intervals with no later guard click. The legacy receipts do not directly establish three post-off ticks."
            ),
        }));
    }
    let output = root.join("W1-lifecycle-replay.json");
    std::fs::write(&output, serde_json::to_vec_pretty(&table).unwrap())
        .unwrap_or_else(|error| panic!("write replay table {}: {error}", output.display()));
}

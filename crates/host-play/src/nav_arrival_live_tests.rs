//! Ignored live proof that a native Walk settles with its retained receipt.
//!
//! Run with `NAV_ARRIVAL_SCENARIO` set to `return`, `yew`, `tower-top`,
//! `tower-basement`, `lumbridge-door`, or `gnome-maze`. Each run requires the
//! copied cache and the caller-supplied throwaway HOME under `BOT_EVIDENCE_DIR`.

use super::*;
use api::snapshot::{GameSnapshot, WorldTile};
use client::render::backend::FrameOutput;
use host::FrameBuf;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkReceipt,
    WalkRequest,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vault::{Profile, ProfileSettings};

const RETURN_ORIGIN: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const RETURN_TARGET: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};
const YEW_SCENE_SEED: WorldTile = WorldTile {
    x: 3150,
    z: 3230,
    level: 0,
};
// content/maps/m50_50.jm2 places closed door 1533 at 0_50_50_38_10 (shape 0, angle 0).
// doors/configs/doors.loc pairs ids 1533 and 1534 on basic_wall.
// doors/scripts/doors.rs2 opens it for 500 ticks; door_procs moves it one tile west.
const LUMBRIDGE_DOOR_TILE: WorldTile = WorldTile {
    x: 3238,
    z: 3210,
    level: 0,
};
const LUMBRIDGE_DOOR_OPEN_TILE: WorldTile = WorldTile {
    x: 3237,
    z: 3210,
    level: 0,
};
const LUMBRIDGE_DOOR_START: WorldTile = WorldTile {
    x: 3237,
    z: 3210,
    level: 0,
};
const LUMBRIDGE_DOOR_TARGET: WorldTile = WorldTile {
    x: 3239,
    z: 3210,
    level: 0,
};
const LUMBRIDGE_DOOR_CLOSED_ID: i32 = 1533;
const LUMBRIDGE_DOOR_OPEN_ID: i32 = 1534;
const LUMBRIDGE_DOOR_CLOSE_TICKS: u32 = 500;
const LUMBRIDGE_DOOR_WAIT_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const GNOME_MAZE_START: WorldTile = WorldTile {
    x: 2517,
    z: 3179,
    level: 0,
};
const GNOME_MAZE_TARGET: WorldTile = WorldTile {
    x: 2515,
    z: 3160,
    level: 0,
};
const SETUP_TIMEOUT: Duration = Duration::from_secs(180);
const WALK_TIMEOUT: Duration = Duration::from_secs(12 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Return,
    Yew,
    TowerTop,
    TowerBasement,
    LumbridgeDoor,
    GnomeMaze,
}

impl Scenario {
    fn from_env() -> Self {
        match std::env::var("NAV_ARRIVAL_SCENARIO").as_deref() {
            Ok("return") => Self::Return,
            Ok("yew") => Self::Yew,
            Ok("tower-top") => Self::TowerTop,
            Ok("tower-basement") => Self::TowerBasement,
            Ok("lumbridge-door") => Self::LumbridgeDoor,
            Ok("gnome-maze") => Self::GnomeMaze,
            other => panic!("unknown NAV_ARRIVAL_SCENARIO: {other:?}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::Yew => "yew",
            Self::TowerTop => "tower-top",
            Self::TowerBasement => "tower-basement",
            Self::LumbridgeDoor => "lumbridge-door",
            Self::GnomeMaze => "gnome-maze",
        }
    }

    fn seed(self) -> WorldTile {
        match self {
            Self::Return => RETURN_ORIGIN,
            Self::Yew => YEW_SCENE_SEED,
            // ladders.rs2:114-119 lands at 0_48_49_33_26.
            Self::TowerTop => WorldTile {
                x: 3105,
                z: 3162,
                level: 0,
            },
            // One tile east of the real basement ladder at 0_48_149_31_40.
            Self::TowerBasement => WorldTile {
                x: 3104,
                z: 9576,
                level: 0,
            },
            // Ordinary closed door 1533 at 0_50_50_38_10.
            Self::LumbridgeDoor => LUMBRIDGE_DOOR_START,
            Self::GnomeMaze => GNOME_MAZE_START,
        }
    }
}

#[derive(Debug, Clone)]
struct WalkSpec {
    target: WorldTile,
    radius: u16,
    allow_danger_zones: bool,
    loc_id: Option<i32>,
    loc_name: Option<String>,
    loc_actions: Vec<String>,
}

impl WalkSpec {
    fn return_walk() -> Self {
        Self {
            target: RETURN_TARGET,
            radius: 12,
            allow_danger_zones: false,
            loc_id: None,
            loc_name: None,
            loc_actions: Vec::new(),
        }
    }

    fn crypt_walk() -> Self {
        Self {
            target: WorldTile {
                x: 3249,
                z: 3192,
                level: 0,
            },
            radius: 3,
            allow_danger_zones: false,
            loc_id: None,
            loc_name: None,
            loc_actions: Vec::new(),
        }
    }

    fn request(&self, required_after: api::quest_progress::EvidenceStamp) -> WalkRequest {
        WalkRequest {
            target: self.target,
            candidate_goals: None,
            radius: self.radius,
            arrival: nav_planner::arrival::ArrivalKind::Reach,
            loc_id: self.loc_id,
            options: script::native::WalkOptions {
                allow_danger_zones: self.allow_danger_zones.then_some(true).into(),
                ..script::native::WalkOptions::default()
            },
            required_after,
            evidence: None,
            cross: Box::default(),
            avoid: Box::default(),
            protect: false,
            food_guard: false,
            cure_guard: false,
            allow: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DoorObservation {
    id: i32,
    tile: WorldTile,
    shape: i32,
    angle: i32,
    open_action: bool,
    close_action: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct SceneProof {
    ingame: bool,
    scene_state: i32,
    player_tile: Option<WorldTile>,
    target_loc_visible: bool,
    target_operable: Option<bool>,
    lumbridge_door: Option<DoorObservation>,
}

#[derive(Default)]
struct LiveState {
    teleport_sent: bool,
    prime_seed_sent: bool,
    prime_open_sent: bool,
    prime_complete: bool,
    first_sample_at: Option<Instant>,
    ready: bool,
    setup_error: Option<String>,
    door_close_wait: Option<Value>,
    target: Option<WalkSpec>,
    scene: SceneProof,
    walk_started: bool,
    start_error: Option<String>,
    receipt: Option<Result<WalkReceipt, ActionError>>,
    samples: Vec<Value>,
}

struct NativeWalkScript {
    state: Arc<parking_lot::Mutex<LiveState>>,
    walk: WalkSpec,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for NativeWalkScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let begin = {
            let mut state = self.state.lock();
            if state.walk_started {
                false
            } else {
                state.walk_started = true;
                true
            }
        };
        if begin {
            match tick
                .actions
                .begin::<Walk>(self.walk.request(tick.cx.evidence()), &mut tick.cx)
            {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    self.state.lock().start_error = Some(format!("native Walk begin: {error:?}"));
                }
            }
        }

        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                self.state.lock().receipt = Some(result);
                self.handle = None;
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

#[derive(Clone)]
struct RouteSample {
    request_id: u64,
    generation: u64,
    route: nav_planner::router::Route,
}

fn stamp_utc() -> String {
    format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_secs()
    )
}

pub(super) fn absolute_env_path(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("{name} must be explicitly supplied")),
    );
    assert!(
        path.is_absolute(),
        "{name} must be absolute: {}",
        path.display()
    );
    path
}

pub(super) fn required_directory(name: &str, evidence_root: &Path) -> PathBuf {
    let supplied = absolute_env_path(name);
    let canonical = std::fs::canonicalize(&supplied)
        .unwrap_or_else(|error| panic!("resolve {name} {}: {error}", supplied.display()));
    assert!(
        canonical.is_dir(),
        "{name} must be a directory: {}",
        canonical.display()
    );
    assert!(
        canonical.starts_with(evidence_root),
        "{name} must resolve inside BOT_EVIDENCE_DIR to a throwaway copy: {}",
        canonical.display()
    );
    canonical
}

fn chebyshev(from: WorldTile, to: WorldTile) -> u32 {
    from.x.abs_diff(to.x).max(from.z.abs_diff(to.z))
}

/// Returns the observed closed or normal open state of the selected door.
fn lumbridge_door_observation(snapshot: &GameSnapshot) -> Option<DoorObservation> {
    snapshot
        .locs()
        .iter()
        .find(|loc| {
            (loc.id == LUMBRIDGE_DOOR_CLOSED_ID && loc.tile == LUMBRIDGE_DOOR_TILE)
                || (loc.id == LUMBRIDGE_DOOR_OPEN_ID && loc.tile == LUMBRIDGE_DOOR_OPEN_TILE)
        })
        .map(|loc| DoorObservation {
            id: loc.id,
            tile: loc.tile,
            shape: loc.shape,
            angle: loc.angle,
            open_action: loc
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("Open")),
            close_action: loc
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("Close")),
        })
}

fn door_observation_json(door: Option<DoorObservation>) -> Value {
    door.map_or(Value::Null, |door| {
        json!({
            "id": door.id,
            "tile": door.tile,
            "shape": door.shape,
            "angle": door.angle,
            "open_action": door.open_action,
            "close_action": door.close_action,
        })
    })
}

fn lumbridge_door_is_closed(door: Option<DoorObservation>) -> bool {
    door.is_some_and(|door| {
        door.id == LUMBRIDGE_DOOR_CLOSED_ID
            && door.tile == LUMBRIDGE_DOOR_TILE
            && door.shape == 0
            && door.angle == 0
            && door.open_action
    })
}

fn lumbridge_door_is_open(door: Option<DoorObservation>) -> bool {
    door.is_some_and(|door| {
        door.id == LUMBRIDGE_DOOR_OPEN_ID
            && door.tile == LUMBRIDGE_DOOR_OPEN_TILE
            && door.shape == 0
            && door.angle == 1
            && door.close_action
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct LumbridgeDoorWalkEvidence {
    opened_during_walk: bool,
    player_on_door_tile_after_open: bool,
    player_at_target_after_door_tile: bool,
}

fn lumbridge_door_walk_evidence(samples: &[Value]) -> LumbridgeDoorWalkEvidence {
    let mut evidence = LumbridgeDoorWalkEvidence::default();
    for sample in samples {
        if !sample["walking"].as_bool().unwrap_or(false) {
            continue;
        }
        let door = &sample["lumbridge_door"];
        if door["id"].as_i64() == Some(i64::from(LUMBRIDGE_DOOR_OPEN_ID))
            && door["tile"] == json!(LUMBRIDGE_DOOR_OPEN_TILE)
            && door["shape"].as_i64() == Some(0)
            && door["angle"].as_i64() == Some(1)
            && door["close_action"].as_bool() == Some(true)
        {
            evidence.opened_during_walk = true;
        }
        if evidence.opened_during_walk && sample["tile"] == json!(LUMBRIDGE_DOOR_TILE) {
            evidence.player_on_door_tile_after_open = true;
        }
        if evidence.player_on_door_tile_after_open && sample["tile"] == json!(LUMBRIDGE_DOOR_TARGET)
        {
            evidence.player_at_target_after_door_tile = true;
        }
    }
    evidence
}

/// The approximate Lumbridge point only opens the live scene. The destination
/// is always selected from the scene's loaded loc facts, never from a guessed id.
fn yew_walk(snapshot: &GameSnapshot, origin: WorldTile) -> Option<WalkSpec> {
    snapshot
        .locs()
        .iter()
        .filter_map(|loc| {
            let name = loc.name.as_deref()?;
            if !name.to_ascii_lowercase().contains("yew")
                || !loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case("Chop down"))
            {
                return None;
            }
            let distance = chebyshev(origin, loc.tile);
            if distance <= 1 || distance > 52 {
                return None;
            }
            Some((
                distance,
                WalkSpec {
                    target: loc.tile,
                    radius: 1,
                    allow_danger_zones: false,
                    loc_id: Some(loc.id),
                    loc_name: Some(name.to_owned()),
                    loc_actions: loc.actions.iter().flatten().cloned().collect(),
                },
            ))
        })
        .max_by_key(|(distance, _)| *distance)
        .map(|(_, walk)| walk)
}

fn frame_hook(
    state: Arc<parking_lot::Mutex<LiveState>>,
    account: String,
    scenario: Scenario,
    door_state_changed: Arc<parking_lot::Condvar>,
) -> impl Fn(&mut client::client::Client, &str, crate::SlotFrameInput) + Send + Sync + 'static {
    let prime_tower_door = std::env::var("NAV_ARRIVAL_PRIME_TOWER_DOOR").as_deref() == Ok("1");
    move |client, username, _hold| {
        if username != account {
            return;
        }
        client.set_draw(true);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        let tile = snapshot
            .tile()
            .map(|(x, z, level)| WorldTile { x, z, level });
        let seed = scenario.seed();
        let mut state = state.lock();
        if scenario == Scenario::Yew
            && state.target.is_none()
            && state.teleport_sent
            && tile == Some(seed)
            && client.ingame
            && client.scene_state == 2
        {
            state.target = yew_walk(&snapshot, seed);
        }
        let lumbridge_door = if scenario == Scenario::LumbridgeDoor {
            lumbridge_door_observation(&snapshot)
        } else {
            None
        };
        state.scene = SceneProof {
            ingame: client.ingame,
            scene_state: client.scene_state,
            player_tile: tile,
            target_loc_visible: state.target.as_ref().is_some_and(|target| {
                target.loc_id.is_some_and(|id| {
                    snapshot
                        .locs()
                        .iter()
                        .any(|loc| loc.id == id && loc.tile == target.target)
                })
            }),
            target_operable: state.target.as_ref().and_then(|target| {
                let id = target.loc_id?;
                let loc = snapshot
                    .locs()
                    .iter()
                    .find(|loc| loc.id == id && loc.tile == target.target)?;
                tile.and_then(|from| {
                    api::query::loc_approach::can_operate_from(loc, snapshot.scene(), from)
                })
            }),
            lumbridge_door,
        };
        if scenario == Scenario::LumbridgeDoor {
            door_state_changed.notify_all();
        }
        if state.samples.last().and_then(|row| row["tick"].as_u64())
            != Some(u64::from(snapshot.tick()))
        {
            let walking = state.walk_started;
            let elapsed_ms = state
                .first_sample_at
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_millis();
            let mut sample = json!({
                "tick": snapshot.tick(),
                "elapsed_ms": elapsed_ms,
                "tile": tile,
                "scene": client.scene_state,
                "walking": walking,
                "map_flag": snapshot.map_flag(),
                "actor_moving": snapshot.local_player().map(|p| p.player.actor.moving),
                "last_movement": snapshot.last_server_movement_tick(),
                // content/pack/obj.pack:554 selects ghostskull.
                "ghostskull": snapshot.inv().iter().filter(|(id, _)| *id == 553).map(|(_, qty)| qty).sum::<i32>(),
                "tower_door": snapshot.locs().iter()
                    .filter(|loc| matches!(loc.id, 1536 | 1537)
                        && loc.tile.level == 0
                        && loc.tile.x.abs_diff(3107) <= 1
                        && loc.tile.z.abs_diff(3162) <= 1)
                    .collect::<Vec<_>>(),
            });
            if scenario == Scenario::LumbridgeDoor {
                sample["lumbridge_door"] = door_observation_json(lumbridge_door);
            }
            state.samples.push(sample);
        }
        if !(client.ingame && client.scene_state == 2) || state.setup_error.is_some() {
            return;
        }
        if prime_tower_door
            && matches!(scenario, Scenario::TowerTop | Scenario::TowerBasement)
            && !state.prime_complete
        {
            if !state.prime_seed_sent {
                api::interact::seed_at(client, 0, 3106, 3161);
                state.prime_seed_sent = true;
                return;
            }
            if snapshot.locs().iter().any(|loc| {
                loc.id == 1537
                    && loc.tile
                        == (WorldTile {
                            x: 3106,
                            z: 3162,
                            level: 0,
                        })
            }) {
                // Preserve the prior tower-entry door state, as in the quest.
                // Never Close or reset a shared-engine door.
                state.prime_complete = true;
            } else if !state.prime_open_sent
                && tile
                    == Some(WorldTile {
                        x: 3106,
                        z: 3161,
                        level: 0,
                    })
            {
                if let Some(door) = snapshot.locs().iter().find(|loc| {
                    loc.id == 1536
                        && loc.tile
                            == (WorldTile {
                                x: 3107,
                                z: 3162,
                                level: 0,
                            })
                }) {
                    state.prime_open_sent = matches!(
                        api::interact::Interactions::new(&snapshot, client).interact(
                            api::interact::OpTarget::Loc(door),
                            api::interact::ActionSpec::Label("Open".into()),
                        ),
                        api::interact::SendResult::Sent { .. }
                    );
                }
            }
            return;
        }
        if !state.teleport_sent {
            if matches!(scenario, Scenario::TowerTop | Scenario::TowerBasement) {
                for command in [
                    "setvar priest 4",
                    "give ghostskull 1",
                    "setstat hitpoints 30",
                ] {
                    assert!(api::interact::cheat(client, command).is_sent());
                }
            }
            api::interact::seed_at(client, seed.level, seed.x, seed.z);
            state.teleport_sent = true;
            return;
        }
        if tile != Some(seed) {
            state.ready = false;
            return;
        }
        state.ready = state.target.is_some()
            && (!matches!(scenario, Scenario::TowerTop | Scenario::TowerBasement)
                || snapshot.inv().iter().any(|&(id, qty)| id == 553 && qty > 0));
    }
}

fn slot_status(play: &Play, account: &str) -> Value {
    play.statuses()
        .into_iter()
        .find(|status| status.username == account)
        .map(|status| {
            json!({
                "username": status.username,
                "ingame": status.ingame,
                "scene_state": status.scene_state,
            })
        })
        .unwrap_or(Value::Null)
}

fn slot_is_ready(play: &Play, account: &str) -> bool {
    play.statuses()
        .iter()
        .any(|status| status.username == account && status.ingame && status.scene_state == 2)
}

fn wait_for_setup(
    play: &Play,
    account: &str,
    state: &Arc<parking_lot::Mutex<LiveState>>,
) -> Result<(), String> {
    let deadline = Instant::now() + SETUP_TIMEOUT;
    loop {
        let (ready, error) = {
            let state = state.lock();
            (state.ready, state.setup_error.clone())
        };
        if let Some(error) = error {
            return Err(error);
        }
        if ready && slot_is_ready(play, account) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account did not reach the scenario seed in ingame scene_state 2 within {SETUP_TIMEOUT:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_lumbridge_door_closed(
    state: &Arc<parking_lot::Mutex<LiveState>>,
    door_state_changed: &parking_lot::Condvar,
) -> Result<(), String> {
    let deadline = Instant::now() + LUMBRIDGE_DOOR_WAIT_TIMEOUT;
    let mut state = state.lock();
    let mut open_since: Option<(u32, DoorObservation)> = None;
    loop {
        let door = state.scene.lumbridge_door;
        let tick = state
            .samples
            .last()
            .and_then(|sample| sample["tick"].as_u64())
            .and_then(|tick| u32::try_from(tick).ok());
        if lumbridge_door_is_closed(door) {
            state.door_close_wait = Some(match open_since {
                Some((opened_tick, opened_door)) => json!({
                    "waited_for_normal_closure": true,
                    "opened_tick": opened_tick,
                    "closed_tick": tick,
                    "observed_ticks": tick.map(|tick| tick.wrapping_sub(opened_tick)),
                    "opened_door": door_observation_json(Some(opened_door)),
                    "closed_door": door_observation_json(door),
                }),
                None => json!({
                    "waited_for_normal_closure": false,
                    "observed_ticks": 0,
                    "closed_door": door_observation_json(door),
                }),
            });
            return Ok(());
        }
        if let Some(door) = door {
            if !lumbridge_door_is_open(Some(door)) {
                state.door_close_wait = Some(json!({
                    "waited_for_normal_closure": open_since.is_some(),
                    "unexpected_door": door_observation_json(Some(door)),
                }));
                return Err(format!(
                    "expected ordinary door {LUMBRIDGE_DOOR_CLOSED_ID} at {LUMBRIDGE_DOOR_TILE:?} or its open state at {LUMBRIDGE_DOOR_OPEN_TILE:?}, observed {door:?}"
                ));
            }
            if let Some(tick) = tick {
                let (opened_tick, _) = *open_since.get_or_insert((tick, door));
                let elapsed = tick.wrapping_sub(opened_tick);
                if elapsed >= LUMBRIDGE_DOOR_CLOSE_TICKS {
                    state.door_close_wait = Some(json!({
                        "waited_for_normal_closure": true,
                        "opened_tick": opened_tick,
                        "last_open_tick": tick,
                        "observed_ticks": elapsed,
                        "maximum_content_ticks": LUMBRIDGE_DOOR_CLOSE_TICKS,
                        "last_open_door": door_observation_json(Some(door)),
                    }));
                    return Err(format!(
                        "ordinary door {LUMBRIDGE_DOOR_OPEN_ID} stayed open for {elapsed} observed game ticks"
                    ));
                }
            }
        } else {
            state.door_close_wait = Some(json!({
                "waited_for_normal_closure": open_since.is_some(),
                "expected_closed_door_id": LUMBRIDGE_DOOR_CLOSED_ID,
                "expected_closed_door_tile": LUMBRIDGE_DOOR_TILE,
                "observed_door": Value::Null,
            }));
            return Err(format!(
                "ordinary door {LUMBRIDGE_DOOR_CLOSED_ID} was not observed at {LUMBRIDGE_DOOR_TILE:?} before native Walk"
            ));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            state.door_close_wait = Some(match open_since {
                Some((opened_tick, opened_door)) => json!({
                    "waited_for_normal_closure": true,
                    "opened_tick": opened_tick,
                    "last_observed_tick": tick,
                    "observed_ticks": tick.map(|tick| tick.wrapping_sub(opened_tick)),
                    "maximum_content_ticks": LUMBRIDGE_DOOR_CLOSE_TICKS,
                    "opened_door": door_observation_json(Some(opened_door)),
                    "last_open_door": door_observation_json(door),
                    "wall_watchdog": LUMBRIDGE_DOOR_WAIT_TIMEOUT,
                }),
                None => json!({
                    "waited_for_normal_closure": false,
                    "last_observed_tick": tick,
                    "observed_door": door_observation_json(door),
                    "wall_watchdog": LUMBRIDGE_DOOR_WAIT_TIMEOUT,
                }),
            });
            return Err(format!(
                "ordinary door did not close within {LUMBRIDGE_DOOR_CLOSE_TICKS} observed game ticks or {LUMBRIDGE_DOOR_WAIT_TIMEOUT:?}"
            ));
        }
        let _ = door_state_changed.wait_for(&mut state, remaining);
    }
}

fn wait_frame_after(mailbox: &FrameBuf, generation: u64, timeout: Duration) -> Option<FrameOutput> {
    let deadline = Instant::now() + timeout;
    loop {
        if mailbox.generation() > generation {
            if let Some(frame) = mailbox.take() {
                return Some(frame);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Native pixmap words for one mailbox frame. Conversion and PNG encoding
/// run on the shared background evidence writer; callers only move the
/// already-rendered frame off the mailbox.
fn frame_words(frame: FrameOutput) -> Result<(u32, u32, Vec<i32>), String> {
    match frame {
        FrameOutput::PixMap(pixmap) => {
            let width = u32::try_from(pixmap.width).map_err(|error| error.to_string())?;
            let height = u32::try_from(pixmap.height).map_err(|error| error.to_string())?;
            Ok((width, height, pixmap.pixels))
        }
        FrameOutput::Texture(texture) => Ok((texture.width, texture.height, texture.read_back())),
    }
}

fn route_sample(play: &Play, account: &str) -> Option<RouteSample> {
    let navs = play.navs.lock().unwrap();
    let bot = navs.get(account)?;
    Some(RouteSample {
        request_id: bot.route_request_id,
        generation: bot.route_generation,
        route: bot.route.as_deref().cloned()?,
    })
}

fn route_json(sample: Option<&RouteSample>) -> Value {
    let Some(sample) = sample else {
        return Value::Null;
    };
    let legs = sample
        .route
        .legs
        .iter()
        .map(|leg| match leg {
            nav_planner::router::Leg::Walk { tiles } => json!({"kind": "walk", "tiles": tiles}),
            nav_planner::router::Leg::Transport { edge } => json!({
                "kind": format!("{:?}", edge.kind),
                "at": edge.at,
                "to": edge.to,
                "loc_id": edge.loc_id,
                "option": edge.option,
                "ticks": edge.ticks,
            }),
        })
        .collect::<Vec<_>>();
    json!({
        "request_id": sample.request_id,
        "generation": sample.generation,
        "destination": sample.route.dest,
        "ticks": sample.route.ticks,
        "legs": legs,
    })
}

fn target_json(walk: Option<&WalkSpec>) -> Value {
    let Some(walk) = walk else {
        return Value::Null;
    };
    json!({
        "tile": walk.target,
        "radius": walk.radius,
        "loc_id": walk.loc_id,
        "loc_name": walk.loc_name,
        "loc_actions": walk.loc_actions,
        "loc_identity_source": walk.loc_id.map(|_| "live GameSnapshot::locs()"),
    })
}

fn scene_json(scene: SceneProof) -> Value {
    let mut value = json!({
        "ingame": scene.ingame,
        "scene_state": scene.scene_state,
        "player_tile": scene.player_tile,
        "target_loc_visible": scene.target_loc_visible,
        "target_operable": scene.target_operable,
    });
    if let Some(door) = scene.lumbridge_door {
        value["lumbridge_door"] = door_observation_json(Some(door));
    }
    value
}

fn receipt_json(result: Option<&Result<WalkReceipt, ActionError>>) -> Value {
    match result {
        Some(Ok(receipt)) => json!({
            "source": "retained result of polling native Walk action handle",
            "request_id": receipt.request_id,
            "evidence": {
                "run": {
                    "slot": receipt.evidence.run.slot,
                    "run": receipt.evidence.run.run,
                    "session": receipt.evidence.run.session,
                },
                "tick": receipt.evidence.tick,
                "sequence": receipt.evidence.sequence,
            },
            "end": format!("{:?}", receipt.end),
            "blocked_zones": receipt.blocked.as_deref().map(|zones| format!("{zones:?}")),
            "detail": receipt.detail.as_deref(),
        }),
        Some(Err(error)) => json!({
            "source": "native Walk action handle returned an error",
            "error": format!("{error:?}"),
        }),
        None => Value::Null,
    }
}

struct EvidenceContext<'a> {
    scenario: Scenario,
    account: &'a str,
    stamp: &'a str,
    nav_pack: &'a Path,
    play: &'a Play,
}

impl EvidenceContext<'_> {
    fn document(
        &self,
        phase: &str,
        target: Option<&WalkSpec>,
        scene: SceneProof,
        route: Option<&RouteSample>,
        result: Option<&Result<WalkReceipt, ActionError>>,
        extra_error: Option<&str>,
    ) -> Value {
        json!({
            "task": "NAV-ARRIVAL-1",
            "scenario": self.scenario.name(),
            "account": self.account,
            "timestamp_utc": self.stamp,
            "phase": phase,
            "client": {"bot_cpu": true, "live": true},
            "setup": {
                "seed_tile": self.scenario.seed(),
                "slot_status": slot_status(self.play, self.account),
                "scene": scene_json(scene),
            },
            "request": target.map(|walk| json!({
                "type": "native Walk",
                "target": walk.target,
                "radius": walk.radius,
                "loc_id": walk.loc_id,
                "allow_danger_zones": walk.allow_danger_zones,
                "allow_teleports": false,
                "allow_wilderness": false,
                "allow_bank_fetch": false,
                "target_identity": target_json(Some(walk)),
            })),
            "route": route_json(route),
            "native_walk_receipt": receipt_json(result),
            "nav_pack": self.nav_pack.display().to_string(),
            "nav_format": nav_planner::pack::FORMAT_ID,
            "harness_error": extra_error,
        })
    }
}

/// Writes a real rendered frame (when one is available) and its matching JSON
/// before the caller performs the corresponding assertion. The frame is
/// already rendered by the slot; conversion, PNG encoding and the PNG write
/// run on a background evidence writer, and this waits bounded for the file
/// before writing the matching JSON, so success contents are unchanged.
pub(super) fn write_capture(
    run_dir: &Path,
    stamp: &str,
    step: &str,
    failed: bool,
    mut report: Value,
    mailbox: &FrameBuf,
) -> bool {
    let stem = if failed {
        format!("{stamp}_FAIL-{step}")
    } else {
        format!("{stamp}_{step}")
    };
    let png_path = run_dir.join(format!("{stem}.png"));
    let json_path = run_dir.join(format!("{stem}.json"));
    let generation = mailbox.generation();
    let framed = wait_frame_after(mailbox, generation, Duration::from_secs(8)).map(frame_words);
    let (png_written, capture) = match framed {
        Some(Ok((width, height, pixels))) => {
            let writer = evidence_writer::EvidenceWriter::new(evidence_writer::DEFAULT_QUEUE_BOUND);
            let job = writer.submit(evidence_writer::EvidenceRequest {
                png_path: png_path.clone(),
                width,
                height,
                pixels,
                color: evidence_writer::PngColor::Rgba,
                sidecar: None,
            });
            match writer.wait(&job, evidence_writer::DEFAULT_FLUSH_WAIT) {
                Some(outcome) if outcome.error.is_none() => (
                    true,
                    json!({
                        "frame_source": "host::FrameBuf actual rendered FrameOutput",
                        "png_file": png_path.file_name().and_then(|name| name.to_str()),
                        "width": outcome.width,
                        "height": outcome.height,
                        "png_written": true,
                    }),
                ),
                Some(outcome) => (
                    false,
                    json!({
                        "frame_source": "host::FrameBuf actual rendered FrameOutput",
                        "png_file": png_path.file_name().and_then(|name| name.to_str()),
                        "png_written": false,
                        "png_error": outcome.error.unwrap_or_else(|| "unknown".into()),
                    }),
                ),
                None => (
                    false,
                    json!({
                        "frame_source": "host::FrameBuf actual rendered FrameOutput",
                        "png_written": false,
                        "capture_error": "evidence writer did not finish before the capture deadline",
                    }),
                ),
            }
        }
        Some(Err(error)) => (
            false,
            json!({
                "frame_source": "host::FrameBuf actual rendered FrameOutput",
                "png_written": false,
                "capture_error": error,
            }),
        ),
        None => (
            false,
            json!({
                "frame_source": "host::FrameBuf actual rendered FrameOutput",
                "png_written": false,
                "capture_error": "no rendered frame arrived before the capture deadline",
            }),
        ),
    };
    report["capture"] = capture;
    // The PNG is fsynced by the background worker before `wait` returns;
    // fsync the matching JSON the same way so the pair is durable before
    // the caller asserts.
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&report).expect("serialize NAV-ARRIVAL live evidence"),
    )
    .unwrap_or_else(|error| panic!("write evidence {}: {error}", json_path.display()));
    // Windows `FlushFileBuffers` needs a handle with write access: a
    // read-only handle fails with error 5. Open for writing, without
    // truncating, as `local_state::files` does for its quarantine copy.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&json_path)
        .and_then(|file| file.sync_all())
        .unwrap_or_else(|error| panic!("fsync evidence {}: {error}", json_path.display()));
    println!("nav-arrival-evidence-json={}", json_path.display());
    if png_written {
        println!("nav-arrival-evidence-png={}", png_path.display());
    }
    png_written
}

pub(super) fn live_profile(name: String, password: String) -> Profile {
    Profile {
        username: name,
        password: password.into(),
        uid: 274_279_311,
        settings: ProfileSettings::default(),
    }
}

/// The evidence JSON is flushed through a handle that can write. On
/// Windows, `sync_all` on a read-only handle fails with error 5.
#[test]
fn write_capture_flushes_its_json_without_a_frame() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let run_dir =
        std::env::temp_dir().join(format!("nav-arrival-sync-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&run_dir).unwrap();
    let mailbox = FrameBuf::new();
    let written = write_capture(
        &run_dir,
        "stamp",
        "sync",
        false,
        json!({ "step": "sync" }),
        &mailbox,
    );
    assert!(!written, "no frame arrived, so no PNG");
    let report: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("stamp_sync.json")).unwrap()).unwrap();
    assert_eq!(report["step"], "sync");
    assert_eq!(report["capture"]["png_written"], false);
    std::fs::remove_dir_all(&run_dir).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, and the local R289 engine/nav fixtures"]
fn live_native_walk_arrival_retains_arrived_receipt() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "requires BOT_CPU=1"
    );
    let scenario = Scenario::from_env();
    let supplied_root = absolute_env_path("BOT_EVIDENCE_DIR");
    std::fs::create_dir_all(&supplied_root).expect("create BOT_EVIDENCE_DIR");
    let evidence_root = std::fs::canonicalize(&supplied_root).expect("resolve BOT_EVIDENCE_DIR");
    let home = required_directory("HOME", &evidence_root);
    let copied_cache = required_directory("BOT_CACHE_DIR", &evidence_root);
    let nav_pack_supplied = absolute_env_path("WORLD_NAV_PACK");
    let nav_pack = std::fs::canonicalize(&nav_pack_supplied).unwrap_or_else(|error| {
        panic!(
            "resolve WORLD_NAV_PACK {}: {error}",
            nav_pack_supplied.display()
        )
    });
    assert!(nav_pack.is_file(), "WORLD_NAV_PACK must be a file");
    let engine_supplied = absolute_env_path("WORLD_ENGINE_DIR");
    let engine_dir = std::fs::canonicalize(&engine_supplied).unwrap_or_else(|error| {
        panic!(
            "resolve WORLD_ENGINE_DIR {}: {error}",
            engine_supplied.display()
        )
    });
    assert!(engine_dir.is_dir(), "WORLD_ENGINE_DIR must be a directory");

    let timestamp = stamp_utc();
    let names = mint_live_names(1);
    let (account, password) = mint_live_entries(&names)
        .into_iter()
        .next()
        .expect("mint one owned live account");
    let run_dir = evidence_root.join(format!(
        "NAV-ARRIVAL-1_{}_{}_{}",
        scenario.name(),
        account,
        timestamp
    ));
    std::fs::create_dir_all(&run_dir).expect("create per-account capture directory");

    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        engine_dir: Some(engine_dir),
        cache_dir: Some(copied_cache.clone()),
        unpack_dir: Some(copied_cache),
        vault_path: Some(home.join("vault-nav-arrival")),
        nav_pack: Some(nav_pack.clone()),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve explicit local-289 profile")
        .prepare_template()
        .expect("prepare local-289 live template with copied cache");
    let state = Arc::new(parking_lot::Mutex::new(LiveState {
        target: match scenario {
            Scenario::Return => Some(WalkSpec::return_walk()),
            Scenario::TowerTop | Scenario::TowerBasement => Some(WalkSpec::crypt_walk()),
            Scenario::LumbridgeDoor => Some(WalkSpec {
                target: LUMBRIDGE_DOOR_TARGET,
                radius: 0,
                allow_danger_zones: false,
                loc_id: None,
                loc_name: None,
                loc_actions: Vec::new(),
            }),
            Scenario::GnomeMaze => Some(WalkSpec {
                target: GNOME_MAZE_TARGET,
                radius: 0,
                // This fixture tests walk timing, not fresh-account danger admission.
                allow_danger_zones: true,
                loc_id: None,
                loc_name: None,
                loc_actions: Vec::new(),
            }),
            Scenario::Yew => None,
        },
        ..LiveState::default()
    }));
    let mailbox = FrameBuf::new();
    let frame_mailbox = Arc::clone(&mailbox);
    let frame_state = Arc::clone(&state);
    let door_state_changed = Arc::new(parking_lot::Condvar::new());
    let frame_door_state_changed = Arc::clone(&door_state_changed);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        false,
        vec![live_profile(account.clone(), password)],
        move |username| {
            if username == frame_account {
                (None, Some(Arc::clone(&frame_mailbox)))
            } else {
                (None, None)
            }
        },
        frame_hook(
            frame_state,
            account.clone(),
            scenario,
            frame_door_state_changed,
        ),
    )
    .expect("start one-slot live Play");

    let setup_result = wait_for_setup(&play, &account, &state).and_then(|()| {
        if scenario == Scenario::LumbridgeDoor {
            wait_for_lumbridge_door_closed(&state, &door_state_changed)
        } else {
            Ok(())
        }
    });
    if let Err(error) = &setup_result {
        state.lock().setup_error = Some(error.clone());
    }
    let (target, setup_scene, setup_error) = {
        let state = state.lock();
        (state.target.clone(), state.scene, state.setup_error.clone())
    };
    let evidence = EvidenceContext {
        scenario,
        account: &account,
        stamp: &timestamp,
        nav_pack: &nav_pack,
        play: &play,
    };
    let setup_ok = setup_result.is_ok()
        && setup_error.is_none()
        && target.is_some()
        && setup_scene.ingame
        && setup_scene.scene_state == 2
        && setup_scene.player_tile == Some(scenario.seed())
        && (scenario != Scenario::Yew || setup_scene.target_loc_visible)
        && (scenario != Scenario::LumbridgeDoor
            || lumbridge_door_is_closed(setup_scene.lumbridge_door))
        && slot_is_ready(&play, &account);
    let mut setup_report = evidence.document(
        "ready-before-native-script",
        target.as_ref(),
        setup_scene,
        None,
        None,
        setup_error.as_deref(),
    );
    if scenario == Scenario::LumbridgeDoor {
        setup_report["lumbridge_door_close_wait"] =
            state.lock().door_close_wait.clone().unwrap_or(Value::Null);
    }
    let setup_png = write_capture(
        &run_dir,
        &timestamp,
        if setup_ok { "01-origin" } else { "setup" },
        !setup_ok,
        setup_report,
        &mailbox,
    );
    assert!(
        setup_ok,
        "account must be ingame with scene_state 2 at {:?} before Walk: {:?}",
        scenario.seed(),
        setup_error
    );
    assert!(
        setup_png,
        "save the real pre-script scene PNG before walking"
    );
    let walk = target.expect("setup evidence checked a selected native Walk target");

    let start_handle = play.script_start_handle();
    match start_handle.start_test_script(
        &account,
        Box::new(NativeWalkScript {
            state: Arc::clone(&state),
            walk: walk.clone(),
            handle: None,
        }),
        None,
    ) {
        Ok(run) => {
            if play.script_native_run(&account) != Some(run) {
                state.lock().start_error = Some("installed native run key was not retained".into());
            }
            play.wake(&account);
        }
        Err(error) => state.lock().start_error = Some(error),
    }

    let deadline = Instant::now() + WALK_TIMEOUT;
    let mut route = None;
    let mut timed_out = false;
    loop {
        if route.is_none() {
            route = route_sample(&play, &account);
        }
        let (terminal, start_error) = {
            let state = state.lock();
            (state.receipt.is_some(), state.start_error.clone())
        };
        if terminal || start_error.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            state.lock().start_error = Some(format!("native Walk wait exceeded {WALK_TIMEOUT:?}"));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(sample) = route_sample(&play, &account) {
        route = Some(sample);
    }
    let (scene, result, start_error) = {
        let state = state.lock();
        (
            state.scene,
            state.receipt.clone(),
            state.start_error.clone(),
        )
    };
    let arrived = matches!(result.as_ref(), Some(Ok(receipt)) if receipt.end == WalkEnd::Arrived);
    let samples = state.lock().samples.clone();
    std::fs::write(
        run_dir.join("tick-samples.json"),
        serde_json::to_vec_pretty(&samples).unwrap(),
    )
    .expect("write observed game ticks");
    let door_walk =
        (scenario == Scenario::LumbridgeDoor).then(|| lumbridge_door_walk_evidence(&samples));
    let door_crossing_proven = scenario != Scenario::LumbridgeDoor
        || (lumbridge_door_is_closed(setup_scene.lumbridge_door)
            && door_walk.is_some_and(|proof| {
                proof.opened_during_walk
                    && proof.player_on_door_tile_after_open
                    && proof.player_at_target_after_door_tile
            }));
    let mut final_report = evidence.document(
        if arrived {
            "native-walk-terminal"
        } else {
            "native-walk-failure"
        },
        Some(&walk),
        scene,
        route.as_ref(),
        result.as_ref(),
        start_error.as_deref(),
    );
    if let Some(proof) = door_walk {
        final_report["lumbridge_door_crossing"] = json!({
            "source": "observed GameSnapshot::locs() and per-game-tick player tiles",
            "driver": "native Walk; harness sends no door interaction",
            "closed_door_id": LUMBRIDGE_DOOR_CLOSED_ID,
            "closed_door_tile": LUMBRIDGE_DOOR_TILE,
            "closed_door_before_walk": door_observation_json(setup_scene.lumbridge_door),
            "open_door_id": LUMBRIDGE_DOOR_OPEN_ID,
            "open_door_tile": LUMBRIDGE_DOOR_OPEN_TILE,
            "walk_start": scenario.seed(),
            "walk_target": walk.target,
            "opened_during_walk": proof.opened_during_walk,
            "player_on_door_tile_after_open": proof.player_on_door_tile_after_open,
            "player_at_target_after_door_tile": proof.player_at_target_after_door_tile,
            "passed": door_crossing_proven,
            "content_timed_close_wait": state.lock().door_close_wait.clone(),
        });
    }
    let final_png = write_capture(
        &run_dir,
        &timestamp,
        "02-final-walk",
        !arrived || timed_out || start_error.is_some() || !door_crossing_proven,
        final_report,
        &mailbox,
    );

    assert!(
        final_png,
        "save the real final-scene PNG and matching receipt JSON"
    );
    assert!(
        !timed_out,
        "native Walk did not produce a retained terminal receipt: {result:?}"
    );
    assert!(
        start_error.is_none(),
        "native Walk script failed to start: {start_error:?}"
    );
    assert!(
        arrived,
        "the native Walk receipt must end at WalkEnd::Arrived, not another host/Traveller terminal: {result:?}"
    );
    assert!(
        door_crossing_proven,
        "lumbridge-door requires the native Walk to observe the closed door, its opened state, the door tile, and the target in order"
    );
    assert!(
        scene.ingame && scene.scene_state == 2 && scene.player_tile.is_some(),
        "final evidence must be from a ready live scene: {scene:?}"
    );
    let end = scene.player_tile.expect("final scene tile asserted above");
    assert_eq!(end.level, walk.target.level, "arrival plane");
    if scenario == Scenario::Return {
        assert!(
            chebyshev(end, walk.target) <= u32::from(walk.radius),
            "Return ended outside the requested anchor radius"
        );
    }
    if scenario == Scenario::Yew {
        assert!(walk.loc_id.is_some() && walk.loc_name.is_some());
        assert_eq!(
            scene.target_operable,
            Some(true),
            "the radius-1 yew walk must end on a legal full-footprint stand"
        );
        println!(
            "nav-arrival-yew-fixture id={} name={:?} tile={:?} actions={:?}",
            walk.loc_id.expect("Yew target has selected loc id"),
            walk.loc_name,
            walk.target,
            walk.loc_actions
        );
    }
}

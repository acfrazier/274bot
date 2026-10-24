//! Engine-A survival-eating receipts for the native route guard and the
//! public `api.combat.fight` path. The per-tick oracle below uses the slot's
//! published poison condition and the actions captured from the live host.
//!
//! Run each ignored cell three times with separate accounts. Use Engine A at
//! its normal 600 ms tick, a throwaway HOME (`isohome`), the v17 reference nav
//! pack, and the accepted read-only 1.1 cache inputs:
//!
//! ```sh
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<unique-prefix> \
//!   GATHERER_GAME_PORT=44594 GATHERER_HTTP_PORT=1080 \
//!   GATHERER_ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine \
//!   GATHERER_NAV_PACK=/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack \
//!   GATHERER_NAV_FLAGS=/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navflags \
//!   GATHERER_CATALOG_ROOT=<catalog-root> \
//!   BOT_CACHE_DIR=/Volumes/dev-scratch/274bot-evidence/SURVIVAL-1.1/live-cache/6dcb7c4ad1b85372 \
//!   CLIENT_UNPACK_DIR=/Volumes/dev-scratch/274bot-evidence/SURVIVAL-1.1/client-home/.274bot/unpack \
//!   LIVE_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/SURVIVAL-INT-123 \
//!   /Users/acfrazier/experiments/274bot/.superpowers/bin/isohome \
//!   cargo test -p host-play --features live-harness,test-support --lib \
//!   survival_eat_live_tests::<cell-name> -- --ignored --nocapture --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::condition::{BotCondition, PoisonState};
use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use parking_lot::Mutex;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkReceipt,
    WalkRequest,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{self, Cell, Prep};
use super::combat_proof::{self, CaptureRegistration, CombatCapture};
use super::{run_with_template, SharedClientTemplate};

const LOBSTER_ID: i32 = 379;
const PROTECT_FROM_MELEE_VARP: i32 = 97;
const PREPARATION_DEADLINE: Duration = Duration::from_secs(180);
const WALK_CELL_WALL_LIMIT: Duration = Duration::from_secs(360);
const COMBAT_CELL_WALL_LIMIT: Duration = Duration::from_secs(300);
const CELL_TICK_LIMIT: u32 = 100;
const POISON_PREFIX_TICKS: u32 = 31;
const POISON_LINE: &str = "You have been poisoned!";

const WALK_ORIGIN: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const WALK_TARGET: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};
const COW_FIELD: WorldTile = WorldTile {
    x: 3255,
    z: 3278,
    level: 0,
};

const POISON_LINE_SEED: &[&str] = &[
    "setstat hitpoints 15",
    "setstat prayer 43",
    "give lobster 5",
];
const FULL_HP_SEED: &[&str] = &[
    "setstat hitpoints 10",
    "setstat prayer 43",
    "give lobster 5",
];
const FRUIT_SEED: &[&str] = &[
    "setstat hitpoints 10",
    "setstat prayer 43",
    "give lobster 5",
    "give macro_triffidfruit 1",
];
const COMBAT_SEED: &[&str] = &[
    "setstat attack 1",
    "setstat strength 1",
    "setstat defence 1",
    "setstat hitpoints 10",
    "setstat prayer 43",
    "give bronze_scimitar 1",
    "give lobster 5",
    "setvar prayer14 1",
];
const COMBAT_FRUIT_SEED: &[&str] = &[
    "setstat attack 1",
    "setstat strength 1",
    "setstat defence 1",
    "setstat hitpoints 10",
    "setstat prayer 43",
    "give bronze_scimitar 1",
    "give lobster 5",
    "give macro_triffidfruit 1",
    "setvar prayer14 1",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EatCase {
    PoisonLine,
    FullHp,
}

impl EatCase {
    fn name(self) -> &'static str {
        match self {
            Self::PoisonLine => "L-EAT-POISON-LINE-WALK",
            Self::FullHp => "L-EAT-FULL-HP-WALK",
        }
    }

    fn seed(self) -> &'static [&'static str] {
        match self {
            Self::PoisonLine => POISON_LINE_SEED,
            Self::FullHp => FULL_HP_SEED,
        }
    }

    fn seed_ready(self) -> fn(&GameSnapshot) -> bool {
        match self {
            Self::PoisonLine => poison_line_seed_ready,
            Self::FullHp => full_hp_seed_ready,
        }
    }

    fn expected_hp(self) -> i32 {
        match self {
            Self::PoisonLine => 15,
            Self::FullHp => 10,
        }
    }

    fn expected_hp_max(self) -> i32 {
        match self {
            Self::PoisonLine => 15,
            Self::FullHp => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveAction {
    Poison,
}

#[derive(Debug, Clone, Copy)]
struct SentAction {
    tick: u32,
    accepted: bool,
}

#[derive(Default)]
struct LiveObserver {
    snapshot: GameSnapshot,
    pump: Pump,
    was_ingame: bool,
    final_login_tick: Option<u32>,
    tele_sent: bool,
    arrived_tick: Option<u32>,
    action: Option<LiveAction>,
    sent: Option<SentAction>,
}

impl LiveObserver {
    fn observe(&mut self, client: &client::client::Client) {
        // Preparation relogs. Keep the condition owner's session-local clock.
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if client.ingame && !self.was_ingame {
            self.final_login_tick = Some(self.snapshot.tick());
        }
        self.was_ingame = client.ingame;
    }
}

fn walk_poison_seed_ready(
    final_login: Option<u32>,
    tick: u32,
    route_active: bool,
    route_progressed: bool,
) -> bool {
    route_active
        && route_progressed
        && final_login.is_some_and(|login| tick.saturating_sub(login) >= POISON_PREFIX_TICKS)
}

struct LiveCell {
    play: super::Play,
    account: String,
    evidence: PathBuf,
    preparation: Arc<Mutex<Cell>>,
    live: Arc<Mutex<LiveObserver>>,
    capture: Arc<StdMutex<CombatCapture>>,
    _capture_registration: CaptureRegistration,
}

#[derive(Debug, Clone)]
struct RouteSample {
    request_id: u64,
    generation: u64,
    guard_active: bool,
    native_owner: bool,
    legs: usize,
    walking_tiles: usize,
}

impl RouteSample {
    fn active(&self) -> bool {
        self.guard_active && self.native_owner
    }

    fn evidence(&self) -> Value {
        json!({
            "request_id": self.request_id,
            "generation": self.generation,
            "guard_active": self.guard_active,
            "native_owner": self.native_owner,
            "legs": self.legs,
            "walking_tiles": self.walking_tiles,
        })
    }
}

#[derive(Default)]
struct WalkState {
    begin_attempted: bool,
    start_error: Option<String>,
    receipt: Option<Result<WalkReceipt, ActionError>>,
}

struct NativeWalkScript {
    state: Arc<Mutex<WalkState>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for NativeWalkScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let begin = {
            let mut state = self.state.lock();
            if state.begin_attempted {
                false
            } else {
                state.begin_attempted = true;
                true
            }
        };
        if begin {
            let request = WalkRequest {
                target: WALK_TARGET,
                candidate_goals: None,
                loc_id: None,
                radius: 12,
                arrival: nav_planner::arrival::ArrivalKind::Reach,
                options: script::native::WalkOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: Box::default(),
                protect: true,
                food_guard: false,
                allow: Default::default(),
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    self.state.lock().start_error = Some(format!("Walk begin: {error:?}"))
                }
            }
        }

        if let Some(handle) = &self.handle {
            if let std::task::Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                self.state.lock().receipt = Some(result);
                self.handle = None;
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

#[derive(Debug, Clone)]
struct ObservedFrame {
    tick: u32,
    hp: i32,
    hp_max: i32,
    lobsters: i32,
    poison: PoisonState,
    poison_line: bool,
    route_active: bool,
    protect_active: bool,
    non_poison_damage: bool,
    evidence: Value,
}

impl ObservedFrame {
    fn evidence(&self) -> Value {
        json!({
            "tick": self.tick,
            "hp": self.hp,
            "hp_max": self.hp_max,
            "lobsters": self.lobsters,
            "poison": format!("{:?}", self.poison),
            "poison_line": self.poison_line,
            "route_active": self.route_active,
            "protect_from_melee": self.protect_active,
            "non_poison_damage": self.non_poison_damage,
            "snapshot": self.evidence,
        })
    }
}

/// A captured action's session snapshot clock, never its host script counter.
/// Keep a missing clock visible so an oracle cannot discard an untimed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ActionSnapshotTick(Option<u32>);

impl ActionSnapshotTick {
    fn from_capture(action: &Value) -> Self {
        Self(
            action["snapshot_tick"]
                .as_u64()
                .and_then(|tick| u32::try_from(tick).ok()),
        )
    }
}

/// The only action-versus-condition time comparison in the survival oracles.
/// All three eating cells, including their heal deadlines, use this clock.
fn action_at_condition_offset(
    action: ActionSnapshotTick,
    condition_tick: u32,
    allowed: std::ops::RangeInclusive<i64>,
) -> bool {
    action
        .0
        .is_some_and(|tick| allowed.contains(&(i64::from(tick) - i64::from(condition_tick))))
}

#[derive(Debug, Clone)]
struct EatAction {
    tick: ActionSnapshotTick,
    hp_before: i32,
    hp_max: i32,
    lobsters_before: i32,
    accepted: bool,
    source: String,
    owner_run: Option<String>,
    fight_action: Option<u64>,
    real_wire: bool,
}

#[derive(Debug, Clone)]
struct AttackAction {
    tick: ActionSnapshotTick,
    accepted: bool,
    owner_run: Option<String>,
    fight_action: Option<u64>,
    real_wire: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OracleDecision {
    Pending,
    Pass,
    Fail(String),
}

fn evidence_root() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR")
            .expect("LIVE_EVIDENCE_DIR must name the request evidence directory"),
    );
    assert!(path.is_absolute(), "LIVE_EVIDENCE_DIR must be absolute");
    std::fs::create_dir_all(&path).expect("create LIVE_EVIDENCE_DIR");
    std::fs::canonicalize(path).expect("resolve LIVE_EVIDENCE_DIR")
}

fn stamp_utc() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs()
}

fn require_live_environment() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "requires BOT_CPU=1"
    );
    assert!(
        std::env::var_os("BOT_LIVE_NAME_PREFIX").is_some(),
        "requires this run's unique BOT_LIVE_NAME_PREFIX"
    );
}

fn start_cell(
    label: &'static str,
    seed: &'static [&'static str],
    seed_ready: fn(&GameSnapshot) -> bool,
    destination: WorldTile,
) -> LiveCell {
    require_live_environment();
    let evidence_root = evidence_root();
    let account = super::mint_live_names(1)
        .into_iter()
        .next()
        .expect("mint one account under BOT_LIVE_NAME_PREFIX");
    let evidence = evidence_root.join(format!("{label}_{}_utc-{}Z", account, stamp_utc()));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create per-account scratch directory");

    let profile = bank_core_live::live_profile(&scratch)
        .expect("live profile inputs")
        .resolve(None)
        .and_then(|profile| profile.bind_runtime())
        .expect("bind the Engine-A 289 profile");
    assert!(
        profile.map_members(),
        "poison requires a members map; this is a fixture precondition"
    );
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    let template = SharedClientTemplate::load(profile).expect("prepare the bound live template");

    let capture = Arc::new(StdMutex::new(CombatCapture::default()));
    let capture_registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let preparation = Arc::new(Mutex::new(Cell::new(
        Duration::from_secs(360),
        seed,
        seed_ready,
        &[LOBSTER_ID],
        json!({"cell": label, "destination": [destination.x, destination.z, destination.level]}),
        |_| false,
        evidence.clone(),
    )));
    let live = Arc::new(Mutex::new(LiveObserver::default()));
    let frame_preparation = Arc::clone(&preparation);
    let frame_live = Arc::clone(&live);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_311,
            settings: never_cure_settings(),
        }],
        |_| (None, None),
        move |client, username, _| {
            if username != frame_account {
                return;
            }
            bank_core_live::frame(client, &frame_preparation, &frame_account);
            let phase = frame_preparation.lock().phase;
            let mut live = frame_live.lock();
            live.observe(client);
            if phase == Prep::Ready && !live.tele_sent {
                interact::seed_at(client, destination.level, destination.x, destination.z);
                live.tele_sent = true;
            }
            if live.tele_sent
                && live.snapshot.tile() == Some((destination.x, destination.z, destination.level))
            {
                let tick = live.snapshot.tick();
                live.arrived_tick.get_or_insert(tick);
            }
            if let Some(LiveAction::Poison) = live.action.take() {
                let accepted = interact::cheat(client, "~poison 27") == client::CheatSend::Sent;
                let tick = live.snapshot.tick();
                live.sent = Some(SentAction { tick, accepted });
            }
        },
    )
    .expect("start one-slot Engine-A Play");

    LiveCell {
        play,
        account,
        evidence,
        preparation,
        live,
        capture,
        _capture_registration: capture_registration,
    }
}

fn never_cure_settings() -> ProfileSettings {
    let mut settings = ProfileSettings::default();
    settings.survival.cure_poison = vault::CurePoison::Never;
    settings
}

fn wait_for_destination(cell: &LiveCell) -> Result<u32, String> {
    let started = cell.preparation.lock().preparation_started;
    let deadline = started + PREPARATION_DEADLINE;
    loop {
        let status = cell
            .play
            .statuses()
            .into_iter()
            .find(|status| status.username == cell.account);
        let (phase, failure) = {
            let preparation = cell.preparation.lock();
            (preparation.phase, preparation.trace.failure.clone())
        };
        let (arrived_tick, snapshot_tick) = {
            let live = cell.live.lock();
            (live.arrived_tick, live.snapshot.tick())
        };
        if phase == Prep::PrerequisiteFailed || phase == Prep::Failed {
            return Err(format!("preparation ended in {phase:?}: {failure:?}"));
        }
        match (status, arrived_tick) {
            (Some(status), Some(arrived))
                if status.ingame
                    && status.scene_state == 2
                    && status.condition.tick == snapshot_tick =>
            {
                return Ok(arrived);
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account did not reach the seeded destination within {PREPARATION_DEADLINE:?}; phase={phase:?}, failure={failure:?}, arrived_tick={arrived_tick:?}, snapshot_tick={snapshot_tick}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn poison_line_seed_ready(snapshot: &GameSnapshot) -> bool {
    stat_levels(snapshot, 3) == Some((15, 15))
        && stat_levels(snapshot, 5) == Some((43, 43))
        && bank_core_live::count(snapshot.inventory(), LOBSTER_ID) == 5
}

fn full_hp_seed_ready(snapshot: &GameSnapshot) -> bool {
    stat_levels(snapshot, 3) == Some((10, 10))
        && stat_levels(snapshot, 5) == Some((43, 43))
        && bank_core_live::count(snapshot.inventory(), LOBSTER_ID) == 5
}

fn fruit_id() -> i32 {
    api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("289 content")
        .item_by_alias("macro_triffidfruit")
        .expect("Strange fruit")
        .id
}

fn fruit_seed_ready(snapshot: &GameSnapshot) -> bool {
    full_hp_seed_ready(snapshot) && bank_core_live::count(snapshot.inventory(), fruit_id()) == 1
}

fn combat_seed_ready(snapshot: &GameSnapshot) -> bool {
    [0, 1, 2, 3, 5].into_iter().all(|index| {
        let expected = match index {
            0..=2 => 1,
            3 => 10,
            5 => 43,
            _ => unreachable!(),
        };
        stat_levels(snapshot, index) == Some((expected, expected))
    }) && bank_core_live::count(snapshot.inventory(), LOBSTER_ID) == 5
        && snapshot.inventory().iter().any(|item| item.def.id == 1321)
        && varp(snapshot, PROTECT_FROM_MELEE_VARP) == 1
}

fn combat_fruit_seed_ready(snapshot: &GameSnapshot) -> bool {
    combat_seed_ready(snapshot) && bank_core_live::count(snapshot.inventory(), fruit_id()) == 1
}

fn stat_levels(snapshot: &GameSnapshot, index: i32) -> Option<(i32, i32)> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == index)
        .map(|stat| (stat.base, stat.effective))
}

fn varp(snapshot: &GameSnapshot, index: i32) -> i32 {
    snapshot
        .varps()
        .iter()
        .find(|row| row.index == index)
        .map_or(0, |row| row.value)
}

fn route_sample(play: &super::Play, account: &str) -> Option<RouteSample> {
    let navs = play.navs.lock().unwrap_or_else(|error| error.into_inner());
    let bot = navs.get(account)?;
    let (legs, walking_tiles) = match bot.route_projection()? {
        super::walk_map::RouteProjectionPath::Legacy(route) => (
            route.legs.len(),
            route
                .legs
                .iter()
                .map(|leg| match leg {
                    nav_planner::router::Leg::Walk { tiles } => tiles.len(),
                    nav_planner::router::Leg::Transport { .. } => 0,
                })
                .sum(),
        ),
        super::walk_map::RouteProjectionPath::Compact(view) => (
            view.leg_count(),
            view.legs()
                .map(|leg| match leg {
                    nav_planner::contract::LegView::Walk(tiles) => tiles.len(),
                    nav_planner::contract::LegView::Transport(_) => 0,
                })
                .sum(),
        ),
    };
    Some(RouteSample {
        request_id: bot.route_request_id,
        generation: bot.route_generation,
        guard_active: bot.walk_guard.is_some(),
        native_owner: bot.native_walk.is_some(),
        legs,
        walking_tiles,
    })
}

fn sample_frame(
    cell: &LiveCell,
    condition: BotCondition,
    route: Option<&RouteSample>,
) -> Option<ObservedFrame> {
    let live = cell.live.lock();
    let snapshot = &live.snapshot;
    let tick = condition.tick;
    if tick != snapshot.tick() {
        return None;
    }
    let raw_chat = snapshot
        .chat_lines()
        .iter()
        .map(|line| json!({"sequence": line.sequence, "text": line.text}))
        .collect::<Vec<_>>();
    let poison_line = snapshot
        .chat_lines()
        .iter()
        .any(|line| line.text == POISON_LINE);
    let raw_hitmarks = snapshot.hitmarks().map(|marks| {
        marks
            .marks
            .iter()
            .map(|mark| {
                json!({
                    "kind": mark.kind,
                    "value": mark.value,
                    "cycle": mark.cycle,
                    "loop_cycle": marks.loop_cycle,
                })
            })
            .collect::<Vec<_>>()
    });
    let non_poison_damage = snapshot.hitmarks().is_some_and(|marks| {
        marks
            .marks
            .iter()
            .any(|mark| mark.kind != 2 && mark.value > 0)
    });
    let protect_active = varp(snapshot, PROTECT_FROM_MELEE_VARP) == 1;
    let route_active = route.is_some_and(RouteSample::active);
    let fact = combat_proof::snapshot_facts(snapshot, Some(u64::from(tick)));
    let evidence = json!({
        "condition": {
            "tick": condition.tick,
            "hp": condition.hp,
            "hp_max": condition.hp_max,
            "hp_known": condition.hp_known,
            "poison": format!("{:?}", condition.poison),
            "cures": format!("{:?}", condition.cures),
        },
        "snapshot_facts": fact,
        "raw_chat": raw_chat,
        "raw_hitmarks": raw_hitmarks,
        "route": route.map(RouteSample::evidence),
        "protect_from_melee": protect_active,
    });
    Some(ObservedFrame {
        tick,
        hp: i32::from(condition.hp),
        hp_max: i32::from(condition.hp_max),
        lobsters: bank_core_live::count(snapshot.inventory(), LOBSTER_ID),
        poison: condition.poison,
        poison_line,
        route_active,
        protect_active,
        non_poison_damage,
        evidence,
    })
}

fn captured_actions(capture: &Arc<StdMutex<CombatCapture>>) -> Vec<Value> {
    capture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .actions
        .clone()
}

fn capture_sample(
    capture: &Arc<StdMutex<CombatCapture>>,
    frame: ObservedFrame,
    last_tick: &mut Option<u32>,
    frames: &mut Vec<ObservedFrame>,
) -> Vec<Value> {
    // Actions can arrive after the first sample of their snapshot tick.
    let actions = captured_actions(capture);
    if *last_tick != Some(frame.tick) {
        *last_tick = Some(frame.tick);
        frames.push(frame);
    }
    actions
}

fn is_eat(action: &Value) -> bool {
    (action["kind"] == json!("guard")
        && action["op"] == json!("eat")
        && action["item"]
            .as_str()
            .is_none_or(|name| name.eq_ignore_ascii_case("lobster")))
        || (action["kind"] == json!("interaction")
            && action["request"]["op"] == json!("held")
            && action["request"]["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("lobster"))
            && action["request"]["action"]
                .as_str()
                .is_some_and(|verb| verb.eq_ignore_ascii_case("eat")))
}

fn is_fight_attack(action: &Value) -> bool {
    action["kind"] == json!("interaction")
        && action["request"]["op"] == json!("npc")
        && action["request"]["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("cow"))
        && action["request"]["action"]
            .as_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("attack"))
}

fn has_decoded_wire(action: &Value) -> bool {
    action["wire_decoded"] == json!(true)
        && action["wire_opcodes"]
            .as_array()
            .is_some_and(|opcodes| !opcodes.is_empty())
}

fn snapshot_stat(snapshot: &Value, index: i64) -> Option<i32> {
    snapshot["stats"]
        .as_array()?
        .iter()
        .find(|row| row["index"] == json!(index))?
        .get("effective")?
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
}

fn snapshot_stat_base(snapshot: &Value, index: i64) -> Option<i32> {
    snapshot["stats"]
        .as_array()?
        .iter()
        .find(|row| row["index"] == json!(index))?
        .get("base")?
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
}

fn snapshot_item_count(snapshot: &Value, id: i64) -> i32 {
    snapshot["inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["id"] == json!(id))
        .filter_map(|row| row["count"].as_i64())
        .filter_map(|count| i32::try_from(count).ok())
        .sum()
}

fn eat_actions(actions: &[Value]) -> Vec<EatAction> {
    actions
        .iter()
        .filter(|action| is_eat(action))
        .filter_map(|action| {
            let snapshot = &action["snapshot"];
            Some(EatAction {
                tick: ActionSnapshotTick::from_capture(action),
                hp_before: snapshot_stat(snapshot, 3)?,
                hp_max: snapshot_stat_base(snapshot, 3)?,
                lobsters_before: snapshot_item_count(snapshot, i64::from(LOBSTER_ID)),
                accepted: action["accepted"] == json!(true),
                source: action["kind"].as_str()?.to_owned(),
                owner_run: action["run"].as_str().map(str::to_owned),
                fight_action: action["action_id"].as_u64(),
                real_wire: has_decoded_wire(action),
            })
        })
        .collect()
}

fn attack_actions(actions: &[Value]) -> Vec<AttackAction> {
    actions
        .iter()
        .filter(|action| is_fight_attack(action))
        .map(|action| AttackAction {
            tick: ActionSnapshotTick::from_capture(action),
            accepted: action["accepted"] == json!(true),
            owner_run: action["run"].as_str().map(str::to_owned),
            fight_action: action["action_id"].as_u64(),
            real_wire: has_decoded_wire(action),
        })
        .collect()
}

fn poison_marks(frames: &[ObservedFrame]) -> Vec<&ObservedFrame> {
    let mut last_mark = None;
    let mut marks = Vec::new();
    for frame in frames {
        if let PoisonState::Poisoned {
            per_tick: _,
            last_mark: mark,
            ..
        } = frame.poison
        {
            if last_mark != Some(mark) {
                last_mark = Some(mark);
                marks.push(frame);
            }
        }
    }
    marks
}

fn onset(frames: &[ObservedFrame]) -> Option<(u32, u32)> {
    frames.iter().find_map(|frame| match frame.poison {
        PoisonState::Onset {
            since, q: Some(11), ..
        } => Some((frame.tick, u32::from(since))),
        _ => None,
    })
}

fn healed_after(frames: &[ObservedFrame], eat: &EatAction, expected_hp: i32) -> bool {
    let expected_food = eat.lobsters_before - 1;
    frames.iter().any(|frame| {
        action_at_condition_offset(eat.tick, frame.tick, -3..=0)
            && frame.hp == expected_hp
            && frame.lobsters == expected_food
    })
}

fn evaluate_walk(
    case: EatCase,
    seed_tick: u32,
    frames: &[ObservedFrame],
    eats: &[EatAction],
) -> OracleDecision {
    if eats.iter().any(|eat| eat.tick.0.is_none()) {
        return OracleDecision::Fail("oracle: captured Eat has no explicit snapshot_tick".into());
    }
    if frames.iter().any(|frame| frame.hp == 0) {
        return OracleDecision::Fail("product: account died during the walk cell".into());
    }
    if let Some(frame) = frames.iter().find(|frame| frame.non_poison_damage) {
        return OracleDecision::Fail(format!(
            "interference: non-poison hitmark at observed tick {}",
            frame.tick
        ));
    }
    if let Some(frame) = frames
        .iter()
        .find(|frame| frame.tick >= seed_tick && !frame.route_active)
    {
        return OracleDecision::Fail(format!(
            "product: native route guard was not active at observed tick {}",
            frame.tick
        ));
    }
    let Some((onset_tick, since)) = onset(frames) else {
        if frames
            .last()
            .is_some_and(|frame| frame.tick.saturating_sub(seed_tick) > CELL_TICK_LIMIT)
        {
            return OracleDecision::Fail(
                "product: accepted poison seed had no latched q=11 onset".into(),
            );
        }
        return OracleDecision::Pending;
    };
    let onset_frame = frames
        .iter()
        .find(|frame| frame.tick == onset_tick)
        .expect("onset frame");
    if !onset_frame.poison_line
        || onset_frame.hp != case.expected_hp()
        || onset_frame.hp_max != case.expected_hp_max()
    {
        return OracleDecision::Fail(format!(
            "fixture: poison line or seeded HP did not match at onset: line={}, hp={}/{}, expected={}/{}",
            onset_frame.poison_line,
            onset_frame.hp,
            onset_frame.hp_max,
            case.expected_hp(),
            case.expected_hp_max()
        ));
    }
    if eats
        .iter()
        .any(|eat| action_at_condition_offset(eat.tick, onset_tick, i64::MIN..=-1))
    {
        return OracleDecision::Fail("product: food was emitted before the poison line".into());
    }
    let marks = poison_marks(frames);
    let first_due = since.saturating_add(30);
    let Some(first) = marks.first().copied() else {
        if frames
            .last()
            .is_some_and(|frame| frame.tick > first_due.saturating_add(2))
        {
            return OracleDecision::Fail(
                "product: first poison mark did not reach the slot condition".into(),
            );
        }
        return OracleDecision::Pending;
    };
    if first.tick < first_due
        || first.tick > first_due.saturating_add(2)
        || !matches!(first.poison, PoisonState::Poisoned { per_tick: 6, .. })
    {
        return OracleDecision::Fail(format!(
            "fixture: first poison mark did not match 6 damage at since+30..32: since={since}, tick={}, poison={:?}",
            first.tick, first.poison
        ));
    }
    let expected_mark_hp = match case {
        EatCase::PoisonLine => 9,
        EatCase::FullHp => 4,
    };
    if first.hp != expected_mark_hp {
        return OracleDecision::Fail(format!(
            "fixture: first poison mark left HP {}, expected {expected_mark_hp}",
            first.hp
        ));
    }

    match case {
        EatCase::PoisonLine => {
            let Some(second) = marks.get(1).copied() else {
                if eats
                    .iter()
                    .any(|eat| action_at_condition_offset(eat.tick, onset_tick, 0..=i64::MAX))
                {
                    return OracleDecision::Fail(
                        "product: food was emitted before the second poison mark".into(),
                    );
                }
                if frames
                    .last()
                    .is_some_and(|frame| frame.tick > first.tick.saturating_add(32))
                {
                    return OracleDecision::Fail(
                        "product: second poison mark did not reach the slot condition".into(),
                    );
                }
                return OracleDecision::Pending;
            };
            if second.tick != first.tick.saturating_add(30)
                || !matches!(second.poison, PoisonState::Poisoned { per_tick: 6, .. })
                || second.hp != 3
            {
                return OracleDecision::Fail(format!(
                    "fixture: second poison mark mismatch at tick {}: hp={}, poison={:?}",
                    second.tick, second.hp, second.poison
                ));
            }
            if eats
                .iter()
                .any(|eat| action_at_condition_offset(eat.tick, second.tick, i64::MIN..=-1))
            {
                return OracleDecision::Fail(
                    "product: food was emitted before the second poison mark".into(),
                );
            }
            let due = eats
                .iter()
                .filter(|eat| action_at_condition_offset(eat.tick, second.tick, 0..=3))
                .collect::<Vec<_>>();
            if due.len() > 1 {
                return OracleDecision::Fail(
                    "product: multiple bites followed the second poison mark".into(),
                );
            }
            let Some(eat) = due.first() else {
                if frames
                    .last()
                    .is_some_and(|frame| frame.tick > second.tick.saturating_add(3))
                {
                    return OracleDecision::Fail(
                        "product: no bite within three observed ticks of the second poison mark"
                            .into(),
                    );
                }
                return OracleDecision::Pending;
            };
            if !eat.accepted
                || eat.source != "guard"
                || eat.hp_before != 3
                || eat.hp_max != 15
                || eat.lobsters_before != 5
            {
                return OracleDecision::Fail(format!(
                    "product: invalid native guard eat action: {eat:?}"
                ));
            }
            if healed_after(frames, eat, 15) {
                OracleDecision::Pass
            } else if frames.last().is_some_and(|frame| {
                action_at_condition_offset(eat.tick, frame.tick, i64::MIN..=-4)
            }) {
                OracleDecision::Fail(
                    "product: lobster action did not heal by 12 HP and consume one lobster".into(),
                )
            } else {
                OracleDecision::Pending
            }
        }
        EatCase::FullHp => {
            if eats
                .iter()
                .any(|eat| action_at_condition_offset(eat.tick, first.tick, i64::MIN..=-1))
            {
                return OracleDecision::Fail(
                    "product: food was emitted at full HP before the first poison mark".into(),
                );
            }
            let first_due_eats = eats
                .iter()
                .filter(|eat| action_at_condition_offset(eat.tick, first.tick, 0..=3))
                .collect::<Vec<_>>();
            if first_due_eats.len() > 1 {
                return OracleDecision::Fail(
                    "product: multiple bites followed the first full-HP poison mark".into(),
                );
            }
            let Some(first_eat) = first_due_eats.first() else {
                if frames
                    .last()
                    .is_some_and(|frame| frame.tick > first.tick.saturating_add(3))
                {
                    return OracleDecision::Fail(
                        "product: no bite within three observed ticks of the first poison mark"
                            .into(),
                    );
                }
                return OracleDecision::Pending;
            };
            if !first_eat.accepted
                || first_eat.source != "guard"
                || first_eat.hp_before != 4
                || first_eat.hp_max != 10
                || first_eat.lobsters_before != 5
            {
                return OracleDecision::Fail(format!(
                    "product: invalid first native guard eat action: {first_eat:?}"
                ));
            }
            if !healed_after(frames, first_eat, 10) {
                if frames.last().is_some_and(|frame| {
                    action_at_condition_offset(first_eat.tick, frame.tick, i64::MIN..=-4)
                }) {
                    return OracleDecision::Fail(
                        "product: first lobster did not heal to full HP".into(),
                    );
                }
                return OracleDecision::Pending;
            }
            let Some(second) = marks.get(1).copied() else {
                if eats.iter().any(|eat| {
                    eat.tick > first_eat.tick
                        && action_at_condition_offset(
                            eat.tick,
                            first.tick.saturating_add(30),
                            i64::MIN..=-1,
                        )
                }) {
                    return OracleDecision::Fail(
                        "product: a second bite occurred before the second poison mark".into(),
                    );
                }
                if frames
                    .last()
                    .is_some_and(|frame| frame.tick > first.tick.saturating_add(32))
                {
                    return OracleDecision::Fail(
                        "product: second poison mark did not reach the slot condition".into(),
                    );
                }
                return OracleDecision::Pending;
            };
            if second.tick != first.tick.saturating_add(30)
                || !matches!(second.poison, PoisonState::Poisoned { per_tick: 6, .. })
                || second.hp != 4
            {
                return OracleDecision::Fail(format!(
                    "fixture: second poison mark mismatch at tick {}: hp={}, poison={:?}",
                    second.tick, second.hp, second.poison
                ));
            }
            if eats.iter().any(|eat| {
                eat.tick > first_eat.tick
                    && action_at_condition_offset(eat.tick, second.tick, i64::MIN..=-1)
            }) {
                return OracleDecision::Fail(
                    "product: a second bite occurred before the second poison mark".into(),
                );
            }
            let due = eats
                .iter()
                .filter(|eat| action_at_condition_offset(eat.tick, second.tick, 0..=3))
                .collect::<Vec<_>>();
            if due.len() > 1 {
                return OracleDecision::Fail(
                    "product: multiple bites followed the second poison mark".into(),
                );
            }
            let Some(eat) = due.first() else {
                if frames
                    .last()
                    .is_some_and(|frame| frame.tick > second.tick.saturating_add(3))
                {
                    return OracleDecision::Fail(
                        "product: no second bite within three observed ticks of the second mark"
                            .into(),
                    );
                }
                return OracleDecision::Pending;
            };
            if !eat.accepted
                || eat.source != "guard"
                || eat.hp_before != 4
                || eat.hp_max != 10
                || eat.lobsters_before != 4
            {
                return OracleDecision::Fail(format!(
                    "product: invalid second native guard eat action: {eat:?}"
                ));
            }
            if healed_after(frames, eat, 10) {
                OracleDecision::Pass
            } else if frames.last().is_some_and(|frame| {
                action_at_condition_offset(eat.tick, frame.tick, i64::MIN..=-4)
            }) {
                OracleDecision::Fail("product: second lobster did not heal to full HP".into())
            } else {
                OracleDecision::Pending
            }
        }
    }
}

/// Food and Strange fruit share %eat_delay: a bite at T permits fruit at T+3.
/// consume.rs2:109,122-127. Fruit removes itself before its cure message
/// (effects/scripts/strange_fruit.rs2:6-12), so count drop and a cleared poison
/// condition prove cure.
fn evaluate_fruit(
    frames: &[ObservedFrame],
    actions: &[Value],
    policy_tick: Option<u32>,
) -> OracleDecision {
    let eats = eat_actions(actions);
    let Some(bite) = eats.first() else {
        return OracleDecision::Pending;
    };
    let Some(bite_tick) = bite.tick.0 else {
        return OracleDecision::Fail("oracle: fruit cell bite has no snapshot tick".into());
    };
    if frames
        .iter()
        .filter(|frame| frame.tick >= bite_tick)
        .any(|frame| {
            frame.hp == 0
                || frame.non_poison_damage
                || (bite.source == "guard" && !frame.route_active)
        })
    {
        return OracleDecision::Fail(
            "fixture: fruit route died, lost its guard, or took other damage".into(),
        );
    }
    if eats.len() != 1 || !bite.accepted || bite.hp_before != 4 || bite.lobsters_before != 5 {
        return OracleDecision::Fail(
            "product: fruit cell needs exactly one due health bite".into(),
        );
    }
    let Some(policy_tick) = policy_tick else {
        return OracleDecision::Pending;
    };
    if policy_tick != bite_tick {
        return OracleDecision::Fail(
            "fixture: cure policy was not enabled immediately after the bite".into(),
        );
    }
    let fruits = actions
        .iter()
        .filter(|action| {
            (action["kind"] == "guard"
                && action["op"] == "eat"
                && action["item"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Strange fruit")))
                || (action["kind"] == "interaction"
                    && action["request"]["op"] == "held"
                    && action["request"]["action"] == "Eat"
                    && action["request"]["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("Strange fruit")))
        })
        .collect::<Vec<_>>();
    let due = bite_tick + 3;
    if fruits
        .iter()
        .any(|action| action["snapshot_tick"].as_u64() != Some(u64::from(due)))
    {
        return OracleDecision::Fail(
            "product: fruit did not use the first content-allowed tick T+3".into(),
        );
    }
    if fruits.len() > 1 {
        return OracleDecision::Fail("product: fruit was retried instead of cured once".into());
    }
    if bite.source == "interaction"
        && fruits.iter().any(|action| {
            !has_decoded_wire(action)
                || action["run"].as_str() != bite.owner_run.as_deref()
                || action["action_id"].as_u64() != bite.fight_action
        })
    {
        return OracleDecision::Fail(
            "oracle: fruit did not share the real wire-decoded Eat fight action".into(),
        );
    }
    if let Some(action) = fruits.first() {
        if action["accepted"] != true {
            return OracleDecision::Fail("product: fruit click was not accepted".into());
        }
        if frames.iter().any(|frame| {
            (due..=due + 3).contains(&frame.tick)
                && matches!(
                    frame.poison,
                    PoisonState::Clear | PoisonState::Immune { .. }
                )
                && frame.hp == 10
                && frame.lobsters == 4
                && snapshot_item_count(&frame.evidence["snapshot_facts"], i64::from(fruit_id()))
                    == 0
        }) {
            return OracleDecision::Pass;
        }
    }
    if frames.last().is_some_and(|frame| frame.tick > due + 3) {
        OracleDecision::Fail(
            "product: fruit did not cure within its content observation window".into(),
        )
    } else {
        OracleDecision::Pending
    }
}

fn evaluate_combat(
    seed_tick: u32,
    frames: &[ObservedFrame],
    eats: &[EatAction],
    attacks: &[AttackAction],
) -> OracleDecision {
    if eats.iter().any(|eat| eat.tick.0.is_none())
        || attacks.iter().any(|attack| attack.tick.0.is_none())
    {
        return OracleDecision::Fail(
            "oracle: captured combat action has no explicit snapshot_tick".into(),
        );
    }
    if frames.iter().any(|frame| frame.hp == 0) {
        return OracleDecision::Fail("product: account died during the combat cell".into());
    }
    if let Some(frame) = frames.iter().find(|frame| frame.non_poison_damage) {
        return OracleDecision::Fail(format!(
            "interference: an unprotected non-poison hit landed at observed tick {}",
            frame.tick
        ));
    }
    let Some((onset_tick, since)) = onset(frames) else {
        if frames
            .last()
            .is_some_and(|frame| frame.tick.saturating_sub(seed_tick) > CELL_TICK_LIMIT)
        {
            return OracleDecision::Fail(
                "product: accepted poison seed had no latched q=11 onset".into(),
            );
        }
        return OracleDecision::Pending;
    };
    let onset_frame = frames
        .iter()
        .find(|frame| frame.tick == onset_tick)
        .expect("onset frame");
    if !onset_frame.poison_line || onset_frame.hp != 10 || onset_frame.hp_max != 10 {
        return OracleDecision::Fail(format!(
            "fixture: combat poison onset did not preserve full HP: line={}, hp={}/{}, expected=10",
            onset_frame.poison_line, onset_frame.hp, onset_frame.hp_max
        ));
    }
    if frames
        .iter()
        .filter(|frame| frame.tick >= onset_tick)
        .any(|frame| !frame.protect_active)
    {
        return OracleDecision::Fail(
            "fixture: Protect from Melee was not held through the poison fire".into(),
        );
    }
    let marks = poison_marks(frames);
    let Some(first) = marks.first().copied() else {
        if frames
            .last()
            .is_some_and(|frame| frame.tick > since.saturating_add(32))
        {
            return OracleDecision::Fail(
                "product: first poison mark did not reach the combat condition".into(),
            );
        }
        return OracleDecision::Pending;
    };
    if first.tick < since.saturating_add(30)
        || first.tick > since.saturating_add(32)
        || !matches!(first.poison, PoisonState::Poisoned { per_tick: 6, .. })
        || first.hp != 4
    {
        return OracleDecision::Fail(format!(
            "fixture: first combat poison mark mismatch at {}: hp={}, poison={:?}",
            first.tick, first.hp, first.poison
        ));
    }
    if eats
        .iter()
        .any(|eat| action_at_condition_offset(eat.tick, first.tick, i64::MIN..=-1))
    {
        return OracleDecision::Fail(
            "product: combat ate before the first poison mark at full HP".into(),
        );
    }
    let due = eats
        .iter()
        .filter(|eat| action_at_condition_offset(eat.tick, first.tick, 0..=3))
        .collect::<Vec<_>>();
    if due.len() > 1 {
        return OracleDecision::Fail(
            "product: multiple combat bites followed the first poison mark".into(),
        );
    }
    let Some(eat) = due.first() else {
        if frames
            .last()
            .is_some_and(|frame| frame.tick > first.tick.saturating_add(3))
        {
            return OracleDecision::Fail(
                "product: api.combat.fight did not eat within three ticks of poison damage".into(),
            );
        }
        return OracleDecision::Pending;
    };
    if !eat.accepted
        || !eat.real_wire
        || eat.source != "interaction"
        || eat.hp_before != 4
        || eat.hp_max != 10
        || eat.lobsters_before != 5
    {
        return OracleDecision::Fail(format!(
            "product: API combat did not emit an accepted real Lobster action: {eat:?}"
        ));
    }
    let same_fight = attacks.iter().any(|attack| {
        attack.accepted
            && attack.real_wire
            && attack.tick <= eat.tick
            && attack.owner_run.is_some()
            && attack.owner_run == eat.owner_run
            && attack.fight_action.is_some_and(|id| id != 0)
            && attack.fight_action == eat.fight_action
    });
    if !same_fight {
        return OracleDecision::Fail(
            "oracle: no accepted wire-decoded Cow attack shares the Eat fight action".into(),
        );
    }
    if healed_after(frames, eat, 10) {
        OracleDecision::Pass
    } else if frames
        .last()
        .is_some_and(|frame| action_at_condition_offset(eat.tick, frame.tick, i64::MIN..=-4))
    {
        OracleDecision::Fail(
            "product: API combat Lobster action did not heal to full and consume one item".into(),
        )
    } else {
        OracleDecision::Pending
    }
}

fn captured_evidence(capture: &Arc<StdMutex<CombatCapture>>) -> Value {
    let capture = capture.lock().unwrap_or_else(|error| error.into_inner());
    json!({
        "start_baseline": capture.start_baseline,
        "frames": capture.frames,
        "statuses": capture.statuses,
        "actions": capture.actions,
        "random_events": capture.random_events,
        "observations": capture.observations,
        "prayer_facts": capture.prayer_facts,
        "invalid_reason": capture.invalid_reason,
    })
}

fn write_receipt(evidence: &Path, receipt: &Value) -> PathBuf {
    std::fs::create_dir_all(evidence).expect("create cell evidence directory");
    let mut receipt = receipt.clone();
    crate::evidence_writer::record_source_identity(&mut receipt);
    let path = evidence.join("receipt.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
    )
    .unwrap_or_else(|error| panic!("write receipt {}: {error}", path.display()));
    println!("survival-eat-receipt={}", path.display());
    path
}

fn walk_receipt(
    cell: &LiveCell,
    case: EatCase,
    setup_tick: u32,
    poison_send: Option<SentAction>,
    frames: &[ObservedFrame],
    failure: Option<(&str, String)>,
    walk_state: &Arc<Mutex<WalkState>>,
) -> Value {
    let seed_tick = poison_send
        .filter(|sent| sent.accepted)
        .map(|sent| sent.tick);
    let raw_actions = captured_actions(&cell.capture);
    let eats = eat_actions(&raw_actions);
    let walk_state = walk_state.lock();
    let walk_result = walk_state
        .receipt
        .as_ref()
        .map(|result| format!("{result:?}"));
    json!({
        "cell": case.name(),
        "real_content": true,
        "account": cell.account,
        "map_members": true,
        "cure_poison": "never",
        "start": [WALK_ORIGIN.x, WALK_ORIGIN.z, WALK_ORIGIN.level],
        "target": [WALK_TARGET.x, WALK_TARGET.z, WALK_TARGET.level],
        "setup_tick": setup_tick,
        "final_login_tick": cell.live.lock().final_login_tick,
        "poison_seed_tick": seed_tick,
        "poison_seed_sent": poison_send.map(|sent| json!({"tick": sent.tick, "accepted": sent.accepted})),
        "outcome": if failure.is_none() { "PASS" } else { "FAIL" },
        "classification": failure.as_ref().map(|(class, _)| *class),
        "failure": failure.map(|(_, reason)| reason),
        "walk_result": walk_result,
        "frames": frames.iter().map(ObservedFrame::evidence).collect::<Vec<_>>(),
        "host_capture": captured_evidence(&cell.capture),
        "eat_actions": eats.iter().map(|eat| json!({
            "tick": eat.tick.0,
            "snapshot_tick": eat.tick.0,
            "hp_before": eat.hp_before,
            "hp_max": eat.hp_max,
            "lobsters_before": eat.lobsters_before,
            "accepted": eat.accepted,
            "source": eat.source,
            "owner_run": eat.owner_run,
            "real_wire": eat.real_wire,
        })).collect::<Vec<_>>(),
        "raw_actions": raw_actions,
        "preparation_phase": format!("{:?}", cell.preparation.lock().phase),
    })
}

fn run_walk_case(case: EatCase, fruit_delay: bool) {
    let label = if fruit_delay {
        "L-CURE-FRUIT-EAT-DELAY"
    } else {
        case.name()
    };
    let (seed, ready) = if fruit_delay {
        (FRUIT_SEED, fruit_seed_ready as fn(&GameSnapshot) -> bool)
    } else {
        (case.seed(), case.seed_ready())
    };
    let cell = start_cell(label, seed, ready, WALK_ORIGIN);
    let mut frames = Vec::new();
    let mut failure = None;
    let mut passed = false;
    let setup_tick = match wait_for_destination(&cell) {
        Ok(tick) => tick,
        Err(error) => {
            failure = Some(("fixture", error));
            0
        }
    };
    let walk_state = Arc::new(Mutex::new(WalkState::default()));
    let mut seed_tick = None;
    let mut poison_send = None;
    let mut seed_scheduled = false;
    let mut policy_tick = None;
    if failure.is_none() {
        {
            let live = cell.live.lock();
            combat_proof::record_start_baseline(&cell.account, &live.snapshot);
        }
        let script_start = cell.play.script_start_handle().start_test_script(
            &cell.account,
            Box::new(NativeWalkScript {
                state: Arc::clone(&walk_state),
                handle: None,
            }),
            None,
        );
        match script_start {
            Ok(_) => cell.play.wake(&cell.account),
            Err(error) => {
                failure = Some(("product", format!("native Walk did not start: {error}")))
            }
        }
    }

    let deadline = Instant::now() + WALK_CELL_WALL_LIMIT;
    let mut last_tick = None;
    while failure.is_none() && !passed {
        if Instant::now() >= deadline {
            failure = Some((
                "fixture",
                format!("wall watchdog expired after {WALK_CELL_WALL_LIMIT:?}"),
            ));
            break;
        }
        let status = cell
            .play
            .statuses()
            .into_iter()
            .find(|status| status.username == cell.account);
        let route = route_sample(&cell.play, &cell.account);
        let Some(status) = status else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let Some(frame) = sample_frame(&cell, status.condition, route.as_ref()) else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let tick = frame.tick;
        let actions = capture_sample(&cell.capture, frame, &mut last_tick, &mut frames);
        if fruit_delay && policy_tick.is_none() && !eat_actions(&actions).is_empty() {
            if let Some(arm) = cell.play.arm(&cell.account) {
                arm.set_cure_poison(vault::CurePoison::WhenNeeded);
                policy_tick = Some(tick);
            } else {
                failure = Some((
                    "fixture",
                    "live arm disappeared before enabling cure".into(),
                ));
            }
        }
        if seed_tick.is_none() && !seed_scheduled && {
            let live = cell.live.lock();
            walk_poison_seed_ready(
                live.final_login_tick,
                tick,
                route.as_ref().is_some_and(RouteSample::active),
                live.snapshot
                    .tile()
                    .is_some_and(|tile| tile != (WALK_ORIGIN.x, WALK_ORIGIN.z, WALK_ORIGIN.level)),
            )
        } {
            cell.live.lock().action = Some(LiveAction::Poison);
            seed_scheduled = true;
        }
        if let Some(sent) = cell.live.lock().sent.take() {
            poison_send = Some(sent);
            if sent.accepted {
                seed_tick = Some(sent.tick);
            } else {
                failure = Some((
                    "fixture",
                    format!("~poison 27 was refused at tick {}", sent.tick),
                ));
            }
        }
        if let Some(seed) = seed_tick {
            if tick.saturating_sub(seed) > CELL_TICK_LIMIT {
                failure = Some((
                    "product",
                    format!("100 observed-tick oracle bound expired at tick {tick}"),
                ));
            }
        }
        let eats = eat_actions(&actions);
        let decision = seed_tick.map_or(OracleDecision::Pending, |seed| {
            if fruit_delay {
                evaluate_fruit(&frames, &actions, policy_tick)
            } else {
                evaluate_walk(case, seed, &frames, &eats)
            }
        });
        match decision {
            OracleDecision::Pending => {}
            OracleDecision::Pass => passed = true,
            OracleDecision::Fail(reason) => {
                let class = if reason.starts_with("interference:") {
                    "interference"
                } else if reason.starts_with("fixture:") {
                    "fixture"
                } else if reason.starts_with("oracle:") {
                    "oracle"
                } else {
                    "product"
                };
                failure = Some((class, reason));
            }
        }
        let walk = walk_state.lock();
        if failure.is_none() && walk.start_error.is_some() {
            failure = Some(("product", walk.start_error.clone().unwrap_or_default()));
        }
        if failure.is_none() && walk.receipt.is_some() && !passed && seed_tick.is_some() {
            failure = Some((
                "product",
                format!(
                    "native walk ended before the eat oracle passed: {:?}",
                    walk.receipt
                ),
            ));
        }
        drop(walk);
        if failure.is_none() && !passed && status.condition.hp == 0 {
            failure = Some(("product", format!("account died at observed tick {tick}")));
        }
        if failure.is_none() && !passed {
            thread::sleep(Duration::from_millis(50));
        }
    }
    cell.play.script_stop(&cell.account);
    let mut receipt = walk_receipt(
        &cell,
        case,
        setup_tick,
        poison_send,
        &frames,
        failure
            .as_ref()
            .map(|(class, reason)| (*class, reason.clone())),
        &walk_state,
    );
    receipt["cell"] = json!(label);
    if fruit_delay {
        receipt["cure_poison"] = json!("never, then when-needed after the bite");
        receipt["policy_change_tick"] = json!(policy_tick);
        receipt["fruit_first_allowed_offset"] = json!(3);
    }
    let receipt_path = write_receipt(&cell.evidence, &receipt);
    assert!(
        passed,
        "{} failed: {}; receipt={}",
        case.name(),
        failure.map_or_else(|| "no passing receipt".to_owned(), |(_, reason)| reason),
        receipt_path.display()
    );
}

const FIGHT_CONSUMER: &str = r#"export const apiVersion = 2;
export function tick(api) {
  if (!globalThis.__eatFight) {
    globalThis.__eatFight = api.combat.fight({
      target: { npc: 'Cow' },
      area: { min_x: 3240, min_z: 3254, max_x: 3266, max_z: 3299, level: 0 },
      prayer: true,
      food: true,
      budgetTicks: 200,
    });
    globalThis.__eatFight.then((value) => {
      globalThis.__eatFightOutcome = value;
      globalThis.__eatFight = null;
    });
  }
}"#;

fn run_combat_cell(fruit_delay: bool) {
    let label = if fruit_delay {
        "L-CURE-FRUIT-API-COMBAT"
    } else {
        "L-EAT-POISON-API-COMBAT"
    };
    let (seed, ready) = if fruit_delay {
        (
            COMBAT_FRUIT_SEED,
            combat_fruit_seed_ready as fn(&GameSnapshot) -> bool,
        )
    } else {
        (COMBAT_SEED, combat_seed_ready as fn(&GameSnapshot) -> bool)
    };
    let cell = start_cell(label, seed, ready, COW_FIELD);
    let mut frames = Vec::new();
    let mut failure = None;
    let mut passed = false;
    let setup_tick = match wait_for_destination(&cell) {
        Ok(tick) => tick,
        Err(error) => {
            failure = Some(("fixture", error));
            0
        }
    };
    let mut fight_start_tick = None;
    let mut seed_tick = None;
    let mut poison_send = None;
    let mut seed_scheduled = false;
    let mut policy_tick = None;
    if failure.is_none() {
        {
            let live = cell.live.lock();
            combat_proof::record_start_baseline(&cell.account, &live.snapshot);
        }
        match cell.play.script_start_load(
            &cell.account,
            FIGHT_CONSUMER.to_owned(),
            script::LoadShape::NativeTick,
            None,
            vec![],
        ) {
            Ok(()) => {
                cell.play.wake(&cell.account);
                fight_start_tick = Some(setup_tick);
            }
            Err(error) => {
                failure = Some((
                    "product",
                    format!("api.combat.fight script did not start: {error}"),
                ));
            }
        }
    }

    let deadline = Instant::now() + COMBAT_CELL_WALL_LIMIT;
    let mut last_tick = None;
    while failure.is_none() && !passed {
        if Instant::now() >= deadline {
            failure = Some((
                "fixture",
                format!("wall watchdog expired after {COMBAT_CELL_WALL_LIMIT:?}"),
            ));
            break;
        }
        let status = cell
            .play
            .statuses()
            .into_iter()
            .find(|status| status.username == cell.account);
        let Some(status) = status else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let Some(frame) = sample_frame(&cell, status.condition, None) else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let tick = frame.tick;
        let actions = capture_sample(&cell.capture, frame, &mut last_tick, &mut frames);
        if fruit_delay && policy_tick.is_none() && !eat_actions(&actions).is_empty() {
            if let Some(arm) = cell.play.arm(&cell.account) {
                arm.set_cure_poison(vault::CurePoison::WhenNeeded);
                policy_tick = Some(tick);
            } else {
                failure = Some((
                    "fixture",
                    "live arm disappeared before enabling cure".into(),
                ));
            }
        }
        let cow_visible = cell.live.lock().snapshot.npcs().iter().any(|npc| {
            npc.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("cow"))
                && (3240..=3266).contains(&npc.tile.x)
                && (3254..=3299).contains(&npc.tile.z)
        });
        let attacks = attack_actions(&actions);
        if !cow_visible && seed_tick.is_none() && tick.saturating_sub(setup_tick) > 15 {
            failure = Some((
                "fixture",
                "no real Cow appeared in the selected field".to_owned(),
            ));
        }
        if seed_tick.is_none()
            && !seed_scheduled
            && tick.saturating_sub(setup_tick) >= POISON_PREFIX_TICKS
            && attacks
                .iter()
                .any(|attack| attack.accepted && attack.real_wire)
            && frames.last().is_some_and(|frame| frame.protect_active)
        {
            cell.live.lock().action = Some(LiveAction::Poison);
            seed_scheduled = true;
        }
        if let Some(sent) = cell.live.lock().sent.take() {
            poison_send = Some(sent);
            if sent.accepted {
                seed_tick = Some(sent.tick);
            } else {
                failure = Some((
                    "fixture",
                    format!("~poison 27 was refused at tick {}", sent.tick),
                ));
            }
        }
        if seed_tick.is_none()
            && fight_start_tick.is_some_and(|started| tick.saturating_sub(started) > 40)
            && !attacks
                .iter()
                .any(|attack| attack.accepted && attack.real_wire)
        {
            failure = Some(("product", "api.combat.fight emitted no accepted wire-decoded Cow attack within 40 observed ticks".to_owned()));
        }
        if let Some(seed) = seed_tick {
            if tick.saturating_sub(seed) > CELL_TICK_LIMIT {
                failure = Some((
                    "product",
                    format!("100 observed-tick oracle bound expired at tick {tick}"),
                ));
            }
        }
        let eats = eat_actions(&actions);
        let decision = seed_tick.map_or(OracleDecision::Pending, |seed| {
            match evaluate_combat(seed, &frames, &eats, &attacks) {
                OracleDecision::Pass if fruit_delay => {
                    evaluate_fruit(&frames, &actions, policy_tick)
                }
                decision => decision,
            }
        });
        match decision {
            OracleDecision::Pending => {}
            OracleDecision::Pass => passed = true,
            OracleDecision::Fail(reason) => {
                let class = if reason.starts_with("interference:") {
                    "interference"
                } else if reason.starts_with("fixture:") {
                    "fixture"
                } else if reason.starts_with("oracle:") {
                    "oracle"
                } else {
                    "product"
                };
                failure = Some((class, reason));
            }
        }
        if failure.is_none() && !passed && status.condition.hp == 0 {
            failure = Some(("product", format!("account died at observed tick {tick}")));
        }
        if failure.is_none() && !passed {
            thread::sleep(Duration::from_millis(50));
        }
    }
    let outcome = script_slot_probe(
        &cell.play,
        &cell.account,
        "globalThis.__eatFightOutcome ?? null",
    );
    cell.play.script_stop(&cell.account);
    let actions = captured_actions(&cell.capture);
    let receipt = json!({
        "cell": label,
        "api_path": "api.combat.fight",
        "fight_source": FIGHT_CONSUMER,
        "real_content": true,
        "target": "Cow",
        "target_spawned": false,
        "account": cell.account,
        "map_members": true,
        "protect_from_melee_varp": PROTECT_FROM_MELEE_VARP,
        "cure_poison": if fruit_delay { "never, then when-needed after the bite" } else { "never" },
        "policy_change_tick": policy_tick,
        "setup_tick": setup_tick,
        "fight_start_tick": fight_start_tick,
        "poison_seed_tick": seed_tick,
        "poison_seed_sent": poison_send.map(|sent| json!({"tick": sent.tick, "accepted": sent.accepted})),
        "fight_outcome": outcome,
        "outcome": if failure.is_none() { "PASS" } else { "FAIL" },
        "classification": failure.as_ref().map(|(class, _)| *class),
        "failure": failure.as_ref().map(|(_, reason)| reason),
        "eat_actions": eat_actions(&actions).iter().map(|eat| json!({
            "tick": eat.tick.0,
            "snapshot_tick": eat.tick.0,
            "hp_before": eat.hp_before,
            "hp_max": eat.hp_max,
            "lobsters_before": eat.lobsters_before,
            "accepted": eat.accepted,
            "source": eat.source,
            "owner_run": eat.owner_run,
            "fight_action": eat.fight_action,
            "real_wire": eat.real_wire,
        })).collect::<Vec<_>>(),
        "attack_actions": attack_actions(&actions).iter().map(|attack| json!({
            "tick": attack.tick.0,
            "snapshot_tick": attack.tick.0,
            "accepted": attack.accepted,
            "owner_run": attack.owner_run,
            "fight_action": attack.fight_action,
            "real_wire": attack.real_wire,
        })).collect::<Vec<_>>(),
        "frames": frames.iter().map(ObservedFrame::evidence).collect::<Vec<_>>(),
        "host_capture": captured_evidence(&cell.capture),
    });
    let receipt_path = write_receipt(&cell.evidence, &receipt);
    assert!(
        passed,
        "{label} failed: {}; receipt={}",
        failure.map_or_else(|| "no passing receipt".to_owned(), |(_, reason)| reason),
        receipt_path.display()
    );
}

fn script_slot_probe(play: &super::Play, account: &str, expression: &str) -> Value {
    let Some(slot) = super::script_slot(&play.scripts, account) else {
        return Value::Null;
    };
    let Ok(slot) = slot.lock() else {
        return Value::Null;
    };
    slot.probe(expression).unwrap_or(Value::Null)
}

#[test]
#[ignore = "requires LIVE=1, Engine A at 600 ms, and the accepted R289 cache/nav fixtures"]
fn live_survival_eat_poison_line_native_walk() {
    run_walk_case(EatCase::PoisonLine, false);
}

#[test]
#[ignore = "requires LIVE=1, Engine A at 600 ms, and the accepted R289 cache/nav fixtures"]
fn live_survival_eat_full_hp_native_walk() {
    run_walk_case(EatCase::FullHp, false);
}

#[test]
#[ignore = "requires LIVE=1, Engine A at 600 ms, and the accepted R289 cache/nav fixtures"]
fn live_survival_strange_fruit_after_bite() {
    run_walk_case(EatCase::FullHp, true);
}

#[test]
#[ignore = "requires LIVE=1, Engine A at 600 ms, and the accepted R289 cache/nav fixtures"]
fn live_survival_eat_poison_through_api_combat_fight() {
    run_combat_cell(false);
}

#[test]
#[ignore = "requires LIVE=1, Engine A at 600 ms, and the accepted R289 cache/nav fixtures"]
fn live_survival_strange_fruit_through_api_combat_fight() {
    run_combat_cell(true);
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    fn fruit_receipt(click_tick: u32, cleared: bool) -> (Vec<ObservedFrame>, Vec<Value>) {
        let mut frames = vec![
            mark_frame(82, 4, 10, 5, 82),
            mark_frame(84, 10, 10, 4, 82),
            mark_frame(86, 10, 10, 4, 82),
            mark_frame(89, 10, 10, 4, 82),
        ];
        for frame in &mut frames {
            frame.route_active = true;
        }
        if cleared {
            frames[2].poison = PoisonState::Clear;
            frames[2].evidence =
                json!({"snapshot_facts": {"inventory": [{"id": LOBSTER_ID, "count": 4}]}});
        }
        let actions = vec![
            json!({
                "kind": "guard", "op": "eat", "item": "Lobster",
                "snapshot_tick": 82, "accepted": true,
                "snapshot": {
                    "stats": [{"index": 3, "effective": 4, "base": 10}],
                    "inventory": [{"id": LOBSTER_ID, "count": 5}],
                },
            }),
            json!({
                "kind": "guard", "op": "eat", "item": "Strange fruit",
                "snapshot_tick": click_tick, "accepted": true,
            }),
        ];
        (frames, actions)
    }

    #[test]
    fn fruit_oracle_accepts_first_allowed_tick_and_observed_cure() {
        let (frames, actions) = fruit_receipt(85, true);
        assert_eq!(
            evaluate_fruit(&frames, &actions, Some(82)),
            OracleDecision::Pass
        );
    }

    #[test]
    fn fruit_oracle_requires_real_combat_actions_from_the_same_fight() {
        let (mut frames, mut actions) = fruit_receipt(85, true);
        for frame in &mut frames {
            frame.route_active = false;
        }
        for (action, name) in actions.iter_mut().zip(["Lobster", "Strange fruit"]) {
            action["kind"] = json!("interaction");
            action["request"] = json!({"op": "held", "name": name, "action": "Eat"});
            action["run"] = json!("run-1");
            action["action_id"] = json!(7);
            action["wire_decoded"] = json!(true);
            action["wire_opcodes"] = json!([76]);
        }
        assert_eq!(
            evaluate_fruit(&frames, &actions, Some(82)),
            OracleDecision::Pass
        );
        actions[1]["action_id"] = json!(8);
        assert!(matches!(evaluate_fruit(&frames, &actions, Some(82)),
            OracleDecision::Fail(reason) if reason.starts_with("oracle:")));
        actions[1]["action_id"] = json!(7);
        actions[1]["wire_decoded"] = json!(false);
        assert!(matches!(evaluate_fruit(&frames, &actions, Some(82)),
            OracleDecision::Fail(reason) if reason.starts_with("oracle:")));
    }

    #[test]
    fn fruit_oracle_rejects_early_or_late_clicks_and_missing_cure() {
        for tick in [83, 84, 86] {
            let (frames, actions) = fruit_receipt(tick, true);
            assert!(matches!(evaluate_fruit(&frames, &actions, Some(82)),
                OracleDecision::Fail(reason) if reason.contains("first content-allowed")));
        }
        let (frames, actions) = fruit_receipt(85, false);
        assert!(matches!(evaluate_fruit(&frames, &actions, Some(82)),
            OracleDecision::Fail(reason) if reason.contains("did not cure")));
    }

    #[test]
    fn fruit_sample_reads_actions_that_arrive_on_a_repeated_tick() {
        let capture = Arc::new(StdMutex::new(CombatCapture::default()));
        let mut last_tick = None;
        let mut frames = Vec::new();
        assert!(capture_sample(
            &capture,
            mark_frame(82, 4, 10, 5, 82),
            &mut last_tick,
            &mut frames
        )
        .is_empty());
        let (_, actions) = fruit_receipt(85, true);
        capture.lock().unwrap().actions = actions;
        let observed = capture_sample(
            &capture,
            mark_frame(82, 4, 10, 5, 82),
            &mut last_tick,
            &mut frames,
        );
        assert_eq!(
            eat_actions(&observed).len(),
            1,
            "a bite captured after the first sample must be read in the same tick"
        );
        assert_eq!(
            frames.len(),
            1,
            "repeated ticks do not duplicate frame rows"
        );
    }

    #[test]
    fn fruit_oracle_records_late_policy_as_fixture_failure_before_assertion() {
        let (frames, actions) = fruit_receipt(85, true);
        let decision = evaluate_fruit(&frames, &actions, Some(83));
        let evidence =
            std::env::temp_dir().join(format!("274bot-fruit-late-policy-{}", std::process::id()));
        let receipt = json!({
            "outcome": "FAIL",
            "classification": "fixture",
            "failure": match &decision {
                OracleDecision::Fail(reason) => reason.as_str(),
                _ => "missing classified failure",
            },
            "policy_change_tick": 83,
        });
        let path = write_receipt(&evidence, &receipt);
        let recorded: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        std::fs::remove_dir_all(&evidence).unwrap();
        assert!(matches!(decision,
            OracleDecision::Fail(reason) if reason.starts_with("fixture:")));
        assert_eq!(recorded["classification"], "fixture");
        assert_eq!(recorded["policy_change_tick"], 83);
        assert!(recorded["failure"]
            .as_str()
            .unwrap()
            .contains("immediately"));
        assert!(recorded["worktree_dirty"].is_boolean());
    }

    #[test]
    fn combat_oracle_rejects_an_attack_from_another_fight_in_the_same_run() {
        let mut actions = combat_capture_with_clocks(89, 82);
        actions[0]["action_id"] = json!(6);
        let frames = vec![
            onset_frame(51, 10, 10),
            mark_frame(82, 4, 10, 5, 82),
            mark_frame(84, 10, 10, 4, 82),
        ];
        assert!(matches!(
            evaluate_combat(50, &frames, &eat_actions(&actions), &attack_actions(&actions)),
            OracleDecision::Fail(reason) if reason.contains("Eat fight action"),
        ));
    }

    #[test]
    fn oracle_action_condition_clock_checks_exact_windows_without_offsets() {
        for (action, condition, first, last, expected) in [
            (81, 82, 0, 3, false),
            (82, 82, 0, 3, true),
            (85, 82, 0, 3, true),
            (86, 82, 0, 3, false),
            (82, 82, -3, 0, true),
            (82, 85, -3, 0, true),
            (82, 86, -3, 0, false),
            (82, 86, i64::MIN, -4, true),
            (0, u32::MAX, i64::MIN, -1, true),
            (u32::MAX, 0, 0, i64::MAX, true),
        ] {
            assert_eq!(
                action_at_condition_offset(
                    ActionSnapshotTick(Some(action)),
                    condition,
                    first..=last,
                ),
                expected,
                "action={action}, condition={condition}, window={first}..={last}",
            );
        }
        let missing = ActionSnapshotTick::from_capture(&json!({"tick": 82, "host_tick": 82}));
        assert_eq!(missing, ActionSnapshotTick(None));
        assert!(!action_at_condition_offset(
            missing,
            82,
            i64::MIN..=i64::MAX
        ));
    }

    fn combat_capture_with_clocks(host_tick: u32, snapshot_tick: u32) -> Vec<Value> {
        vec![
            json!({
                "kind": "interaction",
                "request": {"op": "npc", "name": "Cow", "action": "Attack"},
                "tick": 9000,
                "host_tick": 9000,
                "snapshot_tick": 50,
                "accepted": true,
                "run": "run-1",
                "action_id": 7,
                "wire_decoded": true,
                "wire_opcodes": [72],
            }),
            json!({
                "kind": "interaction",
                "request": {"op": "held", "name": "Lobster", "action": "Eat"},
                "tick": host_tick,
                "host_tick": host_tick,
                "snapshot_tick": snapshot_tick,
                "accepted": true,
                "run": "run-1",
                "action_id": 7,
                "wire_decoded": true,
                "wire_opcodes": [76],
                "snapshot": {
                    "stats": [{"index": 3, "base": 10, "effective": 4}],
                    "inventory": [{"id": LOBSTER_ID, "count": 5}],
                },
            }),
        ]
    }

    #[test]
    fn combat_oracle_uses_explicit_snapshot_clock_when_host_clocks_differ() {
        let actions = combat_capture_with_clocks(89, 82);
        let eats = eat_actions(&actions);
        let attacks = attack_actions(&actions);
        assert_eq!(eats[0].tick, ActionSnapshotTick(Some(82)));
        assert_eq!(attacks[0].tick, ActionSnapshotTick(Some(50)));
        let frames = vec![
            onset_frame(51, 10, 10),
            mark_frame(82, 4, 10, 5, 82),
            mark_frame(84, 10, 10, 4, 82),
        ];
        assert_eq!(
            evaluate_combat(50, &frames, &eats, &attacks),
            OracleDecision::Pass,
        );
    }

    #[test]
    fn combat_oracle_rejects_a_truly_late_snapshot_despite_a_timely_host_clock() {
        let actions = combat_capture_with_clocks(82, 86);
        let frames = vec![
            onset_frame(51, 10, 10),
            mark_frame(82, 4, 10, 5, 82),
            mark_frame(86, 4, 10, 5, 82),
            mark_frame(88, 10, 10, 4, 82),
        ];
        assert!(matches!(
            evaluate_combat(50, &frames, &eat_actions(&actions), &attack_actions(&actions)),
            OracleDecision::Fail(reason) if reason.contains("within three ticks"),
        ));
    }

    #[test]
    fn survival_oracles_reject_untimed_actions_instead_of_using_the_host_clock() {
        let mut actions = combat_capture_with_clocks(82, 82);
        for action in &mut actions {
            action.as_object_mut().unwrap().remove("snapshot_tick");
        }
        let eats = eat_actions(&actions);
        let attacks = attack_actions(&actions);
        assert_eq!(eats.len(), 1);
        assert_eq!(attacks.len(), 1);
        let frames = vec![
            onset_frame(51, 10, 10),
            mark_frame(82, 4, 10, 5, 82),
            mark_frame(84, 10, 10, 4, 82),
        ];
        for decision in [
            evaluate_walk(EatCase::PoisonLine, 50, &frames, &eats),
            evaluate_walk(EatCase::FullHp, 50, &frames, &eats),
            evaluate_combat(50, &frames, &eats, &attacks),
        ] {
            assert!(matches!(
                decision,
                OracleDecision::Fail(reason) if reason.contains("no explicit snapshot_tick"),
            ));
        }
    }

    // Historical receipts contain PoisonState's exact Debug representation.
    // Parse that representation strictly; never infer poison from HP or timing.
    fn saved_poison_state(text: &str) -> Option<PoisonState> {
        let state = if text == "Clear" {
            PoisonState::Clear
        } else if let Some(fields) = text
            .strip_prefix("Unknown { since: ")
            .and_then(|fields| fields.strip_suffix(" }"))
        {
            PoisonState::Unknown {
                since: fields.parse().ok()?,
            }
        } else if let Some(fields) = text
            .strip_prefix("Onset { since: ")
            .and_then(|fields| fields.strip_suffix(" }"))
        {
            let (since, fields) = fields.split_once(", q: ")?;
            let (q, hit) = fields.split_once(", hit_since_line: ")?;
            PoisonState::Onset {
                since: since.parse().ok()?,
                q: if q == "None" {
                    None
                } else {
                    Some(q.strip_prefix("Some(")?.strip_suffix(')')?.parse().ok()?)
                },
                hit_since_line: hit.parse().ok()?,
            }
        } else if let Some(fields) = text
            .strip_prefix("Poisoned { per_tick: ")
            .and_then(|fields| fields.strip_suffix(" }"))
        {
            let (per_tick, fields) = fields.split_once(", last_mark: ")?;
            let (last_mark, hit) = fields.split_once(", hit_since_mark: ")?;
            PoisonState::Poisoned {
                per_tick: per_tick.parse().ok()?,
                last_mark: last_mark.parse().ok()?,
                hit_since_mark: hit.parse().ok()?,
            }
        } else {
            PoisonState::Immune {
                until_lower: text
                    .strip_prefix("Immune { until_lower: ")?
                    .strip_suffix(" }")?
                    .parse()
                    .ok()?,
            }
        };
        (format!("{state:?}") == text).then_some(state)
    }

    fn saved_frame(row: &Value) -> ObservedFrame {
        let tick = u32::try_from(row["tick"].as_u64().expect("saved frame tick"))
            .expect("saved frame u32 tick");
        let hp =
            i32::try_from(row["hp"].as_i64().expect("saved frame HP")).expect("saved frame i32 HP");
        let hp_max = i32::try_from(row["hp_max"].as_i64().expect("saved frame max HP"))
            .expect("saved frame i32 max HP");
        let poison_text = row["poison"].as_str().expect("saved poison text");
        let evidence = &row["snapshot"];
        assert_eq!(evidence["condition"]["tick"], json!(tick));
        assert_eq!(evidence["snapshot_facts"]["snapshot_tick"], json!(tick));
        assert_eq!(evidence["condition"]["hp"], json!(hp));
        assert_eq!(evidence["condition"]["hp_max"], json!(hp_max));
        assert_eq!(evidence["condition"]["poison"], json!(poison_text));
        ObservedFrame {
            tick,
            hp,
            hp_max,
            lobsters: i32::try_from(row["lobsters"].as_i64().expect("saved lobster count"))
                .expect("saved i32 lobster count"),
            poison: saved_poison_state(poison_text).expect("exact saved poison representation"),
            poison_line: row["poison_line"].as_bool().expect("saved poison line"),
            route_active: row["route_active"].as_bool().expect("saved route state"),
            protect_active: row["protect_from_melee"]
                .as_bool()
                .expect("saved protection state"),
            non_poison_damage: row["non_poison_damage"]
                .as_bool()
                .expect("saved damage attribution"),
            evidence: evidence.clone(),
        }
    }

    #[test]
    fn oracle_saved_poison_reader_accepts_exact_states_and_rejects_other_formats() {
        for poison in [
            PoisonState::Unknown { since: 0 },
            PoisonState::Clear,
            PoisonState::Onset {
                since: 32,
                q: Some(11),
                hit_since_line: false,
            },
            PoisonState::Onset {
                since: 32,
                q: None,
                hit_since_line: true,
            },
            PoisonState::Poisoned {
                per_tick: 6,
                last_mark: 93,
                hit_since_mark: false,
            },
            PoisonState::Immune { until_lower: 100 },
        ] {
            let text = format!("{poison:?}");
            assert_eq!(saved_poison_state(&text), Some(poison));
            assert_eq!(saved_poison_state(&(text + " trailing")), None);
        }
        for text in [
            "",
            "Poisoned { per_tick: 6, last_mark: 93 }",
            "Onset { since: 32, q: Some(11), hit_since_line: maybe }",
            "Unknown { since: 65536 }",
        ] {
            assert_eq!(saved_poison_state(text), None);
        }
    }

    #[test]
    #[ignore = "saved receipt replay; set SURVIVAL_REJUDGE_RECEIPTS to a JSON array of paths"]
    fn oracle_rejudges_saved_survival_walk_receipts() {
        let paths: Vec<PathBuf> = serde_json::from_str(
            &std::env::var("SURVIVAL_REJUDGE_RECEIPTS").expect("SURVIVAL_REJUDGE_RECEIPTS"),
        )
        .expect("saved receipt paths");
        assert!(!paths.is_empty(), "at least one saved receipt");
        for path in paths {
            assert!(path.is_absolute(), "absolute receipt path");
            let receipt: Value =
                serde_json::from_slice(&std::fs::read(&path).expect("read saved receipt"))
                    .expect("parse saved receipt");
            let case = match receipt["cell"].as_str().expect("saved cell name") {
                "L-EAT-POISON-LINE-WALK" => EatCase::PoisonLine,
                "L-EAT-FULL-HP-WALK" => EatCase::FullHp,
                other => panic!("not a survival walk receipt: {other}"),
            };
            let seed = u32::try_from(
                receipt["poison_seed_tick"]
                    .as_u64()
                    .expect("saved poison seed tick"),
            )
            .expect("saved u32 poison seed tick");
            let frames = receipt["frames"]
                .as_array()
                .expect("saved observed frames")
                .iter()
                .map(saved_frame)
                .collect::<Vec<_>>();
            let eats = eat_actions(
                receipt["raw_actions"]
                    .as_array()
                    .expect("saved raw actions"),
            );
            let decision = evaluate_walk(case, seed, &frames, &eats);
            println!(
                "survival-rejudged={}",
                json!({
                    "receipt": path,
                    "cell": case.name(),
                    "oracle_decision": format!("{decision:?}"),
                    "action_clock": "explicit snapshot_tick",
                    "eat_snapshot_ticks": eats.iter().map(|eat| eat.tick.0).collect::<Vec<_>>(),
                    "observed_frames": frames.len(),
                }),
            );
            assert_eq!(decision, OracleDecision::Pass, "receipt={}", path.display());
        }
    }

    #[test]
    fn oracle_seed_waits_for_login_prefix_and_active_route_progress() {
        assert!(walk_poison_seed_ready(Some(0), 31, true, true));
        assert!(walk_poison_seed_ready(Some(100), 131, true, true));
        assert!(!walk_poison_seed_ready(Some(0), 30, true, true));
        assert!(!walk_poison_seed_ready(None, 31, true, true));
        assert!(!walk_poison_seed_ready(Some(0), 31, false, true));
        assert!(!walk_poison_seed_ready(Some(0), 31, true, false));
    }

    fn frame(
        tick: u32,
        hp: i32,
        hp_max: i32,
        lobsters: i32,
        poison: PoisonState,
        route_active: bool,
        protect_active: bool,
    ) -> ObservedFrame {
        ObservedFrame {
            tick,
            hp,
            hp_max,
            lobsters,
            poison,
            poison_line: matches!(poison, PoisonState::Onset { .. }),
            route_active,
            protect_active,
            non_poison_damage: false,
            evidence: Value::Null,
        }
    }

    fn onset_frame(tick: u32, hp: i32, max: i32) -> ObservedFrame {
        frame(
            tick,
            hp,
            max,
            5,
            PoisonState::Onset {
                since: tick as u16,
                q: Some(11),
                hit_since_line: false,
            },
            true,
            true,
        )
    }

    fn mark_frame(tick: u32, hp: i32, max: i32, lobsters: i32, last_mark: u16) -> ObservedFrame {
        frame(
            tick,
            hp,
            max,
            lobsters,
            PoisonState::Poisoned {
                per_tick: 6,
                last_mark,
                hit_since_mark: false,
            },
            true,
            true,
        )
    }

    fn eat(
        tick: u32,
        hp: i32,
        max: i32,
        lobsters: i32,
        source: &str,
        run: Option<&str>,
        wire: bool,
    ) -> EatAction {
        EatAction {
            tick: ActionSnapshotTick(Some(tick)),
            hp_before: hp,
            hp_max: max,
            lobsters_before: lobsters,
            accepted: true,
            source: source.to_owned(),
            owner_run: run.map(str::to_owned),
            fight_action: run.map(|_| 7),
            real_wire: wire,
        }
    }

    fn poison_line_receipt() -> (Vec<ObservedFrame>, Vec<EatAction>) {
        (
            vec![
                onset_frame(100, 15, 15),
                mark_frame(130, 9, 15, 5, 130),
                mark_frame(160, 3, 15, 5, 160),
                mark_frame(162, 15, 15, 4, 160),
            ],
            vec![eat(161, 3, 15, 5, "guard", None, false)],
        )
    }

    fn full_hp_receipt() -> (Vec<ObservedFrame>, Vec<EatAction>) {
        (
            vec![
                onset_frame(100, 10, 10),
                mark_frame(130, 4, 10, 5, 130),
                mark_frame(132, 10, 10, 4, 130),
                mark_frame(160, 4, 10, 4, 160),
                mark_frame(162, 10, 10, 3, 160),
            ],
            vec![
                eat(131, 4, 10, 5, "guard", None, false),
                eat(161, 4, 10, 4, "guard", None, false),
            ],
        )
    }

    #[test]
    fn poison_line_oracle_passes_and_rejects_a_bite_before_the_second_mark() {
        let (frames, eats) = poison_line_receipt();
        assert_eq!(
            evaluate_walk(EatCase::PoisonLine, 69, &frames, &eats),
            OracleDecision::Pass
        );
        let early = vec![eat(131, 9, 15, 5, "guard", None, false), eats[0].clone()];
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &frames, &early),
            OracleDecision::Fail(reason) if reason.contains("before the second poison mark")
        ));
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &frames[..2], &[]),
            OracleDecision::Pending
        ));
    }

    #[test]
    fn poison_line_oracle_rejects_late_duplicate_unhealed_dead_and_inactive_receipts() {
        let (frames, eats) = poison_line_receipt();
        let mut late = frames.clone();
        late.push(mark_frame(164, 3, 15, 5, 160));
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &late, &[]),
            OracleDecision::Fail(reason) if reason.contains("no bite within three")
        ));
        let duplicate = vec![eats[0].clone(), eat(162, 3, 15, 5, "guard", None, false)];
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &frames, &duplicate),
            OracleDecision::Fail(reason) if reason.contains("multiple bites")
        ));
        for (hp, food) in [(14, 4), (15, 5)] {
            let mut bad = frames.clone();
            bad[3].hp = hp;
            bad[3].lobsters = food;
            bad.push(mark_frame(165, hp, 15, food, 160));
            assert!(matches!(
                evaluate_walk(EatCase::PoisonLine, 69, &bad, &eats),
                OracleDecision::Fail(reason) if reason.contains("heal by 12 HP")
            ));
        }
        let mut dead = frames.clone();
        dead[2].hp = 0;
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &dead, &eats),
            OracleDecision::Fail(reason) if reason.contains("died")
        ));
        let mut inactive = frames;
        inactive[1].route_active = false;
        assert!(matches!(
            evaluate_walk(EatCase::PoisonLine, 69, &inactive, &eats),
            OracleDecision::Fail(reason) if reason.contains("route guard was not active")
        ));
    }

    #[test]
    fn full_hp_oracle_rejects_late_missing_heal_and_death() {
        let (frames, eats) = full_hp_receipt();
        let mut late = frames[..2].to_vec();
        late.push(mark_frame(134, 4, 10, 5, 130));
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &late, &[]),
            OracleDecision::Fail(reason) if reason.contains("no bite within three")
        ));
        let mut unhealed = frames[..2].to_vec();
        unhealed.push(mark_frame(135, 4, 10, 4, 130));
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &unhealed, &eats[..1]),
            OracleDecision::Fail(reason) if reason.contains("did not heal to full")
        ));
        let mut dead = frames;
        dead[1].hp = 0;
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &dead, &eats),
            OracleDecision::Fail(reason) if reason.contains("died")
        ));
    }

    #[test]
    fn full_hp_oracle_passes_and_rejects_early_or_duplicate_bites() {
        let (frames, eats) = full_hp_receipt();
        assert_eq!(
            evaluate_walk(EatCase::FullHp, 69, &frames, &eats),
            OracleDecision::Pass
        );
        let early = vec![
            eat(110, 10, 10, 5, "guard", None, false),
            eats[0].clone(),
            eats[1].clone(),
        ];
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &frames, &early),
            OracleDecision::Fail(reason) if reason.contains("before the first poison mark")
        ));
        let duplicate = vec![
            eats[0].clone(),
            eat(133, 10, 10, 4, "guard", None, false),
            eats[1].clone(),
        ];
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &frames, &duplicate),
            OracleDecision::Fail(_)
        ));
        assert!(matches!(
            evaluate_walk(EatCase::FullHp, 69, &frames[..3], &eats[..1]),
            OracleDecision::Pending
        ));
    }

    #[test]
    fn combat_oracle_requires_real_attack_and_food_packets_from_one_fight() {
        let frames = vec![
            onset_frame(100, 10, 10),
            mark_frame(130, 4, 10, 5, 130),
            mark_frame(132, 10, 10, 4, 130),
        ];
        let eats = vec![eat(131, 4, 10, 5, "interaction", Some("run-1"), true)];
        let attacks = vec![AttackAction {
            tick: ActionSnapshotTick(Some(95)),
            accepted: true,
            owner_run: Some("run-1".to_owned()),
            fight_action: Some(7),
            real_wire: true,
        }];
        assert_eq!(
            evaluate_combat(99, &frames, &eats, &attacks),
            OracleDecision::Pass
        );
        assert!(matches!(
            evaluate_combat(99, &frames, &eats, &[]),
            OracleDecision::Fail(reason) if reason.contains("Eat fight action")
        ));
        let synthetic_eat = vec![eat(131, 4, 10, 5, "interaction", Some("run-1"), false)];
        assert!(matches!(
            evaluate_combat(99, &frames, &synthetic_eat, &attacks),
            OracleDecision::Fail(reason) if reason.contains("accepted real Lobster action")
        ));
    }

    #[test]
    fn combat_oracle_rejects_early_late_duplicate_bites_and_death() {
        let frames = vec![
            onset_frame(100, 10, 10),
            mark_frame(130, 4, 10, 5, 130),
            mark_frame(132, 10, 10, 4, 130),
        ];
        let eat = eat(131, 4, 10, 5, "interaction", Some("run-1"), true);
        let attacks = vec![AttackAction {
            tick: ActionSnapshotTick(Some(95)),
            accepted: true,
            owner_run: Some("run-1".to_owned()),
            fight_action: Some(7),
            real_wire: true,
        }];
        let mut early = eat.clone();
        early.tick = ActionSnapshotTick(Some(110));
        assert!(matches!(
            evaluate_combat(99, &frames, &[early], &attacks),
            OracleDecision::Fail(reason) if reason.contains("before the first poison mark")
        ));
        let mut late = frames[..2].to_vec();
        late.push(mark_frame(134, 4, 10, 5, 130));
        assert!(matches!(
            evaluate_combat(99, &late, &[], &attacks),
            OracleDecision::Fail(reason) if reason.contains("within three ticks")
        ));
        assert!(matches!(
            evaluate_combat(99, &frames, &[eat.clone(), eat.clone()], &attacks),
            OracleDecision::Fail(reason) if reason.contains("multiple combat bites")
        ));
        let mut dead = frames;
        dead[1].hp = 0;
        assert!(matches!(
            evaluate_combat(99, &dead, &[eat], &attacks),
            OracleDecision::Fail(reason) if reason.contains("died")
        ));
    }
}

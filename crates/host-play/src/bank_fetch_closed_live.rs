//! L-FETCH-CLOSED (design-bank-snapshot §7 S5): a manual WalkTo with
//! BankBudget on plans its bank trip from the account's bank memory while
//! the bank is closed, through real Play at the real Draynor booth.
//!
//! The design names Varrock → Al Kharid; in the R289 pack that route is a
//! free 240-tile walk north of the toll gate, so it needs no coins and no
//! session. The cell uses the Port Sarim → Karamja ship instead: a mandatory
//! 30-coin fare, the nearest bank (Draynor) a real trip away.
//!
//! Three cells, each its own process (M-R2-3: the stale-negative hint must be
//! loaded by a new process; an in-process relog never re-reads the file):
//!
//! 1. `live_fetch_closed_session_port_sarim_to_karamja`: a fresh account,
//!    pack cleared, 100 coins (and 5 bones) seeded in the bank; the native
//!    Select/Open/Close machines open the Draynor booth once (memory
//!    `Session`), the account is `::tele`d to the Port Sarim docks with the
//!    bank shut, and `arm_walk_on` to Karamja with `allow_bank_fetch` arms a
//!    BankBudget session without opening the bank first. The WalkArm pump
//!    (the panel/TUI path) runs the session: it withdraws exactly the
//!    30-coin fare, closes, boards and arrives. The account logs out and its
//!    hint file (70 coins, 5 bones) is checked.
//! 2. `live_fetch_closed_stale_negative_hint`: the same account
//!    (`S5_LIVE_ACCOUNT`) and the same `HOME` (run both under one
//!    `ISOHOME_DIR`). Before Play starts, the coins row is deleted from the
//!    hint file. The new process loads the file as `Hint` (bones only); the
//!    same WalkTo still arms a session, the open bank serves the 30 coins,
//!    and the walk completes: one trip, no `NoPath`.
//! 3. `live_fetch_closed_stale_solid_radius`: the same account and `HOME`
//!    after cell 2, the coins row again deleted before Play starts. A Load
//!    script (`apiVersion = 2`) sends one isolate `walk-near` with
//!    `allow_bank_fetch` and radius 1 to a solid tile on Musa Point, so the
//!    host's solid-target goal search (`calculate_solid`) plans from the
//!    coin-less `Hint` (REVIEW-BANK-SNAPSHOT-S5 H1). The slot's own script
//!    walk pump runs the one verifying trip (30 coins of the 40 banked) and
//!    the walk ends beside the target.
//!
//! Evidence (JSON receipt and CPU-rendered PNG) lands below
//! `LIVE_EVIDENCE_DIR`. Deadline: 10 minutes after the seed is posted.
//!
//! `ISOHOME_DIR=<dir> LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<p> GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=<evidence-root> isohome cargo test -p host-play --features live-harness,test-support --lib live_fetch_closed_session -- --ignored --nocapture --test-threads=1`, then the same with `S5_LIVE_ACCOUNT=<account>` and `live_fetch_closed_stale_negative`, then `live_fetch_closed_stale_solid_radius`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::Origin;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav::bank_fetch::{BankRows, BankStep};
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::world::NavWorld;
use nav::WorldState;
use parking_lot::Mutex;
use script::bank::{Close, Open, OpenArgs, Select, SelectArgs};
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{
    check_prerequisites, count, frame, live_profile, Cell, Prep, CAPTURE_GRACE, DRAYNOR,
    LOGIN_DEADLINE, PREPARATION_DEADLINE,
};
use super::{
    arm_walk_on, run_with_template, step_walk_arm_follow, tele_args, SharedClientTemplate, WalkArm,
    WalkArms,
};

const COINS: i32 = 995;
const BONES: i32 = 526;
const FARE: i32 = 30;
const PROFILE: &str = "local-289";
const SEED: &[&str] = &["givebank coins 100", "givebank bones 5"];
/// The bank's coins after the Session cell's trip, the stale one, and the
/// stale solid-radius one.
const BANKED_AFTER: [i32; 3] = [70, 40, 10];
const CELL_BOUND: Duration = Duration::from_secs(600);
const SAVE_GRACE: Duration = Duration::from_secs(15);
const LOGOUT_DEADLINE: Duration = Duration::from_secs(120);
/// Port Sarim docks (`nav_fares_live`'s destination): no bank is open here;
/// the nearest is Draynor's.
const PORT_SARIM: WorldTile = WorldTile {
    x: 3028,
    z: 3217,
    level: 0,
};
/// Musa Point on Karamja, beside the Customs officer's dock; the cell walks
/// to the nearest standable tile.
const KARAMJA: WorldTile = WorldTile {
    x: 2954,
    z: 3147,
    level: 0,
};

/// The tiles around `around`, Chebyshev rings out to 8, nearest first.
fn rings(around: WorldTile) -> impl Iterator<Item = WorldTile> {
    (0..=8).flat_map(move |ring: i32| {
        (-ring..=ring).flat_map(move |dx| {
            (-ring..=ring)
                .filter(move |dz| dx.abs().max(dz.abs()) == ring)
                .map(move |dz| WorldTile {
                    x: around.x + dx,
                    z: around.z + dz,
                    level: around.level,
                })
        })
    })
}

/// The standable tile nearest `around`.
fn standable_near(world: &NavWorld, around: WorldTile) -> WorldTile {
    rings(around)
        .find(|&tile| world.collision.standable(tile))
        .expect("a standable tile near Musa Point")
}

/// The solid tile nearest `around` that something can stand beside and
/// reach: a radius walk-near there takes the host's solid-target search.
fn solid_near(world: &NavWorld, around: WorldTile) -> WorldTile {
    let collision = &world.collision;
    rings(around)
        .find(|&tile| {
            !collision.standable(tile)
                && api::query::arrival_stands(
                    tile,
                    |stand| collision.standable(stand),
                    |stand| Some(collision.walkable_word(stand.x, stand.z, stand.level) as i32),
                )
                .next()
                .is_some()
        })
        .expect("a solid tile with an arrival stand near Musa Point")
}

/// The Load script of the solid-radius cell: one isolate `walk-near` with
/// BankBudget on, then it only keeps the slot's script running.
fn solid_walk_script(target: WorldTile) -> String {
    format!(
        r#"export const apiVersion = 2;
export function tick(api) {{
  if (globalThis.__sent) return;
  if (api.snapshot.ingame !== true || api.snapshot.here == null) return;
  globalThis.__sent = true;
  api.request({{
    op: 'walk-near',
    x: {x},
    z: {z},
    level: {level},
    radius: {SOLID_RADIUS},
    allow_teleports: false,
    allow_wilderness: false,
    allow_bank_fetch: true,
  }});
}}
"#,
        x = target.x,
        z = target.z,
        level = target.level,
    )
}

/// The solid-radius cell's walk-near radius.
const SOLID_RADIUS: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Variant {
    /// The memory is `Session` from an earlier open in this process.
    Session,
    /// The memory is the edited hint file, loaded by this new process.
    StaleNegative,
    /// The edited hint again, and a script `walk-near` to a solid tile.
    StaleSolidRadius,
}

impl Variant {
    /// Whether this cell's process loads the edited, coin-less hint.
    fn stale(self) -> bool {
        self != Self::Session
    }
}

/// What the frame pump and the test thread observed, in order.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct FetchTrace {
    variant: Option<Variant>,
    account: String,
    home: String,
    hint_path: String,
    hint_before_edit: Option<String>,
    hint_after_edit: Option<String>,
    open_close_done: bool,
    /// `Play::bank_rows` at arm time.
    arm_origin: Option<String>,
    arm_rows: Option<Vec<(i32, i32)>>,
    arm_from: Option<[i32; 3]>,
    arm_bank_loaded: Option<bool>,
    arm_held_coins: Option<i32>,
    planned_steps: Option<Vec<String>>,
    planned_withdraw: Option<i32>,
    /// The armed route's legs: walk spans and transport hops (the
    /// solid-radius cell: the session's post-session route).
    route_legs: Option<Vec<String>>,
    /// The solid-radius cell's walk-near target and radius.
    solid_target: Option<[i32; 3]>,
    solid_radius: Option<i32>,
    /// Each front step the session moved through, with the frame's facts.
    fronts: Vec<String>,
    /// Pack coins when the session cleared (Close landed).
    held_after_session: Option<i32>,
    finished: bool,
    final_tile: Option<[i32; 3]>,
    held_after_walk: Option<i32>,
    /// `Play::bank_rows` after the walk: the trip's open bank observed.
    after_origin: Option<String>,
    after_rows: Option<Vec<(i32, i32)>>,
    hint_after: Option<String>,
    walk_ms: Option<u128>,
    failure: Option<String>,
}

/// The frame pump's shared state: the panel/TUI WalkArm follow, once per
/// player tick, plus the facts the test thread arms from.
#[derive(Default)]
struct Pump {
    state: Option<WorldState>,
    tile: Option<[i32; 3]>,
    held_coins: i32,
    bank_loaded: bool,
    latch: Option<(u64, (i32, i32, i32))>,
    front: Option<Option<BankStep>>,
    pumping: bool,
}

/// The native Select/Open/Close at the Draynor booth: one earlier open that
/// leaves the memory `Session`.
struct OpenClose {
    facts: Arc<NamedBankFacts>,
    bank_name: Arc<str>,
    cell: Arc<Mutex<Cell>>,
    trace: Arc<Mutex<FetchTrace>>,
    select: Option<ActionHandle<Select>>,
    open: Option<ActionHandle<Open>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
    opened: bool,
}

impl OpenClose {
    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.trace.lock().failure = Some(message.clone());
        self.cell.lock().fail(message.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "fetch-closed-live".into(),
            message: message.into(),
        })
    }
}

impl Script for OpenClose {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.access.is_none() {
            if self.select.is_none() {
                let args = SelectArgs {
                    facts: Arc::clone(&self.facts),
                    from: DRAYNOR,
                    preferences: BankPreferences {
                        use_mage_bank: false,
                        use_zanaris_bank: false,
                    },
                    options: script::native::WalkOptions::default(),
                    explicit: Some(Arc::clone(&self.bank_name)),
                };
                match tick.actions.begin::<Select>(args, &mut tick.cx) {
                    Ok(handle) => self.select = Some(handle),
                    Err(error) => return Ok(self.blocked(format!("Select begin: {error:?}"))),
                }
                return Ok(ScriptFlow::Continue);
            }
            return match tick
                .actions
                .poll(self.select.as_ref().expect("Select handle"), &mut tick.cx)
            {
                Poll::Pending => Ok(ScriptFlow::Continue),
                Poll::Ready(Err(error)) => Ok(self.blocked(format!("Select: {error:?}"))),
                Poll::Ready(Ok(selected)) => {
                    self.select = None;
                    match selected.access {
                        Some(access) => {
                            self.access = Some(access);
                            Ok(ScriptFlow::Continue)
                        }
                        None => Ok(self.blocked("Select returned no stand access")),
                    }
                }
            };
        }
        if !self.opened {
            if self.open.is_none() {
                let access = self.access.clone().expect("selected access");
                match tick
                    .actions
                    .begin::<Open>(OpenArgs { access }, &mut tick.cx)
                {
                    Ok(handle) => self.open = Some(handle),
                    Err(error) => return Ok(self.blocked(format!("Open begin: {error:?}"))),
                }
                return Ok(ScriptFlow::Continue);
            }
            return match tick
                .actions
                .poll(self.open.as_ref().expect("Open handle"), &mut tick.cx)
            {
                Poll::Pending => Ok(ScriptFlow::Continue),
                Poll::Ready(Err(error)) => Ok(self.blocked(format!("Open: {error:?}"))),
                Poll::Ready(Ok(())) => {
                    self.open = None;
                    self.opened = true;
                    Ok(ScriptFlow::Continue)
                }
            };
        }
        if self.close.is_none() {
            match tick.actions.begin::<Close>((), &mut tick.cx) {
                Ok(handle) => self.close = Some(handle),
                Err(error) => return Ok(self.blocked(format!("Close begin: {error:?}"))),
            }
            return Ok(ScriptFlow::Continue);
        }
        match tick
            .actions
            .poll(self.close.as_ref().expect("Close handle"), &mut tick.cx)
        {
            Poll::Pending => Ok(ScriptFlow::Continue),
            Poll::Ready(Err(error)) => Ok(self.blocked(format!("Close: {error:?}"))),
            Poll::Ready(Ok(())) => {
                self.trace.lock().open_close_done = true;
                Ok(ScriptFlow::Complete)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prepare,
    OpenClose,
    Teleport {
        since: Instant,
    },
    Arm {
        settled: Option<Instant>,
    },
    /// The solid-radius cell: the script's walk-near is routing off-pump.
    AwaitPlan {
        since: Instant,
    },
    Walking {
        since: Instant,
    },
    LoggingOut {
        since: Instant,
    },
    AwaitHint {
        since: Instant,
    },
    Done,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn hint_path(account: &str) -> PathBuf {
    script::bot_file("bank-hints")
        .join(PROFILE)
        .join(format!("{account}.json"))
}

fn hint_rows(path: &Path) -> Result<(String, Vec<(i32, i32)>), String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let rows = serde_json::from_value(document["rows"].clone())
        .map_err(|error| format!("rows: {error}: {contents}"))?;
    Ok((contents, rows))
}

/// M-R2-3: delete the coins row from the hint file this process will load.
fn drop_coins_from_hint(path: &Path) -> Result<(String, String), String> {
    let before = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut document: serde_json::Value = serde_json::from_str(&before)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let rows: Vec<(i32, i32)> = serde_json::from_value(document["rows"].clone())
        .map_err(|error| format!("rows: {error}: {before}"))?;
    if !rows.iter().any(|&(id, count)| id == COINS && count >= FARE) {
        return Err(format!(
            "the hint must hold the banked fare before the edit: {before}"
        ));
    }
    let kept: Vec<(i32, i32)> = rows.into_iter().filter(|&(id, _)| id != COINS).collect();
    if kept.is_empty() {
        return Err(format!("the edited hint must keep another row: {before}"));
    }
    document["rows"] = serde_json::json!(kept);
    let after = serde_json::to_string(&document).expect("encode the edited hint");
    std::fs::write(path, &after).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok((before, after))
}

fn status_tile(play: &super::Play, account: &str) -> Option<(bool, [i32; 3])> {
    play.statuses()
        .into_iter()
        .find(|status| status.username == account)
        .map(|status| {
            (
                status.ingame
                    && status.scene_state == 2
                    && !status.welcome_hold
                    && status.main_modal_id == -1,
                [status.tile_x, status.tile_z, status.tile_level],
            )
        })
}

/// A route's legs for the trace: walk spans and transport hops.
fn leg_lines(route: &nav::router::Route) -> Vec<String> {
    route
        .legs
        .iter()
        .map(|leg| match leg {
            nav::router::Leg::Walk { tiles } => format!(
                "walk {} tiles {:?} -> {:?}",
                tiles.len(),
                tiles.first(),
                tiles.last()
            ),
            nav::router::Leg::Transport { edge } => format!(
                "{:?} loc={} at={:?} to={:?} item_req={:?} consumed_req={:?} quest_req={:?}",
                edge.kind,
                edge.loc_id,
                edge.at,
                edge.to,
                edge.item_req,
                edge.consumed_req,
                edge.quest_req
            ),
        })
        .collect()
}

/// The slot's script walk as the host's own pump holds it (the solid-radius
/// cell): the session's remaining steps and post-session route, whether a
/// route is followed, and whether the walk ended failed.
struct ScriptWalk {
    steps: Option<Vec<BankStep>>,
    final_route: Option<Vec<String>>,
    routed: bool,
    failed: bool,
}

fn script_walk(play: &super::Play, account: &str) -> ScriptWalk {
    let navs = play.navs.lock().unwrap();
    let bot = navs.get(account);
    let pending = bot.and_then(|bot| bot.bank_fetch.as_ref());
    ScriptWalk {
        steps: pending.map(|pending| pending.steps.iter().cloned().collect()),
        final_route: pending.map(|pending| leg_lines(&pending.final_route)),
        routed: bot.is_some_and(|bot| bot.route.is_some() || bot.route_worker.is_some()),
        failed: bot.is_some_and(|bot| bot.walk_outcome_failed),
    }
}

/// Whether a planned session is the one fare trip: Walk, Open, a Withdraw
/// (or Withdraw-X amount) of exactly the fare, Close.
fn fare_trip(steps: &[BankStep]) -> (bool, Option<i32>) {
    let withdraw = steps.iter().find_map(|step| match step {
        BankStep::Withdraw { id: COINS, count }
        | BankStep::WithdrawXAmount { id: COINS, count } => Some(*count),
        _ => None,
    });
    let shape = matches!(steps.first(), Some(BankStep::Walk { .. }))
        && steps.get(1) == Some(&BankStep::Open)
        && steps.last() == Some(&BankStep::Close);
    (shape && withdraw == Some(FARE), withdraw)
}

#[allow(clippy::too_many_arguments)] // the frame closure's handles, as the panel's per-frame pump
fn pump_frame(
    client: &mut Client,
    name: &str,
    hold: bool,
    pump: &Mutex<Pump>,
    snapshot: &mut GameSnapshot,
    arms: &WalkArms,
    world: &NavWorld,
    map_members: bool,
    trace: &Mutex<FetchTrace>,
) {
    snapshot.rebuild(client);
    if !(client.ingame && client.scene_state == 2) {
        return;
    }
    let Some(here) = snapshot.tile() else {
        return;
    };
    let mut guard = pump.lock();
    guard.state = Some(WorldState::from_snapshot(snapshot).with_map_members(map_members));
    guard.tile = Some([here.0, here.1, here.2]);
    guard.held_coins = count(snapshot.inventory(), COINS);
    guard.bank_loaded = snapshot.bank_loaded();
    if !guard.pumping || !WalkArm::may_follow(hold) {
        return;
    }
    let key = (client.gens.player, here);
    if guard.latch == Some(key) {
        return;
    }
    guard.latch = Some(key);
    let Some(arm) = arms.lock().unwrap().get(name).cloned() else {
        return;
    };
    let mut arm = arm.lock().unwrap();
    let had_session = arm.bank_fetch.is_some();
    let finished = step_walk_arm_follow(
        client,
        snapshot,
        &mut arm,
        Some(world),
        here,
        map_members,
        Some(name),
    );
    let front = arm
        .bank_fetch
        .as_ref()
        .and_then(|pending| pending.steps.front().cloned());
    if guard.front.as_ref() != Some(&front) {
        guard.front = Some(front.clone());
        trace.lock().fronts.push(format!(
            "{front:?} here={here:?} held_coins={} bank_loaded={}",
            guard.held_coins, guard.bank_loaded
        ));
    }
    if had_session && arm.bank_fetch.is_none() && arm.route.is_some() {
        trace.lock().held_after_session = Some(guard.held_coins);
    }
    if finished {
        guard.pumping = false;
        let mut trace = trace.lock();
        trace.finished = true;
        trace.final_tile = Some([here.0, here.1, here.2]);
    }
}

fn run_cell(variant: Variant) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let home = std::env::var("HOME").expect("HOME is the throwaway isohome directory");
    let account = match variant {
        Variant::Session => super::mint_live_names(1).pop().expect("one live account"),
        Variant::StaleNegative | Variant::StaleSolidRadius => std::env::var("S5_LIVE_ACCOUNT")
            .expect("S5_LIVE_ACCOUNT names the account the earlier cells left"),
    };
    let hint = hint_path(&account);
    assert!(
        hint.starts_with(&home),
        "the hint path {} must sit under HOME {home}",
        hint.display()
    );
    let trace = Arc::new(Mutex::new(FetchTrace {
        variant: Some(variant),
        account: account.clone(),
        home: home.clone(),
        hint_path: hint.display().to_string(),
        ..FetchTrace::default()
    }));
    if variant.stale() {
        let (before, after) = drop_coins_from_hint(&hint).unwrap_or_else(|error| {
            panic!("HARNESS PREREQUISITE FAILURE: the earlier cell's hint: {error}")
        });
        let mut guard = trace.lock();
        guard.hint_before_edit = Some(before);
        guard.hint_after_edit = Some(after);
    }
    let label = match variant {
        Variant::Session => "l-fetch-closed",
        Variant::StaleNegative => "l-fetch-closed-stale",
        Variant::StaleSolidRadius => "l-fetch-closed-solid",
    };
    let evidence_dir = root.join(format!("{label}_{account}_utc-{}Z", unix_now()));
    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence scratch");
    let options = live_profile(&scratch).expect("live profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind the local 289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback engine only"
    );
    assert_eq!(profile.name(), PROFILE, "the hint is keyed by this profile");
    let map_members = profile.map_members();
    let selected = profile.game_data().expect("selected 289 game data");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("selected navigation world");
    if world.named_bank_facts().is_none() {
        world
            .bind_named_bank_facts(&selected)
            .expect("bind named bank facts");
    }
    let facts = Arc::clone(world.named_bank_facts().expect("named bank facts"));
    let karamja = standable_near(&world, KARAMJA);
    let solid = solid_near(&world, KARAMJA);
    if variant == Variant::StaleSolidRadius {
        assert!(
            !world.collision.standable(solid),
            "the walk-near target is solid"
        );
        let mut guard = trace.lock();
        guard.solid_target = Some([solid.x, solid.z, solid.level]);
        guard.solid_radius = Some(SOLID_RADIUS);
    }
    let bank_name: Arc<str> = Arc::from(
        facts
            .banks()
            .iter()
            .find(|bank| bank.name.contains("Draynor"))
            .expect("selected facts name a Draynor bank")
            .name,
    );
    let seed: &'static [&'static str] = match variant {
        Variant::Session => SEED,
        Variant::StaleNegative | Variant::StaleSolidRadius => &[],
    };
    let request = match variant {
        Variant::StaleSolidRadius => serde_json::json!({
            "walk_near": [solid.x, solid.z, solid.level],
            "radius": SOLID_RADIUS,
            "from": [PORT_SARIM.x, PORT_SARIM.z, PORT_SARIM.level],
            "allow_bank_fetch": true,
            "owner": "Load script (apiVersion 2) isolate walk-near; the slot's script walk pump",
            "script": solid_walk_script(solid),
        }),
        Variant::Session | Variant::StaleNegative => serde_json::json!({
            "walk_to": [KARAMJA.x, KARAMJA.z, KARAMJA.level],
            "from": [PORT_SARIM.x, PORT_SARIM.z, PORT_SARIM.level],
            "allow_bank_fetch": true,
            "owner": "WalkArm (panel/TUI)",
        }),
    };
    let cell = Cell::new(
        CELL_BOUND,
        seed,
        |_| true,
        &[COINS, BONES],
        serde_json::json!({
            "scenario": "bank_fetch_l_fetch_closed",
            "variant": variant,
            "seed": { "bank": SEED, "pack": "cleared" },
            "request": request,
            "login_deadline_ms": LOGIN_DEADLINE.as_millis(),
            "preparation_deadline_ms": PREPARATION_DEADLINE.as_millis(),
            "cell_bound_ms": CELL_BOUND.as_millis(),
        }),
        |_| true,
        evidence_dir.clone(),
    );
    let preparation_started = cell.preparation_started;
    let cell = Arc::new(Mutex::new(cell));
    let arms: WalkArms = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let pump = Arc::new(Mutex::new(Pump::default()));
    let play = {
        let frame_cell = Arc::clone(&cell);
        let frame_account = account.clone();
        let frame_pump = Arc::clone(&pump);
        let frame_arms = Arc::clone(&arms);
        let frame_world = Arc::clone(&world);
        let frame_trace = Arc::clone(&trace);
        let frame_snapshot = Mutex::new(GameSnapshot::new());
        run_with_template(
            template,
            true,
            vec![Profile {
                username: account.clone(),
                password: account.clone().into(),
                uid: 274_279_105,
                settings: ProfileSettings::default(),
            }],
            |_| (None, None),
            move |client, name, input| {
                frame(client, &frame_cell, &frame_account);
                pump_frame(
                    client,
                    name,
                    input.hold,
                    &frame_pump,
                    &mut frame_snapshot.lock(),
                    &frame_arms,
                    &frame_world,
                    map_members,
                    &frame_trace,
                );
            },
        )
        .unwrap_or_else(|error| {
            panic!(
                "HARNESS PREREQUISITE FAILURE (before the cell deadline): starting login: {error}"
            )
        })
    };
    let start = play.script_start_handle();
    let slot_arm = play.arm(&account).expect("the spawned slot's arm");
    let mut login_ready = false;
    let mut stage = Stage::Prepare;
    // The solid-radius cell watches the slot's own script walk: the front
    // step it last traced, and whether a session was latched last poll.
    let mut script_front: Option<Option<BankStep>> = None;
    let mut script_session = false;
    // Where the walk must end: the exact Karamja tile, or (solid radius)
    // within the radius of the solid target on its level.
    let goal = match variant {
        Variant::StaleSolidRadius => solid,
        Variant::Session | Variant::StaleNegative => karamja,
    };
    let arrived = |tile: Option<[i32; 3]>| match (variant, tile) {
        (_, None) => false,
        (Variant::StaleSolidRadius, Some([x, z, level])) => {
            level == solid.level && (x - solid.x).abs().max((z - solid.z).abs()) <= SOLID_RADIUS
        }
        (Variant::Session | Variant::StaleNegative, Some(tile)) => {
            tile == [karamja.x, karamja.z, karamja.level]
        }
    };
    let fail = |message: String| {
        trace.lock().failure = Some(message.clone());
        cell.lock().fail(message);
    };
    loop {
        std::thread::sleep(Duration::from_millis(100));
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        let deadline = cell
            .lock()
            .started
            .map_or(preparation_started + PREPARATION_DEADLINE, |started| {
                started + CELL_BOUND + CAPTURE_GRACE
            });
        assert!(
            Instant::now() < deadline,
            "FAIL L-FETCH-CLOSED ({variant:?}): no evidence by its fixed cell deadline; stage={stage:?} trace={:#?}",
            trace.lock()
        );
        let failed = cell.lock().terminal && !cell.lock().trace.passed;
        if !failed {
            match stage {
                Stage::Prepare => {
                    let ready = {
                        let guard = cell.lock();
                        guard.phase == Prep::Ready
                            && Instant::now().duration_since(guard.last_action)
                                >= Duration::from_secs(1)
                    };
                    if ready {
                        cell.lock().script_started = Some(Instant::now());
                        match variant {
                            Variant::Session => {
                                let script = OpenClose {
                                    facts: Arc::clone(&facts),
                                    bank_name: Arc::clone(&bank_name),
                                    cell: Arc::clone(&cell),
                                    trace: Arc::clone(&trace),
                                    select: None,
                                    open: None,
                                    close: None,
                                    access: None,
                                    opened: false,
                                };
                                match start.start_test_script(
                                    &account,
                                    Box::new(script),
                                    Some(Arc::clone(&selected)),
                                ) {
                                    Ok(_) => {
                                        play.wake(&account);
                                        stage = Stage::OpenClose;
                                    }
                                    Err(error) => fail(format!("Play refused the script: {error}")),
                                }
                            }
                            Variant::StaleNegative | Variant::StaleSolidRadius => {
                                let rows = play.bank_rows(&account);
                                if rows.origin != Origin::Hint
                                    || rows.rows.iter().any(|&(id, _)| id == COINS)
                                    || rows.rows.is_empty()
                                {
                                    fail(format!(
                                        "the new process must load the edited hint (Hint, no coins): {rows:?}"
                                    ));
                                } else {
                                    stage = Stage::OpenClose;
                                }
                            }
                        }
                    }
                }
                Stage::OpenClose => {
                    let done = variant.stale() || trace.lock().open_close_done;
                    if done {
                        match play.cheat(&account, &tele_args(PORT_SARIM)) {
                            Ok(()) => {
                                stage = Stage::Teleport {
                                    since: Instant::now(),
                                }
                            }
                            Err(refusal) => fail(format!("the Port Sarim teleport: {refusal}")),
                        }
                    }
                }
                Stage::Teleport { since } => {
                    let at = status_tile(&play, &account).is_some_and(|(ready, tile)| {
                        ready && tile == [PORT_SARIM.x, PORT_SARIM.z, 0]
                    });
                    if at {
                        stage = Stage::Arm { settled: None };
                    } else if since.elapsed() > Duration::from_secs(30) {
                        fail("the Port Sarim teleport did not land".into());
                    }
                }
                Stage::Arm { settled } => {
                    let Some(since) = settled else {
                        stage = Stage::Arm {
                            settled: Some(Instant::now()),
                        };
                        continue;
                    };
                    if since.elapsed() < Duration::from_secs(3) {
                        continue;
                    }
                    let (state, tile, held, loaded) = {
                        let guard = pump.lock();
                        (
                            guard.state.clone(),
                            guard.tile,
                            guard.held_coins,
                            guard.bank_loaded,
                        )
                    };
                    let (Some(state), Some(tile)) = (state, tile) else {
                        continue;
                    };
                    let bank: BankRows = play.bank_rows(&account);
                    {
                        let mut guard = trace.lock();
                        guard.arm_origin = Some(format!("{:?}", bank.origin));
                        guard.arm_rows = Some(bank.rows.clone());
                        guard.arm_from = Some(tile);
                        guard.arm_bank_loaded = Some(loaded);
                        guard.arm_held_coins = Some(held);
                    }
                    let expected = if variant.stale() {
                        Origin::Hint
                    } else {
                        Origin::Session
                    };
                    if bank.origin != expected || loaded || held != 0 {
                        fail(format!(
                            "arm preconditions: origin {:?} (want {expected:?}), bank loaded {loaded}, held coins {held}",
                            bank.origin
                        ));
                        continue;
                    }
                    if variant == Variant::StaleSolidRadius {
                        match play.script_start_load(
                            &account,
                            solid_walk_script(solid),
                            script::LoadShape::NativeTick,
                            None,
                            vec![],
                        ) {
                            Ok(_) => {
                                play.wake(&account);
                                stage = Stage::AwaitPlan {
                                    since: Instant::now(),
                                };
                            }
                            Err(error) => fail(format!("Play refused the Load script: {error}")),
                        }
                        continue;
                    }
                    let routed = arm_walk_on(
                        &world,
                        Tile {
                            x: tile[0],
                            z: tile[1],
                            level: tile[2],
                        },
                        Tile {
                            x: karamja.x,
                            z: karamja.z,
                            level: karamja.level,
                        },
                        FindOptions {
                            allow_bank_fetch: true,
                            ..FindOptions::default()
                        },
                        &state,
                        &bank,
                        &arms,
                        Some(&account),
                    );
                    let route = match routed {
                        Ok(route) => route,
                        Err(_) => {
                            fail(format!(
                                "{variant:?}: WalkTo with fetch on must plan, not NoPath"
                            ));
                            continue;
                        }
                    };
                    trace.lock().route_legs = Some(leg_lines(&route));
                    let steps: Vec<BankStep> = arms
                        .lock()
                        .unwrap()
                        .get(&account)
                        .and_then(|arm| {
                            arm.lock()
                                .unwrap()
                                .bank_fetch
                                .as_ref()
                                .map(|pending| pending.steps.iter().cloned().collect())
                        })
                        .unwrap_or_default();
                    let (trip, withdraw) = fare_trip(&steps);
                    {
                        let mut guard = trace.lock();
                        guard.planned_steps =
                            Some(steps.iter().map(|step| format!("{step:?}")).collect());
                        guard.planned_withdraw = withdraw;
                    }
                    if !trip {
                        fail(format!(
                            "the session must be Walk/Open/Withdraw {FARE} coins/Close: {steps:?}"
                        ));
                        continue;
                    }
                    pump.lock().pumping = true;
                    stage = Stage::Walking {
                        since: Instant::now(),
                    };
                }
                Stage::AwaitPlan { since } => {
                    let walk = script_walk(&play, &account);
                    if let Some(steps) = walk.steps {
                        let (trip, withdraw) = fare_trip(&steps);
                        {
                            let mut guard = trace.lock();
                            guard.planned_steps =
                                Some(steps.iter().map(|step| format!("{step:?}")).collect());
                            guard.planned_withdraw = withdraw;
                            guard.route_legs = walk.final_route;
                        }
                        if trip {
                            stage = Stage::Walking {
                                since: Instant::now(),
                            };
                        } else {
                            fail(format!(
                                "the session must be Walk/Open/Withdraw {FARE} coins/Close: {steps:?}"
                            ));
                        }
                    } else if walk.failed {
                        fail(format!(
                            "{variant:?}: the script walk-near with fetch on must plan, not NoPath"
                        ));
                    } else if since.elapsed() > Duration::from_secs(60) {
                        fail("the script walk-near planned nothing".into());
                    }
                }
                Stage::Walking { since } => {
                    if variant == Variant::StaleSolidRadius {
                        let walk = script_walk(&play, &account);
                        let (tile, held, loaded) = {
                            let guard = pump.lock();
                            (guard.tile, guard.held_coins, guard.bank_loaded)
                        };
                        let front = walk.steps.as_ref().and_then(|steps| steps.first().cloned());
                        if script_front.as_ref() != Some(&front) {
                            trace.lock().fronts.push(format!(
                                "{front:?} here={tile:?} held_coins={held} bank_loaded={loaded}"
                            ));
                            script_front = Some(front);
                        }
                        if script_session && walk.steps.is_none() {
                            trace.lock().held_after_session = Some(held);
                        }
                        script_session = walk.steps.is_some();
                        if walk.failed {
                            fail("the script walk ended failed".into());
                            continue;
                        }
                        if walk.steps.is_none() && !walk.routed {
                            let mut guard = trace.lock();
                            guard.finished = true;
                            guard.final_tile = tile;
                        }
                    }
                    let finished = trace.lock().finished;
                    if finished {
                        trace.lock().walk_ms = Some(since.elapsed().as_millis());
                        let (tile, held) = {
                            let guard = pump.lock();
                            (guard.tile, guard.held_coins)
                        };
                        let after = play.bank_rows(&account);
                        let mut guard = trace.lock();
                        guard.held_after_walk = Some(held);
                        guard.after_origin = Some(format!("{:?}", after.origin));
                        guard.after_rows = Some(after.rows.clone());
                        let banked = after
                            .rows
                            .iter()
                            .find(|&&(id, _)| id == COINS)
                            .map(|&(_, count)| count);
                        let want_banked = BANKED_AFTER[variant as usize];
                        let problem = if guard.held_after_session != Some(FARE) {
                            Some(format!(
                                "the session must leave exactly the fare held: {:?}",
                                guard.held_after_session
                            ))
                        } else if !arrived(tile) {
                            Some(format!("the walk ended at {tile:?}, not at {goal:?}"))
                        } else if held != 0 {
                            Some(format!("the fare was not paid: {held} coins held"))
                        } else if after.origin != Origin::Session || banked != Some(want_banked) {
                            Some(format!(
                                "the trip's open bank must leave Session with {want_banked} coins: {after:?}"
                            ))
                        } else {
                            None
                        };
                        drop(guard);
                        match problem {
                            Some(message) => fail(message),
                            None => {
                                slot_arm.request_logout();
                                play.wake(&account);
                                stage = Stage::LoggingOut {
                                    since: Instant::now(),
                                };
                            }
                        }
                    } else if since.elapsed() > CELL_BOUND {
                        fail("the walk did not finish".into());
                    }
                }
                Stage::LoggingOut { since } => {
                    if !play.slot_connected(&account) {
                        stage = Stage::AwaitHint {
                            since: Instant::now(),
                        };
                    } else if since.elapsed() > LOGOUT_DEADLINE {
                        fail("the slot did not log out".into());
                    }
                }
                Stage::AwaitHint { since } => {
                    let want = BANKED_AFTER[variant as usize];
                    match hint_rows(&hint) {
                        Ok((contents, rows))
                            if rows.contains(&(COINS, want)) && rows.contains(&(BONES, 5)) =>
                        {
                            trace.lock().hint_after = Some(contents);
                            let mut guard = cell.lock();
                            guard.result = Some(serde_json::json!(*trace.lock()));
                            guard.trace.passed = true;
                            guard.terminal = true;
                            stage = Stage::Done;
                        }
                        Ok((contents, _)) if since.elapsed() > SAVE_GRACE => {
                            fail(format!("the saved hint after the walk: {contents}"))
                        }
                        Err(error) if since.elapsed() > SAVE_GRACE => fail(error),
                        _ => {}
                    }
                }
                Stage::Done => {}
            }
        } else if cell.lock().result.is_none() {
            cell.lock().result = Some(serde_json::json!(*trace.lock()));
        }
        let written = {
            let guard = cell.lock();
            guard
                .capture_written
                .then(|| (guard.trace.clone(), guard.capture_error.clone()))
        };
        if let Some((cell_trace, error)) = written {
            let fetch_trace = trace.lock().clone();
            let receipt = evidence_dir.join("fetch-trace.json");
            std::fs::write(
                &receipt,
                serde_json::to_vec_pretty(&fetch_trace).expect("encode the trace"),
            )
            .expect("write the fetch trace");
            assert!(
                error.is_none(),
                "evidence capture failed: {error:?}; {fetch_trace:#?}"
            );
            assert!(
                cell_trace.passed,
                "FAIL L-FETCH-CLOSED ({variant:?}): {:?}; stage={stage:?}; trace={fetch_trace:#?}; evidence={}",
                cell_trace.failure,
                evidence_dir.display()
            );
            println!(
                "PASS L-FETCH-CLOSED ({variant:?}) account={account} trace={fetch_trace:?} evidence={}",
                evidence_dir.display()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return;
        }
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_fetch_closed_session_port_sarim_to_karamja() {
    run_cell(Variant::Session);
}

#[test]
#[ignore = "requires LIVE=1, S5_LIVE_ACCOUNT from the Session cell under the same HOME, and a local 289 engine"]
fn live_fetch_closed_stale_negative_hint() {
    run_cell(Variant::StaleNegative);
}

#[test]
#[ignore = "requires LIVE=1, S5_LIVE_ACCOUNT after the stale cell under the same HOME, and a local 289 engine"]
fn live_fetch_closed_stale_solid_radius() {
    run_cell(Variant::StaleSolidRadius);
}

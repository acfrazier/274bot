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
/// `%option_nodef`, the auto retaliation varp (content pack varp.pack:173,
/// `172=option_nodef`); 1 is off (player_controls.constant:5).
const AUTO_RETALIATE_VARP: i32 = 172;
const PRAYER_STABLE_TICKS: u32 = 3;
const STOP_OFF_BUDGET_TICKS: u32 = 4;
const ZONE: &str = "death-plateau-throwers";

// GUARD-EAT-1 crossing seed. Engine and content references are the Engine-A
// 289 sources (engine 574f10f, content a608090). No NPC attacks the account in
// the real-content cell, so the fixture makes the emergency bite due on the
// crossing itself instead of waiting for a Thrower Troll.
/// The `death-plateau-throwers` zone rectangle (nav-bake zones.rs:37-48). The
/// seed starts only once the network tile is inside it, so the bite is due on
/// the crossing, not at the start tile.
const ZONE_MIN_X: i32 = 2843;
const ZONE_MAX_X: i32 = 2878;
const ZONE_MIN_Z: i32 = 3590;
const ZONE_MAX_Z: i32 = 3608;
/// `~stat_drain hitpoints 12 0` runs `stat_drain` (debug_stat.rs2:1-3), which
/// subtracts the constant from the current level with no hitmark and no
/// `if_close` (PlayerOps.ts:574-587). The verified HP 20/40 becomes 8/40. At a
/// known zero danger and with no poison the guard does not bite
/// (policy.rs:550-563); the poison below makes the bite due.
const CROSSING_DRAIN: &str = "~stat_drain hitpoints 12 0";
const CROSSING_HP: i32 = 8;
/// `~poison 27` queues `poison_player` (cheat_other.rs2:1-3), which prints
/// "You have been poisoned!" and sets `%poison` (poison.rs2:6-16). The slot
/// reads that line as poison onset (slot_condition.rs:24,540;
/// condition.rs:364-371). At onset the guard's next-fire damage is the
/// unknown-source pulse (condition.rs:558-575): the largest R289
/// `poison_severity` is 55, so the pulse is `(55 + 4) / 5 = 11`
/// (risk/facts.rs:334-340,492-508). The emergency line at zero danger is then
/// `1 + 11 = 12` (guard.rs:690-701, select.rs:227-236, policy.rs:465-477), so
/// HP 8 is due a bite. The poison is sent only after HP 8 is observed, so the
/// bite's HP-rise check starts from the drained HP.
///
/// Login also starts the 30-tick poison timer (login.rs2:63, poison.rs2:72-75),
/// and `set_poison_timer` does not restart a running timer, so the first fire
/// can land at any tick up to 30 after onset (measured: onset tick 26, fire
/// tick 33 in EAT-CROSSING-1120 try-1). One fire is `(27 + 4) / 5 = 6` damage
/// (poison.rs2:20-29) and the next is 30 ticks later
/// (Player.ts:945-958). HP 8 survives one fire before the bite, and the
/// remaining HP 2 is still at the post-mark line `1 + 6 = 7`
/// (condition.rs:313-334,524-555), so the fire's timing cannot change the
/// outcome.
const CROSSING_POISON: &str = "~poison 27";
/// A cheat is read in the next server tick's client input (World.ts:356,379)
/// and its stat change is seen one tick later. The drained HP must show within
/// this many observed ticks of the drain send.
const CROSSING_SEED_WINDOW: u32 = 3;
/// The poison cheat is read in the next server tick and `poison_player` runs in
/// that tick's queue pass (World.ts:356,379), so the poison line is seen one
/// tick later; the guard decides in that frame. One more tick of slack.
const CROSSING_BITE_WINDOW: u32 = 3;
/// The Eat click runs the lobster's consume script in the next server tick
/// (OpHeldHandler.ts:54-68; consume.rs2:4,115-134), which heals 12 and removes
/// the item (consume_normal.dbrow:172-179, consume_effects.rs2:2-11,69-74); the
/// stat and inventory are seen one tick later. The cell keeps its documented
/// four-tick window (docs/dev/harness.md).
const EAT_HEAL_WINDOW: u32 = 4;
/// The cell tests the crossing Eat, not provisioning. The Quester's
/// stage-survival provisioner sizes `imp:0` food at the 28-slot maximum
/// (quester/survival.rs:450-469; status `provision_need 28` in
/// EAT-CROSSING-1120 map pass 3). With fewer lobsters held and the bank
/// unread it scans the bank before the walk (provision.rs:971-1010,1097-1117;
/// acquire/plan.rs:198-207), which happened in 1 of 10 runs with 4 lobsters.
/// Holding 28 makes the held count meet that target, so no trip can start.
const CROSSING_LOBSTERS: i32 = 28;

fn in_throwers_zone(tile: WorldTile) -> bool {
    tile.level == 0
        && (ZONE_MIN_X..=ZONE_MAX_X).contains(&tile.x)
        && (ZONE_MIN_Z..=ZONE_MAX_Z).contains(&tile.z)
}

/// HP after the drain. `health_regen` heals one point every 100 ticks
/// (stat.rs2:59-60, login.rs2:38), so at most one point can land between the
/// verified start baseline and the drain a few ticks later: HP 8 or 9. Both
/// are at the emergency line once poison onset is read.
fn crossing_drained(hp: i32) -> bool {
    EAT_CROSSING.drained(hp)
}

/// Which seeded real-content crossing a cell runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeedArm {
    /// GUARD-EAT-1: lobsters held; HP 8 and poison on the crossing.
    Eat,
    /// D8 without a cure: no food and no antipoison. The seed lands outside
    /// the zones; the Quester stands still where it is, stops with the
    /// reason, and resumes.
    PoisonStop,
    /// D8 with a cure: one `Antipoison(4)` and no food. The cure comes first.
    PoisonCure,
    /// D8 walk out: no food and no antipoison, Protect from Missiles on, and
    /// the poison lands inside a thrower zone on a walk that leaves it. The
    /// Quester keeps the walk to the first tile outside, stands still there
    /// (server-visible), stops with the reason, and resumes.
    PoisonWalkOut,
}

impl SeedArm {
    /// The Prayer level the arm seeds. Protect from Missiles needs 40
    /// (prayers.dbrow:112-118); the walk-out arm seeds 99 so its points last
    /// the whole cell: the prayer drains one point per 5 ticks
    /// (prayers.enum:17 effect 12, prayer.rs2:30-33,163-173 with the base
    /// resistance 60), so 99 points hold for 495 ticks.
    fn prayer(self) -> i32 {
        match self {
            Self::PoisonWalkOut => 99,
            Self::Eat | Self::PoisonStop | Self::PoisonCure => 37,
        }
    }

    /// The HP the arm starts at: 20 of 40, or 40 for the walk-out arm
    /// ([`WALKOUT_CROSSING`]).
    fn start_hp(self) -> i32 {
        match self {
            Self::PoisonWalkOut => 40,
            Self::Eat | Self::PoisonStop | Self::PoisonCure => 20,
        }
    }
}

/// A crossing seed: the drain cheat, the HP it leaves, and the poison cheat.
#[derive(Clone, Copy, Debug)]
struct CrossingPlan {
    drain: &'static str,
    /// The drain's own HP drop, so a poison fire in the drain's tick can be
    /// told from the drain.
    drain_amount: i32,
    hp: i32,
    poison: &'static str,
    /// Send the poison first and the drain once the poison line is seen.
    poison_first: bool,
}

impl CrossingPlan {
    /// HP after the drain. One `health_regen` point can land between the
    /// verified start baseline and the drain (`crossing_drained`).
    fn drained(self, hp: i32) -> bool {
        (1..=self.hp + 1).contains(&hp)
    }
}

const EAT_CROSSING: CrossingPlan = CrossingPlan {
    drain: CROSSING_DRAIN,
    drain_amount: 12,
    hp: CROSSING_HP,
    poison: CROSSING_POISON,
    poison_first: false,
};

/// D8 seed (operator decision D8, 2026-11-21). Engine and content references
/// are the Engine-A 289 sources (engine 574f10f, content a608090).
///
/// `~stat_drain hitpoints 9 0` takes the verified HP 20/40 to 11 (12 with one
/// regeneration point, `crossing_drained`). `~poison 7` queues
/// `poison_player` (cheat_other.rs2:1-3), which sets `%poison` to 7
/// (poison.rs2:6-17). Each fire of the 30-tick `poison` timer queues
/// `(%poison + 4) / 5` damage and lowers `%poison` by one (poison.rs2:20-30,
/// 72-75): 2, 2, 1, 1, 1, 1, 1, so 9 damage in all. HP 11 or 12 ends at 2
/// or 3: the seed cannot kill, and no step waits on a chance outcome.
///
/// At onset the slot reads poison of unknown strength, priced at the
/// largest R289 pulse `(55 + 4) / 5 = 11` (risk/facts.rs:334-340), so the
/// emergency line at zero danger is `1 + 11 = 12` (combat/select.rs:227-236,
/// 213-222). HP 11 or 12 is then the survival need `Cure { Emergency }`
/// (survival/math.rs:149-160) and the cure reason `Emergency`
/// (survival/cure.rs:147), which no onset wait delays. After the first fire
/// (pulse 2) the worst remaining damage is the series sum to severity 9,
/// 13 (api/src/condition.rs:580-584), at least HP 9 or 10: `Lethal`.
/// After the second fire it is still 13 against HP 7 or 8. After the third
/// fire (pulse 1) it is 4, below HP 6 or 7 and above the line `1 + 1`, so
/// the need ends there: the stop holds from onset to the third fire, at
/// least 61 ticks (the first fire lands at most 30 ticks after onset,
/// poison.rs2:72-75), and then resumes.
const POISON_CROSSING: CrossingPlan = CrossingPlan {
    drain: "~stat_drain hitpoints 9 0",
    drain_amount: 9,
    hp: 11,
    poison: "~poison 7",
    poison_first: false,
};
/// D8 walk-out seed: the same poison and the same HP 11, in the other order.
/// The walk-out account stands inside the zone, where a thrower engages it.
/// The survival step prices an engaged bespoke attacker as unknown danger,
/// so its eating line is half the maximum HP (combat/select.rs:224-242):
/// 20 of 40. A drain to 11 before the poison would be a plain food need with
/// no food, a survival park in place, not the D8 state. So the walk-out arm
/// starts at HP 40, above that line, sends `~poison 7` first (the onset is a
/// `Cure` need at any HP: unknown poison is priced at severity 55,
/// survival/math.rs:149-160), and drains 29 to HP 11 once the poison line is
/// seen. From then on the arithmetic is [`POISON_CROSSING`]'s.
const WALKOUT_CROSSING: CrossingPlan = CrossingPlan {
    drain: "~stat_drain hitpoints 29 0",
    drain_amount: 29,
    hp: 11,
    poison: "~poison 7",
    poison_first: true,
};
/// The chat line `poison_player` prints at onset (poison.rs2:6-16).
const POISONED_LINE: &str = "You have been poisoned";
/// The status text prefix of the Quester's D8 stop
/// (script quester/poison_stop.rs `StopText`).
const POISON_STOP_PREFIX: &str = "poisoned at low HP with no food and no antipoison: stopped at ";
const POISON_STOP_SAFE: &str = " outside the danger area";
/// The poison cheat is read in the next server tick, the onset line one tick
/// later; the stop's survival poll decides in that frame. The stop must be
/// shown before the need can end: the third fire lands at most
/// `1 + 30 + 60` ticks after the send (poison.rs2:72-75).
const POISON_STOP_BOUND: u32 = 91;
/// The need ends with the third fire; its hitmark updates the condition in
/// the next observed tick. Nine ticks of slack for the un-park poll.
const POISON_STOP_END_BOUND: u32 = 100;
/// The antipoison sets `%poison` to at most -5 and the timer's negative
/// branch only counts it up (anti_poison.rs2:5-6, poison.rs2:25-27), so no
/// fire deals damage after it. It does not clear a `poison_damage` already
/// queued (poison.rs2:29,32-40), which lands in the next tick. One full
/// 30-tick timer period after the dose drop without an HP drop proves the
/// cure.
const POISON_CURE_WATCH: u32 = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SentCheat {
    tick: u32,
    accepted: bool,
}

impl SentCheat {
    fn json(self) -> serde_json::Value {
        json!({ "tick": self.tick, "accepted": self.accepted })
    }
}

/// The eat-crossing cell's two-step seed, driven from the frame hook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CrossingSeed {
    drain: Option<SentCheat>,
    /// The first observed tick with HP [`CROSSING_HP`] after the drain.
    drained_tick: Option<u32>,
    poison: Option<SentCheat>,
}

impl CrossingSeed {
    fn json(self) -> serde_json::Value {
        json!({
            "drain": self.drain.map(SentCheat::json),
            "drained_tick": self.drained_tick,
            "poison": self.poison.map(SentCheat::json),
        })
    }
}

fn path_json(dest: WorldTile) -> String {
    format!(
        r#"{{
  "schema": {schema},
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
        dest.x,
        dest.z,
        dest.level,
        dest.x,
        dest.z,
        dest.level,
        dest.x,
        dest.z,
        dest.level,
        schema = script::quester::path::PATH_SCHEMA
    )
}

#[test]
fn throwers_crossing_path_compiles_with_current_schema() {
    if !api::test_support::game_data_available() {
        return;
    }
    let default_pack = default_nav_pack_path();
    let default_pack = default_pack.to_string_lossy();
    let Some(pack) = api::test_support::local_asset_path(
        "NAV_PACK",
        &default_pack,
        "R289 throwers navigation catalog",
    ) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    script::quester::compile::prepare_for_test(move |cap| {
        let bytes = std::fs::read(&pack).expect("read the throwers navigation catalog");
        let world = nav_planner::world::NavWorld::from_bytes(&bytes)
            .expect("load the throwers navigation catalog");
        let nav = nav_planner::contract::NavLease::wrap(Arc::new(world))
            .expect("bind the throwers navigation catalog");
        compile_path(
            path_json(WorldTile {
                x: 2880,
                z: 3590,
                level: 0,
            })
            .as_bytes(),
            &selected,
            &quests,
            cap,
            &script::local_state::OperatorLoadouts::absent(),
            nav.zones(),
        )
    })
    .expect("the live crossing fixture must compile before launching a client");
}

fn throwers_route_context(
    world: &nav_planner::world::NavWorld,
) -> (nav_planner::router::FindOptions, nav_planner::WorldState) {
    let table = world.graph.zones.as_ref().expect("baked zone table");
    let key = table.resolve(ZONE).expect("death-plateau-throwers");
    let zones = nav_planner::zones::ZoneExempt::named(&[key]).expect("zone exempt");
    let opts = nav_planner::router::FindOptions {
        zones,
        ..nav_planner::router::FindOptions::default()
    };
    let state = nav_planner::WorldState::empty().with_map_members(true);
    (opts, state)
}

fn pick_crossing(world: &nav_planner::world::NavWorld) -> (WorldTile, WorldTile) {
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
            if nav_planner::router::find_with(
                &world.collision,
                &world.graph,
                *start,
                *dest,
                opts,
                &state,
            )
            .is_ok()
            {
                return (*start, *dest);
            }
        }
    }
    panic!("no standable route of {ZONE}");
}

/// Route tiles the walk-out arm walks inside the zone before its first tile
/// outside. The seed needs one observed tick for the drain, up to
/// [`CROSSING_SEED_WINDOW`] ticks to see it, one tick to see the poison and
/// one decision frame (`CROSSING_BITE_WINDOW`): about 6 ticks, 12 tiles at
/// run speed. 16 tiles leave margin, so the decision lands inside the zone.
const WALKOUT_INSIDE_TILES: usize = 16;
/// Route tiles after the first tile outside. A stop that did not end the
/// server's queued path would carry the player on toward the destination.
const WALKOUT_BEYOND_TILES: usize = 10;

/// A walk-out crossing: a start inside the thrower zone (and inside the
/// seed rectangle [`in_throwers_zone`]), whose route leaves the zone after
/// at least [`WALKOUT_INSIDE_TILES`] tiles, never re-enters it, and goes on
/// for at least [`WALKOUT_BEYOND_TILES`] tiles outside.
///
/// The pack's zone fills the rectangle and opens to the south along most of
/// its width (about 15 tiles from its north edge), to the west at
/// x 2840-2842, and through a narrow lane to the east. So the start is at
/// the east end, and the destination in the open ground west of the zone:
/// the route crosses about 30 zone tiles first. The search is bounded:
/// at most 7 starts and 16 destinations.
fn pick_walkout(
    world: &nav_planner::world::NavWorld,
    inside: &dyn Fn(WorldTile) -> bool,
) -> (WorldTile, WorldTile) {
    let (opts, state) = throwers_route_context(world);
    let starts = (ZONE_MIN_Z + 4..=ZONE_MAX_Z - 2)
        .step_by(2)
        .filter_map(|z| {
            (ZONE_MAX_X - 12..=ZONE_MAX_X)
                .rev()
                .map(|x| WorldTile { x, z, level: 0 })
                .find(|tile| {
                    world.collision.standable(*tile) && inside(*tile) && in_throwers_zone(*tile)
                })
        });
    for start in starts {
        for (x, z) in (0..4).flat_map(|i| (0..4).map(move |j| (2826 + 4 * i, 3584 + 4 * j))) {
            let dest = WorldTile { x, z, level: 0 };
            if !world.collision.standable(dest) || inside(dest) {
                continue;
            }
            let Ok(route) = nav_planner::router::find_with(
                &world.collision,
                &world.graph,
                start,
                dest,
                opts,
                &state,
            ) else {
                continue;
            };
            let mut tiles = Vec::new();
            for leg in &route.legs {
                match leg {
                    nav_planner::router::Leg::Walk { tiles: walk } => tiles.extend_from_slice(walk),
                    nav_planner::router::Leg::Transport { .. } => tiles.clear(),
                }
            }
            let Some(exit) = tiles.iter().position(|tile| !inside(*tile)) else {
                continue;
            };
            if exit >= WALKOUT_INSIDE_TILES
                && tiles.len() - exit >= WALKOUT_BEYOND_TILES
                && !tiles[exit..].iter().any(|tile| inside(*tile))
            {
                return (start, dest);
            }
        }
    }
    panic!("no walk-out route leaves {ZONE} after {WALKOUT_INSIDE_TILES} tiles");
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
        // CI and the platform gate build with BOT_NAV_BUILD=skip, which bakes
        // no pack on purpose; every other build must have produced it.
        if std::env::var("BOT_NAV_BUILD").as_deref() == Ok("skip") {
            eprintln!("skip: death_plateau_throwers needs the baked 289 pack (BOT_NAV_BUILD=skip)");
            return;
        }
        panic!("W1 pack missing at {}", pack.display());
    }
    let world = nav_planner::world::NavWorld::load_pack(&pack).expect("load baked 289 pack");
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
    world: &nav_planner::world::NavWorld,
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
    health: Vec<(u32, i32, i32)>,
    real_content: bool,
    /// The crossing seed a real-content cell sends inside the zone.
    crossing: Option<CrossingPlan>,
    crossing_seed: CrossingSeed,
    /// First observed tick whose Quester status is the `no step for stage`
    /// block (quester/runner.rs:3651-3691).
    park_tick: Option<u32>,
    /// D8 poison cells: the first tick that shows `Antipoison(3)`, the drink's
    /// dose drop from the seeded `Antipoison(4)`.
    cure_tick: Option<u32>,
    /// D8 poison cells: the Quester's `survival_stop` status as observed.
    stop: StopSeen,
    /// D8 poison cells: the network tile when the poison cheat was sent.
    poison_tile: Option<WorldTile>,
    /// The walk-out seed: the newest chat sequence when the poison was sent.
    poison_chat_since: Option<i32>,
}

/// The Quester's D8 poison stop as the frames show it: the first tick and
/// reason of the `survival_stop` status field, the first later tick without
/// it, and every network tile observed while it was shown.
#[derive(Debug, Default)]
struct StopSeen {
    first: Option<(u32, String)>,
    end: Option<u32>,
    tiles: Vec<(u32, WorldTile)>,
}

impl StopSeen {
    fn observe(&mut self, tick: u32, reason: Option<&str>, tile: Option<WorldTile>) {
        match (reason, self.first.is_some(), self.end) {
            (Some(reason), false, _) => self.first = Some((tick, reason.to_owned())),
            (None, true, None) => self.end = Some(tick),
            _ => {}
        }
        if self.first.is_some() && self.end.is_none() {
            if let Some(tile) =
                tile.filter(|tile| self.tiles.last().is_none_or(|row| row.1 != *tile))
            {
                self.tiles.push((tick, tile));
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        json!({
            "first_tick": self.first.as_ref().map(|(tick, _)| tick),
            "reason": self.first.as_ref().map(|(_, reason)| reason),
            "end_tick": self.end,
            "tiles": self.tiles.iter().map(|(tick, tile)| json!([tick, tile.x, tile.z, tile.level])).collect::<Vec<_>>(),
        })
    }
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

    /// Advance the eat-crossing seed by at most one cheat for this frame.
    fn step_crossing_seed(&mut self, client: &mut client::client::Client, tick: u32) {
        if !self.started || self.arrived {
            return;
        }
        let hp = self
            .snapshot
            .stats()
            .iter()
            .find(|row| row.index == 3)
            .map(|row| row.effective);
        let inside = self
            .last_tile
            .is_some_and(|(x, z, level)| in_throwers_zone(WorldTile { x, z, level }));
        let Some(plan) = self.crossing else {
            return;
        };
        let seed = &mut self.crossing_seed;
        if plan.poison_first {
            // The walk-out arm: the poison lands once the walk is under way
            // inside the zone (the player has left its start tile), and the
            // drain once the poison line is seen.
            let here = self
                .last_tile
                .map(|(x, z, level)| WorldTile { x, z, level });
            let moved = here.is_some_and(|here| here != self.start);
            let chat = self.snapshot.chat_lines();
            match (seed.poison, seed.drain, seed.drained_tick) {
                (None, _, _) if inside && moved => {
                    self.poison_chat_since = chat.iter().map(|line| line.sequence).max();
                    seed.poison = Some(SentCheat {
                        tick,
                        accepted: send_cheat(client, plan.poison),
                    });
                    self.poison_tile = here;
                }
                (Some(poison), None, _)
                    if poison.accepted
                        && chat.iter().any(|line| {
                            self.poison_chat_since
                                .is_none_or(|since| line.sequence > since)
                                && line.text.contains(POISONED_LINE)
                        }) =>
                {
                    seed.drain = Some(SentCheat {
                        tick,
                        accepted: send_cheat(client, plan.drain),
                    });
                }
                (Some(_), Some(drain), None)
                    if drain.accepted && hp.is_some_and(|hp| plan.drained(hp)) =>
                {
                    seed.drained_tick = Some(tick);
                }
                _ => {}
            }
            return;
        }
        match (seed.drain, seed.drained_tick) {
            (None, _) if inside => {
                seed.drain = Some(SentCheat {
                    tick,
                    accepted: send_cheat(client, plan.drain),
                });
            }
            (Some(drain), None) if drain.accepted && hp.is_some_and(|hp| plan.drained(hp)) => {
                seed.drained_tick = Some(tick);
                seed.poison = Some(SentCheat {
                    tick,
                    accepted: send_cheat(client, plan.poison),
                });
                self.poison_tile = self
                    .last_tile
                    .map(|(x, z, level)| WorldTile { x, z, level });
            }
            _ => {}
        }
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.last_tile = self.snapshot.local_player().map(|player| {
            let tile = player.player.network;
            (tile.x, tile.z, tile.level)
        });
        let tick = self.snapshot.tick();
        if let Some((x, z, level)) = self.last_tile {
            self.remember_tile(tick, WorldTile { x, z, level });
        }
        if self.real_content && self.started {
            if let Some(hp) = self.snapshot.stats().iter().find(|row| row.index == 3) {
                if self.health.last().is_none_or(|row| row.0 != tick) {
                    let lobsters = self
                        .snapshot
                        .inventory()
                        .iter()
                        .filter(|item| item.def.name.as_deref() == Some("Lobster"))
                        .map(|item| item.count)
                        .sum();
                    self.health.push((tick, hp.effective, lobsters));
                }
            }
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
        if !self.real_content && self.started && !self.attacker_staged {
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
        if !self.real_content
            && self.attacker_staged
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
        if self.crossing.is_some() {
            self.step_crossing_seed(client, tick);
        }
        if self.real_content && self.started {
            let status = self
                .capture
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .statuses
                .last()
                .map(|status| {
                    (
                        status["fields"]["block_reason"] == "no step for stage",
                        status["fields"]["survival_stop"]
                            .as_str()
                            .map(str::to_owned),
                    )
                });
            let (parked, stop) = status.unwrap_or((false, None));
            if parked && self.park_tick.is_none() {
                self.park_tick = Some(tick);
            }
            let here = self
                .last_tile
                .map(|(x, z, level)| WorldTile { x, z, level });
            self.stop.observe(tick, stop.as_deref(), here);
            if self.cure_tick.is_none()
                && self
                    .snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.name.as_deref() == Some("Antipoison(3)"))
            {
                self.cure_tick = Some(tick);
            }
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

fn scenario_for(
    capture: Arc<Mutex<CombatCapture>>,
    start: WorldTile,
    arm: Option<SeedArm>,
) -> Scenario {
    let mut scenario =
        scenario::quester_stage("walk_guard_throwers", "Imp Catcher", "imp", 0, &[], start);
    let stand_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "stand at the quest start")
        .expect("stand step");
    let extra = if let Some(arm) = arm {
        let prayer = arm.prayer();
        let mut steps = vec![
            cheat_step(
                if prayer == 99 {
                    "seed Prayer 99"
                } else {
                    "seed Prayer 37"
                },
                format!("setstat prayer {prayer}"),
                Proof::Stat { id: 5, min: prayer },
            ),
            cheat_step(
                "seed Hitpoints 40",
                "setstat hitpoints 40".to_owned(),
                Proof::Stat { id: 3, min: 40 },
            ),
        ];
        // The walk-out arm keeps HP 40 until the poison is seen
        // (`WALKOUT_CROSSING`).
        if arm != SeedArm::PoisonWalkOut {
            steps.push(cheat_step(
                "seed HP near eat line",
                "~stat_drain hitpoints 20 0".to_owned(),
                Proof::Stat { id: 3, min: 20 },
            ));
        }
        match arm {
            SeedArm::Eat => steps.push(cheat_step(
                "seed required lobster supply",
                format!("give lobster {CROSSING_LOBSTERS}"),
                Proof::Item {
                    name: "Lobster",
                    count: CROSSING_LOBSTERS,
                },
            )),
            SeedArm::PoisonCure => steps.push(cheat_step(
                "seed one antipoison",
                "give 4doseantipoison 1".to_owned(),
                Proof::Item {
                    name: "Antipoison(4)",
                    count: 1,
                },
            )),
            SeedArm::PoisonStop => {}
            // The account's own protection, before it stands in the zone:
            // a thrower's rock checks the protect prayer and deals 0
            // (death_troll_thrower.rs2:26-32, npc_combat_ranged.rs2:47-53),
            // so no rock is a chance outcome. The walk guard did not raise
            // it, so the guard never lowers it (combat/guard.rs
            // `observe_protect`). Auto retaliation is off first: with it on,
            // each rock's `playerhit_n_retaliate` queue runs `p_opnpc(2)`
            // and walks the player to the thrower (auto_retaliate.rs2:1-24,
            // `%option_nodef` 1 = off, player_controls.constant:4-5).
            SeedArm::PoisonWalkOut => {
                steps.push(Step {
                    name: "seed auto retaliation off",
                    kind: StepKind::Repeat {
                        send: Box::new(|client, snapshot| {
                            matches!(
                                api::interact::Interactions::new(snapshot, client)
                                    .set_retaliate(false),
                                api::interact::SendResult::Sent { .. }
                            )
                        }),
                    },
                    wait: Wait {
                        arm: Proof::VarpExact {
                            id: AUTO_RETALIATE_VARP,
                            value: 1,
                        },
                        budget_ticks: 120,
                    },
                });
                steps.push(Step {
                    name: "seed Protect from Missiles",
                    kind: StepKind::Perform {
                        send: Box::new(|client, _| api::interact::press(client, MISSILES_BUTTON)),
                    },
                    wait: Wait {
                        arm: Proof::VarpExact {
                            id: MISSILES_VARP,
                            value: 1,
                        },
                        budget_ticks: 20,
                    },
                });
            }
        }
        steps
    } else {
        vec![cheat_step(
            "seed Prayer 43",
            "setstat prayer 43".to_owned(),
            Proof::Stat { id: 5, min: 43 },
        )]
    };
    scenario.steps.splice(stand_index..stand_index, extra);
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
            summary: None,
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
    run_live_w1(W1Mode::Crossing, None);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_walk_guard_w1_stop_mid_crossing() {
    run_live_w1(W1Mode::StopMidCrossing, None);
}

#[test]
#[ignore = "requires LIVE=1 and the local R289 engine"]
fn live_walk_guard_eat_crossing() {
    run_live_w1(W1Mode::Crossing, Some(SeedArm::Eat));
}

/// D8 without a cure: poisoned at HP 11 or 12 of 40 with no food and no
/// antipoison on the thrower crossing. Pass: the Quester stands still where
/// the poison finds it (the seed lands outside the pack's thrower zones),
/// stops with its reason, the stop ends when the poison can no longer kill,
/// and the walk then arrives ([`poison_arm_decision`]).
#[test]
#[ignore = "requires LIVE=1 and the local R289 engine"]
fn live_walk_guard_poison_no_food_stop() {
    run_live_w1(W1Mode::Crossing, Some(SeedArm::PoisonStop));
}

/// D8 with a cure: the same seed with one `Antipoison(4)`. Pass: the walk
/// guard drinks it at onset, no poison damage follows, no stop is shown,
/// and the walk arrives ([`poison_arm_decision`]).
#[test]
#[ignore = "requires LIVE=1 and the local R289 engine"]
fn live_walk_guard_poison_no_food_cure() {
    run_live_w1(W1Mode::Crossing, Some(SeedArm::PoisonCure));
}

/// D8 walk out: the account starts inside a thrower zone with Protect from
/// Missiles on, and the same seed lands while it is inside, on a walk that
/// leaves the zone ([`pick_walkout`]). Pass: the Quester keeps the walk to
/// the first tile outside, sends a stand click there, stands still on that
/// tile (the server's queued path does not carry it on), stops with its
/// reason, and resumes only when the need is gone ([`poison_arm_decision`]).
/// No rock can hurt the account (the seed's protection), and the poison
/// seed cannot kill (`POISON_CROSSING`).
#[test]
#[ignore = "requires LIVE=1 and the local R289 engine"]
fn live_walk_guard_poison_no_food_walkout() {
    run_live_w1(W1Mode::Crossing, Some(SeedArm::PoisonWalkOut));
}

/// The walk-out arm's route exists in the baked pack.
#[test]
fn death_plateau_throwers_has_a_walkout_route() {
    let pack = default_nav_pack_path();
    if !pack.exists() {
        if std::env::var("BOT_NAV_BUILD").as_deref() == Ok("skip") {
            eprintln!("skip: the walk-out route needs the baked 289 pack (BOT_NAV_BUILD=skip)");
            return;
        }
        panic!("W1 pack missing at {}", pack.display());
    }
    let world = Arc::new(nav_planner::world::NavWorld::load_pack(&pack).expect("load 289 pack"));
    let lease = nav_planner::producer::lease(&world).expect("navigation catalog");
    let group = lease
        .zones()
        .and_then(|zones| zones.resolve(ZONE))
        .expect("the pack names death-plateau-throwers");
    let inside = |tile: WorldTile| {
        lease.zones().is_some_and(|zones| {
            zones
                .containing(tile)
                .any(|zone| zone.key() == group || zone.group() == Some(group))
        })
    };
    let (start, dest) = pick_walkout(&world, &inside);
    eprintln!("walk-out start {start:?} dest {dest:?}");
    assert!(inside(start));
    assert!(!inside(dest));
}

fn eat_heal(row: &serde_json::Value, health: &[(u32, i32, i32)]) -> Option<(u32, i32, i32)> {
    let tick = row["tick"].as_u64()?;
    let hp = row["snapshot"]["stats"]
        .as_array()?
        .iter()
        .find(|stat| stat["index"] == 3)?["effective"]
        .as_i64()?;
    let count: i64 = row["snapshot"]["inventory"]
        .as_array()?
        .iter()
        .filter(|item| item["id"] == 379)
        .filter_map(|item| item["count"].as_i64())
        .sum();
    health.iter().copied().find(|(at, after_hp, after_count)| {
        u64::from(*at) > tick
            && u64::from(*at) <= tick + u64::from(EAT_HEAL_WINDOW)
            && i64::from(*after_hp) > hp
            && i64::from(*after_count) < count
    })
}

#[test]
fn eat_heal_records_click_to_consumption_and_hp_rise_lag() {
    let click = json!({
        "tick": 10,
        "snapshot": {
            "stats": [{"index": 3, "effective": 20}],
            "inventory": [{"id": 379, "count": 4}],
        },
    });
    assert_eq!(
        eat_heal(&click, &[(10, 20, 4), (11, 20, 4), (12, 32, 3)]),
        Some((12, 32, 3))
    );
    assert_eq!(eat_heal(&click, &[(11, 21, 4)]), None);
    assert_eq!(eat_heal(&click, &[(11, 20, 3)]), None);
    assert_eq!(eat_heal(&click, &[(15, 32, 3)]), None);
}

fn evaluate_eat_crossing(state: &LiveState, capture: &CombatCapture, food: bool) -> W1GateDecision {
    if let Some(baseline) = capture.start_baseline.as_ref() {
        let stat = |index, field: &str| {
            baseline["stats"]
                .as_array()
                .and_then(|stats| stats.iter().find(|stat| stat["index"] == index))
                .and_then(|stat| stat[field].as_i64())
        };
        let lobsters: i64 = baseline["inventory"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["id"] == 379)
            .filter_map(|item| item["count"].as_i64())
            .sum();
        let expected_lobsters = if food { CROSSING_LOBSTERS } else { 0 };
        if stat(5, "base") != Some(37)
            || stat(3, "base") != Some(40)
            || stat(3, "effective") != Some(20)
            || lobsters != i64::from(expected_lobsters)
        {
            return gate_fail(
                "GUARD-EAT-1 seed differs from Prayer37, HP20/40 and requested lobsters",
            );
        }
    } else {
        return W1GateDecision::Pending;
    }
    if state.health.iter().any(|(_, hp, _)| *hp <= 0) {
        return gate_fail("death during throwers crossing");
    }
    let eats: Vec<EatClick> = capture
        .actions
        .iter()
        .filter(|row| row["kind"] == "guard" && row["op"] == "eat")
        .filter_map(|row| {
            let tick = u32::try_from(row["tick"].as_u64()?).ok()?;
            Some((tick, eat_heal(row, &state.health)))
        })
        .collect();
    let decision = crossing_eat_decision(&CrossingEatInput {
        seed: state.crossing_seed,
        arrival_tick: state.arrival_tick,
        park_tick: state.park_tick,
        current_tick: state.snapshot.tick(),
        eats: &eats,
    });
    if decision == W1GateDecision::Pass
        && (!state.snapshot.ingame() || state.snapshot.scene_state() != 2)
    {
        return gate_fail("arrival lacks a ready in-game scene");
    }
    decision
}

/// A guard Eat click tick, with its `eat_heal` result (heal tick, HP, lobsters).
type EatClick = (u32, Option<(u32, i32, i32)>);

struct CrossingEatInput<'a> {
    seed: CrossingSeed,
    arrival_tick: Option<u32>,
    park_tick: Option<u32>,
    current_tick: u32,
    /// Every guard Eat click, in capture order.
    eats: &'a [EatClick],
}

/// GUARD-EAT-1 oracle for the seeded crossing. Pass needs: the drain seen as
/// HP 8 inside the zone, the poison sent, no guard Eat before that HP, the
/// first guard Eat within [`CROSSING_BITE_WINDOW`] observed ticks of the poison
/// send and at or before arrival, and the heal and one lobster consumed within
/// [`EAT_HEAL_WINDOW`] ticks of that Eat. Refused or unseen seeds are fixture
/// failures. An Eat before the seeded HP drop is interference (only outside
/// damage could cause it). A `no step for stage` park before arrival is a
/// product failure: the path's only step is skipped only near the destination
/// (`path_json`), so that park is due only after arrival.
fn crossing_eat_decision(input: &CrossingEatInput<'_>) -> W1GateDecision {
    let seed = input.seed;
    for (name, sent) in [("drain", seed.drain), ("poison", seed.poison)] {
        if let Some(sent) = sent.filter(|sent| !sent.accepted) {
            return gate_fail(format!(
                "fixture: the {name} cheat was refused at tick {}",
                sent.tick
            ));
        }
    }
    if let (Some(drain), None) = (seed.drain, seed.drained_tick) {
        if input.current_tick > drain.tick.saturating_add(CROSSING_SEED_WINDOW) {
            return gate_fail(format!(
                "fixture: HP {CROSSING_HP} was not observed within {CROSSING_SEED_WINDOW} ticks of the drain at tick {}",
                drain.tick
            ));
        }
    }
    if let Some(&(tick, _)) = input
        .eats
        .iter()
        .find(|(tick, _)| seed.drained_tick.is_none_or(|drained| *tick < drained))
    {
        return gate_fail(format!(
            "interference: guard Eat at tick {tick} before the seeded HP drop"
        ));
    }
    if let Some(park) = input.park_tick {
        if input.arrival_tick.is_none_or(|arrival| park < arrival) {
            return gate_fail(format!(
                "product: Quester parked `no step for stage` at tick {park} before arrival"
            ));
        }
    }
    let poison = seed.poison.map(|sent| sent.tick);
    match (input.eats.first(), poison) {
        (Some(&(click, heal)), Some(poison)) => {
            if click > poison.saturating_add(CROSSING_BITE_WINDOW) {
                return gate_fail(format!(
                    "product: the first guard Eat at tick {click} is more than {CROSSING_BITE_WINDOW} ticks after the poison at tick {poison}"
                ));
            }
            if input.arrival_tick.is_some_and(|arrival| click > arrival) {
                return gate_fail(format!(
                    "product: the guard Eat at tick {click} came after arrival"
                ));
            }
            match heal {
                Some(_) if input.arrival_tick.is_some() => W1GateDecision::Pass,
                Some(_) => W1GateDecision::Pending,
                None if input.current_tick <= click.saturating_add(EAT_HEAL_WINDOW) => {
                    W1GateDecision::Pending
                }
                None => gate_fail(format!(
                    "product: the guard Eat at tick {click} was not followed within {EAT_HEAL_WINDOW} ticks by an HP rise and one lobster consumed"
                )),
            }
        }
        (None, Some(poison)) if input.current_tick > poison.saturating_add(CROSSING_BITE_WINDOW) => {
            gate_fail(format!(
                "product: no guard Eat within {CROSSING_BITE_WINDOW} ticks of HP {CROSSING_HP} and the poison at tick {poison}"
            ))
        }
        (None, Some(_)) => W1GateDecision::Pending,
        (_, None) => match input.arrival_tick {
            Some(arrival) => gate_fail(format!(
                "fixture: arrival at tick {arrival} before the crossing seed completed"
            )),
            None => W1GateDecision::Pending,
        },
    }
}

fn seeded_crossing() -> CrossingSeed {
    CrossingSeed {
        drain: Some(SentCheat {
            tick: 30,
            accepted: true,
        }),
        drained_tick: Some(32),
        poison: Some(SentCheat {
            tick: 32,
            accepted: true,
        }),
    }
}

fn crossing_input(
    seed: CrossingSeed,
    arrival_tick: Option<u32>,
    park_tick: Option<u32>,
    current_tick: u32,
    eats: &[EatClick],
) -> W1GateDecision {
    crossing_eat_decision(&CrossingEatInput {
        seed,
        arrival_tick,
        park_tick,
        current_tick,
        eats,
    })
}

fn fails_with(decision: W1GateDecision, prefix: &str) -> bool {
    matches!(decision, W1GateDecision::Fail(reason) if reason.starts_with(prefix))
}

#[test]
fn crossing_eat_passes_only_a_seeded_prompt_healed_eat_before_arrival() {
    let healed = [(34, Some((36, 14, 3)))];
    assert_eq!(
        crossing_input(seeded_crossing(), Some(45), Some(47), 47, &healed),
        W1GateDecision::Pass
    );
    assert_eq!(
        crossing_input(seeded_crossing(), None, None, 36, &healed),
        W1GateDecision::Pending
    );
    // The heal window is still open.
    assert_eq!(
        crossing_input(seeded_crossing(), Some(36), None, 37, &[(35, None)]),
        W1GateDecision::Pending
    );
    assert!(fails_with(
        crossing_input(seeded_crossing(), Some(45), None, 45, &[(35, None)]),
        "product: the guard Eat at tick 35 was not followed"
    ));
    assert!(fails_with(
        crossing_input(seeded_crossing(), None, None, 36, &[]),
        "product: no guard Eat within"
    ));
    assert_eq!(
        crossing_input(seeded_crossing(), None, None, 35, &[]),
        W1GateDecision::Pending
    );
    assert!(fails_with(
        crossing_input(
            seeded_crossing(),
            Some(45),
            None,
            45,
            &[(36, Some((38, 14, 3)))]
        ),
        "product: the first guard Eat at tick 36"
    ));
    assert!(fails_with(
        crossing_input(
            seeded_crossing(),
            Some(33),
            None,
            40,
            &[(34, Some((36, 14, 3)))]
        ),
        "product: the guard Eat at tick 34 came after arrival"
    ));
}

#[test]
fn crossing_eat_classifies_interference_park_and_fixture_failures() {
    // An Eat before the drained HP is visible had no seeded cause.
    assert!(fails_with(
        crossing_input(seeded_crossing(), None, None, 31, &[(31, None)]),
        "interference: guard Eat at tick 31"
    ));
    assert!(fails_with(
        crossing_input(CrossingSeed::default(), None, None, 20, &[(20, None)]),
        "interference:"
    ));
    // The only step is skipped only at the destination.
    assert!(fails_with(
        crossing_input(seeded_crossing(), None, Some(33), 33, &[]),
        "product: Quester parked `no step for stage` at tick 33"
    ));
    assert!(fails_with(
        crossing_input(
            seeded_crossing(),
            Some(45),
            Some(44),
            45,
            &[(34, Some((36, 14, 3)))]
        ),
        "product: Quester parked"
    ));
    let refused = CrossingSeed {
        drain: Some(SentCheat {
            tick: 30,
            accepted: false,
        }),
        ..CrossingSeed::default()
    };
    assert!(fails_with(
        crossing_input(refused, None, None, 30, &[]),
        "fixture: the drain cheat was refused"
    ));
    let unseen = CrossingSeed {
        drained_tick: None,
        poison: None,
        ..seeded_crossing()
    };
    assert_eq!(
        crossing_input(unseen, None, None, 33, &[]),
        W1GateDecision::Pending
    );
    assert!(fails_with(
        crossing_input(unseen, None, None, 34, &[]),
        "fixture: HP 8 was not observed"
    ));
    assert!(fails_with(
        crossing_input(CrossingSeed::default(), Some(40), None, 40, &[]),
        "fixture: arrival at tick 40 before the crossing seed"
    ));
}

#[test]
fn crossing_drain_accepts_one_regeneration_point() {
    assert!(!crossing_drained(0));
    assert!(crossing_drained(CROSSING_HP));
    assert!(crossing_drained(CROSSING_HP + 1));
    assert!(!crossing_drained(CROSSING_HP + 2));
    assert!(in_throwers_zone(WorldTile {
        x: ZONE_MIN_X,
        z: ZONE_MAX_Z,
        level: 0
    }));
    assert!(!in_throwers_zone(WorldTile {
        x: ZONE_MIN_X - 1,
        z: 3600,
        level: 0
    }));
}

/// The tile a D8 stop reason names, when the reason says it is outside the
/// danger area.
fn stop_reason_tile(reason: &str) -> Option<WorldTile> {
    let rest = reason
        .strip_prefix(POISON_STOP_PREFIX)?
        .strip_suffix(POISON_STOP_SAFE)?;
    let mut parts = rest.split(',').map(|part| part.parse::<i32>().ok());
    let tile = WorldTile {
        x: parts.next()??,
        z: parts.next()??,
        level: parts.next()??,
    };
    parts.next().is_none().then_some(tile)
}

/// Every accepted stand click (`walk-to` interaction) the host dispatched,
/// as `(snapshot tick, tile)`. The row's `tick` is the host's own counter;
/// `snapshot_tick` is the observed game tick the oracle's other events use.
fn stand_clicks(actions: &[serde_json::Value]) -> Vec<(u32, WorldTile)> {
    actions
        .iter()
        .filter(|row| {
            row["kind"] == "interaction"
                && row["request"]["op"] == "walk-to"
                && row["accepted"] == true
        })
        .filter_map(|row| {
            let int = |value: &serde_json::Value| i32::try_from(value.as_i64()?).ok();
            Some((
                u32::try_from(row["snapshot_tick"].as_u64()?).ok()?,
                WorldTile {
                    x: int(&row["request"]["x"])?,
                    z: int(&row["request"]["z"])?,
                    level: int(&row["request"]["level"])?,
                },
            ))
        })
        .collect()
}

/// Everything the D8 oracle judges, read from the live state and capture.
struct PoisonArmInput<'a> {
    arm: SeedArm,
    seed: CrossingSeed,
    current_tick: u32,
    arrival_tick: Option<u32>,
    /// First `no step for stage` block (the stage's end).
    park_tick: Option<u32>,
    died: bool,
    /// First tick and reason of the `survival_stop` status.
    stop: Option<(u32, &'a str)>,
    stop_end: Option<u32>,
    /// The tile a safe stop's reason names.
    stop_tile: Option<WorldTile>,
    /// Whether that tile is inside the danger area.
    stop_tile_inside: Option<bool>,
    /// Whether any tile observed while the stop was shown is inside it.
    stop_tiles_inside: bool,
    /// Whether any tile observed while the stop was shown differs from the
    /// stop tile.
    stop_tiles_moved: bool,
    /// Accepted stand clicks, `(tick, tile)`.
    stand_clicks: &'a [(u32, WorldTile)],
    /// Whether the tile at the poison send is inside the danger area.
    poison_tile_inside: Option<bool>,
    /// The walk's destination.
    dest: WorldTile,
    /// Every Drink click (walk guard or Quester), in capture order.
    drinks: &'a [u32],
    /// First tick that shows the dose drop to `Antipoison(3)`.
    cure_tick: Option<u32>,
    /// Ticks whose observed HP is below the previous observation.
    hp_drops: &'a [u32],
    /// Ticks of seeded poison fires after the poison ([`poison_fires`]).
    poison_fires: &'a [u32],
}

/// The seeded poison fires that end the survival need (`POISON_CROSSING`:
/// the need holds until the third fire is seen).
const POISON_NEED_FIRES: usize = 3;
/// A walk-out stop this close to the destination may be where the server's
/// queued path ended, not where the Quester stood still.
const WALKOUT_DEST_MARGIN: i32 = 3;

/// D8 oracle (operator decision D8, 2026-11-21). Shared: a refused or unseen
/// seed is a fixture failure, death is a product failure. Every deadline
/// also rejects an event that is seen, but late.
///
/// `PoisonStop` and `PoisonWalkOut` pass when the Quester shows its stop at
/// most [`POISON_STOP_BOUND`] ticks after the poison, before arrival and
/// before the stage ends; the reason names a tile outside the danger area;
/// the last accepted stand click before the stop is on that tile; every tile
/// seen while stopped is that tile; the stop ends at most
/// [`POISON_STOP_END_BOUND`] ticks after the poison, and not before the
/// [`POISON_NEED_FIRES`]th poison HP drop (the need is gone); and the walk
/// then arrives. `PoisonWalkOut` also needs the poison sent inside the zone
/// and the stop at least [`WALKOUT_DEST_MARGIN`] tiles short of the
/// destination.
///
/// `PoisonCure` passes when the first Drink comes within
/// [`CROSSING_BITE_WINDOW`] ticks of the poison, the dose drop within
/// [`EAT_HEAL_WINDOW`] ticks of it, no HP drop after the tick after the dose
/// drop for [`POISON_CURE_WATCH`] ticks, no stop is shown, and the walk
/// arrives before the stage ends.
fn poison_arm_decision(input: &PoisonArmInput<'_>) -> W1GateDecision {
    let seed = input.seed;
    for (name, sent) in [("drain", seed.drain), ("poison", seed.poison)] {
        if let Some(sent) = sent.filter(|sent| !sent.accepted) {
            return gate_fail(format!(
                "fixture: the {name} cheat was refused at tick {}",
                sent.tick
            ));
        }
    }
    if let (Some(drain), None) = (seed.drain, seed.drained_tick) {
        if input.current_tick > drain.tick.saturating_add(CROSSING_SEED_WINDOW) {
            return gate_fail(format!(
                "fixture: HP {} was not observed within {CROSSING_SEED_WINDOW} ticks of the drain at tick {}",
                POISON_CROSSING.hp, drain.tick
            ));
        }
    }
    if input.died {
        return gate_fail("product: death during the poisoned crossing");
    }
    let Some(poison) = seed.poison.map(|sent| sent.tick) else {
        return match input.arrival_tick {
            Some(arrival) => gate_fail(format!(
                "fixture: arrival at tick {arrival} before the crossing seed completed"
            )),
            None => W1GateDecision::Pending,
        };
    };
    if let Some((tick, _)) = input.stop.filter(|(tick, _)| *tick < poison) {
        return gate_fail(format!(
            "interference: a poison stop at tick {tick} before the seeded poison"
        ));
    }
    match input.arm {
        SeedArm::PoisonCure => cure_arm_decision(input, poison),
        SeedArm::PoisonStop | SeedArm::PoisonWalkOut => stop_arm_decision(input, poison),
        SeedArm::Eat => gate_fail("fixture: the eat arm has its own oracle"),
    }
}

fn cure_arm_decision(input: &PoisonArmInput<'_>, poison: u32) -> W1GateDecision {
    if let Some((tick, reason)) = input.stop {
        return gate_fail(format!(
            "product: the Quester stopped at tick {tick} although it held an antipoison: {reason}"
        ));
    }
    if let Some(park) = input.park_tick {
        if input.arrival_tick.is_none_or(|arrival| park < arrival) {
            return gate_fail(format!(
                "product: Quester parked `no step for stage` at tick {park} before arrival"
            ));
        }
    }
    let Some(&drink) = input.drinks.first() else {
        if input.current_tick > poison.saturating_add(CROSSING_BITE_WINDOW) {
            return gate_fail(format!(
                "product: no Drink within {CROSSING_BITE_WINDOW} ticks of the poison at tick {poison}"
            ));
        }
        return W1GateDecision::Pending;
    };
    if drink > poison.saturating_add(CROSSING_BITE_WINDOW) {
        return gate_fail(format!(
            "product: the first Drink at tick {drink} is more than {CROSSING_BITE_WINDOW} ticks after the poison at tick {poison}"
        ));
    }
    let Some(cure) = input.cure_tick else {
        if input.current_tick > drink.saturating_add(EAT_HEAL_WINDOW) {
            return gate_fail(format!(
                "product: the Drink at tick {drink} was not followed within {EAT_HEAL_WINDOW} ticks by the dose drop"
            ));
        }
        return W1GateDecision::Pending;
    };
    if cure > drink.saturating_add(EAT_HEAL_WINDOW) {
        return gate_fail(format!(
            "product: the dose drop at tick {cure} is more than {EAT_HEAL_WINDOW} ticks after the Drink at tick {drink}"
        ));
    }
    if let Some(drop) = input
        .hp_drops
        .iter()
        .find(|drop| **drop > cure.saturating_add(1))
    {
        return gate_fail(format!(
            "product: HP fell at tick {drop} after the cure at tick {cure}"
        ));
    }
    if input.current_tick < cure.saturating_add(POISON_CURE_WATCH) || input.arrival_tick.is_none() {
        return W1GateDecision::Pending;
    }
    W1GateDecision::Pass
}

fn stop_arm_decision(input: &PoisonArmInput<'_>, poison: u32) -> W1GateDecision {
    if input.arm == SeedArm::PoisonWalkOut {
        let seed = input.seed;
        if let (Some(sent), None) = (seed.poison, seed.drain) {
            if input.current_tick > sent.tick.saturating_add(CROSSING_SEED_WINDOW) {
                return gate_fail(format!(
                    "fixture: the poison line was not seen within {CROSSING_SEED_WINDOW} ticks of the poison at tick {}",
                    sent.tick
                ));
            }
        }
    }
    if input.arm == SeedArm::PoisonWalkOut && input.poison_tile_inside != Some(true) {
        return gate_fail(format!(
            "fixture: the poison was not sent inside the zone (inside: {:?})",
            input.poison_tile_inside
        ));
    }
    let stop_deadline = poison.saturating_add(POISON_STOP_BOUND);
    let Some((stop, reason)) = input.stop else {
        if let Some(park) = input.park_tick {
            return gate_fail(format!(
                "product: Quester ended the stage at tick {park} without the poison stop"
            ));
        }
        if input.current_tick > stop_deadline {
            return gate_fail(format!(
                "product: no poison stop within {POISON_STOP_BOUND} ticks of the poison at tick {poison}"
            ));
        }
        return W1GateDecision::Pending;
    };
    if stop > stop_deadline {
        return gate_fail(format!(
            "product: the poison stop at tick {stop} is more than {POISON_STOP_BOUND} ticks after the poison at tick {poison}"
        ));
    }
    if let Some(arrival) = input.arrival_tick.filter(|arrival| *arrival < stop) {
        return gate_fail(format!(
            "product: arrival at tick {arrival} before the poison stop at tick {stop}"
        ));
    }
    let Some(tile) = input.stop_tile.filter(|_| input.stop_tile_inside.is_some()) else {
        return gate_fail(format!(
            "product: the stop reason names no tile outside the danger area: {reason}"
        ));
    };
    if input.stop_tile_inside == Some(true) {
        return gate_fail(format!(
            "product: the stop tile is inside the danger area: {reason}"
        ));
    }
    let Some(&(click_tick, click)) = input
        .stand_clicks
        .iter()
        .rfind(|(tick, _)| *tick >= poison && *tick <= stop)
    else {
        return gate_fail(format!(
            "product: no stand click between the poison at tick {poison} and the stop at tick {stop}"
        ));
    };
    if click != tile {
        return gate_fail(format!(
            "product: the stop tile {},{},{} is not the stand click tile {},{},{} (tick {click_tick})",
            tile.x, tile.z, tile.level, click.x, click.z, click.level
        ));
    }
    if input.stop_tiles_inside {
        return gate_fail("product: a tile inside the danger area was seen while stopped");
    }
    if input.stop_tiles_moved {
        return gate_fail("product: the player moved while the stop was shown");
    }
    if input.arm == SeedArm::PoisonWalkOut && chebyshev(tile, input.dest) < WALKOUT_DEST_MARGIN {
        return gate_fail(format!(
            "product: the walk-out stop {},{},{} is within {WALKOUT_DEST_MARGIN} tiles of the destination: the queued path was not ended",
            tile.x, tile.z, tile.level
        ));
    }
    let end_deadline = poison.saturating_add(POISON_STOP_END_BOUND);
    let Some(end) = input.stop_end else {
        if input.current_tick > end_deadline {
            return gate_fail(format!(
                "product: the stop did not end within {POISON_STOP_END_BOUND} ticks of the poison at tick {poison}"
            ));
        }
        return W1GateDecision::Pending;
    };
    if end > end_deadline {
        return gate_fail(format!(
            "product: the stop ended at tick {end}, more than {POISON_STOP_END_BOUND} ticks after the poison at tick {poison}"
        ));
    }
    let need_end = input.poison_fires.get(POISON_NEED_FIRES - 1);
    if need_end.is_none_or(|need_end| end < *need_end) {
        return gate_fail(format!(
            "product: the stop ended at tick {end} while the survival need was present (poison fire {POISON_NEED_FIRES} seen at {need_end:?})"
        ));
    }
    if let Some(park) = input.park_tick.filter(|park| *park < end) {
        return gate_fail(format!(
            "product: Quester ended the stage at tick {park} during the poison stop"
        ));
    }
    match input.arrival_tick {
        Some(arrival) if arrival > end => W1GateDecision::Pass,
        _ => W1GateDecision::Pending,
    }
}

/// Ticks of the seeded poison's fires: HP drops after the poison send. The
/// walk-out arm drains after the poison; the drop at the drained tick counts
/// as a fire only when it is larger than the drain itself (a fire in the
/// same tick).
fn poison_fires(health: &[(u32, i32, i32)], seed: CrossingSeed, drain_amount: i32) -> Vec<u32> {
    let Some(poison) = seed.poison.map(|sent| sent.tick) else {
        return Vec::new();
    };
    health
        .windows(2)
        .filter(|rows| rows[1].0 > poison && rows[1].1 < rows[0].1)
        .filter(|rows| Some(rows[1].0) != seed.drained_tick || rows[0].1 - rows[1].1 > drain_amount)
        .map(|rows| rows[1].0)
        .collect()
}

#[test]
fn poison_fires_skip_the_walkout_drain_but_not_a_fire_in_its_tick() {
    let seed = CrossingSeed {
        poison: Some(SentCheat {
            tick: 30,
            accepted: true,
        }),
        drain: Some(SentCheat {
            tick: 32,
            accepted: true,
        }),
        drained_tick: Some(33),
    };
    let health = [
        (29, 40, 0),
        (31, 40, 0),
        (33, 11, 0),
        (60, 9, 0),
        (90, 7, 0),
    ];
    assert_eq!(poison_fires(&health, seed, 29), [60, 90]);
    let merged = [(29, 40, 0), (33, 9, 0), (60, 7, 0)];
    assert_eq!(poison_fires(&merged, seed, 29), [33, 60]);
    // The stop arm drains before the poison: every later drop is a fire.
    let stop = CrossingSeed {
        drain: Some(SentCheat {
            tick: 30,
            accepted: true,
        }),
        drained_tick: Some(32),
        poison: Some(SentCheat {
            tick: 32,
            accepted: true,
        }),
    };
    let health = [
        (29, 20, 0),
        (32, 11, 0),
        (50, 9, 0),
        (80, 7, 0),
        (110, 6, 0),
    ];
    assert_eq!(poison_fires(&health, stop, 9), [50, 80, 110]);
}

/// Read the D8 oracle input from the live state. `inside` says whether a
/// tile is in a `death-plateau-throwers` zone of the selected pack.
fn evaluate_poison_arm(
    state: &LiveState,
    capture: &CombatCapture,
    arm: SeedArm,
    inside: &dyn Fn(WorldTile) -> bool,
) -> W1GateDecision {
    let Some(baseline) = capture.start_baseline.as_ref() else {
        return W1GateDecision::Pending;
    };
    if let Err(reason) = poison_baseline(baseline, arm) {
        return gate_fail(format!("fixture: {reason}"));
    }
    let drinks: Vec<u32> = capture
        .actions
        .iter()
        .filter(|row| {
            (row["kind"] == "guard" && row["op"] == "drink")
                || (row["kind"] == "interaction"
                    && row["request"]["op"] == "held"
                    && row["request"]["action"] == "Drink")
        })
        .filter_map(|row| u32::try_from(row["tick"].as_u64()?).ok())
        .collect();
    let hp_drops: Vec<u32> = state
        .health
        .windows(2)
        .filter(|rows| rows[1].1 < rows[0].1)
        .map(|rows| rows[1].0)
        .collect();
    let fires = poison_fires(
        &state.health,
        state.crossing_seed,
        state.crossing.map_or(0, |plan| plan.drain_amount),
    );
    let clicks = stand_clicks(&capture.actions);
    let stop = state
        .stop
        .first
        .as_ref()
        .map(|(tick, reason)| (*tick, reason.as_str()));
    let stop_tile = stop.and_then(|(_, reason)| stop_reason_tile(reason));
    let decision = poison_arm_decision(&PoisonArmInput {
        arm,
        seed: state.crossing_seed,
        current_tick: state.snapshot.tick(),
        arrival_tick: state.arrival_tick,
        park_tick: state.park_tick,
        died: state.health.iter().any(|(_, hp, _)| *hp <= 0),
        stop,
        stop_end: state.stop.end,
        stop_tile,
        stop_tile_inside: stop_tile.map(inside),
        stop_tiles_inside: state.stop.tiles.iter().any(|(_, tile)| inside(*tile)),
        stop_tiles_moved: stop_tile
            .is_some_and(|stood| state.stop.tiles.iter().any(|(_, tile)| *tile != stood)),
        stand_clicks: &clicks,
        poison_tile_inside: state.poison_tile.map(inside),
        dest: state.dest,
        drinks: &drinks,
        cure_tick: state.cure_tick,
        hp_drops: &hp_drops,
        poison_fires: &fires,
    });
    if decision == W1GateDecision::Pass
        && (!state.snapshot.ingame() || state.snapshot.scene_state() != 2)
    {
        return gate_fail("arrival lacks a ready in-game scene");
    }
    decision
}

/// The D8 start seed: the arm's Prayer level, HP 20 of 40, no food, and one
/// `Antipoison(4)` only in the cure arm.
fn poison_baseline(baseline: &serde_json::Value, arm: SeedArm) -> Result<(), String> {
    let stat = |index, field: &str| {
        baseline["stats"]
            .as_array()
            .and_then(|stats| stats.iter().find(|stat| stat["index"] == index))
            .and_then(|stat| stat[field].as_i64())
    };
    let prayer = arm.prayer();
    let hp = arm.start_hp();
    if stat(5, "base") != Some(i64::from(prayer))
        || stat(3, "base") != Some(40)
        || stat(3, "effective") != Some(i64::from(hp))
    {
        return Err(format!(
            "D8 seed differs from Prayer {prayer} and HP {hp}/40"
        ));
    }
    let tables = script::combat::guard::shared_tables()
        .map_err(|error| format!("combat tables: {error:?}"))?;
    let dose = api::game_data::for_revision(ClientRevision::R289)
        .ok()
        .and_then(|data| data.item_by_alias("4doseantipoison").map(|item| item.id))
        .ok_or("no Antipoison(4) in the selected data")?;
    let mut doses = 0;
    for item in baseline["inventory"].as_array().into_iter().flatten() {
        let id = item["id"].as_i64().and_then(|id| i32::try_from(id).ok());
        if id.is_some_and(|id| tables.food(id).is_some()) {
            return Err(format!("D8 seed holds food: {item}"));
        }
        if id == Some(dose) {
            doses += item["count"].as_i64().unwrap_or(0);
        }
    }
    let expected = i64::from(arm == SeedArm::PoisonCure);
    if doses != expected {
        return Err(format!(
            "D8 seed holds {doses} Antipoison(4), expected {expected}"
        ));
    }
    Ok(())
}

fn poisoned_crossing() -> CrossingSeed {
    CrossingSeed {
        drain: Some(SentCheat {
            tick: 30,
            accepted: true,
        }),
        drained_tick: Some(32),
        poison: Some(SentCheat {
            tick: 32,
            accepted: true,
        }),
    }
}

const SAFE_REASON: &str =
    "poisoned at low HP with no food and no antipoison: stopped at 2880,3600,0 outside the danger area";
const SAFE_TILE: WorldTile = WorldTile {
    x: 2880,
    z: 3600,
    level: 0,
};
const SAFE_CLICK: [(u32, WorldTile); 1] = [(36, SAFE_TILE)];

fn stop_receipt() -> PoisonArmInput<'static> {
    PoisonArmInput {
        arm: SeedArm::PoisonStop,
        seed: poisoned_crossing(),
        current_tick: 150,
        arrival_tick: Some(140),
        park_tick: Some(142),
        died: false,
        stop: Some((40, SAFE_REASON)),
        stop_end: Some(110),
        stop_tile: Some(SAFE_TILE),
        stop_tile_inside: Some(false),
        stop_tiles_inside: false,
        stop_tiles_moved: false,
        stand_clicks: &SAFE_CLICK,
        poison_tile_inside: Some(false),
        dest: WorldTile {
            x: 2895,
            z: 3600,
            level: 0,
        },
        drinks: &[],
        cure_tick: None,
        hp_drops: &[50, 80, 110],
        poison_fires: &[50, 80, 110],
    }
}

fn walkout_receipt() -> PoisonArmInput<'static> {
    PoisonArmInput {
        arm: SeedArm::PoisonWalkOut,
        poison_tile_inside: Some(true),
        ..stop_receipt()
    }
}

fn cure_receipt() -> PoisonArmInput<'static> {
    PoisonArmInput {
        arm: SeedArm::PoisonCure,
        seed: poisoned_crossing(),
        current_tick: 80,
        arrival_tick: Some(60),
        park_tick: Some(62),
        died: false,
        stop: None,
        stop_end: None,
        stop_tile: None,
        stop_tile_inside: None,
        stop_tiles_inside: false,
        stop_tiles_moved: false,
        stand_clicks: &[],
        poison_tile_inside: Some(false),
        dest: SAFE_TILE,
        drinks: &[34],
        cure_tick: Some(36),
        hp_drops: &[37],
        poison_fires: &[],
    }
}

#[test]
fn poison_stop_oracle_passes_a_safe_stop_that_resumes_and_rejects_each_bad_behaviour() {
    assert_eq!(poison_arm_decision(&stop_receipt()), W1GateDecision::Pass);
    // The walk after the stop has not arrived yet.
    let mut walking = stop_receipt();
    walking.arrival_tick = None;
    walking.park_tick = None;
    assert_eq!(poison_arm_decision(&walking), W1GateDecision::Pending);
    let mut missing = stop_receipt();
    missing.stop = None;
    missing.stop_end = None;
    missing.arrival_tick = None;
    missing.park_tick = None;
    missing.current_tick = 32 + POISON_STOP_BOUND;
    assert_eq!(poison_arm_decision(&missing), W1GateDecision::Pending);
    missing.current_tick += 1;
    assert!(fails_with(
        poison_arm_decision(&missing),
        "product: no poison stop within"
    ));
    // A stop first seen after its deadline, in the same observation that
    // shows it at all.
    let mut late_stop = stop_receipt();
    late_stop.stop = Some((32 + POISON_STOP_BOUND + 1, SAFE_REASON));
    late_stop.stand_clicks = &[(32 + POISON_STOP_BOUND, SAFE_TILE)];
    assert!(fails_with(
        poison_arm_decision(&late_stop),
        "product: the poison stop at tick 124 is more than"
    ));
    let mut continued = stop_receipt();
    continued.stop = None;
    continued.stop_end = None;
    assert!(fails_with(
        poison_arm_decision(&continued),
        "product: Quester ended the stage at tick 142 without the poison stop"
    ));
    let mut late = stop_receipt();
    late.arrival_tick = Some(39);
    assert!(fails_with(
        poison_arm_decision(&late),
        "product: arrival at tick 39"
    ));
    let mut inside = stop_receipt();
    inside.stop_tile_inside = Some(true);
    assert!(fails_with(
        poison_arm_decision(&inside),
        "product: the stop tile is inside"
    ));
    let mut unnamed = stop_receipt();
    unnamed.stop_tile = None;
    unnamed.stop_tile_inside = None;
    assert!(fails_with(
        poison_arm_decision(&unnamed),
        "product: the stop reason names no tile"
    ));
    let mut unclicked = stop_receipt();
    unclicked.stand_clicks = &[];
    assert!(fails_with(
        poison_arm_decision(&unclicked),
        "product: no stand click between"
    ));
    let mut elsewhere = stop_receipt();
    elsewhere.stand_clicks = &[(
        36,
        WorldTile {
            x: 2878,
            z: 3600,
            level: 0,
        },
    )];
    assert!(fails_with(
        poison_arm_decision(&elsewhere),
        "product: the stop tile 2880,3600,0 is not the stand click tile 2878,3600,0"
    ));
    let mut wandered = stop_receipt();
    wandered.stop_tiles_inside = true;
    assert!(fails_with(
        poison_arm_decision(&wandered),
        "product: a tile inside"
    ));
    let mut drifted = stop_receipt();
    drifted.stop_tiles_moved = true;
    assert!(fails_with(
        poison_arm_decision(&drifted),
        "product: the player moved while the stop was shown"
    ));
    let mut held = stop_receipt();
    held.stop_end = None;
    held.arrival_tick = None;
    held.park_tick = None;
    held.current_tick = 32 + POISON_STOP_END_BOUND + 1;
    assert!(fails_with(
        poison_arm_decision(&held),
        "product: the stop did not end"
    ));
    // An end first seen after its deadline.
    let mut late_end = stop_receipt();
    late_end.stop_end = Some(32 + POISON_STOP_END_BOUND + 1);
    late_end.arrival_tick = Some(140);
    assert!(fails_with(
        poison_arm_decision(&late_end),
        "product: the stop ended at tick 133, more than"
    ));
    // A resume while the third poison fire has not come.
    let mut early_end = stop_receipt();
    early_end.stop_end = Some(100);
    assert!(fails_with(
        poison_arm_decision(&early_end),
        "product: the stop ended at tick 100 while the survival need was present"
    ));
    let mut unfired = stop_receipt();
    unfired.poison_fires = &[50, 80];
    assert!(fails_with(
        poison_arm_decision(&unfired),
        "product: the stop ended at tick 110 while the survival need was present"
    ));
    let mut ended = stop_receipt();
    ended.park_tick = Some(100);
    ended.stop_end = Some(110);
    assert!(fails_with(
        poison_arm_decision(&ended),
        "product: Quester ended the stage at tick 100 during"
    ));
    let mut dead = stop_receipt();
    dead.died = true;
    assert!(fails_with(poison_arm_decision(&dead), "product: death"));
    let mut early = stop_receipt();
    early.stop = Some((31, SAFE_REASON));
    assert!(fails_with(poison_arm_decision(&early), "interference:"));
    let mut refused = stop_receipt();
    refused.seed.poison = Some(SentCheat {
        tick: 32,
        accepted: false,
    });
    assert!(fails_with(
        poison_arm_decision(&refused),
        "fixture: the poison cheat was refused"
    ));
}

#[test]
fn poison_walkout_oracle_needs_the_poison_inside_and_a_stop_short_of_the_destination() {
    assert_eq!(
        poison_arm_decision(&walkout_receipt()),
        W1GateDecision::Pass
    );
    let mut unseen = walkout_receipt();
    unseen.seed.drain = None;
    unseen.seed.drained_tick = None;
    unseen.current_tick = 32 + CROSSING_SEED_WINDOW + 1;
    assert!(fails_with(
        poison_arm_decision(&unseen),
        "fixture: the poison line was not seen"
    ));
    let mut outside = walkout_receipt();
    outside.poison_tile_inside = Some(false);
    assert!(fails_with(
        poison_arm_decision(&outside),
        "fixture: the poison was not sent inside the zone"
    ));
    let mut carried = walkout_receipt();
    carried.dest = WorldTile {
        x: 2882,
        z: 3600,
        level: 0,
    };
    assert!(fails_with(
        poison_arm_decision(&carried),
        "product: the walk-out stop 2880,3600,0 is within 3 tiles of the destination"
    ));
    let mut drifted = walkout_receipt();
    drifted.stop_tiles_moved = true;
    assert!(fails_with(
        poison_arm_decision(&drifted),
        "product: the player moved while the stop was shown"
    ));
}

#[test]
fn poison_cure_oracle_passes_a_prompt_cure_and_rejects_each_bad_behaviour() {
    assert_eq!(poison_arm_decision(&cure_receipt()), W1GateDecision::Pass);
    let mut watching = cure_receipt();
    watching.current_tick = 36 + POISON_CURE_WATCH - 1;
    assert_eq!(poison_arm_decision(&watching), W1GateDecision::Pending);
    let mut stopped = cure_receipt();
    stopped.stop = Some((40, SAFE_REASON));
    assert!(fails_with(
        poison_arm_decision(&stopped),
        "product: the Quester stopped"
    ));
    let mut none = cure_receipt();
    none.drinks = &[];
    none.cure_tick = None;
    none.current_tick = 32 + CROSSING_BITE_WINDOW;
    assert_eq!(poison_arm_decision(&none), W1GateDecision::Pending);
    none.current_tick += 1;
    assert!(fails_with(
        poison_arm_decision(&none),
        "product: no Drink within"
    ));
    let mut slow = cure_receipt();
    slow.drinks = &[36];
    assert!(fails_with(
        poison_arm_decision(&slow),
        "product: the first Drink at tick 36"
    ));
    let mut dropped = cure_receipt();
    dropped.cure_tick = None;
    assert!(fails_with(
        poison_arm_decision(&dropped),
        "product: the Drink at tick 34 was not followed"
    ));
    // A dose drop first seen after its deadline.
    let mut late_cure = cure_receipt();
    late_cure.cure_tick = Some(34 + EAT_HEAL_WINDOW + 1);
    late_cure.hp_drops = &[];
    late_cure.current_tick = 120;
    assert!(fails_with(
        poison_arm_decision(&late_cure),
        "product: the dose drop at tick 39 is more than"
    ));
    let mut poisoned = cure_receipt();
    poisoned.hp_drops = &[37, 66];
    assert!(fails_with(
        poison_arm_decision(&poisoned),
        "product: HP fell at tick 66"
    ));
    let mut parked = cure_receipt();
    parked.park_tick = Some(50);
    assert!(fails_with(
        poison_arm_decision(&parked),
        "product: Quester parked"
    ));
    let mut dead = cure_receipt();
    dead.died = true;
    assert!(fails_with(poison_arm_decision(&dead), "product: death"));
}

#[test]
fn stop_reason_tile_reads_only_a_safe_stop() {
    assert_eq!(stop_reason_tile(SAFE_REASON), Some(SAFE_TILE));
    assert_eq!(
        stop_reason_tile(
            "poisoned at low HP with no food and no antipoison: stopped at 2860,3600,0 inside the danger area (no safe tile seen in this danger area)"
        ),
        None
    );
    assert_eq!(
        stop_reason_tile(
            "poisoned at low HP with no food and no antipoison: stopped at 2860,3600,0, but the player was not seen standing still (the player still moved)"
        ),
        None
    );
    assert_eq!(
        stop_reason_tile(
            "poisoned at low HP with no food and no antipoison: stopped outside the danger area"
        ),
        None
    );
}

#[test]
fn stand_clicks_reads_accepted_walk_to_rows() {
    let rows = [
        json!({"kind": "interaction", "tick": 49, "snapshot_tick": 36, "accepted": true,
            "request": {"op": "walk-to", "x": 2880, "z": 3600, "level": 0}}),
        json!({"kind": "interaction", "tick": 50, "snapshot_tick": 37, "accepted": false,
            "request": {"op": "walk-to", "x": 2881, "z": 3600, "level": 0}}),
        json!({"kind": "interaction", "tick": 51, "snapshot_tick": 38, "accepted": true,
            "request": {"op": "held", "name": "Antipoison(4)", "action": "Drink"}}),
    ];
    assert_eq!(stand_clicks(&rows), [(36, SAFE_TILE)]);
}

fn run_live_w1(mode: W1Mode, arm: Option<SeedArm>) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "W1 requires LIVE=1"
    );
    let evidence = evidence_dir().expect("W1 live prerequisites");
    std::fs::create_dir_all(&evidence).expect("create W1 lifecycle evidence directory");
    let home = arm
        .is_none()
        .then(ThrowawayHome::enter)
        .transpose()
        .expect("create isolated HOME");
    let home_path = home.as_ref().map_or_else(
        || {
            let id = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("cell clock")
                .as_nanos();
            PathBuf::from(std::env::var_os("HOME").expect("GUARD-EAT-1 requires an isolated HOME"))
                .join(format!("cell-{id}"))
        },
        |home| home.path.clone(),
    );
    let mut options = profile_options(&home_path).expect("W1 live prerequisites");
    if arm.is_some() {
        options.port = Some(crate::live_ports::game_port());
        options.http_port = Some(crate::live_ports::http_port());
    }
    let template = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare isolated local-289 template");
    let world = template.world().expect("selected navigation world");
    let nav = nav_planner::producer::lease(&world).expect("selected navigation catalog");
    // The D8 oracle judges tiles against the pack's own throwers zones.
    let zone_lease = nav_planner::producer::lease(&world).expect("selected navigation catalog");
    let danger_group = zone_lease
        .zones()
        .and_then(|zones| zones.resolve(ZONE))
        .expect("the pack names death-plateau-throwers");
    let inside_danger = |tile: WorldTile| {
        zone_lease.zones().is_some_and(|zones| {
            zones
                .containing(tile)
                .any(|zone| zone.key() == danger_group || zone.group() == Some(danger_group))
        })
    };
    // Start outside the Throwers' hunt radius so Quester walks into the
    // threat; the walk-out arm starts inside it on a walk that leaves it.
    let (start, dest) = if arm == Some(SeedArm::PoisonWalkOut) {
        pick_walkout(world.as_ref(), &inside_danger)
    } else {
        pick_crossing(world.as_ref())
    };
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    let path = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        move |cap| {
            compile_path(
                path_json(dest).as_bytes(),
                &selected,
                &quests,
                cap,
                &script::local_state::OperatorLoadouts::absent(),
                nav.zones(),
            )
        }
    })
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
    let scenario = scenario_for(Arc::clone(&capture), start, arm);
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
        health: Vec::new(),
        real_content: arm.is_some(),
        crossing: match arm {
            Some(SeedArm::Eat) => Some(EAT_CROSSING),
            Some(SeedArm::PoisonStop | SeedArm::PoisonCure) => Some(POISON_CROSSING),
            Some(SeedArm::PoisonWalkOut) => Some(WALKOUT_CROSSING),
            None => None,
        },
        crossing_seed: CrossingSeed::default(),
        park_tick: None,
        cure_tick: None,
        stop: StopSeen::default(),
        poison_tile: None,
        poison_chat_since: None,
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
        let relog_login = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .runner
            .take_relog_login();
        if relog_login {
            if let Some(arm) = play.arm(&account) {
                arm.arm_explicit_login();
            }
        }
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
        let decision = match arm {
            Some(SeedArm::Eat) => evaluate_eat_crossing(&current, &capture_snapshot, true),
            Some(arm) => evaluate_poison_arm(&current, &capture_snapshot, arm, &inside_danger),
            None => evaluate_w1_gate(mode, &gate_input),
        };
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
    let mode_name = match (arm, mode) {
        (Some(SeedArm::Eat), _) => "eat_crossing",
        (Some(SeedArm::PoisonStop), _) => "poison_no_food_stop",
        (Some(SeedArm::PoisonCure), _) => "poison_no_food_cure",
        (Some(SeedArm::PoisonWalkOut), _) => "poison_no_food_walkout",
        (None, W1Mode::Crossing) => "crossing",
        (None, W1Mode::StopMidCrossing) => "stop_mid_crossing",
    };
    let proof_name = match arm {
        Some(SeedArm::Eat) => "GUARD-EAT-1",
        Some(SeedArm::PoisonStop | SeedArm::PoisonCure | SeedArm::PoisonWalkOut) => {
            "QUESTER-POISON-D8"
        }
        None => "WALK-GUARD-LIFECYCLE-1_R2",
    };
    let receipt = json!({
        "proof": proof_name,
        "case": "W1",
        "mode": mode_name,
        "outcome": outcome,
        "food": arm == Some(SeedArm::Eat),
        "poison_stop": snapshot.stop.json(),
        "poison_tile": snapshot.poison_tile.map(|tile| [tile.x, tile.z, tile.level]),
        "stand_clicks": stand_clicks(&capture_snapshot.actions)
            .iter()
            .map(|(tick, tile)| json!([tick, tile.x, tile.z, tile.level]))
            .collect::<Vec<_>>(),
        "cure_tick": snapshot.cure_tick,
        "drinks": capture_snapshot.actions.iter()
            .filter(|row| row["op"] == "drink" || row["request"]["action"] == "Drink")
            .cloned()
            .collect::<Vec<_>>(),
        "health": snapshot.health,
        "eat_heals": capture_snapshot.actions.iter()
            .filter(|row| row["kind"] == "guard" && row["op"] == "eat")
            .map(|row| {
                let heal = eat_heal(row, &snapshot.health);
                json!({
                    "click_tick": row["tick"],
                    "heal_tick": heal.map(|(tick, _, _)| tick),
                    "lag_ticks": heal.and_then(|(tick, _, _)| {
                        row["tick"].as_u64().map(|click| u64::from(tick) - click)
                    }),
                    "hp_after": heal.map(|(_, hp, _)| hp),
                    "lobsters_after": heal.map(|(_, _, count)| count),
                })
            }).collect::<Vec<_>>(),
        "hp_drop_observed": snapshot.health.windows(2).any(|rows| rows[1].1 < rows[0].1),
        "real_content": snapshot.real_content,
        "crossing_seed": snapshot.crossing_seed.json(),
        "park_tick": snapshot.park_tick,
        "start_baseline": capture_snapshot.start_baseline,
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
        "{proof_name}_W1_{mode_name}_{account}_{}",
        scenario::shot::stamp_utc(SystemTime::now())
    ));
    let capture_path = scenario::shot::write_shot(
        &capture_dir,
        if outcome == "PASS" {
            "01-final"
        } else {
            "FAIL-final"
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

#[test]
fn walk_guard_fixture_seeds_between_relog_and_stand() {
    // `quester_stage` relogs before the stand teleport (a fresh-account
    // relog resets position); the prayer/hitpoint seeds must stay between
    // them. The removed workaround preserved colour-relog, seeds, stand, so
    // pin that exact between-order on one food and one no-food row: every
    // seed step sits strictly after the colour relog and strictly before
    // the stand, and each row carries only its own seeds.
    const COLOUR_RELOG: &str = "relog so the quest tab colour matches the seeded varp";
    const STAND: &str = "stand at the quest start";
    let start = WorldTile {
        x: 2632,
        z: 3266,
        level: 0,
    };
    for food in [
        None,
        Some(SeedArm::Eat),
        Some(SeedArm::PoisonStop),
        Some(SeedArm::PoisonCure),
    ] {
        let capture = Arc::new(Mutex::new(CombatCapture::default()));
        let scenario = scenario_for(capture, start, food);
        let relog = scenario
            .steps
            .iter()
            .position(|step| step.name == COLOUR_RELOG)
            .expect("walk-guard fixture has a colour relog");
        let stand = scenario
            .steps
            .iter()
            .position(|step| step.name == STAND)
            .expect("walk-guard fixture has a stand step");
        assert!(
            relog < stand,
            "stand teleport must follow the colour relog (food={food:?})"
        );
        // The colour relog is the last relog: a seed between the tutorial
        // relog and the colour relog would not survive to Start.
        assert_eq!(
            scenario
                .steps
                .iter()
                .rposition(|step| matches!(step.kind, StepKind::Relog)),
            Some(relog),
            "the colour relog must be the last relog (food={food:?})"
        );
        let between = |name: &'static str| {
            let hits: Vec<usize> = scenario
                .steps
                .iter()
                .enumerate()
                .filter(|(_, step)| step.name == name)
                .map(|(index, _)| index)
                .collect();
            assert!(
                !hits.is_empty(),
                "walk-guard fixture is missing {name:?} (food={food:?})"
            );
            for hit in hits {
                assert!(
                    relog < hit && hit < stand,
                    "{name:?} at {hit} must sit between the colour relog ({relog}) and the stand ({stand}) (food={food:?})"
                );
            }
        };
        let absent = |name: &'static str| {
            assert!(
                scenario.steps.iter().all(|step| step.name != name),
                "walk-guard fixture must not contain {name:?} (food={food:?})"
            );
        };
        match food {
            None => {
                between("seed Prayer 43");
                absent("seed Prayer 37");
                absent("seed Hitpoints 40");
                absent("seed HP near eat line");
                absent("seed required lobster supply");
            }
            Some(arm) => {
                between("seed Prayer 37");
                between("seed Hitpoints 40");
                between("seed HP near eat line");
                absent("seed Prayer 43");
                if arm == SeedArm::Eat {
                    between("seed required lobster supply");
                } else {
                    absent("seed required lobster supply");
                }
                if arm == SeedArm::PoisonCure {
                    between("seed one antipoison");
                } else {
                    absent("seed one antipoison");
                }
            }
        }
    }
}

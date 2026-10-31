//! Live native-runner poison interruption and resume proofs using real content.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::condition::{Exposure, PoisonState};
use api::interact::{Interactions, OpTarget, SendResult};
use api::snapshot::{AnimationUpdateView, GameSnapshot, WorldTile};
use quester_live::{Cell, LiveProfileControl, Mode, PathCell};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource};
use script::survival::{
    cure::CureWhy,
    math::{NeedWhy, SurvivalNeed},
    Decision, Owner, SurvivalReport,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// m48_50.jm2 places willow loc 1308 (`pack/loc.pack:1309`) at local (16,34),
// world (3088,3234). The stand is the clear land tile east of it (no loc, no
// blocking flag), well inside the Gatherer's 32-tile radius.
const GATHERER_STAND: WorldTile = WorldTile {
    x: 3089,
    z: 3234,
    level: 0,
};
// m48_50.jm2:4851 places willow loc 1308 (`pack/loc.pack:1309`) at local
// (16,34), world (3088,3234). The work stand is one tile east.
const GATHERER_WILLOW_TILE: WorldTile = WorldTile {
    x: 3088,
    z: 3234,
    level: 0,
};
// Draynor's bankbooth loc 2213 (`pack/loc.pack:2214`) is at local (19,42),
// (19,43) and (19,45) in `m48_50.jm2:4979-4982`, hence world (3091,3242),
// (3091,3243) and (3091,3245). The nearest booth is 10 orthogonal tiles from
// GATHERER_STAND: 2 east plus 8 north.
const GATHERER_BANK_BOOTH_TILE: WorldTile = WorldTile {
    x: 3091,
    z: 3242,
    level: 0,
};
const GATHERER_RADIUS: u32 = 32;
const GATHERER_BANK_ONE_WAY_TILES: u32 = 10;
const GATHERER_CURE_BUDGET_TICKS: u32 = 120;
// 20 tile steps for the bank round trip, plus the existing 120-tick cure
// observation budget. All counts are observed game ticks, not wall time.
const GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS: u32 =
    GATHERER_BANK_ONE_WAY_TILES * 2 + GATHERER_CURE_BUDGET_TICKS;
const GATHERER_STARTING_WILLOW_LOGS: i32 = 25;
const GATHERER_INVENTORY_CAPACITY: i32 = 28;
const MINING_STAND: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};
// m51_52.jm2:5202, :5203 and :5234 place copper rock locs 2090, 2090 and 2091
// (`pack/loc.pack:2091-2092`, `copperrock1` and `copperrock2`) at local (25,33),
// (25,35) and (26,34): world (3289,3361), (3289,3363) and (3290,3362) in the
// Varrock east mine. The stand (25,34) has no loc (the loc rows 5201-5204
// skip z 34) and no flag on its map row (`0 25 34: h10 o22 u50`), and the
// three rocks touch it on its north, south and east sides. Copper rocks
// respawn in `10 * (4000 - playercount) / 4000` ticks
// (`skill_mining/configs/mine.dbrow:34`, `scale_by_playercount`), so the
// three rocks keep the Quester mining through the 31-tick login timer wait
// without a step off the stand. A fourth copper rock sits at (3288,3361)
// (`m51_52.jm2:5171`).
const COPPER_STAND: WorldTile = WorldTile {
    x: 3289,
    z: 3362,
    level: 0,
};
const DESERT_CAPTAIN_STAND: WorldTile = WorldTile {
    x: 3270,
    z: 3029,
    level: 0,
};
// m40_148.jm2:4888 places the chest at local (1,35) on level 0.
const TWISTED_CHEST_TILE: WorldTile = WorldTile {
    x: 40 * 64 + 1,
    z: 148 * 64 + 35,
    level: 0,
};

const WOODCUTTING_STAT: i32 = 8;
const MINING_STAT: i32 = 14;
const HITPOINTS_STAT: i32 = 3;
const WILLOW_LOG_ID: i32 = 1519;
const COPPER_ORE_ID: i32 = 436;
const ANTIPOISON_ALIAS: &str = "1doseantipoison";
const RUNE_AXE_ALIAS: &str = "rune_axe";
// content/scripts/skill_firemaking/configs/firemaking.obj:164-166 defines this
// item alias for logs cut from willow trees.
const WILLOW_LOGS_ALIAS: &str = "willow_logs";
const RUNE_PICKAXE_ALIAS: &str = "rune_pickaxe";
const SINISTER_KEY_ALIAS: &str = "sinister_key";
const SINISTER_KEY_ID: i32 = 993;
const TWISTED_CHEST_LOC_ID: i32 = 377;
// agility_dungeon.rs2:113-120 adds nine reward items.
const TWISTED_CHEST_REWARD_COUNT: i32 = 9;
// p_delay(3) resumes at +4 ticks; queue(..., 0) runs on the next eligible
// player queue pass: PlayerOps.ts:376-379, Player.ts:903-912.
const TWISTED_CHEST_POISON_SIGNAL_TICKS: u32 = 5;
// The debugproc's queue(..., 0) runs on the first eligible queue pass;
// cheat_other.rs2:1-3, Player.ts:903-912.
const DEBUGPROC_POISON_SIGNAL_TICKS: u32 = 2;
const DESERT_STAGE_REPLY: &str = "get desertrescue: 3";
const DESERT_CAPTAIN_OPENING_OPTIONS: [&str; 3] =
    ["Wow! A real captain!", "You there!", "Hey ugly!"];
const POISON_CHAT: &str = "You have been poisoned!";
// The Gatherer, Quester-mining and provisioning cells seed poison with this
// Engine L debugproc on the account's own state at the work site
// (`_test/scripts/cheats/cheat_other.rs2:1-3`: `queue(poison_player, 0,
// severity)`), so the engine's own poison script applies its ticks and damage.
// The dialogue cell uses the authored Yanille chest. Both reach this chat
// through `poison.rs2:12-18`.
const POISON_DEBUG_COMMAND: &str = "~poison 27";
const CURE_CHAT: &str = "You drink some of your antipoison potion.";
// Every login arms the poison timer for 30 ticks (`login.rs2:63`,
// `poison.rs2:69-75`); its first fire with `%poison = 0` clears it
// (`poison.rs2:20-23`), and the next observed game tick proves that fire.
// The count starts at the login after the fixture's last relog: the harness
// snapshot restarts its tick at every session (`host/src/lib.rs:1464-1480`,
// `api/src/snapshot.rs:605-632`), and `SessionTicks` records that start.
const TIMER_CLEAR_TICKS: u32 = 31;
// The antipoison effect sets `%poison <= -5` (`anti_poison.rs2:5-6`) but does
// not clear a `poison_damage` already queued by an earlier timer fire (only
// `clear_poison` does, `poison.rs2:64-67`). That queued hit runs on the first
// queue pass at which the player has access (`Player.ts:903-912`,
// `Player.ts:825-831`). The drink delays the player with `p_delay(1)`
// (`consume_effects.rs2:124-126`, `consume_messages.dbrow:651`), which
// resumes two ticks later (`PlayerOps.ts:377-378`, `World.ts:692`). So at most
// one poison hit can land after the drink line, and no later than this.
const POST_CURE_QUEUED_HIT_TICKS: u32 = 2;
// Runner bound from the poison line to the drink line (the drink bound of
// three observed ticks plus the drink's own landing; design A6.5).
const CURE_LINE_BOUND_TICKS: u32 = 10;
const Q_UNKNOWN: u8 = 11;
const LETHAL_OUTLOOK: u16 = 330;
// The Base40 qualification profile seeds only the combat skills and Prayer
// (`scenario/src/quester.rs:626-635`) after `minme`, so each cell seeds its
// work skill itself. 41 is the rune pickaxe's `levelrequire`
// (`skill_mining/configs/pickaxes.obj:197`, checked by
// `skill_mining/scripts/pickaxe_checker.rs2:4`); willow needs Woodcutting 30
// (`skill_woodcutting/configs/trees.dbrow:103`) and 289 axes have no
// Woodcutting requirement (`skill_woodcutting/scripts/woodcut.rs2:161-183`).
const WORK_SKILL_LEVEL: i32 = 41;

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Woodcutting,
    Mining,
    Dialogue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PoisonSource {
    TwistedChest,
    AccountStateDebugProc,
}

/// Observed game ticks of the current session, in the harness snapshot's own
/// base. That snapshot restarts its tick at every logout and login
/// (`host::publish_snapshot`, `GameSnapshot::reset_session`), so every tick the
/// witness keeps is read from it, never from the scenario runner's snapshot,
/// whose tick runs on across relogs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SessionTicks {
    was_ingame: bool,
    /// The latest in-game tick; `None` while logged out.
    now: Option<u32>,
    /// The first in-game tick of the latest session.
    login: Option<u32>,
    logins: u32,
}

impl SessionTicks {
    fn observe(&mut self, ingame: bool, tick: u32) {
        if !ingame {
            self.was_ingame = false;
            self.now = None;
            return;
        }
        // A drain can carry both sides of a reconnect: the snapshot then
        // restarts its tick without an observed logged-out frame.
        if !self.was_ingame || self.now.is_some_and(|last| tick < last) {
            self.login = Some(tick);
            self.logins = self.logins.saturating_add(1);
        }
        self.was_ingame = true;
        self.now = Some(tick);
    }

    /// True once the latest login's poison timer has fired and cleared.
    fn login_timer_cleared(&self) -> bool {
        matches!((self.now, self.login), (Some(now), Some(login)) if now.saturating_sub(login) >= TIMER_CLEAR_TICKS)
    }
}

/// One poison hitmark event. The client keeps a mark in its slot until the
/// slot's cycle passes, so one event spans several frames; a new hit writes a
/// new cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoisonMark {
    tick: u32,
    slot: usize,
    cycle: i32,
    value: i32,
}

fn record_poison_marks(
    hitmarks: &api::snapshot::HitmarksView,
    tick: u32,
    marks: &mut Vec<PoisonMark>,
) {
    for (slot, mark) in hitmarks.marks.iter().enumerate() {
        if mark.kind != 2 || mark.value <= 0 || mark.cycle <= hitmarks.loop_cycle {
            continue;
        }
        if marks
            .iter()
            .any(|seen| seen.slot == slot && seen.cycle == mark.cycle)
        {
            continue;
        }
        marks.push(PoisonMark {
            tick,
            slot,
            cycle: mark.cycle,
            value: mark.value,
        });
    }
}

/// The content's poison-damage rule for a cell whose poison came after the
/// login timer cleared. A fresh timer fires 30 ticks after the line
/// (`poison.rs2:72-75`), so no hit may precede the drink line. After the drink
/// line, at most one hit may land: the one an earlier fire queued, within
/// `POST_CURE_QUEUED_HIT_TICKS`. Valid on every frame.
fn check_poison_marks(marks: &[PoisonMark], cure_tick: Option<u32>) -> Result<(), String> {
    let mut after_cure = 0;
    for mark in marks {
        match cure_tick {
            Some(cure) if mark.tick > cure => {
                after_cure += 1;
                if after_cure > 1 || mark.tick - cure > POST_CURE_QUEUED_HIT_TICKS {
                    return Err(format!(
                        "poison damage continued past the content bound after the antipoison drink: marks {marks:?}, cure tick {cure}, bound one hit within {POST_CURE_QUEUED_HIT_TICKS} ticks"
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "poison damage before the antipoison drink line: marks {marks:?}, cure tick {cure_tick:?}"
                ));
            }
        }
    }
    Ok(())
}

/// The whole cure outcome once the first scheduled fire is covered: the
/// drink line came inside the runner bound, and the damage obeyed the
/// content's rule through the first scheduled fire after the line.
fn check_cure_outcome(
    poison_tick: Option<u32>,
    cure_tick: Option<u32>,
    marks: &[PoisonMark],
    observed_tick: u32,
) -> Result<(), String> {
    let poison = poison_tick.ok_or("real poison chat was not observed")?;
    check_poison_marks(marks, cure_tick)?;
    let cure = cure_tick.ok_or_else(|| {
        format!("antipoison drink line was not observed: poison tick {poison}, observed {observed_tick}")
    })?;
    if cure < poison || cure - poison > CURE_LINE_BOUND_TICKS {
        return Err(format!(
            "antipoison drink line outside {CURE_LINE_BOUND_TICKS} ticks of the poison line: poison {poison}, cure {cure}"
        ));
    }
    if observed_tick.saturating_sub(poison) < TIMER_CLEAR_TICKS {
        return Err(format!(
            "the first scheduled poison fire was not covered: poison {poison}, observed {observed_tick}"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BankTripAntipoisonFrame {
    tick: u32,
    count: i32,
}

struct Witness {
    activity: Activity,
    poison_source: PoisonSource,
    expected_key_at_start: i32,
    expected_inventory_extra: i32,
    expected_resource_at_start: i32,
    poison_during_provisioning: bool,
    skill_name: &'static str,
    skill_stat: i32,
    resource_id: Option<i32>,
    tool_id: Option<i32>,
    antipoison_id: i32,
    stand: WorldTile,
    desert_stage_reply: Option<String>,
    desert_stage_reply_tick: Option<u32>,
    baseline_chat: Option<i32>,
    baseline_xp: i32,
    baseline_resource: i32,
    baseline_tool: i32,
    expected_tool_at_start: i32,
    key_id: i32,
    baseline_key: i32,
    baseline_antipoison: i32,
    baseline_animation_serial: Option<u32>,
    session: SessionTicks,
    timer_phase_ready_tick: Option<u32>,
    provisioning_tick: Option<u32>,
    work_active_tick: Option<u32>,
    work_animation_tick: Option<u32>,
    work_animation: Option<AnimationUpdateView>,
    wait_for_gatherer_bank_trip: bool,
    bank_trip_start_tick: Option<u32>,
    bank_trip_inventory_at_start: Option<i32>,
    bank_trip_bank_open_tick: Option<u32>,
    bank_trip_deposit_tick: Option<u32>,
    bank_trip_willow_logs_at_deposit: Option<i32>,
    bank_trip_willow_logs_deposited: Option<i32>,
    bank_trip_last_willow_logs: Option<i32>,
    bank_trip_xp_at_deposit: Option<i32>,
    bank_trip_work_animation_tick: Option<u32>,
    bank_trip_work_animation: Option<AnimationUpdateView>,
    /// The bank interface state observed on the frame of the resumed work.
    bank_trip_bank_closed_at_work: Option<bool>,
    bank_trip_antipoison_frames: Vec<BankTripAntipoisonFrame>,
    poison_source_tick: Option<u32>,
    poison_seed_bank_open: Option<bool>,
    poison_seed_tool_count: Option<i32>,
    poison_source_modal: Option<i32>,
    poison_tick: Option<u32>,
    poison_marks: Vec<PoisonMark>,
    need_tick: Option<u32>,
    need_xp: Option<i32>,
    need_resource: Option<i32>,
    need_status: Option<String>,
    need_phase: Option<String>,
    need_step: Option<String>,
    need_report_decision: Option<String>,
    need_report_why: Option<String>,
    need_report: Option<String>,
    need_report_owner: Option<String>,
    need_condition_tick: Option<u32>,
    need_condition_poison: Option<String>,
    need_unlatched_poison: Option<bool>,
    need_outlook_total: Option<u16>,
    need_outlook_per_fire: Option<u8>,
    need_outlook_pending: Option<bool>,
    need_hp: Option<u8>,
    need_hp_max: Option<u8>,
    need_food: Option<u8>,
    need_antipoison: Option<u8>,
    need_in_combat: Option<bool>,
    need_no_hit_at_edge: Option<bool>,
    need_page_open_tick: Option<u32>,
    need_page_options: Vec<String>,
    cure_tick: Option<u32>,
    dose_drop_tick: Option<u32>,
    drink_decision_tick: Option<u32>,
    drink_decision_modal: Option<i32>,
    settled_tick: Option<u32>,
    immune_tick: Option<u32>,
    dialogue_page_tick: Option<u32>,
    profile_change_tick: Option<u32>,
    profile_change_modal: Option<i32>,
    profile_change_options: Vec<String>,
    dialogue_closed_tick: Option<u32>,
    dialogue_reopened_tick: Option<u32>,
    dialogue_reopened_options: Vec<String>,
    dialogue_reopened_step: Option<String>,
    /// The seeded Hitpoints level the start proof expects (`setstat`).
    expected_hp: i32,
    /// Antipoison doses the start proof expects in the pack.
    expected_antipoison: i32,
    /// The seeded food item and its count at Start.
    food_id: Option<i32>,
    expected_food_at_start: i32,
    /// The Engine L debug procedures the poison step sends, in order.
    poison_commands: Vec<&'static str>,
    /// The first lifecycle receipt the harness reported, if any.
    lifecycle_seen: Option<String>,
    track: SurvivalTrack,
}

fn new_witness(
    activity: Activity,
    skill_name: &'static str,
    skill_stat: i32,
    resource_id: Option<i32>,
    tool_alias: Option<&str>,
    poison_source: PoisonSource,
    stand: WorldTile,
) -> Arc<Mutex<Witness>> {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected R289 data");
    let item_id = |alias: &str| {
        selected
            .item_by_alias(alias)
            .unwrap_or_else(|| panic!("missing selected item alias {alias}"))
            .id
    };
    let key_id = item_id(SINISTER_KEY_ALIAS);
    let (expected_key_at_start, expected_inventory_extra) = match (poison_source, activity) {
        (PoisonSource::TwistedChest, Activity::Dialogue) => (0, TWISTED_CHEST_REWARD_COUNT),
        (PoisonSource::TwistedChest, _) => (1, 0),
        (PoisonSource::AccountStateDebugProc, _) => (0, 0),
    };
    let resource_id = resource_id.inspect(|&id| {
        selected
            .item_by_id(id)
            .unwrap_or_else(|| panic!("missing selected resource item id {id}"));
    });
    Arc::new(Mutex::new(Witness {
        activity,
        poison_source,
        expected_key_at_start,
        expected_inventory_extra,
        expected_resource_at_start: 0,
        poison_during_provisioning: false,
        skill_name,
        skill_stat,
        resource_id,
        tool_id: tool_alias.map(item_id),
        antipoison_id: item_id(ANTIPOISON_ALIAS),
        key_id,
        stand,
        desert_stage_reply: None,
        desert_stage_reply_tick: None,
        baseline_chat: None,
        baseline_xp: 0,
        baseline_resource: 0,
        baseline_tool: 0,
        expected_tool_at_start: i32::from(tool_alias.is_some()),
        baseline_antipoison: 0,
        baseline_key: 0,
        baseline_animation_serial: None,
        session: SessionTicks::default(),
        timer_phase_ready_tick: None,
        provisioning_tick: None,
        work_active_tick: None,
        work_animation_tick: None,
        work_animation: None,
        wait_for_gatherer_bank_trip: false,
        bank_trip_start_tick: None,
        bank_trip_inventory_at_start: None,
        bank_trip_bank_open_tick: None,
        bank_trip_deposit_tick: None,
        bank_trip_willow_logs_at_deposit: None,
        bank_trip_willow_logs_deposited: None,
        bank_trip_last_willow_logs: None,
        bank_trip_xp_at_deposit: None,
        bank_trip_work_animation_tick: None,
        bank_trip_work_animation: None,
        bank_trip_bank_closed_at_work: None,
        bank_trip_antipoison_frames: Vec::new(),
        poison_source_tick: None,
        poison_seed_bank_open: None,
        poison_seed_tool_count: None,
        poison_source_modal: None,
        poison_tick: None,
        poison_marks: Vec::new(),
        need_tick: None,
        need_xp: None,
        need_resource: None,
        need_status: None,
        need_phase: None,
        need_step: None,
        need_report_decision: None,
        need_report_why: None,
        need_report: None,
        need_report_owner: None,
        need_condition_tick: None,
        need_condition_poison: None,
        need_unlatched_poison: None,
        need_outlook_total: None,
        need_outlook_per_fire: None,
        need_outlook_pending: None,
        need_hp: None,
        need_hp_max: None,
        need_food: None,
        need_antipoison: None,
        need_in_combat: None,
        need_no_hit_at_edge: None,
        need_page_open_tick: None,
        need_page_options: Vec::new(),
        cure_tick: None,
        dose_drop_tick: None,
        drink_decision_tick: None,
        drink_decision_modal: None,
        settled_tick: None,
        immune_tick: None,
        dialogue_page_tick: None,
        profile_change_tick: None,
        profile_change_modal: None,
        profile_change_options: Vec::new(),
        dialogue_closed_tick: None,
        dialogue_reopened_tick: None,
        dialogue_reopened_options: Vec::new(),
        dialogue_reopened_step: None,
        expected_hp: 30,
        expected_antipoison: 1,
        food_id: None,
        expected_food_at_start: 0,
        poison_commands: vec![POISON_DEBUG_COMMAND],
        lifecycle_seen: None,
        track: SurvivalTrack::default(),
    }))
}

fn status_text<'a>(status: Option<&'a ScriptStatus>, key: &str) -> Option<&'a str> {
    status?.fields.iter().find_map(|field| match &field.value {
        StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
        _ => None,
    })
}

fn stat_xp(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.xp)
}

fn stat_base(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.base)
}

fn latest_chat(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0)
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot.inv_count(id)
}

fn inventory_total(snapshot: &GameSnapshot) -> i32 {
    snapshot.inv().iter().map(|(_, count)| *count).sum()
}

fn active_hitmark(snapshot: &GameSnapshot, kind: Option<i32>) -> Option<bool> {
    let hitmarks = snapshot.hitmarks()?;
    Some(hitmarks.marks.iter().any(|mark| {
        kind.is_none_or(|kind| mark.kind == kind)
            && mark.value > 0
            && mark.cycle > hitmarks.loop_cycle
    }))
}

fn in_combat(snapshot: &GameSnapshot) -> Option<bool> {
    snapshot
        .local_player()
        .map(|player| player.player.actor.in_combat)
}

fn insert_before_start(scenario: &mut scenario::Scenario, steps: Vec<scenario::Step>) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    scenario.steps.splice(start..start, steps);
}

fn fixture(
    activity: Activity,
    label: &str,
    stand: WorldTile,
    extra_items: &[(&str, i32)],
    seed_vars: &[(&str, i32)],
    before_relog: Vec<scenario::Step>,
    mode: Mode,
) -> Cell {
    fixture_with_hp(
        activity,
        label,
        stand,
        extra_items,
        seed_vars,
        before_relog,
        mode,
        30,
    )
}

/// The Base40 qualification profile's Hitpoints level, observed by the
/// fixture's profile proof before the cell seeds its own level.
const PROFILE_HITPOINTS: i32 = 40;

#[expect(
    clippy::too_many_arguments,
    reason = "the fixture's account-owned seeds stay explicit at each cell"
)]
fn fixture_with_hp(
    activity: Activity,
    label: &str,
    stand: WorldTile,
    extra_items: &[(&str, i32)],
    seed_vars: &[(&str, i32)],
    before_relog: Vec<scenario::Step>,
    mode: Mode,
    hitpoints: i32,
) -> Cell {
    assert_ne!(
        hitpoints, PROFILE_HITPOINTS,
        "the seeded level must differ from the profile's to prove the setstat landed"
    );
    let mut cell = quester_live::path_cell(PathCell {
        quest: "cook",
        display: "Cook's Assistant",
        label: label.to_owned(),
        stage: Some("cook:0"),
        loadout: None,
        extra_items,
        seed_vars,
        stand,
        mode,
        before_relog,
    })
    .expect("build account-owned live fixture");

    let mut before_start = Vec::new();
    if activity == Activity::Dialogue {
        before_start.push(scenario::Step {
            name: "prove the Desert Rescue stage from the server",
            kind: scenario::StepKind::Perform {
                send: Box::new(|client, _| {
                    api::interact::cheat(client, "getvar desertrescue").is_sent()
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::FreshChat {
                    needle: DESERT_STAGE_REPLY,
                },
                budget_ticks: 3,
            },
        });
    }
    let (work_skill, work_stat) = match activity {
        Activity::Woodcutting => ("woodcutting", WOODCUTTING_STAT),
        Activity::Mining | Activity::Dialogue => ("mining", MINING_STAT),
    };
    let work_command = format!("setstat {work_skill} {WORK_SKILL_LEVEL}");
    before_start.push(scenario::Step {
        name: "seed the work skill after the fixture profile proof",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &work_command).is_sent()),
        },
        wait: scenario::Wait {
            // `minme` left the skill at 1, so this level proves the
            // `setstat` landed (Player.ts:1904-1909 sets base and current).
            arm: scenario::Proof::Stat {
                id: work_stat,
                min: WORK_SKILL_LEVEL,
            },
            budget_ticks: 10,
        },
    });
    let hitpoints_command = format!("setstat hitpoints {hitpoints}");
    before_start.push(scenario::Step {
        name: "seed the cell's Hitpoints level after the fixture profile proof",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| {
                api::interact::cheat(client, &hitpoints_command).is_sent()
            }),
        },
        wait: scenario::Wait {
            // The profile proof just observed Hitpoints 40, so only the
            // landed `setstat` (Player.ts:1904-1909 sets base and current)
            // satisfies a bound on the other side of 40.
            arm: if hitpoints < PROFILE_HITPOINTS {
                scenario::Proof::StatAtMost {
                    id: HITPOINTS_STAT,
                    max: hitpoints,
                }
            } else {
                scenario::Proof::Stat {
                    id: HITPOINTS_STAT,
                    min: hitpoints,
                }
            },
            budget_ticks: 10,
        },
    });
    insert_before_start(&mut cell.scenario, before_start);
    cell
}

fn bank_seed_step(alias: &'static str) -> scenario::Step {
    let command = format!("givebank {alias} 1");
    scenario::Step {
        name: "seed the rune pickaxe in the account bank before fixture relog",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 3,
        },
    }
}

fn add_login_timer_wait_before_start(
    scenario: &mut scenario::Scenario,
    witness: Arc<Mutex<Witness>>,
) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    let wait_witness = Arc::clone(&witness);
    let wait = scenario::Step {
        name: "wait for the 31-tick login poison timer to clear before provisioning",
        kind: scenario::StepKind::Await {
            evidence:
                "31 observed ticks after the final login; login.rs2:63, poison.rs2:20-23,69-75",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame()
                    || snapshot.scene_state() != 2
                    || snapshot.local_player().is_none()
                {
                    return false;
                }
                // The runner's snapshot keeps counting across relogs; the
                // session ticks come from the harness snapshot.
                wait_witness
                    .lock()
                    .expect("survival witness")
                    .session
                    .login_timer_cleared()
            }),
            summary: None,
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Arrived {
                x: witness.lock().expect("survival witness").stand.x,
                z: witness.lock().expect("survival witness").stand.z,
                level: witness.lock().expect("survival witness").stand.level,
            },
            // The witness reads the harness snapshot one loop pass after the
            // runner, so it can see the 31st tick one observed tick later.
            budget_ticks: TIMER_CLEAR_TICKS + 1,
        },
    };
    scenario.steps.splice(start..start, [wait]);
}

fn install_path_source(evidence: &std::path::Path, name: &str, steps: Value) -> SourceRestore {
    let folder = evidence.join("native-cure-path").join(name);
    std::fs::create_dir_all(&folder).expect("create isolated path source");
    let mut document: Value = serde_json::from_str(COOK_PATH_JSON).expect("embedded Cook path");
    document["roles"][0]["sequences"][0]["steps"] = steps;
    std::fs::write(
        folder.join("index.json"),
        br#"{"schema":1,"paths":[{"id":"cook","file":"cook.json"}]}"#,
    )
    .expect("write isolated path index");
    std::fs::write(
        folder.join("cook.json"),
        serde_json::to_vec_pretty(&document).expect("serialize live Path"),
    )
    .expect("write isolated Cook Path");
    let previous = registry::source();
    registry::set_source(FolderSource {
        enabled: true,
        folder,
    });
    SourceRestore(previous)
}

fn add_poison_steps(
    scenario: &mut scenario::Scenario,
    witness: Arc<Mutex<Witness>>,
    before_start: bool,
    budget_ticks: u32,
) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    let phase_witness = Arc::clone(&witness);
    let phase_arm = {
        let witness = witness.lock().expect("survival witness");
        if witness.poison_during_provisioning {
            scenario::Proof::BankOpen
        } else {
            scenario::Proof::Arrived {
                x: witness.stand.x,
                z: witness.stand.z,
                level: witness.stand.level,
            }
        }
    };
    let phase = scenario::Step {
        name: "wait for cleared login poison timer and authored activity readiness",
        kind: scenario::StepKind::Await {
            evidence: "31 observed ticks after the final login and authored activity readiness",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame()
                    || snapshot.scene_state() != 2
                    || snapshot.local_player().is_none()
                {
                    return false;
                }
                let mut witness = phase_witness.lock().expect("survival witness");
                if witness.baseline_chat.is_none() {
                    witness.baseline_chat = Some(latest_chat(snapshot));
                }
                let Some(now) = witness.session.now else {
                    return false;
                };
                let active = if witness.poison_during_provisioning {
                    witness.provisioning_tick.is_some()
                } else if witness.wait_for_gatherer_bank_trip {
                    witness.bank_trip_work_animation_tick.is_some()
                } else {
                    match witness.activity {
                        Activity::Woodcutting => {
                            stat_xp(snapshot, witness.skill_stat) > witness.baseline_xp
                        }
                        Activity::Mining => witness.work_animation.is_some_and(|first| {
                            // Any current mining animation is the work signal. The
                            // serial changes at every swing, so the first swing's
                            // serial cannot stay current through the 31-tick wait.
                            snapshot
                                .local_player()
                                .and_then(|player| player.animation_update)
                                .is_some_and(|current| {
                                    current.sequence >= 0 && current.sequence == first.sequence
                                })
                        }),
                        Activity::Dialogue => true,
                    }
                };
                if active
                    && witness.activity != Activity::Dialogue
                    && !witness.poison_during_provisioning
                {
                    witness.work_active_tick.get_or_insert(now);
                }
                if active && witness.session.login_timer_cleared() {
                    witness.timer_phase_ready_tick = Some(now);
                    true
                } else {
                    false
                }
            }),
            summary: None,
        },
        wait: scenario::Wait {
            arm: phase_arm,
            budget_ticks,
        },
    };
    let source = witness.lock().expect("survival witness").poison_source;
    let mut inserted = vec![phase];
    match source {
        PoisonSource::TwistedChest => {
            inserted.extend(twisted_chest_steps(Arc::clone(&witness), before_start));
        }
        PoisonSource::AccountStateDebugProc => {
            inserted.extend(account_state_poison_steps(Arc::clone(&witness)));
        }
    }
    let insertion = if before_start { start } else { start + 1 };
    scenario.steps.splice(insertion..insertion, inserted);
}

fn add_poison_steps_after_gatherer_bank_trip(
    scenario: &mut scenario::Scenario,
    witness: Arc<Mutex<Witness>>,
    budget_ticks: u32,
) {
    witness
        .lock()
        .expect("survival witness")
        .wait_for_gatherer_bank_trip = true;
    add_poison_steps(scenario, witness, false, budget_ticks);
}

fn teleport_step(name: &'static str, tile: WorldTile) -> scenario::Step {
    let command = api::interact::tele_args(tile.level, tile.x, tile.z);
    scenario::Step {
        name,
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Arrived {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            },
            budget_ticks: 10,
        },
    }
}

fn twisted_chest_steps(witness: Arc<Mutex<Witness>>, before_start: bool) -> Vec<scenario::Step> {
    let mut steps = vec![
        teleport_step(
            "teleport to the real Yanille twisted chest",
            TWISTED_CHEST_TILE,
        ),
        scenario::Step {
            name: "observe the real Yanille twisted chest",
            kind: scenario::StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::LocIdNear {
                    id: TWISTED_CHEST_LOC_ID,
                    x: TWISTED_CHEST_TILE.x,
                    z: TWISTED_CHEST_TILE.z,
                    level: TWISTED_CHEST_TILE.level,
                    radius: 2,
                },
                // The world map places the authored object on this tile;
                // agility_dungeon.rs2:122 only changes its inactive stage.
                budget_ticks: 10,
            },
        },
    ];
    let source_witness = Arc::clone(&witness);
    steps.push(scenario::Step {
        name: "use the account-owned sinister key on the real chest",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                if snapshot.modals().chat != -1 || active_hitmark(snapshot, None) != Some(false) {
                    return false;
                }
                let Some(loc) = snapshot
                    .locs()
                    .iter()
                    .find(|loc| loc.id == TWISTED_CHEST_LOC_ID)
                else {
                    return false;
                };
                let Some(key) = snapshot
                    .inventory()
                    .iter()
                    .find(|item| item.def.id == SINISTER_KEY_ID && item.count > 0)
                else {
                    return false;
                };
                if !matches!(
                    Interactions::new(snapshot, client).use_item_on(key, OpTarget::Loc(loc)),
                    SendResult::Sent { .. }
                ) {
                    return false;
                }
                let mut witness = source_witness.lock().expect("survival witness");
                witness.poison_source_tick = witness.session.now;
                witness.poison_source_modal = Some(snapshot.modals().chat);
                true
            }),
        },
        wait: scenario::Wait {
            // agility_dungeon.rs2:105 resumes after p_delay(3); the poison
            // queue adds at most one tick (PlayerOps.ts:376-379, Player.ts:903-912).
            arm: scenario::Proof::FreshChat {
                needle: POISON_CHAT,
            },
            budget_ticks: TWISTED_CHEST_POISON_SIGNAL_TICKS,
        },
    });
    if !before_start {
        steps.push(scenario::Step {
            name: "observe the antipoison drink before returning to work",
            kind: scenario::StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::FreshChat { needle: CURE_CHAT },
                budget_ticks: 10,
            },
        });
    }
    let stand = witness.lock().expect("survival witness").stand;
    steps.push(teleport_step(
        if before_start {
            "return to the Desert Captain before Start"
        } else {
            "return to the authored work site after cure"
        },
        stand,
    ));
    steps
}

fn account_state_poison_steps(witness: Arc<Mutex<Witness>>) -> Vec<scenario::Step> {
    let source_witness = Arc::clone(&witness);
    let name = if witness
        .lock()
        .expect("survival witness")
        .poison_during_provisioning
    {
        "seed account-owned poison state while the Quester bank is open"
    } else {
        "seed account-owned poison state at the work site during active work"
    };
    vec![scenario::Step {
        name,
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut witness = source_witness.lock().expect("survival witness");
                let Some(now) = witness.session.now else {
                    return false;
                };
                let phase_ready = if witness.poison_during_provisioning {
                    witness.provisioning_tick.is_some()
                        && snapshot.bank_component_id() >= 0
                        && snapshot.bank_loaded()
                        && witness
                            .tool_id
                            .is_some_and(|id| item_count(snapshot, id) == witness.baseline_tool)
                } else {
                    // The work phase above proved active work at the stand.
                    witness.work_active_tick.is_some()
                };
                if witness.timer_phase_ready_tick.is_none()
                    || !phase_ready
                    || !witness.session.login_timer_cleared()
                    || snapshot.modals().chat != -1
                    || active_hitmark(snapshot, None) != Some(false)
                {
                    return false;
                }
                // Every command goes out in this one client cycle; the cell
                // that sends two records the frames that show their effects.
                let commands = witness.poison_commands.clone();
                if !commands
                    .iter()
                    .all(|command| api::interact::cheat(client, command).is_sent())
                {
                    return false;
                }
                witness.poison_seed_bank_open =
                    Some(snapshot.bank_component_id() >= 0 && snapshot.bank_loaded());
                witness.poison_seed_tool_count = witness.tool_id.map(|id| item_count(snapshot, id));
                witness.poison_source_tick = Some(now);
                witness.poison_source_modal = Some(snapshot.modals().chat);
                true
            }),
        },
        wait: scenario::Wait {
            // cheat_other.rs2:1-3 queues poison with delay 0; the signal is
            // emitted by poison.rs2:12-18 on the first eligible queue pass.
            arm: scenario::Proof::FreshChat {
                needle: POISON_CHAT,
            },
            budget_ticks: DEBUGPROC_POISON_SIGNAL_TICKS,
        },
    }]
}

fn capture_active_work(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &mut Witness,
) {
    match witness.activity {
        Activity::Woodcutting => {
            if stat_xp(snapshot, witness.skill_stat) > witness.baseline_xp {
                witness.work_active_tick.get_or_insert(snapshot.tick());
            }
        }
        Activity::Mining => {
            if status.is_some_and(|status| status.phase == NativePhase::Working)
                && status_text(status, "step_id") == Some("survival-work")
            {
                if let Some(animation) = snapshot
                    .local_player()
                    .and_then(|player| player.animation_update)
                    .filter(|animation| {
                        animation.sequence >= 0
                            && Some(animation.serial) != witness.baseline_animation_serial
                    })
                {
                    witness.work_active_tick.get_or_insert(snapshot.tick());
                    if witness.work_animation.is_none() {
                        witness.work_animation_tick = Some(snapshot.tick());
                        witness.work_animation = Some(animation);
                    }
                }
            }
        }
        Activity::Dialogue => {}
    }
}

fn capture_gatherer_bank_trip(snapshot: &GameSnapshot, witness: &mut Witness) {
    if !witness.wait_for_gatherer_bank_trip
        || witness.baseline_antipoison != 1
        || witness.baseline_resource != witness.expected_resource_at_start
        || witness.bank_trip_work_animation_tick.is_some()
    {
        return;
    }
    let Some(resource_id) = witness.resource_id else {
        return;
    };
    let now = snapshot.tick();
    let willow_logs = item_count(snapshot, resource_id);
    let inventory = inventory_total(snapshot);
    if witness.bank_trip_start_tick.is_none() {
        if willow_logs <= witness.baseline_resource || inventory != GATHERER_INVENTORY_CAPACITY {
            return;
        }
        witness.bank_trip_start_tick = Some(now);
        witness.bank_trip_inventory_at_start = Some(inventory);
    }

    witness
        .bank_trip_antipoison_frames
        .push(BankTripAntipoisonFrame {
            tick: now,
            count: item_count(snapshot, witness.antipoison_id),
        });
    let bank_open = snapshot.bank_component_id() >= 0 && snapshot.bank_loaded();
    if bank_open {
        witness.bank_trip_bank_open_tick.get_or_insert(now);
    }
    let previous_willow_logs = witness.bank_trip_last_willow_logs;
    if witness.bank_trip_deposit_tick.is_none() && bank_open && willow_logs == 0 {
        witness.bank_trip_deposit_tick = Some(now);
        witness.bank_trip_willow_logs_at_deposit = Some(willow_logs);
        witness.bank_trip_willow_logs_deposited = previous_willow_logs;
        witness.bank_trip_xp_at_deposit = Some(stat_xp(snapshot, witness.skill_stat));
    }
    witness.bank_trip_last_willow_logs = Some(willow_logs);

    if let (Some(deposit_tick), Some(xp_at_deposit)) = (
        witness.bank_trip_deposit_tick,
        witness.bank_trip_xp_at_deposit,
    ) {
        // Woodcutting XP after the deposit proves the resumed work; the
        // Gatherer may chop the same willow from another adjacent tile or a
        // nearer willow in its radius, so the tile is not part of the proof.
        // The bank state on that frame goes to the receipt as observed.
        if now > deposit_tick && stat_xp(snapshot, witness.skill_stat) > xp_at_deposit {
            if let Some(animation) = snapshot
                .local_player()
                .and_then(|player| player.animation_update)
                .filter(|animation| animation.sequence >= 0)
            {
                witness.bank_trip_work_animation_tick = Some(now);
                witness.bank_trip_work_animation = Some(animation);
                witness.bank_trip_bank_closed_at_work = Some(!bank_open);
            }
        }
    }
}

/// The step owner's report of a typed lethal cure need, in either state that
/// holds it. `Needed` is the report while the onset latch is open; `Drinking`
/// is the report when the latch closed at the line's own edge and the bot
/// goes straight to the drink (design A6.5 item 3, A1.4 item 6).
fn lethal_cure_report(report: &SurvivalReport) -> bool {
    matches!(report.decision, Decision::Needed | Decision::Drinking)
        && report.why == Some(CureWhy::Lethal)
        && report.owner == Owner::Step
        && matches!(
            report.need,
            Some(SurvivalNeed::Cure {
                why: NeedWhy::Lethal,
                ..
            })
        )
}

fn typed_lethal_need(slot: &host_play::SlotStatus) -> bool {
    lethal_cure_report(&slot.survival)
}

fn capture_need(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    slot: &host_play::SlotStatus,
    witness: &mut Witness,
) -> bool {
    let Some(need_text) = status_text(status, "survival")
        .filter(|value| value.contains("poisoned: would die; needs antipoison"))
    else {
        return false;
    };
    if witness.poison_tick.is_none() || !typed_lethal_need(slot) {
        return false;
    }
    let report = slot.survival;
    let condition = slot.condition;
    let now = condition.tick as u16;
    let outlook =
        api::condition::poison_math::remaining(condition.poison, Exposure::Unknown, now, Q_UNKNOWN);
    let unlatched_poison = matches!(
        condition.poison,
        PoisonState::Onset {
            q: None,
            hit_since_line: false,
            ..
        }
    );
    let no_hit_at_edge = active_hitmark(snapshot, None).is_some_and(|active| !active);
    let is_combat = in_combat(snapshot);
    if witness.need_tick.is_none() {
        witness.need_tick = Some(snapshot.tick());
        witness.need_xp = Some(stat_xp(snapshot, witness.skill_stat));
        witness.need_resource = witness.resource_id.map(|id| item_count(snapshot, id));
        witness.need_status = Some(need_text.to_owned());
        witness.need_phase = status_text(status, "phase")
            .map(str::to_owned)
            .or_else(|| status.map(|status| format!("{:?}", status.phase).to_lowercase()));
        witness.need_step = status_text(status, "step_id").map(str::to_owned);
        witness.need_report_decision = Some(format!("{:?}", report.decision));
        witness.need_report_why = Some(format!("{:?}", report.why));
        witness.need_report = Some(format!("{:?}", report.need));
        witness.need_report_owner = Some(format!("{:?}", report.owner));
        witness.need_condition_tick = Some(condition.tick);
        witness.need_condition_poison = Some(format!("{:?}", condition.poison));
        witness.need_unlatched_poison = Some(unlatched_poison);
        witness.need_outlook_total = Some(outlook.total);
        witness.need_outlook_per_fire = Some(outlook.per_fire_now);
        witness.need_outlook_pending = Some(outlook.pending);
        witness.need_hp = Some(condition.hp);
        witness.need_hp_max = Some(condition.hp_max);
        witness.need_food = Some(condition.food.ordinary_count);
        witness.need_antipoison = Some(condition.cures.antipoison);
        witness.need_in_combat = is_combat;
        witness.need_no_hit_at_edge = Some(no_hit_at_edge);
    }
    if witness.activity == Activity::Dialogue
        && snapshot.modals().chat != -1
        && !snapshot.chat_options().is_empty()
        && witness.need_page_open_tick.is_none()
    {
        witness.dialogue_page_tick = Some(snapshot.tick());
        witness.need_page_open_tick = Some(snapshot.tick());
        witness.need_page_options = snapshot
            .chat_options()
            .iter()
            .map(|option| option.text.clone())
            .collect();
    }
    typed_lethal_need(slot)
}

fn observe_frame(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    slot: Option<&host_play::SlotStatus>,
    witness: &mut Witness,
) -> bool {
    if witness.activity == Activity::Dialogue && witness.desert_stage_reply.is_none() {
        if let Some(line) = snapshot
            .chat_lines()
            .iter()
            .find(|line| line.text == DESERT_STAGE_REPLY)
        {
            witness.desert_stage_reply = Some(line.text.clone());
            witness.desert_stage_reply_tick = Some(snapshot.tick());
        }
    }
    if witness.baseline_chat.is_none() && snapshot.ingame() {
        witness.baseline_chat = Some(latest_chat(snapshot));
    }
    witness.session.observe(snapshot.ingame(), snapshot.tick());
    if witness.poison_during_provisioning
        && witness.provisioning_tick.is_none()
        && status_text(status, "provision") == Some("Withdrawing")
        && snapshot.bank_component_id() >= 0
        && snapshot.bank_loaded()
        && witness
            .tool_id
            .is_some_and(|id| item_count(snapshot, id) == witness.baseline_tool)
    {
        witness.provisioning_tick = Some(snapshot.tick());
    }
    capture_active_work(snapshot, status, witness);
    capture_gatherer_bank_trip(snapshot, witness);
    let baseline_chat = witness.baseline_chat.unwrap_or(0);
    if witness.poison_tick.is_none()
        && snapshot
            .chat_lines()
            .iter()
            .any(|line| line.sequence > baseline_chat && line.text == POISON_CHAT)
    {
        witness.poison_tick = Some(snapshot.tick());
    }
    if witness.poison_tick.is_some() {
        if let Some(hitmarks) = snapshot.hitmarks() {
            record_poison_marks(hitmarks, snapshot.tick(), &mut witness.poison_marks);
        }
        if witness.cure_tick.is_none()
            && snapshot
                .chat_lines()
                .iter()
                .any(|line| line.sequence > baseline_chat && line.text == CURE_CHAT)
        {
            witness.cure_tick = Some(snapshot.tick());
        }
        if witness.dose_drop_tick.is_none()
            && item_count(snapshot, witness.antipoison_id) < witness.baseline_antipoison
        {
            witness.dose_drop_tick = Some(snapshot.tick());
        }
        if witness.drink_decision_tick.is_none()
            && slot.is_some_and(|slot| slot.survival.decision == Decision::Drinking)
        {
            witness.drink_decision_tick = Some(snapshot.tick());
            witness.drink_decision_modal = Some(snapshot.modals().chat);
        }
        if witness.immune_tick.is_none()
            && slot.is_some_and(|slot| matches!(slot.condition.poison, PoisonState::Immune { .. }))
        {
            witness.immune_tick = Some(snapshot.tick());
        }
    }
    let need_now = slot.is_some_and(|slot| capture_need(snapshot, status, slot, witness));
    if witness.activity == Activity::Dialogue
        && witness.profile_change_tick.is_some()
        && snapshot.tick() > witness.profile_change_tick.unwrap_or_default()
    {
        if snapshot.modals().chat == -1 && witness.dialogue_closed_tick.is_none() {
            witness.dialogue_closed_tick = Some(snapshot.tick());
        }
        if witness.settled_tick.is_some()
            && snapshot.tick() > witness.settled_tick.unwrap_or_default()
            && snapshot.modals().chat != -1
            && !snapshot.chat_options().is_empty()
            && status_text(status, "step_id") == Some("survival-talk")
            && witness.dialogue_reopened_tick.is_none()
        {
            witness.dialogue_reopened_tick = Some(snapshot.tick());
            witness.dialogue_reopened_options = snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.clone())
                .collect();
            witness.dialogue_reopened_step = status_text(status, "step_id").map(str::to_owned);
        }
    }
    if witness.settled_tick.is_none()
        && witness.cure_tick.is_some()
        && witness.dose_drop_tick.is_some()
        && witness.immune_tick.is_some()
        && status_text(status, "survival") == Some("None")
    {
        witness.settled_tick = Some(snapshot.tick());
    }
    capture_track(snapshot, status, slot, witness);
    need_now
}

fn profile_control(initial: vault::CurePoison, witness: Arc<Mutex<Witness>>) -> LiveProfileControl {
    LiveProfileControl::new(initial, move |snapshot, status, slot| {
        let mut witness = witness.lock().expect("survival witness");
        let need_now = observe_frame(snapshot, status, slot, &mut witness);
        if initial == vault::CurePoison::Never
            && witness.activity == Activity::Dialogue
            && witness.profile_change_tick.is_none()
            && need_now
            && snapshot.modals().chat != -1
            && !snapshot.chat_options().is_empty()
            && status_text(status, "step_id") == Some("survival-talk")
        {
            witness.profile_change_tick = Some(snapshot.tick());
            witness.profile_change_modal = Some(snapshot.modals().chat);
            witness.profile_change_options = snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.clone())
                .collect();
            Some(vault::CurePoison::WhenNeeded)
        } else {
            None
        }
    })
    .require_members_world()
}

fn check_mark(witness: &Witness) -> Result<(), String> {
    check_poison_marks(&witness.poison_marks, witness.cure_tick).map_err(|error| {
        format!(
            "{error}; poison line tick {:?}, final login tick {:?}",
            witness.poison_tick, witness.session.login
        )
    })
}

fn check_deadline(witness: &Witness, tick: u32, deadline: u32) -> Result<u32, String> {
    let poison = witness
        .poison_tick
        .ok_or("real poison chat was not observed")?;
    let elapsed = tick.saturating_sub(poison);
    if elapsed > deadline {
        return Err(format!(
            "poison outcome exceeded {deadline} observed ticks: now={tick}, poison={poison}"
        ));
    }
    Ok(elapsed)
}

/// The condition at the first lethal-need report. `Needed` is the open-latch
/// edge: the outlook is pending and the line is unlatched. `Drinking` is the
/// straight-to-drink report the design allows when the latch closed at the
/// line's own edge (A6.5 item 3, A1.4 item 6): the poison is still the onset
/// of this line, latched or not, and the outlook is already the widened
/// lethal one. Both need the same HP, food, dose and no hit at the edge.
fn check_need(witness: &Witness) -> Result<(), String> {
    let latch_ok = match witness.need_report_decision.as_deref() {
        Some("Needed") => {
            witness.need_outlook_pending == Some(true)
                && witness.need_unlatched_poison == Some(true)
        }
        Some("Drinking") => witness
            .need_condition_poison
            .as_deref()
            .is_some_and(|poison| poison.starts_with("Onset")),
        _ => false,
    };
    if !latch_ok
        || witness.need_report_why.as_deref() != Some("Some(Lethal)")
        || !witness
            .need_report
            .as_deref()
            .is_some_and(|report| report.contains("why: Lethal"))
        || witness.need_report_owner.as_deref() != Some("Step")
        || witness.need_outlook_total != Some(LETHAL_OUTLOOK)
        || witness.need_outlook_per_fire != Some(Q_UNKNOWN)
        || witness.need_hp != Some(30)
        || witness.need_hp_max != Some(30)
        || witness.need_food != Some(0)
        || witness.need_antipoison != Some(1)
        || witness.need_in_combat != Some(false)
        || witness.need_no_hit_at_edge != Some(true)
    {
        return Err(format!(
            "lethal poison need did not match the controlled edge: status={:?}, report={:?}/{:?}/{:?}/{:?}, condition={:?}, outlook={:?}/{:?}/{:?}, hp={:?}/{:?}, food={:?}, antipoison={:?}, combat={:?}, no_hit={:?}",
            witness.need_status,
            witness.need_report_decision,
            witness.need_report_why,
            witness.need_report,
            witness.need_report_owner,
            witness.need_condition_poison,
            witness.need_outlook_total,
            witness.need_outlook_per_fire,
            witness.need_outlook_pending,
            witness.need_hp,
            witness.need_hp_max,
            witness.need_food,
            witness.need_antipoison,
            witness.need_in_combat,
            witness.need_no_hit_at_edge,
        ));
    }
    Ok(())
}

/// The ticks of one cure, each seen: the poison line, the first lethal need
/// report (`Needed` or `Drinking`), the drink request, the drink line, the
/// dose drop and the settle. The need must match the controlled edge.
struct CureTicks {
    poison: u32,
    need: u32,
    drink: u32,
    cure: u32,
    dose_drop: u32,
    settled: u32,
}

fn cure_ticks(witness: &Witness) -> Result<CureTicks, String> {
    let poison = witness
        .poison_tick
        .ok_or("poison line tick was not observed")?;
    let need = witness.need_tick.ok_or(
        "neither a lethal Needed nor a lethal Drinking report was observed after the poison line",
    )?;
    let drink = witness
        .drink_decision_tick
        .ok_or("survival did not request the antipoison click")?;
    let cure = witness
        .cure_tick
        .ok_or("antipoison drink line was not observed")?;
    let dose_drop = witness
        .dose_drop_tick
        .ok_or("antipoison dose drop was not observed")?;
    let settled = witness
        .settled_tick
        .ok_or("antipoison immunity did not settle")?;
    check_need(witness)?;
    Ok(CureTicks {
        poison,
        need,
        drink,
        cure,
        dose_drop,
        settled,
    })
}

fn common_receipt(
    snapshot: &GameSnapshot,
    witness: &Witness,
    deadline: u32,
) -> Result<Value, String> {
    check_mark(witness)?;
    let elapsed = check_deadline(witness, snapshot.tick(), deadline)?;
    check_cure_outcome(
        witness.poison_tick,
        witness.cure_tick,
        &witness.poison_marks,
        snapshot.tick(),
    )?;
    let login_tick = witness
        .session
        .login
        .ok_or("final login tick was not observed")?;
    let phase_tick = witness
        .timer_phase_ready_tick
        .ok_or("controlled timer phase was not observed")?;
    let source_tick = witness
        .poison_source_tick
        .ok_or("poison source tick was not observed")?;
    let source_label = match witness.poison_source {
        PoisonSource::TwistedChest => "yanille-twisted-chest",
        PoisonSource::AccountStateDebugProc => "account-state-debugproc",
    };
    let source_signal_budget = match witness.poison_source {
        PoisonSource::TwistedChest => TWISTED_CHEST_POISON_SIGNAL_TICKS,
        PoisonSource::AccountStateDebugProc => DEBUGPROC_POISON_SIGNAL_TICKS,
    };
    let CureTicks {
        poison: poison_tick,
        need: need_tick,
        drink: drink_tick,
        cure: cure_tick,
        dose_drop: dose_drop_tick,
        settled: settled_tick,
    } = cure_ticks(witness)?;
    let key_after = item_count(snapshot, witness.key_id);
    let antipoison_after = item_count(snapshot, witness.antipoison_id);
    let tool_after = witness.tool_id.map_or(0, |id| item_count(snapshot, id));
    let resource_after = witness.resource_id.map_or(0, |id| item_count(snapshot, id));
    let inventory_after = inventory_total(snapshot);
    let inventory_at_start = witness.baseline_antipoison
        + witness.baseline_tool
        + witness.baseline_key
        + witness.expected_inventory_extra
        + witness.baseline_resource;
    let chest_rewards_after_start = i32::from(
        witness.poison_source == PoisonSource::TwistedChest
            && witness.activity != Activity::Dialogue,
    ) * TWISTED_CHEST_REWARD_COUNT;
    let chest_key_consumed_after_start = i32::from(
        witness.poison_source == PoisonSource::TwistedChest
            && witness.activity != Activity::Dialogue,
    ) * witness.baseline_key;
    let expected_inventory_after = inventory_at_start + chest_rewards_after_start
        - chest_key_consumed_after_start
        - witness.baseline_antipoison.saturating_sub(antipoison_after)
        + tool_after.saturating_sub(witness.baseline_tool)
        + resource_after
        - witness.baseline_resource;
    let inventory_matches = if witness.activity == Activity::Dialogue {
        inventory_after == expected_inventory_after
    } else {
        inventory_after >= expected_inventory_after
    };
    if phase_tick.saturating_sub(login_tick) < TIMER_CLEAR_TICKS
        || source_tick.saturating_sub(login_tick) < TIMER_CLEAR_TICKS
        || source_tick < phase_tick
        || poison_tick < source_tick
        || poison_tick.saturating_sub(source_tick) > source_signal_budget
        || need_tick < poison_tick
        || need_tick.saturating_sub(poison_tick) > 3
        || drink_tick < poison_tick
        || drink_tick.saturating_sub(poison_tick) > 3
        || cure_tick < poison_tick
        || cure_tick.saturating_sub(poison_tick) > CURE_LINE_BOUND_TICKS
        || dose_drop_tick < poison_tick
        || dose_drop_tick.saturating_sub(poison_tick) > CURE_LINE_BOUND_TICKS
        || settled_tick < cure_tick
        || snapshot.tick().saturating_sub(poison_tick) < TIMER_CLEAR_TICKS
        || witness.poison_source_modal != Some(-1)
        || key_after != 0
        || !inventory_matches
    {
        return Err(format!(
            "cure/source ticks or inventory outside the content and runner bounds: login={login_tick}, phase={phase_tick}, source={source_tick}, source_budget={source_signal_budget}, poison={poison_tick}, need={need_tick}, drink={drink_tick}, cure={cure_tick}, drop={dose_drop_tick}, settled={settled_tick}, key_after={key_after}, inventory={inventory_after}/{expected_inventory_after}, observed={}",
            snapshot.tick()
        ));
    }
    if witness.drink_decision_modal != Some(-1) {
        return Err(format!(
            "antipoison click was requested while a modal was open: {:?}",
            witness.drink_decision_modal
        ));
    }
    Ok(json!({
        "login_tick": login_tick,
        "session_logins": witness.session.logins,
        "tick_base": "harness snapshot, restarted at each session (host/src/lib.rs:1464-1480)",
        "timer_phase_ready_tick": phase_tick,
        "poison_source": source_label,
        "poison_source_tick": source_tick,
        "poison_source_chat_modal": witness.poison_source_modal,
        "poison_source_signal_budget_ticks": source_signal_budget,
        "ticks_after_login_at_source": source_tick.saturating_sub(login_tick),
        "poison_chat_tick": poison_tick,
        "lethal_need_tick": need_tick,
        "survival_report_at_need": {
            "decision": witness.need_report_decision,
            "why": witness.need_report_why,
            "need": witness.need_report,
            "owner": witness.need_report_owner,
            "status": witness.need_status,
            "phase": witness.need_phase,
            "step_id": witness.need_step,
        },
        "condition_at_need": {
            "tick": witness.need_condition_tick,
            "poison": witness.need_condition_poison,
            "unlatched_poison": witness.need_unlatched_poison,
            "outlook_total": witness.need_outlook_total,
            "outlook_per_fire": witness.need_outlook_per_fire,
            "outlook_pending": witness.need_outlook_pending,
            "hitpoints": witness.need_hp,
            "hitpoints_max": witness.need_hp_max,
            "food": witness.need_food,
            "antipoison": witness.need_antipoison,
            "in_combat": witness.need_in_combat,
            "no_hitmark_at_edge": witness.need_no_hit_at_edge,
        },
        "drink_decision_tick": drink_tick,
        "drink_decision_chat_modal": witness.drink_decision_modal,
        "antipoison_drink_chat_tick": cure_tick,
        "antipoison_drop_tick": dose_drop_tick,
        "immune_tick": witness.immune_tick,
        "antipoison_settled_tick": settled_tick,
        "sinister_key_at_start": witness.baseline_key,
        "sinister_key_after": key_after,
        "chest_reward_count_at_start": witness.expected_inventory_extra,
        "chest_rewards_after_start": chest_rewards_after_start,
        "inventory_total_at_start": inventory_at_start,
        "inventory_total_after": inventory_after,
        "inventory_total_minimum_after": expected_inventory_after,
        "ticks_from_poison_to_settled": settled_tick.saturating_sub(poison_tick),
        "poison_marks": witness
            .poison_marks
            .iter()
            .map(|mark| json!({"tick": mark.tick, "slot": mark.slot, "cycle": mark.cycle, "value": mark.value}))
            .collect::<Vec<_>>(),
        "poison_mark_rule": "none before the drink line; at most one queued hit within POST_CURE_QUEUED_HIT_TICKS after it",
        "observed_poison_ticks": elapsed,
        "first_scheduled_fire_covered": true,
        "first_scheduled_fire_covered_tick": snapshot.tick(),
    }))
}

fn first_poison_fire_observed(snapshot: &GameSnapshot, witness: &Witness) -> bool {
    witness
        .poison_tick
        .is_some_and(|poison_tick| snapshot.tick().saturating_sub(poison_tick) >= TIMER_CLEAR_TICKS)
}

fn work_receipt(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &Witness,
    family: &str,
    deadline: u32,
) -> Result<Option<Value>, String> {
    check_mark(witness)?;
    // Work cells poison the bot after Start, so frames before the source and
    // its chat carry no outcome yet. The source step bounds the chat signal
    // itself (`TWISTED_CHEST_POISON_SIGNAL_TICKS`,
    // `DEBUGPROC_POISON_SIGNAL_TICKS`); the outcome deadline counts from it.
    let Some(source_tick) = witness.poison_source_tick else {
        return Ok(None);
    };
    if witness.poison_tick.is_none() {
        return Ok(None);
    }
    check_deadline(witness, snapshot.tick(), deadline)?;
    let Some(work_active_tick) = witness.work_active_tick else {
        if witness.poison_during_provisioning {
            return Ok(None);
        }
        return Err("authored work did not start before poison source".to_owned());
    };
    let invalid_work_evidence = if witness.poison_during_provisioning {
        witness
            .provisioning_tick
            .is_none_or(|tick| tick > source_tick)
            || work_active_tick <= source_tick
            || witness.poison_seed_bank_open != Some(true)
            || witness.poison_seed_tool_count != Some(witness.baseline_tool)
    } else {
        work_active_tick > source_tick
            || (witness.activity == Activity::Mining
                && (witness.work_animation_tick.is_none() || witness.work_animation.is_none()))
    };
    if invalid_work_evidence {
        return Err(format!(
            "authored work evidence did not match poison-source timing: work={work_active_tick}, source={source_tick}, provision={:?}, bank_open={:?}, seed_tool_count={:?}, animation_tick={:?}, animation={:?}",
            witness.provisioning_tick,
            witness.poison_seed_bank_open,
            witness.poison_seed_tool_count,
            witness.work_animation_tick,
            witness.work_animation
        ));
    }
    if witness.poison_during_provisioning && witness.tool_id.is_none() {
        return Err("provisioning poison proof has no selected tool".to_owned());
    }
    let settled = witness.settled_tick;
    let Some(settled) = settled else {
        return Ok(None);
    };
    if snapshot.tick() <= settled
        || status.is_none_or(|status| status.phase != NativePhase::Working)
    {
        return Ok(None);
    }
    let xp = stat_xp(snapshot, witness.skill_stat);
    let resource = witness.resource_id.map_or(0, |id| item_count(snapshot, id));
    let (Some(need_xp), Some(need_resource)) = (witness.need_xp, witness.need_resource) else {
        return Ok(None);
    };
    if xp <= need_xp || (witness.resource_id.is_some() && resource <= need_resource) {
        return Ok(None);
    }
    if witness.activity == Activity::Mining
        && status_text(status, "step_id") != Some("survival-work")
    {
        return Ok(None);
    }
    if !first_poison_fire_observed(snapshot, witness) {
        return Ok(None);
    }
    let mut receipt = common_receipt(snapshot, witness, deadline)?;
    receipt["family"] = json!(family);
    receipt["stand"] = json!(witness.stand);
    receipt["skill"] = json!(witness.skill_name);
    receipt["skill_xp_at_need"] = json!(need_xp);
    receipt["skill_xp_after"] = json!(xp);
    receipt["resource_count_at_start"] = json!(witness.baseline_resource);
    receipt["resource_count_at_need"] = json!(need_resource);
    receipt["resource_count_after"] = json!(resource);
    receipt["tool_count_at_start"] = json!(witness.baseline_tool);
    receipt["tool_count_after"] = json!(witness.tool_id.map_or(0, |id| item_count(snapshot, id)));
    receipt["work_active_tick"] = json!(witness.work_active_tick);
    receipt["work_animation_tick"] = json!(witness.work_animation_tick);
    receipt["work_animation"] = json!(witness.work_animation);
    receipt["provisioning_tick"] = json!(witness.provisioning_tick);
    receipt["poison_seed_bank_open"] = json!(witness.poison_seed_bank_open);
    receipt["poison_seed_tool_count"] = json!(witness.poison_seed_tool_count);
    receipt["resumed_step_id"] = json!(status_text(status, "step_id"));
    receipt["resumed_work_tick"] = json!(snapshot.tick());
    receipt["resumed_work_ticks"] = json!(snapshot.tick().saturating_sub(settled));
    receipt["antipoison_remaining"] = json!(item_count(snapshot, witness.antipoison_id));
    Ok(Some(receipt))
}

fn check_gatherer_bank_trip_antipoison_frames(
    frames: &[BankTripAntipoisonFrame],
    cure_line_tick: Option<u32>,
) -> Result<i32, String> {
    let mut minimum = None;
    for frame in frames {
        if cure_line_tick.is_none_or(|cure_tick| frame.tick < cure_tick) {
            minimum = Some(minimum.map_or(frame.count, |count: i32| count.min(frame.count)));
            if frame.count != 1 {
                return Err(format!(
                    "Gatherer bank-trip antipoison count was {} at tick {} before cure line tick {cure_line_tick:?}",
                    frame.count, frame.tick
                ));
            }
        }
    }
    minimum.ok_or_else(|| "no Gatherer bank-trip antipoison frames before cure line".to_owned())
}

fn check_gatherer_bank_trip_receipt(receipt: &Value) -> Result<(), String> {
    let tick = |key: &str| {
        receipt[key]
            .as_u64()
            .and_then(|tick| u32::try_from(tick).ok())
            .ok_or_else(|| format!("Gatherer bank-trip receipt has no valid {key}"))
    };
    let count = |key: &str| {
        receipt[key]
            .as_i64()
            .and_then(|count| i32::try_from(count).ok())
            .ok_or_else(|| format!("Gatherer bank-trip receipt has no valid {key}"))
    };
    let start_tick = tick("trip_start_tick")?;
    let bank_open_tick = tick("bank_open_tick")?;
    let deposit_tick = tick("willow_logs_deposit_tick")?;
    let end_tick = tick("trip_end_tick")?;
    let work_animation_tick = tick("first_work_animation_after_bank_trip_tick")?;
    let cure_line_tick = tick("cure_line_tick")?;
    let observed_tick = tick("observed_tick")?;
    let deadline_ticks = tick("deadline_ticks")?;
    let elapsed_ticks = tick("trip_and_cure_elapsed_ticks")?;
    if receipt["bank_opened"].as_bool() != Some(true)
        || bank_open_tick < start_tick
        || receipt["willow_logs_deposited"].as_bool() != Some(true)
        || deposit_tick < bank_open_tick
        || count("willow_logs_at_deposit")? != 0
        || count("willow_logs_count_deposited")? <= 0
        || work_animation_tick != end_tick
        || end_tick <= deposit_tick
        || cure_line_tick <= end_tick
        || observed_tick < cure_line_tick
        || receipt["bank_closed_at_resumed_work"].as_bool() != Some(true)
        || deadline_ticks != GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS
        || observed_tick.saturating_sub(start_tick) != elapsed_ticks
        || elapsed_ticks > deadline_ticks
    {
        return Err(format!(
            "Gatherer bank-trip facts or {deadline_ticks}-tick observed deadline failed: receipt={receipt}"
        ));
    }
    if count("inventory_at_trip_start")? != GATHERER_INVENTORY_CAPACITY {
        return Err(format!(
            "Gatherer bank trip did not start with a full {}-item pack",
            GATHERER_INVENTORY_CAPACITY
        ));
    }
    let samples = receipt["antipoison_count_frames"]
        .as_array()
        .ok_or("Gatherer bank-trip antipoison frames are missing")?;
    let mut frames = Vec::with_capacity(samples.len());
    for sample in samples {
        let tick = sample["tick"]
            .as_u64()
            .and_then(|tick| u32::try_from(tick).ok())
            .ok_or("Gatherer bank-trip frame has no valid tick")?;
        let count = sample["count"]
            .as_i64()
            .and_then(|count| i32::try_from(count).ok())
            .ok_or("Gatherer bank-trip frame has no valid antipoison count")?;
        frames.push(BankTripAntipoisonFrame { tick, count });
    }
    if frames.first().map(|frame| frame.tick) != Some(start_tick)
        || frames.last().map(|frame| frame.tick) != Some(end_tick)
        || frames
            .iter()
            .any(|frame| frame.tick < start_tick || frame.tick > end_tick)
        || frames.windows(2).any(|pair| pair[0].tick > pair[1].tick)
    {
        return Err(
            "Gatherer bank-trip antipoison frames do not span the observed trip".to_owned(),
        );
    }
    let minimum = check_gatherer_bank_trip_antipoison_frames(&frames, Some(cure_line_tick))?;
    if count("antipoison_count_min_before_cure")? != minimum || minimum != 1 {
        return Err(format!(
            "Gatherer bank-trip antipoison minimum before cure was {minimum}, expected 1"
        ));
    }
    let animation_sequence = receipt["first_work_animation_after_bank_trip"]["sequence"]
        .as_i64()
        .ok_or("Gatherer post-trip work animation is missing its sequence")?;
    if animation_sequence < 0 {
        return Err("Gatherer post-trip work animation has an invalid sequence".to_owned());
    }
    Ok(())
}

fn gatherer_bank_trip_receipt(witness: &Witness, observed_tick: u32) -> Result<Value, String> {
    let start_tick = witness
        .bank_trip_start_tick
        .ok_or("Gatherer bank-trip start frame was not observed")?;
    let bank_open_tick = witness
        .bank_trip_bank_open_tick
        .ok_or("Gatherer bank did not open during the trip")?;
    let deposit_tick = witness
        .bank_trip_deposit_tick
        .ok_or("Gatherer willow logs were not deposited")?;
    let logs_at_deposit = witness
        .bank_trip_willow_logs_at_deposit
        .ok_or("Gatherer willow-log deposit count was not observed")?;
    let logs_deposited = witness
        .bank_trip_willow_logs_deposited
        .ok_or("Gatherer willow-log deposit amount was not observed")?;
    let work_animation_tick = witness
        .bank_trip_work_animation_tick
        .ok_or("Gatherer did not resume willow work after the bank trip")?;
    let work_animation = witness
        .bank_trip_work_animation
        .ok_or("Gatherer post-trip work animation was not observed")?;
    let cure_line_tick = witness
        .cure_tick
        .ok_or("Gatherer cure line was not observed after the bank trip")?;
    let elapsed_ticks = observed_tick
        .checked_sub(start_tick)
        .ok_or("Gatherer observed tick reset during the bank-trip proof")?;
    if elapsed_ticks > GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS {
        return Err(format!(
            "Gatherer bank trip and cure exceeded {} observed game ticks: start={start_tick}, now={observed_tick}, elapsed={elapsed_ticks}",
            GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS
        ));
    }
    let minimum = check_gatherer_bank_trip_antipoison_frames(
        &witness.bank_trip_antipoison_frames,
        Some(cure_line_tick),
    )?;
    let receipt = json!({
        "trip_start_tick": start_tick,
        "inventory_at_trip_start": witness.bank_trip_inventory_at_start,
        "bank_opened": true,
        "bank_open_tick": bank_open_tick,
        "willow_logs_deposited": logs_deposited > 0 && logs_at_deposit == 0,
        "willow_logs_count_deposited": logs_deposited,
        "willow_logs_at_deposit": logs_at_deposit,
        "willow_logs_deposit_tick": deposit_tick,
        "antipoison_count_frames": witness
            .bank_trip_antipoison_frames
            .iter()
            .map(|frame| json!({"tick": frame.tick, "count": frame.count}))
            .collect::<Vec<_>>(),
        "antipoison_count_min_before_cure": minimum,
        "first_work_animation_after_bank_trip_tick": work_animation_tick,
        "first_work_animation_after_bank_trip": work_animation,
        "bank_closed_at_resumed_work": witness.bank_trip_bank_closed_at_work,
        "trip_end_tick": work_animation_tick,
        "cure_line_tick": cure_line_tick,
        "observed_tick": observed_tick,
        "deadline_ticks": GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS,
        "trip_and_cure_elapsed_ticks": elapsed_ticks,
    });
    check_gatherer_bank_trip_receipt(&receipt)?;
    Ok(receipt)
}

fn gatherer_bank_trip_work_receipt(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    if let Some(start_tick) = witness.bank_trip_start_tick {
        check_gatherer_bank_trip_antipoison_frames(
            &witness.bank_trip_antipoison_frames,
            witness.cure_tick,
        )?;
        let elapsed_ticks = snapshot
            .tick()
            .checked_sub(start_tick)
            .ok_or("Gatherer observed tick reset during the bank-trip proof")?;
        if elapsed_ticks > GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS {
            return Err(format!(
                "Gatherer bank trip and cure exceeded {} observed game ticks: start={start_tick}, now={}, elapsed={elapsed_ticks}",
                GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS,
                snapshot.tick()
            ));
        }
    }
    let Some(mut receipt) = work_receipt(
        snapshot,
        status,
        witness,
        "Gatherer",
        GATHERER_CURE_BUDGET_TICKS,
    )?
    else {
        return Ok(None);
    };
    receipt["gatherer_bank_trip"] = gatherer_bank_trip_receipt(witness, snapshot.tick())?;
    Ok(Some(receipt))
}

fn matches_desert_captain_opening_options(options: &[String]) -> bool {
    options.len() == DESERT_CAPTAIN_OPENING_OPTIONS.len()
        && options
            .iter()
            .map(String::as_str)
            .eq(DESERT_CAPTAIN_OPENING_OPTIONS.iter().copied())
}

fn dialogue_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_mark(witness)?;
    check_deadline(witness, snapshot.tick(), 150)?;
    let (Some(change), Some(closed), Some(reopened), Some(drink), Some(drink_modal)) = (
        witness.profile_change_tick,
        witness.dialogue_closed_tick,
        witness.dialogue_reopened_tick,
        witness.drink_decision_tick,
        witness.drink_decision_modal,
    ) else {
        return Ok(None);
    };
    let Some(cure) = witness.cure_tick else {
        return Ok(None);
    };
    let Some(drop) = witness.dose_drop_tick else {
        return Ok(None);
    };
    let Some(immune) = witness.immune_tick else {
        return Ok(None);
    };
    let Some(settled) = witness.settled_tick else {
        return Ok(None);
    };
    let Some(source_tick) = witness.poison_source_tick else {
        return Ok(None);
    };
    let Some(page_tick) = witness.dialogue_page_tick else {
        return Ok(None);
    };
    check_need(witness)?;
    if !first_poison_fire_observed(snapshot, witness) {
        return Ok(None);
    }
    if witness.desert_stage_reply.as_deref() != Some(DESERT_STAGE_REPLY)
        || witness.profile_change_modal == Some(-1)
        || !matches_desert_captain_opening_options(&witness.profile_change_options)
        || witness.need_page_open_tick != Some(change)
        || !matches_desert_captain_opening_options(&witness.need_page_options)
        || page_tick != change
        || source_tick >= page_tick
        || witness.need_step.as_deref() != Some("survival-talk")
        || closed > change.saturating_add(2)
        || drink < closed
        || drink > change.saturating_add(2)
        || drink_modal != -1
        || cure < change
        || cure.saturating_sub(change) > 10
        || drop < change
        || drop.saturating_sub(change) > 10
        || immune < cure
        || settled < immune
        || reopened <= settled
        || witness.dialogue_reopened_step.as_deref() != Some("survival-talk")
        || !matches_desert_captain_opening_options(&witness.dialogue_reopened_options)
    {
        return Err(format!(
            "dialogue cure/resume order failed: change={change}, stage={:?}, page_tick={page_tick}, source={source_tick}, modal={:?}, options={:?}, need_page={:?}, need_options={:?}, closed={closed}, drink={drink}, drink_modal={drink_modal}, cure={cure}, drop={drop}, immune={immune}, settled={settled}, reopened={reopened}, reopened_options={:?}, marks={:?}, step={:?}",
            witness.desert_stage_reply,
            witness.profile_change_modal,
            witness.profile_change_options,
            witness.need_page_open_tick,
            witness.need_page_options,
            witness.dialogue_reopened_options,
            witness.poison_marks,
            witness.dialogue_reopened_step,
        ));
    }
    let mut receipt = common_receipt(snapshot, witness, 150)?;
    receipt["family"] = json!("Quester");
    receipt["npc_config"] = json!("desertminingcaptain");
    receipt["npc_stage"] = json!(witness.desert_stage_reply);
    receipt["npc_stage_tick"] = json!(witness.desert_stage_reply_tick);
    receipt["dialogue_page_tick"] = json!(page_tick);
    receipt["dialogue_options_at_need"] = json!(witness.need_page_options);
    receipt["profile_change_tick"] = json!(change);
    receipt["profile_change_chat_modal"] = json!(witness.profile_change_modal);
    receipt["profile_change_options"] = json!(witness.profile_change_options);
    receipt["dialogue_closed_tick"] = json!(closed);
    receipt["drink_decision_tick"] = json!(drink);
    receipt["drink_decision_chat_modal"] = json!(drink_modal);
    receipt["dialogue_reopened_tick"] = json!(reopened);
    receipt["dialogue_options_after_resume"] = json!(witness.dialogue_reopened_options);
    receipt["resumed_step_id"] = json!(witness.dialogue_reopened_step);
    Ok(Some(receipt))
}

fn observe_start(snapshot: &GameSnapshot, witness: &mut Witness) -> Result<Value, String> {
    if witness.activity == Activity::Dialogue && witness.desert_stage_reply.is_none() {
        if let Some(line) = snapshot
            .chat_lines()
            .iter()
            .find(|line| line.text == DESERT_STAGE_REPLY)
        {
            witness.desert_stage_reply = Some(line.text.clone());
            witness.desert_stage_reply_tick = Some(snapshot.tick());
        }
    }
    witness
        .baseline_chat
        .get_or_insert_with(|| latest_chat(snapshot));
    witness.baseline_xp = stat_xp(snapshot, witness.skill_stat);
    witness.baseline_resource = witness.resource_id.map_or(0, |id| item_count(snapshot, id));
    witness.baseline_tool = witness.tool_id.map_or(0, |id| item_count(snapshot, id));
    witness.baseline_antipoison = item_count(snapshot, witness.antipoison_id);
    witness.baseline_key = item_count(snapshot, witness.key_id);
    witness.baseline_animation_serial = snapshot
        .local_player()
        .and_then(|player| player.animation_update)
        .map(|animation| animation.serial);
    let hp = stat_base(snapshot, HITPOINTS_STAT);
    let skill = stat_base(snapshot, witness.skill_stat);
    let food = witness.food_id.map_or(0, |id| item_count(snapshot, id));
    let inventory_expected = witness.baseline_antipoison
        + witness.baseline_tool
        + witness.baseline_key
        + witness.expected_inventory_extra
        + witness.expected_resource_at_start
        + witness.expected_food_at_start;
    let inventory_observed = inventory_total(snapshot);
    let tile = snapshot.tile();
    if tile != Some((witness.stand.x, witness.stand.z, witness.stand.level))
        || hp != witness.expected_hp
        || skill != WORK_SKILL_LEVEL
        || witness.baseline_antipoison != witness.expected_antipoison
        || food != witness.expected_food_at_start
        || witness.baseline_tool != witness.expected_tool_at_start
        || witness.baseline_key != witness.expected_key_at_start
        || witness.baseline_resource != witness.expected_resource_at_start
        || inventory_observed != inventory_expected
        || snapshot.modals().chat != -1
    {
        return Err(format!(
            "own-state fixture mismatch: tile={tile:?}, expected={:?}, HP={hp}, {}={skill}, antipoison={}, tool={}, key={}, resource={} (expected {}), inventory_total={inventory_observed}, expected_total={inventory_expected}, chat_modal={}",
            witness.stand,
            witness.skill_name,
            witness.baseline_antipoison,
            witness.baseline_tool,
            witness.baseline_key,
            witness.baseline_resource,
            witness.expected_resource_at_start,
            snapshot.modals().chat,
        ));
    }
    Ok(json!({
        "seed": match (
            witness.activity,
            witness.expected_tool_at_start,
            witness.expected_resource_at_start > 0,
        ) {
            _ if witness.expected_antipoison != 1 || witness.expected_food_at_start > 0 => {
                "account-owned stats, the work tool, and the antipoison and food counts below"
            }
            (Activity::Dialogue, _, _) => {
                "account-owned stats, one antipoison dose and nine herbs from the Yanille chest"
            }
            (Activity::Woodcutting, _, true) => {
                "account-owned stats, one antipoison dose, rune axe and 25 willow logs"
            }
            (_, 0, _) => "account-owned stats, one antipoison dose and a banked work tool",
            _ => "account-owned stats, one antipoison dose, the work tool and a sinister key",
        },
        "food_id": witness.food_id,
        "food": food,
        "start_tile": tile,
        "hitpoints_base": hp,
        "skill": witness.skill_name,
        "skill_base": skill,
        "skill_xp": witness.baseline_xp,
        "resource_count": witness.baseline_resource,
        "antipoison": witness.baseline_antipoison,
        "tool": witness.baseline_tool,
        "sinister_key": witness.baseline_key,
        "chest_reward_count": witness.expected_inventory_extra,
        "no_food": food == 0 && inventory_observed == inventory_expected,
        "inventory_total": inventory_observed,
        "chat_sequence": witness.baseline_chat,
        "login_tick": witness.session.login,
        "session_logins": witness.session.logins,
        "poison_preseeded": witness.poison_tick.is_some(),
    }))
}

fn run_family(
    cell: Cell,
    script_name: &'static str,
    settings: Map<String, Value>,
    witness: Arc<Mutex<Witness>>,
    initial_cure_poison: vault::CurePoison,
    observe: impl FnMut(&GameSnapshot, Option<&ScriptStatus>, &Witness) -> Result<Option<Value>, String>
        + Send
        + 'static,
) -> Value {
    let mut cell = cell;
    cell.scenario.settings.deadline = Duration::from_secs(900);
    cell.start_settings = settings.clone();
    let witness_for_start = Arc::clone(&witness);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let mut witness = witness_for_start
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        observe_start(snapshot, &mut witness)
    }));
    let start: quester_live::StartFamily = Box::new(move |handle, account, _| {
        handle.start_compiled(account, script::CompiledId(script_name), settings.clone())
    });
    let observed_witness = Arc::clone(&witness);
    let mut observe = observe;
    let observe: quester_live::ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let mut witness = observed_witness
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        if let (None, Some(lifecycle)) = (&witness.lifecycle_seen, lifecycle) {
            witness.lifecycle_seen = Some(format!("{lifecycle:?}"));
        }
        observe(snapshot, status, &witness)
    });
    quester_live::run_family_with_profile_control(
        cell,
        start,
        observe,
        profile_control(initial_cure_poison, witness),
    )
    .expect("native survival cure live proof")
}

fn assert_common_cure(receipt: &Value, resumed_ticks: u64) {
    assert_eq!(
        receipt["condition_at_need"]["outlook_total"],
        LETHAL_OUTLOOK
    );
    assert_eq!(receipt["condition_at_need"]["outlook_per_fire"], Q_UNKNOWN);
    // `check_need` applied the per-decision latch rule; restate it. `Needed`
    // is the open latch (outlook pending); `Drinking` is the straight-to-drink
    // report, with the latch either way (A6.5 item 3).
    match receipt["survival_report_at_need"]["decision"].as_str() {
        Some("Needed") => {
            assert_eq!(receipt["condition_at_need"]["outlook_pending"], true);
            assert_eq!(receipt["condition_at_need"]["unlatched_poison"], true);
        }
        Some("Drinking") => assert!(receipt["condition_at_need"]["poison"]
            .as_str()
            .is_some_and(|poison| poison.starts_with("Onset"))),
        other => panic!("need report decision is neither Needed nor Drinking: {other:?}"),
    }
    assert_eq!(receipt["survival_report_at_need"]["why"], "Some(Lethal)");
    assert_eq!(receipt["survival_report_at_need"]["owner"], "Step");
    assert_eq!(receipt["condition_at_need"]["hitpoints"], 30);
    assert_eq!(receipt["condition_at_need"]["hitpoints_max"], 30);
    assert_eq!(receipt["condition_at_need"]["food"], 0);
    assert_eq!(receipt["condition_at_need"]["antipoison"], 1);
    assert_eq!(receipt["condition_at_need"]["in_combat"], false);
    assert_eq!(receipt["condition_at_need"]["no_hitmark_at_edge"], true);
    assert_eq!(receipt["drink_decision_chat_modal"], -1);
    // `check_cure_outcome` already applied the content rule; restate its cap.
    assert!(receipt["poison_marks"].as_array().unwrap().len() <= 1);
    assert!(receipt["first_scheduled_fire_covered"].as_bool().unwrap());
    assert!(receipt["observed_poison_ticks"].as_u64().unwrap() >= u64::from(TIMER_CLEAR_TICKS));
    assert!(receipt["ticks_from_poison_to_settled"].as_u64().unwrap() <= 10);
    assert!(
        receipt["resumed_work_ticks"]
            .as_u64()
            .unwrap_or(resumed_ticks)
            <= resumed_ticks
    );
    assert_eq!(receipt["antipoison_remaining"], 0);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_when_needed_drains_willow_and_resumes_gatherer() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let mut cell = fixture(
        Activity::Woodcutting,
        "L-CURE-WHEN-NEEDED",
        GATHERER_STAND,
        &[(RUNE_AXE_ALIAS, 1), (ANTIPOISON_ALIAS, 1)],
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Woodcutting,
        "Woodcutting",
        WOODCUTTING_STAT,
        Some(WILLOW_LOG_ID),
        Some(RUNE_AXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        GATHERER_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> = serde_json::from_value(json!({
        "skill":"Woodcutting",
        "woodcuttingResources":["willow"],
        "miningResources":[],
        "targetPreference":"Nearest",
        "location":"Start",
        "radius":32,
        "disposition":"Power"
    }))
    .expect("Gatherer settings");
    let receipt = run_family(
        cell,
        "Gatherer",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Gatherer", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Gatherer");
    assert_eq!(proof["poison_source"], "account-state-debugproc");
    assert_eq!(proof["poison_source_chat_modal"], -1);
    assert_eq!(proof["stand"]["x"], GATHERER_STAND.x);
    assert_eq!(proof["stand"]["z"], GATHERER_STAND.z);
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_need"].as_i64().unwrap()
    );
    assert_common_cure(proof, 120);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_gatherer_keeps_dose_through_bank_trip_and_resumes() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let willow_distance = GATHERER_STAND
        .x
        .abs_diff(GATHERER_WILLOW_TILE.x)
        .max(GATHERER_STAND.z.abs_diff(GATHERER_WILLOW_TILE.z));
    assert!(willow_distance <= GATHERER_RADIUS);
    assert_eq!(GATHERER_STAND.level, GATHERER_WILLOW_TILE.level);
    let bank_walk_tiles = GATHERER_STAND.x.abs_diff(GATHERER_BANK_BOOTH_TILE.x)
        + GATHERER_STAND.z.abs_diff(GATHERER_BANK_BOOTH_TILE.z);
    assert_eq!(bank_walk_tiles, GATHERER_BANK_ONE_WAY_TILES);
    assert_eq!(
        GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS,
        bank_walk_tiles * 2 + GATHERER_CURE_BUDGET_TICKS
    );
    assert_eq!(
        GATHERER_STARTING_WILLOW_LOGS + 2,
        GATHERER_INVENTORY_CAPACITY - 1
    );
    let mut cell = fixture(
        Activity::Woodcutting,
        "L-CURE-GATHERER-BANK-TRIP",
        GATHERER_STAND,
        &[
            (RUNE_AXE_ALIAS, 1),
            (ANTIPOISON_ALIAS, 1),
            (WILLOW_LOGS_ALIAS, GATHERER_STARTING_WILLOW_LOGS),
        ],
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Woodcutting,
        "Woodcutting",
        WOODCUTTING_STAT,
        Some(WILLOW_LOG_ID),
        Some(RUNE_AXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        GATHERER_STAND,
    );
    witness
        .lock()
        .expect("survival witness")
        .expected_resource_at_start = GATHERER_STARTING_WILLOW_LOGS;
    add_poison_steps_after_gatherer_bank_trip(
        &mut cell.scenario,
        Arc::clone(&witness),
        GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS,
    );
    let settings: Map<String, Value> = serde_json::from_value(json!({
        "skill":"Woodcutting",
        "woodcuttingResources":["willow"],
        "miningResources":[],
        "targetPreference":"Nearest",
        "location":"Start",
        "radius":GATHERER_RADIUS,
        "disposition":"Bank",
        "bank":"Nearest"
    }))
    .expect("Gatherer bank settings");
    let receipt = run_family(
        cell,
        "Gatherer",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        gatherer_bank_trip_work_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Gatherer");
    assert_eq!(proof["poison_source"], "account-state-debugproc");
    assert_eq!(proof["poison_source_chat_modal"], -1);
    assert_eq!(proof["stand"]["x"], GATHERER_STAND.x);
    assert_eq!(proof["stand"]["z"], GATHERER_STAND.z);
    assert_eq!(
        proof["resource_count_at_start"],
        GATHERER_STARTING_WILLOW_LOGS
    );
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_need"].as_i64().unwrap()
    );
    let trip = &proof["gatherer_bank_trip"];
    assert_eq!(trip["bank_opened"], true);
    assert_eq!(trip["inventory_at_trip_start"], GATHERER_INVENTORY_CAPACITY);
    assert_eq!(trip["willow_logs_deposited"], true);
    assert_eq!(trip["willow_logs_at_deposit"], 0);
    assert!(trip["willow_logs_count_deposited"].as_i64().unwrap() > 0);
    assert_eq!(trip["antipoison_count_min_before_cure"], 1);
    assert_eq!(
        trip["first_work_animation_after_bank_trip_tick"],
        trip["trip_end_tick"]
    );
    assert!(
        trip["first_work_animation_after_bank_trip"]["sequence"]
            .as_i64()
            .unwrap()
            >= 0
    );
    assert_eq!(
        trip["deadline_ticks"],
        GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS
    );
    assert!(
        trip["trip_and_cure_elapsed_ticks"].as_u64().unwrap()
            <= u64::from(GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS)
    );
    assert_common_cure(proof, u64::from(GATHERER_CURE_BUDGET_TICKS));
}

static PATH_SOURCE_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_when_needed_quester_mines_copper_and_resumes_gather() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "copper-mining",
        json!([{
            "id":"survival-work",
            "kind":"gather",
            "version":1,
            "advances":false,
            "args":{
                "skill":"mining",
                "resource":"copper",
                "anchor":{
                    "tile":[COPPER_STAND.x, COPPER_STAND.z, COPPER_STAND.level],
                    "source":"C:maps/m51_52.jm2:5202-5203,5234; C:pack/loc.pack:2091-2092"
                },
                "until":{"obj":{"id":436},"qty":100}
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        Activity::Mining,
        "L-CURE-WHEN-NEEDED-QUESTER",
        COPPER_STAND,
        &[(RUNE_PICKAXE_ALIAS, 1), (ANTIPOISON_ALIAS, 1)],
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Mining,
        "Mining",
        MINING_STAT,
        Some(COPPER_ORE_ID),
        Some(RUNE_PICKAXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        COPPER_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Quester", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "account-state-debugproc");
    assert_eq!(proof["poison_source_chat_modal"], -1);
    assert_eq!(proof["resumed_step_id"], "survival-work");
    assert_eq!(proof["stand"]["x"], COPPER_STAND.x);
    assert_eq!(proof["stand"]["z"], COPPER_STAND.z);
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_need"].as_i64().unwrap()
    );
    assert!(
        proof["resource_count_after"].as_i64().unwrap()
            > proof["resource_count_at_start"].as_i64().unwrap()
    );
    assert!(proof["work_animation_tick"].as_u64().is_some());
    assert!(proof["work_animation"].is_object());
    assert_common_cure(proof, 120);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_quester_interrupts_bank_provisioning_and_resumes_mining() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "copper-mining-provisioning",
        json!([{
            "id":"survival-work",
            "kind":"gather",
            "version":1,
            "advances":false,
            "args":{
                "skill":"mining",
                "resource":"copper",
                "until":{"obj":{"id":436},"qty":100}
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        Activity::Mining,
        "L-CURE-QUESTER-PROVISIONING",
        MINING_STAND,
        &[(ANTIPOISON_ALIAS, 1)],
        &[("cookquest", 0)],
        vec![bank_seed_step(RUNE_PICKAXE_ALIAS)],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Mining,
        "Mining",
        MINING_STAT,
        Some(COPPER_ORE_ID),
        Some(RUNE_PICKAXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        MINING_STAND,
    );
    {
        let mut witness = witness.lock().expect("survival witness");
        witness.expected_tool_at_start = 0;
        witness.poison_during_provisioning = true;
    }
    add_login_timer_wait_before_start(&mut cell.scenario, Arc::clone(&witness));
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Quester", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "account-state-debugproc");
    assert_eq!(proof["poison_source_chat_modal"], -1);
    assert_eq!(proof["resumed_step_id"], "survival-work");
    assert_eq!(proof["tool_count_at_start"], 0);
    assert_eq!(proof["poison_seed_tool_count"], 0);
    assert_eq!(proof["poison_seed_bank_open"], true);
    assert_eq!(proof["tool_count_after"], 1);
    assert!(
        proof["provisioning_tick"].as_u64().unwrap()
            <= proof["poison_source_tick"].as_u64().unwrap()
    );
    assert!(
        proof["work_active_tick"].as_u64().unwrap() > proof["poison_source_tick"].as_u64().unwrap()
    );
    assert_common_cure(proof, 120);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_quester_dialogue_updates_policy_on_desert_captain_page() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "desert-captain-dialogue",
        json!([{
            "id":"survival-talk",
            "kind":"talk",
            "version":1,
            "advances":false,
            "args":{
                "npc":"desertminingcaptain",
                "prefer":[
                    "Wow! A real captain!",
                    "I'd love to work for a tough guy like you!",
                    "Can't I do something for a strong Captain like you?",
                    "Sorry Sir, I don't think I can do that.",
                    "It's a funny captain who can't fight his own battles!"
                ]
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        Activity::Dialogue,
        "L-CURE-QUESTER-DIALOGUE",
        DESERT_CAPTAIN_STAND,
        &[(ANTIPOISON_ALIAS, 1), (SINISTER_KEY_ALIAS, 1)],
        &[("cookquest", 0), ("desertrescue", 3)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Dialogue,
        "Mining",
        MINING_STAT,
        None,
        None,
        PoisonSource::TwistedChest,
        DESERT_CAPTAIN_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), true, 40);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::Never,
        dialogue_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "yanille-twisted-chest");
    assert_eq!(proof["sinister_key_at_start"], 0);
    assert_eq!(proof["sinister_key_after"], 0);
    assert_eq!(
        proof["chest_reward_count_at_start"],
        TWISTED_CHEST_REWARD_COUNT
    );
    assert_eq!(
        proof["inventory_total_at_start"],
        1 + TWISTED_CHEST_REWARD_COUNT
    );
    assert_eq!(proof["inventory_total_after"], TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["npc_config"], "desertminingcaptain");
    assert_eq!(proof["npc_stage"], DESERT_STAGE_REPLY);
    assert!(proof["npc_stage_tick"].as_u64().is_some());
    assert_eq!(
        proof["dialogue_options_at_need"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(
        proof["profile_change_options"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(
        proof["dialogue_options_after_resume"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(proof["resumed_step_id"], "survival-talk");
    assert!(proof["profile_change_chat_modal"]
        .as_i64()
        .is_some_and(|modal| modal != -1));
    assert!(!proof["profile_change_options"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        proof["dialogue_closed_tick"].as_u64().unwrap()
            > proof["profile_change_tick"].as_u64().unwrap()
    );
    assert!(
        proof["dialogue_reopened_tick"].as_u64().unwrap()
            > proof["antipoison_settled_tick"].as_u64().unwrap()
    );
    assert_common_cure(proof, 150);
    let transitions = receipt["transitions"]
        .as_array()
        .expect("transition receipt");
    let change = transitions
        .iter()
        .find(|transition| {
            transition["event"] == "profile-command"
                && transition["command"] == "SetSurvival"
                && transition["setting"] == "cure_poison"
        })
        .expect("profile command transition");
    assert_eq!(change["before"], "never");
    assert_eq!(change["after"], "when_needed");
    assert_eq!(change["slot_mirror_policy"], "when_needed");
    assert_eq!(change["slot_mirror_tick"], change["tick"]);
    assert_eq!(change["slot_mirror_updated"], true);
    assert_eq!(change["tick"], proof["profile_change_tick"]);
    assert_eq!(change["chat_options"], proof["profile_change_options"]);
    assert!(change["chat_modal"]
        .as_i64()
        .is_some_and(|modal| modal != -1));
    assert!(!change["chat_options"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Never, park and bite-after-drink cells (design-final.md:947-950; review
// SURVIVAL-NATIVE-CURE-FIX R2). Each is a Gatherer at the real Draynor willow,
// poisoned with the account-state debug procedure after the login timer
// cleared. Their oracles read the same frames as the cure cells plus the
// per-frame survival row kept by `capture_track`.
// ---------------------------------------------------------------------------

/// Lobster reference heal of the park's food alternative (design A7,
/// `survival/math.rs` `survival_need` with no food held).
const PARK_REF_HEAL: i32 = 12;
/// The worst-case poison total after a mark of 6 (design-final.md:948:
/// `ceil((99 - 54 + 1) / 12) = 4`).
const OUTLOOK_AFTER_MARK_6: i32 = 99;
/// Marks of `~poison 27`: the timer queues `(%poison + 4) / 5` and then
/// lowers `%poison` by one (`poison.rs2:20-30`): 27, 26, 25 give 6, 6, 5.
const POISON_27_MARKS: [i32; 3] = [6, 6, 5];
/// Marks of `~poison 22`: 22 down to 15 give 5, 5, 4, 4, 4, 4, 4, 3.
const POISON_22_MARKS: [i32; 8] = [5, 5, 4, 4, 4, 4, 4, 3];
const POISON_22_COMMAND: &str = "~poison 22";
/// `cheat_other.rs2:22-23` queues `damage_player`, which damages the account
/// itself (`damage.rs2:21-22`).
const HIT_20_COMMAND: &str = "~hit 20";
const LOBSTER_ALIAS: &str = "lobster";
/// `fishing.obj:130-140`: Shrimps, `category_58`, eaten by
/// `@player_consume_item(consume_effect_stat, 2, 3)` (`consume.rs2:4`), healing
/// 3 (`consume_normal.dbrow:76-80`) with no `message_delay` row.
const SHRIMP_ALIAS: &str = "shrimp";
const SHRIMP_HEAL: i32 = 3;
/// The drink run in tick `D` delays the player until `D + 2`
/// (`consume_effects.rs2:124-126`, `PlayerOps.ts:376-379`), and the engine
/// drops a held click run while delayed (`OpHeldHandler.ts:16-19`), so the
/// first bite runs in `D + 3` at the earliest.
const BITE_AFTER_DRINK_MIN_TICKS: u32 = 3;
/// The shrimp's `$eat_delay` of 2 (`consume.rs2:4`, `:122-127`) refuses a bite
/// while `%eat_delay >= map_clock`, so the next bite runs 3 ticks later at
/// the earliest.
const NEXT_BITE_MIN_TICKS: u32 = 3;
/// Two observed ticks of slack above each minimum. A click the engine drops
/// costs the runner its food op's eight-tick bound (`gatherer/oneop.rs`
/// `OneOpArgs::eat`) or the drink's three-tick bound before the resend, so a
/// dropped click cannot fit inside it.
const CLICK_SLACK_TICKS: u32 = 2;

/// One frame of a never, park or food cell, kept when a tracked field changes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackFrame {
    tick: u32,
    hp: i32,
    food: i32,
    antipoison: i32,
    xp: i32,
    decision: Option<Decision>,
    why: Option<CureWhy>,
    need: Option<SurvivalNeed>,
    owner: Option<Owner>,
    survival: Option<String>,
    phase_label: Option<String>,
    native_phase: Option<NativePhase>,
    run: Option<String>,
    failure: Option<String>,
}

impl TrackFrame {
    fn same_state(&self, other: &Self) -> bool {
        Self {
            tick: 0,
            xp: 0,
            ..self.clone()
        } == Self {
            tick: 0,
            xp: 0,
            ..other.clone()
        }
    }

    fn json(&self) -> Value {
        json!({
            "tick": self.tick,
            "hp": self.hp,
            "food": self.food,
            "antipoison": self.antipoison,
            "xp": self.xp,
            "decision": self.decision.map(|decision| format!("{decision:?}")),
            "why": self.why.map(|why| format!("{why:?}")),
            "need": self.need.map(|need| format!("{need:?}")),
            "owner": self.owner.map(|owner| format!("{owner:?}")),
            "survival": self.survival,
            "phase": self.phase_label,
            "native_phase": self.native_phase.map(|phase| format!("{phase:?}")),
            "run": self.run,
            "failure": self.failure,
        })
    }
}

/// One food count drop: the bite's content signal (`consume.rs2` deletes the
/// item and heals in the tick that runs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FoodDrop {
    tick: u32,
    from: i32,
    to: i32,
    hp_before: i32,
    hp_after: i32,
}

/// The survival row, HP and counts observed on the frame of each poison mark.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MarkRow {
    tick: u32,
    value: i32,
    hp: i32,
    food: i32,
    xp: i32,
    decision: Option<Decision>,
    survival: Option<String>,
}

#[derive(Default)]
struct SurvivalTrack {
    frames: Vec<TrackFrame>,
    park_frame: Option<TrackFrame>,
    unpark_frame: Option<TrackFrame>,
    repark_tick: Option<u32>,
    park_texts: Vec<(u32, i32, String)>,
    needed_lethal_tick: Option<u32>,
    drinking_tick: Option<u32>,
    blocked_tick: Option<u32>,
    failure_tick: Option<u32>,
    runs: Vec<String>,
    min_hp: Option<i32>,
    give_tick: Option<u32>,
    dose_seen_tick: Option<u32>,
    hit_tick: Option<u32>,
    food_drops: Vec<FoodDrop>,
    mark_rows: Vec<MarkRow>,
    last: Option<(i32, i32)>,
}

fn stat_effective(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.effective)
}

fn capture_track(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    slot: Option<&host_play::SlotStatus>,
    witness: &mut Witness,
) {
    if witness.poison_source_tick.is_none() || !snapshot.ingame() {
        return;
    }
    let report = slot.map(|slot| slot.survival);
    let frame = TrackFrame {
        tick: snapshot.tick(),
        hp: stat_effective(snapshot, HITPOINTS_STAT),
        food: witness.food_id.map_or(0, |id| item_count(snapshot, id)),
        antipoison: item_count(snapshot, witness.antipoison_id),
        xp: stat_xp(snapshot, witness.skill_stat),
        decision: report.map(|report| report.decision),
        why: report.and_then(|report| report.why),
        need: report.and_then(|report| report.need),
        owner: report.map(|report| report.owner),
        survival: status_text(status, "survival").map(str::to_owned),
        phase_label: status_text(status, "phase").map(str::to_owned),
        native_phase: status.map(|status| status.phase),
        run: status.map(|status| format!("{:?}", status.run)),
        failure: status
            .and_then(|status| status.failure.as_ref())
            .map(|failure| format!("{failure:?}")),
    };
    let tick = frame.tick;
    let expected_antipoison = witness.expected_antipoison;
    let track = &mut witness.track;
    track.min_hp = Some(track.min_hp.map_or(frame.hp, |hp| hp.min(frame.hp)));
    if let Some(run) = &frame.run {
        if !track.runs.contains(run) {
            track.runs.push(run.clone());
        }
    }
    if frame.native_phase == Some(NativePhase::Blocked) {
        track.blocked_tick.get_or_insert(tick);
    }
    if frame.failure.is_some() {
        track.failure_tick.get_or_insert(tick);
    }
    // The Gatherer publishes the phase label `parked` for as long as its
    // hold lasts (`gatherer/runner.rs` `phase`), also while the report shows
    // a bite it takes during the hold.
    match frame.phase_label.as_deref() {
        Some("parked") => {
            if track.park_frame.is_none() {
                track.park_frame = Some(frame.clone());
            } else if track.unpark_frame.is_some() {
                track.repark_tick.get_or_insert(tick);
            }
        }
        Some(_) if track.park_frame.is_some() && track.unpark_frame.is_none() => {
            track.unpark_frame = Some(frame.clone());
        }
        _ => {}
    }
    if let (Some(Decision::Parked), Some(text)) = (frame.decision, &frame.survival) {
        if track.park_texts.last().map(|(_, _, last)| last) != Some(text) {
            track.park_texts.push((tick, frame.hp, text.clone()));
        }
    }
    if frame.decision == Some(Decision::Drinking) {
        track.drinking_tick.get_or_insert(tick);
    }
    if frame.decision == Some(Decision::Needed)
        && report.is_some_and(|report| lethal_cure_report(&report))
    {
        track.needed_lethal_tick.get_or_insert(tick);
    }
    if expected_antipoison == 0 && frame.antipoison > 0 {
        track.dose_seen_tick.get_or_insert(tick);
    }
    if track.hit_tick.is_none() && active_hitmark(snapshot, Some(1)) == Some(true) {
        track.hit_tick = Some(tick);
    }
    if let Some((food, hp)) = track.last {
        if frame.food < food {
            track.food_drops.push(FoodDrop {
                tick,
                from: food,
                to: frame.food,
                hp_before: hp,
                hp_after: frame.hp,
            });
        }
    }
    track.last = Some((frame.food, frame.hp));
    while let Some(mark) = witness.poison_marks.get(track.mark_rows.len()) {
        track.mark_rows.push(MarkRow {
            tick: mark.tick,
            value: mark.value,
            hp: frame.hp,
            food: frame.food,
            xp: frame.xp,
            decision: frame.decision,
            survival: frame.survival.clone(),
        });
    }
    if track
        .frames
        .last()
        .is_none_or(|last| !last.same_state(&frame))
    {
        track.frames.push(frame);
    }
}

/// Hostlog lines the park oracles read: the park transitions and any Script
/// line that names a block or an operator stop. The sink keeps only those.
static SURVIVAL_LOG: Mutex<Vec<Value>> = Mutex::new(Vec::new());
static SURVIVAL_LOG_SINK: SurvivalLogSink = SurvivalLogSink;
struct SurvivalLogSink;

impl api::hostlog::Sink for SurvivalLogSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let lower = record.message.to_ascii_lowercase();
        let script = matches!(record.source, api::hostlog::Source::Script);
        if record.message.starts_with("survival park")
            || record.message.starts_with("survival unpark")
            || lower.contains("operator stop")
            || (script && lower.contains("blocked"))
        {
            if let Ok(mut lines) = SURVIVAL_LOG.lock() {
                lines.push(json!({
                    "slot": record.slot,
                    "tick": record.tick,
                    "source": format!("{:?}", record.source),
                    "message": record.message,
                }));
            }
        }
    }
}

fn install_survival_log() {
    assert!(
        api::hostlog::install_sink(&SURVIVAL_LOG_SINK),
        "the cell's hostlog sink is the process's first"
    );
}

fn survival_log() -> Vec<Value> {
    SURVIVAL_LOG
        .lock()
        .map(|lines| lines.clone())
        .unwrap_or_default()
}

/// Counts of the park and unpark lines, and every block or stop line.
fn check_survival_log(lines: &[Value], parks: usize, unparks: usize) -> Result<(), String> {
    let count = |prefix: &str| {
        lines
            .iter()
            .filter(|line| {
                line["message"]
                    .as_str()
                    .is_some_and(|message| message.starts_with(prefix))
            })
            .count()
    };
    let stops = lines
        .iter()
        .filter(|line| {
            line["message"].as_str().is_some_and(|message| {
                !message.starts_with("survival park") && !message.starts_with("survival unpark")
            })
        })
        .count();
    if count("survival park ") != parks || count("survival unpark ") != unparks || stops != 0 {
        return Err(format!(
            "hostlog wants {parks} park and {unparks} unpark lines and no block or stop line: {lines:?}"
        ));
    }
    Ok(())
}

/// The park's food alternative at the design's reference heal:
/// `ceil((R - hp + 1) / 12)` (design A7, design-final.md:948). Valid while
/// the Gatherer's eating line leaves at least 12 HP of room below the base.
fn park_food_alternative(total: i32, hp: i32) -> i32 {
    (total - hp + 1 + PARK_REF_HEAL - 1).div_euclid(PARK_REF_HEAL)
}

/// The park report at the line: `Parked` with a lethal cure need whose food
/// alternative matches the HP of that frame, shown as the status text.
fn check_park_frame(frame: &TrackFrame, total: i32) -> Result<i32, String> {
    let or_food = park_food_alternative(total, frame.hp);
    let text = format!("Parked: poisoned: would die; needs antipoison or {or_food} food");
    let need_ok = matches!(
        frame.need,
        Some(SurvivalNeed::Cure { or_food: need, ref_heal, why: NeedWhy::Lethal })
            if i32::from(need) == or_food && i32::from(ref_heal) == PARK_REF_HEAL
    );
    if frame.decision != Some(Decision::Parked)
        || !need_ok
        || frame.survival.as_deref() != Some(text.as_str())
        || frame.phase_label.as_deref() != Some("parked")
        || frame.native_phase != Some(NativePhase::Waiting)
        || frame.failure.is_some()
    {
        return Err(format!(
            "park frame does not show the hold `{text}`: {:?}",
            frame
        ));
    }
    Ok(or_food)
}

/// The hold ends inside `[mark, mark + bound]`, never before that mark.
fn check_unpark_window(
    unpark: Option<u32>,
    mark: u32,
    bound: u32,
    now: u32,
) -> Result<Option<u32>, String> {
    match unpark {
        Some(unpark) if unpark < mark => Err(format!(
            "the hold ended at {unpark}, before the release mark at {mark}"
        )),
        Some(unpark) if unpark > mark + bound => Err(format!(
            "the hold ended at {unpark}, more than {bound} ticks after the release mark at {mark}"
        )),
        Some(unpark) => Ok(Some(unpark)),
        None if now > mark + bound => Err(format!(
            "no unpark within {bound} ticks of the release mark at {mark} (now {now})"
        )),
        None => Ok(None),
    }
}

/// Bites after a drink line at `drink`: the first in
/// `[drink + 3, drink + 3 + slack]`, each next one in `[+3, +3 + slack]`.
fn check_bite_spacing(drink: u32, drops: &[FoodDrop]) -> Result<(), String> {
    let mut previous = drink;
    for (index, drop) in drops.iter().enumerate() {
        let minimum = if index == 0 {
            BITE_AFTER_DRINK_MIN_TICKS
        } else {
            NEXT_BITE_MIN_TICKS
        };
        let gap = drop.tick.saturating_sub(previous);
        if drop.tick < previous || gap < minimum || gap > minimum + CLICK_SLACK_TICKS {
            return Err(format!(
                "bite {} at {} is {gap} ticks after {previous}; the content and runner window is {minimum}..={}: drops {drops:?}",
                index + 1,
                drop.tick,
                minimum + CLICK_SLACK_TICKS
            ));
        }
        if drop.from - drop.to != 1 || drop.hp_after <= drop.hp_before {
            return Err(format!(
                "bite {} did not eat one item and heal: {drop:?}",
                index + 1
            ));
        }
        previous = drop.tick;
    }
    Ok(())
}

fn mark_values(marks: &[PoisonMark]) -> Vec<i32> {
    marks.iter().map(|mark| mark.value).collect()
}

/// The facts every never, park and food cell checks on every frame: no
/// `Blocked` phase, no failure, one run key, no lifecycle receipt, no
/// death, inside the cell's deadline from the poison line.
fn check_track_common(
    snapshot: &GameSnapshot,
    witness: &Witness,
    deadline: u32,
) -> Result<(), String> {
    let track = &witness.track;
    if let Some(tick) = track.blocked_tick {
        return Err(format!("the run published a Blocked phase at {tick}"));
    }
    if let Some(tick) = track.failure_tick {
        return Err(format!(
            "the run published a failure at {tick}: {:?}",
            track.frames.last()
        ));
    }
    if track.runs.len() > 1 {
        return Err(format!("the run key changed: {:?}", track.runs));
    }
    if let Some(lifecycle) = &witness.lifecycle_seen {
        return Err(format!("a lifecycle receipt was written: {lifecycle}"));
    }
    if track.min_hp.is_some_and(|hp| hp <= 0) {
        return Err("the account died".into());
    }
    if witness.poison_tick.is_some() {
        check_deadline(witness, snapshot.tick(), deadline)?;
    }
    Ok(())
}

fn track_json(witness: &Witness) -> Value {
    let track = &witness.track;
    json!({
        "frames": track.frames.iter().map(TrackFrame::json).collect::<Vec<_>>(),
        "park": track.park_frame.as_ref().map(TrackFrame::json),
        "unpark": track.unpark_frame.as_ref().map(TrackFrame::json),
        "park_texts": track.park_texts.iter().map(|(tick, hp, text)| json!({"tick": tick, "hp": hp, "text": text})).collect::<Vec<_>>(),
        "mark_rows": track.mark_rows.iter().map(|row| json!({
            "tick": row.tick,
            "value": row.value,
            "hp": row.hp,
            "food": row.food,
            "xp": row.xp,
            "decision": row.decision.map(|decision| format!("{decision:?}")),
            "survival": row.survival,
        })).collect::<Vec<_>>(),
        "food_drops": track.food_drops.iter().map(|drop| json!({
            "tick": drop.tick,
            "from": drop.from,
            "to": drop.to,
            "hp_before": drop.hp_before,
            "hp_after": drop.hp_after,
        })).collect::<Vec<_>>(),
        "runs": track.runs,
        "min_hp": track.min_hp,
        "needed_lethal_tick": track.needed_lethal_tick,
        "drinking_tick": track.drinking_tick,
        "give_tick": track.give_tick,
        "dose_seen_tick": track.dose_seen_tick,
        "hit_tick": track.hit_tick,
        "poison_tick": witness.poison_tick,
        "poison_source_tick": witness.poison_source_tick,
        "login_tick": witness.session.login,
        "timer_phase_ready_tick": witness.timer_phase_ready_tick,
        "poison_marks": witness.poison_marks.iter().map(|mark| json!({"tick": mark.tick, "value": mark.value})).collect::<Vec<_>>(),
        "drink_line_tick": witness.cure_tick,
        "immune_tick": witness.immune_tick,
        "hostlog": survival_log(),
    })
}

/// L-CURE-NEVER (design-final.md:947): no drink; the row reports a lethal
/// cure need from the line; the Gatherer keeps gathering; the cell ends after
/// the third mark, before the fourth.
fn never_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_track_common(snapshot, witness, 100)?;
    let Some(poison) = witness.poison_tick else {
        return Ok(None);
    };
    let track = &witness.track;
    if let Some(tick) = track.drinking_tick {
        return Err(format!("a drink was decided under Never at {tick}"));
    }
    if let Some(tick) = witness.cure_tick {
        return Err(format!(
            "an antipoison drink line came under Never at {tick}"
        ));
    }
    if let Some(frame) = &track.park_frame {
        return Err(format!("the Gatherer parked under Never: {frame:?}"));
    }
    if let Some(frame) = track.frames.iter().find(|frame| frame.antipoison != 1) {
        return Err(format!("the held dose changed under Never: {frame:?}"));
    }
    let need = match track.needed_lethal_tick {
        Some(tick) if tick.saturating_sub(poison) > 3 || tick < poison => {
            return Err(format!("the lethal Needed report came at {tick}, not within 3 ticks of the line at {poison}"));
        }
        Some(tick) => tick,
        None if snapshot.tick().saturating_sub(poison) > 3 => {
            return Err(format!(
                "no lethal Needed report within 3 ticks of the line at {poison}"
            ));
        }
        None => return Ok(None),
    };
    if let Some(frame) = track.frames.iter().find(|frame| {
        frame.tick >= need
            && frame
                .decision
                .is_some_and(|decision| decision != Decision::Needed)
    }) {
        return Err(format!("the row left Needed after the line: {frame:?}"));
    }
    let marks = &witness.poison_marks;
    if marks.len() > POISON_27_MARKS.len() {
        return Err(format!(
            "a fourth mark came before the cell ended: {marks:?}"
        ));
    }
    if marks.len() < POISON_27_MARKS.len() || snapshot.tick() < marks[2].tick + 2 {
        return Ok(None);
    }
    if mark_values(marks) != POISON_27_MARKS {
        return Err(format!(
            "the marks are not the content's 6, 6, 5: {marks:?}"
        ));
    }
    let xp_now = stat_xp(snapshot, witness.skill_stat);
    let xp_at_first_mark = track.mark_rows[0].xp;
    let need_xp = track
        .frames
        .iter()
        .rfind(|frame| frame.tick <= need)
        .map_or(witness.baseline_xp, |frame| frame.xp);
    if xp_now <= xp_at_first_mark || xp_now <= need_xp {
        return Err(format!(
            "the Gatherer did not keep gathering: XP at need {need_xp}, at the first mark {xp_at_first_mark}, now {xp_now}"
        ));
    }
    Ok(Some(json!({
        "cell": "L-CURE-NEVER",
        "policy": "never",
        "poison_line_tick": poison,
        "needed_lethal_tick": need,
        "skill_xp_at_need": need_xp,
        "skill_xp_at_first_mark": xp_at_first_mark,
        "skill_xp_after": xp_now,
        "mark_values": mark_values(marks),
        "antipoison_after": item_count(snapshot, witness.antipoison_id),
        "observed_tick": snapshot.tick(),
        "track": track_json(witness),
    })))
}

/// L-PARK-NO-DOSE (design-final.md:948).
fn park_no_dose_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_track_common(snapshot, witness, 150)?;
    let Some(poison) = witness.poison_tick else {
        return Ok(None);
    };
    let track = &witness.track;
    let now = snapshot.tick();
    let Some(park) = &track.park_frame else {
        return if now.saturating_sub(poison) > 3 {
            Err(format!("no park within 3 ticks of the line at {poison}"))
        } else {
            Ok(None)
        };
    };
    if park.tick < poison || park.tick - poison > 3 {
        return Err(format!(
            "the park came at {}, not within 3 ticks of the line at {poison}",
            park.tick
        ));
    }
    let line_food = check_park_frame(park, i32::from(LETHAL_OUTLOOK))?;
    if let Some(tick) = track.repark_tick {
        return Err(format!("a second park at {tick}"));
    }
    let Some(first_mark) = witness.poison_marks.first() else {
        return Ok(None);
    };
    if first_mark.value != POISON_27_MARKS[0] {
        return Err(format!("the first mark is not 6: {first_mark:?}"));
    }
    let after_mark = track
        .park_texts
        .iter()
        .find(|(tick, _, _)| *tick >= first_mark.tick);
    let Some((text_tick, text_hp, text)) = after_mark else {
        return if now > first_mark.tick + 3 && track.give_tick.is_none() {
            Err(format!(
                "the park text did not change within 3 ticks of the first mark at {}",
                first_mark.tick
            ))
        } else {
            Ok(None)
        };
    };
    let mark_food = park_food_alternative(OUTLOOK_AFTER_MARK_6, *text_hp);
    let mark_text = format!("Parked: poisoned: would die; needs antipoison or {mark_food} food");
    if *text_tick > first_mark.tick + 3 || text != &mark_text {
        return Err(format!(
            "after the first mark at {} the park text is `{text}` at {text_tick}, want `{mark_text}`",
            first_mark.tick
        ));
    }
    if let Some(unpark) = &track.unpark_frame {
        if track.dose_seen_tick.is_none_or(|dose| unpark.tick < dose) {
            return Err(format!(
                "the hold ended before the dose arrived: {unpark:?}"
            ));
        }
    }
    let (Some(give), Some(dose)) = (track.give_tick, track.dose_seen_tick) else {
        return Ok(None);
    };
    let drink = match witness.cure_tick {
        Some(drink) if drink < dose || drink - dose > 3 => {
            return Err(format!(
                "the drink line at {drink} is not within 3 ticks of the dose at {dose}"
            ));
        }
        Some(drink) => drink,
        None if now > dose + 3 => {
            return Err(format!(
                "no drink line within 3 ticks of the dose at {dose}"
            ));
        }
        None => return Ok(None),
    };
    let Some(unpark) = check_unpark_window(
        track.unpark_frame.as_ref().map(|frame| frame.tick),
        dose,
        10,
        now,
    )?
    else {
        return Ok(None);
    };
    let Some(immune) = witness.immune_tick else {
        return Ok(None);
    };
    let unpark_xp = track
        .unpark_frame
        .as_ref()
        .map_or(i32::MAX, |frame| frame.xp);
    let xp_now = stat_xp(snapshot, witness.skill_stat);
    if xp_now <= unpark_xp {
        return Ok(None);
    }
    check_survival_log(&survival_log(), 1, 1)?;
    Ok(Some(json!({
        "cell": "L-PARK-NO-DOSE",
        "poison_line_tick": poison,
        "park_tick": park.tick,
        "park_hp": park.hp,
        "park_food_alternative": line_food,
        "park_text": park.survival,
        "first_mark": {"tick": first_mark.tick, "value": first_mark.value},
        "park_text_after_first_mark": {"tick": text_tick, "hp": text_hp, "text": text, "food_alternative": mark_food},
        "give_tick": give,
        "dose_seen_tick": dose,
        "drink_line_tick": drink,
        "immune_tick": immune,
        "unpark_tick": unpark,
        "skill_xp_at_unpark": unpark_xp,
        "skill_xp_after": xp_now,
        "run_keys": track.runs,
        "observed_tick": now,
        "track": track_json(witness),
    })))
}

/// L-PARK-FALSE-RELEASE (design-final.md:949): held through two marks of 6,
/// released within 3 ticks of the mark of 5 with no rescue item.
fn park_false_release_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_track_common(snapshot, witness, 150)?;
    let Some(poison) = witness.poison_tick else {
        return Ok(None);
    };
    let track = &witness.track;
    let now = snapshot.tick();
    let Some(park) = &track.park_frame else {
        return if now.saturating_sub(poison) > 3 {
            Err(format!("no park within 3 ticks of the line at {poison}"))
        } else {
            Ok(None)
        };
    };
    if park.tick < poison || park.tick - poison > 3 {
        return Err(format!(
            "the park came at {}, not within 3 ticks of the line at {poison}",
            park.tick
        ));
    }
    let line_food = check_park_frame(park, i32::from(LETHAL_OUTLOOK))?;
    if let Some(tick) = track.repark_tick {
        return Err(format!("a second park at {tick}"));
    }
    if let Some(frame) = track
        .frames
        .iter()
        .find(|frame| frame.antipoison != 0 || frame.food != 0)
    {
        return Err(format!("a rescue item appeared: {frame:?}"));
    }
    let marks = &witness.poison_marks;
    if marks.len() > POISON_27_MARKS.len() {
        return Err(format!("a fourth mark came before the release: {marks:?}"));
    }
    let unpark = track.unpark_frame.as_ref().map(|frame| frame.tick);
    let Some(third) = marks.get(2) else {
        if let Some(unpark) = unpark {
            return Err(format!(
                "the hold ended at {unpark}, before the third mark: {marks:?}"
            ));
        }
        return Ok(None);
    };
    if mark_values(marks) != POISON_27_MARKS {
        return Err(format!(
            "the marks are not the content's 6, 6, 5: {marks:?}"
        ));
    }
    let Some(unpark) = check_unpark_window(unpark, third.tick, 3, now)? else {
        return Ok(None);
    };
    let unpark_xp = track
        .unpark_frame
        .as_ref()
        .map_or(i32::MAX, |frame| frame.xp);
    let xp_now = stat_xp(snapshot, witness.skill_stat);
    if xp_now <= unpark_xp {
        return Ok(None);
    }
    check_survival_log(&survival_log(), 1, 1)?;
    Ok(Some(json!({
        "cell": "L-PARK-FALSE-RELEASE",
        "poison_line_tick": poison,
        "park_tick": park.tick,
        "park_hp": park.hp,
        "park_food_alternative": line_food,
        "mark_values": mark_values(marks),
        "third_mark_tick": third.tick,
        "third_mark_after_line_ticks": third.tick.saturating_sub(poison),
        "hp_at_third_mark": track.mark_rows.get(2).map(|row| row.hp),
        "unpark_tick": unpark,
        "skill_xp_at_unpark": unpark_xp,
        "skill_xp_after": xp_now,
        "observed_tick": now,
        "track": track_json(witness),
    })))
}

/// L-PARK-FOOD-RELEASE (design-final.md:950): held through fires 1 to 7 while
/// it eats, released within 6 ticks of the eighth mark with four lobsters.
fn park_food_release_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_track_common(snapshot, witness, 300)?;
    let Some(poison) = witness.poison_tick else {
        return Ok(None);
    };
    let track = &witness.track;
    let now = snapshot.tick();
    let Some(park) = &track.park_frame else {
        return if now.saturating_sub(poison) > 3 {
            Err(format!("no park within 3 ticks of the line at {poison}"))
        } else {
            Ok(None)
        };
    };
    if park.tick < poison || park.tick - poison > 3 || park.decision != Some(Decision::Parked) {
        return Err(format!(
            "the park came at {}, not within 3 ticks of the line at {poison}",
            park.tick
        ));
    }
    if let Some(tick) = track.repark_tick {
        return Err(format!("a second park at {tick}"));
    }
    let marks = &witness.poison_marks;
    if marks.len() > POISON_22_MARKS.len() {
        return Err(format!("a ninth mark came before the release: {marks:?}"));
    }
    if mark_values(marks) != POISON_22_MARKS[..marks.len()] {
        return Err(format!(
            "the marks are not the content's 5, 5, 4, 4, 4, 4, 4, 3: {marks:?}"
        ));
    }
    let unpark = track.unpark_frame.as_ref().map(|frame| frame.tick);
    let Some(eighth) = marks.get(7) else {
        if let Some(unpark) = unpark {
            return Err(format!(
                "the hold ended at {unpark}, before the eighth mark: {marks:?}"
            ));
        }
        return Ok(None);
    };
    let Some(unpark) = check_unpark_window(unpark, eighth.tick, 6, now)? else {
        return Ok(None);
    };
    let unpark_frame = track.unpark_frame.as_ref().ok_or("unpark frame")?;
    let food_left = track
        .frames
        .iter()
        .rfind(|frame| frame.tick <= unpark)
        .map_or(unpark_frame.food, |frame| frame.food);
    if food_left != 4 {
        return Err(format!(
            "{food_left} lobsters left at the unpark, not 4: drops {:?}",
            track.food_drops
        ));
    }
    let xp_now = stat_xp(snapshot, witness.skill_stat);
    if xp_now <= unpark_frame.xp {
        return Ok(None);
    }
    check_survival_log(&survival_log(), 1, 1)?;
    Ok(Some(json!({
        "cell": "L-PARK-FOOD-RELEASE",
        "poison_line_tick": poison,
        "park_tick": park.tick,
        "park_text": park.survival,
        "mark_values": mark_values(marks),
        "eighth_mark_tick": eighth.tick,
        "fifth_mark_tick": marks[4].tick,
        "unpark_tick": unpark,
        "lobsters_left_at_unpark": food_left,
        "bite_hp_before": track.food_drops.iter().map(|drop| drop.hp_before).collect::<Vec<_>>(),
        "skill_xp_at_unpark": unpark_frame.xp,
        "skill_xp_after": xp_now,
        "observed_tick": now,
        "track": track_json(witness),
    })))
}

/// L-CURE-BITE-AFTER-DRINK: a cure and a bite due together. The drink goes
/// first; the first bite waits for the drink's delay counted from its line;
/// each next bite waits for the next-food clock counted from the previous
/// bite's count drop. Each gap stays inside the content minimum plus two
/// ticks, so no click was dropped and resent. Gathering resumes after.
fn bite_after_drink_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_track_common(snapshot, witness, 120)?;
    check_mark(witness)?;
    let Some(poison) = witness.poison_tick else {
        return Ok(None);
    };
    let track = &witness.track;
    let now = snapshot.tick();
    let Some(drink) = witness.cure_tick else {
        return if now.saturating_sub(poison) > CURE_LINE_BOUND_TICKS {
            Err(format!(
                "no drink line within {CURE_LINE_BOUND_TICKS} ticks of the line at {poison}"
            ))
        } else {
            Ok(None)
        };
    };
    if let Some(first) = track.food_drops.first() {
        if first.tick < drink {
            return Err(format!(
                "a bite came before the drink line at {drink}: {first:?}"
            ));
        }
    }
    check_bite_spacing(drink, &track.food_drops)?;
    let last = match track.food_drops.last() {
        Some(last) if track.food_drops.len() >= 2 => *last,
        _ if now
            > drink + BITE_AFTER_DRINK_MIN_TICKS + NEXT_BITE_MIN_TICKS + 2 * CLICK_SLACK_TICKS =>
        {
            return Err(format!(
                "fewer than two bites after the drink at {drink}: {:?}",
                track.food_drops
            ));
        }
        _ => return Ok(None),
    };
    if now <= last.tick + NEXT_BITE_MIN_TICKS + CLICK_SLACK_TICKS {
        return Ok(None);
    }
    let xp_at_last = track
        .frames
        .iter()
        .rfind(|frame| frame.tick <= last.tick)
        .map_or(i32::MAX, |frame| frame.xp);
    let xp_now = stat_xp(snapshot, witness.skill_stat);
    if xp_now <= xp_at_last {
        return Ok(None);
    }
    if witness.dose_drop_tick != Some(drink) {
        return Err(format!(
            "the dose drop at {:?} is not the drink line's tick {drink}",
            witness.dose_drop_tick
        ));
    }
    Ok(Some(json!({
        "cell": "L-CURE-BITE-AFTER-DRINK",
        "poison_line_tick": poison,
        "hit_tick": track.hit_tick,
        "drink_line_tick": drink,
        "dose_drop_tick": witness.dose_drop_tick,
        "immune_tick": witness.immune_tick,
        "bite_ticks": track.food_drops.iter().map(|drop| drop.tick).collect::<Vec<_>>(),
        "bite_gaps": track.food_drops.iter().scan(drink, |previous, drop| {
            let gap = drop.tick - *previous;
            *previous = drop.tick;
            Some(gap)
        }).collect::<Vec<_>>(),
        "bite_window": {
            "first_after_drink_min": BITE_AFTER_DRINK_MIN_TICKS,
            "next_bite_min": NEXT_BITE_MIN_TICKS,
            "slack": CLICK_SLACK_TICKS,
        },
        "shrimp_heal": SHRIMP_HEAL,
        "skill_xp_at_last_bite": xp_at_last,
        "skill_xp_after": xp_now,
        "poison_marks": witness.poison_marks.iter().map(|mark| json!({"tick": mark.tick, "value": mark.value})).collect::<Vec<_>>(),
        "observed_tick": now,
        "track": track_json(witness),
    })))
}

fn insert_after_poison_seed(scenario: &mut scenario::Scenario, steps: Vec<scenario::Step>) {
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("seed account-owned poison state"))
        .expect("the cell seeds its poison");
    scenario.steps.splice(seed + 1..seed + 1, steps);
}

/// L-PARK-NO-DOSE's rescue: once the park text has changed after the first
/// mark, give the account one antipoison dose (`give`, its own inventory).
fn give_dose_steps(witness: Arc<Mutex<Witness>>) -> Vec<scenario::Step> {
    let ready_witness = Arc::clone(&witness);
    let give_witness = Arc::clone(&witness);
    vec![
        scenario::Step {
            name: "wait for the park text after the first poison mark",
            kind: scenario::StepKind::Await {
                evidence: "a park text observed after the first kind-2 mark",
                ready: Box::new(move |_| {
                    let witness = ready_witness.lock().expect("survival witness");
                    witness.poison_marks.first().is_some_and(|mark| {
                        witness
                            .track
                            .park_texts
                            .iter()
                            .any(|(tick, _, _)| *tick >= mark.tick)
                    })
                }),
                summary: None,
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat {
                    id: HITPOINTS_STAT,
                    min: 0,
                },
                // The first mark comes 30 ticks after the line
                // (`poison.rs2:69-75`), plus the text's own tick.
                budget_ticks: TIMER_CLEAR_TICKS + 10,
            },
        },
        scenario::Step {
            name: "give the parked account one antipoison dose",
            kind: scenario::StepKind::Perform {
                send: Box::new(move |client, snapshot| {
                    let command = format!("give {ANTIPOISON_ALIAS} 1");
                    if !api::interact::cheat(client, &command).is_sent() {
                        return false;
                    }
                    give_witness
                        .lock()
                        .expect("survival witness")
                        .track
                        .give_tick = Some(snapshot.tick());
                    true
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat {
                    id: HITPOINTS_STAT,
                    min: 0,
                },
                budget_ticks: 1,
            },
        },
    ]
}

/// A pre-Start step that lowers the account's current Hitpoints with its own
/// `~hit` (`cheat_other.rs2:22-23`, `damage.rs2:21-22`).
fn hit_before_start_step(amount: i32, expect_at_most: i32) -> scenario::Step {
    let command = format!("~hit {amount}");
    scenario::Step {
        name: "lower the account's current Hitpoints with its own hit",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::StatAtMost {
                id: HITPOINTS_STAT,
                max: expect_at_most,
            },
            budget_ticks: 5,
        },
    }
}

fn gatherer_willow_settings(eat_below: i32) -> Map<String, Value> {
    serde_json::from_value(json!({
        "skill":"Woodcutting",
        "woodcuttingResources":["willow"],
        "miningResources":[],
        "targetPreference":"Nearest",
        "location":"Start",
        "radius":GATHERER_RADIUS,
        "disposition":"Power",
        "eatBelow":eat_below
    }))
    .expect("Gatherer settings")
}

fn alias_id(alias: &str) -> i32 {
    api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected R289 data")
        .item_by_alias(alias)
        .unwrap_or_else(|| panic!("missing selected item alias {alias}"))
        .id
}

fn track_cell(
    label: &str,
    hitpoints: i32,
    items: &[(&str, i32)],
    antipoison: i32,
    food: Option<(&str, i32)>,
    poison_commands: Vec<&'static str>,
    deadline: u32,
) -> (Cell, Arc<Mutex<Witness>>) {
    let mut cell = fixture_with_hp(
        Activity::Woodcutting,
        label,
        GATHERER_STAND,
        items,
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
        hitpoints,
    );
    let witness = new_witness(
        Activity::Woodcutting,
        "Woodcutting",
        WOODCUTTING_STAT,
        Some(WILLOW_LOG_ID),
        Some(RUNE_AXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        GATHERER_STAND,
    );
    {
        let mut witness = witness.lock().expect("survival witness");
        witness.expected_hp = hitpoints;
        witness.expected_antipoison = antipoison;
        witness.food_id = food.map(|(alias, _)| alias_id(alias));
        witness.expected_food_at_start = food.map_or(0, |(_, count)| count);
        witness.poison_commands = poison_commands;
    }
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, deadline);
    install_survival_log();
    (cell, witness)
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_never_reports_the_need_and_keeps_gathering() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let (cell, witness) = track_cell(
        "L-CURE-NEVER",
        60,
        &[(RUNE_AXE_ALIAS, 1), (ANTIPOISON_ALIAS, 1)],
        1,
        None,
        vec![POISON_DEBUG_COMMAND],
        100,
    );
    let receipt = run_family(
        cell,
        "Gatherer",
        gatherer_willow_settings(0),
        Arc::clone(&witness),
        vault::CurePoison::Never,
        never_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["cell"], "L-CURE-NEVER");
    assert_eq!(proof["mark_values"], json!(POISON_27_MARKS));
    assert_eq!(proof["antipoison_after"], 1);
    assert!(
        proof["skill_xp_after"].as_i64().unwrap()
            > proof["skill_xp_at_first_mark"].as_i64().unwrap()
    );
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_park_no_dose_holds_then_drinks_a_given_dose_and_resumes() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    // Design-final.md:948: max HP at least 70, HP 60. The account seeds
    // Hitpoints 70 and takes one 10-damage hit of its own before Start.
    let (mut cell, witness) = track_cell(
        "L-PARK-NO-DOSE",
        70,
        &[(RUNE_AXE_ALIAS, 1)],
        0,
        None,
        vec![POISON_DEBUG_COMMAND],
        150,
    );
    insert_before_start(&mut cell.scenario, vec![hit_before_start_step(10, 60)]);
    insert_after_poison_seed(&mut cell.scenario, give_dose_steps(Arc::clone(&witness)));
    let receipt = run_family(
        cell,
        "Gatherer",
        gatherer_willow_settings(0),
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        park_no_dose_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["cell"], "L-PARK-NO-DOSE");
    assert_eq!(proof["run_keys"].as_array().map(Vec::len), Some(1));
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_unpark"].as_i64().unwrap()
    );
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_park_false_release_unparks_at_the_mark_of_five() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let (cell, witness) = track_cell(
        "L-PARK-FALSE-RELEASE",
        90,
        &[(RUNE_AXE_ALIAS, 1)],
        0,
        None,
        vec![POISON_DEBUG_COMMAND],
        150,
    );
    let receipt = run_family(
        cell,
        "Gatherer",
        gatherer_willow_settings(0),
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        park_false_release_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["cell"], "L-PARK-FALSE-RELEASE");
    assert_eq!(proof["mark_values"], json!(POISON_27_MARKS));
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_unpark"].as_i64().unwrap()
    );
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_park_food_release_eats_through_the_hold_and_unparks_at_the_eighth_mark() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let (cell, witness) = track_cell(
        "L-PARK-FOOD-RELEASE",
        10,
        &[(RUNE_AXE_ALIAS, 1), (LOBSTER_ALIAS, 9)],
        0,
        Some((LOBSTER_ALIAS, 9)),
        vec![POISON_22_COMMAND],
        300,
    );
    let receipt = run_family(
        cell,
        "Gatherer",
        gatherer_willow_settings(1),
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        park_food_release_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["cell"], "L-PARK-FOOD-RELEASE");
    assert_eq!(proof["mark_values"], json!(POISON_22_MARKS));
    assert_eq!(proof["lobsters_left_at_unpark"], 4);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_bite_after_drink_waits_for_the_drink_and_the_next_food_clock() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    // HP 30 of 30 holds no bite before the poison. The account's own hit of
    // 20 goes out with the poison, so the cure and a bite are due together:
    // HP 10 is below the Gatherer's default eating line of 15 (half the base,
    // `gatherer/runner.rs` `survival_eat_floor`), and each Shrimps heals 3.
    let (cell, witness) = track_cell(
        "L-CURE-BITE-AFTER-DRINK",
        30,
        &[
            (RUNE_AXE_ALIAS, 1),
            (ANTIPOISON_ALIAS, 1),
            (SHRIMP_ALIAS, 5),
        ],
        1,
        Some((SHRIMP_ALIAS, 5)),
        vec![HIT_20_COMMAND, POISON_DEBUG_COMMAND],
        120,
    );
    let receipt = run_family(
        cell,
        "Gatherer",
        gatherer_willow_settings(0),
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        bite_after_drink_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["cell"], "L-CURE-BITE-AFTER-DRINK");
    assert!(proof["bite_ticks"].as_array().unwrap().len() >= 2);
}

struct SourceRestore(FolderSource);

impl Drop for SourceRestore {
    fn drop(&mut self) {
        registry::set_source(std::mem::replace(
            &mut self.0,
            FolderSource {
                enabled: false,
                folder: PathBuf::new(),
            },
        ));
    }
}

mod oracle_tests {
    use super::*;
    use api::snapshot::{HitmarkView, HitmarksView};

    fn mark(tick: u32) -> PoisonMark {
        PoisonMark {
            tick,
            slot: 0,
            cycle: i32::try_from(tick).unwrap() * 30,
            value: 6,
        }
    }

    #[test]
    fn session_ticks_anchor_the_login_timer_on_the_final_login() {
        let mut session = SessionTicks::default();
        for tick in 1..=20 {
            session.observe(true, tick);
        }
        // The fixture relogs: the harness snapshot restarts its tick.
        session.observe(false, 0);
        for tick in 0..=30 {
            session.observe(true, tick);
            assert!(
                !session.login_timer_cleared(),
                "a timer armed by the final login is still running at {tick}"
            );
        }
        session.observe(true, 31);
        assert!(session.login_timer_cleared());
        assert_eq!(session.login, Some(0));
        assert_eq!(session.logins, 2);
    }

    #[test]
    fn session_ticks_see_a_reconnect_inside_one_drain() {
        let mut session = SessionTicks::default();
        for tick in 0..=40 {
            session.observe(true, tick);
        }
        assert!(session.login_timer_cleared());
        // No logged-out frame: the tick restarts while in game.
        session.observe(true, 2);
        assert!(!session.login_timer_cleared());
        assert_eq!(session.login, Some(2));
        assert_eq!(session.logins, 2);
    }

    #[test]
    fn one_poison_hitmark_spanning_frames_is_one_mark() {
        let empty = HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        };
        let mut view = HitmarksView {
            marks: [empty; 4],
            loop_cycle: 100,
        };
        view.marks[1] = HitmarkView {
            value: 6,
            kind: 2,
            cycle: 170,
        };
        let mut marks = Vec::new();
        record_poison_marks(&view, 34, &mut marks);
        view.loop_cycle = 130;
        record_poison_marks(&view, 35, &mut marks);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].tick, 34);
        // A new hit writes a new cycle into the slot.
        view.marks[1].cycle = 200;
        record_poison_marks(&view, 64, &mut marks);
        assert_eq!(marks.len(), 2);
        // Expired marks and other kinds are not poison damage.
        view.loop_cycle = 300;
        view.marks[2] = HitmarkView {
            value: 3,
            kind: 1,
            cycle: 400,
        };
        record_poison_marks(&view, 70, &mut marks);
        assert_eq!(marks.len(), 2);
    }

    #[test]
    fn a_cure_inside_the_bound_with_no_poison_damage_passes() {
        assert_eq!(check_cure_outcome(Some(40), Some(42), &[], 71), Ok(()));
    }

    #[test]
    fn one_queued_hit_at_the_end_of_the_drink_delay_passes() {
        // Final-round Gatherer receipt shape: poison 31, drink line 32, the
        // hit an earlier fire queued lands when p_delay(1) ends at 34.
        assert_eq!(check_poison_marks(&[mark(34)], Some(32)), Ok(()));
        assert_eq!(
            check_cure_outcome(Some(31), Some(32), &[mark(34)], 62),
            Ok(())
        );
    }

    #[test]
    fn a_cure_that_never_happens_fails() {
        // Final-round Quester receipt shape: poison 26, hit 32, no drink line.
        assert!(check_poison_marks(&[mark(32)], None).is_err());
        assert!(check_cure_outcome(Some(26), None, &[mark(32)], 57).is_err());
        assert!(check_cure_outcome(Some(26), None, &[], 57).is_err());
        assert!(check_cure_outcome(None, None, &[], 57).is_err());
    }

    #[test]
    fn a_cure_outside_the_runner_bound_fails() {
        assert!(check_cure_outcome(Some(40), Some(51), &[], 71).is_err());
    }

    #[test]
    fn poison_damage_past_the_content_bound_after_the_drink_fails() {
        assert!(check_poison_marks(&[mark(35)], Some(32)).is_err());
        assert!(check_poison_marks(&[mark(33), mark(34)], Some(32)).is_err());
        assert!(check_poison_marks(&[mark(34), mark(64)], Some(32)).is_err());
        assert!(check_cure_outcome(Some(31), Some(32), &[mark(62)], 63).is_err());
    }

    #[test]
    fn poison_damage_before_the_drink_line_fails() {
        assert!(check_poison_marks(&[mark(31)], Some(33)).is_err());
        assert!(check_poison_marks(&[mark(32)], Some(32)).is_err());
    }

    #[test]
    fn a_receipt_before_the_first_scheduled_fire_is_not_a_pass() {
        assert!(check_cure_outcome(Some(40), Some(42), &[], 70).is_err());
    }

    fn bank_trip_oracle_receipt(antipoison_frames: Value) -> Value {
        json!({
            "trip_start_tick": 40,
            "inventory_at_trip_start": GATHERER_INVENTORY_CAPACITY,
            "bank_opened": true,
            "bank_open_tick": 48,
            "willow_logs_deposited": true,
            "willow_logs_count_deposited": 26,
            "willow_logs_at_deposit": 0,
            "willow_logs_deposit_tick": 49,
            "antipoison_count_frames": antipoison_frames,
            "antipoison_count_min_before_cure": 1,
            "first_work_animation_after_bank_trip_tick": 60,
            "first_work_animation_after_bank_trip": {"sequence": 867, "serial": 12},
            "bank_closed_at_resumed_work": true,
            "trip_end_tick": 60,
            "cure_line_tick": 70,
            "observed_tick": 100,
            "deadline_ticks": GATHERER_BANK_TRIP_CURE_DEADLINE_TICKS,
            "trip_and_cure_elapsed_ticks": 60,
        })
    }

    #[test]
    fn gatherer_bank_trip_receipt_accepts_dose_retained_through_trip() {
        let receipt = bank_trip_oracle_receipt(json!([
            {"tick": 40, "count": 1},
            {"tick": 41, "count": 1},
            {"tick": 48, "count": 1},
            {"tick": 49, "count": 1},
            {"tick": 60, "count": 1},
        ]));
        assert_eq!(check_gatherer_bank_trip_receipt(&receipt), Ok(()));
    }

    #[test]
    fn gatherer_bank_trip_receipt_names_tick_when_deposit_loses_dose() {
        let mut receipt = bank_trip_oracle_receipt(json!([
            {"tick": 40, "count": 1},
            {"tick": 41, "count": 1},
            {"tick": 48, "count": 1},
            {"tick": 49, "count": 0},
            {"tick": 60, "count": 0},
        ]));
        receipt["antipoison_count_min_before_cure"] = json!(0);
        let error = check_gatherer_bank_trip_receipt(&receipt)
            .expect_err("depositing the antipoison dose must fail");
        assert!(error.contains("tick 49"), "{error}");
    }

    #[test]
    fn gatherer_bank_trip_receipt_fails_work_resumed_with_the_bank_open() {
        let receipt = |closed: Value| {
            let mut receipt = bank_trip_oracle_receipt(json!([
                {"tick": 40, "count": 1},
                {"tick": 60, "count": 1},
            ]));
            receipt["bank_closed_at_resumed_work"] = closed;
            receipt
        };
        assert!(check_gatherer_bank_trip_receipt(&receipt(json!(false))).is_err());
        assert!(check_gatherer_bank_trip_receipt(&receipt(Value::Null)).is_err());
        assert_eq!(
            check_gatherer_bank_trip_receipt(&receipt(json!(true))),
            Ok(())
        );
    }

    fn lethal_report(decision: Decision) -> SurvivalReport {
        SurvivalReport {
            decision,
            why: Some(CureWhy::Lethal),
            need: Some(SurvivalNeed::Cure {
                or_food: 26,
                ref_heal: 3,
                why: NeedWhy::Lethal,
            }),
            tried: 0,
            owner: Owner::Step,
        }
    }

    /// A Gatherer-shaped witness after one cure, filled the way `capture_need`
    /// and `observe_frame` fill it from `report`. The condition is the onset of
    /// the poison line at tick 33: unlatched for a `Needed` edge, latched at
    /// the edge for a straight `Drinking` report. Ticks are harness ticks of
    /// the final session.
    fn cured_witness(report: &SurvivalReport, latched: bool) -> Witness {
        let witness = new_witness(
            Activity::Woodcutting,
            "Woodcutting",
            WOODCUTTING_STAT,
            Some(WILLOW_LOG_ID),
            Some(RUNE_AXE_ALIAS),
            PoisonSource::AccountStateDebugProc,
            GATHERER_STAND,
        );
        let mut witness = Arc::try_unwrap(witness)
            .ok()
            .expect("one owner")
            .into_inner()
            .expect("witness");
        let onset = PoisonState::Onset {
            since: 33,
            q: latched.then_some(Q_UNKNOWN),
            hit_since_line: false,
        };
        let outlook =
            api::condition::poison_math::remaining(onset, Exposure::Unknown, 33, Q_UNKNOWN);
        witness.poison_tick = Some(33);
        witness.need_tick = Some(34);
        witness.need_report_decision = Some(format!("{:?}", report.decision));
        witness.need_report_why = Some(format!("{:?}", report.why));
        witness.need_report = Some(format!("{:?}", report.need));
        witness.need_report_owner = Some(format!("{:?}", report.owner));
        witness.need_condition_poison = Some(format!("{onset:?}"));
        witness.need_unlatched_poison = Some(!latched);
        witness.need_outlook_total = Some(outlook.total);
        witness.need_outlook_per_fire = Some(outlook.per_fire_now);
        witness.need_outlook_pending = Some(outlook.pending);
        witness.need_hp = Some(30);
        witness.need_hp_max = Some(30);
        witness.need_food = Some(0);
        witness.need_antipoison = Some(1);
        witness.need_in_combat = Some(false);
        witness.need_no_hit_at_edge = Some(true);
        witness.drink_decision_tick = Some(34);
        witness.cure_tick = Some(35);
        witness.dose_drop_tick = Some(35);
        witness.immune_tick = Some(35);
        witness.settled_tick = Some(36);
        witness
    }

    /// The whole cure oracle on one witness: the sequence, then the damage
    /// rule and the first scheduled fire at the observed tick.
    fn cure_verdict(witness: &Witness, observed_tick: u32) -> Result<(), String> {
        cure_ticks(witness)?;
        check_cure_outcome(
            witness.poison_tick,
            witness.cure_tick,
            &witness.poison_marks,
            observed_tick,
        )
    }

    #[test]
    fn the_round_gatherer_receipt_passes() {
        // `round-merlot/gatherer-20261030T192811Z`, receipt SHA-256
        // c5d0b2006dfdbc62b59dc6cc1527557959a539ec364648a18de0707bcb652023.
        // The Gatherer went from `survival=None` straight to `Drinking:
        // poisoned: would die; needs antipoison or 26 food`, with no `Needed`
        // status (status.jsonl). The poison line was at tick 33; the drink
        // packet went at script tick 54, the harness tick 34 under the
        // cell's fixed offset of 20; the end state was Hitpoints 30/30, so no
        // hit. The old oracle waited for a `Needed` report that never came and
        // ran out at tick 154. The receipt itself does not record the need
        // frame, so its condition fields are the ones the Drinking rule asks
        // for: the onset of the line, latched at the edge.
        let report = lethal_report(Decision::Drinking);
        assert!(lethal_cure_report(&report));
        let witness = cured_witness(&report, true);
        assert_eq!(check_need(&witness), Ok(()));
        assert_eq!(cure_verdict(&witness, 154), Ok(()));
    }

    #[test]
    fn the_open_latch_needed_edge_still_passes() {
        let report = lethal_report(Decision::Needed);
        assert!(lethal_cure_report(&report));
        let witness = cured_witness(&report, false);
        assert_eq!(cure_verdict(&witness, 154), Ok(()));
    }

    #[test]
    fn a_run_with_no_need_and_no_drink_fails() {
        // Poison line seen, then neither a Needed nor a Drinking report, no
        // drink request, no cure line: the shape of an uncured run.
        let mut witness = cured_witness(&lethal_report(Decision::Drinking), true);
        witness.need_tick = None;
        witness.drink_decision_tick = None;
        witness.cure_tick = None;
        witness.dose_drop_tick = None;
        witness.immune_tick = None;
        witness.settled_tick = None;
        let error = cure_verdict(&witness, 154).expect_err("no need and no drink");
        assert!(error.contains("neither a lethal Needed nor a lethal Drinking"));
        // Reports that are not a lethal cure need record no need either.
        for decision in [Decision::None, Decision::Eating, Decision::Parked] {
            assert!(!lethal_cure_report(&lethal_report(decision)));
        }
    }

    #[test]
    fn a_run_that_drinks_with_no_poison_fails() {
        // A drink request, a drink line and a settle, with no poison line:
        // `observe_frame` records drink and need ticks only after the line, so
        // this witness is what a drink without poison would have to look like.
        let mut witness = cured_witness(&lethal_report(Decision::Drinking), true);
        witness.poison_tick = None;
        let error = cure_verdict(&witness, 154).expect_err("drink with no poison");
        assert!(error.contains("poison line"));
        assert!(check_cure_outcome(None, Some(35), &[], 154).is_err());
    }

    #[test]
    fn a_drink_without_a_lethal_need_report_fails() {
        let mut witness = cured_witness(&lethal_report(Decision::Drinking), true);
        witness.need_tick = None;
        assert!(cure_verdict(&witness, 154).is_err());
    }

    #[test]
    fn a_need_report_must_be_typed_lethal_and_owned_by_the_step() {
        let mut wrong_why = lethal_report(Decision::Drinking);
        wrong_why.why = Some(CureWhy::BelowLine);
        assert!(!lethal_cure_report(&wrong_why));
        let mut no_why = lethal_report(Decision::Drinking);
        no_why.why = None;
        assert!(!lethal_cure_report(&no_why));
        for owner in [Owner::None, Owner::Guard, Owner::Combat] {
            let mut report = lethal_report(Decision::Drinking);
            report.owner = owner;
            assert!(!lethal_cure_report(&report));
        }
        let mut food = lethal_report(Decision::Needed);
        food.need = Some(SurvivalNeed::Food {
            count: 2,
            ref_heal: 3,
        });
        assert!(!lethal_cure_report(&food));
        let mut emergency = lethal_report(Decision::Drinking);
        emergency.need = Some(SurvivalNeed::Cure {
            or_food: 0,
            ref_heal: 3,
            why: NeedWhy::Emergency,
        });
        assert!(!lethal_cure_report(&emergency));
    }

    #[test]
    fn the_need_edge_conditions_hold_for_each_decision() {
        // `Needed` is the open latch: a latched line is not that edge.
        let needed_latched = cured_witness(&lethal_report(Decision::Needed), true);
        assert!(check_need(&needed_latched).is_err());
        // `Drinking` accepts either latch of the line's own onset...
        let drinking_open = cured_witness(&lethal_report(Decision::Drinking), false);
        assert_eq!(check_need(&drinking_open), Ok(()));
        // ...but not a poison that is no longer the line's onset.
        let mut not_onset = cured_witness(&lethal_report(Decision::Drinking), true);
        not_onset.need_condition_poison = Some(format!("{:?}", PoisonState::Clear));
        assert!(check_need(&not_onset).is_err());
        // Either decision needs the controlled HP, dose and no hit at the edge.
        let mut hurt = cured_witness(&lethal_report(Decision::Drinking), true);
        hurt.need_hp = Some(24);
        assert!(check_need(&hurt).is_err());
        let mut hit = cured_witness(&lethal_report(Decision::Drinking), true);
        hit.need_no_hit_at_edge = Some(false);
        assert!(check_need(&hit).is_err());
    }

    /// The fields `assert_common_cure` reads, with the values of the passing
    /// Gatherer receipt of `cells/round-1` (`Drinking`, latched onset).
    fn cure_receipt(decision: &str, pending: bool, unlatched: bool, poison: &str) -> Value {
        json!({
            "condition_at_need": {
                "poison": poison,
                "unlatched_poison": unlatched,
                "outlook_total": LETHAL_OUTLOOK,
                "outlook_per_fire": Q_UNKNOWN,
                "outlook_pending": pending,
                "hitpoints": 30,
                "hitpoints_max": 30,
                "food": 0,
                "antipoison": 1,
                "in_combat": false,
                "no_hitmark_at_edge": true,
            },
            "survival_report_at_need": {"decision": decision, "why": "Some(Lethal)", "owner": "Step"},
            "drink_decision_chat_modal": -1,
            "poison_marks": [],
            "first_scheduled_fire_covered": true,
            "observed_poison_ticks": 31,
            "ticks_from_poison_to_settled": 1,
            "resumed_work_ticks": 30,
            "antipoison_remaining": 0,
        })
    }

    #[test]
    fn the_restated_assertions_accept_both_need_decisions() {
        assert_common_cure(
            &cure_receipt(
                "Drinking",
                false,
                false,
                "Onset { since: 38, q: Some(11), hit_since_line: false }",
            ),
            120,
        );
        assert_common_cure(
            &cure_receipt(
                "Needed",
                true,
                true,
                "Onset { since: 38, q: None, hit_since_line: false }",
            ),
            120,
        );
    }

    #[test]
    #[should_panic(expected = "neither Needed nor Drinking")]
    fn the_restated_assertions_reject_a_receipt_with_no_lethal_report() {
        assert_common_cure(
            &cure_receipt(
                "None",
                false,
                false,
                "Onset { since: 38, q: Some(11), hit_since_line: false }",
            ),
            120,
        );
    }

    #[test]
    #[should_panic]
    fn the_restated_assertions_reject_a_needed_report_with_a_closed_latch() {
        assert_common_cure(
            &cure_receipt(
                "Needed",
                false,
                false,
                "Onset { since: 38, q: Some(11), hit_since_line: false }",
            ),
            120,
        );
    }

    fn drop_at(tick: u32, from: i32) -> FoodDrop {
        FoodDrop {
            tick,
            from,
            to: from - 1,
            hp_before: 10,
            hp_after: 10 + SHRIMP_HEAL,
        }
    }

    #[test]
    fn bite_spacing_accepts_the_drink_lock_and_the_next_food_clock() {
        assert_eq!(
            check_bite_spacing(40, &[drop_at(43, 5), drop_at(46, 4)]),
            Ok(())
        );
        assert_eq!(
            check_bite_spacing(40, &[drop_at(45, 5), drop_at(50, 4)]),
            Ok(())
        );
    }

    #[test]
    fn bite_spacing_rejects_a_bite_inside_a_lock_or_after_a_dropped_click() {
        // Inside the drink's delay: the engine would have dropped it.
        assert!(check_bite_spacing(40, &[drop_at(42, 5)]).is_err());
        // Inside the next-food clock.
        assert!(check_bite_spacing(40, &[drop_at(43, 5), drop_at(45, 4)]).is_err());
        // A dropped bite resent after the food op's eight-tick bound.
        assert!(check_bite_spacing(40, &[drop_at(43, 5), drop_at(54, 4)]).is_err());
        // A first bite resent after a dropped click.
        assert!(check_bite_spacing(40, &[drop_at(51, 5)]).is_err());
        // A count drop with no heal is not a bite.
        let mut no_heal = drop_at(43, 5);
        no_heal.hp_after = no_heal.hp_before;
        assert!(check_bite_spacing(40, &[no_heal]).is_err());
    }

    #[test]
    fn unpark_window_accepts_only_the_release_mark_window() {
        assert_eq!(check_unpark_window(Some(92), 90, 3, 95), Ok(Some(92)));
        assert_eq!(check_unpark_window(None, 90, 3, 92), Ok(None));
        assert!(check_unpark_window(Some(89), 90, 3, 95).is_err());
        assert!(check_unpark_window(Some(94), 90, 3, 95).is_err());
        assert!(check_unpark_window(None, 90, 3, 94).is_err());
    }

    #[test]
    fn park_food_alternative_matches_the_design_examples() {
        // design-final.md:948.
        assert_eq!(park_food_alternative(330, 60), 23);
        assert_eq!(park_food_alternative(OUTLOOK_AFTER_MARK_6, 54), 4);
    }

    fn parked_frame(or_food: u8, text: &str) -> TrackFrame {
        TrackFrame {
            tick: 40,
            hp: 60,
            food: 0,
            antipoison: 0,
            xp: 0,
            decision: Some(Decision::Parked),
            why: Some(CureWhy::Lethal),
            need: Some(SurvivalNeed::Cure {
                or_food,
                ref_heal: 12,
                why: NeedWhy::Lethal,
            }),
            owner: Some(Owner::Step),
            survival: Some(text.to_owned()),
            phase_label: Some("parked".to_owned()),
            native_phase: Some(NativePhase::Waiting),
            run: Some("run".to_owned()),
            failure: None,
        }
    }

    #[test]
    fn park_frame_needs_the_hold_text_and_need_of_its_hp() {
        let good = parked_frame(
            23,
            "Parked: poisoned: would die; needs antipoison or 23 food",
        );
        assert_eq!(check_park_frame(&good, 330), Ok(23));
        let wrong_count = parked_frame(
            22,
            "Parked: poisoned: would die; needs antipoison or 22 food",
        );
        assert!(check_park_frame(&wrong_count, 330).is_err());
        let mut blocked = good.clone();
        blocked.native_phase = Some(NativePhase::Blocked);
        assert!(check_park_frame(&blocked, 330).is_err());
        let mut needed = good;
        needed.decision = Some(Decision::Needed);
        assert!(check_park_frame(&needed, 330).is_err());
    }

    #[test]
    fn survival_log_counts_park_lines_and_rejects_a_stop() {
        let park = json!({"message": "survival park need=x hp=60 R=330 q=11 widened=true"});
        let unpark = json!({"message": "survival unpark hp=73 R=70"});
        let stop = json!({"message": "script blocked: operator stop"});
        assert_eq!(
            check_survival_log(&[park.clone(), unpark.clone()], 1, 1),
            Ok(())
        );
        assert!(check_survival_log(std::slice::from_ref(&park), 1, 1).is_err());
        assert!(check_survival_log(&[park.clone(), park.clone(), unpark.clone()], 1, 1).is_err());
        assert!(check_survival_log(&[park, unpark, stop], 1, 1).is_err());
    }
}

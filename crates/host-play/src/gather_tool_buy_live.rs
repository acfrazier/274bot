//! G1 Gatherer tool-buy and G2 park lifecycle cells against Engine L and the pinned R289 world.
//!
//! G1 prepares only the account's bank, stats and starting position. A native
//! bank open establishes `Session` memory; the account then teleports to a
//! real resource site and the ordinary Gatherer card performs its shop, wear
//! and gather actions. No NPC, shop stock, tree, fishing spot, XP or product
//! is spawned. Fishing catches and woodcutting products are recorded when
//! naturally observed but are not pass criteria: the content's random success
//! roll is never an oracle.
//!
//! The Harpoon cells use Catherby's real Harpoon spot and Harry's stocked
//! shop. The axe cell uses the existing Draynor bank-trip origin, Bob's
//! stocked axe shop and nearby real normal trees. `buyTools` is omitted to
//! exercise its default-on value.
//! The two G2 cells first park a no-coins Gatherer, then exercise account-only
//! supply arrival plus Resume, or an explicit relog with no wake and no trip.
//!
//! `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=gsh GATHERER_GAME_PORT=47594 GATHERER_HTTP_PORT=3080 GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/GATHERER-SHOP isohome cargo test -p host-play --features live-harness,test-support --lib gather_tool_buy_live:: -- --ignored --nocapture --test-threads=1`

//! Bob's 5 Iron axes at baseline stock (lumbridge.inv stock3) cost 56 gp each
//! (axes.obj [iron_axe]); Bob sells at 1000 with delta 20 (lumbridge.npc).
//! With Attack 1, steel is not wieldable (tier5.rs2) while Iron is (tier1.rs2);
//! Woodcutting has no axe-use level gate. At stock 5, Iron axe quote is 56 gp;
//! after one unit, stock 4 quotes 57 gp, so the two-chunk reserve is 113 gp.
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::task::Poll;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::Origin;
use api::game_data::SelectedGameData;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::service_pick::ShopKeeperFacts;
use api::snapshot::{GameSnapshot, WorldTile};
use parking_lot::Mutex;
use script::bank::{Close, Open, OpenArgs, Select, SelectArgs};
use script::native::{
    ActionHandle, NativePhase, NativeTick, Script, ScriptFailure, ScriptFlow, ScriptStatus,
    StatusValue, WalkOptions,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{
    check_prerequisites, frame, live_profile, Cell, Prep, DRAYNOR, PREPARATION_DEADLINE,
};
use super::combat_proof::{
    self, CaptureRegistration, CombatCapture, DriverPacketCounts, MAX_DRIVER_PACKETS_PER_TICK,
};
use super::{run_with_template, tele_args, SharedClientTemplate};

const PROFILE: &str = "local-289";
const ENGINE_L_GAME_PORT: u16 = 47_594;
const ENGINE_L_HTTP_PORT: u16 = 3_080;
const CELL_BOUND: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const TICK_SAMPLES: usize = 11;
const COIN_ID: i32 = 995;
const HARPOON_ID: i32 = 311;
const IRON_AXE_ID: i32 = 1349;
const HARPOON_SEED: &[&str] = &[
    "setstat fishing 35",
    "setstat defence 99",
    "setstat hitpoints 99",
    "givebank coins 50",
];
const NO_COINS_SEED: &[&str] = &[
    "setstat fishing 35",
    "setstat defence 99",
    "setstat hitpoints 99",
];
const AXE_SEED: &[&str] = &[
    "setstat woodcutting 1",
    "setstat attack 1",
    "setstat defence 99",
    "setstat hitpoints 99",
    "givebank coins 500",
];
const HARRY_CATHERBY: WorldTile = WorldTile {
    x: 2_834,
    z: 3_445,
    level: 0,
};
/// Existing live Gatherer bank-trip origin; no tree is seeded at this site.
const TREE_START: WorldTile = WorldTile {
    x: 3_017,
    z: 3_170,
    level: 0,
};
const CATHERBY_HARPOON_START: WorldTile = WorldTile {
    x: 2_840,
    z: 3_436,
    level: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Case {
    Harpoon,
    NoCoins,
    AxeWear,
}

impl Case {
    const fn name(self) -> &'static str {
        match self {
            Self::Harpoon => "gatherer_tool_buy_harpoon",
            Self::NoCoins => "gatherer_tool_buy_no_coins",
            Self::AxeWear => "gatherer_tool_buy_axe_wear",
        }
    }

    const fn is_axe(self) -> bool {
        matches!(self, Self::AxeWear)
    }

    const fn start(self) -> WorldTile {
        match self {
            Self::Harpoon | Self::NoCoins => CATHERBY_HARPOON_START,
            Self::AxeWear => TREE_START,
        }
    }

    const fn coin_seed(self) -> i32 {
        match self {
            Self::Harpoon => 50,
            Self::NoCoins => 0,
            Self::AxeWear => 500,
        }
    }

    const fn seed(self) -> &'static [&'static str] {
        match self {
            Self::Harpoon => HARPOON_SEED,
            Self::NoCoins => NO_COINS_SEED,
            Self::AxeWear => AXE_SEED,
        }
    }

    const fn content_window(self) -> u64 {
        match self {
            // rarefish.rs2:123-138 sets action_delay to map_clock + 5.
            Self::Harpoon | Self::NoCoins => 5,
            // woodcut.rs2:55-61 sets the first tree chop delay to +3.
            Self::AxeWear => 3,
        }
    }

    const fn method(self) -> &'static str {
        if self.is_axe() {
            "woodcutting.normal"
        } else {
            "fishing.rarefish.op3"
        }
    }

    const fn item_alias(self) -> &'static str {
        if self.is_axe() {
            "iron_axe"
        } else {
            "harpoon"
        }
    }

    const fn keeper(self) -> &'static str {
        if self.is_axe() {
            "bob"
        } else {
            "harry"
        }
    }

    const fn gather_action(self) -> &'static str {
        if self.is_axe() {
            "Chop down"
        } else {
            "Harpoon"
        }
    }

    const fn gather_object(self) -> &'static str {
        if self.is_axe() {
            "Tree"
        } else {
            "Fishing spot"
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct ItemFact {
    alias: String,
    id: i32,
    name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct KeeperFact {
    config: String,
    name: String,
    id: i32,
    placement_tile: WorldTile,
    access_tiles: Vec<WorldTile>,
}

#[derive(Debug, Clone)]
struct Fixture {
    coins: ItemFact,
    tool: ItemFact,
    keeper: KeeperFact,
    baseline: i32,
    cost: i32,
    sell: i32,
    delta: i32,
    quote: i64,
    reserve: i64,
}

fn item_fact(data: &SelectedGameData, alias: &str) -> Result<ItemFact, String> {
    let item = data
        .item_by_alias(alias)
        .ok_or_else(|| format!("R289 selected data has no item alias {alias:?}"))?;
    let name = item
        .name
        .as_deref()
        .ok_or_else(|| format!("R289 selected item alias {alias:?} has no name"))?;
    Ok(ItemFact {
        alias: alias.to_owned(),
        id: item.id,
        name: name.to_owned(),
    })
}

fn keeper_fact(
    keepers: &ShopKeeperFacts,
    config: &str,
    required_tile: Option<WorldTile>,
) -> Result<KeeperFact, String> {
    let row = keepers
        .rows()
        .iter()
        .find(|row| {
            row.keeper.keeper_config == config
                && required_tile.is_none_or(|tile| row.tile == tile)
                && !row.access().is_empty()
        })
        .ok_or_else(|| format!("R289 selected shop facts have no accessible {config} placement"))?;
    Ok(KeeperFact {
        config: row.keeper.keeper_config.to_owned(),
        name: row.keeper.keeper_name.to_owned(),
        id: row.keeper.keeper_id,
        placement_tile: row.tile,
        access_tiles: row.access().to_vec(),
    })
}

fn fixture(
    case: Case,
    data: &SelectedGameData,
    keepers: &ShopKeeperFacts,
) -> Result<Fixture, String> {
    let coins = item_fact(data, "coins")?;
    let tool = item_fact(data, case.item_alias())?;
    let keeper = keeper_fact(
        keepers,
        case.keeper(),
        (case == Case::Harpoon || case == Case::NoCoins).then_some(HARRY_CATHERBY),
    )?;
    let vendor = api::shop_facts::vendor_by_keeper_config(data, case.keeper(), tool.id)
        .map_err(|error| format!("shop facts: {error:?}"))?
        .ok_or_else(|| format!("pinned {} shop has no {} row", case.keeper(), tool.alias))?;
    if vendor.keeper_id != keeper.id || vendor.keeper_name != keeper.name {
        return Err(format!(
            "selected {} placement and shop keeper identity disagree",
            case.keeper()
        ));
    }
    let quote = api::shop_facts::quote(vendor.record, vendor.item, 1, vendor.item.baseline);
    let reserve = quote.saturating_add(api::shop_facts::quote(
        vendor.record,
        vendor.item,
        1,
        vendor.item.baseline.saturating_sub(1),
    ));
    Ok(Fixture {
        coins,
        tool,
        keeper,
        baseline: vendor.item.baseline,
        cost: vendor.item.cost,
        sell: vendor.record.sell,
        delta: vendor.record.delta,
        quote,
        reserve,
    })
}

fn settings(case: Case) -> serde_json::Map<String, Value> {
    let Value::Object(mut bag) = json!({
        "skill": if case.is_axe() { "Woodcutting" } else { "Fishing" },
        "woodcuttingResources": if case.is_axe() { vec!["normal"] } else { Vec::<&str>::new() },
        "fishingMethod": case.method(),
        "targetPreference": "Nearest",
        "location": "Start",
        "radius": 12,
        "disposition": "Bank",
        "bank": "Nearest",
        "coinTarget": 0,
        "allowTeleports": false,
        "allowWilderness": false,
        "deathPolicy": "Stop",
    }) else {
        unreachable!("a JSON object literal")
    };
    if case.is_axe() {
        bag.remove("fishingMethod");
    }
    // Omit this field: the default-on value is the subject of these G1 cells.
    bag.remove("buyTools");
    bag
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct PlayerInfoClock {
    #[serde(skip)]
    last: Option<(u64, u64, Instant)>,
    interarrival_ms: Vec<u128>,
    coalesced_observations: usize,
    median_ms: Option<u128>,
}

impl PlayerInfoClock {
    fn observe(&mut self, session: u64, count: u64, now: Instant) {
        if self
            .last
            .is_some_and(|(previous_session, previous_count, _)| {
                session == previous_session && count == previous_count
            })
        {
            return;
        }
        let Some((previous_session, previous_count, previous_at)) =
            self.last.replace((session, count, now))
        else {
            return;
        };
        if session != previous_session {
            self.interarrival_ms.clear();
            self.coalesced_observations = 0;
            self.median_ms = None;
            return;
        }
        if count.checked_sub(previous_count) != Some(1) {
            self.coalesced_observations += 1;
            return;
        }
        if self.interarrival_ms.len() < TICK_SAMPLES {
            self.interarrival_ms
                .push(now.duration_since(previous_at).as_millis());
            if self.interarrival_ms.len() == TICK_SAMPLES {
                let mut sorted: [u128; TICK_SAMPLES] = self
                    .interarrival_ms
                    .as_slice()
                    .try_into()
                    .expect("sample length is fixed");
                sorted.sort_unstable();
                self.median_ms = Some(sorted[TICK_SAMPLES / 2]);
            }
        }
    }

    // Counter gaps measure observer loss and are already excluded from the samples.
    fn accepted(&self) -> bool {
        self.median_ms
            .is_some_and(|median| (570..=630).contains(&median))
    }
}

#[test]
fn player_info_clock_accepts_clean_engine_l_samples_with_coalesced_observations() {
    for coalesced_before in [0_usize, 2] {
        let mut clock = PlayerInfoClock::default();
        let mut now = Instant::now();
        let mut count = 1_u64;
        clock.observe(1, count, now);
        for _ in 0..coalesced_before {
            count += 2;
            now += Duration::from_millis(1_200);
            clock.observe(1, count, now);
        }
        for _ in 0..TICK_SAMPLES {
            count += 1;
            now += Duration::from_millis(600);
            clock.observe(1, count, now);
        }
        assert_eq!(clock.interarrival_ms.as_slice(), &[600; TICK_SAMPLES]);
        assert_eq!(clock.median_ms, Some(600));
        assert!(clock.accepted());

        count += 2;
        now += Duration::from_millis(1_200);
        clock.observe(1, count, now);
        assert_eq!(clock.coalesced_observations, coalesced_before + 1);
        assert_eq!(clock.interarrival_ms.len(), TICK_SAMPLES);
        assert_eq!(clock.median_ms, Some(600));
        assert!(clock.accepted());
    }
}

#[test]
fn player_info_clock_rejects_clean_engine_q_samples() {
    let mut clock = PlayerInfoClock::default();
    let mut now = Instant::now();
    let mut count = 1_u64;
    clock.observe(1, count, now);
    for _ in 0..TICK_SAMPLES {
        count += 1;
        now += Duration::from_millis(300);
        clock.observe(1, count, now);
    }
    assert_eq!(clock.median_ms, Some(300));
    assert!(!clock.accepted());
}

#[test]
fn player_info_clock_rejects_too_few_clean_samples() {
    for clean_samples in 0..TICK_SAMPLES {
        let mut clock = PlayerInfoClock::default();
        let mut now = Instant::now();
        let mut count = 1_u64;
        clock.observe(1, count, now);
        count += 2;
        now += Duration::from_millis(1_200);
        clock.observe(1, count, now);
        for _ in 0..clean_samples {
            count += 1;
            now += Duration::from_millis(600);
            clock.observe(1, count, now);
        }
        assert_eq!(clock.interarrival_ms.len(), clean_samples);
        assert_eq!(clock.coalesced_observations, 1);
        assert_eq!(clock.median_ms, None);
        assert!(!clock.accepted());
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct PrimeTrace {
    before_origin: Option<String>,
    before_rows: Option<Vec<(i32, i32)>>,
    opened_origin: Option<String>,
    opened_rows: Option<Vec<(i32, i32)>>,
    open_generation: Option<u64>,
    closed: bool,
    complete: bool,
    failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum ParkTransition {
    None,
    Resume,
    Relog,
}

impl ParkTransition {
    const fn scenario_name(self, case: Case) -> &'static str {
        match (case, self) {
            (Case::NoCoins, Self::Resume) => "gatherer_park_then_resume",
            (Case::NoCoins, Self::Relog) => "gatherer_park_relog",
            (_, Self::None) => case.name(),
            _ => "invalid_gatherer_park_transition",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
struct ParkObservation {
    run_slot: Option<u64>,
    run_generation: Option<u64>,
    run_session: Option<u64>,
    phase: Option<String>,
    phase_label: Option<String>,
    slot_state: Option<String>,
    slot_running: Option<bool>,
    want_run: Option<bool>,
    host_up: Option<bool>,
    failure: Option<String>,
    block_reason: Option<String>,
    bank_seen: Option<String>,
    bank_origin: Option<String>,
    bank_rows: Option<Vec<(i32, i32)>>,
    bank_open: Option<bool>,
    session_generation: Option<u64>,
    tool_in_pack: Option<i64>,
    coins_in_pack: Option<i64>,
    no_walk: Option<bool>,
    no_bank_trip: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct ParkLifecycleTrace {
    initial: Option<ParkObservation>,
    pause_requested: bool,
    pause_observed: bool,
    supply_command: Option<String>,
    supply_observed: bool,
    resume_requested: bool,
    resume_observed: bool,
    logout_requested: bool,
    logout_observed: bool,
    relog_requested: bool,
    relog_observed: bool,
    after_relog: Option<ParkObservation>,
    relog_observations: Vec<ParkObservation>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct LiveTrace {
    clock: PlayerInfoClock,
    frames: Vec<Value>,
    prime: PrimeTrace,
    park_lifecycle: ParkLifecycleTrace,
    start_requested: bool,
    finish_requested: bool,
    start_session: Option<u64>,
    start_player_info: Option<u64>,
    start_game_tick: Option<u32>,
    start_tile: Option<(i32, i32, i32)>,
    start_bank_origin: Option<String>,
    start_bank_rows: Option<Vec<(i32, i32)>>,
    start_inventory: Option<Value>,
    start_equipment: Option<Value>,
    start_server_closes: Option<u64>,
    start_action_index: Option<usize>,
    start_baseline_recorded: bool,
    final_phase: Option<String>,
    final_phase_label: Option<String>,
    final_slot_state: Option<String>,
    final_slot_running: Option<bool>,
    final_failure: Option<String>,
    final_block_reason: Option<String>,
    final_bank_open: Option<bool>,
    final_shop_open: Option<bool>,
    final_bank_origin: Option<String>,
    final_bank_rows: Option<Vec<(i32, i32)>>,
    park_no_walk: Option<bool>,
    gather_action: Option<Value>,
    content_window_end_tick: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimeStep {
    Record,
    Select,
    Open,
    Close,
    Done,
}

struct BankPrimer {
    facts: Arc<NamedBankFacts>,
    bank_name: Arc<str>,
    expected_rows: Vec<(i32, i32)>,
    trace: Arc<Mutex<LiveTrace>>,
    cell: Arc<Mutex<Cell>>,
    step: PrimeStep,
    select: Option<ActionHandle<Select>>,
    open: Option<ActionHandle<Open>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
}

impl BankPrimer {
    fn blocked(&mut self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.trace.lock().prime.failure = Some(message.clone());
        self.cell.lock().fail(message.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "gatherer-tool-buy-bank-prime".into(),
            message: message.into(),
        })
    }
}

fn origin_name(origin: Origin) -> String {
    format!("{origin:?}")
}

impl Script for BankPrimer {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        match self.step {
            PrimeStep::Done => Ok(ScriptFlow::Complete),
            PrimeStep::Record => {
                let Some(memory) = tick.cx.snapshot().bank_memory() else {
                    return Ok(self.blocked("the initial frame carries no bank memory"));
                };
                let origin = memory.origin();
                let rows = memory.rows().to_vec();
                self.trace.lock().prime.before_origin = Some(origin_name(origin));
                self.trace.lock().prime.before_rows = Some(rows.clone());
                if origin != Origin::Unknown || !rows.is_empty() {
                    return Ok(self.blocked(format!(
                        "fresh account did not begin Unknown/empty: {origin:?} {rows:?}"
                    )));
                }
                self.step = PrimeStep::Select;
                Ok(ScriptFlow::Continue)
            }
            PrimeStep::Select => {
                if self.select.is_none() {
                    let args = SelectArgs {
                        facts: Arc::clone(&self.facts),
                        from: DRAYNOR,
                        preferences: BankPreferences {
                            use_mage_bank: false,
                            use_zanaris_bank: false,
                        },
                        options: WalkOptions::default(),
                        explicit: Some(Arc::clone(&self.bank_name)),
                        allow_air_fallback: false,
                    };
                    match tick.actions.begin::<Select>(args, &mut tick.cx) {
                        Ok(handle) => self.select = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Select begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.select.as_ref().expect("Select handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Select: {error:?}"))),
                    Poll::Ready(Ok(selected)) => {
                        self.select = None;
                        let Some(access) = selected.access else {
                            return Ok(self.blocked("Select returned no bank access"));
                        };
                        self.access = Some(access);
                        self.step = PrimeStep::Open;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            PrimeStep::Open => {
                if self.open.is_none() {
                    let Some(access) = self.access.clone() else {
                        return Ok(self.blocked("Open has no selected stand"));
                    };
                    match tick
                        .actions
                        .begin::<Open>(OpenArgs { access }, &mut tick.cx)
                    {
                        Ok(handle) => self.open = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Open begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.open.as_ref().expect("Open handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Open: {error:?}"))),
                    Poll::Ready(Ok(())) => {
                        self.open = None;
                        let snapshot = tick.cx.snapshot();
                        let Some(memory) = snapshot.bank_memory() else {
                            return Ok(self.blocked("open frame carries no bank memory"));
                        };
                        let origin = memory.origin();
                        let rows = memory.rows().to_vec();
                        self.trace.lock().prime.opened_origin = Some(origin_name(origin));
                        self.trace.lock().prime.opened_rows = Some(rows.clone());
                        self.trace.lock().prime.open_generation = Some(memory.generation());
                        if origin != Origin::Session || rows != self.expected_rows {
                            return Ok(self.blocked(format!(
                                "open did not observe the exact Session seed: {origin:?} {rows:?}, expected {:?}",
                                self.expected_rows
                            )));
                        }
                        self.step = PrimeStep::Close;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            PrimeStep::Close => {
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
                        self.close = None;
                        let closed = tick
                            .cx
                            .snapshot()
                            .bank_session()
                            .is_some_and(|session| !session.value.open);
                        self.trace.lock().prime.closed = closed;
                        self.trace.lock().prime.complete = closed;
                        if !closed {
                            return Ok(self.blocked("the bank remained open after Close"));
                        }
                        self.step = PrimeStep::Done;
                        Ok(ScriptFlow::Complete)
                    }
                }
            }
        }
    }
}

fn frame_value(snapshot: &GameSnapshot, session: u64, player_info: u64) -> Value {
    let mut value = combat_proof::snapshot_facts(snapshot, None);
    let shop = snapshot.shop();
    value["session_generation"] = json!(session);
    value["player_info_generation"] = json!(player_info);
    value["bank_open"] = json!(snapshot.bank_component_id() >= 0);
    value["shop_open"] = json!(shop.open);
    value["shop_server_closes"] = json!(shop.server_closes);
    value["nearby_locs"] = json!(snapshot
        .locs()
        .iter()
        .filter(|loc| loc.distance <= 12)
        .map(|loc| json!({
            "id": loc.id,
            "name": loc.name,
            "tile": loc.tile,
            "distance": loc.distance,
            "actions": loc.actions,
        }))
        .collect::<Vec<_>>());
    value
}

fn count_rows(rows: &Value, item_id: i64) -> i64 {
    rows.as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["id"].as_i64() == Some(item_id))
        .map(|row| row["count"].as_i64().unwrap_or(0))
        .sum()
}

fn held_total(snapshot: &Value, item_id: i64) -> i64 {
    count_rows(&snapshot["inventory"], item_id) + count_rows(&snapshot["equipment"], item_id)
}

fn debug_field<'a>(debug: &'a str, field: &str) -> Option<&'a str> {
    let marker = format!("{field}: ");
    let start = debug.find(&marker)?.checked_add(marker.len())?;
    let rest = &debug[start..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn debug_number(debug: &str, field: &str) -> Option<i64> {
    debug_field(debug, field)?.parse().ok()
}

#[derive(Debug, Clone)]
struct BuyButton {
    sequence: u64,
    tick: u64,
    item_id: i64,
    chunk: i64,
    accepted: bool,
    decoded: bool,
    wire_opcodes: Vec<i64>,
    action_index: usize,
    before: i64,
}

fn buy_button(action: &Value, action_index: usize) -> Option<BuyButton> {
    let debug = action["request"]["debug"].as_str()?;
    if !debug.starts_with("ShopButton {") || !debug.contains("kind: \"buy\"") {
        return None;
    }
    Some(BuyButton {
        sequence: action["sequence"].as_u64()?,
        tick: action["snapshot_tick"]
            .as_u64()
            .or_else(|| action["tick"].as_u64())?,
        item_id: debug_number(debug, "id")?,
        chunk: debug_number(debug, "chunk")?,
        accepted: action["accepted"].as_bool().unwrap_or(false),
        decoded: action["wire_decoded"].as_bool().unwrap_or(false),
        wire_opcodes: action["wire_opcodes"]
            .as_array()
            .map(|rows| rows.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
        action_index,
        before: count_rows(&action["snapshot"]["inventory"], debug_number(debug, "id")?),
    })
}

fn button_analysis(
    actions: &[Value],
    frames: &[Value],
    tool_id: i32,
    expected_buttons: usize,
) -> (Vec<Value>, Vec<String>, usize, Option<u64>) {
    let buttons = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| buy_button(action, index))
        .collect::<Vec<_>>();
    let all_shop_buttons = actions
        .iter()
        .filter(|action| {
            action["request"]["debug"]
                .as_str()
                .is_some_and(|debug| debug.starts_with("ShopButton {"))
        })
        .count();
    let mut rows = Vec::with_capacity(buttons.len());
    let mut failures = Vec::new();
    let mut reconciled = 0usize;
    let mut maximum_reply_edges = None::<u64>;
    if all_shop_buttons != buttons.len() {
        failures.push(format!(
            "every ShopButton must be the one buy dispatch: observed {all_shop_buttons} buttons and {} buys",
            buttons.len()
        ));
    }
    if buttons.len() != expected_buttons {
        failures.push(format!(
            "expected {expected_buttons} buy ShopButtons, saw {}",
            buttons.len()
        ));
    }
    for (position, button) in buttons.iter().enumerate() {
        let next = buttons.get(position + 1);
        let next_tick = next.map_or(u64::MAX, |next| next.tick);
        let mut observations = Vec::<(u64, u64, i64, &'static str)>::new();
        for frame in frames {
            let tick = frame["snapshot_tick"].as_u64().unwrap_or(0);
            if tick > button.tick && tick < next_tick {
                observations.push((
                    tick,
                    0,
                    count_rows(&frame["inventory"], i64::from(tool_id)),
                    "frame",
                ));
            }
        }
        for action in actions.iter().skip(button.action_index + 1) {
            let sequence = action["sequence"].as_u64().unwrap_or(0);
            if next.is_some_and(|next| sequence >= next.sequence) {
                break;
            }
            let tick = action["snapshot_tick"]
                .as_u64()
                .or_else(|| action["tick"].as_u64())
                .unwrap_or(0);
            if tick >= button.tick {
                observations.push((
                    tick,
                    sequence,
                    count_rows(&action["snapshot"]["inventory"], i64::from(tool_id)),
                    "action-snapshot",
                ));
            }
        }
        observations.sort_by_key(|(tick, sequence, _, _)| (*tick, *sequence));
        let response = observations
            .iter()
            .find(|(_, _, held, _)| *held > button.before);
        let target_held = button.before.saturating_add(button.chunk);
        let resolved = observations
            .iter()
            .find(|(_, _, held, _)| *held >= target_held);
        let transferred = resolved.map(|(_, _, held, _)| held.saturating_sub(button.before));
        let response_tick = response.map(|(tick, _, _, _)| *tick);
        let reply_edges = response_tick.map(|tick| tick.saturating_sub(button.tick));
        if !button.accepted || !button.decoded || button.wire_opcodes.is_empty() {
            failures.push(format!(
                "ShopButton sequence {} lacks accepted, decoded Driver packet evidence",
                button.sequence
            ));
        }
        if button.item_id != i64::from(tool_id) || button.chunk != 1 {
            failures.push(format!(
                "buy dispatch targeted item {} chunk {}, expected tool {} chunk 1",
                button.item_id, button.chunk, tool_id
            ));
        }
        if transferred == Some(button.chunk) {
            reconciled += 1;
        } else {
            failures.push(format!(
                "ShopButton sequence {} did not resolve exactly {} units: {transferred:?}",
                button.sequence, button.chunk
            ));
        }
        if let Some(edges) = reply_edges {
            maximum_reply_edges = Some(maximum_reply_edges.map_or(edges, |old| old.max(edges)));
            if edges > script::acquire::buy::BUY_REPLY_TICKS {
                failures.push(format!(
                    "ShopButton sequence {} reply used {edges} observed ticks, above BUY_REPLY_TICKS={}",
                    button.sequence,
                    script::acquire::buy::BUY_REPLY_TICKS
                ));
            }
        } else {
            failures.push(format!(
                "ShopButton sequence {} has no observed reply",
                button.sequence
            ));
        }
        if next.is_some_and(|next| {
            count_rows(
                &actions[next.action_index]["snapshot"]["inventory"],
                i64::from(tool_id),
            ) < target_held
        }) {
            failures.push(format!(
                "another buy dispatch followed sequence {} before its held target resolved",
                button.sequence
            ));
        }
        rows.push(json!({
            "sequence": button.sequence,
            "dispatch_game_tick": button.tick,
            "item_id": button.item_id,
            "chunk": button.chunk,
            "accepted": button.accepted,
            "wire_decoded": button.decoded,
            "wire_opcodes": button.wire_opcodes,
            "held_before": button.before,
            "observed_reply_game_tick": response_tick,
            "observed_reply_edges": reply_edges,
            "reply_tick_cap": script::acquire::buy::BUY_REPLY_TICKS,
            "resolved_game_tick": resolved.map(|(tick, _, _, _)| *tick),
            "resolved_by": resolved.map(|(_, _, _, source)| *source),
            "observed_transfer": transferred,
        }));
    }
    (rows, failures, reconciled, maximum_reply_edges)
}

fn is_close_modal(action: &Value) -> bool {
    action["request"]["debug"].as_str() == Some("CloseModal")
}

fn close_analysis(
    actions: &[Value],
    frames: &[Value],
    baseline: Option<u64>,
) -> (Value, Vec<String>, Option<u64>) {
    let close_actions = actions
        .iter()
        .filter(|action| is_close_modal(action))
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    let mut previous_count = baseline.unwrap_or(0);
    let mut resolutions = Vec::new();
    for action in &close_actions {
        let action_tick = action["snapshot_tick"].as_u64().unwrap_or(0);
        let resolved = frames.iter().find(|frame| {
            frame["snapshot_tick"]
                .as_u64()
                .is_some_and(|tick| tick >= action_tick)
                && frame["shop_server_closes"]
                    .as_u64()
                    .is_some_and(|count| count > previous_count)
                && frame["shop_open"].as_bool() == Some(false)
        });
        if let Some(frame) = resolved {
            let count = frame["shop_server_closes"]
                .as_u64()
                .unwrap_or(previous_count);
            previous_count = count;
            resolutions.push(json!({
                "close_sequence": action["sequence"],
                "dispatch_game_tick": action_tick,
                "resolved_game_tick": frame["snapshot_tick"],
                "server_close_count": count,
            }));
        } else {
            failures.push(format!(
                "CloseModal sequence {} has no later server-counted close with the shop shut",
                action["sequence"].as_u64().unwrap_or(0)
            ));
        }
        if !action["accepted"].as_bool().unwrap_or(false)
            || !action["wire_decoded"].as_bool().unwrap_or(false)
            || action["wire_opcodes"].as_array().is_none_or(Vec::is_empty)
        {
            failures.push("CloseModal lacks accepted, decoded Driver packet evidence".into());
        }
    }
    if close_actions.len() != 1 || resolutions.len() != 1 {
        failures.push(format!(
            "expected one counted shop close resolution, actions={} resolutions={}",
            close_actions.len(),
            resolutions.len()
        ));
    }
    let final_count = frames
        .last()
        .and_then(|frame| frame["shop_server_closes"].as_u64());
    let final_open = frames.last().and_then(|frame| frame["shop_open"].as_bool());
    if final_count.is_none_or(|count| count <= baseline.unwrap_or(0)) {
        failures.push(format!(
            "server close count did not advance beyond Start baseline {:?}: {final_count:?}",
            baseline
        ));
    }
    if final_open != Some(false) {
        failures.push(format!("shop not closed at completion: {final_open:?}"));
    }
    (
        json!({
            "baseline_server_closes": baseline,
            "final_server_closes": final_count,
            "server_close_delta": final_count.map(|count| count.saturating_sub(baseline.unwrap_or(0))),
            "close_modal_actions": close_actions.len(),
            "resolutions": resolutions,
            "final_shop_open": final_open,
        }),
        failures,
        final_count,
    )
}

fn trade_analysis(
    actions: &[Value],
    fixture: &Fixture,
    expected_trades: usize,
) -> (Vec<Value>, Vec<String>) {
    let mut records = Vec::new();
    let mut failures = Vec::new();
    for action in actions {
        let request = &action["request"];
        if request["op"] != "npc"
            || !request["action"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("trade"))
        {
            continue;
        }
        let requested_index = request["index"].as_i64();
        let npc = action["snapshot"]["nearby_npcs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["index"].as_i64() == requested_index);
        let npc_type = npc.and_then(|row| row["type"].as_i64());
        let eligible = npc_type == Some(i64::from(fixture.keeper.id));
        if !action["accepted"].as_bool().unwrap_or(false) || !eligible {
            failures.push(format!(
                "Trade did not resolve to pinned {} id {}: request index={requested_index:?}, observed type={npc_type:?}",
                fixture.keeper.name, fixture.keeper.id
            ));
        }
        records.push(json!({
            "sequence": action["sequence"],
            "game_tick": action["snapshot_tick"],
            "requested_npc_index": requested_index,
            "observed_npc_type": npc_type,
            "observed_npc_name": npc.map(|row| &row["name"]),
            "observed_npc_tile": npc.map(|row| &row["tile"]),
            "pinned_keeper": fixture.keeper,
            "eligible_pinned_keeper": eligible,
            "accepted": action["accepted"],
            "wire_decoded": action["wire_decoded"],
            "wire_opcodes": action["wire_opcodes"],
        }));
    }
    if records.len() != expected_trades {
        failures.push(format!(
            "expected {expected_trades} Trade actions with {}, observed {}",
            fixture.keeper.name,
            records.len()
        ));
    }
    (records, failures)
}

fn wear_action(actions: &[Value], frames: &[Value], item_id: i32) -> (Option<Value>, Vec<String>) {
    let mut failures = Vec::new();
    let rows = actions
        .iter()
        .filter(|action| {
            action["request"]["op"] == "held"
                && action["request"]["action"].as_str() == Some("Wield")
                && action["request"]["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Iron axe"))
        })
        .collect::<Vec<_>>();
    if rows.len() != 1 {
        failures.push(format!(
            "expected one Iron axe Held/Wield action, observed {}",
            rows.len()
        ));
    }
    let row = rows.first().map(|action| {
        let request_slot = action["request"]["slot"].as_i64();
        let observed_inventory_entry = request_slot.and_then(|slot| {
            action["snapshot"]["inventory"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| {
                    entry["id"].as_i64() == Some(i64::from(item_id))
                        && entry["slot"].as_i64() == Some(slot)
                        && entry["count"].as_i64().unwrap_or(0) > 0
                })
        });
        let observed_slot = observed_inventory_entry.and_then(|entry| entry["slot"].as_i64());
        let inventory = count_rows(&action["snapshot"]["inventory"], i64::from(item_id));
        let dispatch_equipment =
            count_rows(&action["snapshot"]["equipment"], i64::from(item_id));
        let later_frame = action["snapshot_tick"].as_u64().and_then(|action_tick| {
            frames.iter().find(|frame| {
                frame["snapshot_tick"]
                    .as_u64()
                    .is_some_and(|tick| tick > action_tick)
                    && count_rows(&frame["equipment"], i64::from(item_id)) == 1
                    && count_rows(&frame["inventory"], i64::from(item_id)) == 0
            })
        });
        let later_inventory = later_frame
            .map(|frame| count_rows(&frame["inventory"], i64::from(item_id)))
            .unwrap_or(0);
        let equipment = later_frame
            .map(|frame| count_rows(&frame["equipment"], i64::from(item_id)))
            .unwrap_or(0);
        if !action["accepted"].as_bool().unwrap_or(false)
            || !action["wire_decoded"].as_bool().unwrap_or(false)
            || observed_inventory_entry.is_none()
            || inventory != 1
            || dispatch_equipment != 0
            || later_frame.is_none()
        {
            failures.push(format!(
                "Held/Wield must be accepted, match the observed Iron axe inventory slot, and show a later equipment transition: request_slot={request_slot:?}, observed_slot={observed_slot:?}, inventory={inventory}, dispatch_equipment={dispatch_equipment}, later_inventory={later_inventory}, later_equipment={equipment}, action={action}"
            ));
        }
        json!({
            "sequence": action["sequence"],
            "game_tick": action["snapshot_tick"],
            "inventory_axe": inventory,
            "equipment_axe": equipment,
            "inventory_slot": request_slot,
            "observed_inventory_slot": observed_slot,
            "later_frame_tick": later_frame.and_then(|frame| frame["snapshot_tick"].as_u64()),
            "accepted": action["accepted"],
            "wire_decoded": action["wire_decoded"],
            "wire_opcodes": action["wire_opcodes"],
        })
    });
    (row, failures)
}

fn gather_action(
    actions: &[Value],
    frames: &[Value],
    case: Case,
    start: WorldTile,
    radius: i32,
) -> Option<Value> {
    actions
        .iter()
        .find(|action| {
            let request = &action["request"];
            let expected_op = if case.is_axe() { "loc" } else { "npc" };
            if request["op"] != expected_op
                || request["action"].as_str() != Some(case.gather_action())
            {
                return false;
            }
            if case.is_axe() {
                if request["name"].as_str() != Some(case.gather_object()) {
                    return false;
                }
                let (Some(id), Some(x), Some(z), Some(level)) = (
                    request["id"].as_i64(),
                    request["x"].as_i64(),
                    request["z"].as_i64(),
                    request["level"].as_i64(),
                ) else {
                    return false;
                };
                let tick = action["snapshot_tick"].as_u64().unwrap_or(0);
                let observed_tree = frames.iter().any(|frame| {
                    frame["snapshot_tick"]
                        .as_u64()
                        .is_some_and(|frame_tick| frame_tick.abs_diff(tick) <= 2)
                        && frame["nearby_locs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|loc| {
                                loc["id"].as_i64() == Some(id)
                                    && loc["name"].as_str() == Some(case.gather_object())
                                    && loc["tile"]["x"].as_i64() == Some(x)
                                    && loc["tile"]["z"].as_i64() == Some(z)
                                    && loc["tile"]["level"].as_i64() == Some(level)
                                    && loc["actions"].as_array().is_some_and(|actions| {
                                        actions
                                            .iter()
                                            .any(|action| action.as_str() == Some("Chop down"))
                                    })
                            })
                });
                if !observed_tree {
                    return false;
                }
            } else {
                let npc_index = request["index"].as_i64();
                let observed = action["snapshot"]["nearby_npcs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|npc| npc["index"].as_i64() == npc_index);
                if observed.is_none_or(|npc| npc["name"].as_str() != Some("Fishing spot")) {
                    return false;
                }
            }
            let Some([x, z, level]) = action["snapshot"]["tile"].as_array().and_then(|tile| {
                Some([
                    i32::try_from(tile.first()?.as_i64()?).ok()?,
                    i32::try_from(tile.get(1)?.as_i64()?).ok()?,
                    i32::try_from(tile.get(2)?.as_i64()?).ok()?,
                ])
            }) else {
                return false;
            };
            action["accepted"].as_bool() == Some(true)
                && level == start.level
                && (x - start.x).abs().max((z - start.z).abs()) <= radius
        })
        .cloned()
}

fn idle_driver_packet_window(case: Case, transition: ParkTransition) -> bool {
    case == Case::NoCoins && matches!(transition, ParkTransition::None | ParkTransition::Relog)
}

#[test]
fn axe_wear_requires_observed_inventory_slot_and_later_equipment_transition() {
    let action = json!({
        "sequence": 12,
        "snapshot_tick": 229,
        "request": {"op": "held", "name": "Iron axe", "action": "Wield", "slot": 1},
        "accepted": true,
        "wire_decoded": true,
        "wire_opcodes": [177],
        "snapshot": {
            "inventory": [{"id": IRON_AXE_ID, "slot": 1, "count": 1}],
            "equipment": [],
        },
    });
    let frames = vec![
        json!({
            "snapshot_tick": 229,
            "inventory": [{"id": IRON_AXE_ID, "slot": 1, "count": 1}],
            "equipment": [],
        }),
        json!({
            "snapshot_tick": 230,
            "inventory": [],
            "equipment": [{"id": IRON_AXE_ID, "slot": 3, "count": 1}],
        }),
    ];

    let (wear, failures) = wear_action(std::slice::from_ref(&action), &frames, IRON_AXE_ID);
    assert!(failures.is_empty(), "{failures:?}");
    let wear = wear.expect("the accepted Held/Wield action is classified");
    assert_eq!(wear["inventory_axe"], 1);
    assert_eq!(wear["equipment_axe"], 1);
    assert_eq!(wear["inventory_slot"], 1);
    assert_eq!(wear["observed_inventory_slot"], 1);
    assert_eq!(wear["later_frame_tick"], 230);

    let mut wrong_slot = action.clone();
    wrong_slot["snapshot"]["inventory"][0]["slot"] = json!(0);
    let (_, failures) = wear_action(&[wrong_slot], &frames, IRON_AXE_ID);
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("observed Iron axe inventory slot")),
        "{failures:?}"
    );

    let (_, failures) = wear_action(&[action], &frames[..1], IRON_AXE_ID);
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("later equipment transition")),
        "{failures:?}"
    );
}

#[test]
fn axe_gather_uses_normalized_loc_request_and_observed_world_tile() {
    let action = json!({
        "snapshot_tick": 529,
        "request": {
            "op": "loc",
            "x": 3025,
            "z": 3169,
            "level": 0,
            "name": "Tree",
            "action": "Chop down",
            "id": 1276,
        },
        "accepted": true,
        "snapshot": {"tile": [3027, 3174, 0]},
    });
    let frame = json!({
        "snapshot_tick": 529,
        "nearby_locs": [{
            "id": 1276,
            "name": "Tree",
            "tile": {"x": 3025, "z": 3169, "level": 0},
            "actions": ["Chop down"],
        }],
    });

    let gather = gather_action(
        std::slice::from_ref(&action),
        std::slice::from_ref(&frame),
        Case::AxeWear,
        TREE_START,
        12,
    )
    .expect("the normalized Loc action matches the observed tree");
    assert_eq!(gather["request"]["op"], "loc");
    assert_eq!(gather["request"]["id"], 1276);

    let debug_only = json!({
        "snapshot_tick": 529,
        "request": {"debug": "Loc { x: 3025, z: 3169, level: 0, action: \"Chop down\", id: Some(1276) }"},
        "accepted": true,
        "snapshot": {"tile": [3027, 3174, 0]},
    });
    assert!(
        gather_action(&[debug_only], &[frame], Case::AxeWear, TREE_START, 12).is_none(),
        "unparsed debug text is not accepted as structured target evidence"
    );
}

#[test]
fn only_no_coin_idle_and_relog_windows_use_idle_packet_proof() {
    assert!(idle_driver_packet_window(
        Case::NoCoins,
        ParkTransition::None
    ));
    assert!(idle_driver_packet_window(
        Case::NoCoins,
        ParkTransition::Relog
    ));
    assert!(!idle_driver_packet_window(
        Case::NoCoins,
        ParkTransition::Resume
    ));
    assert!(!idle_driver_packet_window(
        Case::AxeWear,
        ParkTransition::None
    ));
    assert!(!idle_driver_packet_window(
        Case::Harpoon,
        ParkTransition::None
    ));
}

#[test]
fn park_observation_collects_play_facts_before_locking_trace() {
    let status = ScriptStatus {
        run: api::selected::RunKey {
            slot: 1,
            run: 2,
            session: 3,
        },
        card: script::CompiledId("test"),
        phase: NativePhase::Blocked,
        survival: Default::default(),
        active_settings: 1,
        pending_settings: None,
        fields: Arc::from([]),
        failure: None,
    };
    let fixture = Fixture {
        coins: ItemFact {
            alias: "coins".into(),
            id: COIN_ID,
            name: "Coins".into(),
        },
        tool: ItemFact {
            alias: "harpoon".into(),
            id: HARPOON_ID,
            name: "Harpoon".into(),
        },
        keeper: KeeperFact {
            config: "harry".into(),
            name: "Harry".into(),
            id: 1,
            placement_tile: HARRY_CATHERBY,
            access_tiles: Vec::new(),
        },
        baseline: 0,
        cost: 0,
        sell: 0,
        delta: 0,
        quote: 0,
        reserve: 0,
    };
    let trace = Mutex::new(LiveTrace {
        frames: vec![json!({
            "bank_open": false,
            "session_generation": 4,
            "inventory": [],
        })],
        ..LiveTrace::default()
    });
    let play_state = Mutex::new(());
    let observation = park_observation(&status, &trace, &[], &fixture, || {
        let _play_guard = play_state.lock();
        assert!(
            trace.try_lock().is_some(),
            "Play facts must be collected before the trace mutex is acquired"
        );
        ParkPlaySnapshot {
            slot_state: script::RunState::Running,
            want_run: true,
            host_up: true,
            bank_origin: "Session".into(),
            bank_rows: Vec::new(),
        }
    });
    assert_eq!(observation.run_slot, Some(1));
    assert_eq!(observation.slot_running, Some(true));
    assert_eq!(observation.bank_origin.as_deref(), Some("Session"));
    assert_eq!(observation.bank_rows.as_deref(), Some(&[][..]));
    assert_eq!(observation.no_bank_trip, Some(true));
}

fn fixture_terminal_failure(cell: &Mutex<Cell>) -> Option<String> {
    let cell = cell.lock();
    (cell.terminal && !cell.trace.passed).then(|| {
        cell.trace
            .failure
            .clone()
            .unwrap_or_else(|| "bank fixture failed without a reason".into())
    })
}

fn content_window_finished(trace: &Mutex<LiveTrace>) -> bool {
    let trace = trace.lock();
    let Some(end_tick) = trace.content_window_end_tick else {
        return false;
    };
    trace
        .frames
        .last()
        .and_then(|frame| frame["snapshot_tick"].as_u64())
        .is_some_and(|tick| (tick as u32).wrapping_sub(end_tick) < (u32::MAX / 2))
}

#[test]
fn live_poll_reads_failed_fixture_and_content_window_without_recursive_locks() {
    let (sent, received) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let cell = Mutex::new(Cell::new(
            CELL_BOUND,
            &[],
            |_| true,
            &[],
            json!({}),
            |_| true,
            PathBuf::new(),
        ));
        assert_eq!(fixture_terminal_failure(&cell), None);
        cell.lock().fail("fixture deadline".into());
        assert_eq!(
            fixture_terminal_failure(&cell).as_deref(),
            Some("fixture deadline")
        );
        cell.lock().trace.passed = true;
        assert_eq!(fixture_terminal_failure(&cell), None);
        assert!(cell.try_lock().is_some());

        let trace = Mutex::new(LiveTrace::default());
        assert!(!content_window_finished(&trace));
        trace.lock().content_window_end_tick = Some(10);
        assert!(!content_window_finished(&trace));
        trace.lock().frames.push(json!({"snapshot_tick": 9}));
        assert!(!content_window_finished(&trace));
        trace.lock().frames.push(json!({"snapshot_tick": 10}));
        assert!(content_window_finished(&trace));
        assert!(trace.try_lock().is_some());
        sent.send(()).expect("report completed poll observations");
    });
    received
        .recv_timeout(Duration::from_secs(5))
        .expect("terminal and content-window observations must not deadlock the live deadline");
    worker.join().expect("poll observation assertions pass");
}

fn field_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn nearest_vendor_line(
    data: &SelectedGameData,
    item_id: i32,
    members: bool,
    held: i32,
    banked: i32,
) -> String {
    let vendors = api::shop_facts::vendors_for(data, item_id)
        .expect("generated shop facts")
        .filter(|vendor| members || !vendor.item.members)
        .collect::<Vec<_>>();
    if vendors.is_empty() {
        return "nobody sells it".to_owned();
    }
    let mut names = String::new();
    let mut low = i64::MAX;
    let mut high = 0i64;
    for vendor in &vendors {
        if !names.is_empty() {
            names.push_str(" and ");
        }
        names.push_str(vendor.keeper_name);
        let price = api::shop_facts::quote(vendor.record, vendor.item, 1, vendor.item.baseline);
        low = low.min(price);
        high = high.max(price);
    }
    let price_range = if low == high {
        low.to_string()
    } else {
        format!("{low}–{high}")
    };
    let (noun, verb) = if vendors.len() == 1 {
        ("vendor", "sells")
    } else {
        ("eligible vendors", "sell")
    };
    format!(
        "{noun} {names} {verb} one for {price_range} gp at baseline stock; you hold {held} gp and the bank holds {banked} gp"
    )
}

/// The parked reason's bank clause for the observed status `bank_seen` line,
/// e.g. `Session, 12 min ago, 37 rows` or `Hint (saved), 37 rows`. A relog
/// carries the same observation but relabels it a saved hint.
fn bank_clause_from_seen(bank_seen: &str) -> Option<String> {
    let (source, rest) = if let Some(rest) = bank_seen.strip_prefix("Session") {
        ("this session", rest)
    } else {
        ("from a saved hint", bank_seen.strip_prefix("Hint (saved)")?)
    };
    let rest = rest.strip_prefix(", ")?;
    let rows = rest.strip_suffix(" rows")?;
    let (age, rows) = match rows.rsplit_once(", ") {
        Some((age, rows)) => (Some(age), rows),
        None => (None, rows),
    };
    let seen = match age {
        Some(age) => format!("{age} {source}"),
        None => source.to_owned(),
    };
    Some(format!("in the bank as seen {seen} ({rows} rows)"))
}

/// Rewrite a parked reason's bank clause to the one `bank_seen` names.
fn parked_reason_for_bank_seen(reason: &str, bank_seen: &str) -> Option<String> {
    let start = reason.find("in the bank as seen ")?;
    let end = reason[start..]
        .find(" rows)")
        .map(|at| at + start + " rows)".len())?;
    let clause = bank_clause_from_seen(bank_seen)?;
    Some(format!("{}{clause}{}", &reason[..start], &reason[end..]))
}

fn expected_empty_bank_block(
    data: &SelectedGameData,
    tool: &ItemFact,
    bank_seen: &str,
    members: bool,
) -> Option<String> {
    let clause = bank_clause_from_seen(bank_seen)?;
    let vendor = nearest_vendor_line(data, tool.id, members, 0, 0);
    Some(format!(
        "no {} in the pack, worn, or {clause}; {vendor}; waiting: put a {} or coins in the pack",
        tool.name, tool.name
    ))
}

fn status_value(status: &ScriptStatus) -> Value {
    let fields = status
        .fields
        .iter()
        .map(|field| {
            let value = match &field.value {
                StatusValue::Text(value) => json!(value.as_ref()),
                StatusValue::Integer(value) => json!(value),
                StatusValue::Tile(value) => json!(value),
                StatusValue::Truth(value) => json!(format!("{value:?}")),
                StatusValue::Quest(value) => json!({"debug": format!("{value:?}")}),
            };
            (field.key.to_owned(), value)
        })
        .collect::<serde_json::Map<_, _>>();
    json!({
        "run": format!("{:?}", status.run),
        "phase": format!("{:?}", status.phase),
        "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
        "fields": fields,
    })
}

struct ParkPlaySnapshot {
    slot_state: script::RunState,
    want_run: bool,
    host_up: bool,
    bank_origin: String,
    bank_rows: Vec<(i32, i32)>,
}

fn read_park_play_snapshot(play: &super::Play, account: &str) -> ParkPlaySnapshot {
    let (slot_state, want_run) = super::script_runtime::script_slot(&play.scripts, account)
        .and_then(|slot| slot.lock().ok().map(|slot| (slot.state(), slot.want_run)))
        .unwrap_or((script::RunState::Idle, false));
    let host_up = play
        .statuses()
        .iter()
        .find(|status| status.username == account)
        .is_some_and(super::SlotStatus::is_up);
    let bank = play.bank_rows(account);
    ParkPlaySnapshot {
        slot_state,
        want_run,
        host_up,
        bank_origin: origin_name(bank.origin),
        bank_rows: bank.rows,
    }
}

fn park_observation(
    status: &ScriptStatus,
    trace: &Mutex<LiveTrace>,
    actions: &[Value],
    fixture: &Fixture,
    read_play: impl FnOnce() -> ParkPlaySnapshot,
) -> ParkObservation {
    let play = read_play();
    let trace = trace.lock();
    let frame = trace.frames.last();
    let bank_open = frame.and_then(|frame| frame["bank_open"].as_bool());
    ParkObservation {
        run_slot: Some(status.run.slot),
        run_generation: Some(status.run.run),
        run_session: Some(status.run.session),
        phase: Some(format!("{:?}", status.phase)),
        phase_label: field_text(status, "phase").map(str::to_owned),
        slot_state: Some(format!("{:?}", play.slot_state)),
        slot_running: Some(play.slot_state == script::RunState::Running),
        want_run: Some(play.want_run),
        host_up: Some(play.host_up),
        failure: status
            .failure
            .as_ref()
            .map(|failure| failure.message.to_string()),
        block_reason: field_text(status, "block_reason").map(str::to_owned),
        bank_seen: field_text(status, "bank_seen").map(str::to_owned),
        bank_origin: Some(play.bank_origin),
        bank_rows: Some(play.bank_rows),
        bank_open,
        session_generation: frame.and_then(|frame| frame["session_generation"].as_u64()),
        tool_in_pack: frame
            .map(|frame| count_rows(&frame["inventory"], i64::from(fixture.tool.id))),
        coins_in_pack: frame
            .map(|frame| count_rows(&frame["inventory"], i64::from(fixture.coins.id))),
        no_walk: Some(!actions.iter().any(|action| action["kind"] == "walk")),
        no_bank_trip: Some(
            bank_open == Some(false)
                && !actions
                    .iter()
                    .any(|action| action["request"]["op"] == "open-stand"),
        ),
    }
}

fn relog_park_preserved(initial: &ParkObservation, observation: &ParkObservation) -> bool {
    let reason_matches = match (
        initial.block_reason.as_deref(),
        observation.block_reason.as_deref(),
        observation.bank_seen.as_deref(),
    ) {
        (Some(before), Some(after), Some(bank_seen)) => {
            parked_reason_for_bank_seen(before, bank_seen).is_some_and(|expected| expected == after)
        }
        _ => false,
    };
    reason_matches
        && observation.phase.as_deref() == Some("Blocked")
        && observation.phase_label.as_deref() == Some("blocked")
        && observation.want_run == Some(true)
        && observation.host_up.is_some()
        && matches!(
            (observation.slot_state.as_deref(), observation.slot_running),
            (Some("Running"), Some(true)) | (Some("Paused"), Some(false))
        )
        && observation.failure.is_none()
        && observation.run_slot == initial.run_slot
        && observation.run_generation == initial.run_generation
        && observation.run_session != initial.run_session
        && observation.session_generation != initial.session_generation
        && observation.bank_origin.as_deref() == Some("Hint")
        && observation
            .bank_rows
            .as_ref()
            .is_some_and(|rows| rows.is_empty())
        && observation.bank_open == Some(false)
        && observation.tool_in_pack == Some(0)
        && observation.coins_in_pack == Some(0)
        && observation.no_walk == Some(true)
        && observation.no_bank_trip == Some(true)
}

fn relog_park_settled(
    observation: &ParkObservation,
    clock_accepted: bool,
    elapsed: Duration,
) -> Result<bool, &'static str> {
    // A published ready row and the slot gate can straddle a pump turn.
    // Neither a loading scene nor its retained Paused state is a final pass.
    if observation.host_up == Some(true) && observation.slot_running == Some(true) && clock_accepted
    {
        Ok(true)
    } else if elapsed > Duration::from_secs(30) {
        Err("Running readiness and the 600 ms clock did not stabilize after relog")
    } else {
        Ok(false)
    }
}

fn relog_park_oracle_fixture() -> (ParkObservation, ParkObservation) {
    let initial_reason = "no Harpoon in the pack, worn, or in the bank as seen less than a minute ago this session (0 rows); nobody sells it; waiting: put a Harpoon in the pack";
    let after_relog_reason = "no Harpoon in the pack, worn, or in the bank as seen less than a minute ago from a saved hint (0 rows); nobody sells it; waiting: put a Harpoon in the pack";
    let initial = ParkObservation {
        run_slot: Some(1),
        run_generation: Some(3),
        run_session: Some(1),
        session_generation: Some(2),
        block_reason: Some(initial_reason.into()),
        bank_seen: Some("Session, less than a minute ago, 0 rows".into()),
        bank_origin: Some("Session".into()),
        ..ParkObservation::default()
    };
    let loading = ParkObservation {
        run_slot: Some(1),
        run_generation: Some(3),
        run_session: Some(2),
        session_generation: Some(3),
        phase: Some("Blocked".into()),
        phase_label: Some("blocked".into()),
        slot_state: Some("Paused".into()),
        slot_running: Some(false),
        want_run: Some(true),
        host_up: Some(false),
        block_reason: Some(after_relog_reason.into()),
        bank_seen: Some("Hint (saved), less than a minute ago, 0 rows".into()),
        bank_origin: Some("Hint".into()),
        bank_rows: Some(Vec::new()),
        bank_open: Some(false),
        tool_in_pack: Some(0),
        coins_in_pack: Some(0),
        no_walk: Some(true),
        no_bank_trip: Some(true),
        ..ParkObservation::default()
    };
    (initial, loading)
}

#[test]
fn relog_park_oracle_preserves_a_readiness_pause_until_the_scene_returns() {
    let (initial, mut observation) = relog_park_oracle_fixture();
    assert!(relog_park_preserved(&initial, &observation));
    assert_eq!(
        relog_park_settled(&observation, true, Duration::ZERO),
        Ok(false)
    );
    observation.host_up = Some(true);
    assert!(relog_park_preserved(&initial, &observation));
    assert_eq!(
        relog_park_settled(&observation, true, Duration::ZERO),
        Ok(false)
    );
    observation.slot_state = Some("Running".into());
    observation.slot_running = Some(true);
    assert!(relog_park_preserved(&initial, &observation));
    assert_eq!(
        relog_park_settled(&observation, false, Duration::ZERO),
        Ok(false)
    );
    assert_eq!(
        relog_park_settled(&observation, true, Duration::ZERO),
        Ok(true)
    );
}

#[test]
fn relog_park_oracle_rejects_changed_park_or_operator_pause() {
    type Change = fn(&mut ParkObservation);
    let (initial, observation) = relog_park_oracle_fixture();
    let bad_samples: &[(&str, Change)] = &[
        ("operator Pause", |sample| sample.want_run = Some(false)),
        ("missing intent", |sample| sample.want_run = None),
        ("missing readiness", |sample| sample.host_up = None),
        ("Idle", |sample| sample.slot_state = Some("Idle".into())),
        ("inconsistent gate", |sample| {
            sample.slot_running = Some(true)
        }),
        ("woken phase", |sample| {
            sample.phase = Some("Working".into())
        }),
        ("woken label", |sample| {
            sample.phase_label = Some("gathering".into())
        }),
        ("terminal failure", |sample| {
            sample.failure = Some("failed".into())
        }),
        ("different block reason", |sample| {
            sample.block_reason = Some("changed".into())
        }),
        ("different slot", |sample| sample.run_slot = Some(2)),
        ("different run", |sample| sample.run_generation = Some(4)),
        ("old native session", |sample| sample.run_session = Some(1)),
        ("old client session", |sample| {
            sample.session_generation = Some(2)
        }),
        ("fresh bank evidence", |sample| {
            sample.bank_origin = Some("Session".into())
        }),
        ("changed bank rows", |sample| {
            sample.bank_rows = Some(vec![(995, 1)])
        }),
        ("missing bank rows", |sample| sample.bank_rows = None),
        ("open bank", |sample| sample.bank_open = Some(true)),
        ("tool arrived", |sample| sample.tool_in_pack = Some(1)),
        ("coins arrived", |sample| sample.coins_in_pack = Some(1)),
        ("walk", |sample| sample.no_walk = Some(false)),
        ("bank trip", |sample| sample.no_bank_trip = Some(false)),
    ];
    for (name, change) in bad_samples {
        let mut bad = observation.clone();
        change(&mut bad);
        assert!(!relog_park_preserved(&initial, &bad), "{name}");
    }
}

#[test]
fn relog_park_oracle_rejects_a_pause_or_load_that_never_settles() {
    let (_, mut observation) = relog_park_oracle_fixture();
    let deadline = Duration::from_secs(30) + Duration::from_millis(1);
    assert!(relog_park_settled(&observation, true, deadline).is_err());
    observation.host_up = Some(true);
    assert!(relog_park_settled(&observation, true, deadline).is_err());
    observation.slot_state = Some("Running".into());
    observation.slot_running = Some(true);
    observation.host_up = Some(false);
    assert!(relog_park_settled(&observation, true, deadline).is_err());
    observation.host_up = Some(true);
    assert!(relog_park_settled(&observation, false, deadline).is_err());
}

fn record_final_status(
    play: &super::Play,
    account: &str,
    trace: &Arc<Mutex<LiveTrace>>,
    status: &ScriptStatus,
) {
    let slot_state = play.script_state(account);
    let current = trace.lock().frames.last().cloned();
    let bank = play.bank_rows(account);
    let mut guard = trace.lock();
    guard.final_phase = Some(format!("{:?}", status.phase));
    guard.final_phase_label = field_text(status, "phase").map(str::to_owned);
    guard.final_slot_state = Some(format!("{slot_state:?}"));
    guard.final_slot_running = Some(slot_state == script::RunState::Running);
    guard.final_failure = status
        .failure
        .as_ref()
        .map(|failure| failure.message.to_string());
    guard.final_block_reason = field_text(status, "block_reason").map(str::to_owned);
    guard.final_bank_open = current
        .as_ref()
        .and_then(|frame| frame["bank_open"].as_bool());
    guard.final_shop_open = current
        .as_ref()
        .and_then(|frame| frame["shop_open"].as_bool());
    guard.final_bank_origin = Some(origin_name(bank.origin));
    guard.final_bank_rows = Some(bank.rows.clone());
}

fn over_target_failures(actions: &[Value], frames: &[Value], item_id: i32) -> Vec<String> {
    let mut failures = Vec::new();
    for snapshot in frames
        .iter()
        .chain(actions.iter().map(|action| &action["snapshot"]))
    {
        let held = held_total(snapshot, i64::from(item_id));
        if held > 1 {
            failures.push(format!(
                "held tool count exceeded target 1 at tick {:?}: {held}",
                snapshot["snapshot_tick"]
            ));
        }
    }
    failures
}

fn run_cell(case: Case) {
    run_cell_with_transition(case, ParkTransition::None);
}

fn run_cell_with_transition(case: Case, transition: ParkTransition) {
    assert!(
        transition == ParkTransition::None || case == Case::NoCoins,
        "G2 lifecycle transitions use the no-coins Harpoon fixture"
    );
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "Engine L cells require BOT_CPU=1"
    );
    assert_eq!(
        std::env::var("BOT_LIVE_NAME_PREFIX").as_deref(),
        Ok("gsh"),
        "this branch owns the gsh account namespace"
    );
    assert_eq!(
        std::env::var("GATHERER_GAME_PORT").as_deref(),
        Ok("47594"),
        "use Engine L only"
    );
    assert_eq!(
        std::env::var("GATHERER_HTTP_PORT").as_deref(),
        Ok("3080"),
        "use Engine L only"
    );
    let scenario_name = transition.scenario_name(case);

    let evidence_root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR is required"),
    );
    let account = super::mint_live_names(1)
        .pop()
        .expect("mint one gsh account");
    let relog_hint_path = if transition == ParkTransition::Relog {
        let home = PathBuf::from(std::env::var_os("HOME").expect("isohome HOME"));
        let identity = api::local_state::AccountKey::try_new(PROFILE, &account)
            .expect("a minted live name is a login identity");
        let path = crate::local_state::StateRoot::new(home.join(".274bot"))
            .bank_hint_file(PROFILE, identity.account());
        assert!(
            path.starts_with(&home),
            "bank hint must remain under isohome HOME"
        );
        assert!(
            !path.exists(),
            "minted account must not have a prior bank hint before the relog cell: {}",
            path.display()
        );
        Some(path)
    } else {
        None
    };
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let evidence = evidence_root.join(format!("{scenario_name}-{account}-utc-{epoch}Z"));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence scratch");
    let options = live_profile(&scratch).expect("live profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind Engine L R289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback Engine L only"
    );
    assert_eq!(profile.name(), PROFILE, "bank hint profile identity");
    let selected = profile.game_data().expect("selected R289 game data");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("selected R289 navigation world");
    if world.named_bank_facts().is_none() {
        world
            .bind_named_bank_facts(&selected)
            .expect("bind selected R289 service facts before Play starts");
    }
    if world.shop_keeper_facts().is_none() {
        world
            .bind_shop_keeper_facts(&selected)
            .expect("bind selected R289 shop facts before fixture lookup");
    }
    let keepers = Arc::clone(
        world
            .shop_keeper_facts()
            .expect("selected R289 shop keeper placements"),
    );
    let fixture = fixture(case, &selected, &keepers)
        .unwrap_or_else(|error| panic!("R289 content fixture: {error}"));
    assert_eq!(fixture.coins.id, COIN_ID, "pinned coins id");
    match case {
        Case::Harpoon | Case::NoCoins => assert_eq!(fixture.tool.id, HARPOON_ID),
        Case::AxeWear => assert_eq!(fixture.tool.id, IRON_AXE_ID),
    }
    if case == Case::AxeWear {
        assert_eq!(fixture.baseline, 5, "Bob's pinned Iron axe stock");
        assert_eq!(fixture.cost, 56, "selected R289 Iron axe cost");
        assert_eq!(fixture.sell, 1_000, "Bob's sell multiplier");
        assert_eq!(fixture.delta, 20, "Bob's shop delta");
        assert_eq!(fixture.quote, 56, "Iron axe quote at Bob's baseline stock");
        assert_eq!(fixture.reserve, 113, "two-chunk Iron axe purchase reserve");
    }
    assert!(
        fixture.quote > 0,
        "pinned shop quote must be positive: {}",
        fixture.quote
    );
    if case != Case::NoCoins {
        assert!(
            i64::from(case.coin_seed()) >= fixture.quote,
            "account-only coin seed {} cannot fund selected shop quote {}",
            case.coin_seed(),
            fixture.quote
        );
    }
    let seed_commands = case.seed().to_vec();
    let trace = Arc::new(Mutex::new(LiveTrace::default()));
    let capture = Arc::new(StdMutex::new(CombatCapture::default()));
    let _capture_registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let cell = Cell::new(
        CELL_BOUND,
        case.seed(),
        |_| true,
        &[COIN_ID, HARPOON_ID, IRON_AXE_ID],
        json!({
            "scenario": scenario_name,
            "transition": transition,
            "case": case,
            "seed_commands": seed_commands,
            "bank_coins": case.coin_seed(),
            "start": case.start(),
            "shop": case.keeper(),
            "tool": fixture.tool,
            "cell_bound_ms": CELL_BOUND.as_millis(),
            "engine": {"game_port": ENGINE_L_GAME_PORT, "http_port": ENGINE_L_HTTP_PORT, "tick_ms": 600},
        }),
        |_| true,
        evidence.join("bank-fixture"),
    );
    let preparation_started = cell.preparation_started;
    let cell = Arc::new(Mutex::new(cell));
    let frame_cell = Arc::clone(&cell);
    let frame_trace = Arc::clone(&trace);
    let frame_packets = Arc::new(Mutex::new(DriverPacketCounts::default()));
    let callback_packets = Arc::clone(&frame_packets);
    let frame_account = account.clone();
    let observer = Arc::new(Mutex::new(GameSnapshot::new()));
    let frame_observer = Arc::clone(&observer);
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: (epoch % i32::MAX as u64) as i32,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, username, _| {
            if username != frame_account {
                return;
            }
            frame(client, &frame_cell, &frame_account);
            if !client.ingame || client.scene_state != 2 {
                return;
            }
            let mut observer = frame_observer.lock();
            observer.rebuild(client);
            let row = frame_value(&observer, client.gens.session, client.gens.player_info);
            let (start_requested, finish_requested) = {
                let mut trace = frame_trace.lock();
                trace
                    .clock
                    .observe(client.gens.session, client.gens.player_info, Instant::now());
                let start_requested = trace.start_requested;
                let finish_requested = trace.finish_requested;
                if trace.frames.last() != Some(&row) {
                    trace.frames.push(row);
                }
                (start_requested, finish_requested)
            };
            if start_requested {
                let mut trace = frame_trace.lock();
                if !trace.start_baseline_recorded {
                    combat_proof::record_start_baseline(&frame_account, &observer);
                    trace.start_baseline_recorded = true;
                }
                let mut packets = callback_packets.lock();
                if !packets.started() {
                    packets.begin(client, observer.tick());
                } else if finish_requested {
                    packets.finish(client, observer.tick());
                } else {
                    packets.observe(client, observer.tick());
                }
            }
        },
    )
    .expect("start local Engine L Play");
    let relog_arm = if transition == ParkTransition::Relog {
        Some(play.arm(&account).expect("the gsh account slot arm"))
    } else {
        None
    };
    let named_banks = play.named_banks();
    let bank_name = named_banks
        .banks()
        .iter()
        .find(|bank| bank.name.contains("Draynor"))
        .map(|bank| Arc::<str>::from(bank.name))
        .expect("selected bank facts contain Draynor");
    let expected_bank_rows = if case.coin_seed() > 0 {
        vec![(fixture.coins.id, case.coin_seed())]
    } else {
        Vec::new()
    };
    let primer_trace = Arc::clone(&trace);
    let prime_cell = Arc::clone(&cell);
    let mut primer = Some(BankPrimer {
        facts: Arc::clone(&named_banks),
        bank_name: Arc::clone(&bank_name),
        expected_rows: expected_bank_rows.clone(),
        trace: primer_trace,
        cell: prime_cell,
        step: PrimeStep::Record,
        select: None,
        open: None,
        close: None,
        access: None,
    });
    let mut login_ready = false;
    let mut stage = RunStage::Prepare;
    let mut cell_failure = None::<String>;
    let mut final_status = None::<Arc<ScriptStatus>>;
    let operation_deadline = Instant::now() + PREPARATION_DEADLINE + CELL_BOUND;
    loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        if Instant::now() >= operation_deadline {
            cell_failure = Some(format!("{} exceeded its fixed live deadline", case.name()));
            break;
        }
        if let Some(failure) = fixture_terminal_failure(&cell) {
            cell_failure = Some(failure);
            break;
        }
        let status = play.script_native_status(&account);
        if let Some(status) = status.as_ref() {
            combat_proof::record_status(&account, status, None);
        }
        match stage {
            RunStage::Prepare => {
                let clock = trace.lock().clock.clone();
                if clock.median_ms.is_some() && !clock.accepted() {
                    cell_failure = Some(format!("Engine L tick clock is not 600 ms: {clock:?}"));
                    break;
                }
                let begin = {
                    let mut guard = cell.lock();
                    if clock.accepted()
                        && guard.phase == Prep::Ready
                        && guard.script_started.is_none()
                        && Instant::now().duration_since(guard.last_action)
                            >= Duration::from_secs(1)
                    {
                        guard.script_started = Some(Instant::now());
                        true
                    } else {
                        false
                    }
                };
                if begin {
                    let rows = play.bank_rows(&account);
                    if rows.origin != Origin::Unknown || !rows.rows.is_empty() {
                        cell_failure = Some(format!(
                            "fresh profile did not begin with Unknown bank memory: {rows:?}"
                        ));
                        break;
                    }
                    let script = primer.take().expect("bank primer starts once");
                    match play.script_start_handle().start_test_script(
                        &account,
                        Box::new(script),
                        Some(Arc::clone(&selected)),
                    ) {
                        Ok(run) if play.script_native_run(&account) == Some(run) => {
                            play.wake(&account);
                            stage = RunStage::Prime;
                        }
                        Ok(_) => {
                            cell_failure = Some("Play did not publish the bank-primer run".into());
                            break;
                        }
                        Err(error) => {
                            cell_failure = Some(format!("Play refused bank primer: {error}"));
                            break;
                        }
                    }
                }
            }
            RunStage::Prime => {
                let prime = trace.lock().prime.clone();
                if let Some(error) = prime.failure {
                    cell_failure = Some(error);
                    break;
                }
                if prime.complete {
                    play.script_stop(&account);
                    let rows = play.bank_rows(&account);
                    if rows.origin != Origin::Session || rows.rows != expected_bank_rows {
                        cell_failure = Some(format!(
                            "bank primer did not leave the exact Session memory: {rows:?} expected {expected_bank_rows:?}"
                        ));
                        break;
                    }
                    match play.cheat(&account, &tele_args(case.start())) {
                        Ok(()) => stage = RunStage::Travel,
                        Err(error) => {
                            cell_failure =
                                Some(format!("teleport to the real start site: {error}"));
                            break;
                        }
                    }
                }
            }
            RunStage::Travel => {
                let current = trace.lock().frames.last().cloned();
                let Some(current) = current else {
                    thread::sleep(POLL_INTERVAL);
                    continue;
                };
                let tile = current["tile"].as_array().and_then(|tile| {
                    Some((
                        i32::try_from(tile.first()?.as_i64()?).ok()?,
                        i32::try_from(tile.get(1)?.as_i64()?).ok()?,
                        i32::try_from(tile.get(2)?.as_i64()?).ok()?,
                    ))
                });
                let expected_tile = (case.start().x, case.start().z, case.start().level);
                let bank_rows = play.bank_rows(&account);
                if tile == Some(expected_tile)
                    && current["bank_open"] == false
                    && bank_rows.origin == Origin::Session
                    && bank_rows.rows == expected_bank_rows
                {
                    let mut guard = trace.lock();
                    guard.start_requested = true;
                    guard.start_session = current["session_generation"].as_u64();
                    guard.start_player_info = current["player_info_generation"].as_u64();
                    guard.start_game_tick =
                        current["snapshot_tick"].as_u64().map(|tick| tick as u32);
                    guard.start_tile = tile;
                    guard.start_bank_origin = Some(origin_name(bank_rows.origin));
                    guard.start_bank_rows = Some(bank_rows.rows.clone());
                    guard.start_inventory = Some(current["inventory"].clone());
                    guard.start_equipment = Some(current["equipment"].clone());
                    guard.start_server_closes = current["shop_server_closes"].as_u64();
                    let start_tool_count = held_total(&current, i64::from(fixture.tool.id));
                    let start_coin_count =
                        count_rows(&current["inventory"], i64::from(fixture.coins.id));
                    if start_tool_count != 0 || start_coin_count != 0 {
                        cell_failure = Some(format!(
                            "Start must hold neither tool nor bank-seeded coins: tool={start_tool_count} coins={start_coin_count}"
                        ));
                        break;
                    }
                    let action_count = capture
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .actions
                        .len();
                    guard.start_action_index = Some(action_count);
                    drop(guard);
                    let settings = settings(case);
                    match play.script_start(&account, script::CompiledId("Gatherer"), settings) {
                        Ok(()) => stage = RunStage::Gather,
                        Err(error) => {
                            cell_failure = Some(format!("Gatherer Start: {error}"));
                            break;
                        }
                    }
                }
            }
            RunStage::Gather => {
                if let Some(status) = status.as_ref() {
                    final_status = Some(status.clone());
                    record_final_status(&play, &account, &trace, status);
                    if status.failure.is_some() {
                        cell_failure =
                            Some(format!("Gatherer terminal failure: {:?}", status.failure));
                        break;
                    }
                    let needs_initial_park = case == Case::NoCoins
                        && status.phase == NativePhase::Blocked
                        && trace.lock().park_lifecycle.initial.is_none();
                    if needs_initial_park {
                        let start_index = trace.lock().start_action_index.unwrap_or(0);
                        let actions = capture
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .actions
                            .iter()
                            .skip(start_index)
                            .cloned()
                            .collect::<Vec<_>>();
                        let observation =
                            park_observation(status, &trace, &actions, &fixture, || {
                                read_park_play_snapshot(&play, &account)
                            });
                        trace.lock().park_no_walk = observation.no_walk;
                        trace.lock().park_lifecycle.initial = Some(observation.clone());
                        let expected = observation.bank_seen.as_deref().and_then(|seen| {
                            expected_empty_bank_block(
                                &selected,
                                &fixture.tool,
                                seen,
                                play.map_members(),
                            )
                        });
                        if observation.phase.as_deref() != Some("Blocked")
                            || observation.phase_label.as_deref() != Some("blocked")
                            || observation.run_slot.is_none()
                            || observation.run_generation.is_none()
                            || observation.run_session.is_none()
                            || observation.session_generation.is_none()
                            || observation.slot_running != Some(true)
                            || observation.failure.is_some()
                            || observation.block_reason.is_none()
                            || observation.bank_origin.as_deref() != Some("Session")
                            || observation
                                .bank_rows
                                .as_ref()
                                .is_none_or(|rows| !rows.is_empty())
                            || observation.bank_open != Some(false)
                            || observation.tool_in_pack != Some(0)
                            || observation.coins_in_pack != Some(0)
                            || observation.no_walk != Some(true)
                            || observation.no_bank_trip != Some(true)
                        {
                            cell_failure = Some(format!(
                                "supply shortage did not park Running/Blocked without failure, movement, or a bank trip: {observation:?}"
                            ));
                            break;
                        }
                        if expected.as_deref() != observation.block_reason.as_deref() {
                            cell_failure = Some(format!(
                                "exact no-coins block_reason mismatch: expected {expected:?}, got {:?}",
                                observation.block_reason
                            ));
                            break;
                        }
                        match transition {
                            ParkTransition::None => stage = RunStage::Done,
                            ParkTransition::Resume => {
                                play.script_pause(&account);
                                trace.lock().park_lifecycle.pause_requested = true;
                                stage = RunStage::ParkResumePause {
                                    since: Instant::now(),
                                };
                            }
                            ParkTransition::Relog => {
                                trace.lock().finish_requested = true;
                                stage = RunStage::ParkRelogTraceFinish {
                                    since: Instant::now(),
                                };
                            }
                        }
                    }
                }
                if case != Case::NoCoins || transition == ParkTransition::Resume {
                    let action_start = trace.lock().start_action_index.unwrap_or(0);
                    let actions = capture
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .actions
                        .iter()
                        .skip(action_start)
                        .cloned()
                        .collect::<Vec<_>>();
                    let observed_frames = trace.lock().frames.clone();
                    if let Some(action) =
                        gather_action(&actions, &observed_frames, case, case.start(), 12)
                    {
                        let tick = action["snapshot_tick"].as_u64().unwrap_or(0) as u32;
                        let mut guard = trace.lock();
                        guard.gather_action = Some(action);
                        guard.content_window_end_tick =
                            Some(tick.wrapping_add(case.content_window() as u32));
                    }
                    if content_window_finished(&trace) {
                        stage = RunStage::Done;
                    }
                }
            }
            RunStage::ParkResumePause { since } => {
                if play.script_state(&account) == script::RunState::Paused {
                    trace.lock().park_lifecycle.pause_observed = true;
                    match play.cheat(&account, "give harpoon 1") {
                        Ok(()) => {
                            trace.lock().park_lifecycle.supply_command =
                                Some("give harpoon 1".into());
                            stage = RunStage::ParkResumeSupply {
                                since: Instant::now(),
                            };
                        }
                        Err(error) => {
                            cell_failure =
                                Some(format!("account-only Harpoon arrival was refused: {error}"));
                            break;
                        }
                    }
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure =
                        Some("Gatherer did not enter Paused before supply arrival".into());
                    break;
                }
            }
            RunStage::ParkResumeSupply { since } => {
                let supplied = trace
                    .lock()
                    .frames
                    .last()
                    .is_some_and(|frame| held_total(frame, i64::from(fixture.tool.id)) == 1);
                if supplied {
                    trace.lock().park_lifecycle.supply_observed = true;
                    play.script_resume(&account);
                    trace.lock().park_lifecycle.resume_requested = true;
                    stage = RunStage::ParkResumeObserve {
                        since: Instant::now(),
                    };
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure =
                        Some("account-only Harpoon arrival did not reach the pack".into());
                    break;
                }
            }
            RunStage::ParkResumeObserve { since } => {
                if let Some(status) = status.as_ref() {
                    final_status = Some(status.clone());
                    record_final_status(&play, &account, &trace, status);
                    if status.failure.is_some() {
                        cell_failure = Some(format!(
                            "Gatherer failed after Resume: {:?}",
                            status.failure
                        ));
                        break;
                    }
                    if play.script_state(&account) == script::RunState::Running
                        && status.phase != NativePhase::Blocked
                    {
                        let initial = trace
                            .lock()
                            .park_lifecycle
                            .initial
                            .clone()
                            .expect("Resume follows an observed park");
                        if status.run.slot != initial.run_slot.unwrap_or(u64::MAX)
                            || status.run.run != initial.run_generation.unwrap_or(u64::MAX)
                            || Some(status.run.session) != initial.run_session
                        {
                            cell_failure = Some(format!(
                                "Resume replaced the parked native run: initial={initial:?} resumed={:?}",
                                status.run
                            ));
                            break;
                        }
                        trace.lock().park_lifecycle.resume_observed = true;
                        stage = RunStage::Gather;
                    } else if since.elapsed() > Duration::from_secs(30) {
                        cell_failure = Some("Resume did not re-plan the supplied Harpoon".into());
                        break;
                    }
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure = Some("Resume did not publish a native status".into());
                    break;
                }
            }
            RunStage::ParkRelogTraceFinish { since } => {
                if frame_packets.lock().finished() {
                    let Some(arm) = relog_arm.as_ref() else {
                        cell_failure = Some("relog cell has no account slot arm".into());
                        break;
                    };
                    arm.request_logout();
                    play.wake(&account);
                    trace.lock().park_lifecycle.logout_requested = true;
                    stage = RunStage::ParkRelogLogout {
                        since: Instant::now(),
                    };
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure = Some("Driver trace did not close before relog".into());
                    break;
                }
            }
            RunStage::ParkRelogLogout { since } => {
                if !play.slot_connected(&account) {
                    trace.lock().park_lifecycle.logout_observed = true;
                    let hint_written = relog_hint_path.as_ref().is_some_and(|path| path.is_file());
                    if hint_written {
                        let Some(arm) = relog_arm.as_ref() else {
                            cell_failure = Some("relog cell has no account slot arm".into());
                            break;
                        };
                        arm.arm_explicit_login();
                        play.wake(&account);
                        trace.lock().park_lifecycle.relog_requested = true;
                        stage = RunStage::ParkRelogLogin {
                            since: Instant::now(),
                        };
                    } else if since.elapsed() > Duration::from_secs(30) {
                        cell_failure = Some(format!(
                            "logout did not persist bank hint {}",
                            relog_hint_path.as_ref().map_or_else(
                                || "<missing path>".to_owned(),
                                |path| path.display().to_string()
                            )
                        ));
                        break;
                    }
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure = Some("Gatherer account did not log out".into());
                    break;
                }
            }
            RunStage::ParkRelogLogin { since } => {
                let account_status = play
                    .statuses()
                    .into_iter()
                    .find(|status| status.username == account);
                let ready = account_status.as_ref().is_some_and(|status| {
                    status.connected
                        && status.ingame
                        && status.scene_state == 2
                        && !status.welcome_hold
                        && status.main_modal_id == -1
                });
                let initial_session = trace
                    .lock()
                    .park_lifecycle
                    .initial
                    .as_ref()
                    .and_then(|park| park.session_generation);
                let current_session = trace
                    .lock()
                    .frames
                    .last()
                    .and_then(|frame| frame["session_generation"].as_u64());
                let native = play.script_native_status(&account);
                let initial = trace.lock().park_lifecycle.initial.clone();
                let same_native_run = native.as_ref().is_some_and(|status| {
                    initial.as_ref().is_some_and(|park| {
                        status.run.slot == park.run_slot.unwrap_or(u64::MAX)
                            && status.run.run == park.run_generation.unwrap_or(u64::MAX)
                            && status.run.session != park.run_session.unwrap_or(u64::MAX)
                    })
                });
                if ready
                    && current_session.is_some_and(|session| Some(session) != initial_session)
                    && same_native_run
                    && native.as_ref().is_some_and(|status| {
                        status.phase == NativePhase::Blocked
                            && status.failure.is_none()
                            && play.script_state(&account) == script::RunState::Running
                    })
                {
                    let rows = play.bank_rows(&account);
                    if rows.origin != Origin::Hint || !rows.rows.is_empty() {
                        cell_failure = Some(format!(
                            "relog must demote the empty Session bank to an empty Hint: {rows:?}"
                        ));
                        break;
                    }
                    let Some(status) = native.as_ref() else {
                        cell_failure = Some("parked status disappeared during relog".into());
                        break;
                    };
                    let start_index = trace.lock().start_action_index.unwrap_or(0);
                    let actions = capture
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .actions
                        .iter()
                        .skip(start_index)
                        .cloned()
                        .collect::<Vec<_>>();
                    let observation = park_observation(status, &trace, &actions, &fixture, || {
                        read_park_play_snapshot(&play, &account)
                    });
                    let initial = trace
                        .lock()
                        .park_lifecycle
                        .initial
                        .clone()
                        .expect("relog follows an observed park");
                    let preserves_park = relog_park_preserved(&initial, &observation);
                    if !preserves_park {
                        cell_failure = Some(format!(
                            "relog did not preserve the Running/Blocked park without waking or a bank trip: initial={initial:?} after_relog={observation:?}"
                        ));
                        break;
                    }
                    trace.lock().park_lifecycle.after_relog = Some(observation);
                    trace.lock().park_lifecycle.relog_observed = true;
                    final_status = Some(status.clone());
                    record_final_status(&play, &account, &trace, status);
                    stage = RunStage::ParkRelogObserve {
                        since: Instant::now(),
                    };
                } else if since.elapsed() > Duration::from_secs(60) {
                    cell_failure = Some(format!(
                        "account did not relog into a new parked session: status={account_status:?} native={native:?}"
                    ));
                    break;
                }
            }
            RunStage::ParkRelogObserve { since } => {
                if let Some(status) = status.as_ref() {
                    final_status = Some(status.clone());
                    record_final_status(&play, &account, &trace, status);
                    let start_index = trace.lock().start_action_index.unwrap_or(0);
                    let actions = capture
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .actions
                        .iter()
                        .skip(start_index)
                        .cloned()
                        .collect::<Vec<_>>();
                    let observation = park_observation(status, &trace, &actions, &fixture, || {
                        read_park_play_snapshot(&play, &account)
                    });
                    let initial = trace
                        .lock()
                        .park_lifecycle
                        .initial
                        .clone()
                        .expect("relog observation follows the initial park");
                    if !relog_park_preserved(&initial, &observation) {
                        cell_failure = Some(format!(
                            "park changed during relog observation: {observation:?}"
                        ));
                        break;
                    }
                    let settled = relog_park_settled(
                        &observation,
                        trace.lock().clock.accepted(),
                        since.elapsed(),
                    );
                    {
                        let mut guard = trace.lock();
                        if guard.park_lifecycle.relog_observations.last() != Some(&observation) {
                            guard
                                .park_lifecycle
                                .relog_observations
                                .push(observation.clone());
                        }
                        guard.park_lifecycle.after_relog = Some(observation);
                    }
                    match settled {
                        Ok(true) => stage = RunStage::Done,
                        Ok(false) => {}
                        Err(reason) => {
                            cell_failure = Some(reason.into());
                            break;
                        }
                    }
                } else if since.elapsed() > Duration::from_secs(30) {
                    cell_failure = Some("native status disappeared after relog".into());
                    break;
                }
            }
            RunStage::Done => {
                trace.lock().finish_requested = true;
                stage = RunStage::Finish;
            }
            RunStage::Finish if frame_packets.lock().finished() => break,
            RunStage::Finish => {}
        }
        thread::sleep(POLL_INTERVAL);
    }
    play.script_stop(&account);

    if !trace.lock().clock.accepted() {
        cell_failure.get_or_insert_with(|| {
            format!(
                "Engine L PLAYER_INFO clock failed: {:?}",
                trace.lock().clock
            )
        });
    }
    let all_actions = capture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .actions
        .clone();
    let start_action_index = trace.lock().start_action_index.unwrap_or(0);
    let gather_actions = all_actions
        .iter()
        .skip(start_action_index)
        .cloned()
        .collect::<Vec<_>>();
    let start_game_tick = trace.lock().start_game_tick.unwrap_or(0);
    let start_session = trace.lock().start_session.unwrap_or(0);
    let game_frames = trace
        .lock()
        .frames
        .iter()
        .filter(|frame| {
            frame["session_generation"].as_u64() == Some(start_session)
                && frame["snapshot_tick"]
                    .as_u64()
                    .is_some_and(|tick| tick >= u64::from(start_game_tick))
        })
        .cloned()
        .collect::<Vec<_>>();
    let expected_shop_actions = usize::from(case != Case::NoCoins);
    let (buttons, mut button_failures, reconciled, maximum_reply_edges) = button_analysis(
        &gather_actions,
        &game_frames,
        fixture.tool.id,
        expected_shop_actions,
    );
    let (trades, mut trade_failures) =
        trade_analysis(&gather_actions, &fixture, expected_shop_actions);
    let (wear, mut wear_failures) = if case.is_axe() {
        wear_action(&gather_actions, &game_frames, fixture.tool.id)
    } else {
        (None, Vec::new())
    };
    if case.is_axe() && wear.is_none() {
        wear_failures.push("axe purchase did not produce a Wear action".into());
    }
    if !case.is_axe()
        && gather_actions
            .iter()
            .any(|action| action["request"]["op"] == "wear")
    {
        wear_failures.push("the carried Harpoon must not be worn".into());
    }
    if buttons.len() != reconciled {
        button_failures.push(format!(
            "buy buttons sent {} differ from resolved dispatches {reconciled}",
            buttons.len()
        ));
    }
    let (close, mut close_failures, final_closes) = if case == Case::NoCoins {
        (
            json!({"not_applicable": "no shop interaction was expected"}),
            Vec::new(),
            None,
        )
    } else {
        close_analysis(
            &gather_actions,
            &game_frames,
            trace.lock().start_server_closes,
        )
    };
    let mut failures = Vec::new();
    failures.append(&mut button_failures);
    failures.append(&mut trade_failures);
    failures.append(&mut wear_failures);
    failures.append(&mut close_failures);
    failures.extend(over_target_failures(
        &gather_actions,
        &game_frames,
        fixture.tool.id,
    ));
    let (driver_packets, max_driver_packets, active_driver_trace_complete) =
        frame_packets.lock().value();
    let driver_trace_complete = if idle_driver_packet_window(case, transition) {
        frame_packets.lock().trustworthy_idle_window()
    } else {
        active_driver_trace_complete
    };
    if !driver_trace_complete {
        failures.push("Driver packet trace was incomplete".into());
    }
    if max_driver_packets > MAX_DRIVER_PACKETS_PER_TICK {
        failures.push(format!(
            "{max_driver_packets} Driver packets in one game tick; limit is {MAX_DRIVER_PACKETS_PER_TICK}"
        ));
    }
    let bank_open_observed = game_frames
        .iter()
        .any(|frame| frame["bank_open"].as_bool() == Some(true));
    let bank_open_request_observed = gather_actions
        .iter()
        .any(|action| action["request"]["op"] == "open-stand");
    let expected_no_coin_reason = if case == Case::NoCoins {
        trace
            .lock()
            .park_lifecycle
            .initial
            .as_ref()
            .and_then(|park| park.bank_seen.clone())
            .as_deref()
            .and_then(|seen| {
                expected_empty_bank_block(&selected, &fixture.tool, seen, play.map_members())
            })
    } else {
        None
    };
    let (
        lifecycle,
        park_no_walk,
        final_phase,
        final_phase_label,
        final_slot_running,
        final_failure,
        final_block_reason,
        final_bank_open,
        final_bank_origin,
        final_bank_rows,
        final_gather_action,
    ) = {
        let guard = trace.lock();
        (
            guard.park_lifecycle.clone(),
            guard.park_no_walk,
            guard.final_phase.clone(),
            guard.final_phase_label.clone(),
            guard.final_slot_running,
            guard.final_failure.clone(),
            guard.final_block_reason.clone(),
            guard.final_bank_open,
            guard.final_bank_origin.clone(),
            guard.final_bank_rows.clone(),
            guard.gather_action.clone(),
        )
    };
    if case == Case::NoCoins {
        let initial = lifecycle.initial.clone();
        let no_walk = !gather_actions.iter().any(|action| action["kind"] == "walk");
        if !buttons.is_empty()
            || !trades.is_empty()
            || bank_open_observed
            || bank_open_request_observed
            || !no_walk
            || park_no_walk != Some(true)
        {
            failures.push("no-coins lifecycle dispatched a shop action, walk, or bank trip".into());
        }
        if expected_no_coin_reason.is_none()
            || initial
                .as_ref()
                .and_then(|park| park.block_reason.as_deref())
                != expected_no_coin_reason.as_deref()
        {
            failures.push(format!(
                "initial no-coins park did not preserve its exact Session shortage message: initial={initial:?} expected={expected_no_coin_reason:?}"
            ));
        }
        match transition {
            ParkTransition::None => {
                if final_phase.as_deref() != Some("Blocked")
                    || final_phase_label.as_deref() != Some("blocked")
                    || final_slot_running != Some(true)
                    || final_failure.is_some()
                    || final_block_reason.as_deref() != expected_no_coin_reason.as_deref()
                    || final_bank_open != Some(false)
                    || final_bank_origin.as_deref() != Some("Session")
                    || final_bank_rows
                        .as_deref()
                        .is_none_or(|rows| !rows.is_empty())
                    || final_gather_action.is_some()
                {
                    failures.push(
                        "no-coins cell did not retain Running/Blocked/no-failure with a closed Session bank"
                            .into(),
                    );
                }
            }
            ParkTransition::Resume => {
                let supply_was_carried = final_gather_action.as_ref().is_some_and(|action| {
                    held_total(&action["snapshot"], i64::from(fixture.tool.id)) == 1
                });
                if !lifecycle.pause_requested
                    || !lifecycle.pause_observed
                    || lifecycle.supply_command.as_deref() != Some("give harpoon 1")
                    || !lifecycle.supply_observed
                    || !lifecycle.resume_requested
                    || !lifecycle.resume_observed
                    || !supply_was_carried
                {
                    failures.push(format!(
                        "Resume did not consume a real account-only Harpoon arrival and gather: lifecycle={lifecycle:?} supply_was_carried={supply_was_carried}"
                    ));
                }
                if final_gather_action.is_none()
                    || final_phase
                        .as_deref()
                        .is_none_or(|phase| phase == "Blocked")
                    || final_slot_running != Some(true)
                    || final_failure.is_some()
                    || final_bank_open != Some(false)
                    || final_bank_origin.as_deref() != Some("Session")
                    || final_bank_rows
                        .as_deref()
                        .is_none_or(|rows| !rows.is_empty())
                {
                    failures.push(
                        "account-only supply plus Resume did not leave the Gatherer running without a bank trip"
                            .into(),
                    );
                }
            }
            ParkTransition::Relog => {
                let after_relog = lifecycle.after_relog.as_ref();
                let expected_after_relog = after_relog.and_then(|park| {
                    parked_reason_for_bank_seen(
                        expected_no_coin_reason.as_deref()?,
                        park.bank_seen.as_deref()?,
                    )
                });
                if !lifecycle.logout_requested
                    || !lifecycle.logout_observed
                    || !lifecycle.relog_requested
                    || !lifecycle.relog_observed
                    || after_relog.is_none()
                    || expected_after_relog.is_none()
                    || after_relog.is_some_and(|park| {
                        park.phase.as_deref() != Some("Blocked")
                            || park.phase_label.as_deref() != Some("blocked")
                            || park.slot_running != Some(true)
                            || park.failure.is_some()
                            || park.block_reason.as_deref() != expected_after_relog.as_deref()
                            || park.bank_origin.as_deref() != Some("Hint")
                            || park.bank_rows.as_ref().is_none_or(|rows| !rows.is_empty())
                            || park.bank_open != Some(false)
                            || park.tool_in_pack != Some(0)
                            || park.coins_in_pack != Some(0)
                            || park.no_walk != Some(true)
                            || park.no_bank_trip != Some(true)
                    })
                    || initial
                        .as_ref()
                        .zip(after_relog)
                        .is_none_or(|(before, after)| {
                            before.run_slot != after.run_slot
                                || before.run_generation != after.run_generation
                                || before.run_session == after.run_session
                                || before.session_generation == after.session_generation
                        })
                    || final_phase.as_deref() != Some("Blocked")
                    || final_phase_label.as_deref() != Some("blocked")
                    || final_slot_running != Some(true)
                    || final_failure.is_some()
                    || final_block_reason
                        .as_deref()
                        .is_none_or(|reason| !reason.contains("from a saved hint"))
                    || final_bank_open != Some(false)
                    || final_bank_origin.as_deref() != Some("Hint")
                    || final_bank_rows
                        .as_deref()
                        .is_none_or(|rows| !rows.is_empty())
                    || final_gather_action.is_some()
                {
                    failures.push(format!(
                        "relog did not keep the same parked Gatherer without evidence, a bank trip, or a terminal failure: lifecycle={lifecycle:?}"
                    ));
                }
            }
        }
    } else {
        if buttons.len() != 1
            || trades.len() != 1
            || reconciled != 1
            || final_closes.is_none()
            || trace.lock().gather_action.is_none()
        {
            failures.push(format!(
                "positive buy path must have one Trade, one buy, one reconciliation, one close and a real gather: trade={} button={} resolved={reconciled} close={final_closes:?} gather={}",
                trades.len(),
                buttons.len(),
                trace.lock().gather_action.is_some()
            ));
        }
        if !bank_open_observed || !bank_open_request_observed {
            failures.push(
                "positive purchase did not open the bank to use the account-only coin seed".into(),
            );
        }
        let final_rows = trace.lock().final_bank_rows.clone().unwrap_or_default();
        let final_coin_balance = final_rows
            .iter()
            .copied()
            .find(|(id, _)| *id == fixture.coins.id)
            .map(|(_, count)| count);
        if final_coin_balance.is_none_or(|count| count >= case.coin_seed()) {
            failures.push(format!(
                "bank memory did not show a coin withdrawal: start={} final={final_coin_balance:?}",
                case.coin_seed()
            ));
        }
        let trade_sequence = trades.first().and_then(|row| row["sequence"].as_u64());
        let buy_sequence = buttons.first().and_then(|row| row["sequence"].as_u64());
        let close_sequence = close["resolutions"]
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["close_sequence"].as_u64());
        let gather_sequence = trace
            .lock()
            .gather_action
            .as_ref()
            .and_then(|row| row["sequence"].as_u64());
        let wear_sequence = wear.as_ref().and_then(|row| row["sequence"].as_u64());
        let ordered = match (
            trade_sequence,
            buy_sequence,
            close_sequence,
            wear_sequence,
            gather_sequence,
        ) {
            (Some(trade), Some(buy), Some(close), Some(wear), Some(gather)) if case.is_axe() => {
                trade < buy && buy < close && close < wear && wear < gather
            }
            (Some(trade), Some(buy), Some(close), _, Some(gather)) if !case.is_axe() => {
                trade < buy && buy < close && close < gather
            }
            _ => false,
        };
        if !ordered {
            failures.push(format!(
                "purchase sequence must be Trade < buy < shop close < {} < real gather dispatch: trade={trade_sequence:?} buy={buy_sequence:?} close={close_sequence:?} wear={wear_sequence:?} gather={gather_sequence:?}",
                if case.is_axe() { "Wear" } else { "resource" }
            ));
        }
    }
    let mut content_rows = Vec::new();
    for frame in &game_frames {
        let xp = frame["stats"].as_array().into_iter().flatten().filter(|stat| {
            let wanted = if case.is_axe() { "Woodcutting" } else { "Fishing" };
            stat["name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        }).map(|stat| json!({"tick": frame["snapshot_tick"], "name": stat["name"], "base": stat["base"], "xp": stat["xp"]})).collect::<Vec<_>>();
        if !xp.is_empty() {
            content_rows.extend(xp);
        }
    }
    let capture_guard = capture.lock().unwrap_or_else(|error| error.into_inner());
    let action_capture = json!({
        "frames": capture_guard.frames,
        "statuses": capture_guard.statuses,
        "actions": capture_guard.actions,
        "observations": capture_guard.observations,
        "start_baseline": capture_guard.start_baseline,
        "invalid_reason": capture_guard.invalid_reason,
    });
    if let Some(reason) = action_capture["invalid_reason"].as_str() {
        failures.push(format!("action capture invalid: {reason}"));
    }
    let mut receipt = json!({
        "scenario": scenario_name,
        "case": case.name(),
        "account": account,
        "engine": {"host": "127.0.0.1", "game_port": ENGINE_L_GAME_PORT, "http_port": ENGINE_L_HTTP_PORT, "tick_ms": 600},
        "selected_revision": "R289",
        "reference_nav_pack": std::env::var("GATHERER_NAV_PACK").ok(),
        "reference_catalog_root": std::env::var("GATHERER_CATALOG_ROOT").ok(),
        "settings": settings(case),
        "buyTools": {"provided": false, "default_under_test": true},
        "fixture": {
            "start_tile": case.start(),
            "radius": 12,
            "real_resource": case.gather_object(),
            "tool": fixture.tool,
            "shop_keeper": fixture.keeper,
            "bank_coins_seeded": case.coin_seed(),
            "shop_item": {
                "keeper_config": case.keeper(),
                "baseline": fixture.baseline,
                "item_cost": fixture.cost,
                "sell_multiplier": fixture.sell,
                "delta": fixture.delta,
                "quote_at_baseline": fixture.quote,
                "two_chunk_reserve": fixture.reserve,
            },
        },
        "bank_prime": trace.lock().prime,
        "start": {
            "tile": trace.lock().start_tile,
            "bank_origin": trace.lock().start_bank_origin,
            "bank_rows": trace.lock().start_bank_rows,
            "inventory": trace.lock().start_inventory,
            "equipment": trace.lock().start_equipment,
            "player_info_generation": trace.lock().start_player_info,
            "server_closes": trace.lock().start_server_closes,
        },
        "final_native_status": final_status.as_ref().map(|status| status_value(status.as_ref())),
        "park": {
            "lifecycle": trace.lock().park_lifecycle,
            "phase": trace.lock().final_phase,
            "label": trace.lock().final_phase_label,
            "slot_state": trace.lock().final_slot_state,
            "slot_running": trace.lock().final_slot_running,
            "failure": trace.lock().final_failure,
            "block_reason": trace.lock().final_block_reason,
            "expected_no_coin_message": expected_no_coin_reason,
            "bank_open": trace.lock().final_bank_open,
            "bank_origin": trace.lock().final_bank_origin,
            "bank_rows": trace.lock().final_bank_rows,
            "no_walk": trace.lock().park_no_walk,
        },
        "buy_dispatches": buttons,
        "buy_reconciliations": reconciled,
        "maximum_reply_edges": maximum_reply_edges,
        "reply_tick_cap": script::acquire::buy::BUY_REPLY_TICKS,
        "trades": trades,
        "wear": wear,
        "close_reconciliation": close,
        "final_shop_server_closes": final_closes,
        "gather_action": trace.lock().gather_action,
        "gather_content_window_end_tick": trace.lock().content_window_end_tick,
        "naturally_observed_skill_xp": content_rows,
        "player_info_clock": trace.lock().clock,
        "driver_packets_by_game_tick": driver_packets,
        "maximum_driver_packets_per_game_tick": max_driver_packets,
        "driver_packet_trace_complete": driver_trace_complete,
        "raw_frame_observations": trace.lock().frames,
        "action_capture": action_capture,
        "failure": cell_failure,
        "oracle_failures": failures,
    });
    let passed = cell_failure.is_none() && failures.is_empty();
    receipt["passed"] = json!(passed);
    std::fs::write(
        evidence.join("actions.json"),
        serde_json::to_vec_pretty(&gather_actions).expect("serialize actions"),
    )
    .expect("write raw Gatherer action capture");
    std::fs::write(
        evidence.join("frames.json"),
        serde_json::to_vec_pretty(&game_frames).expect("serialize frames"),
    )
    .expect("write observed game frames");
    std::fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
    )
    .expect("write structured live receipt");
    drop(capture_guard);
    drop(play);
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(
        passed,
        "{} failed; cell_failure={cell_failure:?}; oracle_failures={failures:?}; evidence={}",
        scenario_name,
        evidence.display()
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStage {
    Prepare,
    Prime,
    Travel,
    Gather,
    ParkResumePause { since: Instant },
    ParkResumeSupply { since: Instant },
    ParkResumeObserve { since: Instant },
    ParkRelogTraceFinish { since: Instant },
    ParkRelogLogout { since: Instant },
    ParkRelogLogin { since: Instant },
    ParkRelogObserve { since: Instant },
    Done,
    Finish,
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, Engine L on ports 47594/3080, gsh account prefix, R289 pack/catalog and copied cache"]
fn gatherer_tool_buy_harpoon() {
    run_cell(Case::Harpoon);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, Engine L on ports 47594/3080, gsh account prefix, R289 pack/catalog and copied cache"]
fn gatherer_tool_buy_no_coins() {
    run_cell(Case::NoCoins);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, Engine L on ports 47594/3080, gsh account prefix, R289 pack/catalog and copied cache"]
fn gatherer_tool_buy_axe_wear() {
    run_cell(Case::AxeWear);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, Engine L on ports 47594/3080, gsh account prefix, R289 pack/catalog and copied cache"]
fn live_gatherer_park_then_resume() {
    run_cell_with_transition(Case::NoCoins, ParkTransition::Resume);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, Engine L on ports 47594/3080, gsh account prefix, R289 pack/catalog and copied cache"]
fn live_gatherer_park_relog() {
    run_cell_with_transition(Case::NoCoins, ParkTransition::Relog);
}

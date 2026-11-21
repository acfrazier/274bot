//! PROBE-BUY-REPLY on Engine A. This ignored live cell measures native
//! interaction dispatches against Gerrant's real R289 shop. It does not use
//! a script timer or a local modal clear as a reply.
//!
//! Five fresh, owned accounts run independently fenced batches: three 100-buy
//! batches, one 100-refusal batch, and one 100-close batch. Their exact replies
//! aggregate to the unchanged 300/100/100 acceptance oracle.
//!
//! Main registers this module in `lib.rs` under `cfg(all(test,
//! feature="live-harness", feature="test-support"))`. Run the ignored test
//! through the evidence driver in `GATHERER-SHOP/buy-reply/run-probe.sh`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::Poll;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, SelectedGameData};
use api::interact::Driver;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey};
use api::service_pick::ShopKeeperFacts;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host::{FrameBuf, Pump};
use parking_lot::Mutex;
use scenario::{self, Proof, RunnerStatus, Scenario, ScenarioRunner, Step, StepKind, Wait};
use script::interact::InteractReq;
use script::native::walk::Walk;
use script::native::{
    ActionContext, ActionError, ActionHandle, NativeMachine, NativeTick, Script, ScriptFailure,
    ScriptFlow, WalkEnd,
};
use script::quester::families::reach::walk_request;
use script::shop_open::{self, Decision, Facts, Keeper, OpenCore, Session};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

use super::combat_proof::{
    self, CaptureRegistration, CombatCapture, DriverPacketCounts, DriverPacketTrace,
    MAX_DRIVER_PACKETS_PER_TICK,
};
use super::gather_bait_live::PlayerInfoClock;
use super::{run_with_template, ProfileOptions, ScriptStartHandle, SharedClientTemplate};

const KEEPER_CONFIG: &str = "gerrant";
const ITEM_ALIAS: &str = "feather";
const COINS_ALIAS: &str = "coins";
const SEED_COINS: i32 = 500_000;
const HELD_SAMPLES: usize = 300;
const REFUSAL_SAMPLES: usize = 100;
const CLOSE_SAMPLES: usize = 100;
/// This only bounds a missing-reply defect. It is not a latency expectation
/// or the BUY_REPLY_TICKS value. All sample timing uses observed evidence ticks.
const SAMPLE_REPLY_LIMIT_TICKS: u64 = 32;
/// Ruling 3 of design 4.8 observes two later edges before settling a refusal.
const REFUSAL_PACK_OBSERVE_TICKS: u64 = 2;
const PREPARATION_DEADLINE: Duration = Duration::from_secs(600);
const CELL_DEADLINE: Duration = Duration::from_secs(3_600);
const RUNNER_COMPLETE_DEADLINE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_CAPTURE_ROWS: usize = 100_000;
const SEED_ITEMS: &[(&str, i32)] = &[(COINS_ALIAS, SEED_COINS)];
const HELD_BATCH_SAMPLES: usize = 100;
/// Engine macro events start 500 game ticks after login (`login_logout/login.rs2:39-48`).
const MAX_BATCH_LOGIN_TICKS: u64 = 500;
/// Setup, dispatches and settlement all count against the login-relative limit.
const BATCH_SCHEDULE: [BatchPlan; 5] = [
    BatchPlan {
        series: Series::Held,
        first_ordinal: 1,
        samples: HELD_BATCH_SAMPLES,
    },
    BatchPlan {
        series: Series::Held,
        first_ordinal: HELD_BATCH_SAMPLES + 1,
        samples: HELD_BATCH_SAMPLES,
    },
    BatchPlan {
        series: Series::Held,
        first_ordinal: HELD_BATCH_SAMPLES * 2 + 1,
        samples: HELD_BATCH_SAMPLES,
    },
    BatchPlan {
        series: Series::Refusal,
        first_ordinal: 1,
        samples: REFUSAL_SAMPLES,
    },
    BatchPlan {
        series: Series::Close,
        first_ordinal: 1,
        samples: CLOSE_SAMPLES,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct BatchPlan {
    series: Series,
    first_ordinal: usize,
    samples: usize,
}

impl BatchPlan {
    fn ordinal(self, next: usize) -> usize {
        self.first_ordinal + next
    }
}

#[derive(Clone)]
struct ItemFact {
    id: i32,
    name: Arc<str>,
}

#[derive(Clone)]
struct KeeperFact {
    config: Arc<str>,
    name: Arc<str>,
    id: i32,
    tile: WorldTile,
    access: Vec<WorldTile>,
}

#[derive(Clone)]
struct Fixture {
    feather: ItemFact,
    coins: ItemFact,
    keeper: KeeperFact,
    stand: WorldTile,
    baseline_stock: i32,
    baseline_unit_price: i64,
}

#[derive(Clone, Copy, Default)]
struct SampleBaseline {
    held: Option<i32>,
    coins: Option<i32>,
    stock: Option<i32>,
    item_slot: Option<i32>,
    item_component: Option<i32>,
    server_closes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Series {
    Held,
    Refusal,
    Close,
}

impl Series {
    fn expected(self) -> usize {
        match self {
            Self::Held => HELD_SAMPLES,
            Self::Refusal => REFUSAL_SAMPLES,
            Self::Close => CLOSE_SAMPLES,
        }
    }

    fn request_kind(self) -> &'static str {
        match self {
            Self::Held | Self::Refusal => "shop-button-buy-1",
            Self::Close => "close-modal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum SampleStatus {
    Pending,
    Replied,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct SessionIdentity {
    slot: u64,
    run: u64,
    session: u64,
}

impl From<RunKey> for SessionIdentity {
    fn from(run: RunKey) -> Self {
        Self {
            slot: run.slot,
            run: run.run,
            session: run.session,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct Sample {
    batch_index: usize,
    series: Series,
    ordinal: usize,
    account: String,
    session: SessionIdentity,
    request_kind: &'static str,
    request_id: u64,
    request_tick: u64,
    dispatch_tick: Option<u64>,
    captured_dispatch_tick: Option<u64>,
    captured_snapshot_tick: Option<u64>,
    dispatch_accepted: Option<bool>,
    dispatch_wire_decoded: Option<bool>,
    dispatch_wire_opcodes: Vec<u8>,
    item_id: Option<i32>,
    item_slot: Option<i32>,
    item_component: Option<i32>,
    coins_before: Option<i32>,
    held_before: Option<i32>,
    held_after: Option<i32>,
    stock_before: Option<i32>,
    stock_after: Option<i32>,
    chat_watermark: Option<i32>,
    refusal_sequence: Option<i32>,
    refusal_text: Option<String>,
    refusal_tick: Option<u64>,
    refusal_pack_update_tick: Option<u64>,
    refusal_pack_update_edges: Option<u64>,
    refusal_pack_observed_through_tick: Option<u64>,
    server_closes_before: Option<u64>,
    server_closes_after: Option<u64>,
    server_close_tick: Option<u64>,
    reply_tick: Option<u64>,
    reply_edges: Option<u64>,
    status: SampleStatus,
    error: Option<String>,
}

impl Sample {
    fn new(
        batch_index: usize,
        series: Series,
        ordinal: usize,
        account: &str,
        session: SessionIdentity,
        request_id: u64,
        request_tick: u64,
    ) -> Self {
        Self {
            batch_index,
            series,
            ordinal,
            account: account.to_owned(),
            session,
            request_kind: series.request_kind(),
            request_id,
            request_tick,
            dispatch_tick: None,
            captured_dispatch_tick: None,
            captured_snapshot_tick: None,
            dispatch_accepted: None,
            dispatch_wire_decoded: None,
            dispatch_wire_opcodes: Vec::new(),
            item_id: None,
            item_slot: None,
            item_component: None,
            coins_before: None,
            held_before: None,
            held_after: None,
            stock_before: None,
            stock_after: None,
            chat_watermark: None,
            refusal_sequence: None,
            refusal_text: None,
            refusal_tick: None,
            refusal_pack_update_tick: None,
            refusal_pack_update_edges: None,
            refusal_pack_observed_through_tick: None,
            server_closes_before: None,
            server_closes_after: None,
            server_close_tick: None,
            reply_tick: None,
            reply_edges: None,
            status: SampleStatus::Pending,
            error: None,
        }
    }

    fn mark_error(&mut self, reason: impl Into<String>) {
        self.status = SampleStatus::Error;
        self.error = Some(reason.into());
    }

    fn mark_reply(&mut self, tick: u64) -> Result<(), String> {
        let dispatch = self
            .dispatch_tick
            .ok_or_else(|| "reply observed before the host dispatch receipt".to_owned())?;
        let edges = tick
            .checked_sub(dispatch)
            .ok_or_else(|| format!("reply tick {tick} precedes dispatch tick {dispatch}"))?;
        self.reply_tick = Some(tick);
        self.reply_edges = Some(edges);
        self.status = SampleStatus::Replied;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
struct LastReply {
    batch_index: usize,
    series: Series,
    ordinal: usize,
    tick: u64,
    session: SessionIdentity,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BatchAdmission {
    plan: BatchPlan,
    account: String,
    session: SessionIdentity,
    login_player_info_ticks: u64,
}

fn validate_batch_admission(samples: &[Sample], batches: &[BatchAdmission]) -> Result<(), String> {
    if batches.len() != BATCH_SCHEDULE.len() {
        return Err(format!(
            "received {} login batches, expected {}",
            batches.len(),
            BATCH_SCHEDULE.len()
        ));
    }
    let mut accounts = std::collections::BTreeSet::new();
    for (batch_index, (batch, expected)) in batches.iter().zip(BATCH_SCHEDULE).enumerate() {
        if batch.plan != expected {
            return Err(format!(
                "batch {batch_index} does not match the required schedule"
            ));
        }
        if !accounts.insert(batch.account.as_str()) {
            return Err(format!(
                "batch {batch_index} reused account {}",
                batch.account
            ));
        }
        if batch.login_player_info_ticks > MAX_BATCH_LOGIN_TICKS {
            return Err(format!(
                "batch {batch_index} ended at login-relative tick {}, above {MAX_BATCH_LOGIN_TICKS}",
                batch.login_player_info_ticks
            ));
        }
        let rows = samples
            .iter()
            .filter(|sample| sample.batch_index == batch_index)
            .collect::<Vec<_>>();
        if rows.len() != expected.samples {
            return Err(format!(
                "batch {batch_index} has {} samples, expected {}",
                rows.len(),
                expected.samples
            ));
        }
        for ordinal in expected.first_ordinal..expected.first_ordinal + expected.samples {
            let matches = rows
                .iter()
                .filter(|sample| sample.ordinal == ordinal)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(format!(
                    "batch {batch_index} has {} samples for {:?} ordinal {ordinal}, expected exactly one",
                    matches.len(),
                    expected.series
                ));
            }
            let sample = matches[0];
            if sample.series != expected.series {
                return Err(format!(
                    "batch {batch_index} ordinal {ordinal} belongs to {:?}, expected {:?}",
                    sample.series, expected.series
                ));
            }
            if sample.account != batch.account || sample.session != batch.session {
                return Err(format!(
                    "batch {batch_index} ordinal {ordinal} crossed its account or run/session fence"
                ));
            }
            if sample.status != SampleStatus::Replied {
                return Err(format!(
                    "batch {batch_index} ordinal {ordinal} did not receive its exact reply"
                ));
            }
        }
    }
    if samples
        .iter()
        .any(|sample| sample.batch_index >= batches.len())
    {
        return Err("a sample names no admitted login batch".into());
    }
    Ok(())
}
// Tile movement is diagnostic. Client watermarks identify scene invalidations
// and applied local-player Teleport events.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneIdentity {
    map_base_x: i32,
    map_base_z: i32,
    level: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneObservation {
    identity: SceneIdentity,
    player_tile: WorldTile,
    evidence_tick: u64,
    local_player_teleport_watermark: u64,
    rebuild_invalidation_watermark: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneFence {
    identity: SceneIdentity,
    baseline_player_tile: WorldTile,
    last_player_tile: WorldTile,
    baseline_evidence_tick: u64,
    local_player_teleport_watermark: u64,
    rebuild_invalidation_watermark: u64,
}

impl SceneFence {
    fn new(observation: SceneObservation) -> Self {
        Self {
            identity: observation.identity,
            baseline_player_tile: observation.player_tile,
            last_player_tile: observation.player_tile,
            baseline_evidence_tick: observation.evidence_tick,
            local_player_teleport_watermark: observation.local_player_teleport_watermark,
            rebuild_invalidation_watermark: observation.rebuild_invalidation_watermark,
        }
    }
}

fn scene_observation(cx: &ActionContext<'_>) -> Option<SceneObservation> {
    let view = cx.snapshot();
    let world = view.world()?;
    let player = view.local_player()?;
    let local_player_teleport_watermark = view.local_player_teleport_watermark()?.value;
    let rebuild_invalidation_watermark = view.rebuild_invalidation_watermark()?.value;
    let mut player_tile = player.value.player.network;
    player_tile.level = world.value.level;
    Some(SceneObservation {
        identity: SceneIdentity {
            map_base_x: world.value.map_base_x,
            map_base_z: world.value.map_base_z,
            level: world.value.level,
        },
        player_tile,
        evidence_tick: cx.evidence().tick,
        local_player_teleport_watermark,
        rebuild_invalidation_watermark,
    })
}

fn record_scene_baseline(
    trace: &mut ProbeTrace,
    observation: Option<SceneObservation>,
) -> Result<(), String> {
    if trace.scene_fence.is_some() {
        return Ok(());
    }
    let observation = observation.ok_or_else(|| {
        "player scene observation unavailable when Gerrant's shop opened".to_owned()
    })?;
    trace.scene_fence = Some(SceneFence::new(observation));
    Ok(())
}

fn validate_context_observation(
    trace: &mut ProbeTrace,
    session: SessionIdentity,
    observation: Option<SceneObservation>,
    active_sample: Option<usize>,
) -> Option<String> {
    let error = match trace.session {
        Some(expected) if expected != session => Some(format!(
            "session changed during probe: expected {expected:?}, observed {session:?}"
        )),
        None => {
            trace.session = Some(session);
            None
        }
        Some(_) => None,
    }
    .or_else(|| {
        let fence = trace.scene_fence.as_mut()?;
        let Some(observation) = observation else {
            return Some("player scene observation unavailable while probe is active".into());
        };
        if fence.rebuild_invalidation_watermark != observation.rebuild_invalidation_watermark {
            return Some(format!(
                "client rebuild invalidation changed during probe: expected watermark {}, observed {}",
                fence.rebuild_invalidation_watermark, observation.rebuild_invalidation_watermark
            ));
        }
        if fence.identity != observation.identity {
            return Some(format!(
                "player scene changed: expected {:?}, observed {:?}",
                fence.identity, observation.identity
            ));
        }
        if fence.local_player_teleport_watermark
            != observation.local_player_teleport_watermark
        {
            return Some(format!(
                "local player Teleport applied after shop admission: expected watermark {}, observed {} (baseline tile {:?}, observed tile {:?})",
                fence.local_player_teleport_watermark,
                observation.local_player_teleport_watermark,
                fence.baseline_player_tile,
                observation.player_tile
            ));
        }
        fence.last_player_tile = observation.player_tile;
        None
    });
    if let Some(reason) = error.as_ref() {
        if let Some(row) = active_sample.and_then(|index| trace.samples.get_mut(index)) {
            row.mark_error(reason.clone());
        }
        trace.fail(reason.clone());
    }
    error
}
/// The script-start key can precede host session alignment. Establish the
/// measurement session from the first native context; admit a scene only
/// after the Trade opens a usable shop page.
fn record_script_start(trace: &mut ProbeTrace, tick: u64, run: RunKey) {
    trace.started = true;
    trace.start_tick = Some(tick);
    trace.script_run = Some(format!("{run:?}"));
}

#[derive(Default)]
struct ProbeTrace {
    started: bool,
    start_tick: Option<u64>,
    session: Option<SessionIdentity>,
    script_run: Option<String>,
    phase: Option<&'static str>,
    scene_fence: Option<SceneFence>,
    login_player_info_ticks: Option<u64>,
    settled_replies: usize,
    last_reply: Option<LastReply>,
    terminal_context: Option<Value>,
    complete_tick: Option<u64>,
    completed: bool,
    failure: Option<String>,
    samples: Vec<Sample>,
    observations: Vec<Value>,
    player_info_clock: PlayerInfoClock,
    last_observation_signature: Option<String>,
}

impl ProbeTrace {
    fn fail(&mut self, reason: impl Into<String>) {
        if self.failure.is_none() {
            self.failure = Some(reason.into());
        }
    }
}

fn item_fact(data: &SelectedGameData, alias: &str) -> Result<ItemFact, String> {
    let row = data
        .item_by_alias(alias)
        .ok_or_else(|| format!("selected R289 content has no item alias {alias:?}"))?;
    Ok(ItemFact {
        id: row.id,
        name: Arc::from(
            row.name
                .as_deref()
                .ok_or_else(|| format!("selected R289 item alias {alias:?} has no name"))?,
        ),
    })
}

fn content_keeper(keepers: &ShopKeeperFacts, config: &str) -> Result<KeeperFact, String> {
    let row = keepers
        .rows()
        .iter()
        .find(|row| row.keeper.keeper_config == config && !row.access().is_empty())
        .ok_or_else(|| format!("selected R289 world has no accessible {config} placement"))?;
    Ok(KeeperFact {
        config: Arc::from(row.keeper.keeper_config),
        name: Arc::from(row.keeper.keeper_name),
        id: row.keeper.keeper_id,
        tile: row.tile,
        access: row.access().to_vec(),
    })
}

fn fixture(data: &SelectedGameData, keepers: &ShopKeeperFacts) -> Result<Fixture, String> {
    let feather = item_fact(data, ITEM_ALIAS)?;
    let coins = item_fact(data, COINS_ALIAS)?;
    let keeper = content_keeper(keepers, KEEPER_CONFIG)?;
    let vendor = api::shop_facts::vendor_by_keeper_config(data, KEEPER_CONFIG, feather.id)
        .map_err(|error| format!("shop facts: {error:?}"))?
        .ok_or_else(|| {
            "pinned Gerrant shop facts do not sell the selected feather item".to_owned()
        })?;
    if vendor.keeper_id != keeper.id || vendor.keeper_name != keeper.name.as_ref() {
        return Err(
            "selected Gerrant placement disagrees with the pinned shop keeper identity".into(),
        );
    }
    if vendor.record.inv != "fishingshop" {
        return Err(format!(
            "Gerrant's selected feather vendor is {}, expected fishingshop",
            vendor.record.inv
        ));
    }
    let baseline_unit_price =
        api::shop_facts::quote(vendor.record, vendor.item, 1, vendor.item.baseline);
    if baseline_unit_price <= 0 {
        return Err(format!(
            "Gerrant's feather baseline price must be positive for a deterministic 0-gp refusal, got {baseline_unit_price}"
        ));
    }
    let stand = *keeper
        .access
        .first()
        .ok_or_else(|| "Gerrant has no selected navigation access tile".to_owned())?;
    Ok(Fixture {
        feather,
        coins,
        keeper,
        stand,
        baseline_stock: vendor.item.baseline,
        baseline_unit_price,
    })
}

fn profile_options(scratch: &Path) -> Result<ProfileOptions, String> {
    let engine_dir = std::env::var_os("GATHERER_ENGINE_DIR")
        .or_else(|| std::env::var_os("BOT_ENGINE_DIR"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            "buy reply probe requires GATHERER_ENGINE_DIR or BOT_ENGINE_DIR".to_owned()
        })?;
    let cache_dir = std::env::var_os("BOT_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .ok_or_else(|| "buy reply probe requires a per-run BOT_CACHE_DIR clone".to_owned())?;
    let nav_pack = std::env::var_os("GATHERER_NAV_PACK")
        .map(PathBuf::from)
        .ok_or_else(|| "buy reply probe requires GATHERER_NAV_PACK".to_owned())?;
    let nav_flags = std::env::var_os("GATHERER_NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| nav_pack.with_extension("navflags"));
    let game_port = crate::live_ports::required_port("WORLD_GAME_PORT")?;
    let http_port = crate::live_ports::required_port("WORLD_HTTP_PORT")?;
    for (name, path) in [
        ("GATHERER_NAV_PACK", &nav_pack),
        ("GATHERER_NAV_FLAGS", &nav_flags),
    ] {
        if !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
    }
    Ok(ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(game_port),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(http_port),
        engine_dir: Some(engine_dir),
        cache_dir: Some(cache_dir),
        unpack_dir: Some(scratch.join("unpack")),
        nav_pack: Some(nav_pack),
        nav_flags: Some(nav_flags),
        vault_path: Some(scratch.join("vault")),
        ..ProfileOptions::default()
    })
}
fn probe_scenario(stand: WorldTile, trace: Arc<Mutex<ProbeTrace>>) -> Scenario {
    let mut scenario = scenario::quester_stage(
        "buy_reply_probe_engine_a",
        "Cook's Assistant",
        "cook",
        0,
        SEED_ITEMS,
        stand,
    );
    let reset = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .expect("quester-stage preparation has a removable quest-stage seed");
    scenario.steps.remove(reset);
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("account preparation has StartScript");
    scenario.steps.truncate(start + 1);
    scenario.steps.push(Step {
        name: "await completed buy reply probe",
        kind: StepKind::Await {
            evidence: "all three sampled reply series completed",
            ready: Box::new(move |_| trace.lock().completed),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 0, min: 1 },
            budget_ticks: u32::MAX,
        },
    });
    scenario.proof = Proof::Stat { id: 0, min: 1 };
    scenario.settings.deadline = PREPARATION_DEADLINE + CELL_DEADLINE;
    scenario.settings.nav.engine_speed_ms = None;
    scenario
}

#[derive(Clone, Copy)]
enum Phase {
    OpenShop,
    Held {
        next: usize,
    },
    AwaitHeld {
        sample: usize,
    },
    DropCoins,
    AwaitCoinDrop {
        request_id: u64,
        request_tick: u64,
        dispatch_tick: Option<u64>,
        coins_before: i32,
    },
    Refusal {
        next: usize,
    },
    AwaitRefusal {
        sample: usize,
    },
    RefusalObserve {
        sample: usize,
        line_tick: u64,
        held_before: i32,
    },
    Close {
        next: usize,
    },
    AwaitClose {
        sample: usize,
    },
    Reopen {
        next: usize,
        close_tick: u64,
        closes_after: u64,
    },
    Done,
}
struct PendingOpenTrade {
    keeper: Keeper,
    request_id: u64,
    request_tick: u64,
}

struct BuyReplyProbeScript {
    account: String,
    fixture: Fixture,
    trace: Arc<Mutex<ProbeTrace>>,
    batch_index: usize,
    batch: BatchPlan,
    phase: Phase,
    last_emit_tick: Option<u64>,
    open: OpenCore,
    open_walk: Option<Walk>,
    open_trade: Option<PendingOpenTrade>,
    open_seen_trade: bool,
    keeper_index: Option<i32>,
    reopening_after_coin_drop: bool,
}

fn keeper_candidate<'a>(
    npc: &'a api::snapshot::NpcView,
    fallback_name: &'a str,
    here: Option<WorldTile>,
) -> shop_open::Candidate<'a> {
    shop_open::Candidate::new(
        npc.name.as_deref().unwrap_or(fallback_name),
        npc.actions
            .iter()
            .flatten()
            .find(|action| action.eq_ignore_ascii_case("Trade"))
            .map(String::as_str),
        i32::try_from(npc.index).unwrap_or(-1),
        npc.network,
        npc.size,
        here.map(|here| api::query::chebyshev_to(npc.network, here))
            .unwrap_or(i32::MAX),
    )
}
impl BuyReplyProbeScript {
    fn new(account: String, fixture: Fixture, trace: Arc<Mutex<ProbeTrace>>) -> Self {
        Self::for_batch(account, fixture, trace, 0, BATCH_SCHEDULE[0])
    }

    fn for_batch(
        account: String,
        fixture: Fixture,
        trace: Arc<Mutex<ProbeTrace>>,
        batch_index: usize,
        batch: BatchPlan,
    ) -> Self {
        Self {
            account,
            fixture,
            trace,
            batch_index,
            batch,
            phase: Phase::OpenShop,
            last_emit_tick: None,
            open: OpenCore::default(),
            open_walk: None,
            open_trade: None,
            open_seen_trade: false,
            keeper_index: None,
            reopening_after_coin_drop: false,
        }
    }

    fn emit(
        &mut self,
        cx: &mut ActionContext<'_>,
        tick: u64,
        request: InteractReq,
    ) -> Result<Option<u64>, String> {
        if self.last_emit_tick == Some(tick) {
            return Ok(None);
        }
        self.last_emit_tick = Some(tick);
        match cx.emit(request) {
            Ok(request_id) => Ok(Some(request_id)),
            Err(ActionError::BudgetExhausted) => Ok(None),
            Err(error) => Err(format!("native interaction admission failed: {error:?}")),
        }
    }

    fn add_sample(
        &self,
        series: Series,
        ordinal: usize,
        request_id: u64,
        request_tick: u64,
        session: SessionIdentity,
    ) -> usize {
        let mut trace = self.trace.lock();
        trace.samples.push(Sample::new(
            self.batch_index,
            series,
            ordinal,
            &self.account,
            session,
            request_id,
            request_tick,
        ));
        trace.samples.len() - 1
    }

    fn sample(&self, index: usize) -> Sample {
        self.trace.lock().samples[index].clone()
    }

    fn mark_sample_error(&self, index: usize, reason: impl Into<String>) {
        self.trace.lock().samples[index].mark_error(reason);
    }

    fn active_sample(&self) -> Option<usize> {
        match self.phase {
            Phase::AwaitHeld { sample }
            | Phase::AwaitRefusal { sample }
            | Phase::RefusalObserve { sample, .. }
            | Phase::AwaitClose { sample } => Some(sample),
            _ => None,
        }
    }

    fn settle_reply(&self, index: usize, tick: u64) -> Result<(), String> {
        let mut trace = self.trace.lock();
        let last_reply = {
            let sample = trace
                .samples
                .get_mut(index)
                .ok_or_else(|| format!("reply names missing sample {index}"))?;
            if sample.status != SampleStatus::Pending {
                return Err(format!(
                    "reply cannot settle sample {index} in state {:?}",
                    sample.status
                ));
            }
            sample.mark_reply(tick)?;
            LastReply {
                batch_index: sample.batch_index,
                series: sample.series,
                ordinal: sample.ordinal,
                tick,
                session: sample.session,
            }
        };
        trace.settled_replies = trace
            .settled_replies
            .checked_add(1)
            .expect("settled reply count is bounded by admitted samples");
        trace.last_reply = Some(last_reply);
        Ok(())
    }
    fn complete_batch(&mut self, tick: u64) -> ScriptFlow {
        let mut trace = self.trace.lock();
        trace.completed = true;
        trace.complete_tick = Some(tick);
        drop(trace);
        self.phase = Phase::Done;
        ScriptFlow::Complete
    }

    fn validate_context(&self, cx: &ActionContext<'_>) -> Option<String> {
        let session = SessionIdentity::from(cx.run());
        let needs_scene = self.trace.lock().scene_fence.is_some();
        let observation = needs_scene.then(|| scene_observation(cx)).flatten();
        let active_sample = self.active_sample();
        let mut trace = self.trace.lock();
        validate_context_observation(&mut trace, session, observation, active_sample)
    }

    fn fail(&self, reason: impl Into<String>, sample: Option<usize>) -> ScriptFlow {
        let reason = reason.into();
        if let Some(index) = sample {
            self.mark_sample_error(index, reason.clone());
        }
        self.trace.lock().fail(reason.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "buy-reply-probe".into(),
            message: reason.into(),
        })
    }

    fn remember_receipt(
        &self,
        cx: &ActionContext<'_>,
        sample_index: usize,
    ) -> Result<Option<u64>, String> {
        let current = self.sample(sample_index);
        if let Some(tick) = current.dispatch_tick {
            return Ok(Some(tick));
        }
        let Some(receipt) = cx.interaction_receipt(current.request_id).copied() else {
            return Ok(None);
        };
        let mut trace = self.trace.lock();
        let sample = &mut trace.samples[sample_index];
        sample.dispatch_tick = Some(receipt.evidence.tick);
        sample.chat_watermark = Some(receipt.chat_since);
        sample.dispatch_accepted = Some(receipt.accepted);
        if !receipt.accepted {
            sample.mark_error("host refused the interaction before a game packet was sent");
            return Err(sample
                .error
                .clone()
                .unwrap_or_else(|| "host refused interaction".into()));
        }
        Ok(Some(receipt.evidence.tick))
    }

    fn record_observation(&self, cx: &ActionContext<'_>, phase: &'static str) {
        let snapshot = cx.snapshot();
        let tick = cx.evidence().tick;
        let held = held_count(cx, self.fixture.feather.id);
        let coins = held_count(cx, self.fixture.coins.id);
        let shop = snapshot.shop();
        let shop_open = shop.map(|row| row.value.open);
        let server_closes = shop.map(|row| row.value.server_closes);
        let feather_stock = shop.and_then(|row| {
            row.value
                .stock
                .iter()
                .find(|item| item.def.id == self.fixture.feather.id)
                .map(|item| item.count)
        });
        let latest_chat = snapshot.chat_lines(i32::MIN).and_then(|lines| {
            lines
                .value
                .iter()
                .max_by_key(|line| line.sequence)
                .map(|line| (line.sequence, line.text.clone()))
        });
        let row = json!({
            "tick": tick,
            "phase": phase,
            "held_feather": held,
            "held_coins": coins,
            "shop_open": shop_open,
            "shop_server_closes": server_closes,
            "feather_stock": feather_stock,
            "latest_chat": latest_chat.map(|(sequence, text)| json!({"sequence": sequence, "text": text})),
        });
        let signature = serde_json::to_string(&row).unwrap_or_default();
        let mut trace = self.trace.lock();
        trace.phase = Some(phase);
        if trace.last_observation_signature.as_deref() != Some(signature.as_str())
            && trace.observations.len() < MAX_CAPTURE_ROWS
        {
            trace.last_observation_signature = Some(signature);
            trace.observations.push(row);
        }
    }

    fn keeper(&self, cx: &ActionContext<'_>) -> Option<Keeper> {
        let snapshot = cx.snapshot();
        let rows = snapshot.npcs()?;
        let here = snapshot.local_player().map(|row| row.value.player.network);
        let view = script::reach_route::live(snapshot.reach().map(|row| row.value), here);
        let candidates = rows
            .value
            .iter()
            .filter(|npc| npc.r#type == usize::try_from(self.fixture.keeper.id).ok())
            .filter(|npc| {
                self.keeper_index
                    .is_none_or(|index| i32::try_from(npc.index) == Ok(index))
            })
            .filter(|npc| {
                npc.name.as_deref().is_some_and(|name| {
                    name.eq_ignore_ascii_case(self.fixture.keeper.name.as_ref())
                })
            })
            .map(|npc| keeper_candidate(npc, self.fixture.keeper.name.as_ref(), here));
        shop_open::choose_keeper(candidates, view.zip(here))
    }

    fn reset_open(&mut self) {
        self.open = OpenCore::default();
        self.open_walk = None;
        self.open_trade = None;
        self.open_seen_trade = false;
    }

    fn poll_open(&mut self, cx: &mut ActionContext<'_>) -> Result<bool, String> {
        let now = cx.evidence().tick;
        if let Some(pending) = &self.open_trade {
            let Some(receipt) = cx.interaction_receipt(pending.request_id).copied() else {
                if now.saturating_sub(pending.request_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                    return Err(
                        "no actual Gerrant Trade dispatch receipt within the tick bound".into(),
                    );
                }
                return Ok(false);
            };
            if !receipt.accepted || receipt.evidence.run != cx.run() {
                return Err("Gerrant Trade was not accepted in the probe's current run".into());
            }
            let dispatched = Facts::new(
                Some(receipt.evidence.tick),
                Session::new(receipt.evidence.run.run, receipt.evidence.run.session),
                receipt.chat_since,
                0,
                false,
                false,
            );
            self.open.traded(&dispatched, &pending.keeper);
            self.keeper_index.get_or_insert(pending.keeper.index());
            self.open_seen_trade = true;
            self.open_trade = None;
        }
        let snapshot = cx.snapshot();
        let (chat_seq, cant_reach_seq) = snapshot.chat_lines(0).map_or((0, 0), |lines| {
            lines.value.iter().fold((0, 0), |(all, refused), line| {
                (
                    all.max(line.sequence),
                    if line.username.is_none()
                        && line.type_ == 0
                        && shop_open::is_cant_reach(&line.text)
                    {
                        refused.max(line.sequence)
                    } else {
                        refused
                    },
                )
            })
        });
        let facts = Facts::new(
            Some(now),
            Session::new(cx.run().run, cx.run().session),
            chat_seq,
            cant_reach_seq,
            self.open_seen_trade
                && snapshot.shop().is_some_and(|shop| shop.value.open)
                && self.stock_row(cx).is_some(),
            self.last_emit_tick == Some(now),
        );
        let decision = if let Some(walk) = self.open_walk.as_mut() {
            match walk.poll(cx) {
                Poll::Pending => return Ok(false),
                Poll::Ready(Err(error)) => {
                    return Err(format!("shared shop approach failed: {error:?}"))
                }
                Poll::Ready(Ok(receipt)) => {
                    self.open_walk = None;
                    let keeper = self.keeper(cx);
                    self.open.walked(receipt.end == WalkEnd::Arrived, || keeper)
                }
            }
        } else {
            let keeper = self.keeper(cx);
            self.open.tick(&facts, || keeper)
        };
        match decision {
            Decision::Wait => Ok(false),
            Decision::Opened => {
                let observation = scene_observation(cx);
                let mut trace = self.trace.lock();
                record_scene_baseline(&mut trace, observation)?;
                Ok(true)
            }
            Decision::Failed(reason) => Err(format!("shared Gerrant shop opener failed: {reason}")),
            Decision::Trade(keeper) => {
                let request = InteractReq::Npc {
                    name: keeper.name().to_owned(),
                    action: keeper.action().to_owned(),
                    index: Some(keeper.index()),
                };
                if let Some(request_id) = self.emit(cx, now, request)? {
                    self.open_trade = Some(PendingOpenTrade {
                        keeper,
                        request_id,
                        request_tick: now,
                    });
                }
                Ok(false)
            }
            Decision::Walk(tile) => {
                self.open_walk = Some(
                    Walk::begin(walk_request(tile, 1, None, cx.evidence()), cx).map_err(
                        |error| format!("shared shop approach was not admitted: {error:?}"),
                    )?,
                );
                Ok(false)
            }
        }
    }

    fn stock_row(&self, cx: &ActionContext<'_>) -> Option<(i32, i32, i32)> {
        cx.snapshot().shop()?.value.stock.iter().find_map(|row| {
            (row.def.id == self.fixture.feather.id && row.count > 0).then_some((
                row.slot,
                row.component_id,
                row.count,
            ))
        })
    }

    fn begin_sample(
        &mut self,
        cx: &mut ActionContext<'_>,
        series: Series,
        ordinal: usize,
        now: u64,
        before: SampleBaseline,
    ) -> Result<Option<usize>, String> {
        if self.last_emit_tick == Some(now) {
            return Ok(None);
        }
        let request = match series {
            Series::Held | Series::Refusal => {
                let Some((slot, component, stock)) = self.stock_row(cx) else {
                    return Err("Gerrant feather stock row is absent or empty".into());
                };
                if before.stock != Some(stock)
                    || before.item_slot != Some(slot)
                    || before.item_component != Some(component)
                {
                    return Err("posted feather row changed before the ShopButton dispatch".into());
                }
                InteractReq::ShopButton {
                    kind: "buy".into(),
                    name: self.fixture.feather.name.to_string(),
                    id: self.fixture.feather.id,
                    slot,
                    component,
                    chunk: 1,
                }
            }
            Series::Close => InteractReq::CloseModal,
        };
        let Some(request_id) = self.emit(cx, now, request)? else {
            return Ok(None);
        };
        let sample_index = self.add_sample(
            series,
            ordinal,
            request_id,
            now,
            SessionIdentity::from(cx.run()),
        );
        {
            let mut trace = self.trace.lock();
            let sample = &mut trace.samples[sample_index];
            sample.item_id =
                matches!(series, Series::Held | Series::Refusal).then_some(self.fixture.feather.id);
            sample.item_slot = before.item_slot;
            sample.item_component = before.item_component;
            sample.coins_before = before.coins;
            sample.held_before = before.held;
            sample.stock_before = before.stock;
            sample.server_closes_before = before.server_closes;
        }
        Ok(Some(sample_index))
    }

    fn dispatch_receipt(
        &self,
        cx: &ActionContext<'_>,
        request_id: u64,
    ) -> Option<script::native::InteractionReceipt> {
        cx.interaction_receipt(request_id).copied()
    }

    fn complete_control(
        &self,
        cx: &ActionContext<'_>,
        request_id: u64,
        requested_tick: u64,
        dispatch_tick: &mut Option<u64>,
        now: u64,
        label: &str,
    ) -> Result<bool, String> {
        if dispatch_tick.is_none() {
            if let Some(receipt) = self.dispatch_receipt(cx, request_id) {
                if !receipt.accepted {
                    return Err(format!("host refused {label} before dispatch"));
                }
                *dispatch_tick = Some(receipt.evidence.tick);
            } else if now.saturating_sub(requested_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                return Err(format!("no host dispatch receipt for {label} within {SAMPLE_REPLY_LIMIT_TICKS} observed ticks"));
            }
        }
        Ok(dispatch_tick.is_some())
    }

    fn fail_on_tick_limit(&self, now: u64, dispatch_tick: u64, label: &str) -> Result<(), String> {
        if now.saturating_sub(dispatch_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
            Err(format!(
                "no {label} reply within {SAMPLE_REPLY_LIMIT_TICKS} observed PLAYER_INFO ticks"
            ))
        } else {
            Ok(())
        }
    }

    fn refusal_line(&self, cx: &ActionContext<'_>, watermark: i32) -> Option<(i32, String)> {
        cx.snapshot()
            .chat_lines(watermark)?
            .value
            .iter()
            .filter(|line| {
                matches!(
                    line.text.as_str(),
                    "You don't have enough coins."
                        | "The shop has run out of stock."
                        | "You don't have enough inventory space."
                )
            })
            .min_by_key(|line| line.sequence)
            .map(|line| (line.sequence, line.text.clone()))
    }

    fn sample_error_flow(&self, sample: usize, reason: impl Into<String>) -> ScriptFlow {
        self.fail(reason, Some(sample))
    }
}

impl BuyReplyProbeScript {
    fn advance(&mut self, cx: &mut ActionContext<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let now = cx.evidence().tick;
        let phase_name = phase_name(self.phase);
        self.record_observation(cx, phase_name);
        let trace_started = self.trace.lock().started;
        if !trace_started {
            let mut trace = self.trace.lock();
            trace.started = true;
            trace.start_tick = Some(now);
            trace.session = Some(SessionIdentity::from(cx.run()));
        }
        if self
            .trace
            .lock()
            .login_player_info_ticks
            .is_some_and(|ticks| ticks > MAX_BATCH_LOGIN_TICKS)
        {
            return Ok(self.fail(
                format!("batch exceeded the first {MAX_BATCH_LOGIN_TICKS} login ticks"),
                self.active_sample(),
            ));
        }
        if let Some(reason) = self.validate_context(cx) {
            return Ok(self.fail(reason, self.active_sample()));
        }
        let flow = match self.phase {
            Phase::OpenShop => {
                if !self.open_seen_trade
                    && self.open_trade.is_none()
                    && cx.snapshot().shop().is_some_and(|shop| shop.value.open)
                {
                    self.fail(
                        "shop page was already open before the probe's own Trade",
                        None,
                    )
                } else {
                    match self.poll_open(cx) {
                        Ok(true) => {
                            self.phase = if self.reopening_after_coin_drop {
                                Phase::Refusal { next: 0 }
                            } else {
                                match self.batch.series {
                                    Series::Held => Phase::Held { next: 0 },
                                    Series::Refusal => Phase::DropCoins,
                                    Series::Close => Phase::Close { next: 0 },
                                }
                            };
                            ScriptFlow::Continue
                        }
                        Ok(false) => ScriptFlow::Continue,
                        Err(error) => self.fail(error, None),
                    }
                }
            }
            Phase::Held { next } => {
                if next >= self.batch.samples {
                    self.complete_batch(now)
                } else {
                    let shop_open = cx.snapshot().shop().is_some_and(|shop| shop.value.open);
                    let held = held_count(cx, self.fixture.feather.id);
                    let coins = held_count(cx, self.fixture.coins.id);
                    let Some((slot, component, stock)) = self.stock_row(cx) else {
                        return Ok(
                            self.fail("Gerrant's real feather stock row is unavailable", None)
                        );
                    };
                    let remaining = self.batch.samples - next;
                    if !shop_open {
                        self.fail("shop page closed during the held-change series", None)
                    } else if stock < remaining as i32 {
                        self.fail(
                            format!(
                                "Gerrant has {stock} feather stock for {remaining} remaining held buys in this login batch"
                            ),
                            None,
                        )
                    } else if held != Some(next as i32) {
                        self.fail(
                            format!(
                                "held series expected {next} feathers before batch sample {}, observed {held:?}",
                                next + 1
                            ),
                            None,
                        )
                    } else if coins.is_none_or(|count| count <= 0) {
                        self.fail(
                            format!(
                                "account coin seed was exhausted before {} held samples",
                                self.batch.samples
                            ),
                            None,
                        )
                    } else {
                        match self.begin_sample(
                            cx,
                            Series::Held,
                            self.batch.ordinal(next),
                            now,
                            SampleBaseline {
                                held,
                                coins,
                                stock: Some(stock),
                                item_slot: Some(slot),
                                item_component: Some(component),
                                ..SampleBaseline::default()
                            },
                        ) {
                            Ok(Some(sample)) => {
                                self.phase = Phase::AwaitHeld { sample };
                                ScriptFlow::Continue
                            }
                            Ok(None) => ScriptFlow::Continue,
                            Err(error) => self.fail(error, None),
                        }
                    }
                }
            }
            Phase::AwaitHeld { sample } => {
                let pending = self.sample(sample);
                match self.remember_receipt(cx, sample) {
                    Err(error) => self.sample_error_flow(sample, error),
                    Ok(None) => {
                        if now.saturating_sub(pending.request_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                            self.sample_error_flow(
                                sample,
                                "no actual ShopButton dispatch receipt within the tick bound",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                    Ok(Some(dispatch_tick)) => {
                        let current = held_count(cx, self.fixture.feather.id);
                        if let Some(held_after) =
                            current.filter(|count| *count > pending.held_before.unwrap_or(i32::MAX))
                        {
                            let before = pending.held_before.unwrap_or_default();
                            if held_after != before + 1 {
                                self.sample_error_flow(
                                    sample,
                                    format!("chunk-1 button changed held feathers from {before} to {held_after}"),
                                )
                            } else {
                                let mut trace = self.trace.lock();
                                let row = &mut trace.samples[sample];
                                row.held_after = Some(held_after);
                                row.stock_after = self.stock_row(cx).map(|(_, _, stock)| stock);
                                let next = row.ordinal - self.batch.first_ordinal + 1;
                                drop(trace);
                                if let Err(error) = self.settle_reply(sample, now) {
                                    self.sample_error_flow(sample, error)
                                } else {
                                    self.phase = Phase::Held { next };
                                    ScriptFlow::Continue
                                }
                            }
                        } else if self
                            .fail_on_tick_limit(now, dispatch_tick, "held-count")
                            .is_err()
                        {
                            self.sample_error_flow(
                                sample,
                                "accepted ShopButton produced no held-count change within the observed-tick bound",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                }
            }
            Phase::DropCoins => {
                let Some(coins) = held_count(cx, self.fixture.coins.id) else {
                    return Ok(self.fail(
                        "inventory projection is unavailable before the zero-coin series",
                        None,
                    ));
                };
                if coins == 0 {
                    self.phase = Phase::Refusal { next: 0 };
                    ScriptFlow::Continue
                } else if !cx.snapshot().shop().is_some_and(|shop| shop.value.open) {
                    self.fail("shop closed before the account-only coin drop", None)
                } else if self.stock_row(cx).is_none() {
                    self.fail(
                        "Gerrant must retain feather stock so 0-gp requests are coin-refused, not stock-refused",
                        None,
                    )
                } else {
                    let item = cx.snapshot().inventory().and_then(|inventory| {
                        inventory
                            .value
                            .iter()
                            .find(|item| item.def.id == self.fixture.coins.id && item.count > 0)
                    });
                    let Some(item) = item else {
                        return Ok(self.fail(
                            "seeded coins are absent from the owned account inventory",
                            None,
                        ));
                    };
                    let item_slot = item.slot;
                    let has_drop_action =
                        item.actions.iter().flatten().any(|action| action == "Drop");
                    if !has_drop_action {
                        self.fail("owned coin stack has no real Drop action", None)
                    } else {
                        match self.emit(
                            cx,
                            now,
                            InteractReq::Held {
                                name: self.fixture.coins.name.to_string(),
                                action: "Drop".into(),
                                slot: Some(item_slot),
                                target_item_id: Some(self.fixture.coins.id),
                            },
                        ) {
                            Ok(Some(request_id)) => {
                                self.phase = Phase::AwaitCoinDrop {
                                    request_id,
                                    request_tick: now,
                                    dispatch_tick: None,
                                    coins_before: coins,
                                };
                                ScriptFlow::Continue
                            }
                            Ok(None) => ScriptFlow::Continue,
                            Err(error) => self.fail(error, None),
                        }
                    }
                }
            }
            Phase::AwaitCoinDrop {
                request_id,
                request_tick,
                mut dispatch_tick,
                coins_before,
            } => match self.complete_control(
                cx,
                request_id,
                request_tick,
                &mut dispatch_tick,
                now,
                "owned coin Drop",
            ) {
                Err(error) => self.fail(error, None),
                Ok(false) => {
                    self.phase = Phase::AwaitCoinDrop {
                        request_id,
                        request_tick,
                        dispatch_tick,
                        coins_before,
                    };
                    ScriptFlow::Continue
                }
                Ok(true) => {
                    if held_count(cx, self.fixture.coins.id) == Some(0) {
                        self.reopening_after_coin_drop = true;
                        self.reset_open();
                        self.phase = Phase::OpenShop;
                        ScriptFlow::Continue
                    } else if self
                        .fail_on_tick_limit(now, dispatch_tick.unwrap_or(request_tick), "coin drop")
                        .is_err()
                    {
                        self.fail(
                            format!(
                                "owned coin Drop did not clear {} coins from the posted pack",
                                coins_before
                            ),
                            None,
                        )
                    } else {
                        self.phase = Phase::AwaitCoinDrop {
                            request_id,
                            request_tick,
                            dispatch_tick,
                            coins_before,
                        };
                        ScriptFlow::Continue
                    }
                }
            },
            Phase::Refusal { next } => {
                if next >= self.batch.samples {
                    self.complete_batch(now)
                } else {
                    let shop_open = cx.snapshot().shop().is_some_and(|shop| shop.value.open);
                    let held = held_count(cx, self.fixture.feather.id);
                    let coins = held_count(cx, self.fixture.coins.id);
                    let Some((slot, component, stock)) = self.stock_row(cx) else {
                        return Ok(self.fail(
                            "Gerrant feather row is unavailable for zero-coin refusal",
                            None,
                        ));
                    };
                    if !shop_open {
                        self.fail("shop page closed during the refusal series", None)
                    } else if coins != Some(0) {
                        self.fail(
                            format!("refusal sample requires 0 coins, observed {coins:?}"),
                            None,
                        )
                    } else if held.is_none() {
                        self.fail(
                            "pack count is unavailable before zero-coin ShopButton",
                            None,
                        )
                    } else {
                        match self.begin_sample(
                            cx,
                            Series::Refusal,
                            self.batch.ordinal(next),
                            now,
                            SampleBaseline {
                                held,
                                coins,
                                stock: Some(stock),
                                item_slot: Some(slot),
                                item_component: Some(component),
                                ..SampleBaseline::default()
                            },
                        ) {
                            Ok(Some(sample)) => {
                                self.phase = Phase::AwaitRefusal { sample };
                                ScriptFlow::Continue
                            }
                            Ok(None) => ScriptFlow::Continue,
                            Err(error) => self.fail(error, None),
                        }
                    }
                }
            }
            Phase::AwaitRefusal { sample } => {
                let pending = self.sample(sample);
                match self.remember_receipt(cx, sample) {
                    Err(error) => self.sample_error_flow(sample, error),
                    Ok(None) => {
                        if now.saturating_sub(pending.request_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                            self.sample_error_flow(
                                sample,
                                "no actual zero-coin ShopButton dispatch receipt within the tick bound",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                    Ok(Some(dispatch_tick)) => {
                        let watermark = self.sample(sample).chat_watermark.unwrap_or(i32::MAX);
                        let held_after = held_count(cx, self.fixture.feather.id);
                        let line = self.refusal_line(cx, watermark);
                        if let Some((sequence, text)) = line {
                            if sequence <= watermark {
                                self.sample_error_flow(sample, "stale refusal line was not newer than the host dispatch watermark")
                            } else {
                                let mut trace = self.trace.lock();
                                let row = &mut trace.samples[sample];
                                row.refusal_sequence = Some(sequence);
                                row.refusal_text = Some(text.clone());
                                row.refusal_tick = Some(now);
                                row.held_after = held_after;
                                if held_after != row.held_before {
                                    row.refusal_pack_update_tick = Some(now);
                                    row.refusal_pack_update_edges = Some(0);
                                    row.mark_error(
                                        "zero-coin refusal changed the held feather count",
                                    );
                                    drop(trace);
                                    self.fail(
                                        "zero-coin refusal changed held inventory",
                                        Some(sample),
                                    )
                                } else if text != "You don't have enough coins." {
                                    row.mark_error(format!(
                                        "zero-coin fixture produced the wrong refusal: {text}"
                                    ));
                                    drop(trace);
                                    self.fail(
                                        "zero-coin fixture did not produce the coins refusal",
                                        Some(sample),
                                    )
                                } else {
                                    let held_before = row.held_before.unwrap_or_default();
                                    drop(trace);
                                    self.phase = Phase::RefusalObserve {
                                        sample,
                                        line_tick: now,
                                        held_before,
                                    };
                                    ScriptFlow::Continue
                                }
                            }
                        } else if held_after.is_some_and(|count| Some(count) != pending.held_before)
                        {
                            self.sample_error_flow(
                                sample,
                                "held pack changed before a fresh refusal line appeared",
                            )
                        } else if self
                            .fail_on_tick_limit(now, dispatch_tick, "fresh refusal line")
                            .is_err()
                        {
                            self.sample_error_flow(
                                sample,
                                "no dispatch-scoped refusal line newer than InteractionReceipt.chat_since",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                }
            }
            Phase::RefusalObserve {
                sample,
                line_tick,
                held_before,
            } => {
                let Some(held_after) = held_count(cx, self.fixture.feather.id) else {
                    return Ok(self.sample_error_flow(
                        sample,
                        "pack count became unavailable while settling the refusal",
                    ));
                };
                if held_after != held_before {
                    let mut trace = self.trace.lock();
                    let row = &mut trace.samples[sample];
                    row.refusal_pack_update_tick = Some(now);
                    row.refusal_pack_update_edges = Some(now.saturating_sub(line_tick));
                    row.refusal_pack_observed_through_tick = Some(now);
                    row.mark_error("zero-coin refusal was followed by an unexpected pack update");
                    drop(trace);
                    self.fail(
                        "zero-coin refusal produced an unexpected pack update",
                        Some(sample),
                    )
                } else if now.saturating_sub(line_tick) >= REFUSAL_PACK_OBSERVE_TICKS {
                    let mut trace = self.trace.lock();
                    let row = &mut trace.samples[sample];
                    row.refusal_pack_observed_through_tick = Some(now);
                    let local_next = row.ordinal - self.batch.first_ordinal + 1;
                    drop(trace);
                    if let Err(error) = self.settle_reply(sample, line_tick) {
                        self.sample_error_flow(sample, error)
                    } else {
                        self.phase = Phase::Refusal { next: local_next };
                        ScriptFlow::Continue
                    }
                } else {
                    ScriptFlow::Continue
                }
            }
            Phase::Close { next } => {
                if next >= self.batch.samples {
                    self.complete_batch(now)
                } else {
                    let snapshot = cx.snapshot();
                    let shop = snapshot.shop();
                    let Some(shop) = shop else {
                        return Ok(self.fail("ShopView unavailable before CloseModal sample", None));
                    };
                    let shop_open = shop.value.open;
                    let server_closes = shop.value.server_closes;
                    let coins_before = held_count(cx, self.fixture.coins.id);
                    if !shop_open {
                        self.fail(
                            "CloseModal sample requires a locally open real shop page",
                            None,
                        )
                    } else {
                        match self.begin_sample(
                            cx,
                            Series::Close,
                            self.batch.ordinal(next),
                            now,
                            SampleBaseline {
                                coins: coins_before,
                                server_closes: Some(server_closes),
                                ..SampleBaseline::default()
                            },
                        ) {
                            Ok(Some(sample)) => {
                                self.phase = Phase::AwaitClose { sample };
                                ScriptFlow::Continue
                            }
                            Ok(None) => ScriptFlow::Continue,
                            Err(error) => self.fail(error, None),
                        }
                    }
                }
            }
            Phase::AwaitClose { sample } => {
                let pending = self.sample(sample);
                match self.remember_receipt(cx, sample) {
                    Err(error) => self.sample_error_flow(sample, error),
                    Ok(None) => {
                        if now.saturating_sub(pending.request_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                            self.sample_error_flow(
                                sample,
                                "no actual CloseModal dispatch receipt within the tick bound",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                    Ok(Some(dispatch_tick)) => {
                        let before = pending.server_closes_before.unwrap_or(u64::MAX);
                        let current = cx.snapshot().shop().map(|shop| shop.value.server_closes);
                        if current.is_some_and(|count| count > before) {
                            let after = current.unwrap_or(before);
                            let mut trace = self.trace.lock();
                            let row = &mut trace.samples[sample];
                            row.server_closes_after = Some(after);
                            row.server_close_tick = Some(now);
                            let count = after.saturating_sub(before);
                            if count != 1 {
                                row.mark_error(format!(
                                    "one CloseModal produced {count} server close packets"
                                ));
                                drop(trace);
                                self.fail(
                                    "CloseModal did not produce exactly one server close increment",
                                    Some(sample),
                                )
                            } else {
                                let ordinal = row.ordinal;
                                drop(trace);
                                if let Err(error) = self.settle_reply(sample, now) {
                                    self.sample_error_flow(sample, error)
                                } else {
                                    let next = ordinal - self.batch.first_ordinal + 1;
                                    self.reset_open();
                                    self.phase = if next == self.batch.samples {
                                        Phase::Close { next }
                                    } else {
                                        Phase::Reopen {
                                            next,
                                            close_tick: now,
                                            closes_after: after,
                                        }
                                    };
                                    ScriptFlow::Continue
                                }
                            }
                        } else if self
                            .fail_on_tick_limit(
                                now,
                                dispatch_tick,
                                "ShopView.server_closes increment",
                            )
                            .is_err()
                        {
                            self.sample_error_flow(
                                sample,
                                "no ShopView.server_closes increment after CloseModal; local shop.open is not a reply",
                            )
                        } else {
                            ScriptFlow::Continue
                        }
                    }
                }
            }
            Phase::Reopen {
                next,
                close_tick,
                closes_after,
            } => {
                let snapshot = cx.snapshot();
                let shop = snapshot.shop();
                if shop.is_some_and(|shop| shop.value.server_closes != closes_after) {
                    self.fail(
                        "server close counter changed outside the measured CloseModal dispatch",
                        None,
                    )
                } else if !self.open_seen_trade
                    && self.open_trade.is_none()
                    && shop.is_some_and(|shop| shop.value.open)
                {
                    if now.saturating_sub(close_tick) >= SAMPLE_REPLY_LIMIT_TICKS {
                        self.fail(
                            "shop page did not become locally absent before the next Trade",
                            None,
                        )
                    } else {
                        ScriptFlow::Continue
                    }
                } else {
                    match self.poll_open(cx) {
                        Ok(true) => {
                            self.phase = Phase::Close { next };
                            ScriptFlow::Continue
                        }
                        Ok(false) => ScriptFlow::Continue,
                        Err(error) => self.fail(error, None),
                    }
                }
            }
            Phase::Done => ScriptFlow::Complete,
        };
        Ok(flow)
    }
}

impl NativeMachine for BuyReplyProbeScript {
    type Args = Self;
    type Output = ScriptFlow;

    fn begin(probe: Self, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(probe)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<ScriptFlow, ActionError>> {
        match self.advance(cx) {
            Ok(ScriptFlow::Continue) => Poll::Pending,
            Ok(flow) => Poll::Ready(Ok(flow)),
            Err(failure) => Poll::Ready(Err(ActionError::Failed(failure.message))),
        }
    }
}

struct OwnedProbeScript {
    pending: Option<BuyReplyProbeScript>,
    action: Option<ActionHandle<BuyReplyProbeScript>>,
    trace: Arc<Mutex<ProbeTrace>>,
    noted_settled_replies: usize,
    #[cfg(test)]
    progress_only: bool,
}

impl OwnedProbeScript {
    fn new(probe: BuyReplyProbeScript) -> Self {
        let trace = Arc::clone(&probe.trace);
        let noted_settled_replies = trace.lock().settled_replies;
        Self {
            trace,
            pending: Some(probe),
            action: None,
            noted_settled_replies,
            #[cfg(test)]
            progress_only: false,
        }
    }

    #[cfg(test)]
    fn progress_fixture(trace: Arc<Mutex<ProbeTrace>>) -> Self {
        let noted_settled_replies = trace.lock().settled_replies;
        Self {
            pending: None,
            action: None,
            trace,
            noted_settled_replies,
            progress_only: true,
        }
    }

    fn poll_action(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.action.is_none() {
            let probe = self.pending.take().expect("probe begins once");
            self.action = Some(
                tick.actions
                    .begin::<BuyReplyProbeScript>(probe, &mut tick.cx)
                    .map_err(|error| self.failure(error))?,
            );
        }
        match tick.actions.poll(
            self.action.as_ref().expect("probe action began"),
            &mut tick.cx,
        ) {
            Poll::Pending => Ok(ScriptFlow::Continue),
            Poll::Ready(result) => {
                self.action = None;
                result.map_err(|error| self.failure(error))
            }
        }
    }

    fn note_settled_progress(&mut self, tick: &mut NativeTick<'_>) {
        let settled = self.trace.lock().settled_replies;
        for _ in self.noted_settled_replies..settled {
            tick.note_progress();
        }
        self.noted_settled_replies = settled;
    }
    fn failure(&self, error: ActionError) -> ScriptFailure {
        let message = format!("owned probe action failed: {error}");
        self.trace.lock().fail(message.clone());
        ScriptFailure {
            code: "buy-reply-probe".into(),
            message: message.into(),
        }
    }
}

impl Script for OwnedProbeScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        #[cfg(test)]
        if self.progress_only {
            self.note_settled_progress(tick);
            return Ok(ScriptFlow::Continue);
        }
        let flow = self.poll_action(tick)?;
        self.note_settled_progress(tick);
        Ok(flow)
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::OpenShop => "open_shop",
        Phase::Held { .. } => "held",
        Phase::AwaitHeld { .. } => "await_held",
        Phase::DropCoins => "drop_coins",
        Phase::AwaitCoinDrop { .. } => "await_coin_drop",
        Phase::Refusal { .. } => "refusal",
        Phase::AwaitRefusal { .. } => "await_refusal",
        Phase::RefusalObserve { .. } => "refusal_observe",
        Phase::Close { .. } => "close",
        Phase::AwaitClose { .. } => "await_close",
        Phase::Reopen { .. } => "reopen",
        Phase::Done => "done",
    }
}

fn held_count(cx: &ActionContext<'_>, item_id: i32) -> Option<i32> {
    let inventory = cx.snapshot().inventory()?;
    Some(
        inventory
            .value
            .iter()
            .filter(|item| item.def.id == item_id)
            .map(|item| item.count)
            .sum(),
    )
}

struct TerminalObservation<'a> {
    snapshot: &'a GameSnapshot,
    client_session_generation: u64,
    player_info_generation: u64,
    npc_info_generation: u64,
    native_status: Option<&'a str>,
}

fn terminal_context(
    account: &str,
    fixture: &Fixture,
    trace: &ProbeTrace,
    observation: TerminalObservation<'_>,
) -> Value {
    let TerminalObservation {
        snapshot,
        client_session_generation,
        player_info_generation,
        npc_info_generation,
        native_status,
    } = observation;
    let run = trace.session.map_or(
        RunKey {
            slot: 0,
            run: 0,
            session: 0,
        },
        |identity| RunKey {
            slot: identity.slot,
            run: identity.run,
            session: identity.session,
        },
    );
    let view = api::snapshot::SnapshotView::new(
        Some(snapshot),
        EvidenceStamp {
            run,
            tick: u64::from(snapshot.tick()),
            sequence: 0,
        },
    );
    let observed_world = view.world().map(|world| world.value);
    let player_tile = view.local_player().map(|row| {
        let mut tile = row.value.player.network;
        if let Some(world) = observed_world {
            tile.level = world.level;
        }
        tile
    });
    let observed_identity = observed_world.map(|world| SceneIdentity {
        map_base_x: world.map_base_x,
        map_base_z: world.map_base_z,
        level: world.level,
    });
    let local_player_teleport_watermark = view
        .local_player_teleport_watermark()
        .map(|watermark| watermark.value);
    let rebuild_invalidation_watermark = view
        .rebuild_invalidation_watermark()
        .map(|watermark| watermark.value);
    let expected_scene = trace.scene_fence.map(|fence| {
        json!({
            "map_base_x": fence.identity.map_base_x,
            "map_base_z": fence.identity.map_base_z,
            "level": fence.identity.level,
        })
    });
    let observed_scene = observed_identity.map(|identity| {
        json!({
            "map_base_x": identity.map_base_x,
            "map_base_z": identity.map_base_z,
            "level": identity.level,
        })
    });
    let player_scene = match (
        trace.scene_fence,
        observed_identity,
        player_tile,
        rebuild_invalidation_watermark,
        local_player_teleport_watermark,
    ) {
        (Some(fence), Some(identity), _, _, _) if fence.identity != identity => "scene_changed",
        (Some(fence), _, _, Some(watermark), _)
            if fence.rebuild_invalidation_watermark != watermark =>
        {
            "rebuild_invalidated"
        }
        (Some(fence), _, _, Some(_), Some(watermark))
            if fence.local_player_teleport_watermark != watermark =>
        {
            "local_player_teleported"
        }
        (Some(fence), Some(identity), Some(tile), Some(_), Some(_))
            if fence.identity == identity && fence.last_player_tile == tile =>
        {
            "within_admitted_scene"
        }
        (Some(fence), Some(identity), Some(_), Some(_), Some(_)) if fence.identity == identity => {
            "player_moved_since_last_script_tick"
        }
        (Some(_), _, _, _, _) => "scene_evidence_unavailable",
        (None, _, Some(_), _, _) => "before_shop_admission",
        (None, _, _, _, _) => "unavailable",
    };
    let npc_view = view.npcs();
    let npc_info_state = match npc_view {
        None => "unavailable",
        Some(rows) if rows.value.is_empty() => "coherent_empty",
        Some(_) => "coherent_rows",
    };
    let keeper_type = usize::try_from(fixture.keeper.id).ok();
    let matching_npc_rows = npc_view
        .map(|rows| {
            rows.value
                .iter()
                .filter(|npc| {
                    npc.r#type == keeper_type
                        || npc.name.as_deref().is_some_and(|name| {
                            name.eq_ignore_ascii_case(fixture.keeper.name.as_ref())
                        })
                })
                .map(|npc| {
                    json!({
                        "index": npc.index,
                        "type": npc.r#type,
                        "name": npc.name,
                        "network_tile": npc.network,
                        "rendered_tile": npc.tile,
                        "actions": npc.actions,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let keeper_state = if npc_view.is_none() {
        "npc_view_unavailable"
    } else if !matching_npc_rows.is_empty() {
        "matching_keeper_rows_present"
    } else if npc_info_state == "coherent_empty" {
        "coherent_empty_view"
    } else {
        "absent_from_coherent_view"
    };
    json!({
        "account": account,
        "phase": trace.phase,
        "last_reply": trace.last_reply,
        "run_session": trace.session,
        "client_session_generation": client_session_generation,
        "player_info_generation": player_info_generation,
        "npc_info_generation": npc_info_generation,
        "player_network_tile": player_tile,
        "player_scene": {
            "state": player_scene,
            "expected_scene": expected_scene,
            "observed_scene": observed_scene,
            "baseline_tile": trace.scene_fence.map(|fence| fence.baseline_player_tile),
            "baseline_evidence_tick": trace.scene_fence.map(|fence| fence.baseline_evidence_tick),
            "last_observed_tile": trace.scene_fence.map(|fence| fence.last_player_tile),
            "observed_tile": player_tile,
            "expected_rebuild_invalidation_watermark": trace
                .scene_fence
                .map(|fence| fence.rebuild_invalidation_watermark),
            "observed_rebuild_invalidation_watermark": rebuild_invalidation_watermark,
            "expected_local_player_teleport_watermark": trace
                .scene_fence
                .map(|fence| fence.local_player_teleport_watermark),
            "observed_local_player_teleport_watermark": local_player_teleport_watermark,
        },
        "npc_info_state": npc_info_state,
        "keeper_state": keeper_state,
        "matching_npc_rows": matching_npc_rows,
        "native_status": native_status,
        "terminal_reason": trace.failure,
    })
}

struct LiveState {
    account: String,
    fixture: Fixture,
    selected: Arc<SelectedGameData>,
    batch_index: usize,
    batch: BatchPlan,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    start_context: Option<ScriptStartHandle>,
    trace: Arc<Mutex<ProbeTrace>>,
    started_at: Option<Instant>,
    login_client_session: Option<u64>,
    login_start_tick: Option<u64>,
    started: bool,
    packet_counts: DriverPacketCounts,
}

impl LiveState {
    fn observe_login_clock(&mut self, client: &Client) {
        if self.login_client_session != Some(client.gens.session) {
            if self.started && self.login_client_session.is_some() {
                self.trace.lock().fail(format!(
                    "client session changed during batch: expected {:?}, observed {}",
                    self.login_client_session, client.gens.session
                ));
            }
            self.login_client_session = Some(client.gens.session);
            self.login_start_tick = None;
        }
        if client.ingame && client.scene_state == 2 {
            let tick = u64::from(self.snapshot.tick());
            let start = *self.login_start_tick.get_or_insert(tick);
            let login_ticks = tick.saturating_sub(start).saturating_add(1);
            let mut trace = self.trace.lock();
            trace.login_player_info_ticks = Some(login_ticks);
            if self.started && login_ticks > MAX_BATCH_LOGIN_TICKS {
                trace.fail(format!(
                    "batch exceeded the first {MAX_BATCH_LOGIN_TICKS} login ticks"
                ));
            }
        }
    }

    fn update_terminal_context(&self, client: &Client, native_status: Option<&str>) {
        let mut trace = self.trace.lock();
        let context = terminal_context(
            &self.account,
            &self.fixture,
            &trace,
            TerminalObservation {
                snapshot: &self.snapshot,
                client_session_generation: client.gens.session,
                player_info_generation: client.gens.player_info,
                npc_info_generation: client.gens.npc_info,
                native_status,
            },
        );
        trace.terminal_context = Some(context);
    }

    fn start_error(&self) -> Option<String> {
        if !self.snapshot.ingame() || self.snapshot.scene_state() != 2 {
            return Some("StartScript arrived before ingame scene_state=2".into());
        }
        if self.snapshot.tile()
            != Some((
                self.fixture.stand.x,
                self.fixture.stand.z,
                self.fixture.stand.level,
            ))
        {
            return Some(format!(
                "account-only teleport did not reach Gerrant's selected access tile {:?}: {:?}",
                self.fixture.stand,
                self.snapshot.tile()
            ));
        }
        if self.snapshot.inv_count(self.fixture.coins.id) != SEED_COINS {
            return Some(format!(
                "account-only coin seed was not exact: expected {SEED_COINS}, observed {}",
                self.snapshot.inv_count(self.fixture.coins.id)
            ));
        }
        if self.snapshot.inv_count(self.fixture.feather.id) != 0 {
            return Some("fresh account unexpectedly held feathers at probe start".into());
        }
        if self.snapshot.shop().open {
            return Some("shop page was already open before the probe's Trade".into());
        }
        None
    }

    fn frame(&mut self, client: &mut Client, hold: bool) {
        Driver::finish_pending_packet(client);
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.observe_login_clock(client);
        self.update_terminal_context(client, None);
        if self.started {
            self.packet_counts.observe(client, self.snapshot.tick());
        }
        if client.ingame && client.scene_state == 2 {
            self.trace.lock().player_info_clock.observe(
                client.gens.session,
                client.gens.player_info,
                Instant::now(),
            );
        }

        if self.runner.on_start_script() && !self.started {
            let (median, clock_accepted) = {
                let trace = self.trace.lock();
                (
                    trace.player_info_clock.median_ms,
                    trace.player_info_clock.accepted(),
                )
            };
            if !clock_accepted {
                if median.is_some() {
                    self.trace.lock().fail(format!(
                        "Engine A PLAYER_INFO median {median:?} ms is outside 600 ms +/-30 ms; no probe Start was sent"
                    ));
                }
                return;
            }
            if let Some(reason) = self.start_error() {
                self.trace.lock().fail(reason);
                return;
            }
            let Some(handle) = self.start_context.as_ref() else {
                self.trace
                    .lock()
                    .fail("StartScript reached before native script context was installed");
                return;
            };
            let script = BuyReplyProbeScript::for_batch(
                self.account.clone(),
                self.fixture.clone(),
                Arc::clone(&self.trace),
                self.batch_index,
                self.batch,
            );
            match handle.start_test_script(
                &self.account,
                Box::new(OwnedProbeScript::new(script)),
                Some(Arc::clone(&self.selected)),
            ) {
                Ok(run) => {
                    {
                        let mut trace = self.trace.lock();
                        record_script_start(&mut trace, u64::from(self.snapshot.tick()), run);
                    }
                    self.started = true;
                    self.started_at = Some(Instant::now());
                    self.runner.observe_script_running();
                }
                Err(error) => {
                    self.trace
                        .lock()
                        .fail(format!("native probe start failed: {error}"));
                    return;
                }
            }
        }

        if matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.update_terminal_context(client, None);
            return;
        }
        self.runner.tick_with_hold(client, hold);
        if self.started {
            self.packet_counts.observe(client, self.snapshot.tick());
        }
        self.update_terminal_context(client, None);
    }
}

fn mint_account() -> (String, String) {
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .expect("buy reply probe requires its own BOT_LIVE_NAME_PREFIX");
    assert_eq!(
        prefix.len(),
        3,
        "BOT_LIVE_NAME_PREFIX must be exactly three bytes"
    );
    assert!(prefix.is_ascii(), "BOT_LIVE_NAME_PREFIX must be ASCII");
    let names = super::mint_live_names(1);
    let account = names.first().expect("mint one owned account").clone();
    assert!(
        account.starts_with(&prefix),
        "minted account escaped this probe's prefix"
    );
    let (name, password) = super::mint_live_entries(&names)
        .into_iter()
        .find(|(name, _)| name == &account)
        .expect("minted account password");
    assert_eq!(name, account);
    (account, password)
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

fn action_matches_session(action: &Value, session: SessionIdentity) -> bool {
    action["run"].as_str().is_some_and(|run| {
        debug_number(run, "slot") == i64::try_from(session.slot).ok()
            && debug_number(run, "run") == i64::try_from(session.run).ok()
            && debug_number(run, "session") == i64::try_from(session.session).ok()
    })
}

fn record_capture_evidence(samples: &mut [Sample], actions: &[Value]) -> Vec<String> {
    let mut failures = Vec::new();
    for sample in samples {
        let matches = actions
            .iter()
            .filter(|action| {
                action["request_id"].as_u64() == Some(sample.request_id)
                    && action_matches_session(action, sample.session)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            let message = format!(
                "request {} has {} actual host action records, expected exactly one",
                sample.request_id,
                matches.len()
            );
            sample.mark_error(message.clone());
            failures.push(message);
            continue;
        }
        let action = matches[0];
        let debug = action["request"]["debug"].as_str().unwrap_or_default();
        let request_matches = match sample.series {
            Series::Held | Series::Refusal => {
                debug.starts_with("ShopButton {")
                    && debug.contains("kind: \"buy\"")
                    && debug_number(debug, "chunk") == Some(1)
                    && debug_number(debug, "id") == sample.item_id.map(i64::from)
                    && debug_number(debug, "slot") == sample.item_slot.map(i64::from)
                    && debug_number(debug, "component") == sample.item_component.map(i64::from)
            }
            Series::Close => debug == "CloseModal",
        };
        let tick = action["tick"].as_u64();
        let accepted = action["accepted"].as_bool();
        let decoded = action["wire_decoded"].as_bool();
        let opcodes = action["wire_opcodes"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_u64)
                    .map(|v| v as u8)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sample.captured_dispatch_tick = tick;
        sample.captured_snapshot_tick = action["snapshot_tick"].as_u64();
        sample.dispatch_accepted = accepted;
        sample.dispatch_wire_decoded = decoded;
        sample.dispatch_wire_opcodes = opcodes;
        if !request_matches {
            let message = format!(
                "request {} capture did not match its measured {:?} dispatch",
                sample.request_id, sample.series
            );
            sample.mark_error(message.clone());
            failures.push(message);
        }
        if accepted != Some(true)
            || decoded != Some(true)
            || sample.dispatch_wire_opcodes.is_empty()
        {
            let message = format!(
                "request {} lacks accepted, decoded Driver packet evidence",
                sample.request_id
            );
            sample.mark_error(message.clone());
            failures.push(message);
        }
        if tick != sample.dispatch_tick {
            let message = format!(
                "request {} dispatch tick mismatch: receipt {:?}, captured action {:?}",
                sample.request_id, sample.dispatch_tick, tick
            );
            sample.mark_error(message.clone());
            failures.push(message);
        }
    }
    failures
}

fn histogram(samples: &[Sample], series: Series) -> BTreeMap<String, usize> {
    let mut values = BTreeMap::new();
    for sample in samples.iter().filter(|sample| sample.series == series) {
        if sample.status == SampleStatus::Replied {
            if let Some(edges) = sample.reply_edges {
                *values.entry(edges.to_string()).or_insert(0) += 1;
            }
        }
    }
    values
}

fn series_summary(samples: &[Sample], series: Series) -> Value {
    let rows = samples
        .iter()
        .filter(|sample| sample.series == series)
        .collect::<Vec<_>>();
    let replies = rows
        .iter()
        .filter(|sample| sample.status == SampleStatus::Replied && sample.reply_edges.is_some())
        .count();
    let errors = rows
        .iter()
        .filter(|sample| sample.status == SampleStatus::Error)
        .count();
    let edge_values = rows
        .iter()
        .filter(|sample| sample.status == SampleStatus::Replied)
        .filter_map(|sample| sample.reply_edges)
        .collect::<Vec<_>>();
    json!({
        "required_samples": series.expected(),
        "attempted_samples": rows.len(),
        "reply_samples": replies,
        "error_samples": errors,
        "missing_samples": series.expected().saturating_sub(rows.len()),
        "min_reply_edges": edge_values.iter().min(),
        "max_reply_edges": edge_values.iter().max(),
        "distribution_tick_edges": histogram(samples, series),
    })
}

fn refusal_pack_update_summary(samples: &[Sample]) -> Value {
    let rows = samples
        .iter()
        .filter(|sample| sample.series == Series::Refusal)
        .collect::<Vec<_>>();
    let mut histogram = BTreeMap::<String, usize>::new();
    for sample in &rows {
        if let Some(edges) = sample.refusal_pack_update_edges {
            *histogram.entry(edges.to_string()).or_insert(0) += 1;
        }
    }
    json!({
        "samples": rows.len(),
        "samples_with_pack_update": rows.iter().filter(|sample| sample.refusal_pack_update_tick.is_some()).count(),
        "samples_without_pack_update_through_observed_tick": rows.iter().filter(|sample| sample.refusal_pack_update_tick.is_none() && sample.refusal_pack_observed_through_tick.is_some()).count(),
        "line_to_pack_update_distribution_tick_edges": histogram,
    })
}

fn proposed_verdict(samples: &[Sample]) -> (Option<u64>, Option<u64>, String) {
    let complete_counts = [Series::Held, Series::Refusal, Series::Close]
        .into_iter()
        .all(|series| {
            let rows = samples
                .iter()
                .filter(|sample| sample.series == series)
                .collect::<Vec<_>>();
            rows.len() == series.expected()
                && rows
                    .iter()
                    .all(|sample| sample.status == SampleStatus::Replied)
        });
    let all_have_dispatch = samples.iter().all(|sample| {
        sample.dispatch_tick.is_some()
            && sample.dispatch_accepted == Some(true)
            && sample.dispatch_wire_decoded == Some(true)
            && !sample.dispatch_wire_opcodes.is_empty()
    });
    let refusal_lines_are_fresh = samples
        .iter()
        .filter(|sample| sample.series == Series::Refusal)
        .all(|sample| {
            sample
                .refusal_sequence
                .zip(sample.chat_watermark)
                .is_some_and(|(sequence, watermark)| sequence > watermark)
                && sample.refusal_text.as_deref() == Some("You don't have enough coins.")
                && sample.held_after == sample.held_before
        });
    if !complete_counts || !all_have_dispatch || !refusal_lines_are_fresh {
        return (
            None,
            None,
            "NO VERDICT: required sample counts, actual dispatch evidence, or fresh-refusal checks failed".into(),
        );
    }
    let k = samples
        .iter()
        .filter(|sample| matches!(sample.series, Series::Held | Series::Close))
        .filter_map(|sample| sample.reply_edges)
        .max();
    let Some(k) = k else {
        return (
            None,
            None,
            "NO VERDICT: held/close latency set is empty".into(),
        );
    };
    let proposed = if k <= 2 { 2 } else { k };
    let verdict = if k <= 2 {
        format!("CONFIRM BUY_REPLY_TICKS = 2 because K = {k} <= 2")
    } else {
        format!("SET BUY_REPLY_TICKS = {k} because K = {k} > 2")
    };
    (Some(k), Some(proposed), verdict)
}

fn mark_unfinished_samples(samples: &mut [Sample], reason: &str) {
    for sample in samples {
        if sample.status == SampleStatus::Pending {
            sample.mark_error(reason.to_owned());
        }
    }
}

struct BatchOutcome {
    admission: Option<BatchAdmission>,
    samples: Vec<Sample>,
    observations: Vec<Value>,
    actions: Vec<Value>,
    interaction_packets: Value,
    driver_packets: Value,
    max_packets: usize,
    packet_trace_complete: bool,
    capture_failures: Vec<String>,
    terminal_error: Option<String>,
    script: Value,
}

struct BatchRunConfig {
    batch_index: usize,
    batch: BatchPlan,
    account: String,
    password: String,
    uid: i32,
    engine_name: &'static str,
    template: Arc<SharedClientTemplate>,
    selected: Arc<SelectedGameData>,
    fixture: Fixture,
}

fn run_live_batch(config: BatchRunConfig) -> BatchOutcome {
    let BatchRunConfig {
        batch_index,
        batch,
        account,
        password,
        uid,
        engine_name,
        template,
        selected,
        fixture,
    } = config;
    let trace = Arc::new(Mutex::new(ProbeTrace::default()));
    let capture = Arc::new(StdMutex::new(CombatCapture::default()));
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let mut runner = ScenarioRunner::with_world(
        probe_scenario(fixture.stand, Arc::clone(&trace)),
        template.world(),
    );
    runner.set_live_names(std::slice::from_ref(&account));
    runner.set_shot_sink(Box::new(|_, _| {}));
    let frame = FrameBuf::new();
    let frame_state = Arc::new(Mutex::new(LiveState {
        account: account.clone(),
        fixture: fixture.clone(),
        selected,
        batch_index,
        batch,
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_context: None,
        trace: Arc::clone(&trace),
        started_at: None,
        login_client_session: None,
        login_start_tick: None,
        started: false,
        packet_counts: DriverPacketCounts::new(),
    }));
    let state = Arc::clone(&frame_state);
    let frame_account = account.clone();
    let profile = Profile {
        username: account.clone(),
        password: password.into(),
        uid,
        settings: ProfileSettings {
            random_events: false,
            ..ProfileSettings::default()
        },
    };
    let mut play = run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username == frame_account {
                client.set_draw(true);
                state.lock().frame(client, hold.hold);
            }
        },
    )
    .unwrap_or_else(|error| panic!("connect to forwarded Engine {engine_name}: {error}"));
    let start_context = play.script_start_handle();
    {
        let mut state = frame_state.lock();
        state.runner.set_obj_names(play.obj_names());
        state.start_context = Some(start_context);
    }
    let observed_state = Arc::clone(&frame_state);
    play.set_post_native_player_info_observer(move |client, username| {
        let mut state = observed_state.lock();
        if username == state.account && state.started {
            let tick = state.snapshot.tick();
            state.packet_counts.observe(client, tick);
        }
    });
    play.try_spawn_slot(profile, None, Some(frame), None)
        .expect("spawn probe only after its packet observer and script context are installed");
    play.focus(&account);

    let preparation_deadline = Instant::now() + PREPARATION_DEADLINE;
    let mut terminal_error = None;
    let mut last_native_status = None;
    let mut completion_seen_at = None;
    loop {
        if let Some(arm) = play.arm(&account) {
            if frame_state.lock().runner.take_relog_login() {
                arm.arm_explicit_login();
            }
        }
        let native_status = play.script_native_status(&account);
        if let Some(status) = native_status.as_ref() {
            combat_proof::record_status(&account, status, None);
            last_native_status = Some(format!("{status:?}"));
        }
        let (runner_status, started_at, completed, failure, complete_tick, current_tick) = {
            let state = frame_state.lock();
            let trace = trace.lock();
            (
                format!("{:?}", state.runner.status()),
                state.started_at,
                trace.completed,
                trace.failure.clone(),
                trace.complete_tick,
                u64::from(state.snapshot.tick()),
            )
        };
        if let Some(reason) = failure {
            terminal_error = Some(reason);
            break;
        }
        if let Some(status) = native_status.as_ref() {
            if status.phase == script::native::NativePhase::Blocked {
                terminal_error = Some(status.failure.as_ref().map_or_else(
                    || "native probe entered terminal Blocked status".to_owned(),
                    |failure| format!("native probe entered Blocked status: {}", failure.message),
                ));
                break;
            }
            if status.phase == script::native::NativePhase::Complete && !completed {
                terminal_error =
                    Some("native probe became terminal before its reply trace completed".into());
                break;
            }
        }
        if completed {
            if runner_status == "Passed" {
                break;
            }
            let complete_seen_at = completion_seen_at.get_or_insert_with(Instant::now);
            let complete_ticks = complete_tick.map_or(0, |tick| current_tick.saturating_sub(tick));
            if complete_ticks >= 2 || complete_seen_at.elapsed() >= RUNNER_COMPLETE_DEADLINE {
                terminal_error = Some(format!(
                    "native probe completed but ScenarioRunner remained {runner_status} after {complete_ticks} game ticks"
                ));
                break;
            }
        } else if let Some(started_at) = started_at {
            if started_at.elapsed() >= CELL_DEADLINE {
                terminal_error = Some(format!(
                    "batch exceeded its {} second harness safety deadline",
                    CELL_DEADLINE.as_secs()
                ));
                break;
            }
        } else if Instant::now() >= preparation_deadline {
            terminal_error = Some(format!(
                "account preparation exceeded its {} second safety deadline",
                PREPARATION_DEADLINE.as_secs()
            ));
            break;
        }
        if runner_status.starts_with("Failed") {
            terminal_error = Some(format!("ScenarioRunner ended with {runner_status}"));
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }

    {
        let mut trace = trace.lock();
        if let Some(error) = terminal_error.as_deref() {
            trace.fail(error.to_owned());
        }
        let observed_context = trace.terminal_context.clone().unwrap_or_else(|| json!({}));
        trace.terminal_context = Some(json!({
            "frame": observed_context,
            "phase": trace.phase,
            "last_reply": trace.last_reply,
            "run_session": trace.session,
            "native_status": last_native_status,
            "terminal_reason": trace.failure,
        }));
    }
    play.script_stop(&account);

    let (mut samples, observations, script, admission) = {
        let trace = trace.lock();
        let mut samples = trace.samples.clone();
        mark_unfinished_samples(
            &mut samples,
            "harness ended before this dispatch produced a reply",
        );
        let admission = trace.session.zip(trace.login_player_info_ticks).map(
            |(session, login_player_info_ticks)| BatchAdmission {
                plan: batch,
                account: account.clone(),
                session,
                login_player_info_ticks,
            },
        );
        let script = json!({
            "batch_index": batch_index,
            "plan": batch,
            "account": account,
            "started": trace.started,
            "start_tick": trace.start_tick,
            "session": trace.session,
            "script_run": trace.script_run,
            "phase": trace.phase,
            "settled_replies": trace.settled_replies,
            "last_reply": trace.last_reply,
            "login_player_info_ticks": trace.login_player_info_ticks,
            "complete_tick": trace.complete_tick,
            "completed": trace.completed,
            "failure": trace.failure,
            "terminal_context": trace.terminal_context,
            "player_info_clock": trace.player_info_clock,
        });
        (samples, trace.observations.clone(), script, admission)
    };
    let actions = capture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .actions
        .clone();
    let interactions = DriverPacketTrace::from_actions(&actions);
    let (interaction_packets, _, _) = interactions.value();
    let state = frame_state.lock();
    let (driver_packets, max_packets, packet_trace_complete) = state.packet_counts.value();
    let interactions_counted = state.packet_counts.includes_interactions(&interactions);
    drop(state);
    let mut capture_failures = record_capture_evidence(&mut samples, &actions);
    if !interactions_counted {
        capture_failures.push("real sink counts omitted a decoded interaction packet".into());
    }
    if !packet_trace_complete {
        capture_failures.push("outgoing Driver packet trace was incomplete".into());
    }
    if max_packets > MAX_DRIVER_PACKETS_PER_TICK {
        capture_failures.push(format!(
            "observed {max_packets} Driver packets in one PLAYER_INFO tick, above the limit of {MAX_DRIVER_PACKETS_PER_TICK}"
        ));
    }
    BatchOutcome {
        admission,
        samples,
        observations,
        actions,
        interaction_packets,
        driver_packets,
        max_packets,
        packet_trace_complete,
        capture_failures,
        terminal_error,
        script,
    }
}

fn run_live_probe() {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "probe requires LIVE=1"
    );
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "Engine A probe requires BOT_CPU=1"
    );
    let evidence_root = std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .expect("buy reply probe requires LIVE_EVIDENCE_DIR");
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let evidence = evidence_root.join(format!("buy-reply-probe-engine-a-{epoch}"));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create per-run probe evidence directory");

    let options = profile_options(&scratch)
        .unwrap_or_else(|error| panic!("buy reply probe prerequisites: {error}"));
    let engine_game_port = options
        .port
        .expect("profile options require WORLD_GAME_PORT");
    let engine_http_port = options
        .http_port
        .expect("profile options require WORLD_HTTP_PORT");
    let nav_pack = options.nav_pack.clone().expect("selected navigation pack");
    let nav_flags = options
        .nav_flags
        .clone()
        .expect("selected navigation flags");
    let template = Arc::new(
        options
            .resolve(None)
            .expect("resolve Engine A R289 profile")
            .prepare_template()
            .expect("prepare isolated Engine A R289 client template"),
    );
    let selected = game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let nav_world = template.world().expect("selected R289 navigation world");
    if nav_world.named_bank_facts().is_none() {
        nav_world
            .bind_named_bank_facts(&selected)
            .expect("bind selected R289 service facts before Play starts");
    }
    if nav_world.shop_keeper_facts().is_none() {
        nav_world
            .bind_shop_keeper_facts(&selected)
            .expect("bind selected R289 shop facts before fixture lookup");
    }
    let keepers = Arc::clone(
        nav_world
            .shop_keeper_facts()
            .expect("selected R289 world binds shop keeper facts"),
    );
    let fixture = fixture(&selected, &keepers)
        .unwrap_or_else(|error| panic!("buy reply probe content fixture: {error}"));

    let mut samples = Vec::new();
    let mut observations = Vec::new();
    let mut actions = Vec::new();
    let mut interaction_packets = Vec::new();
    let mut driver_packets = Vec::new();
    let mut batch_scripts = Vec::new();
    let mut admissions = Vec::new();
    let mut capture_failures = Vec::new();
    let mut terminal_errors = Vec::new();
    let mut max_packets = 0;
    let mut packet_trace_complete = true;

    for (batch_index, batch) in BATCH_SCHEDULE.into_iter().enumerate() {
        let (account, password) = mint_account();
        let uid = ((u128::from(epoch) + batch_index as u128) % i32::MAX as u128) as i32;
        let outcome = run_live_batch(BatchRunConfig {
            batch_index,
            batch,
            account: account.clone(),
            password,
            uid,
            engine_name: "A",
            template: Arc::clone(&template),
            selected: Arc::clone(&selected),
            fixture: fixture.clone(),
        });
        if let Some(error) = outcome.terminal_error.as_ref() {
            terminal_errors.push(format!("batch {batch_index}: {error}"));
        }
        if let Some(admission) = outcome.admission {
            admissions.push(admission);
        } else {
            terminal_errors.push(format!(
                "batch {batch_index} did not establish its login session fence"
            ));
        }
        capture_failures.extend(
            outcome
                .capture_failures
                .into_iter()
                .map(|failure| format!("batch {batch_index}: {failure}")),
        );
        max_packets = max_packets.max(outcome.max_packets);
        packet_trace_complete &= outcome.packet_trace_complete;
        samples.extend(outcome.samples);
        observations.push(json!({
            "batch_index": batch_index,
            "account": account,
            "observations": outcome.observations,
        }));
        actions.push(json!({
            "batch_index": batch_index,
            "account": account,
            "actions": outcome.actions,
        }));
        interaction_packets.push(json!({
            "batch_index": batch_index,
            "account": account,
            "packets": outcome.interaction_packets,
        }));
        driver_packets.push(json!({
            "batch_index": batch_index,
            "account": account,
            "packets": outcome.driver_packets,
        }));
        batch_scripts.push(outcome.script);
    }
    if let Err(error) = validate_batch_admission(&samples, &admissions) {
        terminal_errors.push(format!("batch admission failed: {error}"));
    }
    let terminal_error = (!terminal_errors.is_empty()).then(|| terminal_errors.join("; "));
    let (k, candidate_ticks, latency_verdict) = proposed_verdict(&samples);
    let counts_complete = [Series::Held, Series::Refusal, Series::Close]
        .into_iter()
        .all(|series| {
            samples
                .iter()
                .filter(|sample| sample.series == series && sample.status == SampleStatus::Replied)
                .count()
                == series.expected()
        });
    let sample_errors = samples
        .iter()
        .filter(|sample| sample.status == SampleStatus::Error)
        .count();
    let started = batch_scripts.iter().all(|batch| batch["started"] == true);
    let completed = batch_scripts.len() == BATCH_SCHEDULE.len()
        && batch_scripts.iter().all(|batch| batch["completed"] == true);
    let player_info_clocks = batch_scripts
        .iter()
        .map(|batch| batch["player_info_clock"].clone())
        .collect::<Vec<_>>();
    let passed = terminal_error.is_none()
        && started
        && completed
        && counts_complete
        && sample_errors == 0
        && capture_failures.is_empty()
        && packet_trace_complete
        && max_packets <= MAX_DRIVER_PACKETS_PER_TICK
        && k.is_some();
    let proposed_ticks = if passed { candidate_ticks } else { None };
    let verdict = if passed {
        latency_verdict
    } else {
        "NO VERDICT: the complete probe acceptance gate failed; inspect sample errors, capture failures, batch admissions, packet trace, and terminal status".into()
    };
    let receipt = json!({
        "schema": "gatherer-shop-buy-reply-engine-a-r1",
        "request": "PROBE-BUY-REPLY",
        "model": "openai-codex/gpt-6-luna",
        "engine": {
            "name": "A",
            "host": "127.0.0.1",
            "game_port": engine_game_port,
            "http_port": engine_http_port,
            "tick_ms": 600,
            "measured_player_info_clock": player_info_clocks,
            "selected_revision": "R289",
        },
        "accounts": admissions.iter().map(|batch| &batch.account).collect::<Vec<_>>(),
        "sessions": admissions.iter().map(|batch| batch.session).collect::<Vec<_>>(),
        "navigation": {"pack": nav_pack, "flags": nav_flags},
        "fixture": {
            "keeper_config": fixture.keeper.config.as_ref(),
            "keeper_name": fixture.keeper.name.as_ref(),
            "keeper_type_id": fixture.keeper.id,
            "keeper_tile": fixture.keeper.tile,
            "access_tile": fixture.stand,
            "item_id": fixture.feather.id,
            "item_name": fixture.feather.name.as_ref(),
            "shop_inventory": "fishingshop",
            "baseline_stock": fixture.baseline_stock,
            "baseline_unit_price_gp": fixture.baseline_unit_price,
            "seed": {"account_coins": SEED_COINS, "item_stack": 0},
            "refusal_method": "Drop each fresh account's seeded coin stack, then buy one real feather at 0 gp",
        },
        "batch_schedule": BATCH_SCHEDULE,
        "batch_login_tick_limit": MAX_BATCH_LOGIN_TICKS,
        "sample_requirements": {
            "dispatch_to_held_change": HELD_SAMPLES,
            "dispatch_to_fresh_refusal": REFUSAL_SAMPLES,
            "close_modal_to_server_closes_increment": CLOSE_SAMPLES,
        },
        "samples": {
            "held": series_summary(&samples, Series::Held),
            "refusal": series_summary(&samples, Series::Refusal),
            "close": series_summary(&samples, Series::Close),
            "refusal_line_to_pack_update": refusal_pack_update_summary(&samples),
        },
        "K_max_held_or_close_edges": k,
        "proposed_BUY_REPLY_TICKS": proposed_ticks,
        "scripted_verdict": verdict,
        "sample_errors": sample_errors,
        "capture_failures": capture_failures,
        "max_driver_packets_per_PLAYER_INFO_tick": max_packets,
        "driver_packet_limit": MAX_DRIVER_PACKETS_PER_TICK,
        "driver_packet_trace_complete": packet_trace_complete,
        "decoded_interaction_packets": interaction_packets,
        "terminal_error": terminal_error,
        "script": {
            "started": started,
            "completed": completed,
            "batches": batch_scripts,
        },
        "passed": passed,
        "evidence_files": {
            "receipt": evidence.join("receipt.json").display().to_string(),
            "raw_samples": evidence.join("samples.json").display().to_string(),
            "raw_observations": evidence.join("observations.json").display().to_string(),
            "captured_actions": evidence.join("actions.json").display().to_string(),
            "driver_packets": evidence.join("driver-packets.json").display().to_string(),
        },
    });
    std::fs::write(
        evidence.join("samples.json"),
        serde_json::to_vec_pretty(&samples).expect("serialize raw probe samples"),
    )
    .expect("write raw probe samples");
    std::fs::write(
        evidence.join("observations.json"),
        serde_json::to_vec_pretty(&observations).expect("serialize raw tick observations"),
    )
    .expect("write raw tick observations");
    std::fs::write(
        evidence.join("actions.json"),
        serde_json::to_vec_pretty(&actions).expect("serialize actual host actions"),
    )
    .expect("write actual host actions");
    std::fs::write(
        evidence.join("driver-packets.json"),
        serde_json::to_vec_pretty(&driver_packets).expect("serialize actual Driver packet trace"),
    )
    .expect("write actual Driver packet trace");
    let receipt_path = evidence.join("receipt.json");
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize probe verdict"),
    )
    .expect("write machine-readable probe verdict");
    println!("buy-reply-probe-evidence={}", receipt_path.display());
    assert!(
        passed,
        "PROBE-BUY-REPLY failed or remained incomplete; evidence: {}",
        evidence.display()
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, an owned three-byte BOT_LIVE_NAME_PREFIX, configured GATHERER_GAME_PORT/GATHERER_HTTP_PORT, R289 navigation/cache inputs, and LIVE_EVIDENCE_DIR"]
fn live_buy_reply_probe_engine_l() {
    run_live_probe();
}

#[cfg(test)]
mod network_coords_tests {
    use super::*;

    fn test_fixture() -> Fixture {
        let stand = WorldTile {
            x: 10,
            z: 10,
            level: 0,
        };
        Fixture {
            feather: ItemFact {
                id: 314,
                name: "Feather".into(),
            },
            coins: ItemFact {
                id: 995,
                name: "Coins".into(),
            },
            keeper: KeeperFact {
                config: "keeper".into(),
                name: "Keeper".into(),
                id: 1,
                tile: stand,
                access: vec![stand],
            },
            stand,
            baseline_stock: 100,
            baseline_unit_price: 1,
        }
    }

    fn record_fixture_reply(
        probe: &BuyReplyProbeScript,
        session: SessionIdentity,
        ordinal: usize,
        reply_tick: u64,
    ) {
        let sample = probe.add_sample(
            Series::Held,
            ordinal,
            ordinal as u64,
            reply_tick.saturating_sub(1),
            session,
        );
        probe.trace.lock().samples[sample].dispatch_tick = Some(reply_tick.saturating_sub(1));
        probe
            .settle_reply(sample, reply_tick)
            .expect("fixture reply passes the production reply oracle");
    }

    fn drive_progress_script(
        slot: &mut script::SlotScript,
        client: &mut Client,
        snapshot: &GameSnapshot,
        tick: u64,
        here: WorldTile,
    ) {
        slot.on_game_tick(&mut script::ScriptCtx {
            driver: client,
            tick,
            here: Some((here.x, here.z, here.level)),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: script::CompiledTick::default(),
        });
    }

    fn run_fenced_observation(
        baseline: SceneObservation,
        observed: SceneObservation,
        phase: Phase,
        session_changed: bool,
    ) -> (Option<String>, SampleStatus) {
        let fixture = test_fixture();
        let trace = Arc::new(Mutex::new(ProbeTrace::default()));
        let mut probe = BuyReplyProbeScript::new("alice".into(), fixture, Arc::clone(&trace));
        probe.phase = phase;
        let mut slot = script::SlotScript::new();
        slot.start_test_script(Box::new(OwnedProbeScript::new(probe)), None)
            .expect("install the real buy-reply probe in SlotScript");
        let run = slot.native_run().expect("fenced probe run");
        let session = SessionIdentity::from(run);
        {
            let mut trace = trace.lock();
            let expected_session = if session_changed {
                SessionIdentity {
                    session: session.session + 1,
                    ..session
                }
            } else {
                session
            };
            trace.started = true;
            trace.session = Some(expected_session);
            trace.scene_fence = Some(SceneFence::new(baseline));
            trace.samples.push(Sample::new(
                0,
                Series::Held,
                1,
                "alice",
                expected_session,
                1,
                1,
            ));
        }

        let mut client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: false,
            lowmem: true,
        });
        client.ingame = true;
        client.local_player_teleport_watermark = observed.local_player_teleport_watermark;
        client.gens.invalidations = observed.rebuild_invalidation_watermark;
        client.scene_state = 2;
        client.map_build_base_x = observed.identity.map_base_x;
        client.map_build_base_z = observed.identity.map_base_z;
        client.minusedlevel = observed.identity.level;
        client.build_minusedlevel = observed.identity.level;
        client.local_player = Some(client::dash3d::ClientPlayer::at(
            observed.player_tile.x - observed.identity.map_base_x,
            observed.player_tile.z - observed.identity.map_base_z,
        ));
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: observed.identity.map_base_x,
            map_base_z: observed.identity.map_base_z,
            level: observed.identity.level,
            ..Default::default()
        });
        client.bump_gens(client::io::ServerProt::PLAYER_INFO);
        assert!(snapshot.rebuild_family(&client, api::snapshot::Family::Player));
        slot.on_game_tick(&mut script::ScriptCtx {
            driver: &mut client,
            tick: 2,
            here: Some((
                observed.player_tile.x,
                observed.player_tile.z,
                observed.identity.level,
            )),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(&snapshot),
            obj_names: None,
            compiled: script::CompiledTick::default(),
        });
        let trace = trace.lock();
        let result = (trace.failure.clone(), trace.samples[0].status);
        drop(trace);
        slot.stop();
        result
    }

    #[test]
    fn scene_fence_is_deferred_until_shop_admission() {
        let start_run = RunKey {
            slot: 1,
            run: 1,
            session: 130,
        };
        let native_session = SessionIdentity {
            slot: 1,
            run: 1,
            session: 2,
        };
        let identity = SceneIdentity {
            map_base_x: 3200,
            map_base_z: 3400,
            level: 0,
        };
        let stand = SceneObservation {
            identity,
            player_tile: WorldTile {
                x: 3210,
                z: 3410,
                level: 0,
            },
            evidence_tick: 12,
            local_player_teleport_watermark: 0,
            rebuild_invalidation_watermark: 0,
        };
        let approach = SceneObservation {
            player_tile: WorldTile {
                x: stand.player_tile.x + 1,
                ..stand.player_tile
            },
            ..stand
        };
        let mut trace = ProbeTrace::default();

        let start_run_label = format!("{start_run:?}");
        record_script_start(&mut trace, 12, start_run);
        assert_eq!(trace.session, None);
        assert_eq!(trace.script_run.as_deref(), Some(start_run_label.as_str()));
        assert_eq!(
            validate_context_observation(&mut trace, native_session, Some(stand), None),
            None,
            "session alignment before the first native context is not a sampled boundary"
        );
        assert_eq!(trace.session, Some(native_session));
        assert_eq!(trace.scene_fence, None);
        assert_eq!(
            validate_context_observation(&mut trace, native_session, Some(approach), None),
            None,
            "approach movement before an opened shop must not establish the scene fence"
        );
        record_scene_baseline(&mut trace, Some(approach))
            .expect("the first usable shop page supplies the scene baseline");
        assert_eq!(trace.scene_fence, Some(SceneFence::new(approach)));

        trace.samples.push(Sample::new(
            0,
            Series::Held,
            1,
            "alice",
            native_session,
            1,
            13,
        ));
        let changed_session = SessionIdentity {
            session: native_session.session + 1,
            ..native_session
        };
        let error =
            validate_context_observation(&mut trace, changed_session, Some(approach), Some(0))
                .expect("a post-baseline session change invalidates the active sample");
        assert!(error.contains("session changed during probe"));
        assert_eq!(trace.samples[0].status, SampleStatus::Error);
    }

    #[test]
    fn accepted_fixture_replies_keep_watchdog_alive_across_wedge() {
        let fixture = test_fixture();
        let trace = Arc::new(Mutex::new(ProbeTrace::default()));
        let probe = BuyReplyProbeScript::new("alice".into(), fixture.clone(), Arc::clone(&trace));
        let mut slot = script::SlotScript::new();
        slot.start_test_script(
            Box::new(OwnedProbeScript::progress_fixture(Arc::clone(&trace))),
            None,
        )
        .expect("install the actual owned probe adapter");
        let run = slot.native_run().expect("owned probe run");
        let session = SessionIdentity::from(run);
        {
            let mut trace = trace.lock();
            trace.started = true;
            trace.session = Some(session);
        }

        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: false,
            lowmem: true,
        });
        let tile = (fixture.stand.x, fixture.stand.z, fixture.stand.level);
        let xp = [17, 23];
        let started = Instant::now();
        assert_eq!(
            slot.feed_watchdog(started, Some(tile), &xp, false, true, &[]),
            script::watchdog::WatchdogAction::None
        );

        record_fixture_reply(&probe, session, 1, 2);
        let before_wedge = started + script::watchdog::WEDGE - Duration::from_secs(1);
        drive_progress_script(&mut slot, &mut client, &snapshot, 1, fixture.stand);
        slot.note_game_packet_tick(before_wedge);
        assert_eq!(
            slot.feed_watchdog(before_wedge, Some(tile), &xp, false, true, &[]),
            script::watchdog::WatchdogAction::None
        );

        record_fixture_reply(&probe, session, 2, 3);
        let after_wedge = started + script::watchdog::WEDGE + Duration::from_secs(1);
        drive_progress_script(&mut slot, &mut client, &snapshot, 2, fixture.stand);
        slot.note_game_packet_tick(after_wedge);
        assert_eq!(
            slot.feed_watchdog(after_wedge, Some(tile), &xp, false, true, &[]),
            script::watchdog::WatchdogAction::None,
            "the second accepted reply refreshes progress after the original wedge boundary"
        );
        assert_eq!(trace.lock().settled_replies, 2);
        assert_eq!(
            trace
                .lock()
                .samples
                .iter()
                .filter(|sample| sample.status == SampleStatus::Replied)
                .count(),
            2,
            "the next fixture sample is accepted on the same native run"
        );
        assert_eq!(slot.native_run(), Some(run));
        slot.stop();

        let empty_trace = Arc::new(Mutex::new(ProbeTrace::default()));
        let mut empty_slot = script::SlotScript::new();
        empty_slot
            .start_test_script(
                Box::new(OwnedProbeScript::progress_fixture(Arc::clone(&empty_trace))),
                None,
            )
            .expect("install the no-reply owned probe adapter");
        let empty_run = empty_slot.native_run().expect("no-reply probe run");
        let mut empty_client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: false,
            lowmem: true,
        });
        let empty_started = Instant::now();
        assert_eq!(
            empty_slot.feed_watchdog(empty_started, Some(tile), &xp, false, true, &[]),
            script::watchdog::WatchdogAction::None
        );
        drive_progress_script(
            &mut empty_slot,
            &mut empty_client,
            &snapshot,
            1,
            fixture.stand,
        );
        let empty_wedge = empty_started + script::watchdog::WEDGE + Duration::from_secs(1);
        empty_slot.note_game_packet_tick(empty_wedge);
        assert_eq!(
            empty_slot.feed_watchdog(empty_wedge, Some(tile), &xp, false, true, &[]),
            script::watchdog::WatchdogAction::Restart {
                reason: script::watchdog::RestartReason::Wedge,
            },
            "missing valid replies must not refresh the gameplay watchdog"
        );
        assert_eq!(empty_slot.native_run(), Some(empty_run));
        empty_slot.stop();
    }

    #[test]
    fn batch_schedule_and_player_scene_fence_replay() {
        // The first measured dispatch was at login-relative tick 31 (StartScript
        // was tick 12); setup must include the ticks before StartScript.
        let setup_ticks = 31_u64;
        let settlement_ticks = REFUSAL_PACK_OBSERVE_TICKS;
        // The archived Engine A series measured 191/199/199 ticks for 100
        // held samples, 310 for refusal, and 398 for close.
        let measured_series_ticks = [191_u64, 199, 199, 310, 398];
        let mut admissions = Vec::new();
        let mut samples = Vec::new();
        for (batch_index, plan) in BATCH_SCHEDULE.into_iter().enumerate() {
            let account = format!("alice{batch_index}");
            let session = SessionIdentity {
                slot: 1,
                run: 1,
                session: batch_index as u64 + 1,
            };
            let series_ticks = measured_series_ticks[batch_index];
            let login_player_info_ticks = setup_ticks + series_ticks + settlement_ticks;
            assert!(
                login_player_info_ticks <= MAX_BATCH_LOGIN_TICKS,
                "batch {batch_index} includes setup and settlement within the login bound"
            );
            admissions.push(BatchAdmission {
                plan,
                account: account.clone(),
                session,
                login_player_info_ticks,
            });
            for offset in 0..plan.samples {
                let dispatch_tick =
                    setup_ticks + series_ticks * offset as u64 / plan.samples as u64;
                let reply_tick =
                    setup_ticks + series_ticks * (offset as u64 + 1) / plan.samples as u64;
                let ordinal = plan.ordinal(offset);
                let mut sample = Sample::new(
                    batch_index,
                    plan.series,
                    ordinal,
                    &account,
                    session,
                    ordinal as u64,
                    dispatch_tick,
                );
                sample.dispatch_tick = Some(dispatch_tick);
                sample.dispatch_accepted = Some(true);
                sample
                    .mark_reply(reply_tick)
                    .expect("measured reply follows its dispatch");
                samples.push(sample);
            }
        }

        let old_one_session_ticks = setup_ticks + 591 + 310 + 398 + settlement_ticks;
        assert!(
            old_one_session_ticks > MAX_BATCH_LOGIN_TICKS,
            "the old continuous run crosses the first macro-event timer"
        );
        assert_eq!(
            samples.len(),
            HELD_SAMPLES + REFUSAL_SAMPLES + CLOSE_SAMPLES
        );
        assert!(validate_batch_admission(&samples, &admissions).is_ok());

        let mut missing = samples.clone();
        missing.retain(|sample| !(sample.batch_index == 0 && sample.ordinal == 1));
        assert!(validate_batch_admission(&missing, &admissions)
            .unwrap_err()
            .contains("has 99 samples"));

        let mut duplicate = samples.clone();
        duplicate
            .iter_mut()
            .find(|sample| sample.batch_index == 0 && sample.ordinal == 2)
            .expect("second held reply")
            .ordinal = 1;
        assert!(validate_batch_admission(&duplicate, &admissions)
            .unwrap_err()
            .contains("has 2 samples"));

        let mut crossed = samples.clone();
        crossed[0].session = admissions[1].session;
        assert!(validate_batch_admission(&crossed, &admissions)
            .unwrap_err()
            .contains("crossed its account or run/session fence"));

        let identity = SceneIdentity {
            map_base_x: 3200,
            map_base_z: 3400,
            level: 0,
        };
        let baseline = SceneObservation {
            identity,
            player_tile: WorldTile {
                x: 3210,
                z: 3410,
                level: 0,
            },
            evidence_tick: 1,
            local_player_teleport_watermark: 0,
            rebuild_invalidation_watermark: 0,
        };
        let cases = [
            (
                "one-tile walk",
                SceneObservation {
                    player_tile: WorldTile {
                        x: baseline.player_tile.x + 1,
                        ..baseline.player_tile
                    },
                    ..baseline
                },
                None,
            ),
            (
                "two-tile run on each axis",
                SceneObservation {
                    player_tile: WorldTile {
                        x: baseline.player_tile.x + 2,
                        z: baseline.player_tile.z + 2,
                        ..baseline.player_tile
                    },
                    ..baseline
                },
                None,
            ),
            (
                "one-tile local-player teleport",
                SceneObservation {
                    player_tile: WorldTile {
                        x: baseline.player_tile.x + 1,
                        ..baseline.player_tile
                    },
                    local_player_teleport_watermark: baseline.local_player_teleport_watermark + 1,
                    ..baseline
                },
                Some("local player Teleport applied"),
            ),
            (
                "same-scene map rebuild invalidation",
                SceneObservation {
                    rebuild_invalidation_watermark: baseline.rebuild_invalidation_watermark + 1,
                    ..baseline
                },
                Some("client rebuild invalidation changed"),
            ),
            (
                "scene base change",
                SceneObservation {
                    identity: SceneIdentity {
                        map_base_x: identity.map_base_x + 8,
                        ..identity
                    },
                    ..baseline
                },
                Some("player scene changed"),
            ),
            (
                "plane change",
                SceneObservation {
                    identity: SceneIdentity {
                        level: 1,
                        ..identity
                    },
                    player_tile: WorldTile {
                        level: 1,
                        ..baseline.player_tile
                    },
                    ..baseline
                },
                Some("player scene changed"),
            ),
        ];
        for (label, observed, expected_failure) in cases {
            let (failure, status) =
                run_fenced_observation(baseline, observed, Phase::AwaitHeld { sample: 0 }, false);
            if let Some(expected) = expected_failure {
                let reason = failure
                    .unwrap_or_else(|| panic!("{label} should invalidate the active sample"));
                assert!(
                    reason.contains(expected),
                    "{label} had unexpected failure: {reason}"
                );
                assert_eq!(status, SampleStatus::Error, "{label} sample status");
            } else {
                assert!(
                    failure.is_none(),
                    "{label} unexpectedly failed: {failure:?}"
                );
                assert_eq!(status, SampleStatus::Pending, "{label} sample status");
            }
        }
        let (session_failure, session_status) =
            run_fenced_observation(baseline, baseline, Phase::AwaitHeld { sample: 0 }, true);
        let session_failure = session_failure.expect("session changes invalidate the probe");
        assert!(session_failure.contains("session changed during probe"));
        assert_eq!(session_status, SampleStatus::Error);

        let (reopen_failure, _) = run_fenced_observation(
            baseline,
            cases[0].1,
            Phase::Reopen {
                next: 1,
                close_tick: 1,
                closes_after: 0,
            },
            false,
        );
        let reopen_failure = reopen_failure.expect("the fixture has no keeper for a reopened shop");
        assert!(reopen_failure.contains("the keeper is gone"));

        let incomplete = samples
            .iter()
            .filter(|sample| sample.batch_index != 4 || sample.ordinal == 1)
            .cloned()
            .collect::<Vec<_>>();
        assert!(validate_batch_admission(&incomplete, &admissions)
            .unwrap_err()
            .contains("batch 4 has 1 samples"));
    }

    #[test]
    fn keeper_candidate_uses_network_coords_without_size_fallback() {
        if !api::test_support::game_data_available() {
            return;
        }
        let rendered = WorldTile {
            x: 10,
            z: 10,
            level: 0,
        };
        let network = WorldTile {
            x: 40,
            z: 10,
            level: 0,
        };
        let npc = api::snapshot::NpcView {
            index: 1,
            r#type: Some(1),
            name: Some("Keeper".into()),
            actions: vec![Some("Trade".into())],
            tile: rendered,
            distance: 0,
            animation: 0,
            animation_frame: 0,
            pose_animation: 0,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 0,
            network,
            x: rendered.x,
            z: rendered.z,
            yaw: 0,
        };
        let here = WorldTile {
            x: 10,
            z: 10,
            level: 0,
        };

        struct InspectKeeper {
            probe: BuyReplyProbeScript,
            seen: Arc<Mutex<Option<Keeper>>>,
        }
        impl Script for InspectKeeper {
            fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
                *self.seen.lock() = self.probe.keeper(&tick.cx);
                Ok(ScriptFlow::Continue)
            }
        }

        for (origin, distance) in [(Some(here), 30), (None, i32::MAX)] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_npcs(vec![npc.clone()]);
            if let Some(network) = origin {
                snapshot.seed_local_player(api::snapshot::LocalPlayerView {
                    player: api::snapshot::PlayerView {
                        index: 0,
                        network,
                        actor: api::snapshot::ActorView {
                            name: Some("Local".into()),
                            actions: Vec::new(),
                            tile: WorldTile {
                                x: -20,
                                z: 10,
                                level: 0,
                            },
                            distance: 0,
                            animation: -1,
                            animation_frame: 0,
                            pose_animation: -1,
                            orientation: 0,
                            target_orientation: 0,
                            overhead_text: None,
                            spot_animation: -1,
                            spot_animation_stamp: 0,
                            health: 10,
                            total_health: 10,
                            face_entity: -1,
                            target: None,
                            moving: false,
                            running: false,
                            in_combat: false,
                        },
                        combat_level: 3,
                        skill_level: 3,
                        headicons: 0,
                        weapon: None,
                    },
                    energy: 100,
                    weight: 0,
                    animation_update: None,
                });
            }
            let fixture = Fixture {
                feather: ItemFact {
                    id: 314,
                    name: "Feather".into(),
                },
                coins: ItemFact {
                    id: 995,
                    name: "Coins".into(),
                },
                keeper: KeeperFact {
                    config: "keeper".into(),
                    name: "Keeper".into(),
                    id: 1,
                    tile: network,
                    access: vec![here],
                },
                stand: here,
                baseline_stock: 0,
                baseline_unit_price: 1,
            };
            let seen = Arc::new(Mutex::new(None));
            let mut slot = script::SlotScript::new();
            slot.start_test_script(
                Box::new(InspectKeeper {
                    probe: BuyReplyProbeScript::new(
                        "coords".into(),
                        fixture,
                        Arc::new(Mutex::new(ProbeTrace::default())),
                    ),
                    seen: seen.clone(),
                }),
                None,
            )
            .unwrap();
            let mut driver = Client::new(client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 0,
                cache_dir: String::new(),
                members: false,
                lowmem: true,
            });
            slot.on_game_tick(&mut script::ScriptCtx {
                driver: &mut driver,
                tick: 1,
                here: Some((here.x, here.z, here.level)),
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&snapshot),
                obj_names: None,
                compiled: script::CompiledTick::default(),
            });
            assert!(slot.take_native_action().is_none());
            let actual = seen.lock().clone().expect("the posted Trade keeper");
            let expected = shop_open::choose_keeper(
                [shop_open::Candidate::new(
                    "Keeper",
                    Some("Trade"),
                    1,
                    network,
                    0,
                    distance,
                )],
                None,
            )
            .expect("the expected network keeper");
            assert_eq!(actual, expected);
        }
    }

    const PROBE_TEST_BASE_X: i32 = 3000;
    const PROBE_TEST_BASE_Z: i32 = 3200;
    const PROBE_TEST_KEEPER_INDEX: usize = 434;

    fn gerrant_fixture() -> Fixture {
        let stand = WorldTile {
            x: 3012,
            z: 3225,
            level: 0,
        };
        Fixture {
            feather: ItemFact {
                id: 314,
                name: "Feather".into(),
            },
            coins: ItemFact {
                id: 995,
                name: "Coins".into(),
            },
            keeper: KeeperFact {
                config: "gerrant".into(),
                name: "Gerrant".into(),
                id: 558,
                tile: WorldTile {
                    x: 3013,
                    z: 3225,
                    level: 0,
                },
                access: vec![stand],
            },
            stand,
            baseline_stock: 100,
            baseline_unit_price: 1,
        }
    }

    fn probe_inventory(feathers: i32) -> Vec<api::snapshot::ItemView> {
        let mut items = vec![api::snapshot::ItemView {
            def: api::ItemDefView {
                id: 995,
                name: Some("Coins".into()),
                stackable: true,
                members: false,
                base_value: 1,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: api::snapshot::ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot: 0,
            count: 1000,
            actions: vec![Some("Drop".into())],
            component_id: -1,
        }];
        if feathers > 0 {
            items.push(api::snapshot::ItemView {
                def: api::ItemDefView {
                    id: 314,
                    name: Some("Feather".into()),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 1,
                count: feathers,
                actions: Vec::new(),
                component_id: -1,
            });
        }
        items
    }

    fn dispatch_r289_packet(client: &mut Client, ptype: i32, bytes: Vec<u8>) {
        client.psize = bytes.len() as i32;
        let mut packet = client::io::Packet::new(bytes);
        client.handle_packet(ptype, &mut packet);
        assert_eq!(packet.pos, packet.length());
    }

    fn accept_probe_interaction(
        slot: &mut script::SlotScript,
        action: &script::native::HostAction,
        run: RunKey,
        tick: u64,
    ) -> u64 {
        assert_eq!(action.run(), run);
        let authority = action.authority();
        assert!(authority.live());
        assert_eq!(authority.run(), run);
        let request_id = authority.request_id().get();
        slot.complete_native_interaction(
            &authority,
            script::native::InteractionReceipt {
                request_id,
                evidence: EvidenceStamp {
                    run,
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
        request_id
    }

    struct SlotProbeRig {
        fixture: Fixture,
        trace: Arc<Mutex<ProbeTrace>>,
        slot: script::SlotScript,
        client: Client,
        snapshot: GameSnapshot,
        run: RunKey,
        keeper_type: usize,
    }

    impl SlotProbeRig {
        fn new(batch_index: usize, batch: BatchPlan) -> Self {
            let fixture = gerrant_fixture();
            let trace = Arc::new(Mutex::new(ProbeTrace::default()));
            let probe = BuyReplyProbeScript::for_batch(
                "alice".into(),
                fixture.clone(),
                Arc::clone(&trace),
                batch_index,
                batch,
            );
            let mut slot = script::SlotScript::new();
            slot.start_test_script(Box::new(OwnedProbeScript::new(probe)), None)
                .expect("install the real buy-reply probe in SlotScript");
            let run = slot.native_run().expect("owned probe run");

            let mut client = Client::from_shared_with_revision(
                client::client::ClientConfig {
                    host: "127.0.0.1".into(),
                    port: 0,
                    cache_dir: String::new(),
                    members: false,
                    lowmem: true,
                },
                Arc::new(client::config::Cache::default()),
                Arc::new(Vec::new()),
                Vec::new(),
                client::io::ClientRevision::R289,
            );
            client.ingame = true;
            client.scene_state = 2;
            client.map_build_base_x = PROBE_TEST_BASE_X;
            client.map_build_base_z = PROBE_TEST_BASE_Z;
            client.map_build_centre_zone_x = PROBE_TEST_BASE_X / 8 + 6;
            client.map_build_centre_zone_z = PROBE_TEST_BASE_Z / 8 + 6;
            client.minusedlevel = 0;
            client.build_minusedlevel = 0;
            client.main_modal_id = -1;
            client.side_modal_id = -1;
            let keeper_type = fixture.keeper.id as usize;
            {
                let cache = Arc::get_mut(&mut client.cache).expect("sole test cache owner");
                for (id, name) in [(314, "Feather"), (995, "Coins")] {
                    let index = id as usize;
                    if cache.objs.len() <= index {
                        cache
                            .objs
                            .resize(index + 1, client::config::ObjType::default());
                    }
                    cache.objs[index] = client::config::ObjType {
                        id,
                        name: name.into(),
                        ..Default::default()
                    };
                }
                cache
                    .npcs
                    .resize_with(keeper_type + 1, client::config::NpcType::default);
                cache.npcs[keeper_type] = client::config::NpcType {
                    id: fixture.keeper.id,
                    name: "Gerrant".into(),
                    op: vec![Some("Trade".into())],
                    ..Default::default()
                };
            }

            let mut npc = client::client::ClientNpc::at(
                fixture.keeper.tile.x - PROBE_TEST_BASE_X,
                fixture.keeper.tile.z - PROBE_TEST_BASE_Z,
            );
            npc.r#type = Some(keeper_type);
            client.npc[PROBE_TEST_KEEPER_INDEX] = Some(Box::new(npc));
            client.npc_ids[0] = PROBE_TEST_KEEPER_INDEX as i32;
            client.npc_count = 1;
            client.local_player = Some(client::dash3d::ClientPlayer::at(
                fixture.stand.x - PROBE_TEST_BASE_X,
                fixture.stand.z - PROBE_TEST_BASE_Z,
            ));

            client.set_iface(
                3824,
                client::config::IfType {
                    id: 3824,
                    layer_id: 3824,
                    r#type: client::config::if_type::ComponentType::TYPE_LAYER,
                    ..Default::default()
                },
            );
            client.set_iface(
                3900,
                client::config::IfType {
                    id: 3900,
                    layer_id: 3824,
                    r#type: client::config::if_type::ComponentType::TYPE_INV,
                    iop: [
                        Some("Value".into()),
                        Some("Buy 1".into()),
                        Some("Buy 5".into()),
                        Some("Buy 10".into()),
                        None,
                    ],
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                3900,
                client::config::IfTypeMut {
                    link_obj_type: Some(vec![315, 0]),
                    link_obj_number: Some(vec![100, 0]),
                    ..Default::default()
                },
            );

            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_world(api::snapshot::WorldStateView {
                map_base_x: PROBE_TEST_BASE_X,
                map_base_z: PROBE_TEST_BASE_Z,
                level: 0,
                ..Default::default()
            });
            client.gens.player += 1;
            client.gens.player_info += 1;
            assert!(snapshot.rebuild_family(&client, api::snapshot::Family::Player));
            client.gens.npc += 1;
            client.gens.npc_info += 1;
            assert!(snapshot.rebuild_family(&client, api::snapshot::Family::Npc));
            snapshot.seed_inventory(probe_inventory(0), 28);
            client.gens.inv += 1;
            assert!(snapshot.rebuild_family(&client, api::snapshot::Family::Shop));
            assert!(!snapshot.shop().open);

            Self {
                fixture,
                trace,
                slot,
                client,
                snapshot,
                run,
                keeper_type,
            }
        }

        fn publish_player(&mut self, tick: u64, tile: WorldTile) {
            self.client.local_player = Some(client::dash3d::ClientPlayer::at(
                tile.x - PROBE_TEST_BASE_X,
                tile.z - PROBE_TEST_BASE_Z,
            ));
            self.client.gens.player += 1;
            self.client.gens.player_info += 1;
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::Player));
            self.snapshot
                .seed_tick(u32::try_from(tick).expect("test tick fits snapshot clock"));
        }

        fn run_tick(&mut self, tick: u64, tile: WorldTile) {
            self.slot.on_game_tick(&mut script::ScriptCtx {
                driver: &mut self.client,
                tick,
                here: Some((tile.x, tile.z, tile.level)),
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&self.snapshot),
                obj_names: None,
                compiled: script::CompiledTick::default(),
            });
        }

        fn tick(&mut self, tick: u64, tile: WorldTile) {
            self.publish_player(tick, tile);
            self.run_tick(tick, tile);
        }

        fn accept_trade(&mut self, tick: u64, tile: WorldTile) -> u64 {
            self.tick(tick, tile);
            let trade = self
                .slot
                .take_native_action()
                .expect("the real probe dispatches Gerrant Trade");
            assert!(matches!(
                &trade.effect,
                script::native::HostEffect::Interaction(InteractReq::Npc {
                    name,
                    action,
                    index,
                }) if name == "Gerrant" && action == "Trade" && *index == Some(434)
            ));
            accept_probe_interaction(&mut self.slot, &trade, self.run, tick)
        }

        fn accept_first_buy(&mut self, tick: u64, tile: WorldTile) -> u64 {
            self.tick(tick, tile);
            let buy = self
                .slot
                .take_native_action()
                .expect("the first measured buy is a real native action");
            assert!(matches!(
                &buy.effect,
                script::native::HostEffect::Interaction(InteractReq::ShopButton {
                    kind,
                    name,
                    id,
                    slot,
                    component,
                    chunk,
                }) if kind == "buy"
                    && name == "Feather"
                    && *id == 314
                    && *slot == 0
                    && *component == 3900
                    && *chunk == 1
            ));
            accept_probe_interaction(&mut self.slot, &buy, self.run, tick)
        }

        fn apply_same_scene_rebuild(&mut self) {
            let zone_x = u16::try_from(self.client.map_build_centre_zone_x)
                .expect("test scene center fits the R289 packet");
            let zone_z = u16::try_from(self.client.map_build_centre_zone_z)
                .expect("test scene center fits the R289 packet");
            let invalidations = self.client.gens.invalidations;
            let mut payload = Vec::with_capacity(4);
            payload.extend_from_slice(&zone_x.to_be_bytes());
            payload.extend_from_slice(&zone_z.to_be_bytes());
            dispatch_r289_packet(
                &mut self.client,
                client::io::ServerProt289::REBUILD_NORMAL,
                payload,
            );
            assert_eq!(self.client.gens.invalidations, invalidations + 1);
            assert_eq!(self.client.scene_state, 2);
            assert_eq!(self.client.map_build_base_x, PROBE_TEST_BASE_X);
            assert_eq!(self.client.map_build_base_z, PROBE_TEST_BASE_Z);
        }

        fn apply_new_map_build(&mut self, tick: u64, tile: WorldTile) {
            let zone_x = u16::try_from(self.client.map_build_centre_zone_x + 1)
                .expect("new test scene center fits the R289 packet");
            let zone_z = u16::try_from(self.client.map_build_centre_zone_z)
                .expect("test scene center fits the R289 packet");
            let invalidations = self.client.gens.invalidations;
            let mut payload = Vec::with_capacity(4);
            payload.extend_from_slice(&zone_x.to_be_bytes());
            payload.extend_from_slice(&zone_z.to_be_bytes());
            dispatch_r289_packet(
                &mut self.client,
                client::io::ServerProt289::REBUILD_NORMAL,
                payload,
            );
            assert_eq!(self.client.gens.invalidations, invalidations + 1);
            assert_eq!(self.client.scene_state, 1);
            assert_eq!(self.client.map_build_base_x, PROBE_TEST_BASE_X + 8);
            assert_eq!(self.client.map_build_base_z, PROBE_TEST_BASE_Z);

            self.client.scene_state = 2;
            self.client.local_player = Some(client::dash3d::ClientPlayer::at(
                tile.x - self.client.map_build_base_x,
                tile.z - self.client.map_build_base_z,
            ));
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::World));
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::Player));
            self.snapshot
                .seed_tick(u32::try_from(tick).expect("test tick fits snapshot clock"));
            self.run_tick(tick, tile);
        }

        fn apply_local_teleport(&mut self, tick: u64, tile: WorldTile) {
            self.client.local_player_teleport_watermark = self
                .client
                .local_player_teleport_watermark
                .saturating_add(1);
            self.tick(tick, tile);
        }

        fn refresh_keeper_generation(&mut self) {
            let mut npc = client::client::ClientNpc::at(
                self.fixture.keeper.tile.x - PROBE_TEST_BASE_X,
                self.fixture.keeper.tile.z - PROBE_TEST_BASE_Z,
            );
            npc.r#type = Some(self.keeper_type);
            self.client.npc[PROBE_TEST_KEEPER_INDEX] = Some(Box::new(npc));
            self.client.gens.npc += 1;
            self.client.gens.npc_info += 1;
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::Npc));
        }

        fn set_stock(&mut self, count: i32) {
            self.client.set_iface_mut(
                3900,
                client::config::IfTypeMut {
                    link_obj_type: Some(vec![315, 0]),
                    link_obj_number: Some(vec![count, 0]),
                    ..Default::default()
                },
            );
            self.client.gens.inv += 1;
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::Shop));
        }

        fn open_shop(&mut self, stock: i32) {
            dispatch_r289_packet(
                &mut self.client,
                client::io::ServerProt289::IF_OPENMAIN,
                3824u16.to_be_bytes().to_vec(),
            );
            self.set_stock(stock);
            assert!(self.snapshot.shop().open);
        }

        fn close_shop(&mut self) {
            dispatch_r289_packet(
                &mut self.client,
                client::io::ServerProt289::IF_CLOSE,
                Vec::new(),
            );
            self.client.gens.inv += 1;
            assert!(self
                .snapshot
                .rebuild_family(&self.client, api::snapshot::Family::Shop));
            let shop = self.snapshot.shop();
            assert!(!shop.open);
            assert_eq!(shop.server_closes, 1);
        }

        fn set_inventory(&mut self, feathers: i32) {
            self.snapshot.seed_inventory(probe_inventory(feathers), 28);
        }

        fn terminal_context_json(&self) -> Value {
            let trace = self.trace.lock();
            terminal_context(
                "alice",
                &self.fixture,
                &trace,
                TerminalObservation {
                    snapshot: &self.snapshot,
                    client_session_generation: self.client.gens.session,
                    player_info_generation: self.client.gens.player_info,
                    npc_info_generation: self.client.gens.npc_info,
                    native_status: None,
                },
            )
        }
    }

    #[test]
    fn owned_probe_survives_trade_approach_and_settles_first_real_buy() {
        let mut rig = SlotProbeRig::new(0, BATCH_SCHEDULE[0]);
        let run = rig.run;
        let start = rig.fixture.stand;

        rig.accept_trade(1, start);

        let movement_tiles = [
            WorldTile {
                x: 3014,
                z: 3225,
                level: 0,
            },
            WorldTile {
                x: 3014,
                z: 3224,
                level: 0,
            },
            WorldTile {
                x: 3012,
                z: 3224,
                level: 0,
            },
        ];
        for (tick, tile) in (2..=4).zip(movement_tiles) {
            rig.tick(tick, tile);
            let failure = rig.trace.lock().failure.clone();
            assert!(
                failure.is_none(),
                "ordinary same-scene movement failed at tick {tick}: {failure:?}"
            );
            assert!(
                !rig.snapshot.shop().open,
                "the shop remains closed during approach"
            );
            assert_eq!(rig.slot.native_run(), Some(run));
            assert!(rig.slot.take_native_action().is_none());
        }

        rig.open_shop(100);
        let admitted_tile = *movement_tiles.last().expect("three approach deltas");
        rig.tick(5, admitted_tile);
        assert!(rig.trace.lock().failure.is_none());
        assert_eq!(rig.slot.native_run(), Some(run));
        assert!(rig.slot.take_native_action().is_none());

        let buy_request_id = rig.accept_first_buy(6, admitted_tile);
        rig.set_stock(99);
        rig.set_inventory(1);
        rig.tick(7, admitted_tile);

        let terminal = rig.terminal_context_json();
        let trace = rig.trace.lock();
        assert!(trace.failure.is_none(), "{:?}", trace.failure);
        assert_eq!(trace.samples.len(), 1);
        let sample = &trace.samples[0];
        assert_eq!(sample.request_id, buy_request_id);
        assert_eq!(sample.dispatch_tick, Some(6));
        assert_eq!(sample.dispatch_accepted, Some(true));
        assert_eq!(sample.held_before, Some(0));
        assert_eq!(sample.held_after, Some(1));
        assert_eq!(sample.stock_before, Some(100));
        assert_eq!(sample.stock_after, Some(99));
        assert_eq!(sample.status, SampleStatus::Replied);
        assert_eq!(
            terminal["player_scene"]["baseline_evidence_tick"].as_u64(),
            Some(5)
        );
        drop(trace);
        rig.slot.stop();
    }

    #[test]
    fn owned_probe_preserves_scene_fence_through_modal_close_and_reopen() {
        let mut rig = SlotProbeRig::new(4, BATCH_SCHEDULE[4]);
        let run = rig.run;
        let start = rig.fixture.stand;
        rig.accept_trade(1, start);

        rig.open_shop(100);
        rig.tick(2, start);
        assert!(rig.trace.lock().failure.is_none());
        assert!(rig.slot.take_native_action().is_none());

        rig.tick(3, start);
        let close = rig
            .slot
            .take_native_action()
            .expect("the close series dispatches CloseModal");
        assert!(matches!(
            &close.effect,
            script::native::HostEffect::Interaction(InteractReq::CloseModal)
        ));
        let close_request_id = accept_probe_interaction(&mut rig.slot, &close, run, 3);
        rig.close_shop();
        rig.tick(4, start);
        let terminal = rig.terminal_context_json();
        let trace = rig.trace.lock();
        assert!(trace.failure.is_none(), "{:?}", trace.failure);
        assert_eq!(trace.samples.len(), 1);
        assert_eq!(trace.samples[0].request_id, close_request_id);
        assert_eq!(trace.samples[0].dispatch_tick, Some(3));
        assert_eq!(trace.samples[0].status, SampleStatus::Replied);
        assert_eq!(
            terminal["player_scene"]["baseline_evidence_tick"].as_u64(),
            Some(2)
        );
        drop(trace);

        let reopened_step = WorldTile {
            x: 3012,
            z: 3224,
            level: 0,
        };
        rig.publish_player(5, reopened_step);
        rig.refresh_keeper_generation();
        rig.run_tick(5, reopened_step);
        assert!(rig.trace.lock().failure.is_none());
        assert_eq!(rig.slot.native_run(), Some(run));
        let reopen_trade = rig
            .slot
            .take_native_action()
            .expect("a new actor generation remains available for Trade");
        assert!(matches!(
            &reopen_trade.effect,
            script::native::HostEffect::Interaction(InteractReq::Npc {
                name,
                action,
                index,
            }) if name == "Gerrant" && action == "Trade" && *index == Some(434)
        ));
        accept_probe_interaction(&mut rig.slot, &reopen_trade, run, 5);

        rig.publish_player(6, reopened_step);
        rig.refresh_keeper_generation();
        rig.run_tick(6, reopened_step);
        assert!(rig.trace.lock().failure.is_none());
        assert_eq!(rig.slot.native_run(), Some(run));
        assert!(rig.slot.take_native_action().is_none());

        rig.open_shop(100);
        rig.publish_player(7, reopened_step);
        rig.refresh_keeper_generation();
        rig.run_tick(7, reopened_step);
        let terminal = rig.terminal_context_json();
        let trace = rig.trace.lock();
        assert!(trace.failure.is_none(), "{:?}", trace.failure);
        assert_eq!(trace.samples.len(), 1);
        assert_eq!(trace.samples[0].status, SampleStatus::Replied);
        assert_eq!(
            terminal["player_scene"]["baseline_evidence_tick"].as_u64(),
            Some(2)
        );
        drop(trace);
        assert!(rig.slot.take_native_action().is_none());
        rig.slot.stop();
    }

    #[test]
    fn owned_probe_rejects_post_dispatch_scene_and_teleport_witnesses() {
        #[derive(Clone, Copy)]
        enum Witness {
            LocalTeleport,
            SameSceneRebuild,
            NewMapBuild,
        }
        for (witness, failure, event) in [
            (
                "one-tile local-player Teleport",
                "local player Teleport applied",
                Witness::LocalTeleport,
            ),
            (
                "same-scene R289 REBUILD_NORMAL",
                "client rebuild invalidation changed",
                Witness::SameSceneRebuild,
            ),
            (
                "new-map R289 REBUILD_NORMAL",
                "client rebuild invalidation changed",
                Witness::NewMapBuild,
            ),
        ] {
            let mut rig = SlotProbeRig::new(0, BATCH_SCHEDULE[0]);
            let start = rig.fixture.stand;
            assert_eq!((start.x, start.z, start.level), (3012, 3225, 0));
            rig.accept_trade(1, start);
            rig.open_shop(100);
            rig.tick(2, start);
            assert!(rig.trace.lock().failure.is_none(), "{witness}");
            assert!(rig.slot.take_native_action().is_none(), "{witness}");

            let request_id = rig.accept_first_buy(3, start);
            rig.tick(4, start);
            assert!(rig.trace.lock().failure.is_none(), "{witness}");
            assert_eq!(rig.slot.native_run(), Some(rig.run), "{witness}");
            assert!(rig.slot.take_native_action().is_none(), "{witness}");
            {
                let trace = rig.trace.lock();
                assert_eq!(trace.samples.len(), 1, "{witness}");
                assert_eq!(trace.samples[0].dispatch_tick, Some(3), "{witness}");
                assert_eq!(trace.samples[0].dispatch_accepted, Some(true), "{witness}");
                assert_eq!(trace.samples[0].status, SampleStatus::Pending, "{witness}");
            }
            rig.set_stock(99);
            rig.set_inventory(1);
            match event {
                Witness::LocalTeleport => rig.apply_local_teleport(
                    5,
                    WorldTile {
                        x: start.x + 1,
                        z: start.z,
                        level: start.level,
                    },
                ),
                Witness::SameSceneRebuild => {
                    rig.apply_same_scene_rebuild();
                    rig.tick(5, start);
                }
                Witness::NewMapBuild => rig.apply_new_map_build(5, start),
            }
            let expected_map_base_x = if matches!(event, Witness::NewMapBuild) {
                PROBE_TEST_BASE_X + 8
            } else {
                PROBE_TEST_BASE_X
            };
            assert_eq!(
                (rig.client.map_build_base_x, rig.client.map_build_base_z),
                (expected_map_base_x, PROBE_TEST_BASE_Z),
                "{witness} produced an unexpected map build",
            );

            let trace = rig.trace.lock();
            assert!(
                trace
                    .failure
                    .as_deref()
                    .is_some_and(|message| message.contains(failure)),
                "{witness}: {:?}",
                trace.failure,
            );
            assert_eq!(trace.samples.len(), 1, "{witness}");
            let sample = &trace.samples[0];
            assert_eq!(sample.request_id, request_id, "{witness}");
            assert_eq!(sample.dispatch_tick, Some(3), "{witness}");
            assert_eq!(sample.dispatch_accepted, Some(true), "{witness}");
            assert_eq!(sample.status, SampleStatus::Error, "{witness}");
            assert_eq!(trace.settled_replies, 0, "{witness}");
            drop(trace);
            rig.slot.stop();
        }
    }
}

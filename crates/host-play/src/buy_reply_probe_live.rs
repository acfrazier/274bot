//! PROBE-BUY-REPLY on Engine L. This ignored live cell measures native
//! interaction dispatches against Gerrant's real R289 shop. It does not use
//! a script timer or a local modal clear as a reply.
//!
//! One fresh, owned account runs all three series. It receives coins through
//! the account-only scenario seed, buys 300 real feathers one at a time, drops
//! its own coin stack, requests 100 deterministic zero-coin refusals, then
//! measures 100 `CloseModal` requests through `ShopView.server_closes`.
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
use super::{run_with_template, ProfileOptions, ScriptStartHandle};

const ENGINE_L_GAME_PORT: u16 = 47_594;
const ENGINE_L_HTTP_PORT: u16 = 3_080;
const REFERENCE_NAV_PACK: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack";
const REFERENCE_NAV_FLAGS: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navflags";
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
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_CAPTURE_ROWS: usize = 100_000;
const SEED_ITEMS: &[(&str, i32)] = &[(COINS_ALIAS, SEED_COINS)];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
        series: Series,
        ordinal: usize,
        account: &str,
        session: SessionIdentity,
        request_id: u64,
        request_tick: u64,
    ) -> Self {
        Self {
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

#[derive(Default)]
struct ProbeTrace {
    started: bool,
    start_tick: Option<u64>,
    session: Option<SessionIdentity>,
    script_run: Option<String>,
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
        .unwrap_or_else(|| PathBuf::from(REFERENCE_NAV_PACK));
    let nav_flags = std::env::var_os("GATHERER_NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(REFERENCE_NAV_FLAGS));
    for path in [&nav_pack, &nav_flags] {
        if !path.is_file() {
            return Err(format!(
                "required R289 navigation input is absent: {}",
                path.display()
            ));
        }
    }
    Ok(ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        port: Some(ENGINE_L_GAME_PORT),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(ENGINE_L_HTTP_PORT),
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
        "buy_reply_probe_engine_l",
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
    phase: Phase,
    last_emit_tick: Option<u64>,
    open: OpenCore,
    open_walk: Option<Walk>,
    open_trade: Option<PendingOpenTrade>,
    open_seen_trade: bool,
    keeper_index: Option<i32>,
    reopening_after_coin_drop: bool,
}

impl BuyReplyProbeScript {
    fn new(account: String, fixture: Fixture, trace: Arc<Mutex<ProbeTrace>>) -> Self {
        Self {
            account,
            fixture,
            trace,
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
            .map(|npc| {
                shop_open::Candidate::new(
                    npc.name
                        .as_deref()
                        .unwrap_or(self.fixture.keeper.name.as_ref()),
                    npc.actions
                        .iter()
                        .flatten()
                        .find(|action| action.eq_ignore_ascii_case("Trade"))
                        .map(String::as_str),
                    i32::try_from(npc.index).unwrap_or(-1),
                    if npc.size >= 1 { npc.network } else { npc.tile },
                    npc.size,
                    npc.distance,
                )
            });
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
            Decision::Opened => Ok(true),
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
                                Phase::Held { next: 0 }
                            };
                            ScriptFlow::Continue
                        }
                        Ok(false) => ScriptFlow::Continue,
                        Err(error) => self.fail(error, None),
                    }
                }
            }
            Phase::Held { next } => {
                if next >= HELD_SAMPLES {
                    self.phase = Phase::DropCoins;
                    ScriptFlow::Continue
                } else {
                    let shop_open = cx.snapshot().shop().is_some_and(|shop| shop.value.open);
                    let held = held_count(cx, self.fixture.feather.id);
                    let coins = held_count(cx, self.fixture.coins.id);
                    let Some((slot, component, stock)) = self.stock_row(cx) else {
                        return Ok(
                            self.fail("Gerrant's real feather stock row is unavailable", None)
                        );
                    };
                    if !shop_open {
                        self.fail("shop page closed during the held-change series", None)
                    } else if stock < (HELD_SAMPLES - next + 1) as i32 {
                        self.fail(
                            format!(
                                "Gerrant has {stock} feather stock for {} remaining held buys plus one zero-coin refusal",
                                HELD_SAMPLES - next
                            ),
                            None,
                        )
                    } else if held != Some(next as i32) {
                        self.fail(
                            format!(
                                "held series expected {next} feathers before sample {}, observed {held:?}",
                                next + 1
                            ),
                            None,
                        )
                    } else if coins.is_none_or(|count| count <= 0) {
                        self.fail(
                            "account coin seed was exhausted before 300 held samples",
                            None,
                        )
                    } else {
                        match self.begin_sample(
                            cx,
                            Series::Held,
                            next + 1,
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
                                if let Err(error) = row.mark_reply(now) {
                                    drop(trace);
                                    self.sample_error_flow(sample, error)
                                } else {
                                    self.phase = Phase::Held { next: row.ordinal };
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
                if next >= REFUSAL_SAMPLES {
                    self.phase = Phase::Close { next: 0 };
                    ScriptFlow::Continue
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
                            next + 1,
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
                                } else if let Err(error) = row.mark_reply(now) {
                                    row.mark_error(error.clone());
                                    drop(trace);
                                    self.fail(error, Some(sample))
                                } else {
                                    self.phase = Phase::RefusalObserve {
                                        sample,
                                        line_tick: now,
                                        held_before: row.held_before.unwrap_or_default(),
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
                    self.trace.lock().samples[sample].refusal_pack_observed_through_tick =
                        Some(now);
                    let next = self.sample(sample).ordinal;
                    self.phase = Phase::Refusal { next };
                    ScriptFlow::Continue
                } else {
                    ScriptFlow::Continue
                }
            }
            Phase::Close { next } => {
                if next >= CLOSE_SAMPLES {
                    let mut trace = self.trace.lock();
                    trace.completed = true;
                    trace.complete_tick = Some(now);
                    self.phase = Phase::Done;
                    ScriptFlow::Complete
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
                            next + 1,
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
                            if let Err(error) = row.mark_reply(now) {
                                row.mark_error(error.clone());
                                drop(trace);
                                self.fail(error, Some(sample))
                            } else if count != 1 {
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
                                self.reset_open();
                                self.phase = if ordinal == CLOSE_SAMPLES {
                                    Phase::Close { next: ordinal }
                                } else {
                                    Phase::Reopen {
                                        next: ordinal,
                                        close_tick: now,
                                        closes_after: after,
                                    }
                                };
                                ScriptFlow::Continue
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
}

impl OwnedProbeScript {
    fn new(probe: BuyReplyProbeScript) -> Self {
        Self {
            trace: Arc::clone(&probe.trace),
            pending: Some(probe),
            action: None,
        }
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

struct LiveState {
    account: String,
    fixture: Fixture,
    selected: Arc<SelectedGameData>,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    start_context: Option<ScriptStartHandle>,
    trace: Arc<Mutex<ProbeTrace>>,
    started_at: Option<Instant>,
    started: bool,
    packet_counts: DriverPacketCounts,
}

impl LiveState {
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
                        "Engine L PLAYER_INFO median {median:?} ms is outside 600 ms +/-30 ms; no probe Start was sent"
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
            let script = BuyReplyProbeScript::new(
                self.account.clone(),
                self.fixture.clone(),
                Arc::clone(&self.trace),
            );
            match handle.start_test_script(
                &self.account,
                Box::new(OwnedProbeScript::new(script)),
                Some(Arc::clone(&self.selected)),
            ) {
                Ok(run) => {
                    {
                        let mut trace = self.trace.lock();
                        trace.started = true;
                        trace.start_tick = Some(u64::from(self.snapshot.tick()));
                        trace.session = Some(SessionIdentity::from(run));
                        trace.script_run = Some(format!("{run:?}"));
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
            return;
        }
        self.runner.tick_with_hold(client, hold);
        if self.started {
            self.packet_counts.observe(client, self.snapshot.tick());
        }
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

fn run_live_probe() {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "probe requires LIVE=1"
    );
    assert_eq!(
        std::env::var("BOT_CPU").as_deref(),
        Ok("1"),
        "Engine L probe requires BOT_CPU=1"
    );
    let (account, password) = mint_account();
    let evidence_root = std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .expect("buy reply probe requires LIVE_EVIDENCE_DIR");
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let evidence = evidence_root.join(format!("buy-reply-probe-engine-l-{account}-{epoch}"));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create per-account probe evidence directory");

    let options = profile_options(&scratch)
        .unwrap_or_else(|error| panic!("buy reply probe prerequisites: {error}"));
    let nav_pack = options.nav_pack.clone().expect("selected navigation pack");
    let nav_flags = options
        .nav_flags
        .clone()
        .expect("selected navigation flags");
    let template = options
        .resolve(None)
        .expect("resolve Engine L R289 profile")
        .prepare_template()
        .expect("prepare isolated Engine L R289 client template");
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
    let trace = Arc::new(Mutex::new(ProbeTrace::default()));
    let capture = Arc::new(StdMutex::new(CombatCapture::default()));
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let mut runner = ScenarioRunner::with_world(
        probe_scenario(fixture.stand, Arc::clone(&trace)),
        template.world(),
    );
    let names = vec![account.clone()];
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let frame = FrameBuf::new();
    let frame_state = Arc::new(Mutex::new(LiveState {
        account: account.clone(),
        fixture: fixture.clone(),
        selected: Arc::clone(&selected),
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_context: None,
        trace: Arc::clone(&trace),
        started_at: None,
        started: false,
        packet_counts: DriverPacketCounts::new(),
    }));
    let state = Arc::clone(&frame_state);
    let frame_account = account.clone();
    let uid = (u128::from(epoch) % i32::MAX as u128) as i32;
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
    .expect("connect to forwarded Engine L");
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
    loop {
        if let Some(status) = play.script_native_status(&account) {
            combat_proof::record_status(&account, &status, None);
        }
        let (status, started_at, completed, failure) = {
            let state = frame_state.lock();
            let trace = trace.lock();
            (
                format!("{:?}", state.runner.status()),
                state.started_at,
                trace.completed,
                trace.failure.clone(),
            )
        };
        if let Some(reason) = failure {
            terminal_error = Some(reason);
            break;
        }
        if completed {
            if status == "Passed" {
                break;
            }
            if let Some(started_at) = started_at {
                if started_at.elapsed() >= CELL_DEADLINE {
                    terminal_error = Some(format!(
                        "probe did not pass its scenario completion gate within {} seconds",
                        CELL_DEADLINE.as_secs()
                    ));
                    break;
                }
            }
        } else if let Some(started_at) = started_at {
            if started_at.elapsed() >= CELL_DEADLINE {
                terminal_error = Some(format!(
                    "probe exceeded its {} second harness safety deadline",
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
        if status.starts_with("Failed") {
            terminal_error = Some(format!("ScenarioRunner ended with {status}"));
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    play.script_stop(&account);

    let (mut samples, observations, script_trace) = {
        let mut trace = trace.lock();
        if let Some(error) = terminal_error.as_deref() {
            trace.fail(error.to_owned());
        }
        let mut samples = trace.samples.clone();
        mark_unfinished_samples(
            &mut samples,
            "harness ended before this dispatch produced a reply",
        );
        let script_trace = json!({
            "started": trace.started,
            "start_tick": trace.start_tick,
            "session": trace.session,
            "script_run": trace.script_run,
            "complete_tick": trace.complete_tick,
            "completed": trace.completed,
            "failure": trace.failure,
            "player_info_clock": trace.player_info_clock,
        });
        (samples, trace.observations.clone(), script_trace)
    };
    let actions = capture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .actions
        .clone();
    let interactions = DriverPacketTrace::from_actions(&actions);
    let (interaction_packet_rows, _, _) = interactions.value();
    let state = frame_state.lock();
    let (packet_rows, max_packets, packet_trace_complete) = state.packet_counts.value();
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
    let passed = terminal_error.is_none()
        && script_trace["started"] == true
        && script_trace["completed"] == true
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
        "NO VERDICT: the complete probe acceptance gate failed; inspect sample errors, capture failures, packet trace, and terminal status".into()
    };
    let receipt = json!({
        "schema": "gatherer-shop-buy-reply-engine-l-r1",
        "request": "PROBE-BUY-REPLY",
        "model": "openai-codex/gpt-6-luna",
        "engine": {
            "name": "L",
            "host": "127.0.0.1",
            "game_port": ENGINE_L_GAME_PORT,
            "http_port": ENGINE_L_HTTP_PORT,
            "tick_ms": 600,
            "measured_player_info_clock": script_trace["player_info_clock"],
            "selected_revision": "R289",
        },
        "account": account,
        "session": script_trace["session"],
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
            "refusal_method": "Drop the same fresh account's seeded coin stack, then buy one real feather at 0 gp",
        },
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
        "decoded_interaction_packets": interaction_packet_rows,
        "terminal_error": terminal_error,
        "script": script_trace,
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
        serde_json::to_vec_pretty(&packet_rows).expect("serialize actual Driver packet trace"),
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
#[ignore = "requires LIVE=1, BOT_CPU=1, an owned three-byte BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080 (600 ms), R289 navigation/cache inputs, and LIVE_EVIDENCE_DIR"]
fn live_buy_reply_probe_engine_l() {
    run_live_probe();
}

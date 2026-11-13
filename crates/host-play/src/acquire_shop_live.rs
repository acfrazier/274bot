//! A4 core acquisition through the real R289 shops and native host observer.
//!
//! These ignored cells seed only a fresh account's pack and position. The
//! reference R289 world binds its selected-content shop keepers; after Start
//! the test script is a thin wrapper over the real `AcquireRun`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, SelectedGameData};
use api::named_banks::NamedBankFacts;
use api::selected::ClientRevision;
use api::service_pick::ShopKeeperFacts;
use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use client::client::Client;
use host::{FrameBuf, Pump};
use parking_lot::Mutex;
use scenario::{self, Proof, RunnerStatus, Scenario, ScenarioRunner, Step, StepKind, Wait};
use script::acquire::plan;
use script::acquire::run::AcquireRun;
use script::acquire::{
    AcquireOutcome, AcquireRequest, BankChoice, BankStage, CloseBank, CoinPolicy, Need, NeedKind,
    OpenGate, Presence, SourceSet, VendorChoice, WorldFacts,
};
use script::native::{NativeTick, Script, ScriptFailure, ScriptFlow, WalkOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

use super::combat_proof::{
    self, CaptureRegistration, CombatCapture, DriverPacketCounts, DriverPacketTrace,
    MAX_DRIVER_PACKETS_PER_TICK,
};
use super::{run_with_template, ProfileOptions, ScriptStartHandle};

const ENGINE_L_GAME_PORT: u16 = 47_594;
const ENGINE_L_HTTP_PORT: u16 = 3_080;
const REFERENCE_NAV_PACK: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack";
const REFERENCE_NAV_FLAGS: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navflags";
const SEED_COINS: i32 = 100_000;
const SEED_BAIT: i32 = 5;
const HARPOON_TARGET: i32 = 1;
const BAIT_TARGET: i32 = 15;
const PREPARATION_DEADLINE: Duration = Duration::from_secs(600);
const CELL_DEADLINE: Duration = Duration::from_secs(180);
const TERMINAL_CAPTURE_GRACE: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// R289 selected-content Harry placement in Catherby.
const CATHERBY_HARRY: WorldTile = WorldTile {
    x: 2834,
    z: 3445,
    level: 0,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellKind {
    Explicit,
    Nearest,
}

impl CellKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Explicit => "acquire_shop_explicit_live",
            Self::Nearest => "acquire_shop_nearest_live",
        }
    }

    const fn vendor_name(self) -> &'static str {
        match self {
            Self::Explicit => "Explicit Gerrant",
            Self::Nearest => "Nearest",
        }
    }
}

#[derive(Clone)]
struct ItemFact {
    id: i32,
    name: Arc<str>,
}

#[derive(Clone)]
struct KeeperFact {
    row_index: u16,
    config: Arc<str>,
    name: Arc<str>,
    id: i32,
    tile: WorldTile,
    access: Vec<WorldTile>,
}

#[derive(Clone)]
struct Fixture {
    kind: CellKind,
    harpoon: ItemFact,
    bait: ItemFact,
    coins: ItemFact,
    gerrant: KeeperFact,
    harry: KeeperFact,
    stand: WorldTile,
}

impl Fixture {
    fn vendor_choice(&self) -> VendorChoice {
        match self.kind {
            CellKind::Explicit => VendorChoice::Explicit {
                keeper_config: Arc::clone(&self.gerrant.config),
                anchor: Some(self.gerrant.tile),
            },
            CellKind::Nearest => VendorChoice::Nearest,
        }
    }

    fn facts(
        &self,
        keepers: &ShopKeeperFacts,
        data: &SelectedGameData,
        members: Option<bool>,
    ) -> Value {
        let eligible = members.map(|members| {
            json!({
                "harpoon": eligible_keeper_rows(keepers, data, self.harpoon.id, members),
                "bait": eligible_keeper_rows(keepers, data, self.bait.id, members),
            })
        });
        json!({
            "cell": self.kind.name(),
            "vendor_input": self.kind.vendor_name(),
            "start_tile": self.stand,
            "items": {
                "harpoon": {"id": self.harpoon.id, "name": self.harpoon.name.as_ref()},
                "bait": {"id": self.bait.id, "name": self.bait.name.as_ref()},
                "coins": {"id": self.coins.id, "name": self.coins.name.as_ref()},
            },
            "selected_keepers": {
                "gerrant": keeper_value(&self.gerrant),
                "harry_catherby": keeper_value(&self.harry),
            },
            "gerrant_item_rows_present": {
                "harpoon": api::shop_facts::vendor_by_keeper_config(data, "gerrant", self.harpoon.id).ok().flatten().is_some(),
                "bait": api::shop_facts::vendor_by_keeper_config(data, "gerrant", self.bait.id).ok().flatten().is_some(),
            },
            "account_membership": members,
            "eligible_nearest_rows": eligible,
        })
    }
}

fn keeper_value(fact: &KeeperFact) -> Value {
    json!({
        "row_index": fact.row_index,
        "config": fact.config.as_ref(),
        "name": fact.name.as_ref(),
        "keeper_id": fact.id,
        "placement_tile": fact.tile,
        "access_tiles": fact.access,
    })
}

fn eligible_keeper_rows(
    keepers: &ShopKeeperFacts,
    data: &SelectedGameData,
    item_id: i32,
    members: bool,
) -> Vec<Value> {
    keepers
        .candidates_for(data, item_id, members)
        .filter_map(|candidate| {
            let row = keepers.rows().get(usize::from(candidate.index))?;
            Some(json!({
                "row_index": candidate.index,
                "config": row.keeper.keeper_config,
                "name": row.keeper.keeper_name,
                "keeper_id": row.keeper.keeper_id,
                "placement_tile": row.tile,
                "access_tiles": candidate.access,
            }))
        })
        .collect()
}

fn content_keeper(
    keepers: &ShopKeeperFacts,
    config: &str,
    tile: Option<WorldTile>,
) -> Result<KeeperFact, String> {
    let row = keepers
        .rows()
        .iter()
        .find(|row| {
            row.keeper.keeper_config == config
                && tile.is_none_or(|wanted| row.tile == wanted)
                && !row.access().is_empty()
        })
        .ok_or_else(|| format!("R289 selected shop facts have no accessible {config} placement"))?;
    Ok(KeeperFact {
        row_index: row.index,
        config: Arc::from(row.keeper.keeper_config),
        name: Arc::from(row.keeper.keeper_name),
        id: row.keeper.keeper_id,
        tile: row.tile,
        access: row.access().to_vec(),
    })
}

fn fixture(
    kind: CellKind,
    data: &SelectedGameData,
    keepers: &ShopKeeperFacts,
) -> Result<Fixture, String> {
    let item = |alias: &str| {
        let row = data
            .item_by_alias(alias)
            .ok_or_else(|| format!("R289 selected item alias {alias:?} is absent"))?;
        let name = row
            .name
            .as_deref()
            .ok_or_else(|| format!("R289 selected item alias {alias:?} has no name"))?;
        Ok::<_, String>(ItemFact {
            id: row.id,
            name: Arc::from(name),
        })
    };
    let harpoon = item("harpoon")?;
    let bait = item("fishing_bait")?;
    let coins = item("coins")?;
    let gerrant = content_keeper(keepers, "gerrant", None)?;
    let harry = content_keeper(keepers, "harry", Some(CATHERBY_HARRY))?;
    if api::shop_facts::vendor_by_keeper_config(data, "gerrant", harpoon.id)
        .ok()
        .flatten()
        .is_none()
        || api::shop_facts::vendor_by_keeper_config(data, "gerrant", bait.id)
            .ok()
            .flatten()
            .is_none()
        || api::shop_facts::vendor_by_keeper_config(data, "harry", harpoon.id)
            .ok()
            .flatten()
            .is_none()
        || api::shop_facts::vendor_by_keeper_config(data, "harry", bait.id)
            .ok()
            .flatten()
            .is_none()
    {
        return Err(
            "R289 pinned shop records do not contain the required Gerrant/Harry items".into(),
        );
    }
    let stand = match kind {
        CellKind::Explicit => *gerrant
            .access
            .first()
            .ok_or_else(|| "Gerrant has no selected nav access tile".to_owned())?,
        CellKind::Nearest => *harry
            .access
            .first()
            .ok_or_else(|| "Harry has no selected nav access tile".to_owned())?,
    };
    Ok(Fixture {
        kind,
        harpoon,
        bait,
        coins,
        gerrant,
        harry,
        stand,
    })
}

fn profile_options(scratch: &Path) -> Result<ProfileOptions, String> {
    let engine_dir = std::env::var_os("BOT_ENGINE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "acquire shop live cells require BOT_ENGINE_DIR".to_owned())?;
    let cache_dir = std::env::var_os("BOT_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .ok_or_else(|| {
            "acquire shop live cells require a per-run BOT_CACHE_DIR clone".to_owned()
        })?;
    let nav_pack = std::env::var_os("GATHERER_NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(REFERENCE_NAV_PACK));
    let nav_flags = std::env::var_os("GATHERER_NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(REFERENCE_NAV_FLAGS));
    for path in [&nav_pack, &nav_flags] {
        if !path.is_file() {
            return Err(format!(
                "required R289 reference navigation file is absent: {}",
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

fn seed_items() -> &'static [(&'static str, i32)] {
    &[("coins", SEED_COINS), ("fishing_bait", SEED_BAIT)]
}

fn scenario(kind: CellKind, stand: WorldTile, trace: Arc<Mutex<ShopTrace>>) -> Scenario {
    let mut scenario = scenario::quester_stage(
        kind.name(),
        "Cook's Assistant",
        "cook",
        0,
        seed_items(),
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
        name: "await real AcquireRun receipt",
        kind: StepKind::Await {
            evidence: "AcquireOutcome::Done",
            ready: Box::new(move |_| {
                let trace = trace.lock();
                trace.outcome.is_some()
            }),
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

#[derive(Default)]
struct ShopTrace {
    started: bool,
    start_tick: Option<u32>,
    start_snapshot: Option<Value>,
    start_members: Option<bool>,
    start_server_closes: Option<u64>,
    script_run: Option<String>,
    request: Option<Value>,
    outcome: Option<Value>,
    completion_tick: Option<u64>,
    completed_at: Option<Instant>,
    failure: Option<String>,
    shop_frames: Vec<Value>,
    last_shop_frame_signature: Option<String>,
}

impl ShopTrace {
    fn fail(&mut self, reason: impl Into<String>) {
        if self.failure.is_none() {
            self.failure = Some(reason.into());
        }
    }
}

struct LiveState {
    account: String,
    kind: CellKind,
    fixture: Fixture,
    selected: Arc<SelectedGameData>,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    frame: Arc<FrameBuf>,
    start_pixels: Option<Vec<u32>>,
    start_context: Option<(ScriptStartHandle, Arc<NamedBankFacts>)>,
    trace: Arc<Mutex<ShopTrace>>,
    started_at: Option<Instant>,
    started: bool,
    packet_trace: DriverPacketCounts,
}

impl LiveState {
    fn start_error(&self) -> Option<String> {
        if !self.snapshot.ingame() || self.snapshot.scene_state() != 2 {
            return Some("StartScript arrived before ingame scene_state=2".into());
        }
        if self.snapshot.base().is_none() {
            return Some("StartScript arrived without a built world base".into());
        }
        if self.snapshot.tile()
            != Some((
                self.fixture.stand.x,
                self.fixture.stand.z,
                self.fixture.stand.level,
            ))
        {
            return Some(format!(
                "seeded start tile {:?} did not equal selected access tile {:?}",
                self.snapshot.tile(),
                self.fixture.stand
            ));
        }
        if self.snapshot.inv_count(self.fixture.coins.id) < SEED_COINS {
            return Some(format!(
                "PACK coins below seed budget: {}",
                self.snapshot.inv_count(self.fixture.coins.id)
            ));
        }
        if self.snapshot.inv_count(self.fixture.bait.id) != SEED_BAIT {
            return Some(format!("seeded fishing bait was not exactly {SEED_BAIT}"));
        }
        if self.snapshot.inv_count(self.fixture.harpoon.id) != 0 {
            return Some("fresh pack unexpectedly already held a harpoon".into());
        }
        if self.snapshot.shop().open {
            return Some("shop modal was already open at StartScript".into());
        }
        None
    }

    fn record_shop_frame(&mut self) {
        let snapshot = &self.snapshot;
        let row = json!({
            "tick": snapshot.tick(),
            "ingame": snapshot.ingame(),
            "scene_state": snapshot.scene_state(),
            "tile": snapshot.tile(),
            "members": snapshot.world().members,
            "inventory": snapshot.inventory().iter().filter(|item| item.container == api::snapshot::ItemContainer::Inventory).map(item_value).collect::<Vec<_>>(),
            "held": {
                "harpoon": snapshot.inv_count(self.fixture.harpoon.id),
                "bait": snapshot.inv_count(self.fixture.bait.id),
                "coins": snapshot.inv_count(self.fixture.coins.id),
            },
            "shop": {
                "open": snapshot.shop().open,
                "server_closes": snapshot.shop().server_closes,
                "player_available": snapshot.shop().player_available,
                "stock": snapshot.shop().stock.iter().map(item_value).collect::<Vec<_>>(),
                "player": snapshot.shop().player.iter().map(item_value).collect::<Vec<_>>(),
            },
        });
        let signature = serde_json::to_string(&row).unwrap_or_default();
        let mut trace = self.trace.lock();
        if trace.last_shop_frame_signature.as_deref() != Some(signature.as_str()) {
            trace.last_shop_frame_signature = Some(signature);
            trace.shop_frames.push(row);
        }
    }

    fn frame(&mut self, client: &mut Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if self.started {
            self.packet_trace.observe(client, self.snapshot.tick());
            self.record_shop_frame();
        }

        if self.runner.on_start_script() && !self.started {
            if !self.snapshot.ingame() || self.snapshot.scene_state() != 2 {
                return;
            }
            combat_proof::record_start_baseline(&self.account, &self.snapshot);
            let baseline = combat_proof::snapshot_facts(&self.snapshot, None);
            if let Some(reason) = self.start_error() {
                let mut trace = self.trace.lock();
                trace.start_tick = Some(self.snapshot.tick());
                trace.start_snapshot = Some(baseline);
                trace.start_members = Some(self.snapshot.world().members);
                trace.start_server_closes = Some(self.snapshot.shop().server_closes);
                trace.fail(reason);
                return;
            }
            let Some((handle, banks)) = self.start_context.as_ref() else {
                self.trace
                    .lock()
                    .fail("StartScript reached before native context was installed");
                return;
            };
            let request = AcquireShopScript::new(
                self.kind,
                self.fixture.clone(),
                Arc::clone(&self.selected),
                Arc::clone(banks),
                Arc::clone(&self.trace),
            );
            match handle.start_test_script(
                &self.account,
                Box::new(request),
                Some(Arc::clone(&self.selected)),
            ) {
                Ok(run) => {
                    {
                        let mut trace = self.trace.lock();
                        trace.started = true;
                        trace.start_tick = Some(self.snapshot.tick());
                        trace.start_snapshot = Some(baseline);
                        trace.start_members = Some(self.snapshot.world().members);
                        trace.start_server_closes = Some(self.snapshot.shop().server_closes);
                        trace.script_run = Some(format!("{run:?}"));
                    }
                    self.start_pixels = Some(self.frame.snapshot());
                    self.started = true;
                    self.started_at = Some(Instant::now());
                    self.packet_trace.begin(client, self.snapshot.tick());
                    self.record_shop_frame();
                    self.runner.observe_script_running();
                }
                Err(error) => {
                    self.trace
                        .lock()
                        .fail(format!("native start_test_script failed: {error}"));
                    return;
                }
            }
        }

        if matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            if self.started {
                self.packet_trace.observe(client, self.snapshot.tick());
            }
            return;
        }
        self.runner.tick_with_hold(client, hold);
        if self.started {
            self.packet_trace.observe(client, self.snapshot.tick());
        }
    }
}

fn item_value(item: &ItemView) -> Value {
    json!({
        "id": item.def.id,
        "name": item.def.name,
        "slot": item.slot,
        "count": item.count,
        "component_id": item.component_id,
        "container": item.container,
    })
}

struct AcquireShopScript {
    kind: CellKind,
    fixture: Fixture,
    selected: Arc<SelectedGameData>,
    banks: Arc<NamedBankFacts>,
    trace: Arc<Mutex<ShopTrace>>,
    run: Option<AcquireRun>,
}

impl AcquireShopScript {
    fn new(
        kind: CellKind,
        fixture: Fixture,
        selected: Arc<SelectedGameData>,
        banks: Arc<NamedBankFacts>,
        trace: Arc<Mutex<ShopTrace>>,
    ) -> Self {
        Self {
            kind,
            fixture,
            selected,
            banks,
            trace,
            run: None,
        }
    }

    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.trace.lock().fail(message.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "acquire-shop-live".into(),
            message: message.into(),
        })
    }

    fn initialize(&mut self, tick: &mut NativeTick<'_>) -> Result<(), String> {
        let members = tick
            .cx
            .snapshot()
            .world()
            .map(|world| world.value.members)
            .ok_or_else(|| "native acquisition snapshot has no observed world gate".to_owned())?;
        let request = AcquireRequest {
            needs: Arc::from([
                Need {
                    item: self.fixture.harpoon.id,
                    name: Arc::clone(&self.fixture.harpoon.name),
                    target: HARPOON_TARGET,
                    minimum: HARPOON_TARGET,
                    presence: Presence::Held,
                    kind: NeedKind::Required,
                    sources: SourceSet::PACK | SourceSet::SHOP,
                    alternatives: Arc::<[i32]>::from([]),
                },
                Need {
                    item: self.fixture.bait.id,
                    name: Arc::clone(&self.fixture.bait.name),
                    target: BAIT_TARGET,
                    minimum: BAIT_TARGET,
                    presence: Presence::Held,
                    kind: NeedKind::Required,
                    sources: SourceSet::PACK | SourceSet::SHOP,
                    alternatives: Arc::<[i32]>::from([]),
                },
            ]),
            deposit: None,
            bank: BankChoice {
                bank: None,
                explicit: None,
                required: false,
                preferences: Default::default(),
            },
            stage: BankStage::Planned,
            partial_ok: false,
            close: CloseBank::Close,
            vendor: self.fixture.vendor_choice(),
            coins: CoinPolicy::ForPurchase {
                sources: SourceSet::PACK,
            },
            walk: WalkOptions::default(),
            required_after: tick.cx.evidence(),
        };
        let request_facts = json!({
            "cell": self.kind.name(),
            "vendor_input": self.kind.vendor_name(),
            "needs": [
                {"item_id": self.fixture.harpoon.id, "name": self.fixture.harpoon.name.as_ref(), "target": HARPOON_TARGET, "minimum": HARPOON_TARGET, "sources": ["PACK", "SHOP"]},
                {"item_id": self.fixture.bait.id, "name": self.fixture.bait.name.as_ref(), "target": BAIT_TARGET, "minimum": BAIT_TARGET, "sources": ["PACK", "SHOP"]},
            ],
            "coin_policy": {"sources": ["PACK"], "seeded_coins": SEED_COINS},
            "membership_gate": members,
        });
        let stock = tick.cx.snapshot().stock();
        let plan = match plan::plan(
            &request,
            &stock,
            WorldFacts {
                data: Some(&self.selected),
                members,
            },
        )
        .map_err(|error| format!("acquisition planning failed: {error:?}"))?
        {
            script::acquire::AcquireVerdict::Plan(plan) => plan,
            script::acquire::AcquireVerdict::Satisfied => {
                return Err("fresh pack unexpectedly satisfied both acquisition needs".into())
            }
            script::acquire::AcquireVerdict::Blocked(block) => {
                return Err(format!(
                    "planner blocked {}: {}",
                    block.code(),
                    block.message
                ))
            }
        };
        self.trace.lock().request = Some(request_facts);
        self.run = Some(AcquireRun::new(request, plan, &self.banks, &tick.cx));
        Ok(())
    }
}

impl Script for AcquireShopScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.run.is_none() {
            if let Err(message) = self.initialize(tick) {
                return Ok(self.blocked(message));
            }
        }
        match self
            .run
            .as_mut()
            .expect("AcquireRun initialized")
            .poll(tick, OpenGate::Open)
        {
            std::task::Poll::Pending => Ok(ScriptFlow::Continue),
            std::task::Poll::Ready(Err(error)) => {
                Ok(self.blocked(format!("AcquireRun: {error:?}")))
            }
            std::task::Poll::Ready(Ok(AcquireOutcome::ChildDue { need })) => Ok(self.blocked(
                format!("AcquireRun unexpectedly deferred child need {need}"),
            )),
            std::task::Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                let mut trace = self.trace.lock();
                trace.outcome = Some(json!({
                    "variant": "Done",
                    "complete": receipt.complete,
                    "bought": receipt.bought,
                    "withdrew_any": receipt.withdrew_any,
                    "deposited": receipt.deposited,
                    "bank_open": receipt.bank_open,
                }));
                trace.completion_tick = Some(tick.cx.evidence().tick);
                trace.completed_at = Some(Instant::now());
                Ok(ScriptFlow::Complete)
            }
        }
    }
}

#[derive(Debug)]
struct ButtonAction {
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

fn shop_button(action: &Value, action_index: usize) -> Option<ButtonAction> {
    let request = &action["request"];
    let debug = request["debug"].as_str()?;
    if !debug.starts_with("ShopButton {") || !debug.contains("kind: \"buy\"") {
        return None;
    }
    let item_id = debug_number(debug, "id")?;
    let chunk = debug_number(debug, "chunk")?;
    Some(ButtonAction {
        sequence: action["sequence"].as_u64()?,
        tick: action["snapshot_tick"]
            .as_u64()
            .or_else(|| action["tick"].as_u64())?,
        item_id,
        chunk,
        accepted: action["accepted"].as_bool().unwrap_or(false),
        decoded: action["wire_decoded"].as_bool().unwrap_or(false),
        wire_opcodes: action["wire_opcodes"]
            .as_array()
            .map(|rows| rows.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default(),
        action_index,
        before: inventory_count(&action["snapshot"]["inventory"], item_id),
    })
}

fn inventory_count(rows: &Value, item_id: i64) -> i64 {
    rows.as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["id"].as_i64() == Some(item_id))
        .map(|row| row["count"].as_i64().unwrap_or(0))
        .sum()
}

fn held_count(frame: &Value, item_id: i64) -> i64 {
    inventory_count(&frame["inventory"], item_id)
}

fn button_analysis(actions: &[Value], frames: &[Value]) -> (Vec<Value>, Vec<String>, usize) {
    let buttons = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| shop_button(action, index))
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(buttons.len());
    let mut failures = Vec::new();
    let mut reconciled = 0usize;
    for (position, button) in buttons.iter().enumerate() {
        let next = buttons.get(position + 1);
        let next_tick = next.map_or(u64::MAX, |next| next.tick);
        let mut observations = Vec::<(u64, u64, i64, &'static str)>::new();
        for frame in frames {
            let tick = frame["tick"].as_u64().unwrap_or(0);
            if tick > button.tick && tick < next_tick {
                observations.push((tick, 0, held_count(frame, button.item_id), "snapshot-frame"));
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
                    inventory_count(&action["snapshot"]["inventory"], button.item_id),
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
                "ShopButton sequence {} was not accepted with decoded packet evidence",
                button.sequence
            ));
        }
        if let Some(transferred) = transferred {
            reconciled += 1;
            if transferred != button.chunk {
                failures.push(format!(
                    "ShopButton sequence {} requested {} but observed transfer was {}",
                    button.sequence, button.chunk, transferred
                ));
            }
        } else {
            failures.push(format!(
                "ShopButton sequence {} had no observed held-count resolution",
                button.sequence
            ));
        }
        if reply_edges.is_none() {
            failures.push(format!(
                "ShopButton sequence {} had no observed reply edge",
                button.sequence
            ));
        }
        if let Some(edges) = reply_edges {
            if edges > script::acquire::buy::BUY_REPLY_TICKS {
                failures.push(format!("ShopButton sequence {} observed reply after {edges} ticks, above Engine L probe-qualified patience cap {}", button.sequence, script::acquire::buy::BUY_REPLY_TICKS));
            }
        }
        if let Some(next) = next {
            let next_action = &actions[next.action_index];
            let held_before_next =
                inventory_count(&next_action["snapshot"]["inventory"], button.item_id);
            if held_before_next < target_held {
                failures.push(format!("ShopButton sequence {} was followed by another button before its {}-unit transfer was observed", button.sequence, button.chunk));
            }
            if resolved.is_some_and(|(tick, _, _, _)| *tick > next.tick) {
                failures.push(format!(
                    "ShopButton sequence {} resolved after the next button tick",
                    button.sequence
                ));
            }
        }
        let wire = button
            .wire_opcodes
            .iter()
            .map(|opcode| json!(opcode))
            .collect::<Vec<_>>();
        rows.push(json!({
            "sequence": button.sequence,
            "tick": button.tick,
            "item_id": button.item_id,
            "chunk": button.chunk,
            "accepted": button.accepted,
            "wire_decoded": button.decoded,
            "wire_opcodes": wire,
            "held_before": button.before,
            "observed_reply_tick": response_tick,
            "observed_reply_edges": reply_edges,
            "reply_tick_cap": script::acquire::buy::BUY_REPLY_TICKS,
            "reply_cap_status": "Engine L probe-qualified patience cap; this run fails if an observed reply exceeds it",
            "resolved_tick": resolved.map(|(tick, _, _, _)| *tick),
            "resolved_by": resolved.map(|(_, _, _, source)| *source),
            "observed_transfer": transferred,
            "next_button_tick": next.map(|next| next.tick),
            "held_before_next_button": next.map(|next| inventory_count(&actions[next.action_index]["snapshot"]["inventory"], button.item_id)),
        }));
    }
    (rows, failures, reconciled)
}

fn is_close_modal(action: &Value) -> bool {
    action["request"]["debug"].as_str() == Some("CloseModal")
}

fn trade_records(
    actions: &[Value],
    fixture: &Fixture,
    keepers: &ShopKeeperFacts,
    data: &SelectedGameData,
    members: Option<bool>,
) -> (Vec<Value>, Vec<String>) {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        let request = &action["request"];
        if request["op"] != "npc"
            || !request["action"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("trade"))
        {
            continue;
        }
        let npc_index = request["index"].as_i64();
        let npc = action["snapshot"]["nearby_npcs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|npc| npc["index"].as_i64() == npc_index);
        let Some(npc) = npc else {
            failures.push(format!(
                "Trade sequence {} has no matching observed NPC row",
                action["sequence"].as_u64().unwrap_or(0)
            ));
            continue;
        };
        let type_id = npc["type"].as_i64();
        let following_item = actions[index + 1..]
            .iter()
            .enumerate()
            .find_map(|(offset, later)| {
                shop_button(later, index + 1 + offset).map(|button| button.item_id)
            });
        let keeper = type_id.and_then(|id| {
            keepers
                .rows()
                .iter()
                .find(|row| i64::from(row.keeper.keeper_id) == id)
        });
        let config = keeper.map(|row| row.keeper.keeper_config);
        let requested_item = match following_item {
            Some(id) if id == i64::from(fixture.harpoon.id) => Some(fixture.harpoon.id),
            Some(id) if id == i64::from(fixture.bait.id) => Some(fixture.bait.id),
            _ => None,
        };
        let allowed_type = match (requested_item, members, type_id) {
            (Some(item_id), Some(members), Some(type_id)) => keepers
                .candidates_for(data, item_id, members)
                .any(|candidate| {
                    keepers
                        .rows()
                        .get(usize::from(candidate.index))
                        .is_some_and(|row| i64::from(row.keeper.keeper_id) == type_id)
                }),
            _ => false,
        };
        if type_id.is_none() || keeper.is_none() || !allowed_type {
            failures.push(format!(
                "Trade sequence {} did not resolve to an eligible pinned keeper type under the observed account gate",
                action["sequence"].as_u64().unwrap_or(0)
            ));
        }
        if fixture.kind == CellKind::Explicit && type_id != Some(i64::from(fixture.gerrant.id)) {
            failures.push(format!(
                "explicit Gerrant request traded with observed NPC type {type_id:?}"
            ));
        }
        if fixture.kind == CellKind::Nearest
            && following_item == Some(i64::from(fixture.harpoon.id))
            && type_id != Some(i64::from(fixture.harry.id))
        {
            failures.push(format!(
                "nearest harpoon trade did not observe pinned Harry type {}",
                fixture.harry.id
            ));
        }
        rows.push(json!({
            "sequence": action["sequence"],
            "tick": action["snapshot_tick"],
            "request_npc_index": npc_index,
            "observed_npc_type": type_id,
            "observed_npc_name": npc["name"],
            "observed_tile": npc["tile"],
            "observed_network_tile": npc["network"],
            "pinned_keeper_config": config,
            "pinned_keeper_id": keeper.map(|row| row.keeper.keeper_id),
            "pinned_placement_tile": keeper.map(|row| row.tile),
            "next_button_item_id": following_item,
            "account_membership": members,
            "eligible_pinned_type": allowed_type,
        }));
    }
    if rows.is_empty() {
        failures.push("no observed NPC Trade action was captured".into());
    }
    if members.is_none() {
        failures.push(
            "Trade actions cannot be checked against an observed account membership gate".into(),
        );
    }
    (rows, failures)
}

fn close_analysis(
    actions: &[Value],
    frames: &[Value],
    baseline_closes: Option<u64>,
) -> (Value, Vec<String>) {
    let closes = actions
        .iter()
        .filter(|action| is_close_modal(action))
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    let baseline = baseline_closes.unwrap_or(0);
    let final_frame = frames.last();
    let final_count = final_frame
        .and_then(|frame| frame["shop"]["server_closes"].as_u64())
        .unwrap_or(baseline);
    let final_open = final_frame
        .and_then(|frame| frame["shop"]["open"].as_bool())
        .unwrap_or(true);
    let final_action = closes.last();
    if closes.is_empty() {
        failures.push("no CloseModal interaction was captured".into());
    }
    if let Some(action) = final_action {
        if !action["accepted"].as_bool().unwrap_or(false)
            || !action["wire_decoded"].as_bool().unwrap_or(false)
            || action["wire_opcodes"].as_array().is_none_or(Vec::is_empty)
        {
            failures
                .push("final CloseModal lacked accepted, decoded Driver packet evidence".into());
        }
    }
    if final_count <= baseline {
        failures.push("final server close count did not advance beyond the Start baseline".into());
    }
    if final_open {
        failures.push("shop remained locally open in the final observed snapshot".into());
    }
    let close_delta = final_count.saturating_sub(baseline);
    if close_delta < closes.len() as u64 {
        failures.push(format!(
            "server close count advanced by {close_delta} for {} CloseModal actions",
            closes.len()
        ));
    }
    let last_button = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| shop_button(action, index))
        .next_back();
    if let Some(final_action) = final_action {
        match last_button {
            Some(last_button)
                if final_action["sequence"].as_u64().unwrap_or(0) > last_button.sequence => {}
            Some(_) => failures.push("final CloseModal did not follow the final ShopButton".into()),
            None => failures.push("CloseModal had no preceding ShopButton".into()),
        }
    }
    (
        json!({
            "close_modal_actions": closes.len(),
            "baseline_server_closes": baseline,
            "final_server_closes": final_count,
            "server_close_delta": close_delta,
            "final_close_action": final_action.map(|action| json!({
                "sequence": action["sequence"],
                "tick": action["snapshot_tick"],
                "accepted": action["accepted"],
                "wire_decoded": action["wire_decoded"],
                "wire_opcodes": action["wire_opcodes"],
            })),
            "final_shop_open": final_open,
            "final_shop_frame": final_frame,
        }),
        failures,
    )
}

fn packet_action_ticks(actions: &[Value]) -> (Value, usize) {
    let mut by_tick = BTreeMap::<u64, Vec<i64>>::new();
    for action in actions {
        let Some(tick) = action["tick"].as_u64() else {
            continue;
        };
        let Some(opcodes) = action["wire_opcodes"].as_array() else {
            continue;
        };
        by_tick
            .entry(tick)
            .or_default()
            .extend(opcodes.iter().filter_map(Value::as_i64));
    }
    let max = by_tick.values().map(Vec::len).max().unwrap_or(0);
    let rows = by_tick
        .into_iter()
        .map(|(tick, opcodes)| json!({"tick": tick, "count": opcodes.len(), "opcodes": opcodes}))
        .collect::<Vec<_>>();
    (json!(rows), max)
}

fn over_target_failures(actions: &[Value], frames: &[Value], fixture: &Fixture) -> Vec<String> {
    let mut failures = Vec::new();
    for frame in frames {
        let tick = frame["tick"].as_u64().unwrap_or(0);
        for (item, target, label) in [
            (fixture.harpoon.id, HARPOON_TARGET, "harpoon"),
            (fixture.bait.id, BAIT_TARGET, "bait"),
        ] {
            let held = frame["held"][label].as_i64().unwrap_or(-1);
            if held < 0 || held > i64::from(target) {
                failures.push(format!(
                    "{label} held count {held} exceeded target {target} at tick {tick} (item {item})"
                ));
            }
        }
    }
    for action in actions {
        let tick = action["snapshot_tick"].as_u64().unwrap_or(0);
        let Some(inventory) = action["snapshot"]["inventory"].as_array() else {
            failures.push(format!(
                "interaction snapshot at tick {tick} had no inventory evidence"
            ));
            continue;
        };
        for (item, target, label) in [
            (fixture.harpoon.id, HARPOON_TARGET, "harpoon"),
            (fixture.bait.id, BAIT_TARGET, "bait"),
        ] {
            let held = inventory
                .iter()
                .filter(|row| row["id"].as_i64() == Some(i64::from(item)))
                .map(|row| row["count"].as_i64().unwrap_or(0))
                .sum::<i64>();
            if held < 0 || held > i64::from(target) {
                failures.push(format!(
                    "{label} held count {held} exceeded target {target} in action snapshot at tick {tick} (item {item})"
                ));
            }
        }
    }
    failures
}

fn write_png(path: &Path, stem: &str, pixels: &[u32], sidecar: &Value) -> Result<PathBuf, String> {
    if pixels.len() != 765 * 503 {
        return Err(format!(
            "rendered frame had {} pixels, expected 765x503",
            pixels.len()
        ));
    }
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            0xff,
        ]);
    }
    scenario::shot::write_shot(
        path,
        stem,
        &rgba,
        765,
        503,
        &serde_json::to_string_pretty(sidecar).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn run_cell(kind: CellKind) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "{} requires LIVE=1",
        kind.name()
    );
    let root = std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .expect("acquire shop live cells require LIVE_EVIDENCE_DIR");
    let names = super::mint_live_names(1);
    let account = names.first().expect("mint one live account").clone();
    let (minted_name, password) = super::mint_live_entries(&names)
        .into_iter()
        .find(|(name, _)| name == &account)
        .expect("minted live account password");
    assert_eq!(minted_name, account);
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_secs();
    let evidence = root.join(format!("{}-{account}-{epoch}", kind.name()));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence and scratch directories");
    let options =
        profile_options(&scratch).unwrap_or_else(|error| panic!("live prerequisites: {error}"));
    let nav_pack = options.nav_pack.clone().expect("reference navigation pack");
    let nav_flags = options
        .nav_flags
        .clone()
        .expect("reference navigation flags");
    let template = options
        .resolve(None)
        .expect("resolve R289 Engine L profile")
        .prepare_template()
        .expect("prepare isolated R289 reference-pack template");
    let selected = game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let nav_world = template.world().expect("reference navigation world");
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
            .expect("reference world binds selected R289 shop keepers"),
    );
    let fixture = fixture(kind, &selected, &keepers)
        .unwrap_or_else(|error| panic!("R289 content fixture: {error}"));
    let trace = Arc::new(Mutex::new(ShopTrace::default()));
    let capture = Arc::new(StdMutex::new(CombatCapture::default()));
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let scenario = scenario(kind, fixture.stand, Arc::clone(&trace));
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let frame = FrameBuf::new();
    let state = Arc::new(Mutex::new(LiveState {
        account: account.clone(),
        kind,
        fixture: fixture.clone(),
        selected: Arc::clone(&selected),
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        frame: Arc::clone(&frame),
        start_pixels: None,
        start_context: None,
        trace: Arc::clone(&trace),
        started_at: None,
        started: false,
        packet_trace: DriverPacketCounts::default(),
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let frame_buffer = Arc::clone(&frame);
    let uid = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_millis()
        % i32::MAX as u128) as i32;
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
        vec![profile],
        move |_| (None, Some(Arc::clone(&frame_buffer))),
        move |client, username, hold| {
            if username == frame_account {
                client.set_draw(true);
                frame_state.lock().frame(client, hold.hold);
            }
        },
    )
    .expect("start local R289 Engine A Play");
    let obj_names = play.obj_names();
    {
        let mut state = state.lock();
        state.runner.set_obj_names(obj_names);
        state.start_context = Some((play.script_start_handle(), play.named_banks()));
    }
    play.focus(&account);

    let preparation_deadline = Instant::now() + PREPARATION_DEADLINE;
    let mut terminal_at = None;
    let mut terminal_error = None;
    loop {
        // The quester-stage preparation relogs a scriptless bot: the host
        // keeps it logged out after the logout until the operator Login.
        if let Some(arm) = play.arm(&account) {
            if state.lock().runner.take_relog_login() {
                arm.arm_explicit_login();
            }
        }
        if let Some(status) = play.script_native_status(&account) {
            combat_proof::record_status(&account, &status, None);
        }
        let (status, started_at, completed_at, failure) = {
            let state = state.lock();
            let trace = trace.lock();
            (
                format!("{:?}", state.runner.status()),
                state.started_at,
                trace.completed_at,
                trace.failure.clone(),
            )
        };
        if let Some(reason) = failure {
            terminal_error = Some(reason);
            break;
        }
        if let Some(started_at) = started_at {
            if let Some(completed_at) = completed_at {
                if completed_at.saturating_duration_since(started_at) >= CELL_DEADLINE {
                    terminal_error = Some(format!(
                        "AcquireRun exceeded fixed {}s cell deadline",
                        CELL_DEADLINE.as_secs()
                    ));
                    trace
                        .lock()
                        .fail(terminal_error.clone().expect("set error"));
                    break;
                }
                if status != "Passed" && completed_at.elapsed() >= TERMINAL_CAPTURE_GRACE {
                    terminal_error = Some("AcquireRun completed but the ScenarioRunner did not pass within terminal capture grace".into());
                    trace
                        .lock()
                        .fail(terminal_error.clone().expect("set error"));
                    break;
                }
            } else if status != "Passed" && started_at.elapsed() >= CELL_DEADLINE {
                terminal_error = Some(format!(
                    "AcquireRun exceeded fixed {}s cell deadline",
                    CELL_DEADLINE.as_secs()
                ));
                trace
                    .lock()
                    .fail(terminal_error.clone().expect("set error"));
                break;
            }
        } else if status != "Passed" && Instant::now() >= preparation_deadline {
            terminal_error = Some(format!(
                "account preparation exceeded fixed {}s deadline",
                PREPARATION_DEADLINE.as_secs()
            ));
            trace
                .lock()
                .fail(terminal_error.clone().expect("set error"));
            break;
        }
        if status == "Passed" {
            terminal_at.get_or_insert_with(Instant::now);
            if terminal_at.is_some_and(|at| at.elapsed() >= TERMINAL_CAPTURE_GRACE) {
                break;
            }
        } else if status.starts_with("Failed") {
            terminal_error = Some(format!("ScenarioRunner ended with {status}"));
            trace
                .lock()
                .fail(terminal_error.clone().expect("set error"));
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    play.script_stop(&account);

    let (trace_json, shop_frames, baseline_closes) = {
        let trace = trace.lock();
        (
            json!({
                "started": trace.started,
                "start_tick": trace.start_tick,
                "start_snapshot": trace.start_snapshot,
                "start_members": trace.start_members,
                "start_server_closes": trace.start_server_closes,
                "script_run": trace.script_run,
                "request": trace.request,
                "outcome": trace.outcome,
                "completion_tick": trace.completion_tick,
                "failure": trace.failure,
            }),
            trace.shop_frames.clone(),
            trace.start_server_closes,
        )
    };
    let capture_json = {
        let capture = capture.lock().unwrap_or_else(|error| error.into_inner());
        json!({
            "frames": capture.frames,
            "statuses": capture.statuses,
            "actions": capture.actions,
            "random_events": capture.random_events,
            "observations": capture.observations,
            "start_baseline": capture.start_baseline,
            "invalid_reason": capture.invalid_reason,
        })
    };
    let actions = capture_json["actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let all_shop_buttons = actions
        .iter()
        .filter(|action| {
            action["request"]["debug"]
                .as_str()
                .is_some_and(|debug| debug.starts_with("ShopButton {"))
        })
        .count();
    let (buttons, mut button_failures, reconciled) = button_analysis(&actions, &shop_frames);
    let (trades, mut trade_failures) = trade_records(
        &actions,
        &fixture,
        &keepers,
        &selected,
        trace_json["start_members"].as_bool(),
    );
    let (close, mut close_failures) = close_analysis(&actions, &shop_frames, baseline_closes);
    let mut failures = Vec::new();
    if !capture_json["start_baseline"].is_object() {
        failures.push("generic action capture has no StartScript baseline".into());
    }
    if let Some(reason) = capture_json["invalid_reason"].as_str() {
        failures.push(format!("generic action capture was invalid: {reason}"));
    }
    if all_shop_buttons != buttons.len() {
        failures.push(format!(
            "captured {} ShopButton actions but parsed {} buy buttons",
            all_shop_buttons,
            buttons.len()
        ));
    }
    failures.append(&mut button_failures);
    failures.append(&mut trade_failures);
    failures.append(&mut close_failures);
    failures.extend(over_target_failures(&actions, &shop_frames, &fixture));
    let (driver_packets, max_driver_packets, driver_packets_complete) = {
        let state = state.lock();
        if !state
            .packet_trace
            .includes_interactions(&DriverPacketTrace::from_actions(&actions))
        {
            failures.push("real sink counts omitted a decoded interaction packet".into());
        }
        state.packet_trace.value()
    };
    if !driver_packets_complete {
        failures.push("outgoing Driver packet trace was incomplete".into());
    }
    if max_driver_packets > MAX_DRIVER_PACKETS_PER_TICK {
        failures.push(format!(
            "observed {max_driver_packets} Driver packets in one captured tick (limit {MAX_DRIVER_PACKETS_PER_TICK})"
        ));
    }
    let (interaction_packets, max_interaction_packets) = packet_action_ticks(&actions);
    if max_interaction_packets > MAX_DRIVER_PACKETS_PER_TICK {
        failures.push(format!(
            "observed {max_interaction_packets} interaction packets in one captured tick (limit {MAX_DRIVER_PACKETS_PER_TICK})"
        ));
    }
    let final_frame = shop_frames.last();
    let final_harpoon = final_frame.and_then(|frame| frame["held"]["harpoon"].as_i64());
    let final_bait = final_frame.and_then(|frame| frame["held"]["bait"].as_i64());
    let native_outcome = trace_json["outcome"].clone();
    if native_outcome["variant"] != "Done"
        || native_outcome["complete"] != true
        || native_outcome["bought"] != 11
    {
        failures.push("AcquireRun did not return Done(complete=true, bought=11)".into());
    }
    if final_harpoon != Some(i64::from(HARPOON_TARGET))
        || final_bait != Some(i64::from(BAIT_TARGET))
    {
        failures.push(format!(
            "final held targets were harpoon={final_harpoon:?}, bait={final_bait:?}"
        ));
    }
    let sent_buttons = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| shop_button(action, index))
        .map(|button| (button.item_id, button.chunk))
        .collect::<Vec<_>>();
    let expected_buttons = [
        (i64::from(fixture.harpoon.id), i64::from(HARPOON_TARGET)),
        (
            i64::from(fixture.bait.id),
            i64::from(BAIT_TARGET - SEED_BAIT),
        ),
    ];
    if sent_buttons.as_slice() != expected_buttons.as_slice() {
        failures.push(format!(
            "sent button sequence {sent_buttons:?} did not match {expected_buttons:?}"
        ));
    }
    let observed_button_transfer = buttons
        .iter()
        .map(|row| row["observed_transfer"].as_i64().unwrap_or(0))
        .sum::<i64>();
    if observed_button_transfer != 11 || reconciled != buttons.len() || buttons.len() != 2 {
        failures.push(format!("observed button reconciliation was {observed_button_transfer} units across {reconciled}/{} buttons", buttons.len()));
    }
    let fixture_facts = fixture.facts(&keepers, &selected, trace_json["start_members"].as_bool());
    if kind == CellKind::Nearest {
        let harpoon_trade = trades
            .iter()
            .find(|trade| trade["next_button_item_id"] == fixture.harpoon.id);
        if harpoon_trade.is_none()
            || harpoon_trade.is_some_and(|trade| trade["observed_npc_type"] != fixture.harry.id)
        {
            failures.push(
                "nearest harpoon Trade was not correlated to pinned Harry in Catherby".into(),
            );
        }
    }
    if kind == CellKind::Explicit
        && trades
            .iter()
            .any(|trade| trade["observed_npc_type"] != fixture.gerrant.id)
    {
        failures.push("explicit request did not keep Gerrant authoritative for every Trade".into());
    }
    let mut receipt_base = json!({
        "schema": "gatherer-a4-core-acquire-shop-live-r1",
        "cell": kind.name(),
        "account": account,
        "engine": {"host": "127.0.0.1", "game_port": ENGINE_L_GAME_PORT, "http_port": ENGINE_L_HTTP_PORT},
        "model": "openai-codex/gpt-6-luna",
        "selected_revision": "R289",
        "reference_nav_pack": nav_pack,
        "reference_nav_flags": nav_flags,
        "fixture": fixture_facts,
        "scenario_trace": trace_json,
        "acquire_outcome": native_outcome,
        "trade_npcs": trades,
        "buttons": buttons,
        "action_capture_invalid_reason": capture_json["invalid_reason"],
        "all_shop_button_actions": all_shop_buttons,
        "expected_button_sequence": expected_buttons,
        "button_reconciliations": reconciled,
        "observed_button_transfer_total": observed_button_transfer,
        "close_reconciliation": close,
        "final_held": {"harpoon": final_harpoon, "bait": final_bait},
        "driver_packets_by_tick": driver_packets,
        "max_driver_packets_in_observed_tick": max_driver_packets,
        "driver_packet_trace_complete": driver_packets_complete,
        "interaction_packets_by_tick": interaction_packets,
        "max_interaction_packets_in_observed_tick": max_interaction_packets,
        "buy_reply_tick_cap": script::acquire::buy::BUY_REPLY_TICKS,
        "buy_reply_cap_status": "Engine L probe-qualified patience; each observed dispatch reply above the cap fails this run and is reported per button",
        "typed_service_pick_limit": "The test-only action capture does not expose the internal ServicePickReceipt index/access/cost; nearest evidence records its input and actual Trade NPC type/index/tile correlated to pinned keeper facts.",
        "terminal_error": terminal_error,
    });
    std::fs::write(
        evidence.join("actions.json"),
        serde_json::to_vec_pretty(&actions).expect("serialize actions"),
    )
    .expect("write raw action capture");
    std::fs::write(
        evidence.join("snapshots.json"),
        serde_json::to_vec_pretty(&json!({
            "combat_frames": capture_json["frames"],
            "start_baseline": capture_json["start_baseline"],
            "shop_frames": shop_frames,
            "statuses": capture_json["statuses"],
            "driver_packets_by_tick": driver_packets,
        }))
        .expect("serialize snapshots"),
    )
    .expect("write raw snapshot ticks");
    let (start_pixels, terminal_pixels) = {
        let state = state.lock();
        (state.start_pixels.clone(), frame.snapshot())
    };
    let shot_sidecar = |stage| json!({"cell": kind.name(), "account": account, "stage": stage});
    let mut start_png = None;
    if let Some(pixels) = start_pixels.as_deref() {
        match write_png(&evidence, "start", pixels, &shot_sidecar("start")) {
            Ok(path) => start_png = Some(path),
            Err(error) => failures.push(format!("start screenshot failed: {error}")),
        }
    } else if trace_json["started"] == true {
        failures.push("start screenshot frame was not captured".into());
    }
    let terminal_png = match write_png(
        &evidence,
        "terminal",
        &terminal_pixels,
        &shot_sidecar("terminal"),
    ) {
        Ok(path) => Some(path),
        Err(error) => {
            failures.push(format!("terminal screenshot failed: {error}"));
            None
        }
    };
    let passed = failures.is_empty() && terminal_error.is_none() && trace_json["started"] == true;
    receipt_base["failures"] = json!(failures);
    receipt_base["passed"] = json!(passed);
    receipt_base["start_png"] = start_png
        .map(|path| json!(path.display().to_string()))
        .unwrap_or(Value::Null);
    receipt_base["terminal_png"] = terminal_png
        .map(|path| json!(path.display().to_string()))
        .unwrap_or(Value::Null);
    receipt_base["rendered_terminal_pixels"] = json!(terminal_pixels.len());
    std::fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt_base).expect("serialize receipt"),
    )
    .expect("write machine-readable receipt");
    assert!(
        passed,
        "{} failed; evidence: {}",
        kind.name(),
        evidence.display()
    );
}

#[test]
#[ignore = "requires LIVE=1, Engine L, the R289 reference pack, a per-run cache clone, and LIVE_EVIDENCE_DIR"]
fn acquire_shop_explicit_live() {
    run_cell(CellKind::Explicit);
}

#[test]
#[ignore = "requires LIVE=1, Engine L, the R289 reference pack, a per-run cache clone, and LIVE_EVIDENCE_DIR"]
fn acquire_shop_nearest_live() {
    run_cell(CellKind::Nearest);
}

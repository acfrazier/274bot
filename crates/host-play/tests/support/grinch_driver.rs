use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::hostlog::{Record, Sink};
use api::interact::{self, ActionSpec, Driver, Interactions, OpTarget, SendResult};
use api::named_banks::{BankDefinition, NamedBank};
use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use client::client::Client;
use client::util::jstring::JString;
use host::{self, Pump};
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::WorldState;
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use vault::{Profile, ProfileSettings};

use super::grinch_oracle as oracle;
use oracle::*;

const ANCHOR: Tile = Tile {
    x: 3185,
    z: 3440,
    plane: 0,
};
const NEAR: Tile = Tile {
    x: 3184,
    z: 3440,
    plane: 0,
};
const OUTSIDE: Tile = Tile {
    x: 3198,
    z: 3440,
    plane: 0,
};
const RADIUS: u8 = 12;
const CAPTURE_SCHEMA: &str = "grinch-live-capture-v1";
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const INVENTORY_CAPACITY: u16 = 28;

static HOST_LOG_SINK: HostLogSink = HostLogSink;
static HOST_LOGS: Mutex<Vec<ObservedLogRecord>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    ATradeableDrops,
    AGreenMaskSelection,
    ANegativeCandidates,
    APrivateUntradeableDrops,
    BPumpkinRespawns,
    BContestedPumpkin,
    BScytheDuplicate,
    BBunnyEarsDuplicate,
    BYoYoRepeat,
    CDeposit28,
    CDeposit27PreserveItem,
    CInventoryBlocked,
}

impl Case {
    pub fn slug(self) -> &'static str {
        match self {
            Self::ATradeableDrops => "a-tradeable-drops",
            Self::AGreenMaskSelection => "a-green-mask-selection",
            Self::ANegativeCandidates => "a-negative-candidates",
            Self::APrivateUntradeableDrops => "a-private-untradeable-drops",
            Self::BPumpkinRespawns => "b-pumpkin-respawns",
            Self::BContestedPumpkin => "b-contested-pumpkin",
            Self::BScytheDuplicate => "b-scythe-duplicate",
            Self::BBunnyEarsDuplicate => "b-bunny-ears-duplicate",
            Self::BYoYoRepeat => "b-yoyo-repeat",
            Self::CDeposit28 => "c-deposit-28",
            Self::CDeposit27PreserveItem => "c-deposit-27-preserve-item",
            Self::CInventoryBlocked => "c-inventory-blocked",
        }
    }

    fn kind(self) -> CellKind {
        match self {
            Self::ATradeableDrops => CellKind::ATradeableDrops,
            Self::AGreenMaskSelection => CellKind::AGreenMaskSelection,
            Self::ANegativeCandidates => CellKind::ANegativeCandidates,
            Self::APrivateUntradeableDrops => CellKind::APrivateUntradeableDrops,
            Self::BPumpkinRespawns => CellKind::BPumpkinRespawns,
            Self::BContestedPumpkin => CellKind::BContestedPumpkin,
            Self::BScytheDuplicate => CellKind::BScytheDuplicate,
            Self::BBunnyEarsDuplicate => CellKind::BBunnyEarsDuplicate,
            Self::BYoYoRepeat => CellKind::BYoYoRepeat,
            Self::CDeposit28 => CellKind::CDeposit28,
            Self::CDeposit27PreserveItem => CellKind::CDeposit27PreserveItem,
            Self::CInventoryBlocked => CellKind::CInventoryBlocked,
        }
    }

    fn is_a(self) -> bool {
        matches!(
            self,
            Self::ATradeableDrops
                | Self::AGreenMaskSelection
                | Self::ANegativeCandidates
                | Self::APrivateUntradeableDrops
        )
    }

    fn is_b(self) -> bool {
        matches!(
            self,
            Self::BPumpkinRespawns
                | Self::BContestedPumpkin
                | Self::BScytheDuplicate
                | Self::BBunnyEarsDuplicate
                | Self::BYoYoRepeat
        )
    }

    fn is_c(self) -> bool {
        matches!(
            self,
            Self::CDeposit28 | Self::CDeposit27PreserveItem | Self::CInventoryBlocked
        )
    }

    fn uses_helper(self) -> bool {
        matches!(
            self,
            Self::ATradeableDrops
                | Self::AGreenMaskSelection
                | Self::ANegativeCandidates
                | Self::APrivateUntradeableDrops
                | Self::CDeposit28
                | Self::CDeposit27PreserveItem
        )
    }

    fn uses_competitor(self) -> bool {
        self == Self::BContestedPumpkin
    }

    fn selected_ids(self) -> BTreeSet<i32> {
        let ids: &[i32] = match self {
            Self::ATradeableDrops | Self::ANegativeCandidates => &TRADEABLE_IDS,
            Self::AGreenMaskSelection => &[GREEN_MASK_ID],
            Self::APrivateUntradeableDrops => &PRIVATE_UNTRADEABLE_IDS,
            Self::BPumpkinRespawns
            | Self::BContestedPumpkin
            | Self::CDeposit28
            | Self::CDeposit27PreserveItem
            | Self::CInventoryBlocked => &[PUMPKIN_ID],
            Self::BScytheDuplicate => &[SCYTHE_ID],
            Self::BBunnyEarsDuplicate => &[BUNNY_EARS_ID],
            Self::BYoYoRepeat => &[YO_YO_ID],
        };
        ids.iter().copied().collect()
    }

    fn helper_drops(self) -> Vec<DropSpec> {
        let near = NEAR;
        match self {
            Self::ATradeableDrops => TRADEABLE_IDS
                .iter()
                .enumerate()
                .map(|(index, &id)| DropSpec {
                    item_id: id,
                    tile: if index % 2 == 0 { ANCHOR } else { near },
                })
                .collect(),
            Self::AGreenMaskSelection => vec![
                DropSpec {
                    item_id: GREEN_MASK_ID,
                    tile: near,
                },
                DropSpec {
                    item_id: BLUE_MASK_ID,
                    tile: near,
                },
                DropSpec {
                    item_id: RED_MASK_ID,
                    tile: near,
                },
            ],
            Self::ANegativeCandidates => vec![
                DropSpec {
                    item_id: ORDINARY_UNSELECTED_ID,
                    tile: near,
                },
                DropSpec {
                    item_id: EASTER_EGG_CERTIFICATE_ID,
                    tile: near,
                },
                DropSpec {
                    item_id: PUMPKIN_ID,
                    tile: OUTSIDE,
                },
            ],
            Self::APrivateUntradeableDrops => PRIVATE_UNTRADEABLE_IDS
                .iter()
                .map(|&id| DropSpec {
                    item_id: id,
                    tile: near,
                })
                .collect(),
            Self::CDeposit28 | Self::CDeposit27PreserveItem => vec![
                DropSpec {
                    item_id: PUMPKIN_ID,
                    tile: near,
                },
                DropSpec {
                    item_id: PUMPKIN_ID,
                    tile: near,
                },
            ],
            _ => Vec::new(),
        }
    }

    fn helper_seed(self) -> Vec<(i32, u32)> {
        let mut counts = BTreeMap::<i32, u32>::new();
        for drop in self.helper_drops() {
            *counts.entry(drop.item_id).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    fn receiver_seed(self) -> Vec<(i32, u32)> {
        match self {
            Self::CDeposit28 => vec![(PUMPKIN_ID, 27)],
            Self::CDeposit27PreserveItem => vec![(PUMPKIN_ID, 26), (ORDINARY_UNSELECTED_ID, 1)],
            Self::CInventoryBlocked => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn expected_pickups(self) -> u32 {
        match self {
            Self::ATradeableDrops => 15,
            Self::AGreenMaskSelection => 1,
            Self::ANegativeCandidates
            | Self::APrivateUntradeableDrops
            | Self::CInventoryBlocked => 0,
            Self::BPumpkinRespawns => 6,
            Self::BContestedPumpkin => 2,
            Self::BScytheDuplicate | Self::BBunnyEarsDuplicate | Self::BYoYoRepeat => {
                if matches!(self, Self::BYoYoRepeat) {
                    2
                } else {
                    1
                }
            }
            Self::CDeposit28 | Self::CDeposit27PreserveItem => 2,
        }
    }

    fn capture_budget_ticks(self) -> u64 {
        match self {
            Self::ATradeableDrops | Self::AGreenMaskSelection | Self::ANegativeCandidates => 240,
            Self::APrivateUntradeableDrops => 160,
            Self::BPumpkinRespawns => 1080,
            Self::BContestedPumpkin => 2120,
            Self::BScytheDuplicate | Self::BBunnyEarsDuplicate => 600,
            Self::BYoYoRepeat => 560,
            Self::CDeposit28 | Self::CDeposit27PreserveItem => 650,
            Self::CInventoryBlocked => 180,
        }
    }

    fn wall_guard(self) -> Duration {
        match self {
            Self::ATradeableDrops
            | Self::AGreenMaskSelection
            | Self::ANegativeCandidates
            | Self::APrivateUntradeableDrops => Duration::from_secs(330),
            Self::BPumpkinRespawns => Duration::from_secs(820),
            Self::BContestedPumpkin => Duration::from_secs(1450),
            Self::BScytheDuplicate | Self::BBunnyEarsDuplicate => Duration::from_secs(480),
            Self::BYoYoRepeat => Duration::from_secs(460),
            Self::CDeposit28 | Self::CDeposit27PreserveItem => Duration::from_secs(600),
            Self::CInventoryBlocked => Duration::from_secs(150),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DropSpec {
    item_id: i32,
    tile: Tile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActorRole {
    Main,
    Helper,
    Competitor,
}

#[derive(Debug, Clone)]
struct RawFrame {
    role: ActorRole,
    account: String,
    actor: ActorRef,
    tick: u64,
    observation_sequence: u64,
    player_info_sequence: u64,
    client_session_generation: u64,
    position: Tile,
    inventory: ItemSnapshot,
    bank: Option<ItemSnapshot>,
    bank_open: bool,
    bank_loaded: bool,
    worn: ItemSnapshot,
    ground: GroundSnapshot,
    object_turn: Option<u64>,
    server_cycle: Option<u64>,
    output_phase: Option<ClientOutputPhase>,
    visible_players: Vec<Value>,
    inventory_generation: u64,
    ground_generation: u64,
    scene_generation: u64,
}
impl RawFrame {
    fn session_id(&self) -> String {
        format!(
            "client:{}:session:{}",
            self.account, self.client_session_generation
        )
    }

    fn to_player_frame(
        &self,
        status: Option<NativeStatus>,
        native_session: Option<&str>,
    ) -> PlayerFrame {
        PlayerFrame {
            tick: self.tick,
            observation_sequence: self.observation_sequence,
            player_info_sequence: Some(self.player_info_sequence),
            actor: self.actor.clone(),
            session_id: native_session
                .map(str::to_owned)
                .or_else(|| Some(self.session_id())),
            position: self.position,
            inventory: Some(self.inventory.clone()),
            bank: self.bank.clone(),
            bank_open: Some(self.bank_open),
            bank_loaded: Some(self.bank_loaded),
            worn: Some(self.worn.clone()),
            ground: Some(self.ground.clone()),
            status,
            object_turn: self.object_turn,
            server_cycle: self.server_cycle,
            client_output_phase: self.output_phase,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogChannel {
    StructuredHost,
    CapturedStderr,
}

#[derive(Debug, Clone)]
struct ObservedLogRecord {
    channel: LogChannel,
    ordinal: u64,
    slot: Option<String>,
    tick: Option<u32>,
    source: String,
    level: String,
    message: String,
}

struct HostLogSink;

impl Sink for HostLogSink {
    fn record(&self, record: &Record<'_>) {
        if let Ok(mut rows) = HOST_LOGS.lock() {
            let ordinal = u64::try_from(rows.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            rows.push(ObservedLogRecord {
                channel: LogChannel::StructuredHost,
                ordinal,
                slot: record.slot.map(str::to_owned),
                tick: record.tick,
                source: format!("{:?}", record.source),
                level: format!("{:?}", record.level),
                message: record.message.to_owned(),
            });
        }
    }
}

fn host_logs_since(cursor: &mut usize) -> Result<Vec<ObservedLogRecord>, String> {
    let rows = HOST_LOGS.lock().map_err(|_| "host log store poisoned")?;
    if *cursor > rows.len() {
        return Err("host log cursor exceeds complete observed ledger".into());
    }
    let result = rows[*cursor..].to_vec();
    *cursor = rows.len();
    Ok(result)
}

fn all_host_logs() -> Result<Vec<ObservedLogRecord>, String> {
    HOST_LOGS
        .lock()
        .map(|rows| rows.clone())
        .map_err(|_| "host log store poisoned".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WireRunKey {
    slot: u64,
    run: u64,
    session: u64,
}

impl From<api::selected::RunKey> for WireRunKey {
    fn from(key: api::selected::RunKey) -> Self {
        Self {
            slot: key.slot,
            run: key.run,
            session: key.session,
        }
    }
}

impl WireRunKey {
    fn run_id(self, account: &str) -> String {
        format!("account:{account}:slot:{}:run:{}", self.slot, self.run)
    }
}

#[derive(Debug, Clone)]
struct StateEvent {
    slot_name: String,
    key: WireRunKey,
    // PLAYER_INFO clock from the owning host record, not the native payload clock.
    tick: u64,
    host_log_ordinal: u64,
    payload: Value,
}
#[derive(Debug, Clone)]
struct TakeEvent {
    slot_name: String,
    key: WireRunKey,
    tick: u64,
    host_log_ordinal: u64,
    item_id: i32,
    tile: Tile,
    outcome: String,
    units: u32,
}
#[derive(Debug, Clone)]
struct PacketEvent {
    slot_name: String,
    account: String,
    key: Option<WireRunKey>,
    tick: u64,
    request: u64,
    count: u32,
    decoded: bool,
    accepted: bool,
    opcodes: Vec<u8>,
    packet_stderr_line_ordinals: Vec<u64>,
    dispatch_stderr_line_ordinal: Option<u64>,
}
#[derive(Debug, Clone)]
struct GroundRequest {
    item_id: i32,
    tile: Tile,
    action: String,
}
#[derive(Debug, Clone)]
enum InteractionKind {
    Ground(GroundRequest),
    OpenBank {
        tile: Option<Tile>,
        name: Option<String>,
        id: Option<i32>,
    },
    CloseBank,
    Other,
}
#[derive(Debug, Clone)]
struct InteractEvent {
    slot_name: String,
    tick: Option<u64>,
    stderr_line_ordinal: u64,
    request: u64,
    text: String,
    kind: InteractionKind,
}
#[derive(Debug, Clone)]
struct BankClickEvent {
    key: WireRunKey,
    bank_generation: u64,
    transfer: u64,
    request: u64,
    click_tick: u64,
    item_id: i32,
    option: String,
    operation: i32,
    main_before: u32,
    host_log_ordinal: u64,
}
#[derive(Debug, Clone)]
struct BankDispatchEvent {
    key: WireRunKey,
    tick: u64,
    request: u64,
    bank_generation: u64,
    transfer: u64,
    item_id: i32,
    accepted: bool,
    emitted_packets: u32,
    opcode: Option<u8>,
    host_log_ordinal: u64,
}
#[derive(Debug, Clone)]
struct WatchdogEvent {
    key: Option<WireRunKey>,
    tick: Option<u64>,
    action: String,
    raw_record: String,
    restarted: bool,
    slot_name: Option<String>,
    host_tick: Option<u32>,
    host_log_ordinal: u64,
    source: String,
    level: String,
}
#[derive(Debug, Clone)]
struct TakeReceiveEvent {
    receipt_sequence: u64,
    server_line_ordinal: u64,
    cycle: u64,
    actor: ActorRef,
    item_id: i32,
    tile: Tile,
    option: i32,
    action: Option<String>,
    accepted: bool,
    reason: String,
}
#[derive(Debug, Clone)]
struct ClientOutputEvent {
    server_line_ordinal: u64,
    cycle: u64,
    actor: ActorRef,
    player_info_sequence: u64,
}
#[derive(Debug, Clone)]
struct CycleEvent {
    server_line_ordinal: u64,
    payload: Value,
}
#[derive(Debug, Clone)]
struct ObjectLifecycleEvent {
    server_line_ordinal: u64,
    receipt: ObjectLifecycleReceipt,
}

#[derive(Debug, Clone)]
struct DebugTickEvent {
    server_line_ordinal: u64,
    cycle: u64,
    raw_line: String,
}

#[derive(Debug, Clone)]
struct WireTake {
    key: Option<WireRunKey>,
    dispatch: TakeDispatch,
    outcome: Option<TakeEvent>,
    receive: Option<TakeReceiveEvent>,
}

#[derive(Clone, Default)]
struct ObservedLogs {
    states: Vec<StateEvent>,
    take_events: Vec<TakeEvent>,
    packets: Vec<PacketEvent>,
    interacts: Vec<InteractEvent>,
    bank_clicks: Vec<BankClickEvent>,
    bank_dispatches: Vec<BankDispatchEvent>,
    watchdog: Vec<WatchdogEvent>,
    take_receives: Vec<TakeReceiveEvent>,
    removals: Vec<ObjectLifecycleEvent>,
    respawns: Vec<ObjectLifecycleEvent>,
    client_outputs: Vec<ClientOutputEvent>,
    cycles: Vec<CycleEvent>,
    debug_ticks: Vec<DebugTickEvent>,
    malformed: Vec<String>,
    server_lines: Vec<String>,
}
impl ObservedLogs {
    fn ingest_record(&mut self, row: ObservedLogRecord) {
        let message = row.message.as_str();
        if message.starts_with("native-packet") && row.channel != LogChannel::CapturedStderr {
            self.malformed
                .push(format!("native packet outside captured stderr: {message}"));
            return;
        }
        if let Some(payload) = message.strip_prefix("GRINCH_STATE ") {
            match serde_json::from_str::<Value>(payload)
                .ok()
                .and_then(|value| {
                    let key = run_key_from_json(&value["run"])?;
                    value["tick"].as_u64()?;
                    let tick = u64::from(row.tick?);
                    Some((key, tick, value))
                }) {
                Some((key, tick, payload)) => self.states.push(StateEvent {
                    slot_name: row.slot.unwrap_or_default(),
                    key,
                    tick,
                    host_log_ordinal: row.ordinal,
                    payload,
                }),
                None => self.malformed.push(format!("state: {message}")),
            }
        } else if let Some(payload) = message.strip_prefix("GRINCH_TAKE ") {
            match serde_json::from_str::<Value>(payload)
                .ok()
                .and_then(|value| {
                    let key = run_key_from_json(&value["run"])?;
                    value["tick"].as_u64()?;
                    let tick = u64::from(row.tick?);
                    let item_id = i32::try_from(value["item_id"].as_i64()?).ok()?;
                    let tile = tile_from_json(&value["tile"])?;
                    let outcome = value["outcome"].as_str()?.to_owned();
                    let units = u32::try_from(value["units"].as_u64()?).ok()?;
                    Some((key, tick, item_id, tile, outcome, units))
                }) {
                Some((key, tick, item_id, tile, outcome, units)) => {
                    self.take_events.push(TakeEvent {
                        slot_name: row.slot.unwrap_or_default(),
                        key,
                        tick,
                        host_log_ordinal: row.ordinal,
                        item_id,
                        tile,
                        outcome,
                        units,
                    })
                }
                None => self.malformed.push(format!("take: {message}")),
            }
        } else if message.starts_with("native-packet ") {
            if let Some((key, account, tick, request, opcode)) =
                parse_native_packet(message, row.tick)
            {
                if let Some(packet) = self.packets.iter_mut().rev().find(|packet| {
                    packet.key == key
                        && packet.account == account
                        && packet.tick == tick
                        && packet.request == request
                }) {
                    packet.opcodes.push(opcode);
                    packet.packet_stderr_line_ordinals.push(row.ordinal);
                } else {
                    self.packets.push(PacketEvent {
                        slot_name: row.slot.clone().unwrap_or_else(|| account.clone()),
                        account,
                        key,
                        tick,
                        request,
                        count: 0,
                        decoded: false,
                        accepted: false,
                        opcodes: vec![opcode],
                        packet_stderr_line_ordinals: vec![row.ordinal],
                        dispatch_stderr_line_ordinal: None,
                    });
                }
            } else {
                self.malformed.push(format!("packet: {message}"));
            }
        } else if message.starts_with("native-packets ") {
            if let Some(NativePacketSummary {
                key,
                account,
                tick,
                request,
                count,
                decoded,
                accepted,
            }) = parse_native_packets(message, row.tick)
            {
                if let Some(packet) = self.packets.iter_mut().rev().find(|packet| {
                    packet.key == key
                        && packet.account == account
                        && packet.tick == tick
                        && packet.request == request
                }) {
                    packet.count = count;
                    packet.decoded = decoded;
                    packet.accepted = accepted;
                    packet.dispatch_stderr_line_ordinal = Some(row.ordinal);
                } else {
                    self.packets.push(PacketEvent {
                        slot_name: row.slot.clone().unwrap_or_else(|| account.clone()),
                        account,
                        key,
                        tick,
                        request,
                        count,
                        decoded,
                        accepted,
                        opcodes: Vec::new(),
                        packet_stderr_line_ordinals: Vec::new(),
                        dispatch_stderr_line_ordinal: Some(row.ordinal),
                    });
                }
            } else {
                self.malformed.push(format!("packets: {message}"));
            }
        } else if message.starts_with("bank-deposit-click ") {
            if let Some(mut click) = parse_bank_click(message, row.tick) {
                click.host_log_ordinal = row.ordinal;
                self.bank_clicks.push(click);
            } else {
                self.malformed.push(format!("bank-click: {message}"));
            }
        } else if message.starts_with("bank-deposit-dispatch ") {
            if let Some(mut dispatch) = parse_bank_dispatch(message, row.tick) {
                dispatch.host_log_ordinal = row.ordinal;
                self.bank_dispatches.push(dispatch);
            } else {
                self.malformed.push(format!("bank-dispatch: {message}"));
            }
        } else if message.starts_with("watchdog-action ") {
            if let Some(event) = parse_watchdog_action(message, &row) {
                self.watchdog.push(event);
            } else {
                self.malformed.push(format!("watchdog-action: {message}"));
            }
        } else if message.contains("restarted the script") {
            self.watchdog.push(WatchdogEvent {
                key: run_key_from_debug(message),
                tick: row.tick.map(u64::from),
                action: "legacy_script_restart".to_owned(),
                raw_record: message.to_owned(),
                restarted: true,
                slot_name: row.slot,
                host_tick: row.tick,
                host_log_ordinal: row.ordinal,
                source: row.source,
                level: row.level,
            });
        }
    }

    fn ingest_captured_stderr(
        &mut self,
        text: &str,
        runtime: &RuntimeReport,
    ) -> Result<(), String> {
        // InteractTrace deliberately bypasses Sink. Its payload clock is the
        // observed PLAYER_INFO sequence, not a synthetic host-log header.
        for state in &self.states {
            let native_tick = state.payload["tick"]
                .as_u64()
                .ok_or("state lacks its original native clock")?;
            let frame = frame_for_native_tick(runtime, &state.slot_name, state.key, native_tick)?;
            if frame.tick != state.tick {
                return Err(format!(
                    "native clock {native_tick} for {} disagrees with its owning structured host tick {}",
                    state.slot_name, state.tick
                ));
            }
        }
        let mut pending = HashMap::<String, (u64, String)>::new();
        for (index, line) in text.lines().enumerate() {
            let Some(body) = line.strip_prefix("[interact ") else {
                continue;
            };
            let (account, message) = body
                .split_once("] ")
                .ok_or("malformed captured InteractTrace prefix")?;
            if !runtime.actors.iter().any(|actor| actor.account == account) {
                return Err(format!(
                    "captured InteractTrace names unknown account {account}"
                ));
            }
            let ordinal = u64::try_from(index)
                .map_err(|_| "stderr line ordinal overflow")?
                .checked_add(1)
                .ok_or("stderr line ordinal overflow")?;
            if let Some(request) = message.strip_prefix("interact ") {
                pending.insert(account.to_owned(), (ordinal, request.to_owned()));
                continue;
            }
            let summary = message.starts_with("native-packets ");
            if !summary && !message.starts_with("native-packet ") {
                continue;
            }
            if word_field(message, "account=").as_deref() != Some(account) {
                return Err(
                    "native packet payload account disagrees with its stderr prefix".into(),
                );
            }
            let key = run_key_from_debug(message).ok_or("native packet lacks its actual RunKey")?;
            let native_tick =
                number_field(message, "tick=").ok_or("native packet lacks its original clock")?;
            let frame = frame_for_native_tick(runtime, account, key, native_tick)?;
            let client_tick = u32::try_from(frame.tick)
                .map_err(|_| "native packet owning client tick exceeds u32")?;
            let request =
                number_field(message, "request=").ok_or("native packet lacks its request ID")?;
            if self.packets.iter().any(|packet| {
                packet.account == account
                    && packet.key == Some(key)
                    && packet.request == request
                    && packet.dispatch_stderr_line_ordinal.is_some()
            }) {
                return Err(format!(
                    "native request {request} has a duplicate or late packet record"
                ));
            }
            self.ingest_record(ObservedLogRecord {
                channel: LogChannel::CapturedStderr,
                ordinal,
                slot: Some(account.to_owned()),
                tick: Some(client_tick),
                source: "Script".to_owned(),
                level: "Debug".to_owned(),
                message: message.to_owned(),
            });
            if !summary {
                continue;
            }
            let packet = self
                .packets
                .iter()
                .find(|packet| {
                    packet.account == account
                        && packet.key == Some(key)
                        && packet.tick == frame.tick
                        && packet.request == request
                })
                .ok_or("native dispatch summary lacks its exact packet batch")?;
            if usize::try_from(packet.count).ok() != Some(packet.opcodes.len()) {
                return Err(format!(
                    "native request {request} summary count disagrees with decoded packet rows"
                ));
            }
            if let Some((stderr_line_ordinal, text)) = pending.remove(account) {
                let ground = parse_ground_request(&text);
                if text.starts_with("Obj {")
                    && ground.is_none()
                    && quoted_field(&text, "action:").as_deref() == Some("Take")
                {
                    return Err("captured Obj Take request could not be parsed".into());
                }
                let kind = parse_interaction_kind(&text, ground);
                self.interacts.push(InteractEvent {
                    slot_name: account.to_owned(),
                    tick: Some(frame.tick),
                    stderr_line_ordinal,
                    request,
                    text,
                    kind,
                });
            } else if !self.bank_dispatches.iter().any(|dispatch| {
                dispatch.key == key && dispatch.tick == frame.tick && dispatch.request == request
            }) {
                // Native deposits use their dedicated structured BankOp
                // dispatch rather than the generic "interact" trace.
                return Err(format!(
                    "native request {request} lacks its actual interaction or bank dispatch"
                ));
            }
        }
        if self
            .packets
            .iter()
            .any(|packet| packet.dispatch_stderr_line_ordinal.is_none())
        {
            return Err("captured native packet batch lacks its dispatch summary".into());
        }
        if !self.malformed.is_empty() {
            return Err(format!(
                "malformed captured native traces: {:?}",
                self.malformed
            ));
        }
        Ok(())
    }

    fn ingest_server_chunk(&mut self, text: &str, pending: &mut String, final_fragment: bool) {
        pending.push_str(text);
        while let Some(end) = pending.find('\n') {
            let line = pending[..end].trim_end_matches('\r').to_owned();
            pending.drain(..=end);
            self.ingest_server_text(&line);
        }
        if final_fragment && !pending.is_empty() {
            let partial = std::mem::take(pending);
            self.ingest_server_text(&partial);
            self.malformed
                .push(format!("unterminated scratch server log row: {partial}"));
        }
    }

    fn ingest_server_text(&mut self, text: &str) {
        for line in text.lines() {
            self.server_lines.push(line.to_owned());
            let server_line_ordinal = u64::try_from(self.server_lines.len()).unwrap_or(u64::MAX);
            if let Some((respawn, payload)) = line
                .strip_prefix("GRINCH_OBJ_REMOVE ")
                .map(|payload| (false, payload))
                .or_else(|| {
                    line.strip_prefix("GRINCH_OBJ_RESPAWN ")
                        .map(|payload| (true, payload))
                })
            {
                match serde_json::from_str::<ObjectLifecycleReceipt>(payload) {
                    Ok(receipt) => {
                        let event = ObjectLifecycleEvent {
                            server_line_ordinal,
                            receipt,
                        };
                        if respawn {
                            self.respawns.push(event);
                        } else {
                            self.removals.push(event);
                        }
                    }
                    Err(_) => self.malformed.push(format!("obj-lifecycle: {line}")),
                }
                continue;
            }
            if let Some((_, profile)) = line.split_once("\t INFO\t tick ") {
                let cycle = (|| {
                    let (cycle, stats) = profile.split_once(": ")?;
                    let (elapsed, heap) = stats.split_once("/600 ms, ")?;
                    elapsed.parse::<u64>().ok()?;
                    heap.strip_suffix(" MB heap")?.parse::<u64>().ok()?;
                    cycle.parse::<u64>().ok()
                })();
                match cycle {
                    Some(cycle) => self.debug_ticks.push(DebugTickEvent {
                        server_line_ordinal,
                        cycle,
                        raw_line: line.to_owned(),
                    }),
                    None => self.malformed.push(format!("debug-tick: {line}")),
                }
            } else if let Some(payload) = line.strip_prefix("GRINCH_CYCLE ") {
                if let Ok(value) = serde_json::from_str::<Value>(payload) {
                    self.cycles.push(CycleEvent {
                        server_line_ordinal,
                        payload: value,
                    });
                } else {
                    self.malformed.push(format!("cycle: {line}"));
                }
            } else if let Some(payload) = line.strip_prefix("GRINCH_CLIENT_OUTPUT ") {
                if let Some(event) = serde_json::from_str::<Value>(payload)
                    .ok()
                    .and_then(|value| {
                        Some(ClientOutputEvent {
                            server_line_ordinal,
                            cycle: value["cycle"].as_u64()?,
                            actor: actor_from_json(&value["actor"])?,
                            player_info_sequence: value["player_info_sequence"].as_u64()?,
                        })
                    })
                {
                    self.client_outputs.push(event);
                } else {
                    self.malformed.push(format!("client-output: {line}"));
                }
            } else if let Some(payload) = line.strip_prefix("GRINCH_TAKE_RECEIVE ") {
                if let Some(event) = serde_json::from_str::<Value>(payload)
                    .ok()
                    .and_then(|value| {
                        Some(TakeReceiveEvent {
                            receipt_sequence: value["receipt_sequence"].as_u64()?,
                            server_line_ordinal,
                            cycle: value["cycle"].as_u64()?,
                            actor: actor_from_json(&value["actor"])?,
                            item_id: i32::try_from(value["item_id"].as_i64()?).ok()?,
                            tile: tile_from_json(&value["tile"])?,
                            option: i32::try_from(value["option"].as_i64()?).ok()?,
                            action: value["action"].as_str().map(str::to_owned),
                            accepted: value["accepted"].as_bool()?,
                            reason: value["reason"].as_str()?.to_owned(),
                        })
                    })
                {
                    self.take_receives.push(event);
                } else {
                    self.malformed.push(format!("take-receive: {line}"));
                }
            }
        }
    }
}

fn run_key_from_json(value: &Value) -> Option<WireRunKey> {
    Some(WireRunKey {
        slot: value["slot"].as_u64()?,
        run: value["run"].as_u64()?,
        session: value["session"].as_u64()?,
    })
}

fn run_key_from_debug(text: &str) -> Option<WireRunKey> {
    let start = text.find("RunKey {")?;
    let end = text[start..].find('}')? + start;
    parse_run_key_fields(&text[start..=end])
}

fn parse_run_key_fields(text: &str) -> Option<WireRunKey> {
    Some(WireRunKey {
        slot: number_field(text, "slot:")?,
        run: number_field(text, "run:")?,
        session: number_field(text, "session:")?,
    })
}

fn number_field(text: &str, field: &str) -> Option<u64> {
    let start = text
        .match_indices(field)
        .find(|(offset, _)| {
            *offset == 0
                || (!text.as_bytes()[offset - 1].is_ascii_alphanumeric()
                    && text.as_bytes()[offset - 1] != b'_')
        })?
        .0
        + field.len();
    let digits = text[start..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok()
}

fn bool_field(text: &str, field: &str) -> Option<bool> {
    let start = text.find(field)? + field.len();
    let value = text[start..].trim_start();
    if value.starts_with("true") {
        Some(true)
    } else if value.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn optional_run_key_from_debug(text: &str) -> Option<Option<WireRunKey>> {
    let value = text.split_once("run=")?.1.trim_start();
    if value.starts_with("None") {
        Some(None)
    } else {
        Some(Some(run_key_from_debug(text)?))
    }
}

fn parse_native_packet(
    text: &str,
    host_tick: Option<u32>,
) -> Option<(Option<WireRunKey>, String, u64, u64, u8)> {
    let key = optional_run_key_from_debug(text)?;
    let account = word_field(text, "account=")?;
    number_field(text, "tick=")?;
    let tick = u64::from(host_tick?);
    let request = number_field(text, "request=")?;
    let opcode = u8::try_from(number_field(text, "opcode=")?).ok()?;
    Some((key, account, tick, request, opcode))
}

struct NativePacketSummary {
    key: Option<WireRunKey>,
    account: String,
    tick: u64,
    request: u64,
    count: u32,
    decoded: bool,
    accepted: bool,
}

fn parse_native_packets(text: &str, host_tick: Option<u32>) -> Option<NativePacketSummary> {
    let key = optional_run_key_from_debug(text)?;
    let account = word_field(text, "account=")?;
    number_field(text, "tick=")?;
    let tick = u64::from(host_tick?);
    let request = number_field(text, "request=")?;
    let count = u32::try_from(number_field(text, "count=")?).ok()?;
    let decoded = bool_field(text, "decoded=")?;
    let accepted = bool_field(text, "accepted=")?;
    Some(NativePacketSummary {
        key,
        account,
        tick,
        request,
        count,
        decoded,
        accepted,
    })
}

fn parse_bank_click(text: &str, host_tick: Option<u32>) -> Option<BankClickEvent> {
    number_field(text, "click_tick=")?;
    Some(BankClickEvent {
        key: run_key_from_debug(text)?,
        bank_generation: number_field(text, "bank_generation=")?,
        transfer: number_field(text, "transfer=")?,
        request: number_field(text, "request=")?,
        click_tick: u64::from(host_tick?),
        item_id: i32::try_from(number_field(text, "id=")?).ok()?,
        option: text
            .split_once(" option=")?
            .1
            .split_once(" operation=")?
            .0
            .to_owned(),
        operation: i32::try_from(number_field(text, "operation=")?).ok()?,
        main_before: u32::try_from(number_field(text, "main_before=")?).ok()?,
        host_log_ordinal: 0,
    })
}

fn parse_bank_dispatch(text: &str, host_tick: Option<u32>) -> Option<BankDispatchEvent> {
    number_field(text, "tick=")?;
    Some(BankDispatchEvent {
        key: run_key_from_debug(text)?,
        tick: u64::from(host_tick?),
        request: number_field(text, "request=")?,
        bank_generation: number_field(text, "bank_generation=")?,
        transfer: number_field(text, "transfer=")?,
        item_id: i32::try_from(number_field(text, "id=")?).ok()?,
        accepted: bool_field(text, "accepted=")?,
        emitted_packets: u32::try_from(number_field(text, "emitted_packets=")?).ok()?,
        opcode: number_field(text, "opcode=Some(").and_then(|value| u8::try_from(value).ok()),
        host_log_ordinal: 0,
    })
}
fn signed_field(text: &str, field: &str) -> Option<i32> {
    let start = text.find(field)? + field.len();
    let value = text[start..].trim_start();
    let length = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .count();
    value[..length].parse().ok()
}

fn quoted_field(text: &str, field: &str) -> Option<String> {
    let start = text.find(field)? + field.len();
    let value = text[start..].trim_start().strip_prefix('"')?;
    Some(value.split_once('"')?.0.to_owned())
}

fn parse_ground_request(text: &str) -> Option<GroundRequest> {
    if !text.starts_with("Obj {") {
        return None;
    }
    Some(GroundRequest {
        item_id: i32::try_from(number_field(text, "id: Some(")?).ok()?,
        tile: Tile {
            x: signed_field(text, "x:")?,
            z: signed_field(text, "z:")?,
            plane: signed_field(text, "level:")?,
        },
        action: quoted_field(text, "action:")?,
    })
}
fn debug_option_string(text: &str, field: &str) -> Option<String> {
    let value = text.split_once(field)?.1.trim_start();
    if value.starts_with("None") {
        return None;
    }
    let value = value
        .strip_prefix("Some(")?
        .trim_start()
        .strip_prefix('"')?;
    Some(value.split_once('"')?.0.to_owned())
}

fn parse_interaction_kind(text: &str, ground: Option<GroundRequest>) -> InteractionKind {
    if let Some(ground) = ground {
        return InteractionKind::Ground(ground);
    }
    if text.starts_with("OpenBooth {") || text.starts_with("OpenStand {") {
        let tile = match (
            signed_field(text, "x:"),
            signed_field(text, "z:"),
            signed_field(text, "level:"),
        ) {
            (Some(x), Some(z), Some(plane)) => Some(Tile { x, z, plane }),
            _ => None,
        };
        return InteractionKind::OpenBank {
            tile,
            name: debug_option_string(text, "name:"),
            id: signed_field(text, "id:"),
        };
    }
    if text.trim() == "Close" {
        return InteractionKind::CloseBank;
    }
    InteractionKind::Other
}

fn parse_watchdog_action(text: &str, row: &ObservedLogRecord) -> Option<WatchdogEvent> {
    let action = text.split_once("action=")?.1.trim();
    let known = matches!(
        action,
        "WarnHungLoop" | "WarnNoGamePackets" | "HoldInFightPen" | "RequestAnchor" | "AbortWalk"
    ) || action.starts_with("ArmWalk { x:")
        || action.starts_with("Restart { reason: Stall")
        || action.starts_with("Restart { reason: Wedge")
        || action.starts_with("Restart { reason: WedgeWalk");
    if !known {
        return None;
    }
    number_field(text, "tick=")?;
    Some(WatchdogEvent {
        key: run_key_from_debug(text),
        tick: Some(u64::from(row.tick?)),
        action: action.to_owned(),
        raw_record: text.to_owned(),
        restarted: action.starts_with("Restart {"),
        slot_name: row.slot.clone(),
        host_tick: row.tick,
        host_log_ordinal: row.ordinal,
        source: row.source.clone(),
        level: row.level.clone(),
    })
}

fn word_field(text: &str, field: &str) -> Option<String> {
    let start = text.find(field)? + field.len();
    Some(
        text[start..]
            .split_whitespace()
            .next()?
            .trim_end_matches(',')
            .to_owned(),
    )
}

fn actor_from_json(value: &Value) -> Option<ActorRef> {
    Some(ActorRef {
        account_id: value["account_id"].as_str()?.to_owned(),
        player_name: value["player_name"].as_str()?.to_owned(),
    })
}

fn tile_from_json(value: &Value) -> Option<Tile> {
    Some(Tile {
        x: i32::try_from(value["x"].as_i64()?).ok()?,
        z: i32::try_from(value["z"].as_i64()?).ok()?,
        plane: i32::try_from(
            value
                .get("plane")
                .or_else(|| value.get("level"))?
                .as_i64()?,
        )
        .ok()?,
    })
}

fn parse_run_key_from_play(key: api::selected::RunKey) -> WireRunKey {
    key.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepStage {
    WaitIngame,
    WaitInitialTutorial,
    Relog,
    ReseedTutorial,
    WaitReseedConfirm,
    WaitAnchor,
    IssueSeed(usize),
    OpenBank,
    WaitBank,
    Withdraw(usize),
    CloseBank,
    SampleOpen,
    SampleWait,
    SampleClose,
    Ready,
}

#[derive(Debug, Clone)]
struct PendingWithdraw {
    item_id: i32,
    expected_inventory_count: u32,
}

#[derive(Debug, Clone)]
struct PendingDrop {
    item_id: i32,
    tile: Tile,
    prior_count: u32,
    request_key: String,
    send_tick: u64,
    helper_send_player_info_sequence: u64,
    helper_send_observation_sequence: u64,
    receiver_sequence: u64,
    packet_opcodes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct DropReceipt {
    request_key: String,
    send_tick: u64,
    helper_send_player_info_sequence: u64,
    helper_send_observation_sequence: u64,
    receiver_confirmation_sequence: u64,
    receiver_tick: u64,
    receiver_object_turn: u64,
    receiver_sequence: u64,
    receiver_player_info_sequence: u64,
    helper_confirmation_tick: u64,
    helper_player_info_sequence: u64,
    helper_observation_sequence: u64,
    packet_opcodes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PendingCompetitorTake {
    item_id: i32,
    tile: Tile,
    request_key: String,
    tick: u64,
    observation_sequence: u64,
    player_info_sequence: u64,
    scene_generation: u64,
    observed_ground: GroundRow,
    packet_opcodes: Vec<u8>,
}

impl PendingCompetitorTake {
    fn has_exact_send_evidence(&self, player_info: &RawFrame) -> bool {
        one_obj3_packet(&self.packet_opcodes)
            && self.tick == player_info.tick
            && self.observation_sequence == player_info.observation_sequence
            && self.player_info_sequence == player_info.player_info_sequence
            && self.scene_generation >= player_info.scene_generation
            && self.observed_ground.item_id == self.item_id
            && self.observed_ground.tile == self.tile
            && self.observed_ground.quantity > 0
    }
}

struct ActorState {
    role: ActorRole,
    account: String,
    expected_player_name: String,
    seed: Vec<(i32, u32)>,
    sample_bank: bool,
    pump: Pump,
    snapshot: GameSnapshot,
    prep: PrepStage,
    relogger: ScenarioRunner,
    initial_tutorial_baseline: Option<i32>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    issued_seed: usize,
    seed_commands: Vec<Value>,
    seed_observation: Option<RawFrame>,
    bank_sample: Option<ItemSnapshot>,
    pending_withdraw: Option<PendingWithdraw>,
    setup_note_mode: bool,
    raw_frames: Vec<RawFrame>,
    latest: Option<RawFrame>,
    error: Option<String>,
    helper_drops: Vec<DropSpec>,
    helper_drop_index: usize,
    pending_drop: Option<PendingDrop>,
    drop_actions: Vec<DropAction>,
    drop_receipts: Vec<DropReceipt>,
    helper_post_drop: Option<RawFrame>,
    competitor_sent_on_presence: bool,
    competitor_pending: Option<PendingCompetitorTake>,
    competitor_takes: Vec<PendingCompetitorTake>,
}

fn fresh_account_relogger() -> ScenarioRunner {
    let side_tab = Proof::SideTabAvailable { index: 3 };
    ScenarioRunner::with_world(
        Scenario {
            name: "grinch_tutorial_relog",
            seed: Seed {
                profiles: Vec::new(),
                mainland: false,
            },
            steps: vec![Step {
                name: "clean relog after tutorial skip",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: side_tab,
                    budget_ticks: 600,
                },
            }],
            proof: side_tab,
            companions: Vec::new(),
            settings: ScenarioSettings {
                deadline: Duration::from_secs(90),
                require_mainland_base: false,
                ..ScenarioSettings::default()
            },
        },
        None,
    )
}

impl ActorState {
    fn new(
        role: ActorRole,
        account: String,
        seed: Vec<(i32, u32)>,
        sample_bank: bool,
        helper_drops: Vec<DropSpec>,
    ) -> Self {
        Self {
            expected_player_name: JString::to_screen_name(&account),
            role,
            account,
            seed,
            sample_bank,
            pump: Pump::new(),
            snapshot: GameSnapshot::new(),
            prep: PrepStage::WaitIngame,
            relogger: fresh_account_relogger(),
            initial_tutorial_baseline: None,
            tutorial_reseed: None,
            issued_seed: 0,
            seed_commands: Vec::new(),
            seed_observation: None,
            bank_sample: None,
            pending_withdraw: None,
            setup_note_mode: false,
            raw_frames: Vec::new(),
            latest: None,
            error: None,
            helper_drops,
            helper_drop_index: 0,
            pending_drop: None,
            drop_actions: Vec::new(),
            drop_receipts: Vec::new(),
            helper_post_drop: None,
            competitor_sent_on_presence: false,
            competitor_pending: None,
            competitor_takes: Vec::new(),
        }
    }

    fn ready(&self) -> bool {
        self.prep == PrepStage::Ready
    }

    fn send_setup_cheat(
        &mut self,
        client: &mut Client,
        observation_sequence: u64,
        command: &str,
    ) -> Result<(), String> {
        if !interact::cheat(client, command).is_sent() {
            return Err(format!(
                "fresh-account setup command was refused: {command}"
            ));
        }
        self.seed_commands.push(json!({
            "observation_sequence": observation_sequence,
            "actor": self.account,
            "command": command,
            "accepted": true,
        }));
        Ok(())
    }
}

#[derive(Clone)]
struct ActorReport {
    role: ActorRole,
    account: String,
    frames: Vec<RawFrame>,
    seed_observation: Option<RawFrame>,
    drop_actions: Vec<DropAction>,
    drop_receipts: Vec<DropReceipt>,
    helper_post_drop: Option<RawFrame>,
    competitor_takes: Vec<PendingCompetitorTake>,
    seed_commands: Vec<Value>,
    bank_sample: Option<ItemSnapshot>,
    error: Option<String>,
}
#[derive(Debug, Clone)]
struct RuntimeObservation {
    sample_ordinal: u64,
    host_log_ordinal_fence: u64,
    observation_sequence: u64,
    account: String,
    client_session_id: Option<String>,
    client_session_generation: Option<u64>,
    player_info_sequence: Option<u64>,
    tick: Option<u64>,
    start_requested: bool,
    generation: Option<u64>,
    run_key: Option<WireRunKey>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BankInventoryPacketObservation {
    component: i32,
    full_generation: u64,
    full_observation: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BankCallbackObservation {
    sequence: u64,
    host_log_ordinal_fence: u64,
    tick: Option<u64>,
    player_info_changed: bool,
    session_generation: u64,
    bank_generation: u64,
    bank_component: i32,
    modal_stamp: u64,
    bank_loaded: bool,
    bank_snapshot: Option<ItemSnapshot>,
    inventory_packets: Vec<BankInventoryPacketObservation>,
}

#[derive(Clone)]
struct RuntimeReport {
    case: Case,
    actors: Vec<ActorReport>,
    observation_sequence: u64,
    run_key: Option<WireRunKey>,
    terminal_stop: Option<TerminalNativeStop>,
    start_requested: bool,
    start_observation_sequence: Option<u64>,
    pre_start_frame: Option<RawFrame>,
    runtime_observations: Vec<RuntimeObservation>,
    bank_callbacks: Vec<BankCallbackObservation>,
    setup_error: Option<String>,
}

struct Runtime {
    case: Case,
    actors: Vec<ActorState>,
    main_index: usize,
    helper_index: Option<usize>,
    competitor_index: Option<usize>,
    observation_sequence: u64,
    run_key: Option<WireRunKey>,
    terminal_stop: Option<TerminalNativeStop>,
    start_requested: bool,
    start_observation_sequence: Option<u64>,
    pre_start_frame: Option<RawFrame>,
    runtime_observations: Vec<RuntimeObservation>,
    bank_callbacks: Vec<BankCallbackObservation>,
    error: Option<String>,
    drop_sequence: u64,
    competitor_sequence: u64,
}

impl Runtime {
    fn report(&self) -> RuntimeReport {
        RuntimeReport {
            case: self.case,
            actors: self
                .actors
                .iter()
                .map(|actor| ActorReport {
                    role: actor.role,
                    account: actor.account.clone(),
                    frames: actor.raw_frames.clone(),
                    seed_observation: actor.seed_observation.clone(),
                    drop_actions: actor.drop_actions.clone(),
                    drop_receipts: actor.drop_receipts.clone(),
                    helper_post_drop: actor.helper_post_drop.clone(),
                    competitor_takes: actor.competitor_takes.clone(),
                    seed_commands: actor.seed_commands.clone(),
                    bank_sample: actor.bank_sample.clone(),
                    error: actor.error.clone(),
                })
                .collect(),
            observation_sequence: self.observation_sequence,
            run_key: self.run_key,
            terminal_stop: self.terminal_stop.clone(),
            start_requested: self.start_requested,
            start_observation_sequence: self.start_observation_sequence,
            pre_start_frame: self.pre_start_frame.clone(),
            runtime_observations: self.runtime_observations.clone(),
            bank_callbacks: self.bank_callbacks.clone(),
            setup_error: self.error.clone(),
        }
    }

    fn actor_index(&self, account: &str) -> Option<usize> {
        self.actors
            .iter()
            .position(|actor| actor.account == account)
    }

    fn actor_ready(&self, role: ActorRole) -> bool {
        self.actors
            .iter()
            .find(|actor| actor.role == role)
            .is_some_and(ActorState::ready)
    }

    fn on_frame(
        &mut self,
        client: &mut Client,
        account: &str,
        hold: bool,
        data: &api::game_data::SelectedGameData,
    ) {
        let Some(index) = self.actor_index(account) else {
            self.error = Some(format!(
                "received an output frame for unregistered actor {account}"
            ));
            return;
        };
        let drain = self.actors[index].pump.drain_client(client);
        host::publish_snapshot(&mut self.actors[index].snapshot, client, drain);
        let raw = if drain.player_info {
            let raw = match capture_raw_frame(
                &self.actors[index],
                client,
                self.observation_sequence.saturating_add(1),
            ) {
                Ok(raw) => raw,
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            };
            self.observation_sequence = raw.observation_sequence;
            let actor = &mut self.actors[index];
            actor.latest = Some(raw.clone());
            actor.raw_frames.push(raw.clone());
            Some(raw)
        } else {
            None
        };
        if self.case.is_c() && index == self.main_index {
            if let Err(error) = record_bank_callback(self, client, drain.player_info) {
                self.error = Some(error);
                return;
            }
        }
        if self.actors[index].prep == PrepStage::Relog {
            let actor = &mut self.actors[index];
            actor.relogger.tick_with_hold(client, hold);
            match actor.relogger.status() {
                RunnerStatus::Passed => actor.prep = PrepStage::ReseedTutorial,
                RunnerStatus::Failed(error) => {
                    self.error = Some(format!("fresh-account tutorial relog failed: {error}"));
                }
                RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
            }
            return;
        }
        if hold || self.error.is_some() {
            return;
        }
        if let Some(raw) = raw {
            if let Err(error) = self.advance_prep(index, client, &raw, data) {
                self.error = Some(error);
                return;
            }
            if Some(index) == self.helper_index {
                if let Err(error) = self.advance_helper(index, client, &raw) {
                    self.error = Some(error);
                    return;
                }
            }
        }
        if Some(index) == self.competitor_index {
            if let Err(error) = self.advance_competitor(index, client) {
                self.error = Some(error);
            }
        }
    }

    fn on_post_native_player_info(
        &mut self,
        client: &Client,
        account: &str,
        handle: &ScriptStartHandle,
    ) {
        let Some(index) = self.actor_index(account) else {
            self.error = Some(format!(
                "received a completed frame for unregistered actor {account}"
            ));
            return;
        };
        if index != self.main_index {
            return;
        }
        if !self.actors[index]
            .latest
            .as_ref()
            .is_some_and(|raw| raw.player_info_sequence == client.gens.player_info)
        {
            // Only an actually captured PLAYER_INFO can own this observation.
            return;
        }
        let (generation, run_key) = handle
            .observe_native_runtime(account)
            .map_or((None, None), |(generation, run_key)| {
                (Some(generation), run_key.map(parse_run_key_from_play))
            });
        if let Err(error) = record_runtime_observation(self, generation, run_key) {
            self.error = Some(error);
            return;
        }
        if self.error.is_none() && !self.start_requested && self.should_start() {
            // The control thread may dispatch Start only after this frame's
            // native work has completed. The observer itself never starts it.
            self.pre_start_frame = self.actors[index].latest.clone();
        }
    }

    fn start_if_ready(&mut self, handle: &ScriptStartHandle) {
        if self.error.is_some() || self.pre_start_frame.is_none() || !self.should_start() {
            return;
        }
        let account = &self.actors[self.main_index].account;
        match handle.start_compiled(
            account,
            script::CompiledId("Grinch"),
            card_settings(self.case),
        ) {
            Ok(()) => {
                self.start_requested = true;
                self.start_observation_sequence = self
                    .pre_start_frame
                    .as_ref()
                    .map(|frame| frame.observation_sequence);
            }
            Err(error) => self.error = Some(format!("compiled Grinch Start refused: {error}")),
        }
    }

    fn observe_terminal_stop(
        &mut self,
        status: &script::native::ScriptStatus,
        reported_error: &str,
        logs: &ObservedLogs,
    ) -> Result<bool, String> {
        let failure = status
            .failure
            .as_ref()
            .ok_or("terminal native status has no failure")?;
        if self.case != Case::CInventoryBlocked
            || !self.start_requested
            || status.card != script::CompiledId("Grinch")
            || status.phase != script::native::NativePhase::Blocked
            || failure.code.as_ref() != "inventory-blocked"
            || failure.message.as_ref() != reported_error
        {
            return Err(format!(
                "unexpected native terminal failure: {reported_error}"
            ));
        }
        let key = WireRunKey::from(status.run);
        if self.run_key.is_some_and(|observed| observed != key) {
            return Err("retained terminal status belongs to a different native run".into());
        }
        if self.terminal_stop.is_some() {
            return Ok(true);
        }
        let main = &self.actors[self.main_index];
        let Some(latest) = main.latest.as_ref() else {
            return Ok(false);
        };
        let Some(after) = runtime_sample_for_frame(&self.runtime_observations, latest) else {
            return Ok(false);
        };
        if !after.start_requested {
            return Ok(false);
        }
        let pre = self
            .pre_start_frame
            .as_ref()
            .ok_or("terminal stop lacks its pre-Start frame")?;
        let before = runtime_sample_for_frame(&self.runtime_observations, pre)
            .ok_or("terminal stop lacks its exact pre-Start runtime sample")?;
        let before_generation = before.generation.ok_or("pre-Start generation is absent")?;
        let after_generation = after.generation.ok_or("terminal generation is absent")?;
        if before.start_requested
            || before.run_key.is_some()
            || after.run_key.is_some()
            || before_generation.checked_add(1) != Some(after_generation)
        {
            return Err(
                "terminal stop does not retain the observed Start generation and absent active key"
                    .into(),
            );
        }
        let Some(state) = state_for_frame(
            logs,
            &main.account,
            latest.tick,
            key,
            after.host_log_ordinal_fence,
        ) else {
            return Ok(false);
        };
        if parse_status_core(state).is_none_or(|state| state.phase != StatusPhase::InventoryBlocked)
            || state.payload["blocked_reason"].as_str() != Some("no_selected_item_to_deposit")
        {
            return Err("retained native stop disagrees with its exact Grinch state row".into());
        }
        self.terminal_stop = Some(TerminalNativeStop {
            run_slot: key.slot,
            run_number: key.run,
            native_session: key.session,
            card: status.card.0.to_owned(),
            native_phase: format!("{:?}", status.phase),
            failure_code: failure.code.to_string(),
            failure_message: failure.message.to_string(),
            status_read_observation_sequence: latest.observation_sequence,
            generation_before_start: before_generation,
            generation_after_stop: after_generation,
            active_run_key_after_stop: after.run_key.map(|key| key.run_id(&main.account)),
        });
        // This is the retained, stopped run's real key, not an active-key sample.
        self.run_key = Some(key);
        Ok(true)
    }

    fn advance_prep(
        &mut self,
        index: usize,
        client: &mut Client,
        raw: &RawFrame,
        data: &api::game_data::SelectedGameData,
    ) -> Result<(), String> {
        let actor = &mut self.actors[index];
        if actor.prep == PrepStage::WaitIngame {
            if !actor.snapshot.ingame() || actor.snapshot.scene_state() != 2 {
                return Ok(());
            }
            actor.initial_tutorial_baseline =
                Some(scenario::tutorial::chat_baseline(&actor.snapshot));
            for command in [
                scenario::tutorial::TUTORIAL_SETVAR,
                scenario::tutorial::TUTORIAL_GETVAR,
            ] {
                actor.send_setup_cheat(client, raw.observation_sequence, command)?;
            }
            actor.prep = PrepStage::WaitInitialTutorial;
            return Ok(());
        }
        if actor.prep == PrepStage::WaitInitialTutorial {
            let baseline = actor
                .initial_tutorial_baseline
                .ok_or("initial tutorial proof lacks its chat baseline")?;
            if scenario::tutorial::tutorial_confirmed(&actor.snapshot, baseline) {
                actor.prep = PrepStage::Relog;
            }
            return Ok(());
        }
        if actor.prep == PrepStage::ReseedTutorial {
            let reseed = scenario::tutorial::PostRelogTutorial::new(
                scenario::tutorial::chat_baseline(&actor.snapshot),
            );
            for command in [
                scenario::tutorial::TUTORIAL_SETVAR,
                scenario::tutorial::TUTORIAL_GETVAR,
            ] {
                actor.send_setup_cheat(client, raw.observation_sequence, command)?;
            }
            actor.tutorial_reseed = Some(reseed);
            actor.prep = PrepStage::WaitReseedConfirm;
            return Ok(());
        }
        if actor.prep == PrepStage::WaitReseedConfirm {
            let reseed = actor
                .tutorial_reseed
                .as_ref()
                .ok_or("post-relog tutorial proof was not armed")?;
            if !reseed.check(&actor.snapshot)? {
                return Ok(());
            }
            actor.send_setup_cheat(
                client,
                raw.observation_sequence,
                &interact::tele_args(ANCHOR.plane, ANCHOR.x, ANCHOR.z),
            )?;
            actor.prep = PrepStage::WaitAnchor;
            return Ok(());
        }
        if actor.prep == PrepStage::WaitAnchor {
            if actor.snapshot.tile() != Some((ANCHOR.x, ANCHOR.z, ANCHOR.plane)) {
                return Ok(());
            }
            actor.prep = if actor.seed.is_empty() {
                if actor.sample_bank {
                    PrepStage::SampleOpen
                } else {
                    PrepStage::Ready
                }
            } else {
                PrepStage::IssueSeed(0)
            };
        }
        match actor.prep {
            PrepStage::IssueSeed(next) => {
                if next < actor.seed.len() {
                    let (id, count) = actor.seed[next];
                    let alias = item_alias(data, id)?;
                    let command = format!("givebank {alias} {count}");
                    actor.send_setup_cheat(client, raw.observation_sequence, &command)?;
                    actor.issued_seed += 1;
                    actor.prep = PrepStage::IssueSeed(next + 1);
                } else {
                    actor.prep = PrepStage::OpenBank;
                }
            }
            PrepStage::OpenBank => {
                if actor.snapshot.bank_component_id() >= 0 {
                    actor.prep = PrepStage::WaitBank;
                    return Ok(());
                }
                let checkpoint = client.packet_checkpoint();
                let result = Interactions::new(&actor.snapshot, client).open_nearest_booth();
                let tick = sent_tick(result, "open setup bank")?;
                verify_sent_packet(client, checkpoint, tick, "open setup bank")?;
                actor.prep = PrepStage::WaitBank;
            }
            PrepStage::WaitBank => {
                if actor.snapshot.bank_component_id() < 0 || !actor.snapshot.bank_loaded() {
                    return Ok(());
                }
                actor.prep = PrepStage::Withdraw(0);
            }
            PrepStage::Withdraw(next) => {
                if let Some(pending) = actor.pending_withdraw.as_ref() {
                    if inventory_count(&actor.snapshot, pending.item_id)
                        < pending.expected_inventory_count
                    {
                        return Ok(());
                    }
                    actor.pending_withdraw = None;
                    actor.prep = PrepStage::Withdraw(next + 1);
                    return Ok(());
                }
                if next >= actor.seed.len() {
                    let snapshot = item_snapshot(&actor.snapshot, false, client.gens.inv > 0)?;
                    if !snapshot_matches_seed(&snapshot, &actor.seed) {
                        return Err(format!("observed seeded inventory differs from requested fresh-account setup for {}: {:?}", actor.account, snapshot.quantities));
                    }
                    if actor.issued_seed != actor.seed.len() {
                        return Err(
                            "setup seed command count does not match the complete seed plan".into(),
                        );
                    }
                    if actor.seed_observation.is_none() {
                        actor.seed_observation = Some(raw.clone());
                    }
                    actor.prep = PrepStage::CloseBank;
                    return Ok(());
                }
                let (item_id, expected_count) = actor.seed[next];
                let current = inventory_count(&actor.snapshot, item_id);
                if current >= expected_count {
                    actor.prep = PrepStage::Withdraw(next + 1);
                    return Ok(());
                }
                let note_mode = data
                    .item_by_id(item_id)
                    .ok_or_else(|| format!("setup item {item_id} lacks selected content"))?
                    .is_certificate();
                if actor.setup_note_mode != note_mode {
                    let checkpoint = client.packet_checkpoint();
                    let result =
                        Interactions::new(&actor.snapshot, client).set_note_mode(note_mode);
                    let tick = sent_tick(result, "select setup withdrawal form")?;
                    verify_sent_packet(client, checkpoint, tick, "select setup withdrawal form")?;
                    // A new bank session starts in Item mode. Wait for the
                    // next PLAYER_INFO before withdrawing in the selected form.
                    actor.setup_note_mode = note_mode;
                    return Ok(());
                }
                let row = actor.snapshot.bank().iter().find(|item| item.def.id == item_id && item.count > 0)
                    .ok_or_else(|| format!("bank setup row {item_id} is absent before the requested inventory count was observed"))?;
                let (label, amount) = withdraw_action(row).ok_or_else(|| {
                    format!("bank row {item_id} has no observed Withdraw-All/Withdraw-N action")
                })?;
                let target = current.saturating_add(amount).min(expected_count);
                let checkpoint = client.packet_checkpoint();
                let result = Interactions::new(&actor.snapshot, client)
                    .interact(OpTarget::Item(row), ActionSpec::Label(label));
                let tick = sent_tick(result, "withdraw setup item")?;
                verify_sent_packet(client, checkpoint, tick, "withdraw setup item")?;
                actor.pending_withdraw = Some(PendingWithdraw {
                    item_id,
                    expected_inventory_count: target,
                });
            }
            PrepStage::CloseBank => {
                if actor.snapshot.bank_component_id() < 0 {
                    actor.prep = if actor.sample_bank && actor.bank_sample.is_none() {
                        PrepStage::SampleOpen
                    } else {
                        PrepStage::Ready
                    };
                    return Ok(());
                }
                let checkpoint = client.packet_checkpoint();
                let result = Interactions::new(&actor.snapshot, client).close_modal();
                let tick = sent_tick(result, "close setup bank")?;
                verify_sent_packet(client, checkpoint, tick, "close setup bank")?;
            }
            PrepStage::SampleOpen => {
                if actor.snapshot.bank_component_id() >= 0 {
                    actor.prep = PrepStage::SampleWait;
                    return Ok(());
                }
                let checkpoint = client.packet_checkpoint();
                let result = Interactions::new(&actor.snapshot, client).open_nearest_booth();
                let tick = sent_tick(result, "open bank for baseline snapshot")?;
                verify_sent_packet(client, checkpoint, tick, "open bank for baseline snapshot")?;
                actor.prep = PrepStage::SampleWait;
            }
            PrepStage::SampleWait => {
                if actor.snapshot.bank_component_id() < 0 || !actor.snapshot.bank_loaded() {
                    return Ok(());
                }
                let bank = bank_snapshot(&actor.snapshot)?;
                actor.bank_sample = Some(bank);
                actor.prep = PrepStage::SampleClose;
            }
            PrepStage::SampleClose => {
                if actor.snapshot.bank_component_id() < 0 {
                    actor.prep = PrepStage::Ready;
                    return Ok(());
                }
                let checkpoint = client.packet_checkpoint();
                let result = Interactions::new(&actor.snapshot, client).close_modal();
                let tick = sent_tick(result, "close bank after baseline snapshot")?;
                verify_sent_packet(
                    client,
                    checkpoint,
                    tick,
                    "close bank after baseline snapshot",
                )?;
            }
            PrepStage::Ready
            | PrepStage::WaitIngame
            | PrepStage::WaitInitialTutorial
            | PrepStage::Relog
            | PrepStage::ReseedTutorial
            | PrepStage::WaitReseedConfirm
            | PrepStage::WaitAnchor => {}
        }
        if actor.ready() && actor.seed_observation.is_none() {
            actor.seed_observation = Some(raw.clone());
        }
        Ok(())
    }

    fn should_start(&self) -> bool {
        if self.start_requested || !self.actor_ready(ActorRole::Main) {
            return false;
        }
        if self.case.uses_helper() {
            let Some(index) = self.helper_index else {
                return false;
            };
            if !self.actors[index].ready() || self.actors[index].seed_observation.is_none() {
                return false;
            }
            let helper_seed = self.actors[index]
                .seed_observation
                .as_ref()
                .unwrap()
                .observation_sequence;
            let main_latest = self.actors[self.main_index].latest.as_ref();
            if !main_latest.is_some_and(|frame| frame.observation_sequence > helper_seed) {
                return false;
            }
        }
        if self.case.is_c() && self.case != Case::CInventoryBlocked {
            let Some(index) = self.helper_index else {
                return false;
            };
            let receiver_seed = self.actors[self.main_index].seed_observation.as_ref();
            let helper_seed = self.actors[index].seed_observation.as_ref();
            let Some(main_latest) = self.actors[self.main_index].latest.as_ref() else {
                return false;
            };
            if !receiver_seed
                .is_some_and(|seed| seed.observation_sequence < main_latest.observation_sequence)
                || !helper_seed.is_some_and(|seed| {
                    seed.observation_sequence < main_latest.observation_sequence
                })
            {
                return false;
            }
        }
        if self.case == Case::CInventoryBlocked {
            let main_seed = self.actors[self.main_index].seed_observation.as_ref();
            let Some(main_latest) = self.actors[self.main_index].latest.as_ref() else {
                return false;
            };
            if !main_seed
                .is_some_and(|seed| seed.observation_sequence < main_latest.observation_sequence)
            {
                return false;
            }
        }
        if self.case.uses_competitor() {
            let Some(index) = self.competitor_index else {
                return false;
            };
            if !self.actors[index].ready() {
                return false;
            }
            let competitor_sequence = self.actors[index]
                .latest
                .as_ref()
                .map(|frame| frame.observation_sequence);
            let main_sequence = self.actors[self.main_index]
                .latest
                .as_ref()
                .map(|frame| frame.observation_sequence);
            if !matches!((competitor_sequence, main_sequence), (Some(comp), Some(main)) if comp < main)
            {
                return false;
            }
        }
        true
    }

    fn advance_helper(
        &mut self,
        helper_index: usize,
        client: &mut Client,
        helper_frame: &RawFrame,
    ) -> Result<(), String> {
        if !self.start_requested || self.run_key.is_none() || self.helper_index.is_none() {
            return Ok(());
        }
        if let Some(pending) = self.actors[helper_index].pending_drop.clone() {
            let helper = &self.actors[helper_index];
            let current = inventory_count(&helper.snapshot, pending.item_id);
            if current == pending.prior_count {
                return Ok(());
            }
            if current + 1 != pending.prior_count {
                return Err(format!(
                    "helper inventory changed by an unexpected quantity after real Drop {}",
                    pending.item_id
                ));
            }
            let helper_latest = helper
                .latest
                .as_ref()
                .ok_or("helper Drop confirmation lacks its PLAYER_INFO frame")?;
            let main = &self.actors[self.main_index];
            let receiver_confirmation = main
                .latest
                .as_ref()
                .ok_or("Grinch lacks a receiver frame after the helper Drop send")?;
            if receiver_confirmation.observation_sequence <= pending.receiver_sequence {
                return Ok(());
            }
            let helper_send = helper
                .raw_frames
                .iter()
                .find(|frame| {
                    frame.observation_sequence == pending.helper_send_observation_sequence
                        && frame.player_info_sequence == pending.helper_send_player_info_sequence
                })
                .ok_or("helper Drop lost its exact send PLAYER_INFO observation")?;
            let Some(send_object_turn) = helper_send.object_turn else {
                return Ok(());
            };
            let mut receiver_sends = main
                .raw_frames
                .iter()
                .filter(|frame| frame.object_turn == Some(send_object_turn));
            let Some(receiver) = receiver_sends.next() else {
                return Ok(());
            };
            if receiver_sends.next().is_some() {
                return Err("helper Drop send matches ambiguous receiver output edges".into());
            }
            if receiver_confirmation.observation_sequence <= receiver.observation_sequence
                || receiver_confirmation
                    .object_turn
                    .is_none_or(|turn| turn <= send_object_turn)
            {
                return Ok(());
            }
            let actor = helper_latest.actor.clone();
            let confirmation_tick = helper_latest.tick;
            let helper_player_info_sequence = helper_latest.player_info_sequence;
            let helper_observation_sequence = helper_latest.observation_sequence;
            let receiver_tick = receiver.tick;
            let Some(receiver_object_turn) = receiver.object_turn else {
                return Ok(());
            };
            let receiver_sequence = receiver.observation_sequence;
            let receiver_player_info_sequence = receiver.player_info_sequence;
            let action = DropAction {
                request_key: pending.request_key.clone(),
                actor,
                item_id: pending.item_id,
                tile: pending.tile,
                quantity: 1,
                tick: receiver_tick,
                object_turn: receiver_object_turn,
                source: DropSource::RealPlayerInventoryDrop,
            };
            let receipt = DropReceipt {
                request_key: pending.request_key,
                send_tick: pending.send_tick,
                helper_send_player_info_sequence: pending.helper_send_player_info_sequence,
                helper_send_observation_sequence: pending.helper_send_observation_sequence,
                receiver_confirmation_sequence: receiver_confirmation.observation_sequence,
                receiver_tick,
                receiver_object_turn,
                receiver_sequence,
                receiver_player_info_sequence,
                helper_confirmation_tick: confirmation_tick,
                helper_player_info_sequence,
                helper_observation_sequence,
                packet_opcodes: pending.packet_opcodes,
            };
            let actor = &mut self.actors[helper_index];
            actor.drop_actions.push(action);
            actor.drop_receipts.push(receipt);
            actor.pending_drop = None;
            actor.helper_drop_index += 1;
            if actor.helper_drop_index == actor.helper_drops.len() {
                let final_inventory = item_snapshot(&actor.snapshot, false, client.gens.inv > 0)?;
                if final_inventory.occupied_slots != Some(0) {
                    return Err(
                        "helper real Drop sequence did not empty the seeded inventory".into(),
                    );
                }
                actor.helper_post_drop = actor.latest.clone();
            }
            return Ok(());
        }

        let main = self.actors[self.main_index]
            .latest
            .as_ref()
            .ok_or("Grinch has no observed PLAYER_INFO frame before helper Drop")?;
        let main_tick = main.tick;
        let main_observation_sequence = main.observation_sequence;
        let main_position = main.position;
        let main_bank_open = main.bank_open;
        let main_account = self.actors[self.main_index].account.clone();
        if self.case.is_c() {
            let helper = &self.actors[helper_index];
            if helper.helper_drop_index >= helper.helper_drops.len() {
                return Ok(());
            }
            if helper.helper_drop_index > 0
                && (main_position != ANCHOR
                    || main_bank_open
                    || latest_status_phase(&main_account, self.run_key, main_tick)
                        != Some(StatusPhase::Waiting)
                    || !main_observed_closed_after_deposit(
                        &self.actors[self.main_index],
                        self.run_key,
                        main_tick,
                    ))
            {
                return Ok(());
            }
        }
        let actor = &mut self.actors[helper_index];
        if actor.helper_drop_index >= actor.helper_drops.len() {
            return Ok(());
        }
        let spec = actor.helper_drops[actor.helper_drop_index];
        if actor.snapshot.tile() != Some((spec.tile.x, spec.tile.z, spec.tile.plane)) {
            let tile = WorldTile {
                x: spec.tile.x,
                z: spec.tile.z,
                level: spec.tile.plane,
            };
            let checkpoint = client.packet_checkpoint();
            let result = Interactions::new(&actor.snapshot, client).walk(tile);
            let tick = sent_tick(result, "walk helper to Drop tile")?;
            verify_sent_packet(client, checkpoint, tick, "walk helper to Drop tile")?;
            return Ok(());
        }
        let item = actor
            .snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == spec.item_id && item.count > 0)
            .ok_or_else(|| format!("helper inventory lacks real Drop item {}", spec.item_id))?;
        let label = item
            .actions
            .iter()
            .flatten()
            .find(|label| label.eq_ignore_ascii_case("Drop"))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "helper inventory row {} has no observed Drop action",
                    spec.item_id
                )
            })?;
        let prior_count = inventory_count(&actor.snapshot, spec.item_id);
        let checkpoint = client.packet_checkpoint();
        let result = Interactions::new(&actor.snapshot, client)
            .interact(OpTarget::Item(item), ActionSpec::Label(label));
        let tick = sent_tick(result, "real helper Drop")?;
        let (send_tick, opcodes) =
            verified_send_packet(client, checkpoint, tick, "real helper Drop")?;
        self.drop_sequence = self.drop_sequence.saturating_add(1);
        let request_key = format!("helper-drop:{}:{}", actor.account, self.drop_sequence);
        actor.pending_drop = Some(PendingDrop {
            item_id: spec.item_id,
            tile: spec.tile,
            prior_count,
            request_key,
            send_tick,
            helper_send_player_info_sequence: helper_frame.player_info_sequence,
            helper_send_observation_sequence: helper_frame.observation_sequence,
            receiver_sequence: main_observation_sequence,
            packet_opcodes: opcodes,
        });
        Ok(())
    }

    fn advance_competitor(
        &mut self,
        competitor_index: usize,
        client: &mut Client,
    ) -> Result<(), String> {
        if !self.start_requested {
            return Ok(());
        }
        let actor = &mut self.actors[competitor_index];
        if let Some(pending) = actor.competitor_pending.as_ref() {
            let expected = u32::try_from(actor.competitor_takes.len())
                .map_err(|_| "competitor Take count overflowed")?;
            let observed = inventory_count(&actor.snapshot, PUMPKIN_ID);
            if observed != expected {
                if observed > expected
                    || u64::from(actor.snapshot.tick()).saturating_sub(pending.tick)
                        > MAX_TAKE_SETTLEMENT_TICKS
                {
                    return Err("competitor Take did not settle as exactly one pumpkin".into());
                }
                return Ok(());
            }
            actor.competitor_pending = None;
        }
        if actor.competitor_takes.len() >= 3 {
            return Ok(());
        }
        let pumpkin = actor.snapshot.ground_items().iter().find(|ground| {
            ground.def.id == PUMPKIN_ID
                && ground.tile.level == ANCHOR.plane
                && ground.tile.x == ANCHOR.x
                && ground.tile.z == ANCHOR.z
        });
        if pumpkin.is_none() {
            actor.competitor_sent_on_presence = false;
            return Ok(());
        }
        if actor.competitor_sent_on_presence {
            return Ok(());
        }
        let ground = pumpkin.unwrap();
        let label = ground
            .actions
            .iter()
            .flatten()
            .find(|label| label.eq_ignore_ascii_case("Take"))
            .cloned()
            .ok_or("competitor's visible static pumpkin lacks an observed Take action")?;
        let tile = tile_from_world(ground.tile);
        let frame = actor
            .latest
            .as_ref()
            .ok_or("competitor Take lacks its owning PLAYER_INFO observation")?;
        let observed_ground = GroundRow {
            item_id: ground.def.id,
            tile,
            quantity: u32::try_from(ground.count)
                .map_err(|_| "competitor visible pumpkin has an invalid quantity")?,
        };
        let scene_generation = client.gens.scene;
        let checkpoint = client.packet_checkpoint();
        let result = Interactions::new(&actor.snapshot, client)
            .interact(OpTarget::GroundItem(ground), ActionSpec::Label(label));
        let send_tick = sent_tick(result, "competitor real Take")?;
        let (tick, opcodes) =
            verified_send_packet(client, checkpoint, send_tick, "competitor real Take")?;
        if !one_obj3_packet(&opcodes) {
            return Err(format!(
                "competitor Take batch did not contain exactly one R289 OPOBJ3 packet: {opcodes:?}"
            ));
        }
        self.competitor_sequence = self.competitor_sequence.saturating_add(1);
        let request_key = format!(
            "competitor-take:{}:{}",
            actor.account, self.competitor_sequence
        );
        let take = PendingCompetitorTake {
            item_id: PUMPKIN_ID,
            tile,
            request_key,
            tick,
            observation_sequence: frame.observation_sequence,
            player_info_sequence: frame.player_info_sequence,
            scene_generation,
            observed_ground,
            packet_opcodes: opcodes,
        };
        actor.competitor_takes.push(take.clone());
        actor.competitor_pending = Some(take);
        actor.competitor_sent_on_presence = true;
        Ok(())
    }
}

fn record_runtime_observation(
    runtime: &mut Runtime,
    generation: Option<u64>,
    run_key: Option<WireRunKey>,
) -> Result<(), String> {
    let sample_ordinal = runtime
        .runtime_observations
        .last()
        .map_or(Some(1), |sample| sample.sample_ordinal.checked_add(1))
        .ok_or("runtime observation ordinal overflowed")?;
    let host_log_ordinal_fence = u64::try_from(
        HOST_LOGS
            .lock()
            .map_err(|_| "host log store poisoned while recording a completed PI frame")?
            .len(),
    )
    .map_err(|_| "completed PI host log ordinal overflowed")?;
    let actor = &runtime.actors[runtime.main_index];
    let latest = actor.latest.as_ref();
    let observation = RuntimeObservation {
        sample_ordinal,
        host_log_ordinal_fence,
        observation_sequence: latest
            .map(|frame| frame.observation_sequence)
            .unwrap_or(runtime.observation_sequence),
        account: actor.account.clone(),
        client_session_id: latest.map(RawFrame::session_id),
        client_session_generation: latest.map(|frame| frame.client_session_generation),
        player_info_sequence: latest.map(|frame| frame.player_info_sequence),
        tick: latest.map(|frame| frame.tick),
        start_requested: runtime.start_requested,
        generation,
        run_key,
    };
    if runtime.run_key.is_none() {
        runtime.run_key = run_key;
    }
    runtime.runtime_observations.push(observation);
    Ok(())
}

fn record_bank_callback(
    runtime: &mut Runtime,
    client: &Client,
    player_info_changed: bool,
) -> Result<(), String> {
    let sequence = runtime
        .bank_callbacks
        .last()
        .map_or(Some(1), |sample| sample.sequence.checked_add(1))
        .ok_or("bank callback sequence overflowed")?;
    let host_log_ordinal_fence = u64::try_from(
        HOST_LOGS
            .lock()
            .map_err(|_| "host log store poisoned while capturing bank packets")?
            .len(),
    )
    .map_err(|_| "bank callback host log ordinal overflowed")?;
    let actor = &runtime.actors[runtime.main_index];
    let mut inventory_packets = client
        .inventory_packet_states()
        .iter()
        .map(|(&component, state)| BankInventoryPacketObservation {
            component,
            full_generation: state.full_generation,
            full_observation: state.full_observation,
        })
        .collect::<Vec<_>>();
    inventory_packets.sort_by_key(|packet| packet.component);
    let component = actor.snapshot.bank_component_id();
    let full_observation = |packets: &[BankInventoryPacketObservation]| {
        packets
            .iter()
            .find(|packet| packet.component == component)
            .map(|packet| packet.full_observation)
    };
    let bank_changed = actor.snapshot.bank_loaded()
        && runtime.bank_callbacks.last().is_none_or(|previous| {
            previous.session_generation != client.gens.session
                || previous.bank_generation != actor.snapshot.bank_session_generation()
                || !previous.bank_loaded
                || full_observation(&previous.inventory_packets)
                    != full_observation(&inventory_packets)
        });
    let bank = bank_changed
        .then(|| bank_snapshot(&actor.snapshot))
        .transpose()?;
    runtime.bank_callbacks.push(BankCallbackObservation {
        sequence,
        host_log_ordinal_fence,
        tick: actor
            .latest
            .as_ref()
            .filter(|frame| frame.client_session_generation == client.gens.session)
            .map(|frame| frame.tick),
        player_info_changed,
        session_generation: client.gens.session,
        bank_generation: actor.snapshot.bank_session_generation(),
        bank_component: actor.snapshot.bank_component_id(),
        modal_stamp: client.main_modal_packet_state().opened_observation,
        bank_loaded: actor.snapshot.bank_loaded(),
        bank_snapshot: bank,
        inventory_packets,
    });
    Ok(())
}

fn capture_raw_frame(
    actor: &ActorState,
    client: &Client,
    observation_sequence: u64,
) -> Result<RawFrame, String> {
    let snapshot = &actor.snapshot;
    let actual = snapshot
        .local_player()
        .and_then(|player| player.player.actor.name.as_deref());
    let actor_ref = actual
        .map(|player_name| ActorRef {
            account_id: actor.account.clone(),
            player_name: player_name.to_owned(),
        })
        .ok_or_else(|| {
            format!(
                "{} PLAYER_INFO frame lacks a local player name",
                actor.account
            )
        })?;
    if !actor_ref
        .player_name
        .eq_ignore_ascii_case(&actor.expected_player_name)
    {
        return Err(format!(
            "observed local player identity {:?} differs from minted account {:?}",
            actual, actor.account
        ));
    }
    let position = snapshot
        .local_player()
        .map(|player| tile_from_world(player.player.actor.tile))
        .or_else(|| snapshot.tile().map(|(x, z, plane)| Tile { x, z, plane }))
        .ok_or_else(|| {
            format!(
                "{} PLAYER_INFO frame lacks an observed local tile",
                actor.account
            )
        })?;
    let inventory = item_snapshot(
        snapshot,
        false,
        snapshot.inventory_size() == i32::from(INVENTORY_CAPACITY) && client.gens.inv > 0,
    )?;
    let bank_open = snapshot.bank_component_id() >= 0;
    let bank_loaded = bank_open && snapshot.bank_loaded();
    let bank = if bank_open && bank_loaded {
        Some(bank_snapshot(snapshot)?)
    } else {
        None
    };
    let worn = worn_snapshot(snapshot, client.gens.inv > 0);
    let ground_state =
        if snapshot.scene_state() == 2 && snapshot.scene().available && client.gens.world > 0 {
            GroundPageState::Complete
        } else if snapshot.scene_state() == 2 {
            GroundPageState::Partial
        } else {
            GroundPageState::Unavailable
        };
    let mut ground_rows = Vec::new();
    for item in snapshot
        .ground_items()
        .iter()
        .filter(|item| item.def.id >= 0 && item.count > 0)
    {
        ground_rows.push(GroundRow {
            item_id: item.def.id,
            tile: tile_from_world(item.tile),
            quantity: u32::try_from(item.count)
                .map_err(|_| "ground row has a nonpositive or overflowing quantity")?,
        });
    }
    ground_rows.sort_by_key(|row| (row.tile.plane, row.tile.x, row.tile.z, row.item_id));
    let ground = GroundSnapshot {
        state: ground_state,
        rows: ground_rows,
    };
    let visible_players = snapshot.players().iter().filter_map(|player| {
        let name = player.actor.name.as_deref()?;
        if actor_ref.player_name.eq_ignore_ascii_case(name) { return None; }
        Some(json!({"account_id": Value::Null, "player_name": name, "position": tile_to_json(tile_from_world(player.actor.tile))}))
    }).collect();
    Ok(RawFrame {
        role: actor.role,
        account: actor.account.clone(),
        actor: actor_ref,
        tick: u64::from(snapshot.tick()),
        observation_sequence,
        player_info_sequence: client.gens.player_info,
        client_session_generation: client.gens.session,
        position,
        inventory,
        bank,
        bank_open,
        bank_loaded,
        worn,
        ground,
        object_turn: None,
        server_cycle: None,
        output_phase: None,
        visible_players,
        inventory_generation: client.gens.inv,
        ground_generation: client.gens.world,
        scene_generation: client.gens.scene,
    })
}

fn tile_from_world(tile: WorldTile) -> Tile {
    Tile {
        x: tile.x,
        z: tile.z,
        plane: tile.level,
    }
}
fn tile_to_json(tile: Tile) -> Value {
    json!({"x":tile.x,"z":tile.z,"plane":tile.plane})
}

fn item_snapshot(
    snapshot: &GameSnapshot,
    bank: bool,
    complete: bool,
) -> Result<ItemSnapshot, String> {
    let rows = if bank {
        snapshot.bank()
    } else {
        snapshot.inventory()
    };
    let mut quantities = BTreeMap::new();
    let mut occupied = 0u16;
    for row in rows
        .iter()
        .filter(|item| item.def.id >= 0 && item.count > 0 && item.slot >= 0)
    {
        let count =
            u32::try_from(row.count).map_err(|_| "negative item count in observed snapshot")?;
        *quantities.entry(row.def.id).or_default() += count;
        occupied = occupied.saturating_add(1);
    }
    Ok(ItemSnapshot {
        complete,
        quantities,
        occupied_slots: if bank { None } else { Some(occupied) },
        slot_capacity: if bank {
            None
        } else {
            u16::try_from(snapshot.inventory_size()).ok()
        },
    })
}

fn bank_snapshot(snapshot: &GameSnapshot) -> Result<ItemSnapshot, String> {
    if snapshot.bank_component_id() < 0
        || !snapshot.bank_loaded()
        || snapshot.bank_snapshot_generation().is_none()
    {
        return Err("bank snapshot is not current for the observed open bank session".into());
    }
    item_snapshot(snapshot, true, true)
}

fn worn_snapshot(snapshot: &GameSnapshot, complete: bool) -> ItemSnapshot {
    let mut quantities = BTreeMap::new();
    let mut occupied = 0u16;
    for row in snapshot
        .equipment()
        .iter()
        .filter(|item| item.def.id >= 0 && item.count > 0)
    {
        *quantities.entry(row.def.id).or_default() += u32::try_from(row.count).unwrap_or_default();
        occupied = occupied.saturating_add(1);
    }
    ItemSnapshot {
        complete,
        quantities,
        occupied_slots: Some(occupied),
        slot_capacity: Some(14),
    }
}

fn inventory_count(snapshot: &GameSnapshot, item_id: i32) -> u32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == item_id && item.count > 0)
        .map(|item| u32::try_from(item.count).unwrap_or_default())
        .sum()
}

fn snapshot_matches_seed(snapshot: &ItemSnapshot, seed: &[(i32, u32)]) -> bool {
    let expected = seed
        .iter()
        .copied()
        .filter(|(_, count)| *count > 0)
        .collect::<BTreeMap<_, _>>();
    snapshot.complete
        && snapshot.quantities == expected
        && snapshot.slot_capacity == Some(INVENTORY_CAPACITY)
}

fn item_alias(data: &api::game_data::SelectedGameData, id: i32) -> Result<String, String> {
    let item = data
        .item_by_id(id)
        .ok_or_else(|| format!("selected R289 content has no item row for {id}"))?;
    let alias = item
        .alias
        .as_deref()
        .filter(|alias| !alias.trim().is_empty())
        .ok_or_else(|| format!("selected R289 content has no exact setup alias for item {id}"))?;
    Ok(alias.to_owned())
}
fn full_inventory_seed(data: &api::game_data::SelectedGameData) -> Result<Vec<(i32, u32)>, String> {
    let mut ids = data
        .items()
        .iter()
        .filter(|item| {
            item.id > 0
                && item.id != PUMPKIN_ID
                && !item.stackable
                && !item.stack_variant
                && !item.is_certificate()
        })
        .filter(|item| {
            item.alias
                .as_deref()
                .is_some_and(|alias| !alias.trim().is_empty())
        })
        .map(|item| item.id)
        .take(usize::from(INVENTORY_CAPACITY))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    if ids.len() != usize::from(INVENTORY_CAPACITY) {
        return Err(format!(
            "selected R289 facts expose only {} eligible nonstackable full-pack seeds",
            ids.len()
        ));
    }
    Ok(ids.into_iter().map(|id| (id, 1)).collect())
}

fn withdraw_action(item: &ItemView) -> Option<(String, u32)> {
    let mut choices = Vec::new();
    for label in item.actions.iter().flatten() {
        let lower = label.to_ascii_lowercase();
        if !lower.starts_with("withdraw") {
            continue;
        }
        let amount = if lower.contains("all") {
            u32::try_from(item.count).ok()?
        } else {
            let digits = lower
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>();
            digits
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .unwrap_or(1)
        };
        choices.push((label.clone(), amount));
    }
    choices.sort_by_key(|(_, amount)| std::cmp::Reverse(*amount));
    choices.into_iter().next()
}

fn sent_tick(result: SendResult<'_>, name: &str) -> Result<u64, String> {
    match result {
        SendResult::Sent { tick, .. } => Ok(tick),
        SendResult::Refused { reason, .. } => {
            Err(format!("{name} was refused before send: {reason:?}"))
        }
    }
}

fn verify_sent_packet(
    client: &Client,
    checkpoint: Option<Box<api::interact::PacketCheckpoint>>,
    tick: u64,
    name: &str,
) -> Result<(), String> {
    let (_tick, opcodes) = verified_send_packet(client, checkpoint, tick, name)?;
    if opcodes.is_empty() {
        return Err(format!("{name} emitted no observed wire opcode"));
    }
    Ok(())
}

fn verified_send_packet(
    client: &Client,
    checkpoint: Option<Box<api::interact::PacketCheckpoint>>,
    tick: u64,
    name: &str,
) -> Result<(u64, Vec<u8>), String> {
    let checkpoint =
        checkpoint.ok_or_else(|| format!("{name} lacks a real encrypted packet checkpoint"))?;
    let mut opcodes = Vec::new();
    let decoded = client.trace_packets(*checkpoint, &mut |opcode| opcodes.push(opcode));
    if !decoded {
        return Err(format!(
            "{name} outbound packet stream could not be decoded completely"
        ));
    }
    if opcodes.is_empty() {
        return Err(format!("{name} returned Sent but wrote no actual packet"));
    }
    Ok((tick, opcodes))
}

fn card_settings(case: Case) -> serde_json::Map<String, Value> {
    let items = Value::Array(
        case.selected_ids()
            .into_iter()
            .map(|id| Value::String(id.to_string()))
            .collect(),
    );
    serde_json::Map::from_iter([
        ("items".to_owned(), items),
        ("radius".to_owned(), json!(RADIUS)),
        ("bank".to_owned(), json!("Nearest")),
        ("allow_wilderness".to_owned(), json!(false)),
        ("allow_danger_zones".to_owned(), json!(false)),
        ("allow_teleports".to_owned(), json!(false)),
    ])
}

fn profile_for(account: &str, password: &str, uid_offset: usize) -> Result<Profile, String> {
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock: {error}"))?
        .as_millis();
    let base = i32::try_from(base % i32::MAX as u128)
        .map_err(|_| "profile UID clock conversion failed")?;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid: base.saturating_add(i32::try_from(uid_offset).map_err(|_| "too many account UIDs")?),
        settings: ProfileSettings::default(),
    })
}

struct LiveInputs {
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    nav_pack: PathBuf,
    evidence_dir: PathBuf,
    game_port: u16,
    http_port: u16,
    management_port: u16,
    endpoint_id: u32,
    evidence_manifest: PathBuf,
    host_log_path: PathBuf,
    stderr_log_path: PathBuf,
    server_log_path: Option<PathBuf>,
    members: bool,
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let value = std::env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute: {}", path.display()));
    }
    Ok(path)
}

fn required_port(name: &str) -> Result<u16, String> {
    std::env::var(name)
        .map_err(|_| format!("{name} is required"))?
        .parse::<u16>()
        .map_err(|_| format!("{name} must be a valid TCP port"))
}

fn live_inputs() -> Result<LiveInputs, String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("Grinch live capture requires LIVE=1".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("Grinch live capture requires BOT_CPU=1".into());
    }
    if std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("gr25") {
        return Err("Grinch live capture requires BOT_LIVE_NAME_PREFIX=gr25".into());
    }
    let server_log_path = Some(required_path("GRINCH_SERVER_LOG")?);
    let engine_dir = required_path("GRINCH_ENGINE_DIR")?;
    let catalog_root = required_path("GRINCH_CATALOG_ROOT")?;
    let nav_pack = required_path("GRINCH_NAV_PACK")?;
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    let evidence_manifest = required_path("GRINCH_ENGINE_EVIDENCE")?;
    let host_log_path = required_path("GRINCH_HOST_LOG")?;
    let stderr_log_path = required_path("GRINCH_STDERR_LOG")?;
    let game_port = required_port("GRINCH_GAME_PORT")?;
    let http_port = required_port("GRINCH_HTTP_PORT")?;
    let management_port = required_port("GRINCH_MANAGEMENT_PORT")?;
    let endpoint_id = std::env::var("GRINCH_ENDPOINT_ID")
        .map_err(|_| "GRINCH_ENDPOINT_ID is required")?
        .parse::<u32>()
        .map_err(|_| "GRINCH_ENDPOINT_ID must be decimal")?;
    let members_value =
        std::env::var("GRINCH_MEMBERS").map_err(|_| "GRINCH_MEMBERS is required")?;
    if members_value != "1" {
        return Err("Grinch requires GRINCH_MEMBERS=1".into());
    }
    let members = true;
    let ports = [game_port, http_port, management_port];
    if ports.iter().copied().collect::<BTreeSet<_>>().len() != ports.len() {
        return Err("Grinch loopback service ports must be distinct".into());
    }
    if endpoint_id != u32::from(game_port) {
        return Err("scratch endpoint ID must match its runner-selected game port".into());
    }
    if ports
        .iter()
        .any(|port| [44594, 1080, 9898, 44694, 1180, 9988].contains(port))
    {
        return Err(
            "scratch capture attempted to use a protected Engine A or Engine Q port".into(),
        );
    }
    for (name, path, file) in [
        ("GRINCH_ENGINE_DIR", &engine_dir, false),
        ("GRINCH_CATALOG_ROOT", &catalog_root, false),
        ("GRINCH_NAV_PACK", &nav_pack, true),
        ("GRINCH_STDERR_LOG", &stderr_log_path, true),
    ] {
        let valid = if file { path.is_file() } else { path.is_dir() };
        if !valid {
            return Err(format!(
                "{name} has the wrong filesystem kind: {}",
                path.display()
            ));
        }
    }
    let case_dir = evidence_dir.clone();
    fs::create_dir_all(&case_dir)
        .map_err(|error| format!("create evidence directory {}: {error}", case_dir.display()))?;
    if let Some(path) = &server_log_path {
        if !path.is_file() {
            return Err(format!(
                "GRINCH_SERVER_LOG is not an existing runner-owned log file: {}",
                path.display()
            ));
        }
    }
    Ok(LiveInputs {
        engine_dir,
        catalog_root,
        nav_pack,
        evidence_dir: case_dir,
        game_port,
        http_port,
        management_port,
        endpoint_id,
        evidence_manifest,
        host_log_path,
        stderr_log_path,
        server_log_path,
        members,
    })
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        let base = std::env::temp_dir();
        for attempt in 0..32u32 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock: {error}"))?
                .as_nanos();
            let path = base.join(format!(
                "grinch-s4-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("create isolated profile root: {error}")),
            }
        }
        Err("could not allocate an isolated profile root".into())
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn selected_profile(
    inputs: &LiveInputs,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(inputs.game_port),
        http_port: Some(inputs.http_port),
        nav_pack: Some(inputs.nav_pack.clone()),
        engine_dir: Some(inputs.engine_dir.clone()),
        catalog_root: Some(inputs.catalog_root.clone()),
        vault_path: Some(temp.join("vault")),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("Grinch capture requires a loopback-only local R289 profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected R289 template has no navigation world".into());
    }
    Ok((profile, template))
}

fn http_body(port: u16, path: &str) -> Result<String, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| format!("connect loopback management endpoint {port}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("set management read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("set management write timeout: {error}"))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| format!("write management GET: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read management GET: {error}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|error| format!("management endpoint returned invalid UTF-8: {error}"))?;
    let (header, body) = text
        .split_once("\r\n\r\n")
        .ok_or("management endpoint omitted HTTP header terminator")?;
    if !header
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "))
    {
        return Err(format!(
            "management GET failed: {}",
            header.lines().next().unwrap_or("missing status")
        ));
    }
    if header.lines().any(|line| {
        line.to_ascii_lowercase().starts_with("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked")
    }) {
        return decode_chunked(body);
    }
    Ok(body.to_owned())
}

fn decode_chunked(mut body: &str) -> Result<String, String> {
    let mut decoded = Vec::new();
    loop {
        let (line, rest) = body
            .split_once("\r\n")
            .ok_or("truncated chunked management response")?;
        let length = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
            .map_err(|_| "invalid HTTP chunk length")?;
        body = rest;
        if length == 0 {
            break;
        }
        if body.len() < length + 2 {
            return Err("truncated HTTP chunk".into());
        }
        decoded.extend_from_slice(&body.as_bytes()[..length]);
        if &body.as_bytes()[length..length + 2] != b"\r\n" {
            return Err("invalid HTTP chunk terminator".into());
        }
        body = &body[length + 2..];
    }
    String::from_utf8(decoded)
        .map_err(|error| format!("chunked response is invalid UTF-8: {error}"))
}

fn verify_selected_endpoint(
    inputs: &LiveInputs,
    profile: &host_play::ServerProfile,
) -> Result<Value, String> {
    let text = http_body(inputs.management_port, "/setup/config")?;
    let mut value: Value =
        serde_json::from_str(&text).map_err(|error| format!("parse /setup/config: {error}"))?;
    if let Some(config) = value.get("config") {
        value = config.clone();
    }
    let node = value.get("node").ok_or("/setup/config lacks node object")?;
    let web = value.get("web").ok_or("/setup/config lacks web object")?;
    let checks = [
        (
            node["id"].as_u64() == Some(u64::from(inputs.endpoint_id)),
            "node.id",
        ),
        (
            node["port"].as_u64() == Some(u64::from(inputs.game_port)),
            "node.port",
        ),
        (
            web["port"].as_u64() == Some(u64::from(inputs.http_port)),
            "web.port",
        ),
        (
            web["managementPort"].as_u64() == Some(u64::from(inputs.management_port)),
            "web.managementPort",
        ),
        (
            node["members"].as_bool() == Some(inputs.members),
            "node.members",
        ),
    ];
    for (valid, field) in checks {
        if !valid {
            return Err(format!("selected /setup/config {field} does not match the caller-selected loopback endpoint"));
        }
    }
    if profile.client().game_port() != inputs.game_port {
        return Err("bound local profile game port differs from the selected endpoint".into());
    }
    Ok(value)
}

#[derive(Debug, Clone)]
struct BankCandidate {
    name: String,
    tile: Tile,
    ticks: f64,
    walking_steps: u16,
    ordinary: bool,
    interaction_tiles: Vec<Tile>,
}

fn bank_candidates(
    world: &NavWorld,
    banks: &[NamedBank],
    anchor: Tile,
) -> Result<Vec<BankCandidate>, String> {
    let state = WorldState::default();
    let mut candidates = Vec::new();
    for bank in banks.iter().filter(|bank| bank.routable) {
        let definition = match bank.definition {
            Some(definition) => definition,
            None => continue,
        };
        let ordinary = ordinary_bank(definition);
        if !ordinary {
            continue;
        }
        let route = match find_with(
            &world.collision,
            &world.graph,
            WorldTile {
                x: anchor.x,
                z: anchor.z,
                level: anchor.plane,
            },
            bank.tile,
            FindOptions {
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                ..FindOptions::default()
            },
            &state,
        ) {
            Ok(route) => route,
            Err(_) => continue,
        };
        if route.legs.iter().any(
            |leg| matches!(leg, Leg::Transport { edge } if edge.kind == TransportKind::Teleport),
        ) {
            continue;
        }
        let steps = route
            .legs
            .iter()
            .map(|leg| match leg {
                Leg::Walk { tiles } => tiles.len(),
                Leg::Transport { .. } => 0,
            })
            .sum::<usize>();
        candidates.push(BankCandidate {
            name: bank.name.to_owned(),
            tile: tile_from_world(bank.tile),
            ticks: route.ticks,
            walking_steps: u16::try_from(steps).unwrap_or(u16::MAX),
            ordinary,
            interaction_tiles: world
                .banks()
                .iter()
                .filter(|stand| {
                    nav::bank_fetch::bank_access_tiles(&world.collision, stand)
                        .any(|tile| tile == bank.tile)
                })
                .map(|stand| tile_from_world(stand.tile))
                .collect(),
        });
    }
    candidates.sort_by(|left, right| {
        left.ticks
            .total_cmp(&right.ticks)
            .then_with(|| left.name.cmp(&right.name))
    });
    if candidates.is_empty() {
        return Err(
            "selected navigation pack proves no ordinary routable bank from the Grinch anchor"
                .into(),
        );
    }
    Ok(candidates)
}

fn ordinary_bank(definition: &BankDefinition) -> bool {
    definition.skill.is_none()
        && definition.quest.is_none()
        && definition.setting.is_none()
        && definition.object.is_none()
        && definition.open_first.is_none()
        && definition.npc.is_none()
}

fn bank_open_matches_candidate(candidate: &BankCandidate, tile: Option<Tile>) -> bool {
    tile.is_some_and(|tile| candidate.interaction_tiles.contains(&tile))
}

#[derive(Clone)]
struct PreloginAccountStore {
    profile: String,
    database_usernames: BTreeSet<[u8; 32]>,
    profile_saves: BTreeSet<[u8; 32]>,
}

fn account_name_fingerprint(name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for byte in name.bytes() {
        hasher.update([byte.to_ascii_lowercase()]);
    }
    let digest = hasher.finalize();
    let mut fingerprint = [0; 32];
    fingerprint.copy_from_slice(&digest);
    fingerprint
}

fn parse_sha256_fingerprints(evidence: &Value, field: &str) -> Result<BTreeSet<[u8; 32]>, String> {
    let rows = evidence
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("pre-login account-store evidence lacks {field}"))?;
    let mut fingerprints = BTreeSet::new();
    for row in rows {
        let encoded = row
            .as_str()
            .ok_or_else(|| format!("pre-login account-store {field} contains a non-string"))?;
        let bytes = encoded.as_bytes();
        if bytes.len() != 64 {
            return Err(format!(
                "pre-login account-store {field} contains a malformed SHA-256"
            ));
        }
        let mut fingerprint = [0; 32];
        for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            let high = match pair[0] {
                b'0'..=b'9' => pair[0] - b'0',
                b'a'..=b'f' => pair[0] - b'a' + 10,
                _ => {
                    return Err(format!(
                        "pre-login account-store {field} contains a malformed SHA-256"
                    ))
                }
            };
            let low = match pair[1] {
                b'0'..=b'9' => pair[1] - b'0',
                b'a'..=b'f' => pair[1] - b'a' + 10,
                _ => {
                    return Err(format!(
                        "pre-login account-store {field} contains a malformed SHA-256"
                    ))
                }
            };
            fingerprint[index] = (high << 4) | low;
        }
        if !fingerprints.insert(fingerprint) {
            return Err(format!(
                "pre-login account-store {field} contains a duplicate SHA-256"
            ));
        }
    }
    Ok(fingerprints)
}

fn parse_prelogin_account_store(manifest: &Value) -> Result<PreloginAccountStore, String> {
    let evidence = manifest
        .get("scratch")
        .and_then(|scratch| scratch.get("account_store_prelogin"))
        .ok_or("scratch manifest lacks pre-login account-store evidence")?;
    if evidence["captured_before_server_start"].as_bool() != Some(true) {
        return Err("scratch account-store evidence was not captured before server start".into());
    }
    if evidence["database_path"].as_str() != Some("db.sqlite")
        || evidence["account_table"].as_str() != Some("account")
    {
        return Err("scratch account-store evidence does not identify db.sqlite.account".into());
    }
    if evidence["username_hash_scheme"].as_str() != Some("sha256-utf8-ascii-lowercase")
        || evidence["save_suffix"].as_str() != Some(".sav")
    {
        return Err(
            "scratch account-store evidence uses an unsupported username fingerprint".into(),
        );
    }
    let profile = evidence["profile"]
        .as_str()
        .ok_or("scratch account-store evidence lacks its player-save profile")?;
    if profile.is_empty()
        || profile == "."
        || profile == ".."
        || profile.contains('/')
        || profile.contains('\\')
    {
        return Err("scratch account-store evidence has an invalid player-save profile".into());
    }
    let save_directory = format!("data/players/{profile}");
    if evidence["save_directory"].as_str() != Some(save_directory.as_str()) {
        return Err(
            "scratch account-store evidence has an inconsistent player-save directory".into(),
        );
    }
    Ok(PreloginAccountStore {
        profile: profile.to_owned(),
        database_usernames: parse_sha256_fingerprints(evidence, "database_username_sha256")?,
        profile_saves: parse_sha256_fingerprints(evidence, "profile_save_name_sha256")?,
    })
}

fn validate_fresh_account_names(
    store: &PreloginAccountStore,
    names: &[String],
) -> Result<BTreeSet<String>, String> {
    let mut fresh = BTreeSet::new();
    for name in names {
        let fingerprint = account_name_fingerprint(name);
        if store.database_usernames.contains(&fingerprint) {
            return Err(format!(
                "pre-login db.sqlite account table already contains minted account {name}"
            ));
        }
        if store.profile_saves.contains(&fingerprint) {
            return Err(format!(
                "pre-login player save already exists for minted account {name} at data/players/{}/{name}.sav",
                store.profile
            ));
        }
        if !fresh.insert(name.clone()) {
            return Err("live account minting returned duplicate names".into());
        }
    }
    Ok(fresh)
}

#[derive(Clone)]
struct SessionData {
    runtime: RuntimeReport,
    logs: ObservedLogs,
    host_rows: Vec<ObservedLogRecord>,
    engine: EngineEvidence,
    fresh_accounts: BTreeSet<String>,
    account_store_prelogin: Value,
    setup_config: Value,
    bank_candidates: Vec<BankCandidate>,
    evidence_manifest_path: PathBuf,
    host_log_path: PathBuf,
    stderr_log_path: PathBuf,
    stderr_capture_prefix_bytes: u64,
    server_log_path: Option<PathBuf>,
    runtime_error: Option<String>,
}

fn capture_live_inner(case: Case, inputs: &LiveInputs) -> Result<SessionData, String> {
    if !HOST_LOGS
        .lock()
        .map_err(|_| "host log store poisoned")?
        .is_empty()
    {
        return Err(
            "host log sink is not empty; run one exact Grinch case per test process".into(),
        );
    }
    api::hostlog::set_debug(true);
    if !api::hostlog::install_sink(&HOST_LOG_SINK) {
        return Err("another host log sink is already installed; exact Grinch case requires exclusive sink ownership".into());
    }
    let raw_engine = fs::read(&inputs.evidence_manifest).map_err(|error| {
        format!(
            "read caller-owned engine evidence {}: {error}",
            inputs.evidence_manifest.display()
        )
    })?;
    let engine_manifest: Value = serde_json::from_slice(&raw_engine).map_err(|error| {
        format!("parse raw engine evidence for account-store snapshot: {error}")
    })?;
    let account_store = parse_prelogin_account_store(&engine_manifest)?;
    let account_store_prelogin = engine_manifest["scratch"]["account_store_prelogin"].clone();
    let engine: EngineEvidence = serde_json::from_slice(&raw_engine)
        .map_err(|error| format!("parse typed engine evidence: {error}"))?;
    if case.is_b() != engine.scratch.content_patch.is_some() {
        return Err("scratch content patch presence does not match the requested cell".into());
    }
    let temp = TempRoot::new()?;
    let _home = script::IsolatedEnv::enter(&format!("grinch-s4-{}", case.slug()));
    let (profile, template) = selected_profile(inputs, temp.path())?;
    let setup_config = verify_selected_endpoint(inputs, &profile)?;
    if let Some(configured_profile) = setup_config["node"]["profile"].as_str() {
        if configured_profile != account_store.profile {
            return Err(
                "pre-login account-store profile differs from the selected /setup/config profile"
                    .into(),
            );
        }
    }
    let world = template
        .world()
        .ok_or("selected template has no navigation world")?;
    for tile in [ANCHOR, NEAR, OUTSIDE] {
        if !world.collision.standable(WorldTile {
            x: tile.x,
            z: tile.z,
            level: tile.plane,
        }) {
            return Err(format!(
                "Grinch fixture tile is not standable in the selected nav pack: {tile:?}"
            ));
        }
    }
    let game_data = profile
        .game_data()
        .ok_or("selected R289 game data unavailable")?;
    let active_route = if case.is_c() && case != Case::CInventoryBlocked {
        if world.named_bank_facts().is_none() {
            world.bind_named_bank_facts(&game_data)?;
        }
        let banks = world
            .named_bank_facts()
            .ok_or("selected navigation world lacks bound named-bank facts")?;
        bank_candidates(&world, banks.banks(), ANCHOR)?
    } else {
        Vec::new()
    };
    let helper_needed = case.uses_helper();
    let competitor_needed = case.uses_competitor();
    let actor_count = 1 + usize::from(helper_needed) + usize::from(competitor_needed);
    let names = host_play::mint_live_names(actor_count);
    if names.len() != actor_count || names.iter().collect::<BTreeSet<_>>().len() != actor_count {
        return Err("live account minting did not return distinct fresh names".into());
    }
    let fresh_accounts = validate_fresh_account_names(&account_store, &names)?;
    let credentials = host_play::mint_live_entries(&names);
    if credentials.len() != actor_count {
        return Err("live account credential minting was incomplete".into());
    }
    let main_name_index = if competitor_needed { 1 } else { 0 };
    let competitor_index = competitor_needed.then_some(0);
    let helper_name_index = helper_needed.then_some(if competitor_needed { 2 } else { 1 });
    let mut actors = Vec::new();
    if competitor_needed {
        actors.push(ActorState::new(
            ActorRole::Competitor,
            names[0].clone(),
            Vec::new(),
            false,
            Vec::new(),
        ));
    }
    let receiver_seed = if case == Case::CInventoryBlocked {
        full_inventory_seed(&game_data)?
    } else {
        case.receiver_seed()
    };
    let main_index = actors.len();
    actors.push(ActorState::new(
        ActorRole::Main,
        names[main_name_index].clone(),
        receiver_seed,
        case.is_c(),
        case.helper_drops(),
    ));
    let helper_index = if let Some(name_index) = helper_name_index {
        let index = actors.len();
        actors.push(ActorState::new(
            ActorRole::Helper,
            names[name_index].clone(),
            case.helper_seed(),
            false,
            case.helper_drops(),
        ));
        Some(index)
    } else {
        None
    };
    let runtime = Arc::new(Mutex::new(Runtime {
        case,
        actors,
        main_index,
        helper_index,
        competitor_index: competitor_index.map(|_| 0),
        observation_sequence: 0,
        run_key: None,
        terminal_stop: None,
        start_requested: false,
        start_observation_sequence: None,
        pre_start_frame: None,
        runtime_observations: Vec::new(),
        bank_callbacks: Vec::new(),
        error: None,
        drop_sequence: 0,
        competitor_sequence: 0,
    }));
    let callback_runtime = Arc::clone(&runtime);
    let selected = Arc::clone(&game_data);
    let mut server_offset = if let Some(path) = &inputs.server_log_path {
        fs::metadata(path)
            .map_err(|error| format!("stat runner-owned scratch log {}: {error}", path.display()))?
            .len()
    } else {
        0
    };
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if let Ok(mut runtime) = callback_runtime.lock() {
                runtime.on_frame(client, username, frame.hold, &selected);
            }
        },
    )?;
    let start_handle = play.script_start_handle();
    let observed_runtime = Arc::clone(&runtime);
    let observed_handle = start_handle.clone();
    play.set_post_native_player_info_observer(move |client, username| {
        if let Ok(mut runtime) = observed_runtime.lock() {
            runtime.on_post_native_player_info(client, username, &observed_handle);
        }
    });
    let mut spawned = Vec::with_capacity(actor_count);
    let mut next_spawn_index = 0;
    let mut logs = ObservedLogs::default();
    let mut host_cursor = 0usize;
    let mut pending_server = String::new();
    let started = Instant::now();
    let mut runtime_error = None;
    let mut completion_tick = None;
    loop {
        match host_logs_since(&mut host_cursor) {
            Ok(rows) => {
                for row in rows {
                    logs.ingest_record(row);
                }
            }
            Err(error) => {
                runtime_error = Some(error);
                break;
            }
        }
        let Some(path) = inputs.server_log_path.as_deref() else {
            runtime_error = Some("scratch capture lacks its runner-owned server log".into());
            break;
        };
        match read_appended_text(path, &mut server_offset) {
            Ok(text) => logs.ingest_server_chunk(&text, &mut pending_server, false),
            Err(error) => {
                runtime_error = Some(error);
                break;
            }
        }
        if next_spawn_index < actor_count {
            let may_spawn = if competitor_needed && next_spawn_index > 0 {
                match runtime.lock() {
                    Ok(state) => state.actor_ready(ActorRole::Competitor),
                    Err(_) => {
                        runtime_error =
                            Some("live runtime state poisoned before actor launch".into());
                        break;
                    }
                }
            } else {
                true
            };
            if may_spawn {
                let result = (|| -> Result<(), String> {
                    let account = &names[next_spawn_index];
                    let password = credentials
                        .iter()
                        .find(|(username, _)| username == account)
                        .map(|(_, password)| password.as_str())
                        .ok_or("actor password is missing")?;
                    play.try_spawn_slot(
                        profile_for(account, password, next_spawn_index + 1)?,
                        None,
                        None,
                        None,
                    )
                    .map_err(|error| {
                        format!("spawn freshly minted Play actor {account}: {error}")
                    })?;
                    spawned.push(account.clone());
                    next_spawn_index += 1;
                    Ok(())
                })();
                if let Err(error) = result {
                    runtime_error = Some(error);
                    break;
                }
            }
        }
        {
            let mut state = match runtime.lock() {
                Ok(state) => state,
                Err(_) => {
                    runtime_error = Some("live runtime state poisoned".into());
                    break;
                }
            };
            state.start_if_ready(&start_handle);
            if let Some(error) = state.error.clone() {
                runtime_error = Some(error);
                break;
            }
            if let Err(error) = reconcile_raw_frames(&mut state, &logs, case) {
                runtime_error = Some(error);
                break;
            }
            if !logs.malformed.is_empty() {
                runtime_error = Some(format!(
                    "observed host/server ledger is malformed: {}",
                    logs.malformed[0]
                ));
                break;
            }
            let reached = capture_condition(case, &state, &logs);
            let elapsed_ticks = state.actors[state.main_index]
                .latest
                .as_ref()
                .and_then(|frame| state.run_key.map(|_| frame.tick))
                .unwrap_or_default();
            if let Some((start_tick, max_tick)) = reached.1 {
                if elapsed_ticks.saturating_sub(start_tick) > max_tick {
                    runtime_error = Some(format!(
                        "observed game-tick budget exceeded for {}",
                        case.slug()
                    ));
                    break;
                }
            }
            if reached.0 {
                let tick = state.actors[state.main_index]
                    .latest
                    .as_ref()
                    .map(|frame| frame.tick)
                    .unwrap_or_default();
                let fence = *completion_tick.get_or_insert(tick);
                let after_tail = tick >= fence.saturating_add(MAX_TAKE_SETTLEMENT_TICKS);
                let competitor_after_tail = if case.uses_competitor() {
                    state
                        .competitor_index
                        .and_then(|index| state.actors[index].latest.as_ref())
                        .is_some_and(|frame| {
                            frame.tick >= fence.saturating_add(MAX_TAKE_SETTLEMENT_TICKS)
                        })
                } else {
                    true
                };
                if after_tail && competitor_after_tail {
                    break;
                }
            }
        }
        if let Some(error) = play.script_last_error(&names[main_name_index]) {
            let terminal = if case == Case::CInventoryBlocked {
                play.script_native_status(&names[main_name_index])
            } else {
                None
            };
            match terminal {
                Some(status) => {
                    let observed = runtime
                        .lock()
                        .map_err(|_| "terminal runtime state poisoned".to_owned())
                        .and_then(|mut state| state.observe_terminal_stop(&status, &error, &logs));
                    if let Err(error) = observed {
                        runtime_error = Some(error);
                        break;
                    }
                }
                None => {
                    runtime_error = Some(format!("Grinch native Start failed: {error}"));
                    break;
                }
            }
        }
        if started.elapsed() >= case.wall_guard() {
            runtime_error = Some(format!(
                "wall-clock safety guard expired without meeting observed game evidence for {}",
                case.slug()
            ));
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    for account in &spawned {
        play.script_stop(account);
        play.stop_slot(account);
    }
    match host_logs_since(&mut host_cursor) {
        Ok(rows) => {
            for row in rows {
                logs.ingest_record(row);
            }
        }
        Err(error) if runtime_error.is_none() => runtime_error = Some(error),
        Err(_) => {}
    }
    if let Some(path) = inputs.server_log_path.as_deref() {
        let result = if runtime_error.is_none() {
            runtime
                .lock()
                .map_err(|_| {
                    "live runtime state poisoned during server log finalization".to_owned()
                })
                .and_then(|state| {
                    let frames = state
                        .actors
                        .iter()
                        .flat_map(|actor| actor.raw_frames.iter());
                    finish_required_server_log(
                        path,
                        frames,
                        &mut logs,
                        &mut server_offset,
                        &mut pending_server,
                    )
                })
        } else {
            read_appended_text(path, &mut server_offset)
                .map(|text| logs.ingest_server_chunk(&text, &mut pending_server, true))
        };
        if let Err(error) = result {
            if runtime_error.is_none() {
                runtime_error = Some(error);
            }
        }
    }
    let host_rows = match all_host_logs() {
        Ok(rows) => rows,
        Err(error) => {
            if runtime_error.is_none() {
                runtime_error = Some(error);
            }
            Vec::new()
        }
    };
    if let Err(error) = write_host_log(&inputs.host_log_path, &host_rows) {
        if runtime_error.is_none() {
            runtime_error = Some(error);
        }
    }
    if let Ok(mut state) = runtime.lock() {
        if let Err(error) = reconcile_raw_frames(&mut state, &logs, case) {
            if runtime_error.is_none() {
                runtime_error = Some(error);
            }
        }
    } else if runtime_error.is_none() {
        runtime_error = Some("live runtime state poisoned during finalization".into());
    }
    let report = runtime
        .lock()
        .map_err(|_| "live runtime state poisoned")?
        .report();
    let mut stderr_capture_prefix_bytes = 0;
    match fs::read_to_string(&inputs.stderr_log_path) {
        Ok(text) => {
            stderr_capture_prefix_bytes = u64::try_from(text.len())
                .map_err(|_| "captured stderr prefix length exceeds u64")?;
            if let Err(error) = logs.ingest_captured_stderr(&text, &report) {
                if runtime_error.is_none() {
                    runtime_error = Some(error);
                }
            }
        }
        Err(error) if runtime_error.is_none() => {
            runtime_error = Some(format!(
                "read captured stderr {}: {error}",
                inputs.stderr_log_path.display()
            ));
        }
        Err(_) => {}
    }
    if runtime_error.is_none() {
        runtime_error = report.setup_error.clone();
    }
    Ok(SessionData {
        runtime: report,
        logs,
        host_rows,
        engine,
        fresh_accounts,
        account_store_prelogin,
        setup_config,
        bank_candidates: active_route,
        evidence_manifest_path: inputs.evidence_manifest.clone(),
        host_log_path: inputs.host_log_path.clone(),
        stderr_log_path: inputs.stderr_log_path.clone(),
        stderr_capture_prefix_bytes,
        server_log_path: inputs.server_log_path.clone(),
        runtime_error,
    })
}

fn read_appended_text(path: &Path, offset: &mut u64) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open runner-owned scratch log {}: {error}", path.display()))?;
    let length = file
        .metadata()
        .map_err(|error| format!("stat runner-owned scratch log {}: {error}", path.display()))?
        .len();
    if length < *offset {
        return Err("runner-owned scratch log was truncated during capture".into());
    }
    file.seek(SeekFrom::Start(*offset))
        .map_err(|error| format!("seek scratch log: {error}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("read scratch log: {error}"))?;
    *offset = offset
        .checked_add(bytes.len() as u64)
        .ok_or("scratch log offset overflowed")?;
    String::from_utf8(bytes).map_err(|error| format!("scratch log contains invalid UTF-8: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct ServerLogAppendWatch {
    _file: File,
    notifications: OwnedFd,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl ServerLogAppendWatch {
    fn new(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|error| format!("open scratch append watch: {error}"))?;
        #[cfg(target_os = "macos")]
        {
            // SAFETY: kqueue has no pointer arguments and returns a new owned fd.
            let raw = unsafe { libc::kqueue() };
            if raw < 0 {
                return Err(format!(
                    "create scratch append watch: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: the successful kqueue call transferred this fd to us.
            let notifications = unsafe { OwnedFd::from_raw_fd(raw) };
            let event = libc::kevent {
                ident: file.as_raw_fd() as usize,
                filter: libc::EVFILT_VNODE,
                flags: libc::EV_ADD | libc::EV_CLEAR,
                fflags: libc::NOTE_WRITE
                    | libc::NOTE_EXTEND
                    | libc::NOTE_DELETE
                    | libc::NOTE_RENAME,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            // SAFETY: event is initialized and valid for one input; there are no output events.
            if unsafe {
                libc::kevent(
                    notifications.as_raw_fd(),
                    &event,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            } < 0
            {
                return Err(format!(
                    "register scratch append watch: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self {
                _file: file,
                notifications,
            })
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStrExt;

            // SAFETY: inotify_init1 has no pointer arguments and returns a new owned fd.
            let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
            if raw < 0 {
                return Err(format!(
                    "create scratch append watch: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: the successful inotify_init1 call transferred this fd to us.
            let notifications = unsafe { OwnedFd::from_raw_fd(raw) };
            let path = std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| "scratch log path contains an interior NUL")?;
            // SAFETY: path is NUL-terminated and notifications is a live inotify fd.
            if unsafe {
                libc::inotify_add_watch(
                    notifications.as_raw_fd(),
                    path.as_ptr(),
                    libc::IN_MODIFY | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF,
                )
            } < 0
            {
                return Err(format!(
                    "register scratch append watch: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self {
                _file: file,
                notifications,
            })
        }
    }

    fn wait(&self) -> Result<(), String> {
        loop {
            #[cfg(target_os = "macos")]
            let count = {
                // SAFETY: zero is valid for all kevent fields; the kernel initializes the output.
                let mut event: libc::kevent = unsafe { std::mem::zeroed() };
                // SAFETY: event holds space for one output; a null timeout blocks for a real event.
                unsafe {
                    libc::kevent(
                        self.notifications.as_raw_fd(),
                        std::ptr::null(),
                        0,
                        &mut event,
                        1,
                        std::ptr::null(),
                    )
                }
            };
            #[cfg(target_os = "linux")]
            let count = {
                let mut events = [0u8; 4096];
                // SAFETY: events is writable for its full length; this blocking fd owns its queue.
                unsafe {
                    libc::read(
                        self.notifications.as_raw_fd(),
                        events.as_mut_ptr().cast(),
                        events.len(),
                    )
                }
            };
            if count > 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if count < 0 && error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("wait for scratch log append: {error}"));
        }
    }
}

fn required_server_cycle_window<'a>(
    frames: impl Iterator<Item = &'a RawFrame>,
    logs: &ObservedLogs,
) -> Result<Option<(u64, u64)>, String> {
    let mut outputs = HashMap::with_capacity(logs.client_outputs.len());
    for output in &logs.client_outputs {
        outputs
            .entry((&output.actor, output.player_info_sequence))
            .and_modify(|cycle| *cycle = None)
            .or_insert(Some(output.cycle));
    }
    let mut window: Option<(u64, u64)> = None;
    let mut missing_frame = None;
    let mut debug_ordinals: Option<HashMap<u64, u64>> = None;
    for frame in frames {
        let cycle = match outputs.get(&(&frame.actor, frame.player_info_sequence)) {
            Some(Some(cycle)) => *cycle,
            Some(None) => {
                return Err("captured PLAYER_INFO maps to duplicate server output rows".into())
            }
            None => {
                let debug_rows = debug_ordinals.get_or_insert_with(|| {
                    logs.debug_ticks
                        .iter()
                        .map(|row| (row.cycle, row.server_line_ordinal))
                        .collect()
                });
                let advanced_cycle = logs.cycles.iter().find_map(|event| {
                    let cycle = event.payload["cycle"].as_u64()?;
                    let debug_ordinal = *debug_rows.get(&cycle)?;
                    if event.server_line_ordinal == 0 || debug_ordinal <= event.server_line_ordinal
                    {
                        return None;
                    }
                    let has_client_output = event.payload["phase_order"]
                        .as_array()?
                        .iter()
                        .any(|phase| phase.as_str() == Some("client_output"));
                    if !has_client_output {
                        return None;
                    }
                    let actor_sequence = event.payload["active_actors"]
                        .as_array()?
                        .iter()
                        .find(|active| {
                            active["active"].as_bool() == Some(true)
                                && active["actor"]["account_id"].as_str()
                                    == Some(frame.actor.account_id.as_str())
                                && active["actor"]["player_name"].as_str()
                                    == Some(frame.actor.player_name.as_str())
                        })
                        .and_then(|active| active["player_info_sequence"].as_u64())?;
                    (actor_sequence >= frame.player_info_sequence)
                        .then_some((cycle, actor_sequence))
                });
                if let Some((cycle, sequence)) = advanced_cycle {
                    return Err(format!(
                        "captured PLAYER_INFO for {} (account {}) sequence {} lacks its GRINCH_CLIENT_OUTPUT row after completed server cycle {} recorded sequence {}",
                        frame.actor.player_name,
                        frame.actor.account_id,
                        frame.player_info_sequence,
                        cycle,
                        sequence
                    ));
                }
                missing_frame.get_or_insert(frame);
                continue;
            }
        };
        window = Some(window.map_or((cycle, cycle), |(first, last)| {
            (first.min(cycle), last.max(cycle))
        }));
    }
    if let Some(frame) = missing_frame {
        if logs
            .server_lines
            .iter()
            .any(|line| line.contains("Server shutdown complete"))
        {
            return Err(format!(
                "scratch server shut down before required GRINCH_CLIENT_OUTPUT row for captured PLAYER_INFO sequence {} (account {}, player {})",
                frame.player_info_sequence, frame.actor.account_id, frame.actor.player_name
            ));
        }
        return Ok(None);
    }
    Ok(window)
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn required_debug_profile_window<'a>(
    frames: impl Iterator<Item = &'a RawFrame>,
    logs: &ObservedLogs,
) -> Result<Option<(u64, u64)>, String> {
    let window = required_server_cycle_window(frames, logs)?;
    if let Some((_, last)) = window {
        if logs.debug_ticks.iter().any(|row| row.cycle == last) {
            return Ok(window);
        }
    }
    if logs
        .server_lines
        .iter()
        .any(|line| line.contains("Server shutdown complete"))
    {
        return Err(
            "scratch server shut down before its last required debugProfile tick row".into(),
        );
    }
    Ok(None)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn finish_required_server_log<'a>(
    path: &Path,
    frames: impl Iterator<Item = &'a RawFrame> + Clone,
    logs: &mut ObservedLogs,
    offset: &mut u64,
    pending: &mut String,
) -> Result<(), String> {
    // Register before reading so an append between the read and wait cannot be lost.
    // All client workers have already been synchronously joined by stop_slot.
    let watch = ServerLogAppendWatch::new(path)?;
    loop {
        let text = read_appended_text(path, offset)?;
        logs.ingest_server_chunk(&text, pending, false);
        if required_debug_profile_window(frames.clone(), logs)?.is_some() {
            if !pending.is_empty() {
                // A later incomplete row is raw trailing evidence, not cycle authority.
                logs.server_lines.push(std::mem::take(pending));
            }
            return Ok(());
        }
        watch.wait()?;
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn finish_required_server_log<'a>(
    _path: &Path,
    _frames: impl Iterator<Item = &'a RawFrame> + Clone,
    _logs: &mut ObservedLogs,
    _offset: &mut u64,
    _pending: &mut String,
) -> Result<(), String> {
    Err("scratch log append notifications require macOS or Linux".into())
}

fn server_cycles(logs: &ObservedLogs) -> Result<Vec<ServerCycle>, String> {
    let mut cycles = Vec::with_capacity(logs.cycles.len());
    for event in &logs.cycles {
        let value = &event.payload;
        let cycle = value["cycle"]
            .as_u64()
            .ok_or("GRINCH_CYCLE lacks an unsigned cycle")?;
        let object_turn = value["object_turn"]
            .as_u64()
            .ok_or("GRINCH_CYCLE lacks an unsigned object turn")?;
        let phase_order = value["phase_order"]
            .as_array()
            .ok_or("GRINCH_CYCLE lacks phase_order")?
            .iter()
            .map(|phase| match phase.as_str() {
                Some("player_processing") => Ok(ServerPhase::PlayerProcessing),
                Some("object_lifecycle") => Ok(ServerPhase::ObjectLifecycle),
                Some("client_output") => Ok(ServerPhase::ClientOutput),
                _ => Err(format!("GRINCH_CYCLE {cycle} has an unknown server phase")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let active_actors = value["active_actors"]
            .as_array()
            .ok_or("GRINCH_CYCLE lacks active_actors")?
            .iter()
            .map(|active| {
                Ok(ActiveActor {
                    actor: actor_from_json(&active["actor"]).ok_or_else(|| {
                        format!("GRINCH_CYCLE {cycle} has an incomplete actor identity")
                    })?,
                    active: active["active"]
                        .as_bool()
                        .ok_or_else(|| format!("GRINCH_CYCLE {cycle} has no active flag"))?,
                    tile: tile_from_json(&active["tile"])
                        .ok_or_else(|| format!("GRINCH_CYCLE {cycle} has no actor tile"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let player_processing_order = value["player_processing_order"]
            .as_array()
            .ok_or("GRINCH_CYCLE lacks player_processing_order")?
            .iter()
            .map(|account| {
                account.as_str().map(str::to_owned).ok_or_else(|| {
                    format!("GRINCH_CYCLE {cycle} has a non-string processing account")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        cycles.push(ServerCycle {
            cycle,
            object_turn,
            phase_order,
            active_actors,
            player_processing_order,
        });
    }
    cycles.sort_by_key(|cycle| cycle.cycle);
    if cycles.windows(2).any(|pair| pair[0].cycle == pair[1].cycle) {
        return Err("scratch server log contains duplicate GRINCH_CYCLE identities".into());
    }
    Ok(cycles)
}

fn reconcile_raw_frames(
    runtime: &mut Runtime,
    logs: &ObservedLogs,
    case: Case,
) -> Result<(), String> {
    let cycles = server_cycles(logs)?;
    for actor in &mut runtime.actors {
        for frame in &mut actor.raw_frames {
            if frame.object_turn.is_some() {
                continue;
            }
            let mut matching_outputs = logs.client_outputs.iter().filter(|output| {
                output.actor == frame.actor
                    && output.player_info_sequence == frame.player_info_sequence
            });
            let Some(output) = matching_outputs.next() else {
                continue;
            };
            if matching_outputs.next().is_some() {
                return Err(format!(
                    "{} PLAYER_INFO edge {} maps to multiple GRINCH_CLIENT_OUTPUT rows",
                    frame.account, frame.player_info_sequence
                ));
            }
            let Ok(index) = cycles.binary_search_by_key(&output.cycle, |cycle| cycle.cycle) else {
                continue;
            };
            let cycle = &cycles[index];
            frame.object_turn = Some(cycle.object_turn);
            if case.is_b() {
                frame.server_cycle = Some(cycle.cycle);
                frame.output_phase = Some(
                    if cycle.phase_order
                        == [
                            ServerPhase::PlayerProcessing,
                            ServerPhase::ObjectLifecycle,
                            ServerPhase::ClientOutput,
                        ]
                    {
                        ClientOutputPhase::AfterObjectLifecycle
                    } else {
                        ClientOutputPhase::Unknown
                    },
                );
            }
        }
        if let Some(latest) = actor.raw_frames.last() {
            actor.latest = Some(latest.clone());
        }
    }
    Ok(())
}

fn validate_take_receive(take: &TakeDispatch, receive: &TakeReceiveEvent) -> Result<(), String> {
    let outcome_matches = matches!(
        (
            receive.accepted,
            receive.action.as_deref(),
            receive.reason.as_str()
        ),
        (true, Some("Take"), "interaction_armed") | (false, None, "handler_rejected")
    );
    if receive.actor != take.actor
        || receive.item_id != take.item_id
        || receive.tile != take.tile
        || receive.option != 3
        || !outcome_matches
    {
        return Err(format!(
            "server receipt {} does not identify the exact Take {} at {:?}",
            receive.receipt_sequence, take.item_id, take.tile
        ));
    }
    Ok(())
}

fn one_obj3_packet(opcodes: &[u8]) -> bool {
    let opcode = client::io::ClientProt289::OPOBJ3.id as u8;
    opcodes.iter().filter(|&&value| value == opcode).count() == 1
}

fn build_wire_takes(
    runtime: &RuntimeReport,
    logs: &ObservedLogs,
    case: Case,
    cycles: &[ServerCycle],
) -> Result<Vec<WireTake>, String> {
    let opcode = client::io::ClientProt289::OPOBJ3.id as u8;
    let mut takes = Vec::new();
    let mut used_packets = HashSet::new();
    for interaction in &logs.interacts {
        let InteractionKind::Ground(request) = &interaction.kind else {
            continue;
        };
        if request.action != "Take" {
            continue;
        }
        let tick = interaction
            .tick
            .ok_or("ground Take interaction lacks its host tick")?;
        let actor = runtime
            .actors
            .iter()
            .find(|actor| actor.account == interaction.slot_name)
            .ok_or_else(|| {
                format!(
                    "ground Take came from unregistered Play slot {}",
                    interaction.slot_name
                )
            })?;
        if actor.role == ActorRole::Helper {
            return Err("setup helper issued an unexpected ground Take".into());
        }
        let packets: Vec<&PacketEvent> = logs
            .packets
            .iter()
            .filter(|packet| {
                packet.slot_name == actor.account
                    && packet.account == actor.account
                    && packet.tick == tick
                    && packet.request == interaction.request
                    && if actor.role == ActorRole::Competitor {
                        packet.key.is_none()
                    } else {
                        packet.key.is_some()
                    }
            })
            .collect();
        if packets.len() != 1 {
            return Err(format!(
                "{} Take at tick {tick} maps to {} native packet records",
                actor.account,
                packets.len()
            ));
        }
        let packet = packets[0];
        if usize::try_from(packet.count).ok() != Some(packet.opcodes.len())
            || !one_obj3_packet(&packet.opcodes)
            || !packet.decoded
            || !packet.accepted
        {
            return Err(format!(
                "{} Take request {} was not one accepted decoded batch containing exactly one R289 OPOBJ3: count={} decoded={} accepted={} opcodes={:?}",
                actor.account, packet.request, packet.count, packet.decoded, packet.accepted, packet.opcodes,
            ));
        }
        if !used_packets.insert((packet.account.clone(), packet.tick, packet.request)) {
            return Err(
                "one native packet record was attributed to more than one ground Take".into(),
            );
        }
        takes.push(WireTake {
            key: packet.key,
            dispatch: TakeDispatch {
                request_key: format!("take:{}:{}:{}", actor.account, packet.tick, packet.request),
                actor: actor
                    .frames
                    .first()
                    .ok_or("Take actor has no observed PLAYER_INFO identity")?
                    .actor
                    .clone(),
                item_id: request.item_id,
                tile: request.tile,
                tick: packet.tick,
                server_cycle: None,
            },
            outcome: None,
            receive: None,
        });
    }
    for packet in &logs.packets {
        if packet.accepted
            && packet.opcodes.contains(&opcode)
            && !used_packets.contains(&(packet.account.clone(), packet.tick, packet.request))
        {
            return Err(format!(
                "accepted OPOBJ3 packet {} at tick {} lacks its exact interact Obj Take request",
                packet.request, packet.tick
            ));
        }
    }
    for actor in runtime
        .actors
        .iter()
        .filter(|actor| actor.role == ActorRole::Competitor)
    {
        for sent in &actor.competitor_takes {
            let frame = actor
                .frames
                .iter()
                .find(|frame| frame.observation_sequence == sent.observation_sequence)
                .ok_or("competitor Take lacks its exact observed PLAYER_INFO frame")?;
            if !sent.has_exact_send_evidence(frame) {
                return Err(
                    "competitor Take lacks its exact visible row and accepted OPOBJ3 wire".into(),
                );
            }
            takes.push(WireTake {
                key: None,
                dispatch: TakeDispatch {
                    request_key: sent.request_key.clone(),
                    actor: frame.actor.clone(),
                    item_id: sent.item_id,
                    tile: sent.tile,
                    tick: sent.tick,
                    server_cycle: None,
                },
                outcome: None,
                receive: None,
            });
        }
    }

    if case.is_b() {
        for actor in runtime
            .actors
            .iter()
            .filter(|actor| matches!(actor.role, ActorRole::Main | ActorRole::Competitor))
        {
            let actor_ref = actor
                .frames
                .first()
                .ok_or("scratch Take actor has no observed identity")?
                .actor
                .clone();
            let mut send_indices = takes
                .iter()
                .enumerate()
                .filter_map(|(index, take)| (take.dispatch.actor == actor_ref).then_some(index))
                .collect::<Vec<_>>();
            send_indices.sort_by_key(|index| {
                (
                    takes[*index].dispatch.tick,
                    takes[*index].dispatch.request_key.clone(),
                )
            });
            let mut receives = logs
                .take_receives
                .iter()
                .filter(|receive| receive.actor == actor_ref)
                .collect::<Vec<_>>();
            receives.sort_by_key(|receive| receive.receipt_sequence);
            if send_indices.len() != receives.len() {
                return Err(format!(
                    "{} has {} accepted ground Takes but {} exact server Take-receive receipts",
                    actor.account,
                    send_indices.len(),
                    receives.len(),
                ));
            }
            for (index, receive) in send_indices.into_iter().zip(receives) {
                let take = &mut takes[index];
                validate_take_receive(&take.dispatch, receive)?;
                if !cycles.iter().any(|cycle| cycle.cycle == receive.cycle) {
                    return Err(format!(
                        "server Take receipt {} references an unobserved cycle {}",
                        receive.receipt_sequence, receive.cycle
                    ));
                }
                take.dispatch.server_cycle = Some(receive.cycle);
                take.receive = Some(receive.clone());
            }
        }
    }

    let main = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("missing main actor")?;
    for take in takes
        .iter_mut()
        .filter(|take| take.dispatch.actor.account_id == main.account)
    {
        let key = take
            .key
            .ok_or("main Take lacks an observed native RunKey")?;
        let candidates = logs
            .take_events
            .iter()
            .filter(|event| {
                event.slot_name == main.account
                    && event.key == key
                    && event.item_id == take.dispatch.item_id
                    && event.tile == take.dispatch.tile
                    && event.tick >= take.dispatch.tick
                    && event.tick.saturating_sub(take.dispatch.tick) <= MAX_TAKE_SETTLEMENT_TICKS
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(format!(
                "main Take {} at tick {} maps to {} exact GRINCH_TAKE outcomes inside the settlement bound",
                take.dispatch.request_key, take.dispatch.tick, candidates.len(),
            ));
        }
        take.outcome = Some(candidates[0].clone());
    }
    let matched_outcomes = takes.iter().filter(|take| take.outcome.is_some()).count();
    let main_keys = takes
        .iter()
        .filter_map(|take| take.key)
        .collect::<HashSet<_>>();
    let observed_outcomes = logs
        .take_events
        .iter()
        .filter(|event| event.slot_name == main.account && main_keys.contains(&event.key))
        .count();
    if matched_outcomes != observed_outcomes {
        return Err("one or more GRINCH_TAKE outcomes lack a unique accepted wire dispatch".into());
    }
    Ok(takes)
}

#[derive(Debug, Clone)]
struct RestartTransition {
    account: String,
    action_key: WireRunKey,
    action_tick: u64,
    host_log_ordinal: u64,
    completion_host_log_ordinal: u64,
    before: RuntimeObservation,
    after: RuntimeObservation,
}

fn build_restart_transitions(
    runtime: &RuntimeReport,
    logs: &ObservedLogs,
) -> Result<Vec<RestartTransition>, String> {
    let mut actions = logs
        .watchdog
        .iter()
        .filter(|event| event.action.starts_with("Restart {"))
        .collect::<Vec<_>>();
    let mut completions = logs
        .watchdog
        .iter()
        .filter(|event| event.action == "legacy_script_restart")
        .collect::<Vec<_>>();
    actions.sort_by_key(|event| event.host_log_ordinal);
    completions.sort_by_key(|event| event.host_log_ordinal);
    if actions.len() != completions.len() {
        return Err(format!("watchdog restart action/completion ledger is incomplete: {} actions, {} completion rows", actions.len(), completions.len()));
    }
    let mut transitions = Vec::with_capacity(actions.len());
    let mut used_samples = HashSet::new();
    for (action, completion) in actions.into_iter().zip(completions) {
        if action.slot_name != completion.slot_name
            || action.host_log_ordinal >= completion.host_log_ordinal
        {
            return Err("watchdog restart action is not paired with its ordered same-slot completion record".into());
        }
        let account = action
            .slot_name
            .as_deref()
            .filter(|account| !account.is_empty())
            .ok_or("watchdog Restart action has no exact account identity")?;
        let key = action
            .key
            .ok_or("watchdog Restart action has no observed native RunKey")?;
        if completion.key != Some(key) {
            return Err(
                "watchdog restart completion does not retain the exact action RunKey".into(),
            );
        }
        let tick = action
            .tick
            .ok_or("watchdog Restart action has no observed native tick")?;
        if completion.tick != Some(tick) {
            return Err("watchdog restart completion does not retain the exact action tick".into());
        }
        let before = runtime
            .runtime_observations
            .iter()
            .rev()
            .find(|sample| {
                sample.start_requested
                    && sample.account == account
                    && sample.generation.is_some()
                    && sample.run_key == Some(key)
                    && sample.tick.is_some_and(|sample_tick| sample_tick <= tick)
            })
            .cloned()
            .ok_or(
                "watchdog Restart action has no preceding same-account generation/RunKey sample",
            )?;
        let after = runtime.runtime_observations.iter().find(|sample| {
            sample.sample_ordinal > before.sample_ordinal
                && sample.account == account
                && sample.start_requested
                && sample.generation.is_some()
                && sample.generation != before.generation
                && sample.run_key.is_some()
                && sample.run_key != before.run_key
                && sample.observation_sequence > before.observation_sequence
                && sample.tick.is_some_and(|sample_tick| sample_tick >= tick)
                && !used_samples.contains(&sample.sample_ordinal)
        }).cloned().ok_or("watchdog Restart action lacks a linked same-account runtime-generation and RunKey transition")?;
        used_samples.insert(after.sample_ordinal);
        transitions.push(RestartTransition {
            account: account.to_owned(),
            action_key: key,
            action_tick: tick,
            host_log_ordinal: action.host_log_ordinal,
            completion_host_log_ordinal: completion.host_log_ordinal,
            before,
            after,
        });
    }
    let mut previous_after = HashMap::<String, WireRunKey>::new();
    let mut transitioned_keys = HashSet::<(String, WireRunKey)>::new();
    for transition in &transitions {
        let after_key = transition
            .after
            .run_key
            .ok_or("restart transition lacks its exact post-restart RunKey")?;
        if !transitioned_keys.insert((transition.account.clone(), after_key)) {
            return Err("restart transition ledger repeats an account RunKey".into());
        }
        let before_key = transition
            .before
            .run_key
            .ok_or("restart transition lacks its exact pre-restart RunKey")?;
        if previous_after
            .get(&transition.account)
            .is_some_and(|prior| *prior != before_key)
        {
            return Err(
                "restart transition ledger does not chain from the prior sampled RunKey".into(),
            );
        }
        previous_after.insert(transition.account.clone(), after_key);
    }
    Ok(transitions)
}

fn required_u32(value: &Value, field: &str, context: &str) -> Result<u32, String> {
    u32::try_from(
        value[field]
            .as_u64()
            .ok_or_else(|| format!("{context} lacks unsigned {field}"))?,
    )
    .map_err(|_| format!("{context} {field} overflows u32"))
}

fn strict_id_set(value: &Value, field: &str, context: &str) -> Result<BTreeSet<i32>, String> {
    value[field]
        .as_array()
        .ok_or_else(|| format!("{context} lacks {field} array"))?
        .iter()
        .map(|id| {
            i32::try_from(
                id.as_i64()
                    .ok_or_else(|| format!("{context} has non-integer {field} member"))?,
            )
            .map_err(|_| format!("{context} has out-of-range {field} member"))
        })
        .collect()
}

fn failed_key_snapshot(
    value: &Value,
    context: &str,
) -> Result<(BTreeMap<GroundKey, u32>, BTreeSet<GroundKey>), String> {
    let records = value["failed_keys"]
        .as_array()
        .ok_or_else(|| format!("{context} lacks complete failed_keys array"))?;
    let mut counts = BTreeMap::new();
    let mut exclusions = BTreeSet::new();
    for record in records {
        let key = GroundKey {
            item_id: i32::try_from(
                record["item_id"]
                    .as_i64()
                    .ok_or_else(|| format!("{context} has failed-key row without item ID"))?,
            )
            .map_err(|_| format!("{context} has out-of-range failed-key item ID"))?,
            tile: tile_from_json(&record["tile"])
                .ok_or_else(|| format!("{context} has failed-key row without exact tile"))?,
        };
        let failures = u32::try_from(
            record["failures"]
                .as_u64()
                .ok_or_else(|| format!("{context} has failed-key row without failure count"))?,
        )
        .map_err(|_| format!("{context} failed-key count overflows u32"))?;
        if failures == 0 || record["last_failure_tick"].as_u64().is_none() {
            return Err(format!(
                "{context} has incomplete failed-key failure evidence"
            ));
        }
        let excluded = record["excluded"]
            .as_bool()
            .ok_or_else(|| format!("{context} has failed-key row without exclusion state"))?;
        if counts.insert(key.clone(), failures).is_some() {
            return Err(format!("{context} repeats one failed ground key"));
        }
        if excluded {
            exclusions.insert(key);
        }
    }
    Ok((counts, exclusions))
}

fn native_status_for_frame(
    event: &StateEvent,
    capture: &RuntimeObservation,
    client_session_id: &str,
    logs: &ObservedLogs,
    takes: &[WireTake],
    restarts: &[RestartTransition],
    raw_frames: &[RawFrame],
) -> Result<NativeStatus, String> {
    let frame_tick = capture
        .tick
        .ok_or("native status capture lacks its owning PLAYER_INFO tick")?;
    let context = format!(
        "GRINCH_STATE run {} tick {}",
        event.key.run_id(&event.slot_name),
        event.tick
    );
    let value = &event.payload;
    let anchor =
        tile_from_json(&value["anchor"]).ok_or_else(|| format!("{context} lacks its anchor"))?;
    let phase = status_phase(
        value["phase"]
            .as_str()
            .ok_or_else(|| format!("{context} lacks phase"))?,
    )
    .ok_or_else(|| format!("{context} has unknown phase"))?;
    let blocked_reason = match value["blocked_reason"].as_str() {
        Some("no_selected_item_to_deposit") => Some(BlockedReason::NoSelectedItemToDeposit),
        Some("deposit_did_not_free_space") => Some(BlockedReason::DepositDidNotFreeSpace),
        Some(_) => return Err(format!("{context} has unknown blocked reason")),
        None if value["blocked_reason"].is_null() => None,
        None => return Err(format!("{context} has malformed blocked reason")),
    };
    let foreground_children = required_u32(value, "foreground_children", &context)?;
    let id_exclusions = strict_id_set(value, "id_exclusions", &context)?;
    let (_, failed_key_exclusions) = failed_key_snapshot(value, &context)?;
    let pickup_units = required_u32(value, "picked", &context)?;
    let banked_units = required_u32(value, "banked", &context)?;
    let take_sends = takes
        .iter()
        .filter(|take| {
            take.key == Some(event.key)
                && take.dispatch.actor.account_id == event.slot_name
                && take.dispatch.tick < frame_tick
        })
        .count();
    let failed_key_attempts = takes
        .iter()
        .filter(|take| {
            take.key == Some(event.key)
                && take.dispatch.actor.account_id == event.slot_name
                && take.outcome.as_ref().is_some_and(|outcome| {
                    outcome.outcome == "failed_key"
                        && outcome.tick <= frame_tick
                        && outcome.host_log_ordinal <= capture.host_log_ordinal_fence
                })
        })
        .count();
    let duplicate_refusals = takes
        .iter()
        .filter(|take| {
            take.key == Some(event.key)
                && take.dispatch.actor.account_id == event.slot_name
                && take.outcome.as_ref().is_some_and(|outcome| {
                    outcome.outcome == "already_owned"
                        && outcome.tick <= frame_tick
                        && outcome.host_log_ordinal <= capture.host_log_ordinal_fence
                })
        })
        .count();
    let bank_trips = bank_trip_count(logs, event.key, &event.slot_name, frame_tick, raw_frames)?;
    let recovery_count = logs
        .watchdog
        .iter()
        .filter(|action| {
            action.key == Some(event.key)
                && action.slot_name.as_deref() == Some(event.slot_name.as_str())
                && action.tick.is_some_and(|tick| tick <= frame_tick)
                && action.host_log_ordinal <= capture.host_log_ordinal_fence
                && action.action.starts_with("ArmWalk {")
        })
        .count();
    let restart_count = restarts
        .iter()
        .filter(|restart| restart.account == event.slot_name)
        .position(|restart| restart.after.run_key == Some(event.key))
        .map_or(0, |index| index + 1);
    let counts = avoidance_entry_count(
        logs,
        event.key,
        &event.slot_name,
        event.tick,
        event.host_log_ordinal,
    )?;
    Ok(NativeStatus {
        run_id: event.key.run_id(&event.slot_name),
        session_id: client_session_id.to_owned(),
        anchor,
        phase,
        blocked_reason,
        pickup_units,
        banked_units,
        take_sends: u32::try_from(take_sends)
            .map_err(|_| "native Take-send count overflows u32")?,
        bank_trips,
        failed_key_attempts: u32::try_from(failed_key_attempts)
            .map_err(|_| "native failed-key count overflows u32")?,
        avoidance_entries: counts,
        duplicate_refusals: u32::try_from(duplicate_refusals)
            .map_err(|_| "native duplicate-refusal count overflows u32")?,
        failed_key_exclusions,
        id_exclusions,
        recovery_count: u32::try_from(recovery_count)
            .map_err(|_| "native recovery count overflows u32")?,
        restart_count: u32::try_from(restart_count)
            .map_err(|_| "native restart count overflows u32")?,
        foreground_children,
    })
}

fn bank_trip_count(
    logs: &ObservedLogs,
    key: WireRunKey,
    slot_name: &str,
    through_tick: u64,
    raw_frames: &[RawFrame],
) -> Result<u32, String> {
    let mut trips = 0u32;
    for interaction in logs.interacts.iter().filter(|event| {
        event.slot_name == slot_name
            && event.tick.is_some_and(|tick| tick <= through_tick)
            && matches!(event.kind, InteractionKind::OpenBank { .. })
    }) {
        let packet = accepted_packet_for_interaction(logs, slot_name, key, interaction)?;
        if raw_frames.iter().any(|frame| {
            frame.tick >= packet.tick
                && frame.tick <= through_tick
                && frame.bank_open
                && frame.bank_loaded
                && frame.bank.as_ref().is_some_and(|bank| bank.complete)
        }) {
            trips = trips.checked_add(1).ok_or("bank-trip count overflowed")?;
        }
    }
    Ok(trips)
}

fn avoidance_entry_count(
    logs: &ObservedLogs,
    key: WireRunKey,
    slot_name: &str,
    through_tick: u64,
    through_host_log_ordinal: u64,
) -> Result<u32, String> {
    let mut events = logs
        .states
        .iter()
        .filter(|event| {
            event.key == key
                && event.slot_name == slot_name
                && event.tick <= through_tick
                && event.host_log_ordinal <= through_host_log_ordinal
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.tick);
    let mut prior = BTreeMap::<GroundKey, u32>::new();
    let mut total = 0u32;
    for event in events {
        let (current, _) = failed_key_snapshot(&event.payload, "GRINCH_STATE history")?;
        for (ground_key, failures) in &current {
            let previous = prior.get(ground_key).copied().unwrap_or(0);
            if *failures < previous {
                return Err("failed-key counter moved backwards within one native RunKey".into());
            }
            total = total
                .checked_add(failures - previous)
                .ok_or("avoidance-entry count overflowed")?;
        }
        prior = current;
    }
    Ok(total)
}

fn capture_condition(
    case: Case,
    runtime: &Runtime,
    logs: &ObservedLogs,
) -> (bool, Option<(u64, u64)>) {
    let Some(key) = runtime.run_key else {
        return (false, None);
    };
    let main = &runtime.actors[runtime.main_index];
    let Some(first_state) = logs
        .states
        .iter()
        .filter(|state| state.key == key)
        .min_by_key(|state| state.tick)
    else {
        return (false, None);
    };
    let Some(latest) = main.latest.as_ref() else {
        return (false, Some((first_state.tick, case.capture_budget_ticks())));
    };
    let latest_status =
        runtime_sample_for_frame(&runtime.runtime_observations, latest).and_then(|sample| {
            state_for_frame(
                logs,
                &main.account,
                latest.tick,
                key,
                sample.host_log_ordinal_fence,
            )
        });
    let status = latest_status.and_then(parse_status_core);
    let Some(status) = status else {
        return (false, Some((first_state.tick, case.capture_budget_ticks())));
    };
    let frames_after = main
        .raw_frames
        .iter()
        .filter(|frame| frame.tick >= first_state.tick)
        .count();
    let cond = match case {
        Case::ATradeableDrops | Case::AGreenMaskSelection => {
            let needed = case.expected_pickups();
            let last_delta = observed_inventory_delta_tick(
                main,
                first_state.tick,
                case.selected_ids()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap_or(first_state.tick);
            status.pickup_units == needed
                && status.phase == StatusPhase::Waiting
                && latest.tick.saturating_sub(last_delta) >= 40
        }
        Case::ANegativeCandidates => {
            let last_drop = runtime.actors[runtime.helper_index.unwrap()]
                .drop_actions
                .iter()
                .map(|drop| drop.tick)
                .max()
                .unwrap_or(first_state.tick);
            let rows_visible = runtime.actors[runtime.helper_index.unwrap()]
                .drop_actions
                .iter()
                .all(|drop| {
                    main.latest
                        .as_ref()
                        .is_some_and(|frame| frame.ground.count(drop.item_id, drop.tile) > 0)
                });
            status.pickup_units == 0 && rows_visible && latest.tick.saturating_sub(last_drop) >= 40
        }
        Case::APrivateUntradeableDrops => {
            let helper = &runtime.actors[runtime.helper_index.unwrap()];
            let Some(post_drop) = helper.helper_post_drop.as_ref() else {
                return (false, Some((first_state.tick, 160)));
            };
            let fence = main.raw_frames.iter().find(|frame| {
                frame.observation_sequence > post_drop.observation_sequence
                    && frame.tick
                        > helper
                            .drop_actions
                            .iter()
                            .map(|drop| drop.tick)
                            .max()
                            .unwrap_or(0)
            });
            fence.is_some_and(|fence| {
                main.raw_frames
                    .iter()
                    .filter(|frame| frame.observation_sequence > fence.observation_sequence)
                    .count()
                    >= 140
                    && latest.tick >= fence.tick.saturating_add(140)
                    && status.phase == StatusPhase::Waiting
            })
        }
        Case::BPumpkinRespawns => {
            status.pickup_units == 6
                && inventory_count(&main.snapshot, PUMPKIN_ID) == 6
                && latest.tick.saturating_sub(status.last_state_tick) >= MAX_TAKE_SETTLEMENT_TICKS
        }
        Case::BContestedPumpkin => {
            let comp = runtime.competitor_index.map(|index| &runtime.actors[index]);
            status.pickup_units == 2
                && comp.is_some_and(|actor| inventory_count(&actor.snapshot, PUMPKIN_ID) == 3)
                && inventory_count(&main.snapshot, PUMPKIN_ID) == 2
        }
        Case::BScytheDuplicate | Case::BBunnyEarsDuplicate => {
            let item = if case == Case::BScytheDuplicate {
                SCYTHE_ID
            } else {
                BUNNY_EARS_ID
            };
            let refusal_tick = logs
                .take_events
                .iter()
                .filter(|event| {
                    event.key == key && event.outcome == "already_owned" && event.item_id == item
                })
                .map(|event| event.tick)
                .max();
            refusal_tick.is_some_and(|tick| {
                status.id_exclusions.contains(&item)
                    && status.phase == StatusPhase::Waiting
                    && latest.tick.saturating_sub(tick) >= 40
            })
        }
        Case::BYoYoRepeat => {
            status.pickup_units == 2 && inventory_count(&main.snapshot, YO_YO_ID) == 2
        }
        Case::CDeposit28 | Case::CDeposit27PreserveItem => {
            let amount = if case == Case::CDeposit28 { 28 } else { 27 };
            status.pickup_units == 2
                && status.banked_units == amount
                && latest.position == ANCHOR
                && !latest.bank_open
                && latest.tick.saturating_sub(status.last_state_tick) >= MAX_TAKE_SETTLEMENT_TICKS
        }
        Case::CInventoryBlocked => {
            status.phase == StatusPhase::InventoryBlocked && frames_after >= 2
        }
    };
    (cond, Some((first_state.tick, case.capture_budget_ticks())))
}

#[derive(Debug, Clone)]
struct StatusCore {
    phase: StatusPhase,
    pickup_units: u32,
    banked_units: u32,
    id_exclusions: BTreeSet<i32>,
    last_state_tick: u64,
}

fn parse_status_core(event: &StateEvent) -> Option<StatusCore> {
    let value = &event.payload;
    Some(StatusCore {
        phase: status_phase(value["phase"].as_str()?)?,
        pickup_units: u32::try_from(value["picked"].as_u64()?).ok()?,
        banked_units: u32::try_from(value["banked"].as_u64()?).ok()?,
        id_exclusions: value["id_exclusions"]
            .as_array()?
            .iter()
            .filter_map(Value::as_i64)
            .filter_map(|id| i32::try_from(id).ok())
            .collect(),
        last_state_tick: event.tick,
    })
}

fn observed_inventory_delta_tick(main: &ActorState, start_tick: u64, ids: &[i32]) -> Option<u64> {
    main.raw_frames
        .windows(2)
        .rev()
        .find(|pair| {
            pair[0].tick >= start_tick
                && ids
                    .iter()
                    .any(|&id| pair[1].inventory.count(id) > pair[0].inventory.count(id))
        })
        .map(|pair| pair[1].tick)
}

fn status_phase(value: &str) -> Option<StatusPhase> {
    match value {
        "preparing" => Some(StatusPhase::Preparing),
        "scanning" | "taking" => Some(StatusPhase::Working),
        "waiting" => Some(StatusPhase::Waiting),
        "banking" => Some(StatusPhase::Banking),
        "returning" => Some(StatusPhase::Returning),
        "inventory_blocked" => Some(StatusPhase::InventoryBlocked),
        "stopped" => Some(StatusPhase::Stopped),
        _ => None,
    }
}
fn latest_status_phase(
    account: &str,
    run_key: Option<WireRunKey>,
    tick: u64,
) -> Option<StatusPhase> {
    let key = run_key?;
    let rows = HOST_LOGS.lock().ok()?;
    rows.iter().rev().find_map(|row| {
        if row.slot.as_deref() != Some(account) {
            return None;
        }
        let payload = row.message.strip_prefix("GRINCH_STATE ")?;
        let value = serde_json::from_str::<Value>(payload).ok()?;
        value["tick"].as_u64()?;
        if run_key_from_json(&value["run"]) != Some(key) || u64::from(row.tick?) > tick {
            return None;
        }
        status_phase(value["phase"].as_str()?)
    })
}

fn main_observed_closed_after_deposit(
    main: &ActorState,
    run_key: Option<WireRunKey>,
    through_tick: u64,
) -> bool {
    let Some(key) = run_key else {
        return false;
    };
    let Ok(rows) = HOST_LOGS.lock() else {
        return false;
    };
    rows.iter().any(|row| {
        row.slot.as_deref() == Some(main.account.as_str())
            && parse_bank_dispatch(&row.message, row.tick).is_some_and(|dispatch| {
                dispatch.key == key
                    && dispatch.accepted
                    && dispatch.emitted_packets > 0
                    && main.raw_frames.iter().any(|frame| {
                        frame.tick > dispatch.tick && frame.tick <= through_tick && !frame.bank_open
                    })
            })
    })
}

fn write_host_log(path: &Path, rows: &[ObservedLogRecord]) -> Result<(), String> {
    let mut text = String::new();
    for row in rows {
        text.push_str(&format!(
            "slot={:?} tick={:?} source={} level={} {}\n",
            row.slot, row.tick, row.source, row.level, row.message
        ));
    }
    atomic_write(path, text.as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let suffix = format!(
        "{}.{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let temp = path.with_extension(suffix);
    fs::write(&temp, bytes).map_err(|error| format!("write {}: {error}", temp.display()))?;
    fs::rename(&temp, path).map_err(|error| format!("publish {}: {error}", path.display()))
}

fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize evidence JSON: {error}"))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn evidence_path(dir: &Path, case: Case, suffix: &str) -> PathBuf {
    dir.join(format!("{}.{}", case.slug(), suffix))
}

fn state_for_frame<'a>(
    logs: &'a ObservedLogs,
    account: &str,
    tick: u64,
    key: WireRunKey,
    host_log_ordinal_fence: u64,
) -> Option<&'a StateEvent> {
    logs.states
        .iter()
        .filter(|event| {
            event.slot_name == account
                && event.tick <= tick
                && event.key == key
                && event.host_log_ordinal <= host_log_ordinal_fence
        })
        .max_by_key(|event| event.host_log_ordinal)
}

fn run_key_value(key: Option<WireRunKey>) -> Value {
    key.map(|key| json!({"slot":key.slot,"run":key.run,"session":key.session}))
        .unwrap_or(Value::Null)
}

fn setup_seed_actions(runtime: &RuntimeReport) -> Vec<SetupSeedAction> {
    let mut actions = runtime
        .actors
        .iter()
        .filter_map(|actor| {
            let has_seed = actor.seed_commands.iter().any(|command| {
                command["command"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("givebank "))
            });
            if !has_seed {
                return None;
            }
            let seed = actor.seed_observation.as_ref()?;
            Some(SetupSeedAction {
                actor: seed.actor.clone(),
                observation_sequence: seed.observation_sequence,
                inventory: seed.inventory.clone(),
            })
        })
        .collect::<Vec<_>>();
    actions.sort_by_key(|action| action.observation_sequence);
    actions
}

fn setup_seed_ledger_complete(runtime: &RuntimeReport, start_fence: Option<u64>) -> bool {
    let Some(start_fence) = start_fence else {
        return false;
    };
    runtime.actors.iter().all(|actor| {
        let commands_valid = actor.seed_commands.iter().all(|command| {
            command["actor"].as_str() == Some(actor.account.as_str())
                && command["accepted"].as_bool() == Some(true)
                && command["command"].as_str().is_some()
        });
        let item_seeded = actor.seed_commands.iter().any(|command| {
            command["command"]
                .as_str()
                .is_some_and(|text| text.starts_with("givebank "))
        });
        commands_valid
            && (!item_seeded
                || actor
                    .seed_observation
                    .as_ref()
                    .is_some_and(|seed| seed.observation_sequence < start_fence))
    })
}

fn runtime_sample_for_frame<'a>(
    observations: &'a [RuntimeObservation],
    frame: &RawFrame,
) -> Option<&'a RuntimeObservation> {
    observations
        .iter()
        .filter(|sample| {
            sample.account == frame.account
                && sample.observation_sequence == frame.observation_sequence
                && sample.player_info_sequence == Some(frame.player_info_sequence)
                && sample.tick == Some(frame.tick)
                && sample.client_session_generation == Some(frame.client_session_generation)
                && sample.client_session_id.as_deref().is_some_and(|id| {
                    let Some(generation) = id
                        .strip_prefix("client:")
                        .and_then(|id| id.strip_prefix(frame.account.as_str()))
                        .and_then(|id| id.strip_prefix(":session:"))
                    else {
                        return false;
                    };
                    (generation == "0" || !generation.starts_with('0'))
                        && generation.bytes().all(|byte| byte.is_ascii_digit())
                        && generation.parse::<u64>().ok() == Some(frame.client_session_generation)
                })
        })
        .max_by_key(|sample| sample.sample_ordinal)
}

fn frame_for_native_tick<'a>(
    runtime: &'a RuntimeReport,
    account: &str,
    key: WireRunKey,
    native_tick: u64,
) -> Result<&'a RawFrame, String> {
    let mut frames = runtime
        .actors
        .iter()
        .filter(|actor| actor.account == account)
        .flat_map(|actor| &actor.frames)
        .filter(|frame| {
            frame.player_info_sequence == native_tick
                && runtime_sample_for_frame(&runtime.runtime_observations, frame)
                    .is_some_and(|sample| runtime_sample_has_key(runtime, sample, key))
        });
    let frame = frames.next().ok_or_else(|| {
        format!("native clock {native_tick} for {account} lacks an exact RunKey-linked PLAYER_INFO edge")
    })?;
    if frames.next().is_some() {
        return Err(format!(
            "native clock {native_tick} for {account} maps to multiple PLAYER_INFO edges"
        ));
    }
    Ok(frame)
}

fn player_run_parts(
    case: Case,
    session: &SessionData,
    takes: &[WireTake],
    restarts: &[RestartTransition],
) -> Result<
    (
        RunIdentity,
        PreStartInventoryObservation,
        Vec<SetupSeedAction>,
        Vec<PlayerFrame>,
    ),
    String,
> {
    let runtime = &session.runtime;
    if !runtime.start_requested {
        return Err("compiled Grinch Start was not observed".into());
    }
    let start_fence = runtime
        .start_observation_sequence
        .ok_or("compiled Grinch Start lacks its pre-Start observation fence")?;
    let pre = runtime
        .pre_start_frame
        .as_ref()
        .ok_or("compiled Grinch Start lacks its exact pre-Start PLAYER_INFO frame")?;
    let main = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("live report lacks the main actor")?;
    let key = runtime
        .run_key
        .ok_or("compiled Grinch Start has no observed native RunKey")?;
    let first = main
        .frames
        .iter()
        .find(|frame| {
            frame.observation_sequence > start_fence
                && runtime_sample_for_frame(&runtime.runtime_observations, frame)
                    .is_some_and(|sample| {
                        runtime_sample_has_key(runtime, sample, key)
                            && state_for_frame(
                                &session.logs,
                                &main.account,
                                frame.tick,
                                key,
                                sample.host_log_ordinal_fence,
                            )
                            .is_some()
                    })
        })
        .ok_or("compiled Grinch Start has no later PLAYER_INFO edge with its exact native RunKey and state")?;
    let accounts = runtime
        .actors
        .iter()
        .map(|actor| actor.account.as_str())
        .collect::<BTreeSet<_>>();
    let fresh = accounts.len() == runtime.actors.len()
        && accounts
            .iter()
            .all(|account| session.fresh_accounts.contains(*account));
    let identity = RunIdentity {
        actor: first.actor.clone(),
        run_id: key.run_id(&main.account),
        session_id: pre.session_id(),
        anchor: ANCHOR,
        start_tick: first.tick,
        fresh_account: fresh,
    };
    let pre_start_inventory = PreStartInventoryObservation {
        actor: pre.actor.clone(),
        session_id: pre.session_id(),
        player_info_sequence: pre.player_info_sequence,
        observation_sequence: pre.observation_sequence,
        inventory: pre.inventory.clone(),
    };
    let seeds = setup_seed_actions(runtime);
    let mut frames = Vec::new();
    for raw in main
        .frames
        .iter()
        .filter(|frame| frame.observation_sequence >= first.observation_sequence)
    {
        let capture_sample = runtime_sample_for_frame(&runtime.runtime_observations, raw)
            .ok_or_else(|| {
                format!(
                "main PLAYER_INFO frame at tick {} lacks an exact linked runtime capture sample",
                raw.tick
            )
            })?;
        let key = capture_sample
            .run_key
            .or_else(|| terminal_sample_has_key(runtime, capture_sample, key).then_some(key))
            .ok_or_else(|| {
                format!(
                    "main PLAYER_INFO frame at tick {} lacks its sampled native RunKey",
                    raw.tick
                )
            })?;
        let status = state_for_frame(
            &session.logs,
            &main.account,
            raw.tick,
            key,
            capture_sample.host_log_ordinal_fence,
        )
        .map(|event| {
            native_status_for_frame(
                event,
                capture_sample,
                &identity.session_id,
                &session.logs,
                takes,
                restarts,
                &main.frames,
            )
        })
        .transpose()?;
        frames.push(raw.to_player_frame(status, Some(&identity.session_id)));
    }
    if frames.is_empty() {
        return Err("compiled Grinch Start has no captured main PLAYER_INFO frames".into());
    }
    let _ = case;
    Ok((identity, pre_start_inventory, seeds, frames))
}

fn helper_setup(runtime: &RuntimeReport) -> Result<HelperSetup, String> {
    let helper = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Helper)
        .ok_or("this trace requires a real helper actor")?;
    let seed = helper
        .seed_observation
        .as_ref()
        .ok_or("helper lacks a complete pre-Start seeded PLAYER_INFO observation")?;
    let post_drop = helper
        .helper_post_drop
        .as_ref()
        .ok_or("helper lacks a post-Drop PLAYER_INFO confirmation")?;
    let last_drop = helper
        .drop_actions
        .iter()
        .map(|drop| drop.tick)
        .max()
        .ok_or("helper has no observed real Drop action")?;
    let main = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("missing receiver actor")?;
    let receiver_fence = main
        .frames
        .iter()
        .find(|frame| {
            frame.observation_sequence > post_drop.observation_sequence && frame.tick > last_drop
        })
        .ok_or("no receiver PLAYER_INFO edge followed final helper Drop confirmation")?;
    let account_set = runtime
        .actors
        .iter()
        .map(|actor| actor.account.as_str())
        .collect::<BTreeSet<_>>();
    let helper_seed_action_isolated_to_helper = helper.seed_commands.iter().all(|command| {
        command["actor"].as_str() == Some(helper.account.as_str())
            && command["accepted"].as_bool() == Some(true)
    }) && account_set.len() == runtime.actors.len();
    let target_spawn_cheat_used = helper.seed_commands.iter().any(|command| {
        command["command"].as_str().is_some_and(|text| {
            let lower = text.to_ascii_lowercase();
            lower.contains("spawn") || lower.contains("placeobj") || lower.contains("addobj")
        })
    });
    Ok(HelperSetup {
        actor: seed.actor.clone(),
        seeded_inventory: seed.inventory.clone(),
        post_drop_inventory: post_drop.inventory.clone(),
        post_drop_player_info_sequence: post_drop.player_info_sequence,
        post_drop_observation_sequence: post_drop.observation_sequence,
        receiver_fence_tick: receiver_fence.tick,
        seeded_before_grinch_start: seed.observation_sequence
            < runtime
                .start_observation_sequence
                .ok_or("missing Start fence")?,
        helper_seed_action_isolated_to_helper,
        target_spawn_cheat_used,
    })
}

fn trace_authority(
    runtime: &RuntimeReport,
    main_frames: &[PlayerFrame],
    helper_complete: bool,
    server_complete: bool,
    bank_log_complete: bool,
    bank_snapshots_complete: bool,
    takes_complete: bool,
) -> TraceAuthority {
    TraceAuthority {
        player_info_edges_complete: !main_frames.is_empty()
            && main_frames
                .iter()
                .all(|frame| frame.player_info_sequence.is_some()),
        ground_pages_complete: !main_frames.is_empty()
            && main_frames.iter().all(|frame| {
                frame
                    .ground
                    .as_ref()
                    .is_some_and(|ground| ground.state == GroundPageState::Complete)
            }),
        inventory_snapshots_complete: !main_frames.is_empty()
            && main_frames.iter().all(|frame| {
                frame
                    .inventory
                    .as_ref()
                    .is_some_and(|inventory| inventory.complete)
            }),
        native_status_complete: runtime_observation_ledger_is_complete(runtime)
            && !main_frames.is_empty()
            && main_frames.iter().all(|frame| frame.status.is_some()),
        accepted_take_ledger_complete: takes_complete,
        setup_seed_actions_complete: setup_seed_ledger_complete(
            runtime,
            runtime.start_observation_sequence,
        ),
        helper_drop_actions_complete: helper_complete,
        object_turns_complete: !main_frames.is_empty()
            && main_frames.iter().all(|frame| frame.object_turn.is_some()),
        server_cycles_complete: server_complete,
        bank_action_log_complete: bank_log_complete,
        bank_snapshots_complete,
    }
}

fn validate_server_cycle_ledger(
    logs: &ObservedLogs,
    cycles: &[ServerCycle],
    (first, last): (u64, u64),
) -> Result<(), String> {
    let cycles = &cycles[cycles.partition_point(|cycle| cycle.cycle < first)
        ..cycles.partition_point(|cycle| cycle.cycle <= last)];
    let debug_ticks = logs
        .debug_ticks
        .iter()
        .filter(|event| first <= event.cycle && event.cycle <= last);
    let debug_count = debug_ticks.clone().count();
    if cycles.is_empty() || debug_count == 0 {
        return Err(
            "scratch capture lacks complete GRINCH_CYCLE and debugProfile tick evidence".into(),
        );
    }
    if cycles.len() != debug_count {
        return Err(format!(
            "scratch cycle/debug tick ledgers differ: {} cycles, {} debug ticks",
            cycles.len(),
            debug_count
        ));
    }
    let mut debug = HashMap::<u64, &DebugTickEvent>::new();
    for event in debug_ticks {
        if debug.insert(event.cycle, event).is_some() {
            return Err(format!(
                "scratch debugProfile repeats cycle {}",
                event.cycle
            ));
        }
    }
    for cycle in cycles {
        let event = debug.get(&cycle.cycle).ok_or_else(|| {
            format!(
                "GRINCH_CYCLE {} lacks its exact debugProfile tick row",
                cycle.cycle
            )
        })?;
        if event.server_line_ordinal == 0 {
            return Err("debugProfile tick row has no server log ordinal".into());
        }
    }
    if cycles.first().map(|row| row.cycle) != Some(first)
        || cycles.last().map(|row| row.cycle) != Some(last)
        || cycles
            .windows(2)
            .any(|pair| pair[1].cycle != pair[0].cycle.saturating_add(1))
    {
        return Err(
            "scratch GRINCH_CYCLE ledger has a missing server cycle in its required window".into(),
        );
    }
    let cycle_ids = cycles
        .iter()
        .map(|cycle| cycle.cycle)
        .collect::<HashSet<_>>();
    let mut output_ids = HashSet::new();
    for output in logs
        .client_outputs
        .iter()
        .filter(|output| first <= output.cycle && output.cycle <= last)
    {
        if !cycle_ids.contains(&output.cycle) {
            return Err(format!(
                "GRINCH_CLIENT_OUTPUT references unobserved cycle {}",
                output.cycle
            ));
        }
        if !output_ids.insert((output.actor.clone(), output.player_info_sequence)) {
            return Err("scratch server log repeats one actor PLAYER_INFO output edge".into());
        }
        let tick = debug
            .get(&output.cycle)
            .ok_or("client output has no debugProfile cycle row")?;
        if output.server_line_ordinal >= tick.server_line_ordinal {
            return Err(format!(
                "PLAYER_INFO output in cycle {} did not precede node.debugProfile tick output",
                output.cycle
            ));
        }
    }
    for receive in logs
        .take_receives
        .iter()
        .filter(|receive| first <= receive.cycle && receive.cycle <= last)
    {
        if !cycle_ids.contains(&receive.cycle) {
            return Err(format!(
                "server Take receive {} references unobserved cycle {}",
                receive.receipt_sequence, receive.cycle
            ));
        }
    }
    Ok(())
}

fn server_frames_complete(runtime: &RuntimeReport, logs: &ObservedLogs, start_fence: u64) -> bool {
    let mut observed = HashSet::new();
    for actor in &runtime.actors {
        for frame in actor.frames.iter().filter(|frame| {
            actor.role != ActorRole::Main || frame.observation_sequence > start_fence
        }) {
            let key = (frame.actor.clone(), frame.player_info_sequence);
            if frame.server_cycle.is_none()
                || frame.output_phase != Some(ClientOutputPhase::AfterObjectLifecycle)
                || !observed.insert(key)
            {
                return false;
            }
        }
    }
    let main_pre_sequence = runtime
        .pre_start_frame
        .as_ref()
        .map(|frame| frame.player_info_sequence);
    let logged_outputs = logs
        .client_outputs
        .iter()
        .filter(|output| {
            runtime.actors.iter().any(|actor| {
                if actor.account != output.actor.account_id {
                    return false;
                }
                actor.role != ActorRole::Main
                    || main_pre_sequence
                        .is_some_and(|sequence| output.player_info_sequence > sequence)
            })
        })
        .map(|output| (output.actor.clone(), output.player_info_sequence))
        .collect::<HashSet<_>>();
    observed == logged_outputs
}

fn helper_drop_trace(
    session: &SessionData,
    identity: RunIdentity,
    pre_start_inventory: PreStartInventoryObservation,
    setup_seed_actions: Vec<SetupSeedAction>,
    frames: Vec<PlayerFrame>,
    takes: Vec<TakeDispatch>,
    authority: TraceAuthority,
) -> Result<HelperDropTrace, String> {
    let case = session.runtime.case;
    let helper = helper_setup(&session.runtime)?;
    let actor = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Helper)
        .ok_or("missing helper actor")?;
    let run = PlayerRunTrace {
        kind: case.kind(),
        engine: session.engine.clone(),
        authority,
        identity,
        pre_start_inventory,
        setup_seed_actions,
        radius: RADIUS,
        selected_ids: case.selected_ids(),
        frames,
        takes,
        drops: actor.drop_actions.clone(),
    };
    Ok(HelperDropTrace { run, helper })
}

fn negative_drop_trace(
    session: &SessionData,
    identity: RunIdentity,
    pre_start_inventory: PreStartInventoryObservation,
    setup_seed_actions: Vec<SetupSeedAction>,
    frames: Vec<PlayerFrame>,
    takes: Vec<TakeDispatch>,
    authority: TraceAuthority,
) -> Result<NegativeDropTrace, String> {
    let helper = helper_setup(&session.runtime)?;
    let actor = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Helper)
        .ok_or("missing helper actor")?;
    let candidates = actor
        .drop_actions
        .iter()
        .map(|drop| {
            let kind = if drop.item_id == ORDINARY_UNSELECTED_ID {
                NegativeCandidateKind::OrdinaryUnselected
            } else if drop.item_id == EASTER_EGG_CERTIFICATE_ID {
                NegativeCandidateKind::Certificate
            } else if session.runtime.case.selected_ids().contains(&drop.item_id)
                && !drop.tile.within(ANCHOR, RADIUS)
            {
                NegativeCandidateKind::SelectedOutsideRadius
            } else {
                return Err(format!(
                    "helper Drop {} at {:?} is not one of the exact negative candidates",
                    drop.item_id, drop.tile
                ));
            };
            Ok(NegativeCandidate {
                kind,
                item_id: drop.item_id,
                tile: drop.tile,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(NegativeDropTrace {
        run: PlayerRunTrace {
            kind: CellKind::ANegativeCandidates,
            engine: session.engine.clone(),
            authority,
            identity,
            pre_start_inventory,
            setup_seed_actions,
            radius: RADIUS,
            selected_ids: session.runtime.case.selected_ids(),
            frames,
            takes,
            drops: actor.drop_actions.clone(),
        },
        helper,
        candidates,
    })
}

fn project_competitor_frames(
    report: &ActorReport,
    fresh_accounts: &BTreeSet<String>,
    mut first_cycle: u64,
    last_cycle: u64,
    active_before_grinch_start: bool,
) -> Result<(ParticipantIdentity, Vec<PlayerFrame>), String> {
    for take in &report.competitor_takes {
        let owner = report
            .frames
            .iter()
            .find(|frame| frame.observation_sequence == take.observation_sequence)
            .ok_or("competitor send-owning PLAYER_INFO frame is missing")?;
        if !take.has_exact_send_evidence(owner) {
            return Err("competitor send-owning PLAYER_INFO identity changed".into());
        }
        let cycle = owner
            .server_cycle
            .ok_or("competitor send-owning PLAYER_INFO frame lacks its actual cycle")?;
        if cycle > last_cycle {
            return Err(
                "competitor send-owning PLAYER_INFO cycle exceeds the main capture window".into(),
            );
        }
        first_cycle = first_cycle.min(cycle);
    }
    let frames = report
        .frames
        .iter()
        .filter(|frame| {
            frame
                .server_cycle
                .is_some_and(|cycle| first_cycle <= cycle && cycle <= last_cycle)
        })
        .map(|frame| frame.to_player_frame(None, None))
        .collect::<Vec<_>>();
    let first = frames
        .first()
        .ok_or("competitor lacks PLAYER_INFO frames in the main cycle window")?;
    let identity = ParticipantIdentity {
        actor: first.actor.clone(),
        session_id: first
            .session_id
            .clone()
            .ok_or("competitor's first retained frame lacks its real client session")?,
        start_tick: first.tick,
        fresh_account: fresh_accounts.contains(&report.account),
        active_before_grinch_start,
    };
    Ok((identity, frames))
}

fn build_static_trace(
    session: &SessionData,
    case: Case,
    cycles: &[ServerCycle],
    wire_takes: &[WireTake],
    restarts: &[RestartTransition],
) -> Result<StaticSpawnTrace, String> {
    let (identity, _, _, frames) = player_run_parts(case, session, wire_takes, restarts)?;
    let start_fence = session
        .runtime
        .start_observation_sequence
        .ok_or("missing Start observation fence")?;
    if !server_frames_complete(&session.runtime, &session.logs, start_fence) {
        return Err("scratch PLAYER_INFO frames do not have complete exact output-cycle and post-lifecycle evidence".into());
    }
    let first_cycle = frames
        .iter()
        .filter_map(|frame| frame.server_cycle)
        .min()
        .ok_or("main frames lack server-cycle identities")?;
    let last_cycle = frames
        .iter()
        .filter_map(|frame| frame.server_cycle)
        .max()
        .ok_or("main frames lack server-cycle identities")?;
    let competitor_report = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Competitor);
    let (competitor, competitor_frames) = if let Some(report) = competitor_report {
        let spawn_rows = &session
            .engine
            .scratch
            .content_patch
            .as_ref()
            .ok_or("static trace lacks its respawn content patch")?
            .static_spawns;
        let race_tiles = spawn_rows
            .iter()
            .filter(|spawn| spawn.item_id == PUMPKIN_ID && spawn.tile.within(ANCHOR, RADIUS))
            .map(|spawn| spawn.tile)
            .collect::<Vec<_>>();
        if case == Case::BContestedPumpkin && race_tiles.len() != 1 {
            return Err(format!(
                "contested scratch patch has {} in-radius pumpkin rows, expected one",
                race_tiles.len()
            ));
        }
        let race_tile = race_tiles
            .first()
            .copied()
            .ok_or("competitor run lacks an in-radius pumpkin static row")?;
        let active_before = report
            .frames
            .iter()
            .filter(|frame| frame.observation_sequence < start_fence)
            .any(|frame| {
                frame.position == race_tile
                    && frame.server_cycle.is_some_and(|cycle_id| {
                        cycles
                            .iter()
                            .find(|cycle| cycle.cycle == cycle_id)
                            .is_some_and(|cycle| {
                                cycle.active_actors.iter().any(|active| {
                                    active.actor == frame.actor
                                        && active.active
                                        && active.tile == race_tile
                                })
                            })
                    })
            });
        let (participant, participant_frames) = project_competitor_frames(
            report,
            &session.fresh_accounts,
            first_cycle,
            last_cycle,
            active_before,
        )?;
        (Some(participant), participant_frames)
    } else {
        (None, Vec::new())
    };
    let first_cycle = competitor_frames
        .first()
        .and_then(|frame| frame.server_cycle)
        .map_or(first_cycle, |cycle| cycle.min(first_cycle));
    let selected_cycles = cycles
        .iter()
        .filter(|cycle| first_cycle <= cycle.cycle && cycle.cycle <= last_cycle)
        .cloned()
        .collect::<Vec<_>>();
    if selected_cycles.len() as u64 != last_cycle.saturating_sub(first_cycle).saturating_add(1) {
        return Err("static capture range lacks a complete contiguous server-cycle ledger".into());
    }
    let main_report = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("missing main actor")?;
    let bank_actions_absent = session
        .logs
        .bank_clicks
        .iter()
        .all(|click| click.click_tick < identity.start_tick)
        && session
            .logs
            .bank_dispatches
            .iter()
            .all(|dispatch| dispatch.tick < identity.start_tick)
        && !session.logs.interacts.iter().any(|event| {
            event.slot_name == main_report.account
                && event.tick.is_some_and(|tick| tick >= identity.start_tick)
                && matches!(
                    &event.kind,
                    InteractionKind::OpenBank { .. } | InteractionKind::CloseBank
                )
        });
    let authority = trace_authority(
        &session.runtime,
        &frames,
        false,
        true,
        bank_actions_absent,
        false,
        true,
    );
    let refusals = wire_takes
        .iter()
        .filter_map(|take| {
            let event = take.outcome.as_ref()?;
            if event.outcome != "already_owned" {
                return None;
            }
            let kind = match event.item_id {
                SCYTHE_ID => RefusalKind::ScytheAlreadyOwned,
                BUNNY_EARS_ID => RefusalKind::BunnyEarsAlreadyOwned,
                _ => return None,
            };
            Some(RefusalReceipt {
                request_key: take.dispatch.request_key.clone(),
                item_id: event.item_id,
                tile: event.tile,
                tick: event.tick,
                kind,
            })
        })
        .collect::<Vec<_>>();
    if wire_takes.iter().any(|take| {
        take.outcome
            .as_ref()
            .is_some_and(|event| event.outcome == "already_owned")
            && !matches!(take.dispatch.item_id, SCYTHE_ID | BUNNY_EARS_ID)
    }) {
        return Err("scratch trace contains an untyped duplicate refusal".into());
    }
    Ok(StaticSpawnTrace {
        kind: case.kind(),
        engine: session.engine.clone(),
        authority,
        identity,
        radius: RADIUS,
        selected_ids: case.selected_ids(),
        frames,
        takes: wire_takes
            .iter()
            .map(|take| take.dispatch.clone())
            .collect(),
        cycles: selected_cycles,
        removals: session
            .logs
            .removals
            .iter()
            .map(|event| &event.receipt)
            .filter(|receipt| first_cycle <= receipt.cycle && receipt.cycle <= last_cycle)
            .cloned()
            .collect(),
        respawns: session
            .logs
            .respawns
            .iter()
            .map(|event| &event.receipt)
            .filter(|receipt| first_cycle <= receipt.cycle && receipt.cycle <= last_cycle)
            .cloned()
            .collect(),
        refusals,
        competitor,
        competitor_frames,
    })
}
fn accepted_packet_for_interaction<'a>(
    logs: &'a ObservedLogs,
    account: &str,
    key: WireRunKey,
    interaction: &InteractEvent,
) -> Result<&'a PacketEvent, String> {
    let tick = interaction
        .tick
        .ok_or("native bank interaction lacks an observed host tick")?;
    let matches = logs
        .packets
        .iter()
        .filter(|packet| {
            packet.account == account
                && packet.key == Some(key)
                && packet.tick == tick
                && packet.request == interaction.request
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "bank interaction at tick {tick} maps to {} exact native packet requests",
            matches.len()
        ));
    }
    let packet = matches[0];
    if packet.count == 0
        || usize::try_from(packet.count).ok() != Some(packet.opcodes.len())
        || !packet.decoded
        || !packet.accepted
        || packet.opcodes.is_empty()
        || packet.dispatch_stderr_line_ordinal.is_none()
    {
        return Err(format!(
            "bank interaction at tick {tick} lacks one accepted decoded wire request"
        ));
    }
    Ok(packet)
}

fn helper_actions_complete(case: Case, runtime: &RuntimeReport, identity: &RunIdentity) -> bool {
    let Some(helper) = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Helper)
    else {
        return !case.uses_helper();
    };
    let expected = case.helper_drops();
    if helper.drop_actions.len() != expected.len()
        || helper.drop_receipts.len() != expected.len()
        || helper.helper_post_drop.is_none()
    {
        return false;
    }
    let Some(start_fence) = runtime.start_observation_sequence else {
        return false;
    };
    let Some(main) = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
    else {
        return false;
    };
    expected
        .iter()
        .zip(helper.drop_actions.iter().zip(&helper.drop_receipts))
        .all(|(expected, (drop, receipt))| {
            drop.item_id == expected.item_id
                && drop.tile == expected.tile
                && drop.quantity == 1
                && drop.source == DropSource::RealPlayerInventoryDrop
                && drop.request_key == receipt.request_key
                && drop.tick == receipt.receiver_tick
                && drop.object_turn == receipt.receiver_object_turn
                && !receipt.packet_opcodes.is_empty()
                && receipt.receiver_sequence > start_fence
                && receipt.helper_observation_sequence > start_fence
                && receipt.helper_send_observation_sequence > start_fence
                && helper.frames.iter().any(|frame| {
                    frame.observation_sequence == receipt.helper_send_observation_sequence
                        && frame.player_info_sequence == receipt.helper_send_player_info_sequence
                        && frame.object_turn == Some(drop.object_turn)
                })
                && main.frames.iter().any(|frame| {
                    frame.observation_sequence == receipt.receiver_sequence
                        && frame.player_info_sequence == receipt.receiver_player_info_sequence
                        && frame.tick == drop.tick
                        && frame.object_turn == Some(drop.object_turn)
                })
                && receipt.receiver_confirmation_sequence > receipt.receiver_sequence
                && main.frames.iter().any(|frame| {
                    frame.observation_sequence == receipt.receiver_confirmation_sequence
                        && frame
                            .object_turn
                            .is_some_and(|turn| turn > drop.object_turn)
                })
        })
        && helper.helper_post_drop.as_ref().is_some_and(|frame| {
            frame.inventory.complete
                && frame.inventory.occupied_slots == Some(0)
                && frame.observation_sequence > start_fence
        })
        && identity.fresh_account
}

fn bank_packet_order(
    callbacks: &[BankCallbackObservation],
    click: &BankClickEvent,
    request_key: &str,
) -> Result<(BankPacketOrderProof, u64), String> {
    // The next callback's fence includes the synchronous native click record.
    // Thus the last earlier fence identifies the callback that supplied its UI.
    let deposit = callbacks
        .iter()
        .rev()
        .find(|row| row.host_log_ordinal_fence < click.host_log_ordinal)
        .ok_or("accepted deposit has no preceding captured callback")?;
    if deposit.tick != Some(click.click_tick)
        || deposit.bank_generation != click.bank_generation
        || deposit.bank_component < 0
        || !deposit.bank_loaded
    {
        return Err("accepted deposit callback lacks its exact loaded bank generation".into());
    }
    let table_at_deposit = deposit
        .inventory_packets
        .iter()
        .find(|packet| packet.component == deposit.bank_component)
        .ok_or("deposit callback lacks its bank table packet state")?;
    let modal = callbacks
        .iter()
        .find(|row| {
            row.session_generation == deposit.session_generation
                && row.modal_stamp != 0
                && row.modal_stamp == deposit.modal_stamp
        })
        .ok_or("missing bank interface-open packet receipt")?;
    let (loaded_index, first_loaded) = callbacks
        .iter()
        .enumerate()
        .find(|(_, row)| {
            row.session_generation == deposit.session_generation
                && row.bank_generation == deposit.bank_generation
                && row.bank_component == deposit.bank_component
                && row.bank_loaded
        })
        .ok_or("missing bank table packet receipt")?;
    let loaded_table = first_loaded
        .inventory_packets
        .iter()
        .find(|packet| packet.component == deposit.bank_component && packet.full_observation != 0)
        .ok_or("missing bank table packet receipt")?;
    let closed_generation = callbacks[..loaded_index]
        .iter()
        .rev()
        .find(|row| {
            row.session_generation == deposit.session_generation
                && row.bank_generation != deposit.bank_generation
        })
        .ok_or("bank table arrival lacks its prior closed-generation baseline")?
        .bank_generation;
    let baseline = callbacks[..loaded_index]
        .iter()
        .find(|row| {
            row.session_generation == deposit.session_generation
                && row.bank_generation == closed_generation
        })
        .filter(|row| !row.bank_loaded)
        .ok_or("bank table arrival lacks its prior closed-generation baseline")?;
    let baseline_full_generation = baseline
        .inventory_packets
        .iter()
        .find(|packet| packet.component == deposit.bank_component)
        .map_or(0, |packet| packet.full_generation);
    let (table_index, table, first_table) = callbacks
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.session_generation == deposit.session_generation
                && row.sequence >= baseline.sequence
                && row.sequence <= first_loaded.sequence
        })
        .find_map(|(index, row)| {
            row.inventory_packets
                .iter()
                .find(|packet| {
                    packet.component == deposit.bank_component
                        && packet.full_generation > baseline_full_generation
                })
                .map(|packet| (index, row, packet))
        })
        .ok_or("missing first bank table packet arrival")?;
    if loaded_table.full_observation != first_table.full_observation {
        return Err(
            "newer bank table packet arrived before its first-loaded generation binding".into(),
        );
    }
    let prior_full_generation = callbacks[..table_index]
        .iter()
        .rev()
        .filter(|row| row.session_generation == deposit.session_generation)
        .find_map(|row| {
            row.inventory_packets
                .iter()
                .find(|packet| packet.component == deposit.bank_component)
                .map(|packet| packet.full_generation)
        })
        .unwrap_or(0);
    if prior_full_generation.checked_add(1) != Some(first_table.full_generation) {
        return Err("first bank table packet was not individually captured".into());
    }
    let modal_tick = modal
        .tick
        .ok_or("bank interface-open packet has no PI edge")?;
    let table_tick = table.tick.ok_or("bank table packet has no PI edge")?;
    // Enumerate the real relevant stream: the first modal/table packets, then
    // the actual click within its owning callback. UI packets in one callback
    // use the client's shared packet-order stamps, never HashMap iteration.
    let mut arrivals = [
        (modal.sequence, 0, modal.modal_stamp, 0, modal_tick),
        (
            table.sequence,
            0,
            first_table.full_observation,
            1,
            table_tick,
        ),
        (deposit.sequence, 1, 0, 2, click.click_tick),
    ];
    arrivals.sort_by_key(|row| (row.0, row.1, row.2));
    let mut positions = BTreeMap::<u64, u64>::new();
    let mut ordinals = [None; 3];
    for (_, _, _, kind, tick) in arrivals {
        let position = positions.entry(tick).or_default();
        *position += 1;
        ordinals[kind] = Some(BankPacketOrdinal {
            tick,
            position: *position,
        });
    }
    let ambiguous_ui_batches = [modal, table]
        .into_iter()
        .filter(|row| row.player_info_changed)
        .map(|row| row.sequence)
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let ui_receipt =
        |row: &BankCallbackObservation, stamp, ordinal, bank_snapshot| BankUiPacketReceipt {
            ordinal: if row.player_info_changed {
                None
            } else {
                ordinal
            },
            session_generation: row.session_generation,
            arrival_bank_generation: row.bank_generation,
            callback_sequence: row.sequence,
            packet_observation: stamp,
            player_info_changed: row.player_info_changed,
            bank_snapshot,
        };
    Ok((
        BankPacketOrderProof {
            modal: Some(ui_receipt(modal, modal.modal_stamp, ordinals[0], None)),
            table: Some(ui_receipt(
                table,
                first_table.full_observation,
                ordinals[1],
                table.bank_snapshot.clone(),
            )),
            table_binding: Some(BankTableBinding {
                session_generation: first_loaded.session_generation,
                bound_to_generation: first_loaded.bank_generation,
                callback_sequence: first_loaded.sequence,
                packet_observation: first_table.full_observation,
                bank_snapshot: first_loaded.bank_snapshot.clone(),
            }),
            deposit: Some(BankDepositPacketReceipt {
                request_key: request_key.to_owned(),
                ordinal: ordinals[2],
                session_generation: deposit.session_generation,
                bank_generation: deposit.bank_generation,
                callback_sequence: deposit.sequence,
                observed_modal_stamp: deposit.modal_stamp,
                observed_table_stamp: table_at_deposit.full_observation,
                host_log_ordinal_fence: deposit.host_log_ordinal_fence,
                click_host_log_ordinal: click.host_log_ordinal,
            }),
            ambiguous_ui_batches,
        },
        modal_tick.max(table_tick),
    ))
}

fn build_bank_flow(
    session: &SessionData,
    identity: RunIdentity,
    pre_start_inventory: PreStartInventoryObservation,
    setup_seed_actions: Vec<SetupSeedAction>,
    frames: Vec<PlayerFrame>,
    takes: Vec<TakeDispatch>,
    helper: HelperSetup,
) -> Result<BankFlowTrace, String> {
    let case = session.runtime.case;
    let main = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("missing bank-flow receiver")?;
    let bank_before = main
        .bank_sample
        .clone()
        .ok_or("bank flow lacks a complete pre-Start bank snapshot")?;
    let key = session
        .runtime
        .run_key
        .ok_or("bank flow lacks its exact native RunKey")?;
    let open_events = session
        .logs
        .interacts
        .iter()
        .filter(|event| {
            event.slot_name == main.account
                && event.tick.is_some_and(|tick| tick >= identity.start_tick)
                && matches!(&event.kind, InteractionKind::OpenBank { .. })
        })
        .collect::<Vec<_>>();
    let close_events = session
        .logs
        .interacts
        .iter()
        .filter(|event| {
            event.slot_name == main.account
                && event.tick.is_some_and(|tick| tick >= identity.start_tick)
                && matches!(&event.kind, InteractionKind::CloseBank)
        })
        .collect::<Vec<_>>();
    if open_events.len() != 1 || close_events.len() != 1 {
        return Err(format!("bank flow observed {} OpenBank and {} CloseBank interactions, expected exactly one each", open_events.len(), close_events.len()));
    }
    let open_event = open_events[0];
    let close_event = close_events[0];
    let open_packet =
        accepted_packet_for_interaction(&session.logs, &main.account, key, open_event)?;
    let close_packet =
        accepted_packet_for_interaction(&session.logs, &main.account, key, close_event)?;
    let bank_before_frame = frames.first().ok_or("bank flow has no main frames")?;
    let starting_pumpkins = bank_before_frame
        .inventory
        .as_ref()
        .ok_or("bank-flow baseline lacks inventory")?
        .count(PUMPKIN_ID);
    let open_tick = open_event.tick.ok_or("OpenBank lacks host tick")?;
    let close_tick = close_event.tick.ok_or("CloseBank lacks host tick")?;
    let close_frame = frames
        .iter()
        .find(|frame| frame.tick >= close_tick && frame.bank_open == Some(false))
        .ok_or("CloseBank interaction lacks a later closed-interface PLAYER_INFO receipt")?;
    let returned = frames
        .iter()
        .find(|frame| {
            frame.tick > close_frame.tick
                && frame.position == ANCHOR
                && frame.bank_open == Some(false)
        })
        .ok_or("bank flow lacks a later exact-anchor PLAYER_INFO return receipt")?;
    let clicks = session
        .logs
        .bank_clicks
        .iter()
        .filter(|click| click.key == key)
        .collect::<Vec<_>>();
    let dispatches = session
        .logs
        .bank_dispatches
        .iter()
        .filter(|dispatch| dispatch.key == key)
        .collect::<Vec<_>>();
    if clicks.len() != 1 || dispatches.len() != 1 {
        return Err(format!(
            "bank flow observed {} deposit clicks and {} dispatches, expected exactly one each",
            clicks.len(),
            dispatches.len()
        ));
    }
    let click = clicks[0];
    let dispatch = dispatches[0];
    if click.request != dispatch.request
        || click.bank_generation != dispatch.bank_generation
        || click.transfer != dispatch.transfer
        || click.item_id != PUMPKIN_ID
        || dispatch.item_id != PUMPKIN_ID
        || click.option != "Deposit All"
        || click.operation != 4
        || !dispatch.accepted
        || dispatch.emitted_packets == 0
        || dispatch.opcode.is_none()
        || dispatch.tick < click.click_tick
        || dispatch.host_log_ordinal <= click.host_log_ordinal
    {
        return Err(
            "bank deposit click and accepted dispatch are not one exact Deposit All transaction"
                .into(),
        );
    }
    let deposit_frame = frames
        .iter()
        .find(|frame| frame.tick == click.click_tick)
        .ok_or("bank deposit click tick lacks its exact full-inventory PLAYER_INFO frame")?;
    let inventory_before = deposit_frame
        .inventory
        .as_ref()
        .ok_or("deposit click frame lacks inventory")?;
    if !inventory_before.complete || inventory_before.count(PUMPKIN_ID) != click.main_before {
        return Err("bank click main-before count contradicts its exact inventory snapshot".into());
    }
    let bank_after = frames
        .iter()
        .filter(|frame| {
            frame.tick >= dispatch.tick
                && frame.tick <= close_frame.tick
                && frame.bank_open == Some(true)
                && frame.bank_loaded == Some(true)
        })
        .filter_map(|frame| frame.bank.as_ref())
        .next_back()
        .cloned()
        .ok_or("accepted deposit lacks a complete post-dispatch bank snapshot")?;
    let chosen = session
        .bank_candidates
        .first()
        .ok_or("selected nav pack lacks the computed nearest ordinary bank route")?;
    let open_details = match &open_event.kind {
        InteractionKind::OpenBank { tile, name, id } => (*tile, name.as_deref(), *id),
        _ => return Err("OpenBank event changed during bank-flow construction".into()),
    };
    let exact_route = bank_open_matches_candidate(chosen, open_details.0);
    let observed_bank_id = if exact_route {
        chosen.name.clone()
    } else {
        open_details
            .1
            .map(str::to_owned)
            .or_else(|| open_details.2.map(|id| format!("object:{id}")))
            .or_else(|| {
                open_details
                    .0
                    .map(|tile| format!("tile:{}:{}:{}", tile.plane, tile.x, tile.z))
            })
            .unwrap_or_else(|| "unidentified-open-bank".into())
    };
    let route = BankRouteEvidence {
        bank_id: observed_bank_id.clone(),
        nearest_routable: exact_route,
        ordinary_accessible_bank: exact_route && chosen.ordinary,
        exact_route_proven: exact_route,
        used_air_distance_fallback: false,
        walking_steps: chosen.walking_steps,
    };
    let open_request = format!(
        "bank-open:{}:{}:{}",
        main.account, open_packet.tick, open_packet.request
    );
    let deposit_request = format!(
        "bank-deposit:{}:{}:{}:{}",
        identity.run_id, click.transfer, click.request, click.item_id
    );
    let close_request = format!(
        "bank-close:{}:{}:{}",
        main.account, close_packet.tick, close_packet.request
    );
    let return_request = format!(
        "anchor-return:{}:{}:{}",
        identity.run_id, returned.actor.account_id, returned.observation_sequence
    );
    let (packet_order, open_receipt_tick) =
        bank_packet_order(&session.runtime.bank_callbacks, click, &deposit_request)?;
    if open_receipt_tick < open_tick || open_receipt_tick >= close_frame.tick {
        return Err(
            "bank packet open receipt is outside the accepted open/close transaction".into(),
        );
    }
    let bank_actions = vec![
        BankAction {
            request_key: open_request.clone(),
            tick: open_receipt_tick,
            kind: BankActionKind::Open,
        },
        BankAction {
            request_key: deposit_request,
            tick: click.click_tick,
            kind: BankActionKind::Deposit {
                item_id: click.item_id,
                quantity: click.main_before,
            },
        },
        BankAction {
            request_key: close_request.clone(),
            tick: close_frame.tick,
            kind: BankActionKind::Close,
        },
    ];
    let receipts = BankFlowReceipts {
        open: Some(BankOpenReceipt {
            request_key: open_request,
            tick: open_receipt_tick,
            bank_id: observed_bank_id.clone(),
            interface_open: true,
            bank_loaded: true,
        }),
        close: Some(BankCloseReceipt {
            request_key: close_request,
            tick: close_frame.tick,
            bank_id: observed_bank_id,
            interface_closed: close_frame.bank_open == Some(false),
        }),
        return_to_anchor: Some(AnchorReturnReceipt {
            request_key: return_request,
            tick: returned.tick,
            tile: returned.position,
            arrival_receipt_observed: returned.position == identity.anchor,
        }),
    };
    let carried_item_id = if case == Case::CDeposit27PreserveItem {
        let carried = bank_before_frame
            .inventory
            .as_ref()
            .ok_or("C preserve-item baseline lacks inventory")?
            .quantities
            .iter()
            .filter(|(id, count)| **id != PUMPKIN_ID && **count > 0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        if carried.len() != 1 {
            return Err(format!(
                "C preserve-item baseline contains {} nonselected item IDs, expected one",
                carried.len()
            ));
        }
        Some(carried[0])
    } else {
        None
    };
    let post_start_click_count = session
        .logs
        .bank_clicks
        .iter()
        .filter(|click| click.click_tick >= identity.start_tick)
        .count();
    let post_start_dispatch_count = session
        .logs
        .bank_dispatches
        .iter()
        .filter(|event| event.tick >= identity.start_tick)
        .count();
    let bank_log_complete = clicks.len() == 1
        && dispatches.len() == 1
        && post_start_click_count == clicks.len()
        && post_start_dispatch_count == dispatches.len()
        && open_events.len() == 1
        && close_events.len() == 1
        && bank_actions.len() == 3
        && bank_before.complete
        && bank_after.complete;
    let helper_complete = helper_actions_complete(case, &session.runtime, &identity);
    let authority = trace_authority(
        &session.runtime,
        &frames,
        helper_complete,
        false,
        bank_log_complete,
        bank_before.complete && bank_after.complete,
        true,
    );
    Ok(BankFlowTrace {
        kind: case.kind(),
        engine: session.engine.clone(),
        authority,
        identity,
        pre_start_inventory,
        setup_seed_actions,
        radius: RADIUS,
        selected_ids: case.selected_ids(),
        frames,
        takes,
        drops: session
            .runtime
            .actors
            .iter()
            .find(|actor| actor.role == ActorRole::Helper)
            .ok_or("bank flow lacks helper actor")?
            .drop_actions
            .clone(),
        helper,
        starting_pumpkins,
        carried_item_id,
        route,
        bank_before: Some(bank_before),
        bank_after: Some(bank_after),
        bank_actions,
        receipts,
        packet_order,
    })
}
fn post_start_bank_action_observed(
    session: &SessionData,
    identity: &RunIdentity,
    account: &str,
) -> bool {
    session
        .logs
        .bank_clicks
        .iter()
        .any(|click| click.click_tick >= identity.start_tick)
        || session
            .logs
            .bank_dispatches
            .iter()
            .any(|dispatch| dispatch.tick >= identity.start_tick)
        || session.logs.interacts.iter().any(|event| {
            event.slot_name == account
                && event.tick.is_some_and(|tick| tick >= identity.start_tick)
                && matches!(
                    &event.kind,
                    InteractionKind::OpenBank { .. } | InteractionKind::CloseBank
                )
        })
}

fn oracle_receipt(case: Case, name: &str, result: Result<(), String>, trace_path: &Path) -> Value {
    match result {
        Ok(()) => json!({
            "schema":CAPTURE_SCHEMA,"case":case.slug(),"oracle":name,"passed":true,
            "trace_file":trace_path,"error":Value::Null,
        }),
        Err(error) => json!({
            "schema":CAPTURE_SCHEMA,"case":case.slug(),"oracle":name,"passed":false,
            "trace_file":trace_path,"error":error,
        }),
    }
}

fn terminal_sample_has_key(
    runtime: &RuntimeReport,
    sample: &RuntimeObservation,
    key: WireRunKey,
) -> bool {
    let Some(terminal) = runtime.terminal_stop.as_ref() else {
        return false;
    };
    runtime.case == Case::CInventoryBlocked
        && terminal.run_slot == key.slot
        && terminal.run_number == key.run
        && terminal.native_session == key.session
        && terminal.card == "Grinch"
        && terminal.native_phase == "Blocked"
        && terminal.failure_code == "inventory-blocked"
        && terminal.active_run_key_after_stop.is_none()
        && sample.start_requested
        && runtime
            .start_observation_sequence
            .is_some_and(|fence| sample.observation_sequence > fence)
        && sample.generation == Some(terminal.generation_after_stop)
        && sample.run_key.is_none()
}

fn runtime_sample_has_key(
    runtime: &RuntimeReport,
    sample: &RuntimeObservation,
    key: WireRunKey,
) -> bool {
    sample.run_key == Some(key) || terminal_sample_has_key(runtime, sample, key)
}

fn runtime_observation_ledger_is_complete(runtime: &RuntimeReport) -> bool {
    let Some(main) = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
    else {
        return false;
    };
    let valid_link = |sample: &RuntimeObservation| {
        sample.account == main.account
            && sample.tick.is_some()
            && sample.client_session_id.is_some()
            && sample.client_session_generation.is_some()
            && sample.player_info_sequence.is_some()
    };
    let Some(start_fence) = runtime.start_observation_sequence else {
        return false;
    };
    let before_start = runtime
        .runtime_observations
        .iter()
        .any(|sample| valid_link(sample) && !sample.start_requested && sample.generation.is_some());
    let after_start = runtime.runtime_observations.iter().any(|sample| {
        valid_link(sample)
            && sample.start_requested
            && sample.observation_sequence >= start_fence
            && sample.generation.is_some()
            && runtime
                .run_key
                .is_some_and(|key| runtime_sample_has_key(runtime, sample, key))
    });
    let terminal_consistent = runtime.terminal_stop.as_ref().is_none_or(|terminal| {
        runtime.case == Case::CInventoryBlocked
            && terminal.generation_before_start.checked_add(1)
                == Some(terminal.generation_after_stop)
            && runtime.runtime_observations.iter().any(|sample| {
                valid_link(sample)
                    && !sample.start_requested
                    && sample.observation_sequence == start_fence
                    && sample.generation == Some(terminal.generation_before_start)
                    && sample.run_key.is_none()
            })
            && runtime.run_key.is_some_and(|key| {
                runtime
                    .runtime_observations
                    .iter()
                    .filter(|sample| sample.start_requested)
                    .all(|sample| terminal_sample_has_key(runtime, sample, key))
            })
    });
    runtime
        .runtime_observations
        .first()
        .is_some_and(|sample| sample.sample_ordinal == 1)
        && runtime
            .runtime_observations
            .iter()
            .all(|sample| sample.account == main.account)
        && runtime.runtime_observations.windows(2).all(|pair| {
            pair[0].sample_ordinal.checked_add(1) == Some(pair[1].sample_ordinal)
                && pair[0].observation_sequence <= pair[1].observation_sequence
                && pair[0].host_log_ordinal_fence <= pair[1].host_log_ordinal_fence
                && (pair[0].client_session_id != pair[1].client_session_id
                    || match pair[0].tick.zip(pair[1].tick) {
                        Some((before, after)) => before <= after,
                        None => true,
                    })
        })
        && before_start
        && after_start
        && terminal_consistent
}

fn validate_watchdog_ledger(session: &SessionData) -> Result<(), String> {
    if session
        .host_rows
        .first()
        .is_some_and(|row| row.ordinal != 1)
        || !session
            .host_rows
            .windows(2)
            .all(|pair| pair[0].ordinal.checked_add(1) == Some(pair[1].ordinal))
    {
        return Err("raw host log capture ordinals are incomplete or noncontiguous".into());
    }
    let raw_actions = session
        .host_rows
        .iter()
        .filter(|row| row.message.starts_with("watchdog-action "))
        .collect::<Vec<_>>();
    let parsed_actions = session
        .logs
        .watchdog
        .iter()
        .filter(|event| event.action != "legacy_script_restart")
        .collect::<Vec<_>>();
    let raw_completions = session
        .host_rows
        .iter()
        .filter(|row| row.message.contains("restarted the script"))
        .collect::<Vec<_>>();
    let parsed_completions = session
        .logs
        .watchdog
        .iter()
        .filter(|event| event.action == "legacy_script_restart")
        .collect::<Vec<_>>();
    if raw_actions.len() != parsed_actions.len()
        || raw_completions.len() != parsed_completions.len()
    {
        return Err(format!(
            "raw watchdog ledger is incomplete: {} action rows/{} parsed, {} restart completions/{} parsed",
            raw_actions.len(), parsed_actions.len(), raw_completions.len(), parsed_completions.len(),
        ));
    }
    for event in &session.logs.watchdog {
        let account = event
            .slot_name
            .as_deref()
            .filter(|account| !account.is_empty())
            .ok_or("watchdog event lacks exact actor account identity")?;
        event
            .key
            .ok_or("watchdog event lacks exact native RunKey")?;
        event.tick.ok_or("watchdog event lacks exact native tick")?;
        let row = session
            .host_rows
            .iter()
            .find(|row| row.ordinal == event.host_log_ordinal)
            .ok_or("watchdog event capture ordinal is absent from raw host ledger")?;
        if row.message != event.raw_record || row.slot.as_deref() != Some(account) {
            return Err(
                "watchdog event does not match its exact raw host record, actor, or native RunKey"
                    .into(),
            );
        }
    }
    Ok(())
}

fn build_case_trace(
    case: Case,
    session: &mut SessionData,
    evidence_dir: &Path,
) -> Result<(Value, Value, Value), String> {
    if !session.logs.malformed.is_empty() {
        return Err(format!(
            "observed live evidence ledger is malformed: {}",
            session.logs.malformed[0]
        ));
    }
    validate_watchdog_ledger(session)?;
    let cycles = server_cycles(&session.logs)?;
    let required_window = required_server_cycle_window(
        session
            .runtime
            .actors
            .iter()
            .flat_map(|actor| actor.frames.iter()),
        &session.logs,
    )?
    .ok_or("captured PLAYER_INFO frames lack their complete server output window")?;
    validate_server_cycle_ledger(&session.logs, &cycles, required_window)?;
    let restarts = build_restart_transitions(&session.runtime, &session.logs)?;
    let wire_takes = build_wire_takes(&session.runtime, &session.logs, case, &cycles)?;
    let (identity, pre_start_inventory, seed_actions, frames) =
        player_run_parts(case, session, &wire_takes, &restarts)?;
    let main = session
        .runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main)
        .ok_or("missing Grinch main actor")?;
    let helper_complete = helper_actions_complete(case, &session.runtime, &identity);
    let takes = wire_takes
        .iter()
        .map(|take| take.dispatch.clone())
        .collect::<Vec<_>>();
    let (trace, oracle_name) = match case {
        Case::ATradeableDrops
        | Case::AGreenMaskSelection
        | Case::ANegativeCandidates
        | Case::APrivateUntradeableDrops => {
            let bank_clear = !post_start_bank_action_observed(session, &identity, &main.account);
            let authority = trace_authority(
                &session.runtime,
                &frames,
                helper_complete,
                false,
                bank_clear,
                false,
                true,
            );
            match case {
                Case::ATradeableDrops => {
                    let trace = helper_drop_trace(
                        session,
                        identity.clone(),
                        pre_start_inventory.clone(),
                        seed_actions.clone(),
                        frames.clone(),
                        takes,
                        authority,
                    )?;
                    (
                        serde_json::to_value(trace)
                            .map_err(|error| format!("serialize A tradeable trace: {error}"))?,
                        "check_a_tradeable",
                    )
                }
                Case::AGreenMaskSelection => {
                    let trace = helper_drop_trace(
                        session,
                        identity.clone(),
                        pre_start_inventory.clone(),
                        seed_actions.clone(),
                        frames.clone(),
                        takes,
                        authority,
                    )?;
                    (
                        serde_json::to_value(trace)
                            .map_err(|error| format!("serialize A green-mask trace: {error}"))?,
                        "check_a_green_mask",
                    )
                }
                Case::ANegativeCandidates => {
                    let trace = negative_drop_trace(
                        session,
                        identity.clone(),
                        pre_start_inventory.clone(),
                        seed_actions.clone(),
                        frames.clone(),
                        takes,
                        authority,
                    )?;
                    (
                        serde_json::to_value(trace).map_err(|error| {
                            format!("serialize A negative-candidate trace: {error}")
                        })?,
                        "check_a_negative_candidates",
                    )
                }
                Case::APrivateUntradeableDrops => {
                    let trace = helper_drop_trace(
                        session,
                        identity.clone(),
                        pre_start_inventory.clone(),
                        seed_actions.clone(),
                        frames.clone(),
                        takes,
                        authority,
                    )?;
                    (
                        serde_json::to_value(trace)
                            .map_err(|error| format!("serialize A private-drop trace: {error}"))?,
                        "check_a_private_untradeable",
                    )
                }
                _ => unreachable!(),
            }
        }
        Case::BPumpkinRespawns
        | Case::BContestedPumpkin
        | Case::BScytheDuplicate
        | Case::BBunnyEarsDuplicate
        | Case::BYoYoRepeat => {
            let trace = build_static_trace(session, case, &cycles, &wire_takes, &restarts)?;
            (
                serde_json::to_value(trace)
                    .map_err(|error| format!("serialize B static-spawn trace: {error}"))?,
                "deferred_until_scratch_shutdown",
            )
        }
        Case::CDeposit28 | Case::CDeposit27PreserveItem => {
            let helper = helper_setup(&session.runtime)?;
            let trace = build_bank_flow(
                session,
                identity.clone(),
                pre_start_inventory.clone(),
                seed_actions.clone(),
                frames.clone(),
                takes,
                helper,
            )?;
            let name = if case == Case::CDeposit28 {
                "check_c_deposit_28"
            } else {
                "check_c_deposit_27_preserve_item"
            };
            (
                serde_json::to_value(trace)
                    .map_err(|error| format!("serialize C bank-flow trace: {error}"))?,
                name,
            )
        }
        Case::CInventoryBlocked => {
            if post_start_bank_action_observed(session, &identity, &main.account) {
                return Err(
                    "inventory-blocked Grinch issued an observed post-Start bank action".into(),
                );
            }
            let bank_snapshot_complete =
                main.bank_sample.as_ref().is_some_and(|bank| bank.complete);
            let authority = trace_authority(
                &session.runtime,
                &frames,
                false,
                false,
                true,
                bank_snapshot_complete,
                wire_takes.is_empty(),
            );
            let trace =
                InventoryBlockedTrace {
                    kind: case.kind(),
                    engine: session.engine.clone(),
                    authority,
                    identity: identity.clone(),
                    terminal_stop: session.runtime.terminal_stop.clone().ok_or(
                        "inventory-blocked trace lacks its retained native terminal status",
                    )?,
                    pre_start_inventory,
                    setup_seed_actions: seed_actions,
                    selected_ids: case.selected_ids(),
                    frames: frames.clone(),
                    bank_actions: Vec::new(),
                };
            (
                serde_json::to_value(trace)
                    .map_err(|error| format!("serialize C inventory-blocked trace: {error}"))?,
                "check_c_inventory_blocked",
            )
        }
    };
    let trace_path = evidence_path(evidence_dir, case, "trace.json");
    let trace_value = trace;
    let meta = session_meta_json(
        session,
        Some(&identity),
        &frames,
        &restarts,
        &wire_takes,
        true,
        None,
    );
    let receipt = json!({
        "schema":CAPTURE_SCHEMA,"case":case.slug(),"oracle":oracle_name,"passed":Value::Null,
        "deferred":true,"trace_file":trace_path,"error":Value::Null,
    });
    Ok((trace_value, meta, receipt))
}
fn runtime_observation_json(sample: &RuntimeObservation) -> Value {
    json!({
        "sample_ordinal":sample.sample_ordinal,
        "capture_ordinal":sample.sample_ordinal,
        "host_log_ordinal_fence":sample.host_log_ordinal_fence,
        "observation_sequence":sample.observation_sequence,
        "account":sample.account,
        "client_session_id":sample.client_session_id,
        "client_session_generation":sample.client_session_generation,
        "player_info_sequence":sample.player_info_sequence,
        "tick":sample.tick,
        "start_requested":sample.start_requested,
        "runtime_generation":sample.generation,
        "native_run_key":run_key_value(sample.run_key),
    })
}
fn raw_frame_with_runtime_json(frame: &RawFrame, runtime: &RuntimeReport) -> Value {
    let client_session_id = frame.session_id();
    let linked = runtime
        .runtime_observations
        .iter()
        .filter(|sample| {
            sample.account == frame.account
                && sample.observation_sequence == frame.observation_sequence
                && sample.player_info_sequence == Some(frame.player_info_sequence)
                && sample.tick == Some(frame.tick)
                && sample.client_session_id.as_deref() == Some(client_session_id.as_str())
        })
        .map(runtime_observation_json)
        .collect::<Vec<_>>();
    let mut value = raw_frame_json(frame);
    value["linked_runtime_observations"] = json!(linked);
    value
}

fn native_status_lineage_json(
    frame: &PlayerFrame,
    session: &SessionData,
    takes: &[WireTake],
    restarts: &[RestartTransition],
) -> Value {
    let account = &frame.actor.account_id;
    let linked = session
        .runtime
        .runtime_observations
        .iter()
        .filter(|sample| {
            sample.account == *account
                && sample.observation_sequence == frame.observation_sequence
                && sample.player_info_sequence == frame.player_info_sequence
                && sample.tick == Some(frame.tick)
                && sample.client_session_id.as_deref() == frame.session_id.as_deref()
        })
        .collect::<Vec<_>>();
    let capture = linked.last().copied();
    let key = capture.and_then(|sample| {
        sample.run_key.or_else(|| {
            session
                .runtime
                .run_key
                .filter(|key| terminal_sample_has_key(&session.runtime, sample, *key))
        })
    });
    let host_log_ordinal_fence = capture.map(|sample| sample.host_log_ordinal_fence);
    let status_state = key.zip(capture).and_then(|(run_key, sample)| {
        state_for_frame(
            &session.logs,
            account,
            frame.tick,
            run_key,
            sample.host_log_ordinal_fence,
        )
    });
    let through_tick = frame.tick;
    let take_sources = takes
        .iter()
        .filter(|take| {
            take.key == key
                && take.dispatch.actor.account_id == *account
                && take.dispatch.tick < frame.tick
        })
        .map(|take| {
            let packet = session.logs.packets.iter().find(|packet| {
                packet.account == *account
                    && packet.tick == take.dispatch.tick
                    && packet.key == key
                    && one_obj3_packet(&packet.opcodes)
            });
            json!({
                "request_key":take.dispatch.request_key,"tick":take.dispatch.tick,
                "item_id":take.dispatch.item_id,"tile":take.dispatch.tile,
                "wire_run_key":run_key_value(key),
                "packet_stderr_line_ordinals":packet.map(|event| &event.packet_stderr_line_ordinals),
                "dispatch_stderr_line_ordinal":packet.and_then(|event| event.dispatch_stderr_line_ordinal),
                "accepted":packet.map(|event| event.accepted),
            })
        })
        .collect::<Vec<_>>();
    let take_outcomes = session.logs.take_events.iter().filter(|event| {
        key == Some(event.key) && event.slot_name == *account && event.tick <= through_tick
            && host_log_ordinal_fence.is_some_and(|fence| event.host_log_ordinal <= fence)
    }).map(|event| json!({
        "capture_ordinal":event.host_log_ordinal,"host_log_ordinal":event.host_log_ordinal,"tick":event.tick,
        "run_key":run_key_value(Some(event.key)),
        "item_id":event.item_id,"tile":event.tile,"outcome":event.outcome,"units":event.units,
    })).collect::<Vec<_>>();
    let state_history = session.logs.states.iter().filter(|event| {
        key == Some(event.key) && event.slot_name == *account && event.tick <= through_tick
            && host_log_ordinal_fence.is_some_and(|fence| event.host_log_ordinal <= fence)
    }).map(|event| json!({
        "capture_ordinal":event.host_log_ordinal,"host_log_ordinal":event.host_log_ordinal,
        "run_key":run_key_value(Some(event.key)),"tick":event.tick,"raw_status":event.payload,
    })).collect::<Vec<_>>();
    let recovery_actions = session
        .logs
        .watchdog
        .iter()
        .filter(|event| {
            event.action.starts_with("ArmWalk {")
                && event.key == key
                && event.slot_name.as_deref() == Some(account.as_str())
                && event.tick.is_some_and(|tick| tick <= through_tick)
                && host_log_ordinal_fence.is_some_and(|fence| event.host_log_ordinal <= fence)
        })
        .map(|event| {
            json!({
                "capture_ordinal":event.host_log_ordinal,"host_log_ordinal":event.host_log_ordinal,
                "run_key":run_key_value(event.key),"tick":event.tick,"raw_record":event.raw_record,
            })
        })
        .collect::<Vec<_>>();
    let restart_sources = restarts
        .iter()
        .filter(|restart| {
            restart.account == *account
                && restart.after.run_key == key
                && capture
                    .is_some_and(|sample| restart.after.sample_ordinal <= sample.sample_ordinal)
        })
        .map(|restart| {
            json!({
                "action_capture_ordinal":restart.host_log_ordinal,
                "action_host_log_ordinal":restart.host_log_ordinal,
                "completion_capture_ordinal":restart.completion_host_log_ordinal,
                "completion_host_log_ordinal":restart.completion_host_log_ordinal,
                "action_tick":restart.action_tick,
                "before_sample_ordinal":restart.before.sample_ordinal,
                "after_sample_ordinal":restart.after.sample_ordinal,
                "before_run_key":run_key_value(restart.before.run_key),
                "after_run_key":run_key_value(restart.after.run_key),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "actor":frame.actor,"client_session_id":frame.session_id,
        "player_info_sequence":frame.player_info_sequence,"observation_sequence":frame.observation_sequence,
        "tick":frame.tick,"capture_sample_ordinals":linked.iter().map(|sample| sample.sample_ordinal).collect::<Vec<_>>(),
        "host_log_ordinal_fence":host_log_ordinal_fence,
        "sampled_runtime_generation":capture.and_then(|sample| sample.generation),
        "sampled_native_run_key":run_key_value(capture.and_then(|sample| sample.run_key)),
        "retained_terminal_native_run_key":run_key_value(capture.and_then(|sample| {
            session.runtime.run_key.filter(|key| terminal_sample_has_key(&session.runtime, sample, *key))
        })),
        "native_status":frame.status,
        "status_state_host_log_ordinal":status_state.map(|event| event.host_log_ordinal),
        "status_state_run_key":status_state.map(|event| run_key_value(Some(event.key))),
        "counter_source_ledger":{
            "accepted_take_dispatches_before_frame":take_sources,
            "take_outcomes_through_frame_tick":take_outcomes,
            "state_history_through_frame_tick":state_history,
            "recovery_actions_through_frame_tick":recovery_actions,
            "restart_transitions_into_sampled_run":restart_sources,
        },
    })
}

fn session_meta_json(
    session: &SessionData,
    identity: Option<&RunIdentity>,
    frames: &[PlayerFrame],
    restarts: &[RestartTransition],
    takes: &[WireTake],
    oracle_deferred: bool,
    failure: Option<&str>,
) -> Value {
    let case = session.runtime.case;
    let runtime = &session.runtime;
    let main = runtime
        .actors
        .iter()
        .find(|actor| actor.role == ActorRole::Main);
    let main_account = main.map(|actor| actor.account.as_str());
    let before_start = runtime.runtime_observations.iter().rev().find(|sample| {
        Some(sample.account.as_str()) == main_account
            && !sample.start_requested
            && sample.generation.is_some()
    });
    let after_start = runtime.runtime_observations.iter().find(|sample| {
        Some(sample.account.as_str()) == main_account
            && sample.start_requested
            && sample.generation.is_some()
    });
    let runtime_transitions = runtime.runtime_observations.windows(2).filter_map(|pair| {
        let before = &pair[0];
        let after = &pair[1];
        (before.generation != after.generation || before.run_key != after.run_key)
            .then(|| json!({"before":runtime_observation_json(before),"after":runtime_observation_json(after)}))
    }).collect::<Vec<_>>();
    let runtime_observation_ledger_complete = runtime_observation_ledger_is_complete(runtime);
    let raw_watchdog_actions = session
        .host_rows
        .iter()
        .filter(|row| row.message.starts_with("watchdog-action "))
        .count();
    let parsed_watchdog_actions = session
        .logs
        .watchdog
        .iter()
        .filter(|event| event.action != "legacy_script_restart")
        .count();
    let raw_restart_completions = session
        .host_rows
        .iter()
        .filter(|row| row.message.contains("restarted the script"))
        .count();
    let parsed_restart_completions = session
        .logs
        .watchdog
        .iter()
        .filter(|event| event.action == "legacy_script_restart")
        .count();
    let restart_action_count = session
        .logs
        .watchdog
        .iter()
        .filter(|event| event.action.starts_with("Restart {"))
        .count();
    let restart_transition_ledger_complete = restart_action_count == parsed_restart_completions
        && restart_action_count == restarts.len();
    let watchdog_action_ledger_complete = validate_watchdog_ledger(session).is_ok();
    let status_rows = frames
        .iter()
        .map(|frame| native_status_lineage_json(frame, session, takes, restarts))
        .collect::<Vec<_>>();
    let raw_actors = runtime.actors.iter().map(|actor| json!({
        "role":format!("{:?}",actor.role),
        "account":actor.account,
        "frames":actor.frames.iter().map(|frame| raw_frame_with_runtime_json(frame,runtime)).collect::<Vec<_>>(),
        "seed_observation":actor.seed_observation.as_ref().map(|frame| raw_frame_with_runtime_json(frame,runtime)),
        "seed_commands":actor.seed_commands,
        "bank_sample":actor.bank_sample,
        "drop_actions":actor.drop_actions,
        "drop_receipts":actor.drop_receipts.iter().map(|receipt| json!({
            "request_key":receipt.request_key,
            "send_tick":receipt.send_tick,
            "helper_send_player_info_sequence":receipt.helper_send_player_info_sequence,
            "helper_send_observation_sequence":receipt.helper_send_observation_sequence,
            "receiver_confirmation_sequence":receipt.receiver_confirmation_sequence,
            "receiver_tick":receipt.receiver_tick,
            "receiver_object_turn":receipt.receiver_object_turn,
            "receiver_sequence":receipt.receiver_sequence,
            "receiver_player_info_sequence":receipt.receiver_player_info_sequence,
            "helper_confirmation_tick":receipt.helper_confirmation_tick,
            "helper_player_info_sequence":receipt.helper_player_info_sequence,
            "helper_observation_sequence":receipt.helper_observation_sequence,
            "packet_opcodes":receipt.packet_opcodes,
        })).collect::<Vec<_>>(),
        "helper_post_drop":actor.helper_post_drop.as_ref().map(|frame| raw_frame_with_runtime_json(frame,runtime)),
        "competitor_takes":actor.competitor_takes.iter().map(|take| json!({
            "item_id":take.item_id,"tile":take.tile,"request_key":take.request_key,
            "tick":take.tick,"observation_sequence":take.observation_sequence,"packet_opcodes":take.packet_opcodes,
            "player_info_sequence":take.player_info_sequence,
            "scene_generation_at_send":take.scene_generation,
            "observed_ground_at_send":take.observed_ground,
        })).collect::<Vec<_>>(),
        "error":actor.error,
    })).collect::<Vec<_>>();
    let states = session
        .logs
        .states
        .iter()
        .map(|event| {
            json!({
                "host_log_ordinal":event.host_log_ordinal,
                "account":event.slot_name,
                "run_key":run_key_value(Some(event.key)),
                "tick":event.tick,
                "raw_status":event.payload,
            })
        })
        .collect::<Vec<_>>();
    let take_events = session
        .logs
        .take_events
        .iter()
        .map(|event| {
            json!({
                "host_log_ordinal":event.host_log_ordinal,
                "account":event.slot_name,
                "run_key":run_key_value(Some(event.key)),
                "tick":event.tick,"item_id":event.item_id,"tile":event.tile,
                "outcome":event.outcome,"units":event.units,
            })
        })
        .collect::<Vec<_>>();
    let packets = session.logs.packets.iter().map(|packet| json!({
        "slot":packet.slot_name,"account":packet.account,"run_key":run_key_value(packet.key),
        "tick":packet.tick,"request":packet.request,"count":packet.count,
        "decoded":packet.decoded,"accepted":packet.accepted,"opcodes":packet.opcodes,
        "packet_stderr_line_ordinals":packet.packet_stderr_line_ordinals,
        "dispatch_stderr_line_ordinal":packet.dispatch_stderr_line_ordinal,
    })).collect::<Vec<_>>();
    let interactions = session.logs.interacts.iter().map(|event| json!({
        "stderr_line_ordinal":event.stderr_line_ordinal,"request":event.request,"account":event.slot_name,"tick":event.tick,
        "raw_request":event.text,
    })).collect::<Vec<_>>();
    let bank_clicks = session.logs.bank_clicks.iter().map(|event| json!({
        "host_log_ordinal":event.host_log_ordinal,"run_key":run_key_value(Some(event.key)),
        "bank_generation":event.bank_generation,"transfer":event.transfer,"request":event.request,
        "click_tick":event.click_tick,"item_id":event.item_id,"option":event.option,
        "operation":event.operation,"main_before":event.main_before,
    })).collect::<Vec<_>>();
    let bank_dispatches = session
        .logs
        .bank_dispatches
        .iter()
        .map(|event| {
            json!({
                "host_log_ordinal":event.host_log_ordinal,"run_key":run_key_value(Some(event.key)),
                "tick":event.tick,"request":event.request,"bank_generation":event.bank_generation,
                "transfer":event.transfer,"item_id":event.item_id,"accepted":event.accepted,
                "emitted_packets":event.emitted_packets,"opcode":event.opcode,
            })
        })
        .collect::<Vec<_>>();
    let watchdog_actions = session
        .logs
        .watchdog
        .iter()
        .map(|event| {
            json!({
                "capture_ordinal":event.host_log_ordinal,"host_log_ordinal":event.host_log_ordinal,
                "source":event.source,"level":event.level,
                "account":event.slot_name,"host_tick":event.host_tick,"tick":event.tick,
                "run_key":run_key_value(event.key),"action":event.action,
                "restarted":event.restarted,"raw_record":event.raw_record,
            })
        })
        .collect::<Vec<_>>();
    let cycles = session
        .logs
        .cycles
        .iter()
        .map(|event| {
            json!({
                "server_line_ordinal":event.server_line_ordinal,"raw_cycle":event.payload,
            })
        })
        .collect::<Vec<_>>();
    let outputs = session
        .logs
        .client_outputs
        .iter()
        .map(|event| {
            json!({
                "server_line_ordinal":event.server_line_ordinal,"cycle":event.cycle,
                "actor":event.actor,"player_info_sequence":event.player_info_sequence,
            })
        })
        .collect::<Vec<_>>();
    let receives = session.logs.take_receives.iter().map(|event| json!({
        "receipt_sequence":event.receipt_sequence,"server_line_ordinal":event.server_line_ordinal,
        "cycle":event.cycle,"actor":event.actor,"item_id":event.item_id,"tile":event.tile,
        "option":event.option,"action":event.action,"accepted":event.accepted,"reason":event.reason,
    })).collect::<Vec<_>>();
    let debug_ticks = session.logs.debug_ticks.iter().map(|event| json!({
        "server_line_ordinal":event.server_line_ordinal,"cycle":event.cycle,"raw_line":event.raw_line,
    })).collect::<Vec<_>>();
    let wire_take_ledger = takes.iter().map(|take| {
        let interaction = session.logs.interacts.iter().find(|event| {
            event.slot_name == take.dispatch.actor.account_id
                && event.tick == Some(take.dispatch.tick)
                && matches!(&event.kind, InteractionKind::Ground(request)
                    if request.item_id == take.dispatch.item_id && request.tile == take.dispatch.tile && request.action == "Take")
        });
        let packet = session.logs.packets.iter().find(|packet| {
            packet.account == take.dispatch.actor.account_id
                && packet.tick == take.dispatch.tick
                && packet.key == take.key
                && one_obj3_packet(&packet.opcodes)
        });
        let receive = take.receive.as_ref().map(|event| json!({
            "receipt_sequence":event.receipt_sequence,"server_line_ordinal":event.server_line_ordinal,
            "cycle":event.cycle,"actor":event.actor,"item_id":event.item_id,"tile":event.tile,
            "option":event.option,"action":event.action,"accepted":event.accepted,"reason":event.reason,
        }));
        let outcome = take.outcome.as_ref().map(|event| json!({
            "host_log_ordinal":event.host_log_ordinal,"account":event.slot_name,
            "run_key":run_key_value(Some(event.key)),"tick":event.tick,
            "item_id":event.item_id,"tile":event.tile,"outcome":event.outcome,"units":event.units,
        }));
        json!({
            "wire_run_key":run_key_value(take.key),
            "dispatch":take.dispatch,
            "interaction_stderr_line_ordinal":interaction.map(|event| event.stderr_line_ordinal),
            "packet_stderr_line_ordinals":packet.map(|event| &event.packet_stderr_line_ordinals),
            "packet_dispatch_stderr_line_ordinal":packet.and_then(|event| event.dispatch_stderr_line_ordinal),
            "packet_accepted":packet.map(|event| event.accepted),
            "server_receive":receive,
            "grinch_outcome":outcome,
        })
    }).collect::<Vec<_>>();
    let restart_transitions = restarts
        .iter()
        .map(|restart| {
            json!({
                "account":restart.account,
                "action_run_key":run_key_value(Some(restart.action_key)),
                "action_tick":restart.action_tick,
                "action_capture_ordinal":restart.host_log_ordinal,
                "action_host_log_ordinal":restart.host_log_ordinal,
                "completion_capture_ordinal":restart.completion_host_log_ordinal,
                "completion_host_log_ordinal":restart.completion_host_log_ordinal,
                "before":runtime_observation_json(&restart.before),
                "after":runtime_observation_json(&restart.after),
            })
        })
        .collect::<Vec<_>>();
    let server_lines = session
        .logs
        .server_lines
        .iter()
        .enumerate()
        .map(|(index, line)| json!({"server_line_ordinal":index.saturating_add(1),"raw_line":line}))
        .collect::<Vec<_>>();
    let raw_host_records = session
        .host_rows
        .iter()
        .map(|row| {
            json!({
                "channel":"structured_host","capture_ordinal":row.ordinal,"account":row.slot,"tick":row.tick,
                "source":row.source,"level":row.level,"raw_record":row.message,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema":"grinch-s4-live-meta-v1",
        "case":case.slug(),
        "complete":session.runtime_error.is_none() && failure.is_none(),
        "trace_build_complete":failure.is_none(),
        "accepted_take_wire_ledger_complete":failure.is_none(),
        "failure_reason":failure,
        "oracle_deferred_until_scratch_shutdown":oracle_deferred,
        "run_identity":identity,
        "engine_evidence_manifest_path":session.evidence_manifest_path,
        "engine_evidence_at_capture":session.engine,
        "account_store_prelogin_at_capture":session.account_store_prelogin,
        "host_log_artifact_path":session.host_log_path,
        "stderr_capture_artifact_path":session.stderr_log_path,
        "stderr_capture_prefix_bytes":session.stderr_capture_prefix_bytes,
        "server_log_artifact_path":session.server_log_path,
        "setup_config":session.setup_config,
        "selected_endpoint":{
            "node":session.setup_config.get("node"),
            "web":session.setup_config.get("web"),
        },
        "start":{
            "requested":runtime.start_requested,
            "pre_start_observation_sequence":runtime.start_observation_sequence,
            "pre_start_frame":runtime.pre_start_frame.as_ref().map(|frame| raw_frame_with_runtime_json(frame,runtime)),
            "generation_and_run_key_before_start":before_start.map(runtime_observation_json),
            "generation_and_run_key_after_start":after_start.map(runtime_observation_json),
            "all_runtime_generation_run_key_transitions":runtime_transitions,
            "runtime_observations":runtime.runtime_observations.iter().map(runtime_observation_json).collect::<Vec<_>>(),
            "runtime_observation_ledger_complete":runtime_observation_ledger_complete,
            "initial_native_run_key":run_key_value(runtime.run_key),
            "retained_terminal_stop":runtime.terminal_stop,
        },
        "actors":raw_actors,
        "bank_packet_callbacks":runtime.bank_callbacks,
        "host_event_ledger":{
            "tick_authority":"structured records use their owning client header tick; captured stderr native clocks map to one exact RunKey-linked PLAYER_INFO sequence and must agree with every corresponding structured state clock; no offset inference",
            "raw_host_records":raw_host_records,
            "states":states,"take_events":take_events,"packets":packets,"interactions":interactions,
            "bank_clicks":bank_clicks,"bank_dispatches":bank_dispatches,
            "watchdog_actions":watchdog_actions,"malformed":session.logs.malformed,
        },
        "scratch_server_ledger":{
            "cycles":cycles,"client_outputs":outputs,"take_receives":receives,
            "obj_removals":session.logs.removals.iter().map(|event| json!({
                "server_line_ordinal":event.server_line_ordinal,"receipt":event.receipt,
            })).collect::<Vec<_>>(),
            "obj_respawns":session.logs.respawns.iter().map(|event| json!({
                "server_line_ordinal":event.server_line_ordinal,"receipt":event.receipt,
            })).collect::<Vec<_>>(),
            "debug_ticks":debug_ticks,"raw_lines":server_lines,
        },
        "accepted_take_wire_ledger":wire_take_ledger,
        "restart_transitions":restart_transitions,
        "native_status_by_frame":status_rows,
        "native_status_counter_derivation":{
            "take_sends":"count accepted decoded exact R289 OPOBJ3 dispatch rows for this actor/run before the frame tick",
            "failed_key_attempts":"count exact GRINCH_TAKE outcome rows with outcome=failed_key through the frame tick",
            "avoidance_entries":"sum per-key positive failure-count increments across complete GRINCH_STATE snapshots for the run",
            "duplicate_refusals":"count exact GRINCH_TAKE outcome rows with outcome=already_owned through the frame tick",
            "bank_trips":"count exact accepted OpenBank wire dispatches with a subsequent observed loaded-open PLAYER_INFO receipt through the frame tick",
            "recovery_count":"count raw watchdog ArmWalk actions for the exact run through the frame tick",
            "restart_count":"count paired raw Restart-action/completion transitions into the exact sampled RunKey",
            "raw_watchdog_action_records":raw_watchdog_actions,
            "parsed_watchdog_action_records":parsed_watchdog_actions,
            "raw_restart_completion_records":raw_restart_completions,
            "parsed_restart_completion_records":parsed_restart_completions,
            "raw_watchdog_action_ledger_complete":watchdog_action_ledger_complete,
            "restart_transition_ledger_complete":restart_transition_ledger_complete,
            "runtime_observation_ledger_complete":runtime_observation_ledger_complete,
        },
        "runtime_error":session.runtime_error,
        "setup_error":runtime.setup_error,
        "active_route_candidates":session.bank_candidates.iter().map(|candidate| json!({
            "name":candidate.name,"tile":candidate.tile,"route_ticks":candidate.ticks,
            "walking_steps":candidate.walking_steps,"ordinary_accessible":candidate.ordinary,
            "interaction_tiles":candidate.interaction_tiles,
        })).collect::<Vec<_>>(),
    })
}
pub fn capture_live(case: Case) -> Result<(), String> {
    let inputs = match live_inputs() {
        Ok(inputs) => inputs,
        Err(error) => {
            let dir = required_path("LIVE_EVIDENCE_DIR").map_err(|path_error| {
                format!("{error}; failure evidence cannot be written: {path_error}")
            })?;
            fs::create_dir_all(&dir).map_err(|write_error| {
                format!(
                    "{error}; create failure evidence directory {}: {write_error}",
                    dir.display()
                )
            })?;
            let (rows, capture_error) = match all_host_logs() {
                Ok(rows) => (rows, None),
                Err(capture_error) => (Vec::new(), Some(capture_error)),
            };
            let host_path = std::env::var_os("GRINCH_HOST_LOG")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute());
            let server_path = std::env::var_os("GRINCH_SERVER_LOG")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute());
            let host_write_error = host_path
                .as_deref()
                .and_then(|path| write_host_log(path, &rows).err());
            let meta = failed_capture_meta_json(
                case,
                &error,
                &rows,
                host_path.as_deref(),
                server_path.as_deref(),
                capture_error.as_deref().or(host_write_error.as_deref()),
            );
            if let Err(write_error) = write_failure_receipts(&dir, case, &error, &meta, &meta) {
                return Err(format!(
                    "{error}; failure evidence write failed: {write_error}"
                ));
            }
            return Err(error);
        }
    };
    let mut capture = match capture_live_inner(case, &inputs) {
        Ok(session) => session,
        Err(error) => {
            let (rows, capture_error) = match all_host_logs() {
                Ok(rows) => (rows, None),
                Err(capture_error) => (Vec::new(), Some(capture_error)),
            };
            let host_write_error = write_host_log(&inputs.host_log_path, &rows).err();
            let meta = failed_capture_meta_json(
                case,
                &error,
                &rows,
                Some(&inputs.host_log_path),
                inputs.server_log_path.as_deref(),
                capture_error.as_deref().or(host_write_error.as_deref()),
            );
            let partial =
                json!({"capture_failure":error,"host_event_ledger":meta["host_event_ledger"]});
            if let Err(write_error) =
                write_failure_receipts(&inputs.evidence_dir, case, &error, &partial, &meta)
            {
                return Err(format!(
                    "{error}; failure evidence write failed: {write_error}"
                ));
            }
            return Err(error);
        }
    };
    match build_case_trace(case, &mut capture, &inputs.evidence_dir) {
        Ok((trace, mut meta, variant_oracle)) => {
            let trace_path = evidence_path(&inputs.evidence_dir, case, "trace.json");
            let meta_path = evidence_path(&inputs.evidence_dir, case, "meta.json");
            let oracle_path = evidence_path(&inputs.evidence_dir, case, "oracle.json");
            let runtime_error = capture.runtime_error.clone();
            let mut final_oracle = variant_oracle;
            if let Some(error) = runtime_error.as_deref() {
                meta["complete"] = Value::Bool(false);
                meta["failure_reason"] = json!(error);
                final_oracle = json!({
                    "schema":CAPTURE_SCHEMA,"case":case.slug(),
                    "oracle":final_oracle["oracle"],"passed":false,
                    "incomplete_capture":true,"error":error,"trace_file":trace_path,
                });
            }
            atomic_json(&trace_path, &trace)?;
            atomic_json(&meta_path, &meta)?;
            atomic_json(&oracle_path, &final_oracle)?;
            if let Some(error) = runtime_error {
                return Err(error);
            }
            if final_oracle["passed"].as_bool() == Some(false) {
                return Err(final_oracle["error"]
                    .as_str()
                    .unwrap_or("per-variant oracle failed")
                    .to_owned());
            }
            Ok(())
        }
        Err(error) => {
            let error = match capture.runtime_error.as_deref() {
                Some(runtime_error) => {
                    format!("{runtime_error}; trace construction failed: {error}")
                }
                None => error,
            };
            let partial = runtime_partial_json(&capture.runtime);
            let meta = session_meta_json(&capture, None, &[], &[], &[], false, Some(&error));
            if let Err(write_error) =
                write_failure_receipts(&inputs.evidence_dir, case, &error, &partial, &meta)
            {
                return Err(format!(
                    "{error}; failure evidence write failed: {write_error}"
                ));
            }
            Err(error)
        }
    }
}

fn host_record_json(row: &ObservedLogRecord) -> Value {
    json!({
        "channel":"structured_host","capture_ordinal":row.ordinal,"account":row.slot,"tick":row.tick,
        "source":row.source,"level":row.level,"raw_record":row.message,
    })
}

fn failed_capture_meta_json(
    case: Case,
    error: &str,
    rows: &[ObservedLogRecord],
    host_log_path: Option<&Path>,
    server_log_path: Option<&Path>,
    capture_error: Option<&str>,
) -> Value {
    let raw_watchdog = rows
        .iter()
        .filter(|row| row.message.starts_with("watchdog-action "))
        .map(host_record_json)
        .collect::<Vec<_>>();
    let raw_completions = rows
        .iter()
        .filter(|row| row.message.contains("restarted the script"))
        .map(host_record_json)
        .collect::<Vec<_>>();
    json!({
        "schema":"grinch-s4-live-meta-v1","case":case.slug(),
        "complete":false,"trace_build_complete":false,"accepted_take_wire_ledger_complete":false,
        "failure_reason":error,"runtime_capture_finalized":false,
        "runtime_observations_available":false,
        "runtime_observation_ledger_complete":false,
        "generation_and_run_key_observations":{
            "before_start":Value::Null,"after_start":Value::Null,"transitions":[],
            "unavailable_reason":"capture_live_inner did not return its finalized runtime report",
        },
        "host_log_artifact_path":host_log_path.map(|path| path.display().to_string()),
        "server_log_artifact_path":server_log_path.map(|path| path.display().to_string()),
        "host_event_ledger":{
            "raw_host_records":rows.iter().map(host_record_json).collect::<Vec<_>>(),
            "raw_watchdog_action_records":raw_watchdog,
            "raw_restart_completion_records":raw_completions,
            "capture_error":capture_error,
        },
        "raw_host_log_capture_complete":capture_error.is_none(),
    })
}

fn write_failure_receipts(
    dir: &Path,
    case: Case,
    error: &str,
    partial: &Value,
    meta: &Value,
) -> Result<(), String> {
    let envelope = json!({"schema":CAPTURE_SCHEMA,"case":case.slug(),"complete":false,"trace":Value::Null,"partial":partial,"failure":error});
    let receipt = json!({"schema":CAPTURE_SCHEMA,"case":case.slug(),"passed":false,"oracle":"not_passed","error":error,"trace_file":evidence_path(dir,case,"trace.json")});
    atomic_json(&evidence_path(dir, case, "trace.json"), &envelope)?;
    atomic_json(&evidence_path(dir, case, "meta.json"), meta)?;
    atomic_json(&evidence_path(dir, case, "oracle.json"), &receipt)?;
    Ok(())
}

fn runtime_partial_json(report: &RuntimeReport) -> Value {
    json!({
        "observation_sequence":report.observation_sequence,
        "start_requested":report.start_requested,
        "start_observation_sequence":report.start_observation_sequence,
        "pre_start_frame":report.pre_start_frame.as_ref().map(raw_frame_json),
        "native_run_key":run_key_value(report.run_key),
        "runtime_observations":report.runtime_observations.iter().map(runtime_observation_json).collect::<Vec<_>>(),
        "bank_packet_callbacks":report.bank_callbacks,
        "actors":report.actors.iter().map(|actor| json!({
            "role":format!("{:?}",actor.role),"account":actor.account,
            "frames":actor.frames.iter().map(raw_frame_json).collect::<Vec<_>>(),
            "seed_observation":actor.seed_observation.as_ref().map(raw_frame_json),
            "drop_actions":actor.drop_actions,
            "drop_receipts":actor.drop_receipts.iter().map(|receipt| json!({
                "request_key":receipt.request_key,"send_tick":receipt.send_tick,
                "helper_send_player_info_sequence":receipt.helper_send_player_info_sequence,
                "helper_send_observation_sequence":receipt.helper_send_observation_sequence,
                "receiver_confirmation_sequence":receipt.receiver_confirmation_sequence,
                "receiver_tick":receipt.receiver_tick,"receiver_object_turn":receipt.receiver_object_turn,
                "receiver_sequence":receipt.receiver_sequence,
                "receiver_player_info_sequence":receipt.receiver_player_info_sequence,
                "helper_confirmation_tick":receipt.helper_confirmation_tick,
                "helper_player_info_sequence":receipt.helper_player_info_sequence,
                "helper_observation_sequence":receipt.helper_observation_sequence,
                "packet_opcodes":receipt.packet_opcodes,
            })).collect::<Vec<_>>(),
            "helper_post_drop":actor.helper_post_drop.as_ref().map(raw_frame_json),
            "competitor_takes":actor.competitor_takes.iter().map(|take| json!({
                "item_id":take.item_id,"tile":take.tile,"request_key":take.request_key,
                "tick":take.tick,"observation_sequence":take.observation_sequence,"packet_opcodes":take.packet_opcodes,
            })).collect::<Vec<_>>(),
            "seed_commands":actor.seed_commands,"bank_sample":actor.bank_sample,"error":actor.error,
        })).collect::<Vec<_>>(),
        "setup_error":report.setup_error,
    })
}

fn raw_frame_json(frame: &RawFrame) -> Value {
    json!({
        "role":format!("{:?}",frame.role),"account":frame.account,"actor":frame.actor,
        "tick":frame.tick,"observation_sequence":frame.observation_sequence,"player_info_sequence":frame.player_info_sequence,
        "session_id":frame.session_id(),"position":frame.position,"inventory":frame.inventory,"bank":frame.bank,
        "bank_open":frame.bank_open,"bank_loaded":frame.bank_loaded,"worn":frame.worn,"ground":frame.ground,
        "object_turn":frame.object_turn,"server_cycle":frame.server_cycle,"client_output_phase":frame.output_phase,
        "visible_players":frame.visible_players,"inventory_generation":frame.inventory_generation,"ground_generation":frame.ground_generation,
        "scene_generation":frame.scene_generation
    })
}

fn read_json_artifact(path: &Path, description: &str) -> Result<Value, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("read {description} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {description} {}: {error}", path.display()))
}

fn case_artifacts(dir: &Path, case: Case) -> Result<(Value, Value, Value), String> {
    let trace_path = evidence_path(dir, case, "trace.json");
    let meta_path = evidence_path(dir, case, "meta.json");
    let oracle_path = evidence_path(dir, case, "oracle.json");
    Ok((
        read_json_artifact(&trace_path, "typed case trace")?,
        read_json_artifact(&meta_path, "case metadata")?,
        read_json_artifact(&oracle_path, "variant oracle receipt")?,
    ))
}

fn load_completed_case<T: serde::de::DeserializeOwned>(
    dir: &Path,
    case: Case,
) -> Result<T, String> {
    let (trace, result) = match finalize_variant(dir, case) {
        Ok(finalized) => finalized,
        Err(error) => {
            record_finalization_failure(dir, case, &error)?;
            return Err(error);
        }
    };
    result?;
    serde_json::from_value(trace)
        .map_err(|error| format!("decode {} typed trace: {error}", case.slug()))
}

fn cell_trace_path(dir: &Path, cell: &str) -> PathBuf {
    dir.join(format!("{cell}.trace.json"))
}

fn write_cell_outcome(
    dir: &Path,
    cell: &str,
    oracle_name: &str,
    trace: Option<&Value>,
    result: Result<(), String>,
) -> Result<(), String> {
    let trace_path = cell_trace_path(dir, cell);
    let trace_value = trace.cloned().unwrap_or_else(|| {
        json!({
            "schema":CAPTURE_SCHEMA,"cell":cell,"complete":false,
            "trace":Value::Null,"failure":"typed cell trace could not be assembled",
        })
    });
    let receipt = match &result {
        Ok(()) => json!({
            "schema":CAPTURE_SCHEMA,"cell":cell,"oracle":oracle_name,
            "passed":true,"trace_file":trace_path,"error":Value::Null,
        }),
        Err(error) => json!({
            "schema":CAPTURE_SCHEMA,"cell":cell,"oracle":oracle_name,
            "passed":false,"trace_file":trace_path,"error":error,
        }),
    };
    atomic_json(&trace_path, &trace_value)?;
    atomic_json(&dir.join(format!("{cell}.oracle.json")), &receipt)?;
    result
}

pub fn replay_cell_a() -> Result<(), String> {
    let dir = required_path("LIVE_EVIDENCE_DIR")?;
    let replay = (|| {
        let cell = CellA {
            tradeable: load_completed_case::<HelperDropTrace>(&dir, Case::ATradeableDrops)?,
            green_mask: load_completed_case::<HelperDropTrace>(&dir, Case::AGreenMaskSelection)?,
            negative_candidates: load_completed_case::<NegativeDropTrace>(
                &dir,
                Case::ANegativeCandidates,
            )?,
            private_untradeable: load_completed_case::<HelperDropTrace>(
                &dir,
                Case::APrivateUntradeableDrops,
            )?,
        };
        let trace = serde_json::to_value(&cell)
            .map_err(|error| format!("serialize A cell trace: {error}"))?;
        let result = oracle::check_cell_a(&cell);
        Ok::<_, String>((trace, result))
    })();
    match replay {
        Ok((trace, result)) => {
            write_cell_outcome(&dir, "a-cell", "check_cell_a", Some(&trace), result)
        }
        Err(error) => {
            write_cell_outcome(&dir, "a-cell", "check_cell_a", None, Err(error.clone()))?;
            Err(error)
        }
    }
}

pub fn replay_cell_c() -> Result<(), String> {
    let dir = required_path("LIVE_EVIDENCE_DIR")?;
    let replay = (|| {
        let cell = CellC {
            deposit_28: load_completed_case::<BankFlowTrace>(&dir, Case::CDeposit28)?,
            deposit_27_preserve_item: load_completed_case::<BankFlowTrace>(
                &dir,
                Case::CDeposit27PreserveItem,
            )?,
            inventory_blocked: load_completed_case::<InventoryBlockedTrace>(
                &dir,
                Case::CInventoryBlocked,
            )?,
        };
        let trace = serde_json::to_value(&cell)
            .map_err(|error| format!("serialize C cell trace: {error}"))?;
        let result = oracle::check_cell_c(&cell);
        Ok::<_, String>((trace, result))
    })();
    match replay {
        Ok((trace, result)) => {
            write_cell_outcome(&dir, "c-cell", "check_cell_c", Some(&trace), result)
        }
        Err(error) => {
            write_cell_outcome(&dir, "c-cell", "check_cell_c", None, Err(error.clone()))?;
            Err(error)
        }
    }
}

fn same_scratch_identity(left: &ScratchAuthority, right: &ScratchAuthority) -> bool {
    left.request_id == right.request_id
        && left.scratch_id == right.scratch_id
        && left.source_engine_identity == right.source_engine_identity
        && left.source_content_identity == right.source_content_identity
        && left.engine_identity == right.engine_identity
        && left.content_identity == right.content_identity
        && left.server_tick_ms == right.server_tick_ms
        && left.world_identity == right.world_identity
        && left.account_store_identity == right.account_store_identity
        && left.ports == right.ports
        && left.members_mode == right.members_mode
        && left.copied_engine_content_map_inputs_and_account_storage
            == right.copied_engine_content_map_inputs_and_account_storage
        && left.respawn_phase_convention == right.respawn_phase_convention
        && left.content_patch == right.content_patch
        && left.engine_a_touched == right.engine_a_touched
        && left.engine_q_touched == right.engine_q_touched
}

fn decode_variant<T: serde::de::DeserializeOwned>(case: Case, trace: &Value) -> Result<T, String> {
    serde_json::from_value(trace.clone())
        .map_err(|error| format!("decode {} typed trace: {error}", case.slug()))
}

fn variant_oracle(case: Case, trace: &Value) -> Result<(&'static str, Result<(), String>), String> {
    Ok(match case {
        Case::ATradeableDrops => (
            "check_a_tradeable",
            oracle::check_a_tradeable(&decode_variant(case, trace)?),
        ),
        Case::AGreenMaskSelection => (
            "check_a_green_mask",
            oracle::check_a_green_mask(&decode_variant(case, trace)?),
        ),
        Case::ANegativeCandidates => (
            "check_a_negative_candidates",
            oracle::check_a_negative_candidates(&decode_variant(case, trace)?),
        ),
        Case::APrivateUntradeableDrops => (
            "check_a_private_untradeable",
            oracle::check_a_private_untradeable(&decode_variant(case, trace)?),
        ),
        Case::BPumpkinRespawns => (
            "check_b_pumpkin_respawns",
            oracle::check_b_pumpkin_respawns(&decode_variant(case, trace)?),
        ),
        Case::BContestedPumpkin => (
            "check_b_contested_pumpkin",
            oracle::check_b_contested_pumpkin(&decode_variant(case, trace)?),
        ),
        Case::BScytheDuplicate => (
            "check_b_scythe_duplicate",
            oracle::check_b_duplicate_untradeable(&decode_variant(case, trace)?, SCYTHE_ID),
        ),
        Case::BBunnyEarsDuplicate => (
            "check_b_bunny_ears_duplicate",
            oracle::check_b_duplicate_untradeable(&decode_variant(case, trace)?, BUNNY_EARS_ID),
        ),
        Case::BYoYoRepeat => (
            "check_b_yoyo_repeat",
            oracle::check_b_yoyo_repeat(&decode_variant(case, trace)?),
        ),
        Case::CDeposit28 => (
            "check_c_deposit_28",
            oracle::check_c_deposit_28(&decode_variant(case, trace)?),
        ),
        Case::CDeposit27PreserveItem => (
            "check_c_deposit_27_preserve_item",
            oracle::check_c_deposit_27_preserve_item(&decode_variant(case, trace)?),
        ),
        Case::CInventoryBlocked => (
            "check_c_inventory_blocked",
            oracle::check_c_inventory_blocked(&decode_variant(case, trace)?),
        ),
    })
}

fn finalize_variant(dir: &Path, case: Case) -> Result<(Value, Result<(), String>), String> {
    let (mut trace, mut meta, receipt) = case_artifacts(dir, case)?;
    if meta["case"].as_str() != Some(case.slug())
        || meta["complete"].as_bool() != Some(true)
        || meta["trace_build_complete"].as_bool() != Some(true)
        || meta["accepted_take_wire_ledger_complete"].as_bool() != Some(true)
    {
        return Err(format!("{} capture metadata is incomplete", case.slug()));
    }
    let pending = meta["oracle_deferred_until_scratch_shutdown"].as_bool() == Some(true)
        && receipt["deferred"].as_bool() == Some(true)
        && receipt["passed"].is_null();
    let already_finalized = meta["oracle_deferred_until_scratch_shutdown"].as_bool() == Some(false)
        && meta["scratch_final_stop_verified"].as_bool() == Some(true)
        && receipt["passed"].as_bool() == Some(true);
    if !pending && !already_finalized {
        return Err(format!(
            "{} does not have a valid deferred scratch oracle receipt",
            case.slug()
        ));
    }
    let captured_engine: EngineEvidence =
        serde_json::from_value(meta["engine_evidence_at_capture"].clone()).map_err(|error| {
            format!(
                "{} metadata lacks typed capture-time scratch authority: {error}",
                case.slug()
            )
        })?;
    let captured_authority = &captured_engine.scratch;
    let trace_engine = if case.is_a() {
        trace.get_mut("run").and_then(|run| run.get_mut("engine"))
    } else {
        trace.get_mut("engine")
    }
    .ok_or_else(|| format!("{} typed trace lacks its engine authority", case.slug()))?;
    let trace_authority: EngineEvidence = serde_json::from_value(trace_engine.clone())
        .map_err(|error| format!("decode {} trace engine authority: {error}", case.slug()))?;
    let trace_matches_capture = trace_authority == captured_engine;
    let manifest = meta["engine_evidence_manifest_path"]
        .as_str()
        .ok_or_else(|| {
            format!(
                "{} metadata lacks its final evidence-manifest path",
                case.slug()
            )
        })?;
    let manifest_path = PathBuf::from(manifest);
    if !manifest_path.is_absolute() {
        return Err(format!(
            "{} final evidence-manifest path is not absolute",
            case.slug()
        ));
    }
    let final_bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "read final scratch manifest {} after shutdown: {error}",
            manifest_path.display()
        )
    })?;
    let final_manifest: Value = serde_json::from_slice(&final_bytes).map_err(|error| {
        format!(
            "parse final scratch manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    if final_manifest["scratch"]["account_store_prelogin"]
        != meta["account_store_prelogin_at_capture"]
    {
        return Err(format!(
            "{} pre-login account-store evidence changed after capture",
            case.slug()
        ));
    }
    let final_engine: EngineEvidence = serde_json::from_value(final_manifest).map_err(|error| {
        format!(
            "parse typed final scratch manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let final_authority = &final_engine.scratch;
    if !same_scratch_identity(captured_authority, final_authority) {
        return Err(format!(
            "{} scratch identity changed between capture and final shutdown evidence",
            case.slug()
        ));
    }
    if !trace_matches_capture && trace_authority != final_engine {
        return Err(format!(
            "{} trace engine evidence matches neither capture nor final scratch manifest",
            case.slug()
        ));
    }
    if final_authority.start_count != 1
        || final_authority.stop_count != 1
        || !final_authority.process_tree_stopped
        || final_authority.engine_a_touched
        || final_authority.engine_q_touched
    {
        return Err(format!(
            "{} lacks exact one-start/one-stop full-process-tree scratch evidence",
            case.slug()
        ));
    }
    *trace_engine = serde_json::to_value(&final_engine)
        .map_err(|error| format!("serialize final scratch authority: {error}"))?;
    let (oracle_name, result) = variant_oracle(case, &trace)?;
    let trace_path = evidence_path(dir, case, "trace.json");
    meta["engine_evidence_after_shutdown"] = serde_json::to_value(&final_engine)
        .map_err(|error| format!("serialize final scratch authority: {error}"))?;
    meta["scratch_final_stop_verified"] = Value::Bool(true);
    meta["oracle_deferred_until_scratch_shutdown"] = Value::Bool(false);
    meta["failure_reason"] = Value::Null;
    meta["final_manifest_path"] = json!(manifest_path);
    meta["final_variant_oracle_error"] = result
        .as_ref()
        .err()
        .map_or(Value::Null, |error| json!(error));
    let final_receipt = oracle_receipt(case, oracle_name, result.clone(), &trace_path);
    atomic_json(&trace_path, &trace)?;
    atomic_json(&evidence_path(dir, case, "meta.json"), &meta)?;
    atomic_json(&evidence_path(dir, case, "oracle.json"), &final_receipt)?;
    Ok((trace, result))
}

fn record_finalization_failure(dir: &Path, case: Case, error: &str) -> Result<(), String> {
    let meta_path = evidence_path(dir, case, "meta.json");
    let mut meta = read_json_artifact(&meta_path, "case metadata")
        .unwrap_or_else(|_| json!({"schema":"grinch-s4-live-meta-v1","case":case.slug()}));
    if !meta.is_object() {
        meta = json!({"schema":"grinch-s4-live-meta-v1","case":case.slug()});
    }
    meta["scratch_final_stop_verified"] = Value::Bool(false);
    meta["scratch_finalization_error"] = json!(error);
    let trace_path = evidence_path(dir, case, "trace.json");
    if !trace_path.exists() {
        atomic_json(
            &trace_path,
            &json!({
                "schema":CAPTURE_SCHEMA,"case":case.slug(),"complete":false,
                "trace":Value::Null,"failure":error,
            }),
        )?;
    }
    let receipt = json!({
        "schema":CAPTURE_SCHEMA,"case":case.slug(),"oracle":"scratch_finalization",
        "passed":false,"error":error,"trace_file":trace_path,
    });
    atomic_json(&meta_path, &meta)?;
    atomic_json(&evidence_path(dir, case, "oracle.json"), &receipt)
}

pub fn replay_cell_b() -> Result<(), String> {
    let dir = required_path("LIVE_EVIDENCE_DIR")?;
    let cases = [
        Case::BPumpkinRespawns,
        Case::BContestedPumpkin,
        Case::BScytheDuplicate,
        Case::BBunnyEarsDuplicate,
        Case::BYoYoRepeat,
    ];
    let mut traces = Vec::with_capacity(cases.len());
    let mut failures = Vec::new();
    for case in cases {
        let finalized = finalize_variant(&dir, case).and_then(|(trace, result)| {
            decode_variant::<StaticSpawnTrace>(case, &trace).map(|trace| (trace, result))
        });
        match finalized {
            Ok((trace, result)) => {
                if let Err(error) = result {
                    failures.push(format!("{} oracle: {error}", case.slug()));
                }
                traces.push(trace);
            }
            Err(error) => {
                record_finalization_failure(&dir, case, &error)?;
                failures.push(format!("{} finalization: {error}", case.slug()));
            }
        }
    }
    if traces.len() != cases.len() {
        let error = failures.join("; ");
        write_cell_outcome(&dir, "b-cell", "check_cell_b", None, Err(error.clone()))?;
        return Err(error);
    }
    let mut traces = traces.into_iter();
    let cell = CellB {
        pumpkin_respawns: traces.next().ok_or("missing pumpkin respawn trace")?,
        contested_pumpkin: traces.next().ok_or("missing contested pumpkin trace")?,
        scythe_duplicate: traces.next().ok_or("missing Scythe duplicate trace")?,
        bunny_ears_duplicate: traces.next().ok_or("missing Bunny ears duplicate trace")?,
        yo_yo_repeat: traces.next().ok_or("missing Yo-yo repeat trace")?,
    };
    let cell_trace =
        serde_json::to_value(&cell).map_err(|error| format!("serialize B cell trace: {error}"))?;
    let cell_result = oracle::check_cell_b(&cell);
    let result = if failures.is_empty() {
        cell_result
    } else {
        match cell_result {
            Ok(()) => Err(failures.join("; ")),
            Err(error) => Err(format!("{error}; {}", failures.join("; "))),
        }
    };
    write_cell_outcome(&dir, "b-cell", "check_cell_b", Some(&cell_trace), result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint_hex(name: &str) -> String {
        use std::fmt::Write as _;

        let fingerprint = account_name_fingerprint(name);
        let mut encoded = String::with_capacity(64);
        for byte in fingerprint {
            write!(&mut encoded, "{byte:02x}").unwrap();
        }
        encoded
    }

    #[test]
    fn fresh_account_authority_checks_database_and_profile_saves() {
        let manifest = |database_username_sha256: Vec<String>,
                        profile_save_name_sha256: Vec<String>| {
            json!({
                "scratch": {
                    "account_store_prelogin": {
                        "captured_before_server_start": true,
                        "database_path": "db.sqlite",
                        "account_table": "account",
                        "profile": "main",
                        "save_directory": "data/players/main",
                        "save_suffix": ".sav",
                        "username_hash_scheme": "sha256-utf8-ascii-lowercase",
                        "database_username_sha256": database_username_sha256,
                        "profile_save_name_sha256": profile_save_name_sha256
                    }
                }
            })
        };
        let name = "gr25fresh".to_owned();
        let empty_store = parse_prelogin_account_store(&manifest(Vec::new(), Vec::new())).unwrap();
        assert_eq!(
            validate_fresh_account_names(&empty_store, std::slice::from_ref(&name)).unwrap(),
            BTreeSet::from([name.clone()])
        );

        let existing = fingerprint_hex("GR25FRESH");
        let database_store =
            parse_prelogin_account_store(&manifest(vec![existing.clone()], Vec::new())).unwrap();
        assert!(
            validate_fresh_account_names(&database_store, std::slice::from_ref(&name))
                .unwrap_err()
                .contains("db.sqlite account table")
        );

        let save_store =
            parse_prelogin_account_store(&manifest(Vec::new(), vec![existing])).unwrap();
        assert!(
            validate_fresh_account_names(&save_store, std::slice::from_ref(&name))
                .unwrap_err()
                .contains("data/players/main/gr25fresh.sav")
        );
    }

    fn bank_packet_callback_fixture() -> (Vec<BankCallbackObservation>, BankClickEvent) {
        let prior = BankCallbackObservation {
            sequence: 1,
            host_log_ordinal_fence: 10,
            tick: Some(110),
            player_info_changed: false,
            session_generation: 2,
            bank_generation: 2,
            bank_component: -1,
            modal_stamp: 0,
            bank_loaded: false,
            bank_snapshot: None,
            inventory_packets: vec![BankInventoryPacketObservation {
                component: 5382,
                full_generation: 1,
                full_observation: 7,
            }],
        };
        let loaded = BankCallbackObservation {
            sequence: 2,
            host_log_ordinal_fence: 20,
            tick: Some(111),
            player_info_changed: false,
            session_generation: 2,
            bank_generation: 3,
            bank_component: 5382,
            modal_stamp: 12,
            bank_loaded: true,
            bank_snapshot: Some(ItemSnapshot {
                complete: true,
                quantities: BTreeMap::from([(PUMPKIN_ID, 5)]),
                occupied_slots: None,
                slot_capacity: None,
            }),
            inventory_packets: vec![BankInventoryPacketObservation {
                component: 5382,
                full_generation: 2,
                full_observation: 11,
            }],
        };
        let mut following = loaded.clone();
        following.sequence = 3;
        following.host_log_ordinal_fence = 22;
        let click = BankClickEvent {
            key: WireRunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            bank_generation: 3,
            transfer: 1,
            request: 1,
            click_tick: 111,
            item_id: PUMPKIN_ID,
            option: "Deposit All".into(),
            operation: 4,
            main_before: 28,
            host_log_ordinal: 22,
        };
        (vec![prior, loaded, following], click)
    }

    #[test]
    fn bank_packet_capture_uses_actual_shared_stamp_order_and_click_callback() {
        let (callbacks, click) = bank_packet_callback_fixture();
        let (proof, tick) = bank_packet_order(&callbacks, &click, "deposit-real").unwrap();
        assert_eq!(tick, 111);
        assert_eq!(proof.ambiguous_ui_batches, 0);
        let modal = proof.modal.unwrap();
        let table = proof.table.unwrap();
        let deposit = proof.deposit.unwrap();
        assert_eq!(modal.packet_observation, 12);
        assert_eq!(table.packet_observation, 11);
        assert_eq!(table.ordinal.unwrap().position, 1);
        assert_eq!(modal.ordinal.unwrap().position, 2);
        assert_eq!(deposit.ordinal.unwrap().position, 3);
        assert_eq!(deposit.callback_sequence, 2);
        assert_eq!(deposit.host_log_ordinal_fence, 20);
        assert_eq!(deposit.click_host_log_ordinal, 22);
    }

    #[test]
    fn bank_packet_capture_binds_the_same_table_without_relabeling_its_closed_arrival() {
        let (mut callbacks, click) = bank_packet_callback_fixture();
        let mut arrival = callbacks[0].clone();
        arrival.sequence = 2;
        arrival.host_log_ordinal_fence = 15;
        arrival.player_info_changed = true;
        arrival.inventory_packets = callbacks[1].inventory_packets.clone();
        callbacks[1].sequence = 3;
        callbacks[2].sequence = 4;
        callbacks.insert(1, arrival);
        let (proof, _) = bank_packet_order(&callbacks, &click, "deposit-bound").unwrap();
        let table = proof.table.unwrap();
        let binding = proof.table_binding.unwrap();
        assert_eq!(table.arrival_bank_generation, 2);
        assert!(table.ordinal.is_none());
        assert!(table.bank_snapshot.is_none());
        assert_eq!(proof.ambiguous_ui_batches, 1);
        assert_eq!(binding.bound_to_generation, 3);
        assert_eq!(binding.callback_sequence, 3);
        assert_eq!(binding.packet_observation, table.packet_observation);
        assert_eq!(binding.bank_snapshot, callbacks[2].bank_snapshot);
    }

    #[test]
    fn bank_packet_capture_rejects_a_newer_table_before_the_generation_binding() {
        let (mut callbacks, click) = bank_packet_callback_fixture();
        let mut arrival = callbacks[0].clone();
        arrival.sequence = 2;
        arrival.host_log_ordinal_fence = 15;
        arrival.inventory_packets = callbacks[1].inventory_packets.clone();
        callbacks[1].sequence = 3;
        callbacks[1].inventory_packets[0].full_generation += 1;
        callbacks[1].inventory_packets[0].full_observation += 2;
        callbacks[2].sequence = 4;
        callbacks.insert(1, arrival);
        assert_eq!(
            bank_packet_order(&callbacks, &click, "deposit-newer").unwrap_err(),
            "newer bank table packet arrived before its first-loaded generation binding"
        );
    }

    #[test]
    fn bank_packet_capture_keeps_pi_batch_ambiguity_without_inventing_ordinals() {
        let (mut callbacks, click) = bank_packet_callback_fixture();
        callbacks[1].player_info_changed = true;
        let (proof, _) = bank_packet_order(&callbacks, &click, "deposit-ambiguous").unwrap();
        assert_eq!(proof.ambiguous_ui_batches, 1);
        assert!(proof.modal.unwrap().ordinal.is_none());
        assert!(proof.table.unwrap().ordinal.is_none());
        assert!(proof.deposit.unwrap().ordinal.is_some());
    }

    #[test]
    fn bank_packet_capture_never_reuses_another_sessions_identical_stamp() {
        let (mut callbacks, click) = bank_packet_callback_fixture();
        let mut old_session = callbacks[1].clone();
        old_session.sequence = 0;
        old_session.session_generation = 1;
        callbacks.insert(0, old_session);
        let (proof, _) = bank_packet_order(&callbacks, &click, "deposit-current").unwrap();
        assert_eq!(proof.modal.unwrap().session_generation, 2);
        assert_eq!(proof.table.unwrap().session_generation, 2);
    }

    #[test]
    fn bank_packet_capture_rejects_an_unobserved_first_full_table_packet() {
        let (mut callbacks, click) = bank_packet_callback_fixture();
        callbacks[1].inventory_packets[0].full_generation = 3;
        assert_eq!(
            bank_packet_order(&callbacks, &click, "deposit-gap").unwrap_err(),
            "first bank table packet was not individually captured"
        );
    }

    #[test]
    fn ordinary_catalog_banks_allow_default_booth_access() {
        let definition = api::named_banks::BANK_CATALOG
            .iter()
            .find(|definition| definition.name == "Varrock West")
            .unwrap();
        assert!(definition.object.is_none());
        assert!(definition.open_first.is_none());
        assert!(definition.npc.is_none());
        assert!(ordinary_bank(definition));
        for name in ["Fishing Guild", "Canifis", "Mage Arena"] {
            let restricted = api::named_banks::BANK_CATALOG
                .iter()
                .find(|definition| definition.name == name)
                .unwrap();
            assert!(!ordinary_bank(restricted), "{name} is not an ordinary bank");
        }
    }

    #[test]
    fn ordinary_catalog_banks_reject_actual_access_overrides() {
        for name in ["Shantay Pass", "Duel Arena"] {
            let definition = api::named_banks::BANK_CATALOG
                .iter()
                .find(|definition| definition.name == name)
                .unwrap();
            assert!(
                definition.object.is_some()
                    || definition.open_first.is_some()
                    || definition.npc.is_some()
            );
            assert!(
                !ordinary_bank(definition),
                "{name} has a real access override"
            );
        }
    }

    #[test]
    fn bank_route_binds_the_interact_tile_not_the_walk_stand_or_generic_name() {
        let booth = Tile {
            x: ANCHOR.x + 1,
            ..ANCHOR
        };
        let candidate = BankCandidate {
            name: "Varrock West".into(),
            tile: ANCHOR,
            ticks: 0.0,
            walking_steps: 0,
            ordinary: true,
            interaction_tiles: vec![booth],
        };
        assert!(bank_open_matches_candidate(&candidate, Some(booth)));
        assert!(!bank_open_matches_candidate(&candidate, Some(ANCHOR)));
        assert!(!bank_open_matches_candidate(&candidate, Some(OUTSIDE)));
        assert!(!bank_open_matches_candidate(&candidate, None));
    }

    #[test]
    fn server_profile_ticks_parse_real_logger_framing_across_chunks() {
        let row = "10/21/2026 4:59:56 AM\t INFO\t tick 0: 18/600 ms, 345 MB heap";
        let mut logs = ObservedLogs::default();
        let mut pending = String::new();
        let split = row.find("600").unwrap() + 1;
        logs.ingest_server_chunk(&row[..split], &mut pending, false);
        assert!(logs.debug_ticks.is_empty());
        logs.ingest_server_chunk(&format!("{}\n", &row[split..]), &mut pending, false);
        assert!(pending.is_empty());
        assert!(logs.malformed.is_empty());
        assert_eq!(logs.debug_ticks.len(), 1);
        assert_eq!(logs.debug_ticks[0].cycle, 0);
        assert_eq!(logs.debug_ticks[0].server_line_ordinal, 1);
        assert_eq!(logs.debug_ticks[0].raw_line, row);
        logs.ingest_server_text("tick 1");
        assert_eq!(logs.debug_ticks.len(), 1);
        logs.ingest_server_text(&row.replace("/600 ", "/300 "));
        logs.ingest_server_text(&row.replace("18/600 ", "bad/600 "));
        assert_eq!(logs.debug_ticks.len(), 1);
        assert_eq!(logs.malformed.len(), 2);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn append_required_debug_fence(row: &str) -> Result<(), String> {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let frame = runtime.actors.remove(0).frames.remove(0);
        let initial = format!(
            "GRINCH_CLIENT_OUTPUT {}\n",
            json!({
                "cycle":160,
                "actor":frame.actor,
                "player_info_sequence":frame.player_info_sequence,
            })
        );
        let temp = TempRoot::new()?;
        let path = temp.path().join("server.log");
        fs::write(&path, &initial).unwrap();
        let watch = ServerLogAppendWatch::new(&path)?;
        let mut logs = ObservedLogs::default();
        let mut pending = String::new();
        let mut offset = 0;
        let text = read_appended_text(&path, &mut offset)?;
        logs.ingest_server_chunk(&text, &mut pending, false);
        assert_eq!(
            required_debug_profile_window(std::iter::once(&frame), &logs)?,
            None
        );
        {
            let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(writer, "{row}").unwrap();
        }
        // The append deliberately precedes wait: registration must preserve it.
        watch.wait()?;
        finish_required_server_log(
            &path,
            std::iter::once(&frame),
            &mut logs,
            &mut offset,
            &mut pending,
        )
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_accepts_the_required_debug_row_after_a_real_append_event() {
        append_required_debug_fence(
            "10/21/2026 4:59:56 AM\t INFO\t tick 160: 18/600 ms, 345 MB heap",
        )
        .expect("the appended server row completes the actual required output cycle");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_fails_if_shutdown_arrives_without_the_required_debug_row() {
        assert_eq!(
            append_required_debug_fence("10/21/2026 4:59:56 AM\t INFO\t Server shutdown complete")
                .unwrap_err(),
            "scratch server shut down before its last required debugProfile tick row"
        );
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn cycle_log_row(frame: &RawFrame, cycle: u64, sequence: u64) -> String {
        format!(
            "GRINCH_CYCLE {}\n",
            json!({
                "cycle":cycle,
                "object_turn":cycle,
                "phase_order":["player_processing","object_lifecycle","client_output"],
                "active_actors":[{
                    "actor":frame.actor,
                    "active":true,
                    "player_info_sequence":sequence,
                    "tile":{"x":frame.position.x,"z":frame.position.z,"plane":frame.position.plane}
                }],
                "player_processing_order":[frame.actor.account_id]
            })
        )
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn client_output_log_row(frame: &RawFrame, cycle: u64, sequence: u64) -> String {
        format!(
            "GRINCH_CLIENT_OUTPUT {}\n",
            json!({
                "cycle":cycle,
                "actor":frame.actor,
                "player_info_sequence":sequence
            })
        )
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn debug_profile_log_row(cycle: u64) -> String {
        format!("10/21/2026 4:59:56 AM\t INFO\t tick {cycle}: 18/600 ms, 345 MB heap\n")
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn finish_after_server_log_append(
        frame: &RawFrame,
        initial: &str,
        appended: &str,
    ) -> (Result<(), String>, ObservedLogs) {
        let temp = TempRoot::new().unwrap();
        let path = temp.path().join("server.log");
        fs::write(&path, initial).unwrap();
        let writer_path = path.clone();
        let appended = appended.to_owned();
        let writer = thread::spawn(move || {
            let mut output = fs::OpenOptions::new()
                .append(true)
                .open(writer_path)
                .unwrap();
            output.write_all(appended.as_bytes()).unwrap();
        });
        let mut logs = ObservedLogs::default();
        let mut offset = 0;
        let mut pending = String::new();
        let result = finish_required_server_log(
            &path,
            std::iter::once(frame),
            &mut logs,
            &mut offset,
            &mut pending,
        );
        writer.join().expect("server log append worker panicked");
        (result, logs)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_accepts_a_required_output_that_arrives_after_cycle_evidence() {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let frame = runtime.actors.remove(0).frames.remove(0);
        let cycle = frame.player_info_sequence;
        let initial = cycle_log_row(&frame, cycle, frame.player_info_sequence);
        let appended = format!(
            "{}{}",
            client_output_log_row(&frame, cycle, frame.player_info_sequence),
            debug_profile_log_row(cycle)
        );
        let (result, _) = finish_after_server_log_append(&frame, &initial, &appended);
        result.expect("the required output row may be appended asynchronously");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_names_missing_output_after_a_live_server_advance() {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let frame = runtime.actors.remove(0).frames.remove(0);
        let initial = cycle_log_row(
            &frame,
            frame.player_info_sequence,
            frame.player_info_sequence,
        );
        let next_sequence = frame.player_info_sequence + 1;
        let appended = format!(
            "{}{}",
            cycle_log_row(&frame, next_sequence, next_sequence),
            debug_profile_log_row(next_sequence)
        );
        let (result, logs) = finish_after_server_log_append(&frame, &initial, &appended);
        assert!(result
            .unwrap_err()
            .contains("lacks its GRINCH_CLIENT_OUTPUT row after completed server cycle"));
        assert!(logs.cycles.iter().any(|cycle| {
            cycle.payload["cycle"].as_u64() == Some(next_sequence)
                && cycle.payload["active_actors"]
                    .as_array()
                    .is_some_and(|actors| {
                        actors.iter().any(|active| {
                            active["active"].as_bool() == Some(true)
                                && active["actor"]["account_id"].as_str()
                                    == Some(frame.actor.account_id.as_str())
                                && active["player_info_sequence"].as_u64() == Some(next_sequence)
                        })
                    })
        }));
        assert!(!logs
            .server_lines
            .iter()
            .any(|line| line.contains("Server shutdown complete")));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_names_missing_final_output_without_a_later_player_info() {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let frame = runtime.actors.remove(0).frames.remove(0);
        let sequence = frame.player_info_sequence;
        let mut logs = ObservedLogs::default();
        logs.ingest_server_text(&format!(
            "{}{}",
            cycle_log_row(&frame, sequence, sequence),
            debug_profile_log_row(sequence)
        ));
        let error = required_server_cycle_window(std::iter::once(&frame), &logs).unwrap_err();
        assert!(error.contains("lacks its GRINCH_CLIENT_OUTPUT row after completed server cycle"));
        assert!(logs.client_outputs.is_empty());
        assert!(!logs
            .server_lines
            .iter()
            .any(|line| line.contains("Server shutdown complete")));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn server_log_fence_names_missing_output_on_shutdown() {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let frame = runtime.actors.remove(0).frames.remove(0);
        let (result, _) = finish_after_server_log_append(
            &frame,
            "",
            "10/21/2026 4:59:56 AM\t INFO\t Server shutdown complete\n",
        );
        assert!(result
            .unwrap_err()
            .contains("shut down before required GRINCH_CLIENT_OUTPUT row"));
    }

    fn required_cycle_ledger_fixture() -> (ObservedLogs, Vec<ServerCycle>) {
        let mut logs = ObservedLogs::default();
        for cycle in 159..=161 {
            logs.ingest_server_text(&format!(
                "GRINCH_CYCLE {}",
                json!({
                    "cycle":cycle,
                    "object_turn":cycle+1,
                    "phase_order":["player_processing","object_lifecycle","client_output"],
                    "active_actors":[],
                    "player_processing_order":[],
                })
            ));
            if cycle < 161 {
                logs.ingest_server_text(&format!(
                    "10/21/2026 4:59:56 AM\t INFO\t tick {cycle}: 18/600 ms, 345 MB heap"
                ));
            }
        }
        logs.ingest_server_text("10/21/2026 4:59:56 AM\t INFO\t tick 162: 18/600 ms, 345 MB heap");
        logs.ingest_server_text(&format!(
            "GRINCH_CLIENT_OUTPUT {}",
            json!({
                "cycle":163,
                "actor":{"account_id":"receiver","player_name":"Receiver"},
                "player_info_sequence":69,
            })
        ));
        let cycles = server_cycles(&logs).unwrap();
        (logs, cycles)
    }

    #[test]
    fn required_cycle_window_excludes_but_retains_unmatched_trailing_server_rows() {
        let (logs, cycles) = required_cycle_ledger_fixture();
        validate_server_cycle_ledger(&logs, &cycles, (159, 160)).unwrap();
        assert_eq!(cycles.len(), 3);
        assert_eq!(logs.debug_ticks.len(), 3);
        assert_eq!(logs.client_outputs[0].cycle, 163);
        assert_eq!(logs.server_lines.len(), 7);
    }

    #[test]
    fn required_cycle_window_still_rejects_a_missing_in_window_debug_row() {
        let (mut logs, cycles) = required_cycle_ledger_fixture();
        logs.debug_ticks.retain(|row| row.cycle != 159);
        assert_eq!(
            validate_server_cycle_ledger(&logs, &cycles, (159, 160)).unwrap_err(),
            "scratch cycle/debug tick ledgers differ: 2 cycles, 1 debug ticks"
        );
    }

    #[test]
    fn required_cycle_window_still_rejects_a_missing_required_cycle_boundary() {
        let (logs, cycles) = required_cycle_ledger_fixture();
        assert_eq!(
            validate_server_cycle_ledger(&logs, &cycles, (158, 160)).unwrap_err(),
            "scratch GRINCH_CYCLE ledger has a missing server cycle in its required window"
        );
    }

    #[test]
    fn native_numeric_fields_match_complete_field_names() {
        assert_eq!(number_field("account=receiver count=1", "count="), Some(1));
        assert_eq!(number_field("account=receiver", "count="), None);
        assert_eq!(number_field("click_tick=68 tick=69", "tick="), Some(69));
        assert_eq!(number_field("click_tick=68", "tick="), None);
    }

    #[test]
    fn native_log_clocks_use_the_owning_client_context_without_offset_inference() {
        let key = "Some(RunKey { slot: 1, run: 1, session: 1 })";
        let messages = [
            format!(
                "GRINCH_STATE {}",
                json!({"run":{"slot":1,"run":1,"session":1},"tick":68})
            ),
            format!(
                "GRINCH_TAKE {}",
                json!({
                    "run":{"slot":1,"run":1,"session":1},"tick":68,
                    "item_id":PUMPKIN_ID,"tile":ANCHOR,"outcome":"picked","units":1,
                })
            ),
            format!("bank-deposit-click run={key} bank_generation=5 transfer=1 request=10 click_tick=68 sequence=68 id={PUMPKIN_ID} slot=0 component=2006 option=Deposit All operation=4 main_before=28 side_before=Some(28) bank_before=Some(0) main_revision=Some(3) side_revision=Some(6)"),
            format!("bank-deposit-dispatch run={key} bank_generation=5 transfer=1 request=10 click_tick=68 tick=68 id={PUMPKIN_ID} slot=0 component=2006 option=Deposit All operation=4 accepted=true refusal=None emitted_packets=1 opcode=Some(248)"),
            format!("watchdog-action run={key} tick=68 action=ArmWalk {{ x: 3185, z: 3440, level: 0 }}"),
        ];
        let mut observed = ObservedLogs::default();
        let mut missing_context = ObservedLogs::default();
        for (index, message) in messages.iter().enumerate() {
            let row = ObservedLogRecord {
                channel: LogChannel::StructuredHost,
                ordinal: u64::try_from(index).unwrap() + 1,
                slot: Some("receiver".into()),
                tick: Some(61),
                source: "Script".into(),
                level: "Debug".into(),
                message: message.clone(),
            };
            observed.ingest_record(row.clone());
            missing_context.ingest_record(ObservedLogRecord { tick: None, ..row });
        }
        assert!(observed.malformed.is_empty(), "{:?}", observed.malformed);
        assert_eq!(observed.states[0].tick, 61);
        assert_eq!(observed.states[0].payload["tick"], 68);
        assert_eq!(observed.take_events[0].tick, 61);
        assert_eq!(observed.bank_clicks[0].click_tick, 61);
        assert_eq!(observed.bank_clicks[0].option, "Deposit All");
        assert_eq!(observed.bank_clicks[0].operation, 4);
        assert_eq!(observed.bank_dispatches[0].tick, 61);
        assert_eq!(observed.watchdog[0].tick, Some(61));
        assert!(observed.watchdog[0].raw_record.contains("tick=68"));
        assert_eq!(missing_context.malformed.len(), messages.len());
        assert!(missing_context.states.is_empty());
        assert!(missing_context.take_events.is_empty());
        assert!(missing_context.packets.is_empty());
        assert!(missing_context.bank_clicks.is_empty());
        assert!(missing_context.bank_dispatches.is_empty());
        assert!(missing_context.watchdog.is_empty());
    }

    #[test]
    fn native_state_join_does_not_leak_later_same_pi_evidence() {
        let (runtime, mut captured, _) = stderr_clock_fixture();
        captured.payload["picked"] = json!(0);
        let captured_host_log_ordinal = captured.host_log_ordinal;
        let mut later = captured.clone();
        later.host_log_ordinal += 1;
        later.payload["picked"] = json!(1);
        let logs = ObservedLogs {
            states: vec![captured, later],
            ..ObservedLogs::default()
        };
        let frame = &runtime.actors[0].frames[0];
        let state = state_for_frame(
            &logs,
            &frame.account,
            frame.tick,
            runtime.run_key.unwrap(),
            captured_host_log_ordinal,
        )
        .unwrap();
        assert_eq!(state.host_log_ordinal, captured_host_log_ordinal);
        assert_eq!(state.payload["picked"], 0);
        assert!(state_for_frame(
            &logs,
            &frame.account,
            frame.tick,
            runtime.run_key.unwrap(),
            0
        )
        .is_none());
        assert_eq!(
            state_for_frame(
                &logs,
                &frame.account,
                frame.tick,
                runtime.run_key.unwrap(),
                captured_host_log_ordinal + 1,
            )
            .unwrap()
            .payload["picked"],
            1
        );
    }

    fn stderr_clock_fixture() -> (RuntimeReport, StateEvent, String) {
        let key = WireRunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let items = ItemSnapshot {
            complete: true,
            quantities: BTreeMap::new(),
            occupied_slots: Some(0),
            slot_capacity: Some(28),
        };
        let frame = RawFrame {
            role: ActorRole::Main,
            account: "receiver".into(),
            actor: ActorRef {
                account_id: "receiver".into(),
                player_name: "Receiver".into(),
            },
            tick: 61,
            observation_sequence: 113,
            player_info_sequence: 68,
            client_session_generation: 2,
            position: ANCHOR,
            inventory: items.clone(),
            bank: None,
            bank_open: false,
            bank_loaded: false,
            worn: items,
            ground: GroundSnapshot {
                state: GroundPageState::Complete,
                rows: Vec::new(),
            },
            object_turn: None,
            server_cycle: None,
            output_phase: None,
            visible_players: Vec::new(),
            inventory_generation: 1,
            ground_generation: 1,
            scene_generation: 1,
        };
        let sample = RuntimeObservation {
            sample_ordinal: 1,
            host_log_ordinal_fence: 1,
            observation_sequence: frame.observation_sequence,
            account: frame.account.clone(),
            client_session_id: Some(frame.session_id()),
            client_session_generation: Some(frame.client_session_generation),
            player_info_sequence: Some(frame.player_info_sequence),
            tick: Some(frame.tick),
            start_requested: true,
            generation: Some(1),
            run_key: Some(key),
        };
        let report = RuntimeReport {
            case: Case::ATradeableDrops,
            actors: vec![ActorReport {
                role: ActorRole::Main,
                account: "receiver".into(),
                frames: vec![frame],
                seed_observation: None,
                drop_actions: Vec::new(),
                drop_receipts: Vec::new(),
                helper_post_drop: None,
                competitor_takes: Vec::new(),
                seed_commands: Vec::new(),
                bank_sample: None,
                error: None,
            }],
            observation_sequence: 113,
            run_key: Some(key),
            terminal_stop: None,
            start_requested: true,
            start_observation_sequence: Some(109),
            pre_start_frame: None,
            runtime_observations: vec![sample],
            bank_callbacks: Vec::new(),
            setup_error: None,
        };
        let state = StateEvent {
            slot_name: "receiver".into(),
            key,
            tick: 61,
            host_log_ordinal: 1,
            payload: json!({"run":{"slot":1,"run":1,"session":1},"tick":68}),
        };
        let text = format!(
            "unrelated compiler output\n\
             [interact receiver] interact Obj {{ x: 3185, z: 3440, level: 0, name: None, action: \"Take\", id: Some({PUMPKIN_ID}) }}\n\
             [interact receiver] native-packet account=receiver run=RunKey {{ slot: 1, run: 1, session: 1 }} tick=68 request=4 opcode=67\n\
             [interact receiver] native-packet account=receiver run=RunKey {{ slot: 1, run: 1, session: 1 }} tick=68 request=4 opcode=110\n\
             [interact receiver] native-packets account=receiver run=RunKey {{ slot: 1, run: 1, session: 1 }} tick=68 request=4 count=2 decoded=true accepted=true\n"
        );
        (report, state, text)
    }

    #[test]
    fn captured_stderr_uses_exact_player_info_clocks_and_real_line_ordinals() {
        let (runtime, state, text) = stderr_clock_fixture();
        let mut logs = ObservedLogs {
            states: vec![state],
            ..ObservedLogs::default()
        };
        logs.ingest_captured_stderr(&text, &runtime).unwrap();
        assert_eq!(logs.packets.len(), 1);
        let packet = &logs.packets[0];
        assert_eq!(packet.tick, 61);
        assert_eq!(packet.count, 2);
        assert_eq!(packet.opcodes, [67, 110]);
        assert_eq!(packet.packet_stderr_line_ordinals, [3, 4]);
        assert_eq!(packet.dispatch_stderr_line_ordinal, Some(5));
        assert!(packet.decoded && packet.accepted);
        assert!(one_obj3_packet(&packet.opcodes));
        assert_eq!(logs.interacts.len(), 1);
        assert_eq!(logs.interacts[0].tick, Some(61));
        assert_eq!(logs.interacts[0].request, 4);
        assert_eq!(logs.interacts[0].stderr_line_ordinal, 2);
        logs.take_events.push(TakeEvent {
            slot_name: "receiver".into(),
            key: runtime.run_key.unwrap(),
            tick: 62,
            host_log_ordinal: 2,
            item_id: PUMPKIN_ID,
            tile: ANCHOR,
            outcome: "picked".into(),
            units: 1,
        });
        let takes = build_wire_takes(&runtime, &logs, Case::ATradeableDrops, &[]).unwrap();
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].dispatch.tick, 61);
        logs.packets[0].count = 1;
        assert!(build_wire_takes(&runtime, &logs, Case::ATradeableDrops, &[]).is_err());
    }

    #[test]
    fn captured_stderr_fails_closed_on_clock_account_and_batch_mismatches() {
        let (runtime, state, text) = stderr_clock_fixture();
        for damaged in [
            text.replace("tick=68", "tick=69"),
            text.replace("account=receiver", "account=other"),
            text.replace("count=2", "count=1"),
            format!("{text}{}", text.lines().last().unwrap()),
            text.lines()
                .filter(|line| !line.ends_with("opcode=67"))
                .collect::<Vec<_>>()
                .join("\n"),
        ] {
            let mut logs = ObservedLogs {
                states: vec![state.clone()],
                ..ObservedLogs::default()
            };
            assert!(
                logs.ingest_captured_stderr(&damaged, &runtime).is_err(),
                "{damaged}"
            );
        }
        let mut mismatched_state = state;
        mismatched_state.tick = 60;
        let mut logs = ObservedLogs {
            states: vec![mismatched_state],
            ..ObservedLogs::default()
        };
        assert!(logs.ingest_captured_stderr(&text, &runtime).is_err());
        assert!(!one_obj3_packet(&[]));
        assert!(!one_obj3_packet(&[67]));
        assert!(!one_obj3_packet(&[110, 67, 110]));
        assert!(one_obj3_packet(&[110]));
        assert!(one_obj3_packet(&[67, 110]));
    }

    #[test]
    fn player_display_name_uses_the_existing_base37_account_codec() {
        let actor = ActorState::new(
            ActorRole::Main,
            "gr25fc6jkg_0".into(),
            Vec::new(),
            false,
            Vec::new(),
        );
        assert_eq!(actor.expected_player_name, "Gr25fc6jkg 0");
        assert!("GR25FC6JKG 0".eq_ignore_ascii_case(&actor.expected_player_name));
        assert!(!"Gr25fc6jkg 1".eq_ignore_ascii_case(&actor.expected_player_name));
        assert!(!"Gr25fc6jk g0".eq_ignore_ascii_case(&actor.expected_player_name));
        assert!(!"Gr25fc6jkg0".eq_ignore_ascii_case(&actor.expected_player_name));
    }

    fn take_and_receipt() -> (TakeDispatch, TakeReceiveEvent) {
        let actor = ActorRef {
            account_id: "receiver".into(),
            player_name: "Receiver".into(),
        };
        (
            TakeDispatch {
                request_key: "take:receiver:7:1".into(),
                actor: actor.clone(),
                item_id: PUMPKIN_ID,
                tile: ANCHOR,
                tick: 7,
                server_cycle: None,
            },
            TakeReceiveEvent {
                receipt_sequence: 1,
                server_line_ordinal: 2,
                cycle: 8,
                actor,
                item_id: PUMPKIN_ID,
                tile: ANCHOR,
                option: 3,
                action: None,
                accepted: false,
                reason: "handler_rejected".into(),
            },
        )
    }

    #[test]
    fn rejected_handler_receipt_still_matches_actual_take() {
        let (take, mut receipt) = take_and_receipt();
        assert!(validate_take_receive(&take, &receipt).is_ok());
        receipt.accepted = true;
        receipt.action = Some("Take".into());
        receipt.reason = "interaction_armed".into();
        assert!(validate_take_receive(&take, &receipt).is_ok());
    }

    #[test]
    fn take_receipt_requires_exact_actor_option_and_action() {
        let (take, receipt) = take_and_receipt();
        let mut wrong_actor = receipt.clone();
        wrong_actor.actor.account_id = "other".into();
        assert!(validate_take_receive(&take, &wrong_actor).is_err());
        let mut wrong_option = receipt.clone();
        wrong_option.option = 2;
        assert!(validate_take_receive(&take, &wrong_option).is_err());
        let mut missing_action = receipt;
        missing_action.accepted = true;
        missing_action.reason = "interaction_armed".into();
        assert!(validate_take_receive(&take, &missing_action).is_err());
    }

    #[test]
    fn run_ids_are_qualified_by_the_real_actor() {
        let key = WireRunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        assert_ne!(key.run_id("receiver_a"), key.run_id("receiver_b"));
    }

    #[test]
    fn recovery_uses_actual_arm_walk_rows_after_the_last_state_change() {
        let key = WireRunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let state = StateEvent {
            slot_name: "receiver".into(),
            key,
            tick: 0,
            host_log_ordinal: 1,
            payload: json!({
                "anchor":tile_to_json(ANCHOR),"phase":"waiting","blocked_reason":null,
                "foreground_children":0,"id_exclusions":[],"failed_keys":[],
                "picked":0,"banked":0,
            }),
        };
        let mut logs = ObservedLogs::default();
        logs.watchdog.push(WatchdogEvent {
            key: Some(key),
            tick: Some(7),
            action: "ArmWalk { x: 3185, z: 3440, level: 0 }".into(),
            raw_record: "actual test input".into(),
            restarted: false,
            slot_name: Some("receiver".into()),
            host_tick: Some(7),
            host_log_ordinal: 2,
            source: "Watchdog".into(),
            level: "Debug".into(),
        });
        let (runtime, _, _) = stderr_clock_fixture();
        let mut capture = runtime.runtime_observations[0].clone();
        capture.tick = Some(8);
        capture.host_log_ordinal_fence = 1;
        let status =
            native_status_for_frame(&state, &capture, "session", &logs, &[], &[], &[]).unwrap();
        assert_eq!(status.recovery_count, 0);
        capture.host_log_ordinal_fence = 2;
        let status =
            native_status_for_frame(&state, &capture, "session", &logs, &[], &[], &[]).unwrap();
        assert_eq!(status.recovery_count, 1);
    }

    #[test]
    fn runtime_tick_order_is_local_to_the_observed_client_session() {
        let key = WireRunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let runtime = Runtime {
            case: Case::ATradeableDrops,
            actors: vec![ActorState::new(
                ActorRole::Main,
                "receiver".into(),
                Vec::new(),
                false,
                Vec::new(),
            )],
            main_index: 0,
            helper_index: None,
            competitor_index: None,
            observation_sequence: 3,
            run_key: Some(key),
            terminal_stop: None,
            start_requested: true,
            start_observation_sequence: Some(3),
            pre_start_frame: None,
            runtime_observations: Vec::new(),
            bank_callbacks: Vec::new(),
            error: None,
            drop_sequence: 0,
            competitor_sequence: 0,
        };
        let mut report = runtime.report();
        report.runtime_observations = [(7, 1, false), (1, 2, false), (2, 2, true)]
            .into_iter()
            .enumerate()
            .map(|(index, (tick, session, started))| {
                let ordinal = u64::try_from(index).unwrap() + 1;
                RuntimeObservation {
                    sample_ordinal: ordinal,
                    host_log_ordinal_fence: ordinal,
                    observation_sequence: ordinal,
                    account: "receiver".into(),
                    client_session_id: Some(format!("receiver:session:{session}")),
                    client_session_generation: Some(session),
                    player_info_sequence: Some(ordinal),
                    tick: Some(tick),
                    start_requested: started,
                    generation: Some(u64::from(started)),
                    run_key: started.then_some(key),
                }
            })
            .collect();
        assert!(runtime_observation_ledger_is_complete(&report));
        let fence = report.runtime_observations[1].host_log_ordinal_fence;
        report.runtime_observations[1].host_log_ordinal_fence = 0;
        assert!(!runtime_observation_ledger_is_complete(&report));
        report.runtime_observations[1].host_log_ordinal_fence = fence;
        report.runtime_observations[1].client_session_id =
            report.runtime_observations[0].client_session_id.clone();
        report.runtime_observations[1].client_session_generation =
            report.runtime_observations[0].client_session_generation;
        assert!(!runtime_observation_ledger_is_complete(&report));
    }

    #[test]
    fn immediate_block_requires_its_real_retained_key_and_absent_active_samples() {
        let (mut report, _, _) = stderr_clock_fixture();
        let key = report.run_key.unwrap();
        let mut before = report.runtime_observations[0].clone();
        before.sample_ordinal = 1;
        before.observation_sequence -= 1;
        before.tick = before.tick.map(|tick| tick - 1);
        before.player_info_sequence = before.player_info_sequence.map(|sequence| sequence - 1);
        before.start_requested = false;
        before.generation = Some(0);
        before.run_key = None;
        let mut after = report.runtime_observations[0].clone();
        after.sample_ordinal = 2;
        after.run_key = None;
        report.case = Case::CInventoryBlocked;
        report.start_observation_sequence = Some(before.observation_sequence);
        report.terminal_stop = Some(TerminalNativeStop {
            run_slot: key.slot,
            run_number: key.run,
            native_session: key.session,
            card: "Grinch".into(),
            native_phase: "Blocked".into(),
            failure_code: "inventory-blocked".into(),
            failure_message: "inventory is full and contains no selected item to deposit".into(),
            status_read_observation_sequence: after.observation_sequence,
            generation_before_start: 0,
            generation_after_stop: 1,
            active_run_key_after_stop: None,
        });
        report.runtime_observations = vec![before, after];
        assert!(runtime_observation_ledger_is_complete(&report));
        assert_eq!(
            frame_for_native_tick(&report, "receiver", key, 68)
                .unwrap()
                .tick,
            61
        );
        assert!(report.runtime_observations[1].run_key.is_none());
        let mut wrong = report.clone();
        wrong.terminal_stop = None;
        assert!(!runtime_observation_ledger_is_complete(&wrong));
        assert!(frame_for_native_tick(&wrong, "receiver", key, 68).is_err());
        let mut wrong = report.clone();
        wrong.case = Case::ATradeableDrops;
        assert!(!runtime_observation_ledger_is_complete(&wrong));
        assert!(frame_for_native_tick(&wrong, "receiver", key, 68).is_err());
        let mut wrong = report.clone();
        wrong.runtime_observations[1].run_key = Some(key);
        assert!(!runtime_observation_ledger_is_complete(&wrong));
        let mut wrong = report.clone();
        wrong.runtime_observations[1].generation = Some(2);
        assert!(!runtime_observation_ledger_is_complete(&wrong));
        assert!(frame_for_native_tick(&wrong, "receiver", key, 68).is_err());
        for field in ["card", "phase", "failure", "slot", "run"] {
            let mut wrong = report.clone();
            let terminal = wrong.terminal_stop.as_mut().unwrap();
            match field {
                "card" => terminal.card = "other".into(),
                "phase" => terminal.native_phase = "Running".into(),
                "failure" => terminal.failure_code = "other".into(),
                "slot" => terminal.run_slot += 1,
                "run" => terminal.run_number += 1,
                _ => unreachable!(),
            }
            assert!(frame_for_native_tick(&wrong, "receiver", key, 68).is_err());
        }
        report.terminal_stop.as_mut().unwrap().native_session += 1;
        assert!(!runtime_observation_ledger_is_complete(&report));
        assert!(frame_for_native_tick(&report, "receiver", key, 68).is_err());
    }

    #[test]
    fn competitor_observes_ground_updates_between_player_info_edges() {
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let mut actor = ActorState::new(
            ActorRole::Competitor,
            "receiver".into(),
            Vec::new(),
            false,
            Vec::new(),
        );
        actor.prep = PrepStage::Ready;
        let mut runtime = Runtime {
            case: Case::BContestedPumpkin,
            actors: vec![actor],
            main_index: 0,
            helper_index: None,
            competitor_index: Some(0),
            observation_sequence: 0,
            run_key: None,
            terminal_stop: None,
            start_requested: true,
            start_observation_sequence: None,
            pre_start_frame: None,
            runtime_observations: Vec::new(),
            bank_callbacks: Vec::new(),
            error: None,
            drop_sequence: 0,
            competitor_sequence: 0,
        };
        let mut cache = client::config::Cache::default();
        let item_id = usize::try_from(PUMPKIN_ID).unwrap();
        cache
            .objs
            .resize_with(item_id + 1, client::config::ObjType::default);
        cache.objs[item_id] = client::config::ObjType {
            id: PUMPKIN_ID,
            name: "Pumpkin".into(),
            op: [None, None, Some("Take".into()), None, None],
            ..Default::default()
        };
        let mut client = Client::new_with_revision(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            client::io::ClientRevision::R289,
        );
        client.cache = Arc::new(cache);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        client.stream = Some(
            client::io::ClientStream::connect(&address.ip().to_string(), address.port()).unwrap(),
        );
        let (_peer, _) = listener.accept().unwrap();
        let mut player = client::dash3d::ClientPlayer::at(5, 5);
        player.name = Some("Receiver".into());
        player.ready = true;
        client.local_player = Some(player);
        client.map_build_base_x = ANCHOR.x - 5;
        client.map_build_base_z = ANCHOR.z - 5;
        client.ingame = true;
        client.scene_state = 2;
        client.out.random = Some(client::io::Isaac::new(&[1, 2, 3, 4]));
        let mut player_info = client::io::Packet::new(vec![0, 0]);
        player_info.set_frame_end(2);
        client.psize = 2;
        client.handle_packet(client::io::ServerProt289::PLAYER_INFO, &mut player_info);
        assert!(client.ingame);
        assert_eq!(client.gens.player_info, 1);
        runtime.on_frame(&mut client, "receiver", true, &data);
        let player_info_sequence = client.gens.player_info;
        assert_eq!(runtime.actors[0].raw_frames.len(), 1);
        assert!(runtime.actors[0].raw_frames[0].ground.rows.is_empty());
        let item_bytes = u16::try_from(PUMPKIN_ID).unwrap().to_be_bytes();
        let mut object_add =
            client::io::Packet::new(vec![0x55, item_bytes[0], item_bytes[1], 0, 1]);
        object_add.set_frame_end(5);
        client.psize = 5;
        client.handle_packet(client::io::ServerProt289::ZONE_OBJ_ADD, &mut object_add);
        assert!(client.ingame);
        runtime.on_frame(&mut client, "receiver", false, &data);
        assert_eq!(runtime.error, None);
        assert_eq!(client.gens.player_info, player_info_sequence);
        assert_eq!(runtime.actors[0].raw_frames.len(), 1);
        assert_eq!(runtime.actors[0].competitor_takes.len(), 1);
        let take = &runtime.actors[0].competitor_takes[0];
        assert_eq!(take.item_id, PUMPKIN_ID);
        assert_eq!(take.tile, ANCHOR);
        assert!(one_obj3_packet(&take.packet_opcodes));
        let frame = &runtime.actors[0].raw_frames[0];
        assert!(take.has_exact_send_evidence(frame));
        assert!(take.scene_generation > frame.scene_generation);
        assert_eq!(take.observed_ground.quantity, 1);
        let mut wrong = take.clone();
        wrong.player_info_sequence += 1;
        assert!(!wrong.has_exact_send_evidence(frame));
        let mut wrong = take.clone();
        wrong.observed_ground.quantity = 0;
        assert!(!wrong.has_exact_send_evidence(frame));
        let mut wrong = take.clone();
        wrong.observed_ground.item_id += 1;
        assert!(!wrong.has_exact_send_evidence(frame));
        let mut wrong = take.clone();
        wrong.observed_ground.tile.x += 1;
        assert!(!wrong.has_exact_send_evidence(frame));
        let mut later_frame = frame.clone();
        later_frame.scene_generation = take.scene_generation + 1;
        assert!(!take.has_exact_send_evidence(&later_frame));
        runtime.on_frame(&mut client, "receiver", false, &data);
        assert_eq!(runtime.error, None);
        assert_eq!(runtime.actors[0].competitor_takes.len(), 1);
    }

    #[test]
    fn player_info_is_captured_during_a_held_tutorial_relog() {
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        for role in [ActorRole::Main, ActorRole::Competitor] {
            let mut actor = ActorState::new(role, "receiver".into(), Vec::new(), false, Vec::new());
            actor.prep = PrepStage::Relog;
            let mut runtime = Runtime {
                case: Case::BContestedPumpkin,
                actors: vec![actor],
                main_index: 0,
                helper_index: None,
                competitor_index: (role == ActorRole::Competitor).then_some(0),
                observation_sequence: 0,
                run_key: None,
                terminal_stop: None,
                start_requested: false,
                start_observation_sequence: None,
                pre_start_frame: None,
                runtime_observations: Vec::new(),
                bank_callbacks: Vec::new(),
                error: None,
                drop_sequence: 0,
                competitor_sequence: 0,
            };
            let mut client = host::prepare_client(
                client::client::ClientConfig {
                    host: "127.0.0.1".into(),
                    port: 1,
                    cache_dir: String::new(),
                    members: true,
                    lowmem: true,
                },
                1,
                Arc::new(client::config::Cache::default()),
                Arc::new(Vec::new()),
                Vec::new(),
            );
            let mut player = client::dash3d::ClientPlayer::at(1, 1);
            player.name = Some("Receiver".into());
            player.ready = true;
            client.local_player = Some(player);
            client.ingame = true;
            client.scene_state = 2;
            client.bump_gens(client::io::ServerProt::PLAYER_INFO);
            runtime.on_frame(&mut client, "receiver", true, &data);
            assert_eq!(runtime.error, None);
            assert_eq!(runtime.actors[0].raw_frames.len(), 1);
            let frame = &runtime.actors[0].raw_frames[0];
            assert_eq!(frame.actor.account_id, "receiver");
            assert_eq!(frame.actor.player_name, "Receiver");
            assert_eq!(frame.player_info_sequence, client.gens.player_info);
            assert_eq!(frame.observation_sequence, 1);
            assert_eq!(runtime.observation_sequence, 1);
            assert!(matches!(runtime.actors[0].prep, PrepStage::Relog));
            client.bump_gens(client::io::ServerProt::PLAYER_INFO);
            runtime.on_frame(&mut client, "receiver", true, &data);
            assert_eq!(runtime.actors[0].raw_frames.len(), 2);
            assert_eq!(
                runtime.actors[0].raw_frames[1].player_info_sequence,
                client.gens.player_info
            );
            assert_eq!(runtime.observation_sequence, 2);
        }
    }

    #[test]
    fn runtime_samples_preserve_each_player_info_edge_in_a_burst() {
        let (fixture, _, _) = stderr_clock_fixture();
        let key = fixture.run_key.unwrap();
        let mut frame = fixture.actors[0].frames[0].clone();
        let mut runtime = Runtime {
            case: Case::ATradeableDrops,
            actors: vec![ActorState::new(
                ActorRole::Main,
                "receiver".into(),
                Vec::new(),
                false,
                Vec::new(),
            )],
            main_index: 0,
            helper_index: None,
            competitor_index: None,
            observation_sequence: frame.observation_sequence,
            run_key: Some(key),
            terminal_stop: None,
            start_requested: true,
            start_observation_sequence: Some(frame.observation_sequence - 1),
            pre_start_frame: None,
            runtime_observations: Vec::new(),
            bank_callbacks: Vec::new(),
            error: None,
            drop_sequence: 0,
            competitor_sequence: 0,
        };
        for _ in 0..3 {
            frame.tick += 1;
            frame.player_info_sequence += 1;
            frame.observation_sequence += 1;
            runtime.observation_sequence = frame.observation_sequence;
            runtime.actors[0].latest = Some(frame.clone());
            runtime.actors[0].raw_frames.push(frame.clone());
            record_runtime_observation(&mut runtime, Some(1), Some(key)).unwrap();
        }
        let report = runtime.report();
        assert_eq!(report.runtime_observations.len(), 3);
        for (index, frame) in report.actors[0].frames.iter().enumerate() {
            let sample = runtime_sample_for_frame(&report.runtime_observations, frame).unwrap();
            assert_eq!(sample.sample_ordinal, u64::try_from(index).unwrap() + 1);
            assert_eq!(sample.observation_sequence, frame.observation_sequence);
            assert_eq!(
                sample.player_info_sequence,
                Some(frame.player_info_sequence)
            );
            assert_eq!(sample.tick, Some(frame.tick));
            assert_eq!(sample.run_key, Some(key));
        }
    }

    #[test]
    fn inventory_completion_clock_uses_the_last_selected_increase() {
        let (fixture, _, _) = stderr_clock_fixture();
        let mut frame = fixture.actors[0].frames[0].clone();
        let mut main = ActorState::new(
            ActorRole::Main,
            "receiver".into(),
            Vec::new(),
            false,
            Vec::new(),
        );
        for (tick, count) in [(10, 0), (11, 1), (12, 1), (20, 2), (21, 2), (22, 1)] {
            frame.tick = tick;
            frame.inventory.quantities.clear();
            if count > 0 {
                frame.inventory.quantities.insert(PUMPKIN_ID, count);
            }
            frame.inventory.occupied_slots = Some(u16::try_from(count).unwrap());
            main.raw_frames.push(frame.clone());
        }
        assert_eq!(
            observed_inventory_delta_tick(&main, 10, &[PUMPKIN_ID]),
            Some(20)
        );
        assert_eq!(
            observed_inventory_delta_tick(&main, 12, &[PUMPKIN_ID]),
            Some(20)
        );
        assert_eq!(
            observed_inventory_delta_tick(&main, 20, &[PUMPKIN_ID]),
            None
        );
        assert_eq!(
            observed_inventory_delta_tick(&main, 10, &[GREEN_MASK_ID]),
            None
        );
    }

    #[test]
    fn object_lifecycle_receipts_preserve_actual_rows_and_server_ordinals() {
        let removal = ObjectLifecycleReceipt {
            cycle: 62,
            item_id: PUMPKIN_ID,
            tile: NEAR,
        };
        let respawn = ObjectLifecycleReceipt {
            cycle: 160,
            ..removal.clone()
        };
        let mut logs = ObservedLogs::default();
        logs.ingest_server_text(&format!(
            "unrelated server row\nGRINCH_OBJ_REMOVE {}\nGRINCH_OBJ_RESPAWN {}\n",
            serde_json::to_string(&removal).unwrap(),
            serde_json::to_string(&respawn).unwrap()
        ));
        assert!(logs.malformed.is_empty());
        assert_eq!(logs.removals.len(), 1);
        assert_eq!(logs.respawns.len(), 1);
        assert_eq!(logs.removals[0].receipt, removal);
        assert_eq!(logs.respawns[0].receipt, respawn);
        assert_eq!(logs.removals[0].server_line_ordinal, 2);
        assert_eq!(logs.respawns[0].server_line_ordinal, 3);
        assert!(logs.take_receives.is_empty());
        assert!(logs.client_outputs.is_empty());
        for invalid in [
            json!({"item_id":PUMPKIN_ID,"tile":{"x":3184,"z":3440,"plane":0}}),
            json!({"cycle":62,"tile":{"x":3184,"z":3440,"plane":0}}),
            json!({"cycle":62,"item_id":PUMPKIN_ID,"tile":{"x":3184,"z":3440}}),
            json!({"cycle":-1,"item_id":PUMPKIN_ID,"tile":{"x":3184,"z":3440,"plane":0}}),
        ] {
            let mut logs = ObservedLogs::default();
            logs.ingest_server_text(&format!(
                "GRINCH_OBJ_REMOVE {invalid}\nGRINCH_OBJ_RESPAWN {invalid}\n"
            ));
            assert!(logs.removals.is_empty());
            assert!(logs.respawns.is_empty());
            assert_eq!(logs.malformed.len(), 2);
            assert!(logs
                .malformed
                .iter()
                .all(|row| row.starts_with("obj-lifecycle:")));
        }
    }

    fn r5_competitor_send_owner_report() -> ActorReport {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let mut report = runtime.actors.remove(0);
        report.role = ActorRole::Competitor;
        let mut owner = report.frames[0].clone();
        owner.role = ActorRole::Competitor;
        owner.tick = 31;
        owner.observation_sequence = 58;
        owner.player_info_sequence = 40;
        owner.client_session_generation = 2;
        owner.server_cycle = Some(80);
        owner.ground = GroundSnapshot {
            state: GroundPageState::Complete,
            rows: vec![GroundRow {
                item_id: PUMPKIN_ID,
                tile: ANCHOR,
                quantity: 1,
            }],
        };
        let mut first = owner.clone();
        first.tick = 32;
        first.observation_sequence = 61;
        first.player_info_sequence = 41;
        first.server_cycle = Some(81);
        let mut last = first.clone();
        last.tick = 33;
        last.observation_sequence = 62;
        last.player_info_sequence = 42;
        last.server_cycle = Some(82);
        report.competitor_takes = vec![PendingCompetitorTake {
            item_id: PUMPKIN_ID,
            tile: ANCHOR,
            request_key: "competitor-take:r5:1".into(),
            tick: owner.tick,
            observation_sequence: owner.observation_sequence,
            player_info_sequence: owner.player_info_sequence,
            scene_generation: owner.scene_generation,
            observed_ground: owner.ground.rows[0].clone(),
            packet_opcodes: vec![67, 110],
        }];
        report.frames = vec![owner, first, last];
        report
    }
    fn fixture_fresh_accounts(report: &ActorReport) -> BTreeSet<String> {
        BTreeSet::from([report.account.clone()])
    }

    #[test]
    fn competitor_projection_r5_receipt_keeps_earliest_real_send_owner() {
        let report = r5_competitor_send_owner_report();
        let fresh_accounts = fixture_fresh_accounts(&report);
        let (identity, frames) =
            project_competitor_frames(&report, &fresh_accounts, 81, 82, true).unwrap();
        assert_eq!(identity.start_tick, 31);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].server_cycle, Some(80));
        assert_eq!(frames[0].observation_sequence, 58);
        assert_eq!(frames[0].player_info_sequence, Some(40));
        assert_eq!(report.frames.len(), 3);
        assert_eq!(report.frames[0].tick, 31);
    }

    #[test]
    fn competitor_projection_rejects_cropped_send_owner() {
        let mut report = r5_competitor_send_owner_report();
        report.frames.remove(0);
        let fresh_accounts = fixture_fresh_accounts(&report);
        assert!(
            project_competitor_frames(&report, &fresh_accounts, 81, 82, true)
                .unwrap_err()
                .contains("competitor send-owning PLAYER_INFO frame is missing")
        );
    }

    #[test]
    fn competitor_projection_rejects_changed_send_owner_identity() {
        let mut report = r5_competitor_send_owner_report();
        report.frames[0].player_info_sequence += 1;
        let fresh_accounts = fixture_fresh_accounts(&report);
        assert!(
            project_competitor_frames(&report, &fresh_accounts, 81, 82, true)
                .unwrap_err()
                .contains("competitor send-owning PLAYER_INFO identity changed")
        );
    }

    #[test]
    fn competitor_identity_uses_its_first_retained_cycle_window_frame() {
        let (mut runtime, _, _) = stderr_clock_fixture();
        let report = &mut runtime.actors[0];
        report.role = ActorRole::Competitor;
        let mut early = report.frames[0].clone();
        early.role = ActorRole::Competitor;
        early.tick = 1;
        early.observation_sequence = 1;
        early.player_info_sequence = 1;
        early.client_session_generation = 1;
        early.server_cycle = Some(40);
        let mut first = early.clone();
        first.tick = 32;
        first.observation_sequence = 58;
        first.player_info_sequence = 40;
        first.client_session_generation = 2;
        first.server_cycle = Some(81);
        let mut last = first.clone();
        last.tick = 33;
        last.observation_sequence = 60;
        last.player_info_sequence = 41;
        last.server_cycle = Some(82);
        report.frames = vec![early, first, last];
        let fresh_accounts = fixture_fresh_accounts(report);
        let (identity, frames) =
            project_competitor_frames(report, &fresh_accounts, 81, 82, true).unwrap();
        assert_eq!(identity.start_tick, 32);
        assert_eq!(identity.session_id, "client:receiver:session:2");
        assert!(identity.active_before_grinch_start);
        assert!(identity.fresh_account);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].tick, identity.start_tick);
        assert_eq!(
            frames[0].session_id.as_deref(),
            Some(identity.session_id.as_str())
        );
        assert_eq!(frames[0].player_info_sequence, Some(40));
        assert_eq!(report.frames.len(), 3);
        assert_eq!(report.frames[0].tick, 1);
        assert_eq!(report.frames[0].client_session_generation, 1);
        assert!(
            !project_competitor_frames(report, &fresh_accounts, 81, 82, false)
                .unwrap()
                .0
                .active_before_grinch_start
        );
        assert!(
            !project_competitor_frames(report, &fresh_accounts, 200, 201, true)
                .unwrap_err()
                .contains("competitor lacks PLAYER_INFO frames in the main cycle window")
        );
        assert!(
            !project_competitor_frames(report, &BTreeSet::new(), 81, 82, true)
                .unwrap()
                .0
                .fresh_account
        );
    }
}

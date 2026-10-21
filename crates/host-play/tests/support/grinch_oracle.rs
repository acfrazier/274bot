//! Typed, serializable evidence and fail-closed oracles for Grinch live cells.
//!
//! The producer must build these records from accepted host actions, complete
//! client ground pages, `PLAYER_INFO` edges, real inventory snapshots, native
//! status, and (for scratch cells) server-cycle receipts. The checkers derive
//! pickup and bank deltas from snapshots. Status counters only corroborate those
//! deltas. A fixture can exercise an oracle, but it is not a live result.
//!
//! Call [`check_cell_a`], [`check_cell_b`], or [`check_cell_c`] for a complete
//! cell. Each per-variant checker is also public for producer-side diagnosis.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

pub const PUMPKIN_ID: i32 = 1959;
pub const GREEN_MASK_ID: i32 = 1053;
pub const BLUE_MASK_ID: i32 = 1055;
pub const RED_MASK_ID: i32 = 1057;
pub const SCYTHE_ID: i32 = 1419;
pub const BUNNY_EARS_ID: i32 = 1037;
pub const YO_YO_ID: i32 = 4079;
pub const ORDINARY_UNSELECTED_ID: i32 = 995;
pub const EASTER_EGG_CERTIFICATE_ID: i32 = 1962;
pub const RAW_RESPAWN_RATE: u32 = 100;
pub const PUMPKIN_RESPAWN_CYCLE_GAP: u64 = 98;
const RESPAWN_OUTPUT_TICK_GAP: u64 = PUMPKIN_RESPAWN_CYCLE_GAP + 1;
const B_RESPAWN_ITEM_IDS: [i32; 4] = [PUMPKIN_ID, SCYTHE_ID, BUNNY_EARS_ID, YO_YO_ID];
const ATTESTED_PACK_PATHS: [&str; 3] = ["server/obj.dat", "server/npc.dat", "client/config"];
pub const MAX_TAKE_SETTLEMENT_TICKS: u64 = 4;
pub const TRADEABLE_IDS: [i32; 15] = [
    1961,
    981,
    1989,
    PUMPKIN_ID,
    GREEN_MASK_ID,
    BLUE_MASK_ID,
    RED_MASK_ID,
    1050,
    962,
    1038,
    1040,
    1042,
    1044,
    1046,
    1048,
];
pub const PRIVATE_UNTRADEABLE_IDS: [i32; 3] = [SCYTHE_ID, BUNNY_EARS_ID, YO_YO_ID];

pub type OracleResult = Result<(), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

impl Tile {
    pub(super) fn within(self, anchor: Self, radius: u8) -> bool {
        self.plane == anchor.plane
            && (self.x - anchor.x).abs().max((self.z - anchor.z).abs()) <= i32::from(radius)
    }

    fn distance(self, other: Self) -> i32 {
        if self.plane != other.plane {
            i32::MAX
        } else {
            (self.x - other.x).abs().max((self.z - other.z).abs())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorRef {
    pub account_id: String,
    pub player_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    pub actor: ActorRef,
    pub run_id: String,
    pub session_id: String,
    pub anchor: Tile,
    /// First observed `PLAYER_INFO` tick after this Start.
    pub start_tick: u64,
    /// Set only when the producer has evidence that this is a throwaway account.
    pub fresh_account: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantIdentity {
    pub actor: ActorRef,
    pub session_id: String,
    pub start_tick: u64,
    pub fresh_account: bool,
    /// Race-cell prerequisite: the competitor was logged in before Grinch Start.
    pub active_before_grinch_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusPhase {
    Preparing,
    Working,
    Waiting,
    Banking,
    Returning,
    InventoryBlocked,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    NoSelectedItemToDeposit,
    DepositDidNotFreeSpace,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroundKey {
    pub item_id: i32,
    pub tile: Tile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeStatus {
    pub run_id: String,
    pub session_id: String,
    pub anchor: Tile,
    pub phase: StatusPhase,
    pub blocked_reason: Option<BlockedReason>,
    pub pickup_units: u32,
    pub banked_units: u32,
    /// Count of accepted ground Take dispatches, not interaction attempts.
    pub take_sends: u32,
    pub bank_trips: u32,
    pub failed_key_attempts: u32,
    pub avoidance_entries: u32,
    pub duplicate_refusals: u32,
    pub failed_key_exclusions: BTreeSet<GroundKey>,
    pub id_exclusions: BTreeSet<i32>,
    pub recovery_count: u32,
    pub restart_count: u32,
    pub foreground_children: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSnapshot {
    pub complete: bool,
    pub quantities: BTreeMap<i32, u32>,
    /// Required for inventory snapshots. Bank snapshots may leave these absent.
    pub occupied_slots: Option<u16>,
    pub slot_capacity: Option<u16>,
}

impl ItemSnapshot {
    pub fn count(&self, item_id: i32) -> u32 {
        self.quantities.get(&item_id).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreStartInventoryObservation {
    pub actor: ActorRef,
    pub session_id: String,
    pub player_info_sequence: u64,
    pub observation_sequence: u64,
    pub inventory: ItemSnapshot,
}

/// One observed inventory seed operation, recorded separately from its pre-Start baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupSeedAction {
    pub actor: ActorRef,
    /// Producer-wide ordinal for the post-seed PLAYER_INFO observation.
    pub observation_sequence: u64,
    /// Complete inventory snapshot from that observation.
    pub inventory: ItemSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundPageState {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundRow {
    pub item_id: i32,
    pub tile: Tile,
    pub quantity: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundSnapshot {
    pub state: GroundPageState,
    pub rows: Vec<GroundRow>,
}

impl GroundSnapshot {
    pub fn count(&self, item_id: i32, tile: Tile) -> u32 {
        self.rows
            .iter()
            .filter(|row| row.item_id == item_id && row.tile == tile)
            .map(|row| row.quantity)
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientOutputPhase {
    AfterObjectLifecycle,
    BeforeObjectLifecycle,
    Unknown,
}

/// One complete client observation taken from one actual `PLAYER_INFO` edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerFrame {
    pub tick: u64,
    /// Producer-wide ordinal for the actual capture operation, across clients.
    pub observation_sequence: u64,
    /// Per-client sequence for the actual `PLAYER_INFO` packet edge.
    pub player_info_sequence: Option<u64>,
    pub actor: ActorRef,
    pub session_id: Option<String>,
    pub position: Tile,
    pub inventory: Option<ItemSnapshot>,
    pub bank: Option<ItemSnapshot>,
    pub bank_open: Option<bool>,
    pub bank_loaded: Option<bool>,
    pub worn: Option<ItemSnapshot>,
    pub ground: Option<GroundSnapshot>,
    /// Grinch frames require this. A non-Grinch race participant may omit it.
    pub status: Option<NativeStatus>,
    /// Engine-observed object-turn count. Required by helper-drop and scratch checks.
    pub object_turn: Option<u64>,
    /// Scratch server cycle associated with this observed client output.
    pub server_cycle: Option<u64>,
    pub client_output_phase: Option<ClientOutputPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceAuthority {
    pub player_info_edges_complete: bool,
    pub ground_pages_complete: bool,
    pub inventory_snapshots_complete: bool,
    pub native_status_complete: bool,
    pub accepted_take_ledger_complete: bool,
    /// Completeness of this runner's setup seed-command log, not external actors.
    pub setup_seed_actions_complete: bool,
    pub helper_drop_actions_complete: bool,
    pub object_turns_complete: bool,
    pub server_cycles_complete: bool,
    pub bank_action_log_complete: bool,
    pub bank_snapshots_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineEvidence {
    pub scratch: Box<ScratchAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopbackPorts {
    pub game: Endpoint,
    pub web: Endpoint,
    pub management: Endpoint,
    pub probed_free_before_start: bool,
    pub selected_before_login: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentPatch {
    pub sha256: String,
    pub preserved_existing_map_rows: bool,
    pub real_map_packer_used: bool,
    pub real_config_packer_used: bool,
    pub attested_pack_hashes_before: BTreeMap<String, String>,
    pub attested_pack_hashes_after: BTreeMap<String, String>,
    pub raw_respawn_rates: BTreeMap<i32, u32>,
    pub static_spawns: Vec<StaticSpawn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RespawnPhaseConvention {
    FirstObjectTurnInRemovalCycleAndOutputAfterLifecycle,
    Undeclared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchAuthority {
    pub request_id: String,
    pub scratch_id: String,
    pub source_engine_identity: String,
    pub source_content_identity: String,
    pub engine_identity: String,
    pub content_identity: String,
    pub server_tick_ms: u16,
    pub world_identity: String,
    pub account_store_identity: String,
    pub ports: LoopbackPorts,
    pub members_mode: bool,
    pub copied_engine_content_map_inputs_and_account_storage: bool,
    pub respawn_phase_convention: RespawnPhaseConvention,
    pub content_patch: Option<ContentPatch>,
    pub start_count: u8,
    pub stop_count: u8,
    pub process_tree_stopped: bool,
    pub engine_a_touched: bool,
    pub engine_q_touched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerRunTrace {
    pub kind: CellKind,
    pub engine: EngineEvidence,
    pub authority: TraceAuthority,
    pub identity: RunIdentity,
    pub pre_start_inventory: PreStartInventoryObservation,
    pub setup_seed_actions: Vec<SetupSeedAction>,
    pub radius: u8,
    pub selected_ids: BTreeSet<i32>,
    pub frames: Vec<PlayerFrame>,
    pub takes: Vec<TakeDispatch>,
    pub drops: Vec<DropAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperSetup {
    pub actor: ActorRef,
    pub seeded_inventory: ItemSnapshot,
    /// Complete helper inventory snapshot from the PLAYER_INFO edge confirming
    /// the real Drop actions have decreased the seeded inventory.
    pub post_drop_inventory: ItemSnapshot,
    /// Helper client's `PLAYER_INFO` edge sequence that supplied the snapshot.
    pub post_drop_player_info_sequence: u64,
    /// Producer-wide ordinal for the helper confirmation capture operation.
    pub post_drop_observation_sequence: u64,
    /// First fully drained Grinch receiver PLAYER_INFO tick after confirmation.
    pub receiver_fence_tick: u64,
    pub seeded_before_grinch_start: bool,
    /// The helper seed operation itself did not modify another account.
    pub helper_seed_action_isolated_to_helper: bool,
    pub target_spawn_cheat_used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropSource {
    RealPlayerInventoryDrop,
    SpawnOrCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropAction {
    pub request_key: String,
    pub actor: ActorRef,
    pub item_id: i32,
    pub tile: Tile,
    pub quantity: u32,
    /// Engine A receiver tick at the accepted helper Drop; never helper-local time.
    pub tick: u64,
    /// Object-turn count on that same captured Grinch receiver PLAYER_INFO frame.
    pub object_turn: u64,
    pub source: DropSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeDispatch {
    /// Producer-assigned key for this exact accepted host Take send.
    pub request_key: String,
    pub actor: ActorRef,
    pub item_id: i32,
    pub tile: Tile,
    pub tick: u64,
    /// Required for scratch-cell sends. Engine A sends leave this absent.
    pub server_cycle: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    ScytheAlreadyOwned,
    BunnyEarsAlreadyOwned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalReceipt {
    pub request_key: String,
    pub item_id: i32,
    pub tile: Tile,
    pub tick: u64,
    pub kind: RefusalKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperDropTrace {
    pub run: PlayerRunTrace,
    pub helper: HelperSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NegativeCandidateKind {
    OrdinaryUnselected,
    Certificate,
    SelectedOutsideRadius,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegativeCandidate {
    pub kind: NegativeCandidateKind,
    pub item_id: i32,
    pub tile: Tile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegativeDropTrace {
    pub run: PlayerRunTrace,
    pub helper: HelperSetup,
    pub candidates: Vec<NegativeCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellA {
    pub tradeable: HelperDropTrace,
    pub green_mask: HelperDropTrace,
    pub negative_candidates: NegativeDropTrace,
    pub private_untradeable: HelperDropTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticSpawn {
    pub item_id: i32,
    pub tile: Tile,
    pub quantity: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerPhase {
    PlayerProcessing,
    ObjectLifecycle,
    ClientOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveActor {
    pub actor: ActorRef,
    pub active: bool,
    pub tile: Tile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCycle {
    pub cycle: u64,
    pub object_turn: u64,
    pub phase_order: Vec<ServerPhase>,
    pub active_actors: Vec<ActiveActor>,
    /// Actual player-processing order, by account identity.
    pub player_processing_order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectLifecycleReceipt {
    pub cycle: u64,
    pub item_id: i32,
    pub tile: Tile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticSpawnTrace {
    pub kind: CellKind,
    pub engine: EngineEvidence,
    pub authority: TraceAuthority,
    pub identity: RunIdentity,
    pub radius: u8,
    pub selected_ids: BTreeSet<i32>,
    pub frames: Vec<PlayerFrame>,
    pub takes: Vec<TakeDispatch>,
    pub cycles: Vec<ServerCycle>,
    pub removals: Vec<ObjectLifecycleReceipt>,
    pub respawns: Vec<ObjectLifecycleReceipt>,
    pub refusals: Vec<RefusalReceipt>,
    pub competitor: Option<ParticipantIdentity>,
    pub competitor_frames: Vec<PlayerFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellB {
    pub pumpkin_respawns: StaticSpawnTrace,
    pub contested_pumpkin: StaticSpawnTrace,
    pub scythe_duplicate: StaticSpawnTrace,
    pub bunny_ears_duplicate: StaticSpawnTrace,
    pub yo_yo_repeat: StaticSpawnTrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankRouteEvidence {
    pub bank_id: String,
    pub nearest_routable: bool,
    pub ordinary_accessible_bank: bool,
    pub exact_route_proven: bool,
    pub used_air_distance_fallback: bool,
    pub walking_steps: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BankActionKind {
    Open,
    Deposit { item_id: i32, quantity: u32 },
    Withdraw { item_id: i32, quantity: u32 },
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankAction {
    pub request_key: String,
    pub tick: u64,
    pub kind: BankActionKind,
}

/// Position in the captured bank packet stream after an actual PLAYER_INFO edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BankPacketOrdinal {
    pub tick: u64,
    pub position: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankUiPacketReceipt {
    pub ordinal: Option<BankPacketOrdinal>,
    pub session_generation: u64,
    pub arrival_bank_generation: u64,
    pub callback_sequence: u64,
    pub packet_observation: u64,
    pub player_info_changed: bool,
    pub bank_snapshot: Option<ItemSnapshot>,
}

/// Association from the first loaded callback, not a relabeled packet arrival.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankTableBinding {
    pub session_generation: u64,
    pub bound_to_generation: u64,
    pub callback_sequence: u64,
    pub packet_observation: u64,
    pub bank_snapshot: Option<ItemSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankDepositPacketReceipt {
    pub request_key: String,
    pub ordinal: Option<BankPacketOrdinal>,
    pub session_generation: u64,
    pub bank_generation: u64,
    pub callback_sequence: u64,
    pub observed_modal_stamp: u64,
    pub observed_table_stamp: u64,
    pub host_log_ordinal_fence: u64,
    pub click_host_log_ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankPacketOrderProof {
    pub modal: Option<BankUiPacketReceipt>,
    pub table: Option<BankUiPacketReceipt>,
    pub table_binding: Option<BankTableBinding>,
    pub deposit: Option<BankDepositPacketReceipt>,
    pub ambiguous_ui_batches: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankOpenReceipt {
    pub request_key: String,
    pub tick: u64,
    pub bank_id: String,
    pub interface_open: bool,
    pub bank_loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankCloseReceipt {
    pub request_key: String,
    pub tick: u64,
    pub bank_id: String,
    pub interface_closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorReturnReceipt {
    pub request_key: String,
    pub tick: u64,
    pub tile: Tile,
    pub arrival_receipt_observed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankFlowReceipts {
    pub open: Option<BankOpenReceipt>,
    pub close: Option<BankCloseReceipt>,
    pub return_to_anchor: Option<AnchorReturnReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankFlowTrace {
    pub kind: CellKind,
    pub engine: EngineEvidence,
    pub authority: TraceAuthority,
    pub identity: RunIdentity,
    pub pre_start_inventory: PreStartInventoryObservation,
    pub setup_seed_actions: Vec<SetupSeedAction>,
    pub radius: u8,
    pub selected_ids: BTreeSet<i32>,
    pub frames: Vec<PlayerFrame>,
    pub takes: Vec<TakeDispatch>,
    pub drops: Vec<DropAction>,
    pub helper: HelperSetup,
    pub starting_pumpkins: u32,
    pub carried_item_id: Option<i32>,
    pub route: BankRouteEvidence,
    pub bank_before: Option<ItemSnapshot>,
    pub bank_after: Option<ItemSnapshot>,
    pub bank_actions: Vec<BankAction>,
    pub receipts: BankFlowReceipts,
    pub packet_order: BankPacketOrderProof,
}

/// The retained native diagnostic for a run that stops before a post-native
/// observation can see an active RunKey. The sampled active key remains absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNativeStop {
    pub run_slot: u64,
    pub run_number: u64,
    pub native_session: u64,
    pub card: String,
    pub native_phase: String,
    pub failure_code: String,
    pub failure_message: String,
    pub status_read_observation_sequence: u64,
    pub generation_before_start: u64,
    pub generation_after_stop: u64,
    pub active_run_key_after_stop: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryBlockedTrace {
    pub kind: CellKind,
    pub engine: EngineEvidence,
    pub authority: TraceAuthority,
    pub identity: RunIdentity,
    pub terminal_stop: TerminalNativeStop,
    pub pre_start_inventory: PreStartInventoryObservation,
    pub setup_seed_actions: Vec<SetupSeedAction>,
    pub selected_ids: BTreeSet<i32>,
    pub frames: Vec<PlayerFrame>,
    pub bank_actions: Vec<BankAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellC {
    pub deposit_28: BankFlowTrace,
    pub deposit_27_preserve_item: BankFlowTrace,
    pub inventory_blocked: InventoryBlockedTrace,
}

fn ensure(condition: bool, message: impl Into<String>) -> OracleResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn nonempty(value: &str, name: &str) -> OracleResult {
    ensure(!value.trim().is_empty(), format!("missing {name}"))
}

fn set(values: impl IntoIterator<Item = i32>) -> BTreeSet<i32> {
    values.into_iter().collect()
}

fn require_unmodified_scratch(engine: &EngineEvidence) -> OracleResult {
    let scratch = &engine.scratch;
    validate_scratch_authority(scratch)?;
    ensure(
        scratch.content_patch.is_none(),
        "unmodified-content cell received a respawn content patch",
    )?;
    ensure(
        scratch.source_content_identity == scratch.content_identity,
        "unmodified-content scratch differs from the protected source content",
    )
}

fn require_client_settlement_clock(frames: &[PlayerFrame], takes: &[TakeDispatch]) -> OracleResult {
    ensure(
        frames.iter().all(|frame| frame.server_cycle.is_none()),
        "helper/bank trace substitutes server cycles for client settlement ticks",
    )?;
    ensure(
        takes.iter().all(|take| take.server_cycle.is_none()),
        "helper/bank Take substitutes server cycles for client settlement ticks",
    )
}

fn require_trace_authority(
    authority: &TraceAuthority,
    need_helper: bool,
    need_object_turns: bool,
    need_cycles: bool,
    need_bank: bool,
) -> OracleResult {
    ensure(
        authority.player_info_edges_complete,
        "missing complete PLAYER_INFO edge authority",
    )?;
    ensure(
        authority.ground_pages_complete,
        "missing complete posted-ground-page authority",
    )?;
    ensure(
        authority.inventory_snapshots_complete,
        "missing complete inventory-snapshot authority",
    )?;
    ensure(
        authority.native_status_complete,
        "missing native status authority",
    )?;
    ensure(
        authority.accepted_take_ledger_complete,
        "missing accepted Take dispatch ledger authority",
    )?;
    if need_helper {
        ensure(
            authority.helper_drop_actions_complete,
            "missing real helper Drop action authority",
        )?;
    }
    if need_object_turns {
        ensure(
            authority.object_turns_complete,
            "missing observed server object-turn authority",
        )?;
    }
    if need_cycles {
        ensure(
            authority.server_cycles_complete,
            "missing complete scratch server-cycle authority",
        )?;
    }
    if need_bank {
        ensure(
            authority.bank_action_log_complete && authority.bank_snapshots_complete,
            "missing complete bank action or bank snapshot authority",
        )?;
    }
    Ok(())
}

fn require_setup_seed_action_authority(authority: &TraceAuthority) -> OracleResult {
    ensure(
        authority.setup_seed_actions_complete,
        "missing complete runner-owned setup seed-action log",
    )
}

fn require_inventory<'a>(
    snapshot: &'a Option<ItemSnapshot>,
    context: &str,
) -> Result<&'a ItemSnapshot, String> {
    let snapshot = snapshot
        .as_ref()
        .ok_or_else(|| format!("{context}: missing inventory snapshot"))?;
    ensure(
        snapshot.complete,
        format!("{context}: incomplete inventory snapshot"),
    )?;
    ensure(
        snapshot.occupied_slots.is_some() && snapshot.slot_capacity.is_some(),
        format!("{context}: missing inventory slot authority"),
    )?;
    Ok(snapshot)
}

fn require_ground<'a>(
    snapshot: &'a Option<GroundSnapshot>,
    context: &str,
) -> Result<&'a GroundSnapshot, String> {
    let snapshot = snapshot
        .as_ref()
        .ok_or_else(|| format!("{context}: missing ground page"))?;
    ensure(
        snapshot.state == GroundPageState::Complete,
        format!("{context}: ground page is partial or unavailable"),
    )?;
    for row in &snapshot.rows {
        ensure(
            row.item_id > 0 && row.quantity > 0,
            format!("{context}: invalid ground row"),
        )?;
    }
    Ok(snapshot)
}

fn validate_frame_sequence(
    identity: &RunIdentity,
    frames: &[PlayerFrame],
    require_object_turns: bool,
    require_scratch_output: bool,
) -> OracleResult {
    validate_observed_frames(
        &identity.actor,
        &identity.session_id,
        identity.start_tick,
        Some((&identity.run_id, identity.anchor)),
        frames,
        require_object_turns,
        require_scratch_output,
    )
}

fn validate_participant_frame_sequence(
    identity: &ParticipantIdentity,
    frames: &[PlayerFrame],
    require_object_turns: bool,
    require_scratch_output: bool,
) -> OracleResult {
    validate_observed_frames(
        &identity.actor,
        &identity.session_id,
        identity.start_tick,
        None,
        frames,
        require_object_turns,
        require_scratch_output,
    )
}

fn validate_observed_frames(
    actor: &ActorRef,
    session_id: &str,
    start_tick: u64,
    status_identity: Option<(&str, Tile)>,
    frames: &[PlayerFrame],
    require_object_turns: bool,
    require_scratch_output: bool,
) -> OracleResult {
    nonempty(&actor.account_id, "account identity")?;
    nonempty(&actor.player_name, "player identity")?;
    nonempty(session_id, "session identity")?;
    if let Some((run_id, _)) = status_identity {
        nonempty(run_id, "run identity")?;
    }
    ensure(!frames.is_empty(), "missing observed PLAYER_INFO frames")?;
    ensure(
        frames[0].tick == start_tick,
        "first observed frame is not the recorded Start tick",
    )?;
    let mut prior_sequence = None;
    let mut prior_observation_sequence = None;
    let mut prior_object_turn = None;
    let mut previous_status: Option<&NativeStatus> = None;
    for (index, frame) in frames.iter().enumerate() {
        let expected_tick = start_tick + index as u64;
        ensure(
            frame.tick == expected_tick,
            format!("missing or stale PLAYER_INFO tick at {}", expected_tick),
        )?;
        ensure(frame.actor == *actor, "mixed actor identity in run frames")?;
        ensure(
            frame.session_id.as_deref() == Some(session_id),
            "session identity changed or is absent from PLAYER_INFO frame",
        )?;
        if let Some(previous) = prior_observation_sequence {
            ensure(
                frame.observation_sequence > previous,
                "producer observation ordinal is stale or repeated",
            )?;
        }
        prior_observation_sequence = Some(frame.observation_sequence);
        let sequence = frame
            .player_info_sequence
            .ok_or_else(|| format!("tick {} lacks PLAYER_INFO edge sequence", frame.tick))?;
        if let Some(previous) = prior_sequence {
            ensure(
                sequence > previous,
                "PLAYER_INFO edge sequence is stale or repeated",
            )?;
        }
        prior_sequence = Some(sequence);
        require_inventory(&frame.inventory, "PLAYER_INFO frame")?;
        require_ground(&frame.ground, "PLAYER_INFO frame")?;
        if require_object_turns {
            let object_turn = frame
                .object_turn
                .ok_or_else(|| format!("tick {} lacks observed object-turn value", frame.tick))?;
            if let Some(previous) = prior_object_turn {
                ensure(
                    object_turn >= previous,
                    "object-turn observation moved backwards",
                )?;
            }
            prior_object_turn = Some(object_turn);
        }
        if require_scratch_output {
            ensure(
                frame.server_cycle.is_some(),
                format!("tick {} lacks server-cycle identity", frame.tick),
            )?;
            ensure(
                frame.client_output_phase == Some(ClientOutputPhase::AfterObjectLifecycle),
                format!(
                    "tick {} lacks post-lifecycle client-output phase",
                    frame.tick
                ),
            )?;
        }
        if let Some((run_id, anchor)) = status_identity {
            let status = frame
                .status
                .as_ref()
                .ok_or_else(|| format!("tick {} lacks native status", frame.tick))?;
            ensure(
                status.run_id == run_id,
                "run identity changed in native status",
            )?;
            ensure(
                status.session_id == session_id,
                "session identity changed in native status",
            )?;
            ensure(
                status.anchor == anchor,
                "anchor identity changed in native status",
            )?;
            ensure(
                status.foreground_children <= 1,
                "native status retained more than one foreground child",
            )?;
            if let Some(previous) = previous_status {
                ensure(
                    status.pickup_units >= previous.pickup_units
                        && status.banked_units >= previous.banked_units
                        && status.take_sends >= previous.take_sends
                        && status.bank_trips >= previous.bank_trips
                        && status.failed_key_attempts >= previous.failed_key_attempts
                        && status.avoidance_entries >= previous.avoidance_entries
                        && status.duplicate_refusals >= previous.duplicate_refusals
                        && status.recovery_count >= previous.recovery_count
                        && status.restart_count >= previous.restart_count,
                    "native status counter moved backwards",
                )?;
                ensure(
                    previous
                        .failed_key_exclusions
                        .is_subset(&status.failed_key_exclusions)
                        && previous.id_exclusions.is_subset(&status.id_exclusions),
                    "native status exclusion disappeared within one run",
                )?;
            }
            previous_status = Some(status);
        } else {
            ensure(
                frame.status.is_none(),
                "non-Grinch participant unexpectedly carries Grinch status",
            )?;
        }
    }
    Ok(())
}

fn validate_pre_start_inventory(
    observation: &PreStartInventoryObservation,
    identity: &RunIdentity,
    frames: &[PlayerFrame],
) -> OracleResult {
    nonempty(&observation.actor.account_id, "pre-Start account identity")?;
    nonempty(&observation.actor.player_name, "pre-Start player identity")?;
    nonempty(&observation.session_id, "pre-Start session identity")?;
    ensure(
        observation.actor == identity.actor,
        "pre-Start inventory belongs to another account",
    )?;
    ensure(
        observation.session_id == identity.session_id,
        "pre-Start inventory belongs to another session",
    )?;
    ensure(
        observation.inventory.complete,
        "pre-Start inventory snapshot is incomplete",
    )?;
    ensure(
        observation.inventory.occupied_slots.is_some()
            && observation.inventory.slot_capacity.is_some(),
        "pre-Start inventory snapshot lacks slot authority",
    )?;
    let first = frames
        .first()
        .ok_or_else(|| "missing Start PLAYER_INFO frame".to_string())?;
    let start_sequence = first
        .player_info_sequence
        .ok_or_else(|| "Start frame lacks a PLAYER_INFO edge sequence".to_string())?;
    ensure(
        observation.player_info_sequence < start_sequence,
        "pre-Start inventory PLAYER_INFO sequence does not precede Start",
    )?;
    ensure(
        observation.observation_sequence < first.observation_sequence,
        "pre-Start inventory observation does not precede Start",
    )?;
    ensure(
        &observation.inventory == require_inventory(&first.inventory, "Start inventory")?,
        "pre-Start inventory snapshot changed before Start",
    )
}

fn main_status(frame: &PlayerFrame) -> Result<&NativeStatus, String> {
    frame
        .status
        .as_ref()
        .ok_or_else(|| format!("tick {} lacks native status", frame.tick))
}

fn first_status(frames: &[PlayerFrame]) -> Result<&NativeStatus, String> {
    frames
        .first()
        .ok_or_else(|| "missing frames".to_string())
        .and_then(main_status)
}

fn last_status(frames: &[PlayerFrame]) -> Result<&NativeStatus, String> {
    frames
        .last()
        .ok_or_else(|| "missing frames".to_string())
        .and_then(main_status)
}

fn count(snapshot: &Option<ItemSnapshot>, item_id: i32) -> Result<u32, String> {
    Ok(require_inventory(snapshot, "inventory count")?.count(item_id))
}

fn frame_at(frames: &[PlayerFrame], tick: u64) -> Result<&PlayerFrame, String> {
    frames
        .iter()
        .find(|frame| frame.tick == tick)
        .ok_or_else(|| format!("tick {tick} has no observed PLAYER_INFO frame"))
}

fn check_fresh_status(frames: &[PlayerFrame]) -> OracleResult {
    let status = first_status(frames)?;
    ensure(
        status.pickup_units == 0
            && status.banked_units == 0
            && status.take_sends == 0
            && status.bank_trips == 0
            && status.failed_key_attempts == 0
            && status.avoidance_entries == 0
            && status.duplicate_refusals == 0
            && status.failed_key_exclusions.is_empty()
            && status.id_exclusions.is_empty()
            && status.recovery_count == 0
            && status.restart_count == 0,
        "run did not start with clean native counters and exclusions",
    )
}

fn positive_inventory_deltas(frames: &[PlayerFrame]) -> Result<Vec<(i32, u32, u64)>, String> {
    let mut deltas = Vec::new();
    for pair in frames.windows(2) {
        let previous = require_inventory(&pair[0].inventory, "prior inventory")?;
        let current = require_inventory(&pair[1].inventory, "current inventory")?;
        let ids: BTreeSet<i32> = previous
            .quantities
            .keys()
            .chain(current.quantities.keys())
            .copied()
            .collect();
        for item_id in ids {
            let before = previous.count(item_id);
            let after = current.count(item_id);
            if after > before {
                deltas.push((item_id, after - before, pair[1].tick));
            }
        }
    }
    Ok(deltas)
}

fn validate_status_matches_observations(
    frames: &[PlayerFrame],
    takes: &[TakeDispatch],
) -> OracleResult {
    let first = first_status(frames)?;
    let deltas = positive_inventory_deltas(frames)?;
    let mut next_delta = 0;
    let mut observed_pickups = 0_u32;
    for frame in frames {
        while next_delta < deltas.len() && deltas[next_delta].2 <= frame.tick {
            observed_pickups += deltas[next_delta].1;
            next_delta += 1;
        }
        let status = main_status(frame)?;
        ensure(
            status.pickup_units >= first.pickup_units,
            "native pickup counter predates its baseline",
        )?;
        ensure(
            status.take_sends >= first.take_sends,
            "native Take-send counter predates its baseline",
        )?;
        ensure(
            status.pickup_units - first.pickup_units == observed_pickups,
            "native pickup counter disagrees with observed inventory deltas at a PLAYER_INFO edge",
        )?;
        let observed_sends = takes.iter().filter(|take| take.tick < frame.tick).count() as u32;
        ensure(
            status.take_sends - first.take_sends == observed_sends,
            "native Take-send counter disagrees with accepted dispatches at a PLAYER_INFO edge",
        )?;
    }
    Ok(())
}

fn validate_take_ledger(
    identity: &RunIdentity,
    radius: u8,
    selected_ids: &BTreeSet<i32>,
    frames: &[PlayerFrame],
    takes: &[TakeDispatch],
) -> OracleResult {
    let mut keys = HashSet::new();
    for take in takes {
        nonempty(&take.request_key, "Take request key")?;
        ensure(
            keys.insert(take.request_key.as_str()),
            "duplicate Take request key",
        )?;
        ensure(
            take.actor == identity.actor,
            "Take dispatch belongs to another actor",
        )?;
        ensure(
            selected_ids.contains(&take.item_id),
            "Take dispatch used an unselected ID",
        )?;
        ensure(
            take.tile.within(identity.anchor, radius),
            "Take dispatch targeted a tile outside the anchor radius or plane",
        )?;
        let frame = frame_at(frames, take.tick)?;
        let ground = require_ground(&frame.ground, "Take dispatch frame")?;
        ensure(
            ground.count(take.item_id, take.tile) > 0,
            format!(
                "Take {} lacks an exact-ID row at its dispatch tick",
                take.request_key
            ),
        )?;
    }
    Ok(())
}

fn match_successful_takes(
    identity: &RunIdentity,
    frames: &[PlayerFrame],
    takes: &[TakeDispatch],
    allowed_unlanded: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let mut used = BTreeSet::new();
    for (item_id, amount, landed_tick) in positive_inventory_deltas(frames)? {
        ensure(
            amount == 1,
            format!("item {item_id} landed as a non-unit delta"),
        )?;
        let candidates: Vec<&TakeDispatch> = takes
            .iter()
            .filter(|take| {
                take.actor == identity.actor
                    && take.item_id == item_id
                    && take.tick < landed_tick
                    && landed_tick - take.tick <= MAX_TAKE_SETTLEMENT_TICKS
                    && !used.contains(&take.request_key)
            })
            .collect();
        ensure(
            candidates.len() == 1,
            format!("item {item_id} inventory increase lacks one exact-ID Take request"),
        )?;
        let take = candidates[0];
        used.insert(take.request_key.clone());
    }
    for take in takes {
        if !used.contains(&take.request_key) && !allowed_unlanded.contains(&take.request_key) {
            return Err(format!(
                "accepted Take {} neither landed nor has an allowed refusal/race outcome",
                take.request_key
            ));
        }
    }
    Ok(used)
}

fn exact_final_delta(frames: &[PlayerFrame], expected: &BTreeMap<i32, i32>) -> OracleResult {
    let first = require_inventory(&frames[0].inventory, "initial inventory")?;
    let last = require_inventory(&frames[frames.len() - 1].inventory, "final inventory")?;
    let ids: BTreeSet<i32> = first
        .quantities
        .keys()
        .chain(last.quantities.keys())
        .chain(expected.keys())
        .copied()
        .collect();
    for id in ids {
        let before = i64::from(first.count(id));
        let after = i64::from(last.count(id));
        let wanted = i64::from(expected.get(&id).copied().unwrap_or(0));
        ensure(
            after - before == wanted,
            format!(
                "item {id} final pack delta is {}, expected {wanted}",
                after - before
            ),
        )?;
    }
    Ok(())
}

fn exact_positive_delta_map(frames: &[PlayerFrame]) -> Result<BTreeMap<i32, u32>, String> {
    let mut map = BTreeMap::new();
    for (id, amount, _) in positive_inventory_deltas(frames)? {
        *map.entry(id).or_insert(0) += amount;
    }
    Ok(map)
}

fn require_no_failures_or_exclusions(frames: &[PlayerFrame]) -> OracleResult {
    for frame in frames {
        let status = main_status(frame)?;
        ensure(
            status.failed_key_attempts == 0
                && status.avoidance_entries == 0
                && status.failed_key_exclusions.is_empty()
                && status.id_exclusions.is_empty(),
            "trace contains a failure, avoidance entry, or run exclusion",
        )?;
    }
    Ok(())
}

fn ensure_no_bank_trip(frames: &[PlayerFrame]) -> OracleResult {
    for frame in frames {
        let status = main_status(frame)?;
        ensure(status.bank_trips == 0, "unexpected bank trip")?;
        ensure(
            frame.bank_open != Some(true),
            "bank remained open in a no-bank cell",
        )?;
    }
    Ok(())
}

fn check_idle_tail(
    frames: &[PlayerFrame],
    after_tick: u64,
    expected_anchor: Tile,
    require_anchor: bool,
    ticks: usize,
    required_id_exclusions: &BTreeSet<i32>,
    takes: &[TakeDispatch],
) -> OracleResult {
    ensure(
        ticks > 0 && frames.len() >= ticks,
        "missing healthy Waiting tick tail",
    )?;
    let tail = &frames[frames.len() - ticks..];
    ensure(
        tail[0].tick > after_tick,
        "healthy Waiting tail started before the required work completed",
    )?;
    for pair in tail.windows(2) {
        ensure(
            pair[1].tick == pair[0].tick + 1,
            "healthy Waiting tail has a missing observed game tick",
        )?;
    }
    for frame in tail {
        let status = main_status(frame)?;
        ensure(
            status.phase == StatusPhase::Waiting,
            "idle tail left Waiting status",
        )?;
        ensure(
            status.id_exclusions == *required_id_exclusions,
            "idle tail exclusion set does not match the contract",
        )?;
        ensure(
            status.foreground_children == 0,
            "idle tail retained foreground work",
        )?;
        ensure(
            status.restart_count == 0 && status.recovery_count == 0 && status.bank_trips == 0,
            "idle tail restarted, recovered, or entered another bank trip",
        )?;
        if require_anchor {
            ensure(
                frame.position == expected_anchor,
                "idle tail left the exact anchor",
            )?;
        }
        ensure(
            !takes
                .iter()
                .any(|take| take.tick > after_tick && take.tick <= frame.tick),
            "idle tail contains a repeated Take send",
        )?;
        ensure(frame.bank_open != Some(true), "idle tail has an open bank")?;
    }
    Ok(())
}

fn first_visible_tick(frames: &[PlayerFrame], item_id: i32, tile: Tile) -> Result<u64, String> {
    frames
        .iter()
        .find(|frame| {
            frame
                .ground
                .as_ref()
                .is_some_and(|ground| ground.count(item_id, tile) > 0)
        })
        .map(|frame| frame.tick)
        .ok_or_else(|| {
            format!("item {item_id} at {tile:?} never appeared on a complete ground page")
        })
}

fn validate_unmodified_scratch_trace(
    run: &PlayerRunTrace,
    need_helper: bool,
    need_turns: bool,
    need_bank: bool,
) -> OracleResult {
    require_unmodified_scratch(&run.engine)?;
    require_client_settlement_clock(&run.frames, &run.takes)?;
    require_trace_authority(&run.authority, need_helper, need_turns, false, need_bank)?;
    require_setup_seed_action_authority(&run.authority)?;
    ensure(
        (1..=16).contains(&run.radius),
        "run radius is outside 1..=16",
    )?;
    validate_frame_sequence(&run.identity, &run.frames, need_turns, false)?;
    validate_pre_start_inventory(&run.pre_start_inventory, &run.identity, &run.frames)?;
    ensure(
        run.pre_start_inventory.inventory.occupied_slots == Some(0)
            && run.pre_start_inventory.inventory.slot_capacity == Some(28)
            && run
                .pre_start_inventory
                .inventory
                .quantities
                .values()
                .all(|quantity| *quantity == 0),
        "Engine A receiver pre-Start pack was not empty at 28-slot capacity",
    )?;
    validate_status_matches_observations(&run.frames, &run.takes)?;
    validate_take_ledger(
        &run.identity,
        run.radius,
        &run.selected_ids,
        &run.frames,
        &run.takes,
    )?;
    check_fresh_status(&run.frames)?;
    Ok(())
}

fn validate_setup_seed_action(
    action: &SetupSeedAction,
    expected_actor: &ActorRef,
    expected_inventory: &ItemSnapshot,
    pre_start: &PreStartInventoryObservation,
) -> OracleResult {
    ensure(
        action.actor == *expected_actor,
        "setup seed action targeted the wrong account",
    )?;
    ensure(
        action.observation_sequence < pre_start.observation_sequence,
        "setup seed action did not precede the receiver pre-Start observation",
    )?;
    ensure(
        action.inventory.complete,
        "setup seed inventory snapshot is incomplete",
    )?;
    ensure(
        action.inventory.occupied_slots.is_some() && action.inventory.slot_capacity.is_some(),
        "setup seed inventory snapshot lacks slot authority",
    )?;
    ensure(
        &action.inventory == expected_inventory,
        "setup seed action inventory does not match its observed account baseline",
    )
}

fn validate_setup_seed_actions(
    actions: &[SetupSeedAction],
    helper: &HelperSetup,
    identity: &RunIdentity,
    pre_start: &PreStartInventoryObservation,
    receiver_seed_expected: bool,
) -> OracleResult {
    let expected_count = if receiver_seed_expected { 2 } else { 1 };
    ensure(
        actions.len() == expected_count,
        "setup seed action count does not match the cell variant",
    )?;
    let mut helper_action = None;
    let mut receiver_action = None;
    for action in actions {
        if action.actor == helper.actor {
            ensure(
                helper_action.replace(action).is_none(),
                "helper inventory was seeded more than once",
            )?;
        } else if action.actor == identity.actor {
            ensure(
                receiver_action.replace(action).is_none(),
                "receiver inventory was seeded more than once",
            )?;
        } else {
            return Err("setup seed action targeted an unexpected account".into());
        }
    }
    let helper_action =
        helper_action.ok_or_else(|| "setup provenance lacks the helper seed action".to_string())?;
    validate_setup_seed_action(
        helper_action,
        &helper.actor,
        &helper.seeded_inventory,
        pre_start,
    )?;
    if receiver_seed_expected {
        let receiver_action = receiver_action
            .ok_or_else(|| "setup provenance lacks the receiver seed action".to_string())?;
        validate_setup_seed_action(
            receiver_action,
            &identity.actor,
            &pre_start.inventory,
            pre_start,
        )?;
    } else {
        ensure(
            receiver_action.is_none(),
            "Engine A player-drop setup seeded the receiver account",
        )?;
    }
    Ok(())
}

fn validate_receiver_seed_action(
    actions: &[SetupSeedAction],
    identity: &RunIdentity,
    pre_start: &PreStartInventoryObservation,
) -> OracleResult {
    ensure(
        actions.len() == 1,
        "inventory-blocked setup must contain exactly one receiver seed action",
    )?;
    validate_setup_seed_action(
        &actions[0],
        &identity.actor,
        &pre_start.inventory,
        pre_start,
    )
}

fn validate_helper_setup(
    helper: &HelperSetup,
    identity: &RunIdentity,
    pre_start: &PreStartInventoryObservation,
    setup_seed_actions: &[SetupSeedAction],
    receiver_seed_expected: bool,
    drops: &[DropAction],
    frames: &[PlayerFrame],
) -> OracleResult {
    ensure(
        helper.actor != identity.actor,
        "helper and Grinch share one account",
    )?;
    ensure(
        helper.seeded_before_grinch_start,
        "helper inventory was not seeded before Start",
    )?;
    ensure(
        helper.helper_seed_action_isolated_to_helper,
        "helper inventory seed action modified an account other than the helper",
    )?;
    ensure(
        !helper.target_spawn_cheat_used,
        "helper target was created through a spawn command",
    )?;
    ensure(
        helper.seeded_inventory.complete,
        "helper seed inventory is incomplete",
    )?;
    ensure(
        helper.seeded_inventory.occupied_slots.is_some()
            && helper.seeded_inventory.slot_capacity.is_some(),
        "helper seed inventory lacks slot authority",
    )?;
    validate_setup_seed_actions(
        setup_seed_actions,
        helper,
        identity,
        pre_start,
        receiver_seed_expected,
    )?;
    ensure(
        helper.post_drop_inventory.complete,
        "helper post-Drop inventory is incomplete",
    )?;
    ensure(
        helper.post_drop_inventory.occupied_slots == Some(0)
            && helper.post_drop_inventory.slot_capacity.is_some(),
        "helper post-Drop inventory does not confirm an empty pack",
    )?;
    ensure(!drops.is_empty(), "helper trace lacks real Drop actions")?;
    let mut expected = BTreeMap::new();
    let mut request_keys = HashSet::new();
    for drop in drops {
        ensure(
            drop.actor == helper.actor,
            "Drop action came from a non-helper actor",
        )?;
        ensure(
            drop.source == DropSource::RealPlayerInventoryDrop,
            "Drop was not a real player action",
        )?;
        ensure(drop.quantity == 1, "helper Drop quantity was not one")?;
        nonempty(&drop.request_key, "Drop action request key")?;
        ensure(
            request_keys.insert(drop.request_key.as_str()),
            "duplicate helper Drop request key",
        )?;
        *expected.entry(drop.item_id).or_insert(0_u32) += drop.quantity;
    }
    let seeded: BTreeMap<i32, u32> = helper
        .seeded_inventory
        .quantities
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(&id, &quantity)| (id, quantity))
        .collect();
    ensure(
        seeded == expected,
        "helper seed inventory does not exactly match real Drop actions",
    )?;
    let mut remaining = seeded;
    for drop in drops {
        let quantity = remaining
            .get_mut(&drop.item_id)
            .ok_or_else(|| "real Drop item is absent from helper inventory".to_string())?;
        ensure(
            *quantity >= drop.quantity,
            "helper Drop exceeds its observed inventory",
        )?;
        *quantity -= drop.quantity;
    }
    remaining.retain(|_, quantity| *quantity > 0);
    let post_drop: BTreeMap<i32, u32> = helper
        .post_drop_inventory
        .quantities
        .iter()
        .filter(|(_, quantity)| **quantity > 0)
        .map(|(&id, &quantity)| (id, quantity))
        .collect();
    ensure(
        post_drop == remaining,
        "helper post-Drop inventory does not confirm every real Drop decrease",
    )?;
    ensure(
        pre_start.observation_sequence < helper.post_drop_observation_sequence,
        "helper post-Drop confirmation predates the receiver pre-Start inventory",
    )?;
    let last_drop_tick = drops.iter().map(|drop| drop.tick).max().unwrap();
    ensure(
        helper.receiver_fence_tick > last_drop_tick,
        "receiver drain fence does not follow every helper Drop",
    )?;
    let fence = frame_at(frames, helper.receiver_fence_tick)?;
    ensure(
        !frames
            .iter()
            .any(|frame| frame.observation_sequence == helper.post_drop_observation_sequence),
        "helper confirmation observation ordinal collides with a receiver frame",
    )?;
    ensure(
        fence.observation_sequence > helper.post_drop_observation_sequence,
        "receiver drain fence does not follow the helper post-Drop observation",
    )?;
    ensure(
        fence.player_info_sequence.is_some(),
        "receiver drain fence lacks an observed PLAYER_INFO edge",
    )?;
    ensure(
        fence.object_turn.is_some(),
        "receiver drain fence lacks an observed object turn",
    )
}

fn validate_helper_drop_visibility(
    identity: &RunIdentity,
    frames: &[PlayerFrame],
    drops: &[DropAction],
) -> Result<Vec<u64>, String> {
    let mut reveal_ticks = Vec::with_capacity(drops.len());
    for drop in drops {
        ensure(
            drop.tick >= identity.start_tick,
            "helper Drop predates the Grinch run",
        )?;
        ensure(
            drop.tick < frames.last().unwrap().tick,
            "helper Drop has no later observed frame",
        )?;
        let at_drop = frame_at(frames, drop.tick)?;
        ensure(
            at_drop.object_turn == Some(drop.object_turn),
            "helper Drop object-turn value is not observed on its PLAYER_INFO frame",
        )?;
        ensure(
            require_ground(&at_drop.ground, "helper Drop frame")?.count(drop.item_id, drop.tile)
                == 0,
            "helper item was posted before public visibility",
        )?;
        let visible = frames
            .iter()
            .find(|frame| {
                frame.tick > drop.tick
                    && frame
                        .ground
                        .as_ref()
                        .is_some_and(|ground| ground.count(drop.item_id, drop.tile) > 0)
            })
            .ok_or_else(|| format!("helper item {} never became publicly visible", drop.item_id))?;
        ensure(
            require_ground(&visible.ground, "first public helper row")?
                .count(drop.item_id, drop.tile)
                == drop.quantity,
            "first public helper row quantity disagrees with the real Drop action",
        )?;
        let visible_turn = visible
            .object_turn
            .ok_or_else(|| "first public row lacks observed object-turn authority".to_string())?;
        ensure(
            visible_turn.saturating_sub(drop.object_turn) >= 100,
            format!(
                "helper item {} became visible before 100 object turns",
                drop.item_id
            ),
        )?;
        reveal_ticks.push(visible.tick);
    }
    Ok(reveal_ticks)
}

fn verify_drop_set(drops: &[DropAction], expected_ids: &BTreeSet<i32>) -> OracleResult {
    let actual: BTreeSet<i32> = drops.iter().map(|drop| drop.item_id).collect();
    ensure(
        actual == *expected_ids,
        "real helper Drop IDs do not match the cell contract",
    )?;
    ensure(
        drops.len() == expected_ids.len(),
        "helper Drop trace contains repeated or missing rows",
    )
}

fn expected_map<T: Copy>(ids: impl IntoIterator<Item = i32>, amount: T) -> BTreeMap<i32, T> {
    ids.into_iter().map(|id| (id, amount)).collect()
}

fn check_helper_positive(
    run: &PlayerRunTrace,
    helper: &HelperSetup,
    expected_ids: &BTreeSet<i32>,
) -> OracleResult {
    ensure(
        run.selected_ids == *expected_ids,
        "selected ID set does not match required positive set",
    )?;
    verify_drop_set(&run.drops, expected_ids)?;
    validate_helper_setup(
        helper,
        &run.identity,
        &run.pre_start_inventory,
        &run.setup_seed_actions,
        false,
        &run.drops,
        &run.frames,
    )?;
    let reveal_ticks = validate_helper_drop_visibility(&run.identity, &run.frames, &run.drops)?;
    for drop in &run.drops {
        ensure(
            drop.tile.within(run.identity.anchor, run.radius),
            "positive helper Drop is outside the anchor radius",
        )?;
    }
    let reveal_by_id: BTreeMap<i32, u64> = run
        .drops
        .iter()
        .zip(reveal_ticks)
        .map(|(drop, tick)| (drop.item_id, tick))
        .collect();
    ensure(
        reveal_by_id.len() == run.drops.len(),
        "positive helper Drops repeat an item ID",
    )?;
    let actual = exact_positive_delta_map(&run.frames)?;
    ensure(
        actual == expected_map(expected_ids.iter().copied(), 1),
        "observed landed inventory deltas are not exactly one per selected Drop",
    )?;
    ensure(
        run.takes.len() == expected_ids.len(),
        "accepted Take dispatch count does not match selected helper Drops",
    )?;
    match_successful_takes(&run.identity, &run.frames, &run.takes, &BTreeSet::new())?;
    for take in &run.takes {
        let visible_tick = reveal_by_id
            .get(&take.item_id)
            .ok_or_else(|| format!("Take {} has no matching helper reveal", take.request_key))?;
        ensure(
            take.tick >= *visible_tick,
            "Take dispatched before the row was publicly visible",
        )?;
    }
    let last_reveal = reveal_by_id
        .values()
        .copied()
        .max()
        .unwrap_or(run.identity.start_tick);
    let last_landing = positive_inventory_deltas(&run.frames)?
        .iter()
        .map(|(_, _, tick)| *tick)
        .max()
        .unwrap_or(run.identity.start_tick);
    ensure(
        last_landing.saturating_sub(last_reveal) <= 100,
        "last required helper Drop reveal was not collected within 100 observed ticks",
    )?;
    let last_drop_tick = run
        .drops
        .iter()
        .map(|drop| drop.tick)
        .max()
        .unwrap_or(run.identity.start_tick);
    ensure(
        run.frames
            .last()
            .unwrap()
            .tick
            .saturating_sub(last_drop_tick)
            <= 240,
        "helper Drop variant exceeded its 240-tick bound",
    )?;
    exact_final_delta(&run.frames, &expected_map(expected_ids.iter().copied(), 1))?;
    ensure_no_bank_trip(&run.frames)?;
    require_no_failures_or_exclusions(&run.frames)?;
    for frame in &run.frames {
        let status = main_status(frame)?;
        ensure(
            status.bank_trips == 0,
            "positive helper variant made a bank trip",
        )?;
        ensure(
            status.restart_count == 0 && status.recovery_count == 0,
            "positive helper variant restarted or recovered",
        )?;
    }
    check_idle_tail(
        &run.frames,
        last_landing,
        run.identity.anchor,
        true,
        40,
        &BTreeSet::new(),
        &run.takes,
    )
}

/// Checks cell A's 15 real tradeable Drops, exact-ID pickups, mask selection,
/// negative candidates, private untradeable invisibility, and idle tails.
pub fn check_cell_a(cell: &CellA) -> OracleResult {
    check_a_tradeable(&cell.tradeable)?;
    check_a_green_mask(&cell.green_mask)?;
    check_a_negative_candidates(&cell.negative_candidates)?;
    check_a_private_untradeable(&cell.private_untradeable)?;
    let runs = [
        &cell.tradeable.run.identity.run_id,
        &cell.green_mask.run.identity.run_id,
        &cell.negative_candidates.run.identity.run_id,
        &cell.private_untradeable.run.identity.run_id,
    ];
    let sessions = [
        &cell.tradeable.run.identity.session_id,
        &cell.green_mask.run.identity.session_id,
        &cell.negative_candidates.run.identity.session_id,
        &cell.private_untradeable.run.identity.session_id,
    ];
    ensure(
        sessions.iter().collect::<BTreeSet<_>>().len() == sessions.len(),
        "cell A reused a session identity across separate variants",
    )?;
    ensure(
        runs.iter().collect::<BTreeSet<_>>().len() == runs.len(),
        "cell A reused a run identity across separate variants",
    )
}

/// Checks the 15 selected, tradeable helper items.
pub fn check_a_tradeable(trace: &HelperDropTrace) -> OracleResult {
    let run = &trace.run;
    ensure(
        run.kind == CellKind::ATradeableDrops,
        "wrong cell kind for tradeable helper trace",
    )?;
    validate_unmodified_scratch_trace(run, true, true, false)?;
    let expected = set(TRADEABLE_IDS);
    check_helper_positive(run, &trace.helper, &expected)
}

/// Checks green-only mask selection while blue and red masks remain posted.
pub fn check_a_green_mask(trace: &HelperDropTrace) -> OracleResult {
    let run = &trace.run;
    ensure(
        run.kind == CellKind::AGreenMaskSelection,
        "wrong cell kind for mask trace",
    )?;
    validate_unmodified_scratch_trace(run, true, true, false)?;
    ensure(
        run.selected_ids == set([GREEN_MASK_ID]),
        "green-mask variant selected more than green 1053",
    )?;
    verify_drop_set(&run.drops, &set([GREEN_MASK_ID, BLUE_MASK_ID, RED_MASK_ID]))?;
    let shared_tile = run.drops[0].tile;
    ensure(
        run.drops.iter().all(|drop| drop.tile == shared_tile),
        "green, blue, and red masks were not dropped on one tile",
    )?;
    validate_helper_setup(
        &trace.helper,
        &run.identity,
        &run.pre_start_inventory,
        &run.setup_seed_actions,
        false,
        &run.drops,
        &run.frames,
    )?;
    let reveal_ticks = validate_helper_drop_visibility(&run.identity, &run.frames, &run.drops)?;
    ensure(
        run.drops
            .iter()
            .all(|drop| drop.tile.within(run.identity.anchor, run.radius)),
        "mask helper Drop was outside the anchor radius",
    )?;
    let reveal_by_id: BTreeMap<i32, u64> = run
        .drops
        .iter()
        .zip(reveal_ticks)
        .map(|(drop, tick)| (drop.item_id, tick))
        .collect();
    let actual = exact_positive_delta_map(&run.frames)?;
    ensure(
        actual == expected_map([GREEN_MASK_ID], 1),
        "mask pack delta was not green-only",
    )?;
    ensure(
        run.takes.len() == 1 && run.takes[0].item_id == GREEN_MASK_ID,
        "mask Take ledger was not green-only",
    )?;
    match_successful_takes(&run.identity, &run.frames, &run.takes, &BTreeSet::new())?;
    ensure(
        run.takes[0].tick >= reveal_by_id[&GREEN_MASK_ID],
        "green mask Take preceded its public reveal",
    )?;
    let blue_reveal = reveal_by_id[&BLUE_MASK_ID];
    let red_reveal = reveal_by_id[&RED_MASK_ID];
    for frame in &run.frames {
        let ground = require_ground(&frame.ground, "mask selection frame")?;
        if frame.tick >= blue_reveal {
            ensure(
                ground.count(BLUE_MASK_ID, shared_tile) == 1,
                "blue mask did not remain on its tile",
            )?;
        }
        if frame.tick >= red_reveal {
            ensure(
                ground.count(RED_MASK_ID, shared_tile) == 1,
                "red mask did not remain on its tile",
            )?;
        }
    }
    let last_drop = run.drops.iter().map(|drop| drop.tick).max().unwrap();
    ensure(
        run.frames.last().unwrap().tick - last_drop <= 240,
        "mask variant exceeded 240 ticks",
    )?;
    exact_final_delta(&run.frames, &expected_map([GREEN_MASK_ID], 1))?;
    ensure_no_bank_trip(&run.frames)?;
    require_no_failures_or_exclusions(&run.frames)?;
    check_idle_tail(
        &run.frames,
        run.takes[0].tick + MAX_TAKE_SETTLEMENT_TICKS,
        run.identity.anchor,
        true,
        40,
        &BTreeSet::new(),
        &run.takes,
    )
}

/// Checks ordinary unselected, certificate, and selected-but-outside-radius negatives.
pub fn check_a_negative_candidates(trace: &NegativeDropTrace) -> OracleResult {
    let run = &trace.run;
    ensure(
        run.kind == CellKind::ANegativeCandidates,
        "wrong cell kind for negative candidate trace",
    )?;
    validate_unmodified_scratch_trace(run, true, true, false)?;
    let selected = set(TRADEABLE_IDS);
    ensure(
        run.selected_ids == selected,
        "negative variant did not select the exact 15 tradeable IDs",
    )?;
    let wanted = BTreeSet::from([
        (
            NegativeCandidateKind::OrdinaryUnselected,
            ORDINARY_UNSELECTED_ID,
        ),
        (
            NegativeCandidateKind::Certificate,
            EASTER_EGG_CERTIFICATE_ID,
        ),
        (NegativeCandidateKind::SelectedOutsideRadius, PUMPKIN_ID),
    ]);
    let actual: BTreeSet<_> = trace
        .candidates
        .iter()
        .map(|candidate| (candidate.kind, candidate.item_id))
        .collect();
    ensure(
        actual == wanted && trace.candidates.len() == wanted.len(),
        "negative candidate identities are incomplete or duplicated",
    )?;
    verify_drop_set(
        &run.drops,
        &set([
            ORDINARY_UNSELECTED_ID,
            EASTER_EGG_CERTIFICATE_ID,
            PUMPKIN_ID,
        ]),
    )?;
    validate_helper_setup(
        &trace.helper,
        &run.identity,
        &run.pre_start_inventory,
        &run.setup_seed_actions,
        false,
        &run.drops,
        &run.frames,
    )?;
    validate_helper_drop_visibility(&run.identity, &run.frames, &run.drops)?;
    for candidate in &trace.candidates {
        match candidate.kind {
            NegativeCandidateKind::OrdinaryUnselected => {
                ensure(
                    !run.selected_ids.contains(&candidate.item_id),
                    "ordinary negative item is selected",
                )?;
                ensure(
                    candidate.tile.within(run.identity.anchor, run.radius),
                    "ordinary negative is not an in-radius control",
                )?;
            }
            NegativeCandidateKind::Certificate => {
                ensure(
                    candidate.item_id == EASTER_EGG_CERTIFICATE_ID,
                    "wrong certificate ID",
                )?;
                ensure(
                    candidate.tile.within(run.identity.anchor, run.radius),
                    "certificate negative is not an in-radius control",
                )?;
            }
            NegativeCandidateKind::SelectedOutsideRadius => {
                ensure(
                    run.selected_ids.contains(&candidate.item_id),
                    "outside-radius negative is not selected",
                )?;
                ensure(
                    !candidate.tile.within(run.identity.anchor, run.radius),
                    "outside-radius negative is inside the work area",
                )?;
            }
        }
        let drop = run
            .drops
            .iter()
            .find(|drop| drop.item_id == candidate.item_id)
            .ok_or_else(|| "candidate lacks its real helper Drop".to_string())?;
        ensure(
            drop.tile == candidate.tile,
            "candidate location disagrees with helper Drop",
        )?;
    }
    ensure(run.takes.is_empty(), "negative-only variant sent a Take")?;
    ensure(
        exact_positive_delta_map(&run.frames)?.is_empty(),
        "negative-only variant changed pack inventory",
    )?;
    ensure(
        last_status(&run.frames)?.pickup_units == 0,
        "negative-only variant reported a pickup count",
    )?;
    let reveal_ticks = run
        .drops
        .iter()
        .map(|drop| first_visible_tick(&run.frames, drop.item_id, drop.tile))
        .collect::<Result<Vec<_>, _>>()?;
    for (drop, reveal_tick) in run.drops.iter().zip(reveal_ticks) {
        for frame in run.frames.iter().filter(|frame| frame.tick >= reveal_tick) {
            ensure(
                require_ground(&frame.ground, "negative row retention")?
                    .count(drop.item_id, drop.tile)
                    == 1,
                format!("negative row {} disappeared or changed", drop.item_id),
            )?;
        }
    }
    let last_drop = run.drops.iter().map(|drop| drop.tick).max().unwrap();
    ensure(
        run.frames.last().unwrap().tick - last_drop <= 240,
        "negative variant exceeded 240 ticks",
    )?;
    ensure_no_bank_trip(&run.frames)?;
    require_no_failures_or_exclusions(&run.frames)?;
    check_idle_tail(
        &run.frames,
        last_drop,
        run.identity.anchor,
        true,
        40,
        &BTreeSet::new(),
        &run.takes,
    )
}

/// Checks the three private helper drops. They must stay absent after 100 object turns.
pub fn check_a_private_untradeable(trace: &HelperDropTrace) -> OracleResult {
    let run = &trace.run;
    ensure(
        run.kind == CellKind::APrivateUntradeableDrops,
        "wrong cell kind for private-drop trace",
    )?;
    ensure(
        run.takes.is_empty(),
        "private untradeable Drop produced a Take send",
    )?;
    validate_unmodified_scratch_trace(run, true, true, false)?;
    let expected = set(PRIVATE_UNTRADEABLE_IDS);
    ensure(
        run.selected_ids == expected,
        "private-drop variant did not select exactly the three untradeables",
    )?;
    verify_drop_set(&run.drops, &expected)?;
    validate_helper_setup(
        &trace.helper,
        &run.identity,
        &run.pre_start_inventory,
        &run.setup_seed_actions,
        false,
        &run.drops,
        &run.frames,
    )?;
    let fence_frame = frame_at(&run.frames, trace.helper.receiver_fence_tick)?;
    let fence_object_turn = fence_frame
        .object_turn
        .ok_or_else(|| "receiver drain fence lacks an object turn".to_string())?;
    for drop in &run.drops {
        ensure(
            drop.tile.within(run.identity.anchor, run.radius),
            "private helper Drop is outside the anchor radius",
        )?;
        ensure(
            frame_at(&run.frames, drop.tick)?.object_turn == Some(drop.object_turn),
            "private Drop object-turn value is not observed on its PLAYER_INFO frame",
        )?;
        ensure(
            drop.object_turn <= fence_object_turn,
            "private helper Drop occurred after the drained receiver fence",
        )?;
        for frame in run.frames.iter().filter(|frame| frame.tick >= drop.tick) {
            ensure(
                require_ground(&frame.ground, "private-drop ground page")?
                    .count(drop.item_id, drop.tile)
                    == 0,
                format!(
                    "private item {} appeared in Grinch's posted ground page",
                    drop.item_id
                ),
            )?;
        }
    }
    ensure(
        exact_positive_delta_map(&run.frames)?.is_empty(),
        "private untradeable Drop changed pack inventory",
    )?;
    ensure(
        last_status(&run.frames)?.pickup_units == 0,
        "private untradeable Drop produced a pickup count",
    )?;
    let last_drop = run.drops.iter().map(|drop| drop.tick).max().unwrap();
    ensure(
        run.frames.last().unwrap().tick.saturating_sub(last_drop) <= 160,
        "private-drop variant exceeded 160 ticks after the last Drop",
    )?;
    let fence_index = run
        .frames
        .iter()
        .position(|frame| frame.tick == trace.helper.receiver_fence_tick)
        .ok_or_else(|| "receiver drain fence is absent from the trace".to_string())?;
    let threshold_index = fence_index
        .checked_add(100)
        .ok_or_else(|| "private receiver-edge threshold overflowed".to_string())?;
    let threshold_frame = run.frames.get(threshold_index).ok_or_else(|| {
        "private trace lacks 100 distinct receiver edges after the drain fence".to_string()
    })?;
    let required_turn = fence_object_turn
        .checked_add(100)
        .ok_or_else(|| "private object-turn threshold overflowed".to_string())?;
    ensure(
        threshold_frame
            .object_turn
            .is_some_and(|turn| turn >= required_turn),
        "private trace lacks 100 observed object turns after the drained receiver fence",
    )?;
    ensure(
        run.frames.last().unwrap().tick >= threshold_frame.tick + 40,
        "private-drop trace lacks 40 additional healthy ticks after the post-confirmation threshold",
    )?;
    ensure_no_bank_trip(&run.frames)?;
    require_no_failures_or_exclusions(&run.frames)?;
    check_idle_tail(
        &run.frames,
        threshold_frame.tick,
        run.identity.anchor,
        true,
        40,
        &BTreeSet::new(),
        &run.takes,
    )
}

fn validate_scratch_authority(scratch: &ScratchAuthority) -> OracleResult {
    for (value, name) in [
        (&scratch.request_id, "scratch request identity"),
        (&scratch.scratch_id, "scratch identity"),
        (
            &scratch.source_engine_identity,
            "scratch source engine identity",
        ),
        (
            &scratch.source_content_identity,
            "scratch source content identity",
        ),
        (&scratch.engine_identity, "scratch engine identity"),
        (&scratch.content_identity, "scratch content identity"),
        (&scratch.world_identity, "scratch world identity"),
        (
            &scratch.account_store_identity,
            "scratch account-store identity",
        ),
    ] {
        nonempty(value, name)?;
    }
    ensure(
        scratch.server_tick_ms == 600,
        "scratch is not the real 600 ms engine",
    )?;
    ensure(
        scratch.members_mode,
        "scratch world is not configured with members mode",
    )?;
    ensure(
        scratch.copied_engine_content_map_inputs_and_account_storage,
        "scratch request did not copy engine, content, map inputs, and account storage",
    )?;
    ensure(
        scratch.respawn_phase_convention
            == RespawnPhaseConvention::FirstObjectTurnInRemovalCycleAndOutputAfterLifecycle,
        "scratch server-cycle and client-output phase convention is undeclared",
    )?;
    ensure(
        scratch.start_count == 1 && scratch.stop_count == 1 && scratch.process_tree_stopped,
        "scratch runner did not start once and stop its full process tree",
    )?;
    ensure(
        !scratch.engine_a_touched && !scratch.engine_q_touched,
        "scratch runner touched a protected engine",
    )?;
    ensure(
        scratch.ports.probed_free_before_start && scratch.ports.selected_before_login,
        "scratch endpoints were not tested and selected before login",
    )?;
    let endpoints = [
        &scratch.ports.game,
        &scratch.ports.web,
        &scratch.ports.management,
    ];
    let mut ports = BTreeSet::new();
    for endpoint in endpoints {
        ensure(
            endpoint.host == "127.0.0.1",
            "scratch endpoint is not loopback-only",
        )?;
        ensure(endpoint.port > 0, "scratch endpoint has an invalid port")?;
        ensure(
            ports.insert(endpoint.port),
            "scratch endpoints share a port",
        )?;
        ensure(
            ![44594, 1080, 9898, 44694, 1180, 9988].contains(&endpoint.port),
            "scratch endpoint borrowed a protected Engine A or Engine Q port",
        )?;
    }
    Ok(())
}
fn require_respawn_patch(scratch: &ScratchAuthority) -> Result<&ContentPatch, String> {
    let patch = scratch
        .content_patch
        .as_ref()
        .ok_or("static-respawn cell lacks its real content patch")?;
    ensure(
        is_sha256(&patch.sha256),
        "scratch content patch does not have a lowercase SHA-256 digest",
    )?;
    ensure(
        patch.preserved_existing_map_rows
            && patch.real_map_packer_used
            && patch.real_config_packer_used,
        "scratch content patch did not preserve rows and use both real packers",
    )?;
    let expected_rates: BTreeMap<i32, u32> = B_RESPAWN_ITEM_IDS
        .into_iter()
        .map(|item_id| (item_id, RAW_RESPAWN_RATE))
        .collect();
    ensure(
        patch.raw_respawn_rates == expected_rates,
        "scratch pack evidence lacks the four actual default rates decoded by ObjType",
    )?;
    ensure(
        patch.attested_pack_hashes_before.len() == ATTESTED_PACK_PATHS.len()
            && patch.attested_pack_hashes_after.len() == ATTESTED_PACK_PATHS.len(),
        "scratch map-only patch lacks hashes for every attested pack",
    )?;
    for path in ATTESTED_PACK_PATHS {
        let before = patch
            .attested_pack_hashes_before
            .get(path)
            .ok_or_else(|| format!("missing pre-patch hash for attested pack {path}"))?;
        let after = patch
            .attested_pack_hashes_after
            .get(path)
            .ok_or_else(|| format!("missing post-patch hash for attested pack {path}"))?;
        ensure(
            is_sha256(before) && is_sha256(after),
            format!("attested pack {path} has an invalid SHA-256"),
        )?;
        ensure(
            before == after,
            format!("map-only scratch patch changed attested pack {path}"),
        )?;
    }
    Ok(patch)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_scratch_trace(trace: &StaticSpawnTrace, require_competitor: bool) -> OracleResult {
    validate_scratch_authority(&trace.engine.scratch)?;
    require_respawn_patch(&trace.engine.scratch)?;
    require_trace_authority(&trace.authority, false, true, true, false)?;
    ensure(
        (1..=16).contains(&trace.radius),
        "scratch radius is outside 1..=16",
    )?;
    ensure(
        trace.identity.fresh_account,
        "scratch variant did not use a fresh account",
    )?;
    validate_frame_sequence(&trace.identity, &trace.frames, true, true)?;
    let mut observation_sequences = HashSet::new();
    for frame in trace.frames.iter().chain(&trace.competitor_frames) {
        ensure(
            observation_sequences.insert(frame.observation_sequence),
            "scratch clients reused a producer observation ordinal",
        )?;
    }
    let competitor_actor = trace
        .competitor
        .as_ref()
        .map(|competitor| &competitor.actor);
    let mut request_keys = HashSet::new();
    for take in &trace.takes {
        nonempty(&take.request_key, "scratch Take request key")?;
        ensure(
            request_keys.insert(take.request_key.as_str()),
            "scratch trace repeated a Take request key",
        )?;
        ensure(
            take.actor == trace.identity.actor || Some(&take.actor) == competitor_actor,
            "scratch Take belongs to an unrecorded actor",
        )?;
        ensure(
            take.server_cycle.is_some(),
            "scratch Take lacks an observed server cycle",
        )?;
    }
    let main_takes: Vec<TakeDispatch> = trace
        .takes
        .iter()
        .filter(|take| take.actor == trace.identity.actor)
        .cloned()
        .collect();
    validate_status_matches_observations(&trace.frames, &main_takes)?;
    validate_take_ledger(
        &trace.identity,
        trace.radius,
        &trace.selected_ids,
        &trace.frames,
        &main_takes,
    )?;
    check_fresh_status(&trace.frames)?;
    ensure_no_bank_trip(&trace.frames)?;
    if require_competitor {
        ensure(
            trace.competitor.is_some(),
            "contested cell lacks competitor identity",
        )?;
        ensure(
            !trace.competitor_frames.is_empty(),
            "contested cell lacks competitor observations",
        )?;
    } else {
        ensure(
            trace.competitor.is_none() && trace.competitor_frames.is_empty(),
            "single-player cell includes a competitor",
        )?;
    }
    Ok(())
}

fn validate_cycle_log(
    cycles: &[ServerCycle],
    expected_actors: &[ActorRef],
    competitor_tile: Option<Tile>,
) -> OracleResult {
    ensure(!cycles.is_empty(), "missing scratch server-cycle receipts")?;
    let expected_accounts: BTreeSet<String> = expected_actors
        .iter()
        .map(|actor| actor.account_id.clone())
        .collect();
    for (index, cycle) in cycles.iter().enumerate() {
        if index > 0 {
            ensure(
                cycle.cycle == cycles[index - 1].cycle + 1,
                "scratch cycle log has a missing or repeated server cycle",
            )?;
            ensure(
                cycle.object_turn == cycles[index - 1].object_turn + 1,
                "scratch object-turn log has a missing or repeated turn",
            )?;
        }
        ensure(
            cycle.phase_order
                == [
                    ServerPhase::PlayerProcessing,
                    ServerPhase::ObjectLifecycle,
                    ServerPhase::ClientOutput,
                ],
            "scratch cycle phase order is incomplete or contradictory",
        )?;
        let actors: BTreeMap<String, &ActiveActor> = cycle
            .active_actors
            .iter()
            .map(|presence| (presence.actor.account_id.clone(), presence))
            .collect();
        ensure(
            actors.len() == cycle.active_actors.len()
                && actors.keys().cloned().collect::<BTreeSet<_>>() == expected_accounts,
            "scratch actor population changed or contains duplicate accounts",
        )?;
        for actor in expected_actors {
            let presence = actors
                .get(&actor.account_id)
                .ok_or_else(|| "scratch cycle omitted an expected actor".to_string())?;
            ensure(presence.active, "scratch actor was not continuously active")?;
            ensure(presence.actor == *actor, "scratch actor identity changed")?;
            if Some(actor) != expected_actors.first() {
                if let Some(tile) = competitor_tile {
                    ensure(
                        presence.tile == tile,
                        "competitor left the contested respawn tile",
                    )?;
                }
            }
        }
        let order: BTreeSet<String> = cycle.player_processing_order.iter().cloned().collect();
        ensure(
            order.len() == cycle.player_processing_order.len() && order == expected_accounts,
            "server player-processing order does not cover the constant population",
        )?;
    }
    Ok(())
}

fn cycle(cycles: &[ServerCycle], cycle_id: u64) -> Result<&ServerCycle, String> {
    cycles
        .iter()
        .find(|sample| sample.cycle == cycle_id)
        .ok_or_else(|| format!("missing server-cycle receipt for cycle {cycle_id}"))
}

fn validate_frames_match_cycles(frames: &[PlayerFrame], cycles: &[ServerCycle]) -> OracleResult {
    let mut previous_cycle = None;
    for frame in frames {
        let id = frame
            .server_cycle
            .ok_or_else(|| format!("tick {} lacks server cycle", frame.tick))?;
        if let Some(previous) = previous_cycle {
            ensure(
                id > previous,
                "PLAYER_INFO output mapped to a repeated or stale server cycle",
            )?;
        }
        previous_cycle = Some(id);
        let sample = cycle(cycles, id)?;
        ensure(
            frame.object_turn == Some(sample.object_turn),
            format!(
                "tick {} object turn disagrees with its scratch cycle",
                frame.tick
            ),
        )?;
        ensure(
            frame.client_output_phase == Some(ClientOutputPhase::AfterObjectLifecycle),
            format!(
                "tick {} was not captured after object lifecycle",
                frame.tick
            ),
        )?;
    }
    Ok(())
}

fn verify_spawn_patch(
    trace: &StaticSpawnTrace,
    item_id: i32,
    expected: &[(Tile, u32)],
) -> OracleResult {
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    ensure(
        patch.raw_respawn_rates.get(&item_id) == Some(&RAW_RESPAWN_RATE),
        format!("scratch item {item_id} lacks its decoded default respawn rate of 100"),
    )?;
    let actual: BTreeMap<(i32, Tile), u32> = patch
        .static_spawns
        .iter()
        .map(|spawn| ((spawn.item_id, spawn.tile), spawn.quantity))
        .collect();
    ensure(
        actual.len() == patch.static_spawns.len(),
        "scratch patch repeats a static spawn row",
    )?;
    let expected: BTreeMap<(i32, Tile), u32> = expected
        .iter()
        .map(|(tile, quantity)| ((item_id, *tile), *quantity))
        .collect();
    ensure(
        actual == expected,
        "scratch static spawn rows do not match the cell contract",
    )
}

fn spawn_appearances(
    frames: &[PlayerFrame],
    item_id: i32,
    tile: Tile,
) -> Result<Vec<usize>, String> {
    let mut result = Vec::new();
    let mut prior = 0;
    for (index, frame) in frames.iter().enumerate() {
        let current =
            require_ground(&frame.ground, "static spawn observation")?.count(item_id, tile);
        ensure(
            current <= 1,
            "nonstackable static row has more than one identical object",
        )?;
        if current > 0 && prior == 0 {
            result.push(index);
        }
        prior = current;
    }
    Ok(result)
}

fn take_for_appearance<'a>(
    trace: &'a StaticSpawnTrace,
    item_id: i32,
    tile: Tile,
    fresh_tick: u64,
    next_fresh_tick: Option<u64>,
    actor: &ActorRef,
) -> Result<&'a TakeDispatch, String> {
    let matching: Vec<&TakeDispatch> = trace
        .takes
        .iter()
        .filter(|take| {
            take.actor == *actor
                && take.item_id == item_id
                && take.tile == tile
                && take.tick >= fresh_tick
                && next_fresh_tick.is_none_or(|next| take.tick < next)
        })
        .collect();
    ensure(
        matching.len() == 1,
        "fresh static copy lacks exactly one Take dispatch",
    )?;
    Ok(matching[0])
}

fn static_actor_frames<'a>(
    trace: &'a StaticSpawnTrace,
    actor: &ActorRef,
) -> Result<&'a [PlayerFrame], String> {
    if *actor == trace.identity.actor {
        Ok(&trace.frames)
    } else if trace
        .competitor
        .as_ref()
        .is_some_and(|competitor| competitor.actor == *actor)
    {
        Ok(&trace.competitor_frames)
    } else {
        Err("static Take has no observed actor".into())
    }
}

fn require_static_take_window(
    trace: &StaticSpawnTrace,
    item_id: i32,
    tile: Tile,
    fresh_frame: &PlayerFrame,
    take: &TakeDispatch,
    max_ticks: u64,
) -> OracleResult {
    ensure(
        fresh_frame.actor == take.actor,
        "static freshness frame and Take have different actors",
    )?;
    ensure(
        take.tick >= fresh_frame.tick && take.tick - fresh_frame.tick <= max_ticks,
        format!("item {item_id} Take was outside the fresh-row {max_ticks}-tick bound"),
    )?;
    ensure(
        take.server_cycle.is_some(),
        format!("item {item_id} Take lacks its server-cycle receipt"),
    )?;
    let take_cycle = take.server_cycle.unwrap();
    let cycle_sample = cycle(&trace.cycles, take_cycle)?;
    ensure(
        cycle_sample.phase_order[0] == ServerPhase::PlayerProcessing,
        "Take was not recorded in player-processing phase",
    )?;
    let frame = frame_at(static_actor_frames(trace, &take.actor)?, take.tick)?;
    ensure(
        require_ground(&frame.ground, "static Take frame")?.count(item_id, tile) > 0,
        "static Take was not backed by the exact fresh ground row",
    )
}

fn require_respawn_gap(
    trace: &StaticSpawnTrace,
    p: u32,
    previous_take: &TakeDispatch,
    fresh_frame: &PlayerFrame,
) -> OracleResult {
    let scaled_turns = ((4000_u64 - u64::from(p)) * u64::from(RAW_RESPAWN_RATE)) / 4000;
    let expected_cycle_gap = scaled_turns.saturating_sub(1);
    ensure(
        scaled_turns == 99 && expected_cycle_gap == PUMPKIN_RESPAWN_CYCLE_GAP,
        format!(
            "decoded default rate does not scale to 99 lifecycle turns and 98 removal-to-respawn cycles for P={p}"
        ),
    )?;
    let take_cycle = previous_take
        .server_cycle
        .ok_or_else(|| "previous Take lacks server cycle".to_string())?;
    let actor_frames = static_actor_frames(trace, &previous_take.actor)?;
    let settlement_deadline =
        frame_at(actor_frames, previous_take.tick + MAX_TAKE_SETTLEMENT_TICKS)?
            .server_cycle
            .ok_or("Take's PLAYER_INFO settlement deadline lacks server cycle")?;
    let same_row = |receipt: &&ObjectLifecycleReceipt| {
        receipt.item_id == previous_take.item_id && receipt.tile == previous_take.tile
    };
    let mut removals = trace
        .removals
        .iter()
        .filter(same_row)
        .filter(|receipt| take_cycle <= receipt.cycle && receipt.cycle <= settlement_deadline);
    let removal = removals
        .next()
        .ok_or("missing removal receipt for previous Take")?;
    ensure(
        removals.next().is_none(),
        "ambiguous removal receipts for previous Take",
    )?;
    cycle(&trace.cycles, removal.cycle)?;
    let fresh_cycle = fresh_frame
        .server_cycle
        .ok_or_else(|| "fresh row lacks server cycle".to_string())?;
    let previous_frame = frame_at(
        &trace.frames,
        fresh_frame
            .tick
            .checked_sub(1)
            .ok_or("respawn lacks a preceding PLAYER_INFO")?,
    )?;
    ensure(
        require_ground(&previous_frame.ground, "pre-respawn frame")?
            .count(previous_take.item_id, previous_take.tile)
            == 0,
        "respawn was not the first fresh ground row",
    )?;
    let previous_cycle = previous_frame
        .server_cycle
        .ok_or("pre-respawn PLAYER_INFO lacks server cycle")?;
    let mut respawns = trace
        .respawns
        .iter()
        .filter(same_row)
        .filter(|receipt| previous_cycle <= receipt.cycle && receipt.cycle <= fresh_cycle);
    let respawn = respawns
        .next()
        .ok_or("missing respawn receipt for first fresh row")?;
    ensure(
        respawns.next().is_none(),
        "ambiguous respawn receipts for first fresh row",
    )?;
    let sample = cycle(&trace.cycles, respawn.cycle)?;
    ensure(
        fresh_frame.client_output_phase == Some(ClientOutputPhase::AfterObjectLifecycle)
            && sample.phase_order[2] == ServerPhase::ClientOutput,
        "fresh row did not use the declared post-lifecycle client-output phase",
    )?;
    ensure(
        respawn.cycle.checked_sub(removal.cycle) == Some(expected_cycle_gap),
        format!(
            "respawn lifecycle gap was {}, expected exactly {expected_cycle_gap} server cycles",
            respawn.cycle.saturating_sub(removal.cycle)
        ),
    )
}

fn validate_cycle_coverage(
    trace: &StaticSpawnTrace,
    expected_actors: &[ActorRef],
    competitor_tile: Option<Tile>,
) -> OracleResult {
    validate_cycle_log(&trace.cycles, expected_actors, competitor_tile)?;
    validate_frames_match_cycles(&trace.frames, &trace.cycles)?;
    if !trace.competitor_frames.is_empty() {
        validate_frames_match_cycles(&trace.competitor_frames, &trace.cycles)?;
    }
    let first_cycle = trace.frames[0].server_cycle.unwrap();
    let last_cycle = trace.frames.last().unwrap().server_cycle.unwrap();
    ensure(
        trace.cycles.first().unwrap().cycle <= first_cycle
            && trace.cycles.last().unwrap().cycle >= last_cycle,
        "server-cycle authority does not cover the complete observed cell",
    )
}

fn current_frame_idle(
    frames: &[PlayerFrame],
    item_id: i32,
    tiles: &[Tile],
    takes: &[TakeDispatch],
) -> OracleResult {
    for frame in frames {
        let has_target = tiles.iter().any(|tile| {
            frame
                .ground
                .as_ref()
                .is_some_and(|ground| ground.count(item_id, *tile) > 0)
        });
        let in_take_settlement = takes.iter().any(|take| {
            (take.tick..=take.tick.saturating_add(MAX_TAKE_SETTLEMENT_TICKS))
                .contains(&frame.tick)
        });
        if !has_target && !in_take_settlement {
            ensure(
                main_status(frame)?.phase == StatusPhase::Waiting,
                "Grinch did not remain in Waiting when no in-range static target was visible",
            )?;
            ensure(
                main_status(frame)?.foreground_children == 0,
                "Grinch retained foreground work when no in-range static target was visible",
            )?;
        }
    }
    Ok(())
}

fn require_no_recovery_or_restart(frames: &[PlayerFrame]) -> OracleResult {
    for frame in frames {
        let status = main_status(frame)?;
        ensure(
            status.recovery_count == 0 && status.restart_count == 0,
            "scratch respawn cell recovered or restarted",
        )?;
    }
    Ok(())
}

fn check_a_b_scratch_kind(trace: &StaticSpawnTrace, kind: CellKind, item_id: i32) -> OracleResult {
    ensure(
        trace.kind == kind,
        "wrong cell kind for scratch static spawn",
    )?;
    validate_scratch_trace(trace, false)?;
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    ensure(
        trace.selected_ids == set([item_id]),
        "static variant did not select exactly its tested item",
    )?;
    ensure(
        patch.static_spawns.len() == 1,
        "static untradeable variant must have one public map row",
    )?;
    let spawn = &patch.static_spawns[0];
    ensure(
        spawn.item_id == item_id && spawn.quantity == 1,
        "static untradeable map row is wrong",
    )?;
    ensure(
        spawn.tile.within(trace.identity.anchor, trace.radius),
        "static untradeable row is outside the radius",
    )?;
    verify_spawn_patch(trace, item_id, &[(spawn.tile, 1)])?;
    ensure(
        trace
            .cycles
            .iter()
            .all(|sample| sample.active_actors.len() == 1),
        "single-account static variant did not keep population P=1",
    )?;
    validate_cycle_coverage(trace, std::slice::from_ref(&trace.identity.actor), None)?;
    let appearances = spawn_appearances(&trace.frames, item_id, spawn.tile)?;
    ensure(
        appearances.len() >= 2,
        "static untradeable did not appear initially and respawn",
    )?;
    Ok(())
}

/// Checks the P=1 two-pumpkin, three-appearance cell and its 1,080-tick bound.
pub fn check_b_pumpkin_respawns(trace: &StaticSpawnTrace) -> OracleResult {
    ensure(
        trace.kind == CellKind::BPumpkinRespawns,
        "wrong cell kind for pumpkin respawn trace",
    )?;
    validate_scratch_trace(trace, false)?;
    require_no_recovery_or_restart(&trace.frames)?;
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    ensure(
        trace.selected_ids == set([PUMPKIN_ID]),
        "pumpkin cell did not select only 1959",
    )?;
    let in_range: Vec<&StaticSpawn> = patch
        .static_spawns
        .iter()
        .filter(|spawn| {
            spawn.item_id == PUMPKIN_ID && spawn.tile.within(trace.identity.anchor, trace.radius)
        })
        .collect();
    let outside: Vec<&StaticSpawn> = patch
        .static_spawns
        .iter()
        .filter(|spawn| {
            spawn.item_id == PUMPKIN_ID && !spawn.tile.within(trace.identity.anchor, trace.radius)
        })
        .collect();
    ensure(
        in_range.len() == 2 && outside.len() == 1,
        "scratch map must contain two in-radius and one outside-radius pumpkin",
    )?;
    ensure(
        in_range.iter().all(|spawn| spawn.quantity == 1)
            && outside[0].quantity == 1
            && in_range
                .iter()
                .map(|spawn| spawn.tile)
                .collect::<BTreeSet<_>>()
                .len()
                == 2,
        "pumpkin spawn rows are not distinct count-one objects",
    )?;
    ensure(
        in_range
            .iter()
            .any(|spawn| spawn.tile == trace.identity.anchor)
            && in_range.iter().any(|spawn| {
                spawn.tile != trace.identity.anchor
                    && spawn.tile.distance(trace.identity.anchor) <= 1
            }),
        "in-range pumpkin rows are not at the anchor and a nearby walkable tile",
    )?;
    ensure(
        outside[0].tile.plane == trace.identity.anchor.plane
            && outside[0].tile.distance(trace.identity.anchor) > i32::from(trace.radius),
        "outside-radius pumpkin is not a same-plane radius negative",
    )?;
    let expected_spawns: Vec<(Tile, u32)> = patch
        .static_spawns
        .iter()
        .map(|spawn| (spawn.tile, spawn.quantity))
        .collect();
    verify_spawn_patch(trace, PUMPKIN_ID, &expected_spawns)?;
    ensure(
        trace
            .cycles
            .iter()
            .all(|sample| sample.active_actors.len() == 1),
        "P=1 pumpkin cell changed active population",
    )?;
    validate_cycle_coverage(trace, std::slice::from_ref(&trace.identity.actor), None)?;
    let allowed_unlanded = BTreeSet::new();
    for spawn in &in_range {
        let appearance_indexes = spawn_appearances(&trace.frames, PUMPKIN_ID, spawn.tile)?;
        ensure(
            appearance_indexes.len() == 3,
            "each in-range pumpkin must appear initially and respawn twice",
        )?;
        let appearances: Vec<&PlayerFrame> = appearance_indexes
            .iter()
            .map(|index| &trace.frames[*index])
            .collect();
        let mut previous_take = None;
        for (ordinal, appearance) in appearances.iter().enumerate() {
            let next_tick = appearances.get(ordinal + 1).map(|next| next.tick);
            let take = take_for_appearance(
                trace,
                PUMPKIN_ID,
                spawn.tile,
                appearance.tick,
                next_tick,
                &trace.identity.actor,
            )?;
            require_static_take_window(trace, PUMPKIN_ID, spawn.tile, appearance, take, 20)?;
            if let Some(previous) = previous_take {
                require_respawn_gap(trace, 1, previous, appearance)?;
            }
            previous_take = Some(take);
        }
    }
    ensure(
        trace.takes.len() == 6,
        "two static pumpkins require six fresh exact-ID Takes",
    )?;
    ensure(
        trace.takes.iter().all(|take| {
            take.item_id == PUMPKIN_ID && in_range.iter().any(|spawn| spawn.tile == take.tile)
        }),
        "pumpkin trace took an outside-radius or wrong-ID row",
    )?;
    ensure(
        trace
            .frames
            .last()
            .unwrap()
            .tick
            .saturating_sub(trace.identity.start_tick)
            <= 1080,
        "P=1 pumpkin cell exceeded 1,080 observed ticks after Start",
    )?;
    let expected = BTreeMap::from([(PUMPKIN_ID, 6)]);
    ensure(
        exact_positive_delta_map(&trace.frames)? == expected,
        "P=1 pumpkin pack deltas are not exactly six landed units",
    )?;
    match_successful_takes(
        &trace.identity,
        &trace.frames,
        &trace.takes,
        &allowed_unlanded,
    )?;
    ensure(
        last_status(&trace.frames)?.pickup_units == 6,
        "P=1 pumpkin pickup count is not six",
    )?;
    for frame in &trace.frames {
        ensure(
            require_ground(&frame.ground, "outside-radius pumpkin control")?
                .count(PUMPKIN_ID, outside[0].tile)
                == 1,
            "outside-radius pumpkin disappeared or changed",
        )?;
    }
    current_frame_idle(
        &trace.frames,
        PUMPKIN_ID,
        &in_range.iter().map(|spawn| spawn.tile).collect::<Vec<_>>(),
        &trace.takes,
    )?;
    require_no_failures_or_exclusions(&trace.frames)?;
    ensure_no_bank_trip(&trace.frames)?;
    Ok(())
}

/// Checks three competitor pack wins, then two Grinch wins at the same row.
pub fn check_b_contested_pumpkin(trace: &StaticSpawnTrace) -> OracleResult {
    ensure(
        trace.kind == CellKind::BContestedPumpkin,
        "wrong cell kind for contested pumpkin trace",
    )?;
    validate_scratch_trace(trace, true)?;
    require_no_recovery_or_restart(&trace.frames)?;
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    let competitor = trace.competitor.as_ref().unwrap();
    ensure(
        competitor.fresh_account && competitor.active_before_grinch_start,
        "competitor was not fresh and active before Grinch Start",
    )?;
    ensure(
        competitor.actor != trace.identity.actor,
        "race participants share one account",
    )?;
    ensure(
        competitor.session_id != trace.identity.session_id,
        "race participants share a session identity",
    )?;
    ensure(
        trace.selected_ids == set([PUMPKIN_ID]),
        "race cell did not select only pumpkin 1959",
    )?;
    ensure(
        patch.static_spawns.len() == 1,
        "race cell must have one static pumpkin row",
    )?;
    let spawn = &patch.static_spawns[0];
    ensure(
        spawn.item_id == PUMPKIN_ID
            && spawn.quantity == 1
            && spawn.tile.within(trace.identity.anchor, trace.radius),
        "contested pumpkin row is not one in-radius static copy",
    )?;
    verify_spawn_patch(trace, PUMPKIN_ID, &[(spawn.tile, 1)])?;
    validate_participant_frame_sequence(competitor, &trace.competitor_frames, true, true)?;
    validate_cycle_coverage(
        trace,
        &[trace.identity.actor.clone(), competitor.actor.clone()],
        Some(spawn.tile),
    )?;
    let appearances = spawn_appearances(&trace.frames, PUMPKIN_ID, spawn.tile)?;
    ensure(
        appearances.len() == 5,
        "contested pumpkin must have five distinct appearances",
    )?;
    let competitor_appearances =
        spawn_appearances(&trace.competitor_frames, PUMPKIN_ID, spawn.tile)?;
    ensure(
        competitor_appearances.len() >= 3,
        "competitor did not observe three fresh race appearances",
    )?;
    ensure(
        trace
            .frames
            .last()
            .unwrap()
            .tick
            .saturating_sub(trace.identity.start_tick)
            <= 2120,
        "contested pumpkin cell exceeded 2,120 observed ticks after Start",
    )?;
    let grinch_takes: Vec<&TakeDispatch> = trace
        .takes
        .iter()
        .filter(|take| take.actor == trace.identity.actor)
        .collect();
    let competitor_takes: Vec<&TakeDispatch> = trace
        .takes
        .iter()
        .filter(|take| take.actor == competitor.actor)
        .collect();
    ensure(
        grinch_takes.len() == 5,
        "Grinch did not send one fresh Take for each of five identical-key copies",
    )?;
    ensure(
        competitor_takes.len() == 3,
        "competitor did not stop after its first three Takes",
    )?;
    let mut unlanded = BTreeSet::new();
    let mut previous_winner = None;
    for (index, frame_index) in appearances.iter().enumerate() {
        let fresh = &trace.frames[*frame_index];
        let next_tick = appearances
            .get(index + 1)
            .map(|next| trace.frames[*next].tick);
        let grinch_take = take_for_appearance(
            trace,
            PUMPKIN_ID,
            spawn.tile,
            fresh.tick,
            next_tick,
            &trace.identity.actor,
        )?;
        require_static_take_window(trace, PUMPKIN_ID, spawn.tile, fresh, grinch_take, 20)?;
        let winner = if index < 3 {
            let competitor_fresh = &trace.competitor_frames[competitor_appearances[index]];
            let next_competitor_tick = competitor_appearances
                .get(index + 1)
                .map(|next| trace.competitor_frames[*next].tick);
            let competitor_take = take_for_appearance(
                trace,
                PUMPKIN_ID,
                spawn.tile,
                competitor_fresh.tick,
                next_competitor_tick,
                &competitor.actor,
            )?;
            require_static_take_window(
                trace,
                PUMPKIN_ID,
                spawn.tile,
                competitor_fresh,
                competitor_take,
                20,
            )?;
            ensure(
                grinch_take.server_cycle == competitor_take.server_cycle,
                "race Takes were not dispatched in the same server cycle",
            )?;
            let sample = cycle(&trace.cycles, grinch_take.server_cycle.unwrap())?;
            let comp_pos = sample
                .player_processing_order
                .iter()
                .position(|account| account == &competitor.actor.account_id)
                .ok_or_else(|| "competitor is absent from processing order".to_string())?;
            let grinch_pos = sample
                .player_processing_order
                .iter()
                .position(|account| account == &trace.identity.actor.account_id)
                .ok_or_else(|| "Grinch is absent from processing order".to_string())?;
            ensure(
                comp_pos < grinch_pos,
                "competitor did not process before Grinch in the three required races",
            )?;
            unlanded.insert(grinch_take.request_key.clone());
            let comp_frame = frame_at(
                &trace.competitor_frames,
                competitor_take.tick + MAX_TAKE_SETTLEMENT_TICKS,
            )?;
            let grinch_frame =
                frame_at(&trace.frames, grinch_take.tick + MAX_TAKE_SETTLEMENT_TICKS)?;
            ensure(
                count(&comp_frame.inventory, PUMPKIN_ID)? == index as u32 + 1,
                "competitor pack did not increase within four own PLAYER_INFO frames",
            )?;
            ensure(
                count(&grinch_frame.inventory, PUMPKIN_ID)? == 0,
                "Grinch pack changed during a competitor win",
            )?;
            competitor_take
        } else {
            grinch_take
        };
        if let Some(previous) = previous_winner {
            require_respawn_gap(trace, 2, previous, fresh)?;
        }
        previous_winner = Some(winner);
    }
    let grinch_expected = BTreeMap::from([(PUMPKIN_ID, 2)]);
    let competitor_expected = BTreeMap::from([(PUMPKIN_ID, 3)]);
    ensure(
        exact_positive_delta_map(&trace.frames)? == grinch_expected,
        "Grinch race result is not two landed pumpkins",
    )?;
    ensure(
        exact_positive_delta_map(&trace.competitor_frames)? == competitor_expected,
        "competitor race result is not three landed pumpkins",
    )?;
    match_successful_takes(
        &trace.identity,
        &trace.frames,
        &trace
            .takes
            .iter()
            .filter(|take| take.actor == trace.identity.actor)
            .cloned()
            .collect::<Vec<_>>(),
        &unlanded,
    )?;
    let comp_take_list: Vec<TakeDispatch> = competitor_takes.into_iter().cloned().collect();
    match_competitor_picks(&trace.competitor_frames, &comp_take_list)?;
    ensure(
        last_status(&trace.frames)?.pickup_units == 2,
        "race native pickup count is not two",
    )?;
    ensure(
        last_status(&trace.frames)?.take_sends == 5,
        "race native Take-send count is not five",
    )?;
    require_no_failures_or_exclusions(&trace.frames)?;
    ensure_no_bank_trip(&trace.frames)?;
    Ok(())
}

fn match_competitor_picks(frames: &[PlayerFrame], takes: &[TakeDispatch]) -> OracleResult {
    let mut used = BTreeSet::new();
    for (item_id, amount, tick) in positive_inventory_deltas(frames)? {
        ensure(amount == 1, "competitor pack delta is not one item")?;
        let candidates: Vec<&TakeDispatch> = takes
            .iter()
            .filter(|take| {
                take.item_id == item_id
                    && take.tick <= tick
                    && tick - take.tick <= MAX_TAKE_SETTLEMENT_TICKS
                    && !used.contains(&take.request_key)
            })
            .collect();
        ensure(
            candidates.len() == 1,
            "competitor inventory increase lacks its accepted exact-ID Take",
        )?;
        used.insert(candidates[0].request_key.clone());
    }
    ensure(
        used.len() == takes.len(),
        "competitor Take did not win a pack delta",
    )
}

/// Checks one real public Scythe or Bunny ears pickup, refusal, exclusion, and idle tail.
pub fn check_b_duplicate_untradeable(trace: &StaticSpawnTrace, item_id: i32) -> OracleResult {
    let kind = match item_id {
        SCYTHE_ID => CellKind::BScytheDuplicate,
        BUNNY_EARS_ID => CellKind::BBunnyEarsDuplicate,
        _ => return Err("duplicate checker accepts only Scythe or Bunny ears".into()),
    };
    ensure(
        trace.kind == kind,
        "wrong cell kind for public duplicate trace",
    )?;
    check_a_b_scratch_kind(trace, kind, item_id)?;
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    ensure(
        trace.takes.len() == 2,
        "duplicate variant requires one initial and one refused Take",
    )?;
    ensure(
        trace.refusals.len() == 1,
        "duplicate variant requires exactly one content refusal",
    )?;
    let refusal = &trace.refusals[0];
    ensure(
        !trace.takes.iter().any(|take| take.tick > refusal.tick),
        "Grinch sent another Take after duplicate ID exclusion",
    )?;
    let spawn = &patch.static_spawns[0];
    let appearances = spawn_appearances(&trace.frames, item_id, spawn.tile)?;
    ensure(
        appearances.len() == 2,
        "duplicate item did not respawn exactly once",
    )?;
    let first = &trace.frames[appearances[0]];
    let second = &trace.frames[appearances[1]];
    let first_take = take_for_appearance(
        trace,
        item_id,
        spawn.tile,
        first.tick,
        Some(second.tick),
        &trace.identity.actor,
    )?;
    let refused_take = take_for_appearance(
        trace,
        item_id,
        spawn.tile,
        second.tick,
        None,
        &trace.identity.actor,
    )?;
    require_static_take_window(trace, item_id, spawn.tile, first, first_take, 20)?;
    require_respawn_gap(trace, 1, first_take, second)?;
    require_static_take_window(trace, item_id, spawn.tile, second, refused_take, 20)?;
    let expected_refusal = if item_id == SCYTHE_ID {
        RefusalKind::ScytheAlreadyOwned
    } else {
        RefusalKind::BunnyEarsAlreadyOwned
    };
    ensure(
        refusal.request_key == refused_take.request_key
            && refusal.item_id == item_id
            && refusal.tile == spawn.tile
            && refusal.kind == expected_refusal,
        "second-copy refusal lacks the exact typed content refusal receipt",
    )?;
    ensure(
        refusal.tick >= refused_take.tick
            && refusal.tick - refused_take.tick <= MAX_TAKE_SETTLEMENT_TICKS,
        "duplicate refusal was not observed within the Take settlement fence",
    )?;
    ensure(
        refusal.tick <= trace.identity.start_tick + 560,
        "duplicate refusal exceeded 560 observed ticks",
    )?;
    ensure(
        exact_positive_delta_map(&trace.frames)? == BTreeMap::from([(item_id, 1)]),
        "duplicate variant did not land exactly one item",
    )?;
    match_successful_takes(
        &trace.identity,
        &trace.frames,
        &trace.takes,
        &BTreeSet::from([refused_take.request_key.clone()]),
    )?;
    ensure(
        count(&trace.frames.last().unwrap().inventory, item_id)? == 1,
        "duplicate refusal changed the carried copy count",
    )?;
    ensure(
        trace.frames.last().unwrap().tick <= trace.identity.start_tick + 600,
        "duplicate variant exceeded 600 observed ticks",
    )?;
    for frame in trace
        .frames
        .iter()
        .filter(|frame| frame.tick < refusal.tick)
    {
        let status = main_status(frame)?;
        ensure(
            status.id_exclusions.is_empty(),
            "duplicate ID exclusion appeared before refusal",
        )?;
        ensure(
            status.duplicate_refusals == 0,
            "duplicate refusal counter advanced before refusal",
        )?;
    }
    for frame in trace
        .frames
        .iter()
        .filter(|frame| frame.tick >= refusal.tick)
    {
        ensure(
            require_ground(&frame.ground, "refused duplicate row")?.count(item_id, spawn.tile) == 1,
            "refused duplicate row did not remain unchanged",
        )?;
        ensure(
            main_status(frame)?.id_exclusions == BTreeSet::from([item_id]),
            "duplicate item ID exclusion is absent or incorrect",
        )?;
        ensure(
            main_status(frame)?.phase == StatusPhase::Waiting,
            "duplicate exclusion did not return to Waiting",
        )?;
    }
    ensure(
        last_status(&trace.frames)?.duplicate_refusals == 1,
        "native duplicate refusal count is not one",
    )?;
    require_no_failures_or_exclusions_except_id(&trace.frames, item_id)?;
    ensure_no_bank_trip(&trace.frames)?;
    check_idle_tail(
        &trace.frames,
        refusal.tick,
        trace.identity.anchor,
        false,
        40,
        &BTreeSet::from([item_id]),
        &trace.takes,
    )
}

fn require_no_failures_or_exclusions_except_id(
    frames: &[PlayerFrame],
    item_id: i32,
) -> OracleResult {
    for frame in frames {
        let status = main_status(frame)?;
        ensure(
            status.failed_key_attempts == 0
                && status.avoidance_entries == 0
                && status.failed_key_exclusions.is_empty()
                && (status.id_exclusions.is_empty()
                    || status.id_exclusions == BTreeSet::from([item_id])),
            "duplicate variant contains an unrelated failure, avoidance, or exclusion",
        )?;
    }
    Ok(())
}

/// Checks two public Yo-yo pickups with no duplicate refusal or exclusion.
pub fn check_b_yoyo_repeat(trace: &StaticSpawnTrace) -> OracleResult {
    ensure(
        trace.kind == CellKind::BYoYoRepeat,
        "wrong cell kind for Yo-yo trace",
    )?;
    check_a_b_scratch_kind(trace, CellKind::BYoYoRepeat, YO_YO_ID)?;
    let patch = require_respawn_patch(&trace.engine.scratch)?;
    ensure(
        trace.refusals.is_empty(),
        "Yo-yo trace invented a duplicate refusal",
    )?;
    ensure(
        trace.takes.len() == 2,
        "Yo-yo must have one fresh Take per appearance",
    )?;
    let spawn = &patch.static_spawns[0];
    let appearances = spawn_appearances(&trace.frames, YO_YO_ID, spawn.tile)?;
    ensure(
        appearances.len() == 2,
        "Yo-yo must have one initial row and one respawn",
    )?;
    let first = &trace.frames[appearances[0]];
    let second = &trace.frames[appearances[1]];
    let first_take = take_for_appearance(
        trace,
        YO_YO_ID,
        spawn.tile,
        first.tick,
        Some(second.tick),
        &trace.identity.actor,
    )?;
    let second_take = take_for_appearance(
        trace,
        YO_YO_ID,
        spawn.tile,
        second.tick,
        None,
        &trace.identity.actor,
    )?;
    require_static_take_window(trace, YO_YO_ID, spawn.tile, first, first_take, 20)?;
    require_static_take_window(trace, YO_YO_ID, spawn.tile, second, second_take, 20)?;
    require_respawn_gap(trace, 1, first_take, second)?;
    ensure(
        trace.frames.last().unwrap().tick <= trace.identity.start_tick + 560,
        "Yo-yo repeat pickup exceeded 560 observed ticks",
    )?;
    ensure(
        exact_positive_delta_map(&trace.frames)? == BTreeMap::from([(YO_YO_ID, 2)]),
        "Yo-yo did not land exactly twice",
    )?;
    match_successful_takes(
        &trace.identity,
        &trace.frames,
        &trace.takes,
        &BTreeSet::new(),
    )?;
    ensure(
        last_status(&trace.frames)?.pickup_units == 2,
        "Yo-yo pickup counter is not two",
    )?;
    ensure(
        trace
            .frames
            .iter()
            .all(|frame| main_status(frame).unwrap().id_exclusions.is_empty()),
        "Yo-yo acquired an invented ID exclusion",
    )?;
    require_no_failures_or_exclusions(&trace.frames)?;
    ensure_no_bank_trip(&trace.frames)
}

/// Checks all five B variants. Each variant has its own callable checker above.
pub fn check_cell_b(cell: &CellB) -> OracleResult {
    check_b_pumpkin_respawns(&cell.pumpkin_respawns)?;
    check_b_contested_pumpkin(&cell.contested_pumpkin)?;
    check_b_duplicate_untradeable(&cell.scythe_duplicate, SCYTHE_ID)?;
    check_b_duplicate_untradeable(&cell.bunny_ears_duplicate, BUNNY_EARS_ID)?;
    check_b_yoyo_repeat(&cell.yo_yo_repeat)?;
    let traces = [
        &cell.pumpkin_respawns,
        &cell.contested_pumpkin,
        &cell.scythe_duplicate,
        &cell.bunny_ears_duplicate,
        &cell.yo_yo_repeat,
    ];
    let mut request_ids = BTreeSet::new();
    let mut scratch_ids = BTreeSet::new();
    let mut run_ids = BTreeSet::new();
    let mut session_ids = BTreeSet::new();
    for trace in &traces {
        let scratch = &trace.engine.scratch;
        ensure(
            request_ids.insert(scratch.request_id.as_str()),
            "scratch variants reused a request identity",
        )?;
        ensure(
            scratch_ids.insert(scratch.scratch_id.as_str()),
            "scratch variants reused a scratch identity",
        )?;
        ensure(
            run_ids.insert(trace.identity.run_id.as_str()),
            "scratch variants reused a run identity",
        )?;
        ensure(
            session_ids.insert(trace.identity.session_id.as_str()),
            "scratch variants reused a session identity",
        )?;
    }
    ensure(
        request_ids.len() == traces.len(),
        "scratch variants reused a request identity",
    )
}

/// Checks the 28- and 27-pumpkin selected-only bank trips and full-pack stop.
pub fn check_cell_c(cell: &CellC) -> OracleResult {
    check_c_deposit_28(&cell.deposit_28)?;
    check_c_deposit_27_preserve_item(&cell.deposit_27_preserve_item)?;
    check_c_inventory_blocked(&cell.inventory_blocked)?;
    let run_ids = [
        cell.deposit_28.identity.run_id.as_str(),
        cell.deposit_27_preserve_item.identity.run_id.as_str(),
        cell.inventory_blocked.identity.run_id.as_str(),
    ];
    let session_ids = [
        cell.deposit_28.identity.session_id.as_str(),
        cell.deposit_27_preserve_item.identity.session_id.as_str(),
        cell.inventory_blocked.identity.session_id.as_str(),
    ];
    ensure(
        run_ids.iter().collect::<BTreeSet<_>>().len() == run_ids.len(),
        "cell C reused a run identity across variants",
    )?;
    ensure(
        session_ids.iter().collect::<BTreeSet<_>>().len() == session_ids.len(),
        "cell C reused a session identity across variants",
    )
}

/// Checks the selected-only 28-pumpkin deposit, exact return, and later pickup.
pub fn check_c_deposit_28(trace: &BankFlowTrace) -> OracleResult {
    check_c_bank_flow(trace, CellKind::CDeposit28, 27, None, 28)
}

/// Checks the 27-pumpkin deposit while preserving the nonselected carried item.
pub fn check_c_deposit_27_preserve_item(trace: &BankFlowTrace) -> OracleResult {
    check_c_bank_flow(
        trace,
        CellKind::CDeposit27PreserveItem,
        26,
        Some(required_carried_id(trace)?),
        27,
    )
}

fn required_carried_id(trace: &BankFlowTrace) -> Result<i32, String> {
    trace
        .carried_item_id
        .ok_or_else(|| "27-pumpkin variant lacks its nonselected carried item ID".to_string())
}

fn check_bank_packet_order(
    proof: &BankPacketOrderProof,
    action: &BankAction,
    open: &BankOpenReceipt,
) -> OracleResult {
    let modal = proof
        .modal
        .as_ref()
        .ok_or("missing bank interface-open packet receipt")?;
    let table = proof
        .table
        .as_ref()
        .ok_or("missing bank table packet receipt")?;
    let deposit = proof
        .deposit
        .as_ref()
        .ok_or("missing accepted bank deposit packet receipt")?;
    let earlier_ambiguous_table = table.player_info_changed
        && table.ordinal.is_none()
        && !modal.player_info_changed
        && table.callback_sequence < modal.callback_sequence
        && table.packet_observation < modal.packet_observation;
    let ambiguous_batches = [modal, table]
        .into_iter()
        .filter(|packet| packet.player_info_changed)
        .map(|packet| packet.callback_sequence)
        .collect::<BTreeSet<_>>()
        .len() as u64;
    ensure(
        proof.ambiguous_ui_batches == ambiguous_batches
            && !modal.player_info_changed
            && (!table.player_info_changed || earlier_ambiguous_table),
        "bank UI packet arrival order is ambiguous across a PLAYER_INFO edge",
    )?;
    let modal_order = modal
        .ordinal
        .ok_or("missing bank interface-open packet arrival ordinal")?;
    let table_order = if earlier_ambiguous_table {
        None
    } else {
        Some(
            table
                .ordinal
                .ok_or("missing bank table packet arrival ordinal")?,
        )
    };
    let deposit_order = deposit
        .ordinal
        .ok_or("missing accepted bank deposit packet arrival ordinal")?;
    ensure(
        modal.session_generation == deposit.session_generation
            && modal.arrival_bank_generation == deposit.bank_generation,
        "bank interface-open packet belongs to another session or bank generation",
    )?;
    ensure(
        table.session_generation == deposit.session_generation,
        "bank table packet belongs to another session or bank generation",
    )?;
    for (packet, name) in [(modal, "interface-open"), (table, "table")] {
        ensure(
            packet.packet_observation != 0 && packet.callback_sequence != 0,
            format!("bank {name} packet lacks an actual callback and packet-order stamp"),
        )?;
        ensure(
            packet.callback_sequence <= deposit.callback_sequence,
            format!("deposit callback had not observed the bank {name} packet"),
        )?;
    }
    let binding = proof
        .table_binding
        .as_ref()
        .ok_or("missing first-loaded bank table generation binding")?;
    ensure(
        binding.session_generation == deposit.session_generation
            && binding.bound_to_generation == deposit.bank_generation
            && table.arrival_bank_generation != 0
            && table.arrival_bank_generation <= binding.bound_to_generation
            && binding.packet_observation == table.packet_observation
            && table.callback_sequence <= binding.callback_sequence
            && modal.callback_sequence <= binding.callback_sequence
            && binding.callback_sequence <= deposit.callback_sequence,
        "bank table binding does not prove its exact first-loaded generation",
    )?;
    ensure(
        binding
            .bank_snapshot
            .as_ref()
            .is_some_and(|bank| bank.complete),
        "bound bank table lacks its complete generation snapshot",
    )?;
    ensure(
        deposit.observed_table_stamp <= table.packet_observation,
        "newer bank table packet arrived before the accepted deposit",
    )?;
    ensure(
        deposit.session_generation != 0
            && deposit.bank_generation != 0
            && deposit.callback_sequence != 0
            && deposit.observed_modal_stamp == modal.packet_observation
            && deposit.observed_table_stamp == table.packet_observation
            && deposit.host_log_ordinal_fence < deposit.click_host_log_ordinal,
        "deposit callback had not already observed both packets in its bank generation",
    )?;
    ensure(
        modal_order.position != 0
            && table_order.is_none_or(|order| order.position != 0)
            && deposit_order.position != 0,
        "bank packet arrival ordinal has no captured-stream position",
    )?;
    ensure(
        modal.packet_observation != table.packet_observation
            && table_order.is_none_or(|order| {
                (modal.packet_observation < table.packet_observation) == (modal_order < order)
            }),
        "bank UI ordinals contradict their actual shared packet-order stamps",
    )?;
    ensure(
        deposit.request_key == action.request_key && deposit_order.tick == action.tick,
        "accepted deposit packet receipt does not match its exact action",
    )?;
    ensure(
        open.tick == modal_order.max(table_order.unwrap_or(modal_order)).tick,
        "bank-open receipt was rounded away from its actual packet arrival",
    )?;
    ensure(
        deposit_order > modal_order,
        "deposit packet did not strictly follow bank interface-open packet",
    )?;
    ensure(
        table_order.is_none_or(|order| deposit_order > order),
        "deposit packet did not strictly follow bank table packet",
    )
}

fn check_c_bank_flow(
    trace: &BankFlowTrace,
    kind: CellKind,
    starting_pumpkins: u32,
    carried_item: Option<i32>,
    deposit_amount: u32,
) -> OracleResult {
    ensure(trace.kind == kind, "wrong cell kind for bank-flow trace")?;
    require_unmodified_scratch(&trace.engine)?;
    require_client_settlement_clock(&trace.frames, &trace.takes)?;
    require_trace_authority(&trace.authority, true, true, false, true)?;
    require_setup_seed_action_authority(&trace.authority)?;
    ensure(
        trace.radius > 0 && trace.radius <= 16,
        "bank-flow radius is outside 1..=16",
    )?;
    ensure(
        trace.selected_ids == set([PUMPKIN_ID]),
        "bank flow did not select only pumpkins",
    )?;
    validate_frame_sequence(&trace.identity, &trace.frames, true, false)?;
    validate_pre_start_inventory(&trace.pre_start_inventory, &trace.identity, &trace.frames)?;
    validate_status_matches_observations(&trace.frames, &trace.takes)?;
    validate_take_ledger(
        &trace.identity,
        trace.radius,
        &trace.selected_ids,
        &trace.frames,
        &trace.takes,
    )?;
    check_fresh_status(&trace.frames)?;
    validate_helper_setup(
        &trace.helper,
        &trace.identity,
        &trace.pre_start_inventory,
        &trace.setup_seed_actions,
        true,
        &trace.drops,
        &trace.frames,
    )?;
    ensure(
        trace.drops.len() == 2,
        "bank flow requires two real helper Drops",
    )?;
    ensure(
        trace
            .drops
            .iter()
            .all(|drop| drop.item_id == PUMPKIN_ID && drop.quantity == 1),
        "bank flow helper did not drop two single pumpkins",
    )?;
    ensure(
        trace
            .drops
            .iter()
            .all(|drop| drop.tile.within(trace.identity.anchor, trace.radius)),
        "bank flow helper Drop left the anchor radius",
    )?;
    let reveal_ticks =
        validate_helper_drop_visibility(&trace.identity, &trace.frames, &trace.drops)?;
    let first = require_inventory(&trace.frames[0].inventory, "bank-flow baseline")?;
    ensure(
        first.count(PUMPKIN_ID) == starting_pumpkins,
        "bank-flow starting pumpkin count is wrong",
    )?;
    match carried_item {
        Some(id) => {
            ensure(
                id != PUMPKIN_ID && first.count(id) == 1,
                "nonselected carried item baseline is wrong",
            )?;
            ensure(
                first
                    .quantities
                    .iter()
                    .filter(|(item, qty)| **qty > 0 && **item != PUMPKIN_ID)
                    .count()
                    == 1,
                "27-pumpkin repeat did not carry exactly one nonselected item",
            )?;
        }
        None => ensure(
            first
                .quantities
                .iter()
                .all(|(id, qty)| *id == PUMPKIN_ID || *qty == 0),
            "28-pumpkin variant has an unexpected nonselected carried item",
        )?,
    }
    ensure(
        trace.drops[0].tick < trace.drops[1].tick,
        "bank-flow helper Drops are not ordered around the return",
    )?;
    let successful = match_successful_takes(
        &trace.identity,
        &trace.frames,
        &trace.takes,
        &BTreeSet::new(),
    )?;
    ensure(
        successful.len() == 2,
        "bank flow did not land both helper pumpkins",
    )?;
    let first_pick_tick = positive_inventory_deltas(&trace.frames)?
        .first()
        .map(|(_, _, tick)| *tick)
        .ok_or_else(|| "bank flow has no first pickup inventory delta".to_string())?;
    let later_pick_tick = positive_inventory_deltas(&trace.frames)?
        .last()
        .map(|(_, _, tick)| *tick)
        .ok_or_else(|| "bank flow has no subsequent pickup inventory delta".to_string())?;
    ensure(
        later_pick_tick > first_pick_tick && later_pick_tick.saturating_sub(first_pick_tick) <= 600,
        "bank return and subsequent pickup exceeded the 600-tick budget",
    )?;
    ensure(
        trace.route.nearest_routable
            && trace.route.ordinary_accessible_bank
            && trace.route.exact_route_proven
            && !trace.route.used_air_distance_fallback
            && trace.route.walking_steps <= 16,
        "bank route was not the nearest proven ordinary bank within 16 walking steps",
    )?;
    nonempty(&trace.route.bank_id, "chosen bank identity")?;
    ensure(
        trace.drops[1].tick
            > trace
                .receipts
                .return_to_anchor
                .as_ref()
                .map(|receipt| receipt.tick)
                .unwrap_or(0),
        "subsequent helper Drop did not occur after exact-anchor return",
    )?;
    let initial_bank = trace
        .bank_before
        .as_ref()
        .ok_or_else(|| "missing observable bank baseline".to_string())?;
    let final_bank = trace
        .bank_after
        .as_ref()
        .ok_or_else(|| "missing final bank snapshot".to_string())?;
    ensure(
        initial_bank.complete && final_bank.complete,
        "bank snapshot is incomplete",
    )?;
    let mut ids: BTreeSet<i32> = initial_bank
        .quantities
        .keys()
        .chain(final_bank.quantities.keys())
        .copied()
        .collect();
    ids.insert(PUMPKIN_ID);
    for id in ids {
        let before = i64::from(initial_bank.count(id));
        let after = i64::from(final_bank.count(id));
        let expected = if id == PUMPKIN_ID {
            i64::from(deposit_amount)
        } else {
            0
        };
        ensure(
            after - before == expected,
            format!(
                "bank delta for item {id} is {}, expected {expected}",
                after - before
            ),
        )?;
    }
    let open = trace
        .receipts
        .open
        .as_ref()
        .ok_or_else(|| "missing observed bank-open receipt".to_string())?;
    let close = trace
        .receipts
        .close
        .as_ref()
        .ok_or_else(|| "missing observed bank-close receipt".to_string())?;
    let returned = trace
        .receipts
        .return_to_anchor
        .as_ref()
        .ok_or_else(|| "missing exact-anchor return receipt".to_string())?;
    nonempty(&open.request_key, "bank-open request key")?;
    nonempty(&close.request_key, "bank-close request key")?;
    nonempty(&returned.request_key, "anchor-return request key")?;
    ensure(
        open.bank_id == trace.route.bank_id && open.interface_open && open.bank_loaded,
        "bank-open receipt lacks the chosen loaded interface",
    )?;
    ensure(
        close.bank_id == trace.route.bank_id && close.interface_closed,
        "bank-close receipt is absent or contradictory",
    )?;
    ensure(
        returned.tile == trace.identity.anchor && returned.arrival_receipt_observed,
        "return did not prove arrival at the exact anchor",
    )?;
    let mut ordered_takes: Vec<&TakeDispatch> = trace.takes.iter().collect();
    ordered_takes.sort_by_key(|take| take.tick);
    ensure(
        ordered_takes[0].tick >= reveal_ticks[0]
            && ordered_takes[1].tick >= reveal_ticks[1]
            && ordered_takes[0].tick < open.tick
            && ordered_takes[1].tick > returned.tick
            && later_pick_tick > returned.tick,
        "bank flow Takes do not follow the two public reveals and bank return",
    )?;
    ensure(
        first_pick_tick < open.tick,
        "bank opened before the first full-pack pickup",
    )?;
    ensure(
        open.tick < close.tick && close.tick < returned.tick && returned.tick < trace.drops[1].tick,
        "bank open, close, exact-anchor return, and subsequent Drop are out of order",
    )?;
    let mut action_keys = HashSet::new();
    for action in &trace.bank_actions {
        nonempty(&action.request_key, "bank action request key")?;
        ensure(
            action_keys.insert(action.request_key.as_str()),
            "bank action request key was repeated",
        )?;
    }
    let matching_open = trace
        .bank_actions
        .iter()
        .filter(|action| {
            action.request_key == open.request_key
                && action.tick == open.tick
                && action.kind == BankActionKind::Open
        })
        .count();
    let matching_close = trace
        .bank_actions
        .iter()
        .filter(|action| {
            action.request_key == close.request_key
                && action.tick == close.tick
                && action.kind == BankActionKind::Close
        })
        .count();
    let deposits: Vec<&BankAction> = trace
        .bank_actions
        .iter()
        .filter(|action| matches!(&action.kind, BankActionKind::Deposit { .. }))
        .collect();
    ensure(
        matching_open == 1 && matching_close == 1,
        "bank receipts do not match one open and close action",
    )?;
    ensure(
        deposits.len() == 1,
        "bank flow did not issue exactly one deposit action",
    )?;
    match &deposits[0].kind {
        BankActionKind::Deposit { item_id, quantity } => ensure(
            *item_id == PUMPKIN_ID && *quantity == deposit_amount,
            "bank action was not an exact selected-pumpkin deposit",
        )?,
        _ => return Err("bank flow action is not a deposit".into()),
    }
    check_bank_packet_order(&trace.packet_order, deposits[0], open)?;
    ensure(
        deposits[0].tick < close.tick,
        "deposit action is outside the observed bank-open/close interval",
    )?;
    ensure(
        trace.bank_actions.len() == 3,
        "bank flow contains a withdrawal, repeated trip, or unaccounted bank action",
    )?;
    let before_deposit_frame = frame_at(&trace.frames, deposits[0].tick)?;
    let full_pack = require_inventory(&before_deposit_frame.inventory, "full bank-flow pack")?;
    ensure(
        full_pack.count(PUMPKIN_ID) == deposit_amount
            && full_pack.slot_capacity == Some(28)
            && full_pack.occupied_slots == Some(28),
        "pack was not full with exactly the selected pumpkin deposit",
    )?;
    let bank_complete_tick = trace
        .frames
        .iter()
        .find(|frame| {
            frame.tick >= deposits[0].tick && count(&frame.inventory, PUMPKIN_ID).ok() == Some(0)
        })
        .map(|frame| frame.tick)
        .ok_or_else(|| "no observed pack decrease confirms the selected deposit".to_string())?;
    ensure(
        bank_complete_tick <= close.tick,
        "bank close preceded the observed pack decrease",
    )?;
    for frame in &trace.frames {
        let inventory = require_inventory(&frame.inventory, "bank-flow pack observation")?;
        let expected_pumpkins = if frame.tick < first_pick_tick {
            starting_pumpkins
        } else if frame.tick < bank_complete_tick {
            deposit_amount
        } else if frame.tick < later_pick_tick {
            0
        } else {
            1
        };
        ensure(
            inventory.count(PUMPKIN_ID) == expected_pumpkins,
            "bank-flow pumpkin count diverged from pickup and deposit evidence",
        )?;
        let carried_slots = carried_item.is_some() as u16;
        let expected_slots = if frame.tick < first_pick_tick {
            27
        } else if frame.tick < bank_complete_tick {
            28
        } else if frame.tick < later_pick_tick {
            carried_slots
        } else {
            carried_slots + 1
        };
        ensure(
            inventory.slot_capacity == Some(28) && inventory.occupied_slots == Some(expected_slots),
            "bank-flow occupied-slot count diverged from the observed pack lifecycle",
        )?;
        if let Some(item_id) = carried_item {
            ensure(
                inventory.count(item_id) == 1,
                "nonselected item was not preserved across the bank flow",
            )?;
        }
        ensure(
            inventory.quantities.iter().all(|(item, quantity)| {
                *quantity == 0 || *item == PUMPKIN_ID || Some(*item) == carried_item
            }),
            "bank-flow pack acquired an unaccounted item",
        )?;
    }
    let after_bank = frame_at(&trace.frames, bank_complete_tick)?;
    ensure(
        count(&after_bank.inventory, PUMPKIN_ID)? == 0,
        "selected pumpkin pack decrease does not match the deposit",
    )?;
    if let Some(id) = carried_item {
        ensure(
            count(&after_bank.inventory, id)? == 1,
            "bank trip removed the nonselected carried item",
        )?;
    }
    let returned_frame = frame_at(&trace.frames, returned.tick)?;
    ensure(
        returned_frame.position == trace.identity.anchor,
        "return receipt disagrees with observed exact-anchor tile",
    )?;
    ensure(
        trace
            .packet_order
            .table_binding
            .as_ref()
            .and_then(|binding| binding.bank_snapshot.as_ref())
            .is_some_and(|bank| bank.complete && bank.quantities == initial_bank.quantities),
        "bank-open table packet does not match the observed bank baseline",
    )?;
    ensure(
        trace.frames.iter().any(|frame| {
            frame.bank_open == Some(true)
                && frame.bank_loaded == Some(true)
                && frame
                    .bank
                    .as_ref()
                    .is_some_and(|bank| bank.complete && bank.quantities == final_bank.quantities)
        }),
        "loaded bank view does not match the final observed bank snapshot",
    )?;
    ensure(
        trace
            .frames
            .iter()
            .any(|frame| frame.tick == close.tick && frame.bank_open == Some(false)),
        "bank-close receipt lacks matching observed closed interface",
    )?;
    let status = last_status(&trace.frames)?;
    ensure(
        status.banked_units == deposit_amount,
        "native banked counter disagrees with actual bank delta",
    )?;
    ensure(
        status.bank_trips == 1,
        "native status did not record exactly one bank trip",
    )?;
    ensure(
        status.pickup_units == 2,
        "bank flow native pickup counter is not two",
    )?;
    ensure(
        trace
            .frames
            .iter()
            .all(|frame| main_status(frame).unwrap().bank_trips <= 1),
        "bank flow repeated the bank trip",
    )?;
    Ok(())
}

/// Checks the full nonselected pack's inventory-blocked stop without a bank trip.
pub fn check_c_inventory_blocked(trace: &InventoryBlockedTrace) -> OracleResult {
    ensure(
        trace.kind == CellKind::CInventoryBlocked,
        "wrong cell kind for inventory-blocked trace",
    )?;
    require_unmodified_scratch(&trace.engine)?;
    require_client_settlement_clock(&trace.frames, &[])?;
    require_trace_authority(&trace.authority, false, false, false, true)?;
    require_setup_seed_action_authority(&trace.authority)?;
    ensure(
        trace.selected_ids == set([PUMPKIN_ID]),
        "inventory-blocked run did not select only pumpkins",
    )?;
    validate_frame_sequence(&trace.identity, &trace.frames, false, false)?;
    validate_pre_start_inventory(&trace.pre_start_inventory, &trace.identity, &trace.frames)?;
    validate_receiver_seed_action(
        &trace.setup_seed_actions,
        &trace.identity,
        &trace.pre_start_inventory,
    )?;
    validate_status_matches_observations(&trace.frames, &[])?;
    check_fresh_status(&trace.frames)?;
    ensure(
        trace.frames.len() >= 2,
        "inventory-blocked state lacks a later terminal observation",
    )?;
    let terminal = &trace.terminal_stop;
    ensure(
        terminal.run_slot > 0
            && terminal.run_number > 0
            && terminal.native_session > 0
            && trace.identity.run_id
                == format!(
                    "account:{}:slot:{}:run:{}",
                    trace.identity.actor.account_id, terminal.run_slot, terminal.run_number
                )
            && terminal.card == "Grinch"
            && terminal.native_phase == "Blocked"
            && terminal.failure_code == "inventory-blocked"
            && terminal.failure_message
                == "inventory is full and contains no selected item to deposit",
        "inventory-blocked stop lacks its exact retained native diagnostic",
    )?;
    ensure(
        terminal.generation_before_start.checked_add(1) == Some(terminal.generation_after_stop)
            && terminal.active_run_key_after_stop.is_none()
            && terminal.status_read_observation_sequence >= trace.frames[0].observation_sequence
            && terminal.status_read_observation_sequence
                <= trace.frames.last().unwrap().observation_sequence,
        "inventory-blocked stop lacks its observed Start generation and retired active key",
    )?;
    let initial = require_inventory(&trace.frames[0].inventory, "full pack baseline")?;
    ensure(
        initial.slot_capacity == Some(28) && initial.occupied_slots == Some(28),
        "inventory-blocked baseline is not a full 28-slot pack",
    )?;
    ensure(
        initial.count(PUMPKIN_ID) == 0,
        "full inventory contains a selected pumpkin",
    )?;
    for (index, frame) in trace.frames.iter().enumerate() {
        let inventory = require_inventory(&frame.inventory, "inventory-blocked observation")?;
        ensure(
            inventory.count(PUMPKIN_ID) == 0,
            "selected item appeared in inventory-blocked run",
        )?;
        ensure(
            inventory.slot_capacity == Some(28)
                && inventory.occupied_slots == Some(28)
                && inventory.quantities == initial.quantities,
            "inventory-blocked run changed its full pack",
        )?;
        let status = main_status(frame)?;
        if index == 0 {
            ensure(
                status.phase == StatusPhase::InventoryBlocked,
                "full pack did not stop as inventory-blocked",
            )?;
        } else {
            ensure(
                status.phase == StatusPhase::InventoryBlocked,
                "inventory-blocked state did not remain terminal",
            )?;
        }
        ensure(
            status.blocked_reason == Some(BlockedReason::NoSelectedItemToDeposit),
            "inventory-blocked reason is not absence of a selected item",
        )?;
        ensure(
            status.bank_trips == 0 && status.take_sends == 0 && status.banked_units == 0,
            "inventory-blocked run attempted a Take or bank trip",
        )?;
        ensure(
            status.foreground_children == 0,
            "inventory-blocked state retained a child",
        )?;
    }
    ensure(
        trace.bank_actions.is_empty(),
        "inventory-blocked run issued a bank action",
    )?;
    require_no_failures_or_exclusions(&trace.frames)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(name: &str) -> ActorRef {
        ActorRef {
            account_id: format!("acct-{name}"),
            player_name: name.into(),
        }
    }

    fn unmodified_scratch() -> EngineEvidence {
        let mut scratch = scratch(0, Vec::new());
        scratch.content_identity = scratch.source_content_identity.clone();
        scratch.content_patch = None;
        EngineEvidence {
            scratch: Box::new(scratch),
        }
    }

    fn authority(helper: bool, turns: bool, cycles: bool, bank: bool) -> TraceAuthority {
        TraceAuthority {
            player_info_edges_complete: true,
            ground_pages_complete: true,
            inventory_snapshots_complete: true,
            native_status_complete: true,
            accepted_take_ledger_complete: true,
            helper_drop_actions_complete: helper,
            setup_seed_actions_complete: true,
            object_turns_complete: turns,
            server_cycles_complete: cycles,
            bank_action_log_complete: bank,
            bank_snapshots_complete: bank,
        }
    }

    fn item_snapshot(items: &[(i32, u32)], slots: u16, capacity: u16) -> ItemSnapshot {
        ItemSnapshot {
            complete: true,
            quantities: items.iter().copied().collect(),
            occupied_slots: Some(slots),
            slot_capacity: Some(capacity),
        }
    }
    fn pre_start_observation(
        identity: &RunIdentity,
        items: &[(i32, u32)],
        occupied_slots: u16,
    ) -> PreStartInventoryObservation {
        PreStartInventoryObservation {
            actor: identity.actor.clone(),
            session_id: identity.session_id.clone(),
            player_info_sequence: 0,
            observation_sequence: 3,
            inventory: item_snapshot(items, occupied_slots, 28),
        }
    }
    fn setup_seed_action(
        actor: &ActorRef,
        observation_sequence: u64,
        inventory: &ItemSnapshot,
    ) -> SetupSeedAction {
        SetupSeedAction {
            actor: actor.clone(),
            observation_sequence,
            inventory: inventory.clone(),
        }
    }

    fn bank_snapshot(items: &[(i32, u32)]) -> ItemSnapshot {
        ItemSnapshot {
            complete: true,
            quantities: items.iter().copied().collect(),
            occupied_slots: None,
            slot_capacity: None,
        }
    }

    fn status(identity: &RunIdentity) -> NativeStatus {
        NativeStatus {
            run_id: identity.run_id.clone(),
            session_id: identity.session_id.clone(),
            anchor: identity.anchor,
            phase: StatusPhase::Waiting,
            blocked_reason: None,
            pickup_units: 0,
            banked_units: 0,
            take_sends: 0,
            bank_trips: 0,
            failed_key_attempts: 0,
            avoidance_entries: 0,
            duplicate_refusals: 0,
            failed_key_exclusions: BTreeSet::new(),
            id_exclusions: BTreeSet::new(),
            recovery_count: 0,
            restart_count: 0,
            foreground_children: 0,
        }
    }

    fn frame(
        identity: &RunIdentity,
        tick: u64,
        items: &[(i32, u32)],
        ground_rows: &[(i32, Tile, u32)],
        mut native: NativeStatus,
        object_turn: u64,
    ) -> PlayerFrame {
        native.run_id = identity.run_id.clone();
        native.session_id = identity.session_id.clone();
        native.anchor = identity.anchor;
        PlayerFrame {
            tick,
            observation_sequence: tick.saturating_mul(2).saturating_add(4),
            player_info_sequence: Some(tick + 1),
            actor: identity.actor.clone(),
            session_id: Some(identity.session_id.clone()),
            position: identity.anchor,
            inventory: Some(item_snapshot(
                items,
                items.iter().map(|(_, quantity)| quantity).sum::<u32>() as u16,
                28,
            )),
            bank: None,
            bank_open: Some(false),
            bank_loaded: Some(false),
            worn: Some(item_snapshot(&[], 0, 28)),
            ground: Some(GroundSnapshot {
                state: GroundPageState::Complete,
                rows: ground_rows
                    .iter()
                    .map(|(item_id, tile, quantity)| GroundRow {
                        item_id: *item_id,
                        tile: *tile,
                        quantity: *quantity,
                    })
                    .collect(),
            }),
            status: Some(native),
            object_turn: Some(object_turn),
            server_cycle: None,
            client_output_phase: None,
        }
    }

    fn run_identity(name: &str, run: &str, start: u64, anchor: Tile) -> RunIdentity {
        RunIdentity {
            actor: actor(name),
            run_id: run.into(),
            session_id: format!("session-{run}"),
            anchor,
            start_tick: start,
            fresh_account: true,
        }
    }

    fn drop_action(helper: &ActorRef, id: i32, tile: Tile, tick: u64, turn: u64) -> DropAction {
        DropAction {
            request_key: format!("drop-{id}-{tick}"),
            actor: helper.clone(),
            item_id: id,
            tile,
            quantity: 1,
            tick,
            object_turn: turn,
            source: DropSource::RealPlayerInventoryDrop,
        }
    }

    fn take(identity: &RunIdentity, id: i32, tile: Tile, tick: u64) -> TakeDispatch {
        TakeDispatch {
            request_key: format!("take-{}-{tick}", id),
            actor: identity.actor.clone(),
            item_id: id,
            tile,
            tick,
            server_cycle: None,
        }
    }
    fn scratch_take(actor: &ActorRef, id: i32, tile: Tile, tick: u64, cycle: u64) -> TakeDispatch {
        TakeDispatch {
            request_key: format!("take-{}-{id}-{tick}", actor.account_id),
            actor: actor.clone(),
            item_id: id,
            tile,
            tick,
            server_cycle: Some(cycle),
        }
    }

    fn helper_setup(helper: &ActorRef, drops: &[DropAction]) -> HelperSetup {
        let mut quantities = BTreeMap::new();
        let mut slots = 0_u16;
        for drop in drops {
            *quantities.entry(drop.item_id).or_insert(0_u32) += drop.quantity;
            slots += drop.quantity as u16;
        }
        HelperSetup {
            actor: helper.clone(),
            seeded_inventory: ItemSnapshot {
                complete: true,
                quantities,
                occupied_slots: Some(slots),
                slot_capacity: Some(28),
            },
            post_drop_inventory: ItemSnapshot {
                complete: true,
                quantities: BTreeMap::new(),
                occupied_slots: Some(0),
                slot_capacity: Some(28),
            },
            post_drop_observation_sequence: drops
                .iter()
                .map(|drop| drop.tick)
                .max()
                .unwrap()
                .saturating_mul(2)
                .saturating_add(3),
            post_drop_player_info_sequence: drops.len() as u64,
            receiver_fence_tick: drops.iter().map(|drop| drop.tick).max().unwrap() + 1,
            seeded_before_grinch_start: true,
            helper_seed_action_isolated_to_helper: true,
            target_spawn_cheat_used: false,
        }
    }

    fn engine_a_run(
        kind: CellKind,
        identity: RunIdentity,
        selected: BTreeSet<i32>,
        drops: Vec<DropAction>,
        takes: Vec<TakeDispatch>,
        frames: Vec<PlayerFrame>,
    ) -> PlayerRunTrace {
        let helper = drops
            .first()
            .expect("fixture has helper drops")
            .actor
            .clone();
        let helper_setup = helper_setup(&helper, &drops);
        let pre_start_inventory = pre_start_observation(&identity, &[], 0);
        let setup_seed_actions = vec![setup_seed_action(
            &helper,
            1,
            &helper_setup.seeded_inventory,
        )];
        PlayerRunTrace {
            kind,
            engine: unmodified_scratch(),
            authority: authority(true, true, false, false),
            pre_start_inventory,
            setup_seed_actions,
            identity,
            radius: 12,
            selected_ids: selected,
            frames,
            takes,
            drops,
        }
    }

    #[test]
    fn first_working_observation_allows_one_child_but_keeps_clean_counters() {
        let mut trace = a_tradeable_fixture().run;
        let initial = trace.frames[0].status.as_mut().unwrap();
        initial.phase = StatusPhase::Working;
        initial.foreground_children = 1;
        check_fresh_status(&trace.frames).unwrap();
        validate_frame_sequence(&trace.identity, &trace.frames, true, false).unwrap();

        trace.frames[0].status.as_mut().unwrap().foreground_children = 2;
        assert_eq!(
            validate_frame_sequence(&trace.identity, &trace.frames, true, false).unwrap_err(),
            "native status retained more than one foreground child"
        );
        let initial = trace.frames[0].status.as_mut().unwrap();
        initial.foreground_children = 1;
        initial.pickup_units = 1;
        assert_eq!(
            check_fresh_status(&trace.frames).unwrap_err(),
            "run did not start with clean native counters and exclusions"
        );
    }

    fn a_tradeable_fixture() -> HelperDropTrace {
        let anchor = Tile {
            x: 3200,
            z: 3200,
            plane: 0,
        };
        let identity = run_identity("grinch-tradeable", "a-tradeable-run", 0, anchor);
        let helper = actor("helper-tradeable");
        let ids = set(TRADEABLE_IDS);
        let drops: Vec<DropAction> = TRADEABLE_IDS
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let tick = index as u64 + 1;
                drop_action(
                    &helper,
                    *id,
                    Tile {
                        x: anchor.x + (index as i32 % 3),
                        z: anchor.z,
                        plane: 0,
                    },
                    tick,
                    tick + 1,
                )
            })
            .collect();
        let mut takes = Vec::new();
        let mut frames = Vec::new();
        for tick in 0..=160 {
            let mut quantities = BTreeMap::new();
            let mut ground = Vec::new();
            for drop in &drops {
                let visible = tick >= drop.tick + 100;
                let taken = tick >= drop.tick + 104;
                if visible && !taken {
                    ground.push((drop.item_id, drop.tile, 1));
                }
                if taken {
                    quantities.insert(drop.item_id, 1);
                }
                if tick == drop.tick + 100 {
                    takes.push(take(&identity, drop.item_id, drop.tile, tick));
                }
            }
            let mut state = status(&identity);
            state.pickup_units = quantities.len() as u32;
            state.take_sends = takes.iter().filter(|take| take.tick < tick).count() as u32;
            if tick >= 120 {
                state.phase = StatusPhase::Waiting;
            } else if tick > 0 {
                state.phase = StatusPhase::Working;
            }
            let items: Vec<(i32, u32)> = quantities.into_iter().collect();
            frames.push(frame(&identity, tick, &items, &ground, state, tick + 1));
        }
        HelperDropTrace {
            helper: helper_setup(&helper, &drops),
            run: engine_a_run(
                CellKind::ATradeableDrops,
                identity,
                ids,
                drops,
                takes,
                frames,
            ),
        }
    }

    fn a_private_fixture() -> HelperDropTrace {
        let anchor = Tile {
            x: 3300,
            z: 3300,
            plane: 0,
        };
        let identity = run_identity("grinch-private", "a-private-run", 0, anchor);
        let helper = actor("helper-private");
        let drops: Vec<DropAction> = PRIVATE_UNTRADEABLE_IDS
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let tick = index as u64 + 1;
                drop_action(&helper, *id, anchor, tick, tick + 1)
            })
            .collect();
        let frames = (0..=150)
            .map(|tick| {
                let mut state = status(&identity);
                state.phase = StatusPhase::Waiting;
                frame(&identity, tick, &[], &[], state, tick + 1)
            })
            .collect();
        HelperDropTrace {
            helper: helper_setup(&helper, &drops),
            run: engine_a_run(
                CellKind::APrivateUntradeableDrops,
                identity,
                set(PRIVATE_UNTRADEABLE_IDS),
                drops,
                Vec::new(),
                frames,
            ),
        }
    }
    fn a_green_mask_fixture() -> HelperDropTrace {
        let anchor = Tile {
            x: 3250,
            z: 3250,
            plane: 0,
        };
        let identity = run_identity("grinch-mask", "a-mask-run", 0, anchor);
        let helper = actor("helper-mask");
        let mask_ids = [GREEN_MASK_ID, BLUE_MASK_ID, RED_MASK_ID];
        let drops: Vec<DropAction> = mask_ids
            .iter()
            .enumerate()
            .map(|(index, item_id)| {
                let tick = index as u64 + 1;
                drop_action(&helper, *item_id, anchor, tick, tick + 1)
            })
            .collect();
        let mut takes = Vec::new();
        let mut frames = Vec::new();
        for tick in 0..=150 {
            let mut ground = Vec::new();
            for drop in &drops {
                let visible = tick >= drop.tick + 100;
                let taken = drop.item_id == GREEN_MASK_ID && tick >= drop.tick + 104;
                if visible && !taken {
                    ground.push((drop.item_id, drop.tile, 1));
                }
                if drop.item_id == GREEN_MASK_ID && tick == drop.tick + 100 {
                    takes.push(take(&identity, drop.item_id, drop.tile, tick));
                }
            }
            let green_count = (tick >= drops[0].tick + 104) as u32;
            let mut state = status(&identity);
            state.pickup_units = green_count;
            state.take_sends = takes.iter().filter(|take| take.tick < tick).count() as u32;
            let items = if green_count == 0 {
                Vec::new()
            } else {
                vec![(GREEN_MASK_ID, green_count)]
            };
            frames.push(frame(&identity, tick, &items, &ground, state, tick + 1));
        }
        HelperDropTrace {
            helper: helper_setup(&helper, &drops),
            run: engine_a_run(
                CellKind::AGreenMaskSelection,
                identity,
                set([GREEN_MASK_ID]),
                drops,
                takes,
                frames,
            ),
        }
    }

    fn a_negative_candidates_fixture() -> NegativeDropTrace {
        let anchor = Tile {
            x: 3275,
            z: 3275,
            plane: 0,
        };
        let identity = run_identity("grinch-negative", "a-negative-run", 0, anchor);
        let helper = actor("helper-negative");
        let candidates = vec![
            NegativeCandidate {
                kind: NegativeCandidateKind::OrdinaryUnselected,
                item_id: ORDINARY_UNSELECTED_ID,
                tile: anchor,
            },
            NegativeCandidate {
                kind: NegativeCandidateKind::Certificate,
                item_id: EASTER_EGG_CERTIFICATE_ID,
                tile: Tile {
                    x: anchor.x + 1,
                    ..anchor
                },
            },
            NegativeCandidate {
                kind: NegativeCandidateKind::SelectedOutsideRadius,
                item_id: PUMPKIN_ID,
                tile: Tile {
                    x: anchor.x + 13,
                    ..anchor
                },
            },
        ];
        let drops: Vec<DropAction> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let tick = index as u64 + 1;
                drop_action(&helper, candidate.item_id, candidate.tile, tick, tick + 1)
            })
            .collect();
        let frames = (0..=145)
            .map(|tick| {
                let ground: Vec<(i32, Tile, u32)> = drops
                    .iter()
                    .filter(|drop| tick >= drop.tick + 100)
                    .map(|drop| (drop.item_id, drop.tile, 1))
                    .collect();
                frame(&identity, tick, &[], &ground, status(&identity), tick + 1)
            })
            .collect();
        NegativeDropTrace {
            helper: helper_setup(&helper, &drops),
            run: engine_a_run(
                CellKind::ANegativeCandidates,
                identity,
                set(TRADEABLE_IDS),
                drops,
                Vec::new(),
                frames,
            ),
            candidates,
        }
    }

    fn a_cell_fixture() -> CellA {
        CellA {
            tradeable: a_tradeable_fixture(),
            green_mask: a_green_mask_fixture(),
            negative_candidates: a_negative_candidates_fixture(),
            private_untradeable: a_private_fixture(),
        }
    }

    #[test]
    fn cell_a_tradeable_fixture_has_real_shaped_receipts_and_unit_deltas() {
        let trace = a_tradeable_fixture();
        assert!(check_a_tradeable(&trace).is_ok());
    }

    #[test]
    fn cell_a_rejects_wrong_id_dispatch() {
        let mut trace = a_tradeable_fixture();
        trace.run.takes[0].item_id = 1962;
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "Take dispatch used an unselected ID"
        );
    }

    #[test]
    fn cell_a_rejects_false_pickup_count() {
        let mut trace = a_tradeable_fixture();
        for frame in &mut trace.run.frames {
            if let Some(status) = frame.status.as_mut() {
                status.pickup_units = 14;
            }
        }
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "native pickup counter disagrees with observed inventory deltas at a PLAYER_INFO edge"
        );
    }

    #[test]
    fn unmodified_cells_reject_content_changes_and_unproven_shutdown() {
        let mut trace = a_tradeable_fixture();
        trace.run.engine.scratch.content_identity = "changed-content".into();
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "unmodified-content scratch differs from the protected source content"
        );

        let mut trace = a_tradeable_fixture();
        trace.run.engine.scratch.stop_count = 0;
        trace.run.engine.scratch.process_tree_stopped = false;
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "scratch runner did not start once and stop its full process tree"
        );

        let mut trace = a_tradeable_fixture();
        trace.run.engine.scratch.content_patch = scratch(PUMPKIN_ID, Vec::new()).content_patch;
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "unmodified-content cell received a respawn content patch"
        );
    }

    #[test]
    fn cell_a_rejects_missing_ground_authority() {
        let mut trace = a_tradeable_fixture();
        trace.run.authority.ground_pages_complete = false;
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "missing complete posted-ground-page authority"
        );
    }

    #[test]
    fn cell_a_rejects_receiver_seed_in_setup_provenance() {
        let mut trace = a_tradeable_fixture();
        trace.run.setup_seed_actions.push(SetupSeedAction {
            actor: trace.run.identity.actor.clone(),
            observation_sequence: 2,
            inventory: item_snapshot(&[], 0, 28),
        });
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "setup seed action count does not match the cell variant"
        );
    }

    #[test]
    fn cell_a_rejects_incomplete_setup_seed_log() {
        let mut trace = a_tradeable_fixture();
        trace.run.authority.setup_seed_actions_complete = false;
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "missing complete runner-owned setup seed-action log"
        );
    }

    #[test]
    fn cell_a_rejects_inferred_absolute_server_cycle() {
        let mut trace = a_tradeable_fixture();
        trace.run.frames[0].server_cycle = Some(1000);
        assert_eq!(
            check_a_tradeable(&trace).unwrap_err(),
            "helper/bank trace substitutes server cycles for client settlement ticks"
        );
    }

    #[test]
    fn cell_a_rejects_private_untradeable_false_positive() {
        let mut trace = a_private_fixture();
        trace.run.frames[120]
            .ground
            .as_mut()
            .unwrap()
            .rows
            .push(GroundRow {
                item_id: SCYTHE_ID,
                tile: trace.run.drops[0].tile,
                quantity: 1,
            });
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "private item 1419 appeared in Grinch's posted ground page"
        );
    }

    #[test]
    fn cell_a_private_rejects_any_take_dispatch() {
        let mut trace = a_private_fixture();
        trace.run.takes.push(take(
            &trace.run.identity,
            SCYTHE_ID,
            trace.run.drops[0].tile,
            120,
        ));
        sync_helper_take_sends(&mut trace.run);
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "private untradeable Drop produced a Take send"
        );
    }

    #[test]
    fn cell_a_private_rejects_missing_helper_inventory_decrease_confirmation() {
        let mut trace = a_private_fixture();
        trace.helper.post_drop_inventory.complete = false;
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "helper post-Drop inventory is incomplete"
        );
    }

    #[test]
    fn cell_a_private_rejects_post_drop_inventory_not_exactly_decreased_by_drops() {
        let mut trace = a_private_fixture();
        trace.helper.post_drop_inventory.quantities.insert(1961, 1);
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "helper post-Drop inventory does not confirm every real Drop decrease"
        );
    }

    #[test]
    fn cell_a_private_rejects_receiver_fence_before_post_drop_confirmation() {
        let mut trace = a_private_fixture();
        let fence_observation = frame_at(&trace.run.frames, trace.helper.receiver_fence_tick)
            .unwrap()
            .observation_sequence;
        trace.helper.post_drop_observation_sequence = fence_observation + 1;
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "receiver drain fence does not follow the helper post-Drop observation"
        );
    }

    #[test]
    fn cell_a_private_requires_100_edges_after_the_drained_receiver_fence() {
        let mut trace = a_private_fixture();
        trace.helper.receiver_fence_tick = 51;
        assert_eq!(
            check_a_private_untradeable(&trace).unwrap_err(),
            "private trace lacks 100 distinct receiver edges after the drain fence"
        );
    }

    // The full B and C fixtures below deliberately create cycle-by-cycle
    // receipts and PLAYER_INFO snapshots. They are deterministic checker inputs,
    // not claims that a live server produced these events.
    fn loopback_ports() -> LoopbackPorts {
        LoopbackPorts {
            game: Endpoint {
                host: "127.0.0.1".into(),
                port: 44794,
            },
            web: Endpoint {
                host: "127.0.0.1".into(),
                port: 1280,
            },
            management: Endpoint {
                host: "127.0.0.1".into(),
                port: 10098,
            },
            probed_free_before_start: true,
            selected_before_login: true,
        }
    }

    fn scratch(item: i32, spawns: Vec<StaticSpawn>) -> ScratchAuthority {
        let suffix = spawns
            .first()
            .map(|spawn| format!("{}-{}-{}", spawn.item_id, spawn.tile.x, spawn.tile.z))
            .unwrap_or_else(|| "empty".into());
        let attested_pack_hashes = BTreeMap::from([
            ("server/obj.dat".to_string(), "b".repeat(64)),
            ("server/npc.dat".to_string(), "c".repeat(64)),
            ("client/config".to_string(), "d".repeat(64)),
        ]);
        ScratchAuthority {
            request_id: format!("request-{item}-{suffix}"),
            scratch_id: format!("scratch-{item}-{suffix}"),
            source_engine_identity: "engine-source".into(),
            source_content_identity: "content-source".into(),
            engine_identity: format!("engine-scratch-{item}"),
            content_identity: format!("content-scratch-{item}"),
            server_tick_ms: 600,
            world_identity: format!("world-scratch-{item}-{suffix}"),
            account_store_identity: format!("accounts-scratch-{item}-{suffix}"),
            ports: loopback_ports(),
            members_mode: true,
            copied_engine_content_map_inputs_and_account_storage: true,
            respawn_phase_convention:
                RespawnPhaseConvention::FirstObjectTurnInRemovalCycleAndOutputAfterLifecycle,
            content_patch: Some(ContentPatch {
                sha256: "a".repeat(64),
                preserved_existing_map_rows: true,
                real_map_packer_used: true,
                real_config_packer_used: true,
                attested_pack_hashes_before: attested_pack_hashes.clone(),
                attested_pack_hashes_after: attested_pack_hashes,
                raw_respawn_rates: B_RESPAWN_ITEM_IDS
                    .into_iter()
                    .map(|item_id| (item_id, RAW_RESPAWN_RATE))
                    .collect(),
                static_spawns: spawns,
            }),
            start_count: 1,
            stop_count: 1,
            process_tree_stopped: true,
            engine_a_touched: false,
            engine_q_touched: false,
        }
    }

    fn cycle_sample(
        cycle_id: u64,
        actors: &[ActorRef],
        competitor_tile: Option<Tile>,
        order: &[String],
    ) -> ServerCycle {
        ServerCycle {
            cycle: cycle_id,
            object_turn: cycle_id,
            phase_order: vec![
                ServerPhase::PlayerProcessing,
                ServerPhase::ObjectLifecycle,
                ServerPhase::ClientOutput,
            ],
            active_actors: actors
                .iter()
                .map(|actor| ActiveActor {
                    actor: actor.clone(),
                    active: true,
                    tile: if Some(actor) == actors.get(1) {
                        competitor_tile.unwrap()
                    } else {
                        Tile {
                            x: 4000,
                            z: 4000,
                            plane: 0,
                        }
                    },
                })
                .collect(),
            player_processing_order: order.to_vec(),
        }
    }

    fn static_frame(
        identity: &RunIdentity,
        tick: u64,
        cycle_id: u64,
        items: &[(i32, u32)],
        rows: &[(i32, Tile, u32)],
        native: NativeStatus,
    ) -> PlayerFrame {
        let mut result = frame(identity, tick, items, rows, native, cycle_id);
        result.server_cycle = Some(cycle_id);
        result.client_output_phase = Some(ClientOutputPhase::AfterObjectLifecycle);
        result
    }

    fn empty_scratch_trace(
        kind: CellKind,
        item_id: i32,
        anchor: Tile,
        tile: Tile,
        selected: BTreeSet<i32>,
    ) -> StaticSpawnTrace {
        let identity = run_identity(
            &format!("grinch-{item_id}-{}", anchor.x),
            &format!("run-{item_id}-{}", anchor.x),
            0,
            anchor,
        );
        StaticSpawnTrace {
            kind,
            engine: EngineEvidence {
                scratch: Box::new(scratch(
                    item_id,
                    vec![StaticSpawn {
                        item_id,
                        tile,
                        quantity: 1,
                    }],
                )),
            },
            authority: authority(false, true, true, false),
            identity,
            radius: 12,
            selected_ids: selected,
            frames: Vec::new(),
            takes: Vec::new(),
            cycles: Vec::new(),
            removals: Vec::new(),
            respawns: Vec::new(),
            refusals: Vec::new(),
            competitor: None,
            competitor_frames: Vec::new(),
        }
    }

    fn c_frame(
        identity: &RunIdentity,
        tick: u64,
        items: &[(i32, u32)],
        ground_rows: &[(i32, Tile, u32)],
        native: NativeStatus,
        bank: &[(i32, u32)],
        bank_open: bool,
    ) -> PlayerFrame {
        let mut result = frame(identity, tick, items, ground_rows, native, tick + 1);
        result.bank = Some(bank_snapshot(bank));
        result.bank_open = Some(bank_open);
        result.bank_loaded = Some(bank_open);
        result
    }
    fn b_pumpkin_fixture() -> StaticSpawnTrace {
        let anchor = Tile {
            x: 4000,
            z: 4000,
            plane: 0,
        };
        let nearby = Tile {
            x: anchor.x + 1,
            z: anchor.z,
            plane: anchor.plane,
        };
        let outside = Tile {
            x: anchor.x + 13,
            z: anchor.z,
            plane: anchor.plane,
        };
        let spawns = vec![
            StaticSpawn {
                item_id: PUMPKIN_ID,
                tile: anchor,
                quantity: 1,
            },
            StaticSpawn {
                item_id: PUMPKIN_ID,
                tile: nearby,
                quantity: 1,
            },
            StaticSpawn {
                item_id: PUMPKIN_ID,
                tile: outside,
                quantity: 1,
            },
        ];
        let mut trace = empty_scratch_trace(
            CellKind::BPumpkinRespawns,
            PUMPKIN_ID,
            anchor,
            anchor,
            set([PUMPKIN_ID]),
        );
        trace.engine = EngineEvidence {
            scratch: Box::new(scratch(PUMPKIN_ID, spawns)),
        };
        let actor = trace.identity.actor.clone();
        let order = vec![actor.account_id.clone()];
        let respawn_tick = RESPAWN_OUTPUT_TICK_GAP;
        let appearances = [
            (anchor, [0, respawn_tick, 2 * respawn_tick]),
            (nearby, [0, respawn_tick + 1, 2 * respawn_tick + 1]),
        ];
        for (tile, ticks) in appearances {
            for (index, tick) in ticks.into_iter().enumerate() {
                let take_tick = if tile == nearby && index == 0 {
                    1
                } else {
                    tick
                };
                trace.takes.push(scratch_take(
                    &actor,
                    PUMPKIN_ID,
                    tile,
                    take_tick,
                    1001 + take_tick,
                ));
                trace.removals.push(ObjectLifecycleReceipt {
                    cycle: 1001 + take_tick,
                    item_id: PUMPKIN_ID,
                    tile,
                });
                if index > 0 {
                    trace.respawns.push(ObjectLifecycleReceipt {
                        cycle: 1000 + tick,
                        item_id: PUMPKIN_ID,
                        tile,
                    });
                }
            }
        }
        for tick in 0..=(2 * RESPAWN_OUTPUT_TICK_GAP + 42) {
            let cycle_id = 1000 + tick;
            let mut rows = vec![(PUMPKIN_ID, outside, 1)];
            for (tile, tile_appearances) in appearances {
                if tile_appearances.contains(&tick) || (tile == nearby && tick == 1) {
                    rows.push((PUMPKIN_ID, tile, 1));
                }
            }
            let landed = trace.takes.iter().filter(|take| take.tick < tick).count() as u32;
            let mut native = status(&trace.identity);
            native.pickup_units = landed;
            native.take_sends = landed;
            let items = if landed == 0 {
                Vec::new()
            } else {
                vec![(PUMPKIN_ID, landed)]
            };
            trace.frames.push(static_frame(
                &trace.identity,
                tick,
                cycle_id,
                &items,
                &rows,
                native,
            ));
            trace.cycles.push(cycle_sample(
                cycle_id,
                std::slice::from_ref(&actor),
                None,
                &order,
            ));
        }
        trace
    }

    fn b_contested_fixture() -> StaticSpawnTrace {
        let anchor = Tile {
            x: 4100,
            z: 4100,
            plane: 0,
        };
        let mut trace = empty_scratch_trace(
            CellKind::BContestedPumpkin,
            PUMPKIN_ID,
            anchor,
            anchor,
            set([PUMPKIN_ID]),
        );
        let main_actor = trace.identity.actor.clone();
        let competitor_actor = actor("b-competitor");
        let participant = ParticipantIdentity {
            actor: competitor_actor.clone(),
            session_id: "session-b-competitor".into(),
            start_tick: 0,
            fresh_account: true,
            active_before_grinch_start: true,
        };
        let competitor_identity = RunIdentity {
            actor: participant.actor.clone(),
            run_id: "competitor-b-contested".into(),
            session_id: participant.session_id.clone(),
            anchor,
            start_tick: participant.start_tick,
            fresh_account: participant.fresh_account,
        };
        trace.competitor = Some(participant);
        let respawn_tick = RESPAWN_OUTPUT_TICK_GAP;
        let appearances = [
            0,
            respawn_tick,
            2 * respawn_tick,
            3 * respawn_tick,
            4 * respawn_tick,
        ];
        for (index, tick) in appearances.iter().copied().enumerate() {
            trace.takes.push(scratch_take(
                &main_actor,
                PUMPKIN_ID,
                anchor,
                tick,
                1001 + tick,
            ));
            if index < 3 {
                trace.takes.push(scratch_take(
                    &competitor_actor,
                    PUMPKIN_ID,
                    anchor,
                    tick,
                    1001 + tick,
                ));
            }
            trace.removals.push(ObjectLifecycleReceipt {
                cycle: 1001 + tick,
                item_id: PUMPKIN_ID,
                tile: anchor,
            });
            if index > 0 {
                trace.respawns.push(ObjectLifecycleReceipt {
                    cycle: 1000 + tick,
                    item_id: PUMPKIN_ID,
                    tile: anchor,
                });
            }
        }
        let actors = [main_actor.clone(), competitor_actor.clone()];
        let order = vec![
            competitor_actor.account_id.clone(),
            main_actor.account_id.clone(),
        ];
        for tick in 0..=(4 * respawn_tick + 41) {
            let cycle_id = 1000 + tick;
            let rows = if appearances.contains(&tick) {
                vec![(PUMPKIN_ID, anchor, 1)]
            } else {
                Vec::new()
            };
            let competitor_picks = appearances[..3]
                .iter()
                .filter(|appearance| **appearance < tick)
                .count() as u32;
            let grinch_picks = appearances[3..]
                .iter()
                .filter(|appearance| **appearance < tick)
                .count() as u32;
            let mut native = status(&trace.identity);
            native.pickup_units = grinch_picks;
            native.take_sends = trace
                .takes
                .iter()
                .filter(|take| take.actor == main_actor && take.tick < tick)
                .count() as u32;
            let main_items = if grinch_picks == 0 {
                Vec::new()
            } else {
                vec![(PUMPKIN_ID, grinch_picks)]
            };
            let mut main_frame =
                static_frame(&trace.identity, tick, cycle_id, &main_items, &rows, native);
            main_frame.observation_sequence = tick.saturating_mul(2).saturating_add(3);
            trace.frames.push(main_frame);
            let competitor_items = if competitor_picks == 0 {
                Vec::new()
            } else {
                vec![(PUMPKIN_ID, competitor_picks)]
            };
            let mut competitor_frame = frame(
                &competitor_identity,
                tick,
                &competitor_items,
                &rows,
                status(&competitor_identity),
                cycle_id,
            );
            competitor_frame.observation_sequence = tick.saturating_mul(2).saturating_add(2);
            competitor_frame.status = None;
            competitor_frame.position = anchor;
            competitor_frame.server_cycle = Some(cycle_id);
            competitor_frame.client_output_phase = Some(ClientOutputPhase::AfterObjectLifecycle);
            trace.competitor_frames.push(competitor_frame);
            trace
                .cycles
                .push(cycle_sample(cycle_id, &actors, Some(anchor), &order));
        }
        trace
    }

    fn b_contested_fixture_with_competitor_tick_offset(offset: u64) -> StaticSpawnTrace {
        let mut trace = b_contested_fixture();
        let competitor = trace.competitor.as_mut().unwrap();
        competitor.start_tick += offset;
        let competitor_actor = competitor.actor.clone();
        for frame in &mut trace.competitor_frames {
            frame.tick += offset;
        }
        for take in &mut trace.takes {
            if take.actor == competitor_actor {
                take.tick += offset;
            }
        }
        trace
    }

    fn b_contested_r5_receipt_fixture() -> StaticSpawnTrace {
        let mut trace = b_contested_fixture();
        trace.identity.start_tick = 11;
        trace.frames.truncate(402);
        trace.competitor_frames.truncate(402);
        trace.cycles.truncate(402);
        for (index, frame) in trace.frames.iter_mut().enumerate() {
            frame.tick += 11;
            frame.observation_sequence = if index == 0 {
                60
            } else {
                61 + 2 * index as u64
            };
            frame.player_info_sequence = Some(20 + index as u64);
            frame.server_cycle = frame.server_cycle.map(|cycle| cycle - 919);
            frame.object_turn = frame.server_cycle.map(|cycle| cycle + 1);
        }
        for (index, frame) in trace.competitor_frames.iter_mut().enumerate() {
            frame.tick += 32;
            frame.observation_sequence = if index == 0 {
                61
            } else {
                60 + 2 * index as u64
            };
            frame.player_info_sequence = Some(41 + index as u64);
            frame.server_cycle = frame.server_cycle.map(|cycle| cycle - 919);
            frame.object_turn = frame.server_cycle.map(|cycle| cycle + 1);
            if frame.server_cycle == Some(477) {
                frame.ground.as_mut().unwrap().rows.clear();
            }
        }
        let mut first_owner = trace.competitor_frames[0].clone();
        first_owner.tick = 31;
        first_owner.observation_sequence = 58;
        first_owner.player_info_sequence = Some(40);
        first_owner.server_cycle = Some(80);
        first_owner.object_turn = Some(81);
        trace.competitor_frames[1].ground = first_owner.ground.clone();
        trace.competitor_frames[1].inventory = Some(item_snapshot(&[], 0, 28));
        trace.competitor_frames.insert(0, first_owner);
        trace.competitor.as_mut().unwrap().start_tick = 31;
        for take in &mut trace.takes {
            take.tick += if take.actor == trace.identity.actor {
                11
            } else {
                32
            };
            if take.actor != trace.identity.actor && take.tick == 32 {
                take.tick = 31;
            }
            take.server_cycle = take.server_cycle.map(|cycle| cycle - 919);
        }
        for receipt in trace.removals.iter_mut().chain(&mut trace.respawns) {
            receipt.cycle -= 919;
        }
        for cycle in &mut trace.cycles {
            cycle.cycle -= 919;
            cycle.object_turn = cycle.cycle + 1;
        }
        let mut first_cycle = trace.cycles[0].clone();
        first_cycle.cycle = 80;
        first_cycle.object_turn = 81;
        trace.cycles.insert(0, first_cycle);
        trace
    }

    fn b_single_item_fixture(item_id: i32, kind: CellKind) -> StaticSpawnTrace {
        let anchor = Tile {
            x: 4300 + item_id,
            z: 4300,
            plane: 0,
        };
        let mut trace = empty_scratch_trace(kind, item_id, anchor, anchor, set([item_id]));
        let actor = trace.identity.actor.clone();
        let respawn_tick = RESPAWN_OUTPUT_TICK_GAP;
        trace
            .takes
            .push(scratch_take(&actor, item_id, anchor, 0, 1001));
        trace.takes.push(scratch_take(
            &actor,
            item_id,
            anchor,
            respawn_tick,
            1001 + respawn_tick,
        ));
        let duplicate = item_id != YO_YO_ID;
        trace.removals.push(ObjectLifecycleReceipt {
            cycle: 1001,
            item_id,
            tile: anchor,
        });
        trace.respawns.push(ObjectLifecycleReceipt {
            cycle: 1000 + respawn_tick,
            item_id,
            tile: anchor,
        });
        if !duplicate {
            trace.removals.push(ObjectLifecycleReceipt {
                cycle: 1001 + respawn_tick,
                item_id,
                tile: anchor,
            });
        }
        if duplicate {
            let refusal_kind = if item_id == SCYTHE_ID {
                RefusalKind::ScytheAlreadyOwned
            } else {
                RefusalKind::BunnyEarsAlreadyOwned
            };
            trace.refusals.push(RefusalReceipt {
                request_key: trace.takes[1].request_key.clone(),
                item_id,
                tile: anchor,
                tick: respawn_tick + 1,
                kind: refusal_kind,
            });
        }
        let order = vec![actor.account_id.clone()];
        for tick in 0..=(respawn_tick + 42) {
            let cycle_id = 1000 + tick;
            let visible =
                tick == 0 || (tick >= respawn_tick && (duplicate || tick == respawn_tick));
            let rows = if visible {
                vec![(item_id, anchor, 1)]
            } else {
                Vec::new()
            };
            let pickups = (tick >= 1) as u32 + (!duplicate && tick > respawn_tick) as u32;
            let mut native = status(&trace.identity);
            native.pickup_units = pickups;
            native.take_sends = trace.takes.iter().filter(|take| take.tick < tick).count() as u32;
            if duplicate && tick > respawn_tick {
                native.duplicate_refusals = 1;
                native.id_exclusions.insert(item_id);
            }
            let items = if pickups == 0 {
                Vec::new()
            } else {
                vec![(item_id, pickups)]
            };
            trace.frames.push(static_frame(
                &trace.identity,
                tick,
                cycle_id,
                &items,
                &rows,
                native,
            ));
            trace.cycles.push(cycle_sample(
                cycle_id,
                std::slice::from_ref(&actor),
                None,
                &order,
            ));
        }
        trace
    }

    fn b_cell_fixture() -> CellB {
        CellB {
            pumpkin_respawns: b_pumpkin_fixture(),
            contested_pumpkin: b_contested_fixture(),
            scythe_duplicate: b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate),
            bunny_ears_duplicate: b_single_item_fixture(
                BUNNY_EARS_ID,
                CellKind::BBunnyEarsDuplicate,
            ),
            yo_yo_repeat: b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat),
        }
    }

    fn c_bank_fixture(
        run: &str,
        starting_pumpkins: u32,
        carried: Option<i32>,
        deposit: u32,
    ) -> BankFlowTrace {
        let anchor = Tile {
            x: 5000,
            z: 5000,
            plane: 0,
        };
        let identity = run_identity(&format!("grinch-{run}"), run, 0, anchor);
        let helper = actor(&format!("helper-{run}"));
        let drops = vec![
            drop_action(&helper, PUMPKIN_ID, anchor, 1, 2),
            drop_action(&helper, PUMPKIN_ID, anchor, 115, 116),
        ];
        let takes = vec![
            take(&identity, PUMPKIN_ID, anchor, 101),
            take(&identity, PUMPKIN_ID, anchor, 215),
        ];
        let bank_before = vec![(PUMPKIN_ID, 5), (1961, 2)];
        let bank_after = vec![(PUMPKIN_ID, 5 + deposit), (1961, 2)];
        let mut frames = Vec::new();
        for tick in 0..=256 {
            let pumpkins = if tick < 102 {
                starting_pumpkins
            } else if tick < 112 {
                deposit
            } else if tick < 216 {
                0
            } else {
                1
            };
            let mut native = status(&identity);
            native.pickup_units = (tick >= 102) as u32 + (tick >= 216) as u32;
            native.take_sends = takes.iter().filter(|take| take.tick < tick).count() as u32;
            native.bank_trips = (tick >= 110) as u32;
            native.banked_units = if tick >= 112 { deposit } else { 0 };
            native.phase = if (110..113).contains(&tick) {
                StatusPhase::Banking
            } else if (113..115).contains(&tick) {
                StatusPhase::Returning
            } else {
                StatusPhase::Waiting
            };
            let bank = if tick < 112 {
                &bank_before
            } else {
                &bank_after
            };
            let bank_open = (110..113).contains(&tick);
            let mut items = vec![(PUMPKIN_ID, pumpkins)];
            if let Some(item_id) = carried {
                items.push((item_id, 1));
            }
            let rows = if tick == 101 {
                vec![(PUMPKIN_ID, drops[0].tile, 1)]
            } else if tick == 215 {
                vec![(PUMPKIN_ID, drops[1].tile, 1)]
            } else {
                Vec::new()
            };
            frames.push(c_frame(
                &identity, tick, &items, &rows, native, bank, bank_open,
            ));
        }
        let bank_actions = vec![
            BankAction {
                request_key: format!("open-{run}"),
                tick: 110,
                kind: BankActionKind::Open,
            },
            BankAction {
                request_key: format!("deposit-{run}"),
                tick: 111,
                kind: BankActionKind::Deposit {
                    item_id: PUMPKIN_ID,
                    quantity: deposit,
                },
            },
            BankAction {
                request_key: format!("close-{run}"),
                tick: 113,
                kind: BankActionKind::Close,
            },
        ];
        let mut initial_items = vec![(PUMPKIN_ID, starting_pumpkins)];
        if let Some(item_id) = carried {
            initial_items.push((item_id, 1));
        }
        let pre_start = pre_start_observation(
            &identity,
            &initial_items,
            (starting_pumpkins + u32::from(carried.is_some())) as u16,
        );
        let helper_evidence = helper_setup(&helper, &drops);
        let setup_seed_actions = vec![
            setup_seed_action(&helper, 1, &helper_evidence.seeded_inventory),
            setup_seed_action(&identity.actor, 2, &pre_start.inventory),
        ];
        BankFlowTrace {
            kind: if carried.is_some() {
                CellKind::CDeposit27PreserveItem
            } else {
                CellKind::CDeposit28
            },
            engine: unmodified_scratch(),
            authority: authority(true, true, false, true),
            pre_start_inventory: pre_start,
            setup_seed_actions,
            identity,
            radius: 12,
            selected_ids: set([PUMPKIN_ID]),
            frames,
            takes,
            drops: drops.clone(),
            helper: helper_evidence,
            starting_pumpkins,
            carried_item_id: carried,
            route: BankRouteEvidence {
                bank_id: "nearest-ordinary-bank".into(),
                nearest_routable: true,
                ordinary_accessible_bank: true,
                exact_route_proven: true,
                used_air_distance_fallback: false,
                walking_steps: 10,
            },
            bank_before: Some(bank_snapshot(&bank_before)),
            bank_after: Some(bank_snapshot(&bank_after)),
            bank_actions,
            packet_order: BankPacketOrderProof {
                modal: Some(BankUiPacketReceipt {
                    ordinal: Some(BankPacketOrdinal {
                        tick: 110,
                        position: 1,
                    }),
                    session_generation: 2,
                    arrival_bank_generation: 3,
                    callback_sequence: 1100,
                    packet_observation: 10,
                    player_info_changed: false,
                    bank_snapshot: None,
                }),
                table: Some(BankUiPacketReceipt {
                    ordinal: Some(BankPacketOrdinal {
                        tick: 110,
                        position: 2,
                    }),
                    session_generation: 2,
                    arrival_bank_generation: 3,
                    callback_sequence: 1100,
                    packet_observation: 11,
                    player_info_changed: false,
                    bank_snapshot: Some(bank_snapshot(&bank_before)),
                }),
                table_binding: Some(BankTableBinding {
                    session_generation: 2,
                    bound_to_generation: 3,
                    callback_sequence: 1100,
                    packet_observation: 11,
                    bank_snapshot: Some(bank_snapshot(&bank_before)),
                }),
                deposit: Some(BankDepositPacketReceipt {
                    request_key: format!("deposit-{run}"),
                    ordinal: Some(BankPacketOrdinal {
                        tick: 111,
                        position: 1,
                    }),
                    session_generation: 2,
                    bank_generation: 3,
                    callback_sequence: 1110,
                    observed_modal_stamp: 10,
                    observed_table_stamp: 11,
                    host_log_ordinal_fence: 1000,
                    click_host_log_ordinal: 1001,
                }),
                ambiguous_ui_batches: 0,
            },
            receipts: BankFlowReceipts {
                open: Some(BankOpenReceipt {
                    request_key: format!("open-{run}"),
                    tick: 110,
                    bank_id: "nearest-ordinary-bank".into(),
                    interface_open: true,
                    bank_loaded: true,
                }),
                close: Some(BankCloseReceipt {
                    request_key: format!("close-{run}"),
                    tick: 113,
                    bank_id: "nearest-ordinary-bank".into(),
                    interface_closed: true,
                }),
                return_to_anchor: Some(AnchorReturnReceipt {
                    request_key: format!("return-{run}"),
                    tick: 114,
                    tile: anchor,
                    arrival_receipt_observed: true,
                }),
            },
        }
    }

    fn c_inventory_blocked_fixture() -> InventoryBlockedTrace {
        let anchor = Tile {
            x: 5100,
            z: 5100,
            plane: 0,
        };
        let mut identity = run_identity("grinch-c-blocked", "c-blocked-run", 0, anchor);
        identity.run_id = format!("account:{}:slot:1:run:1", identity.actor.account_id);
        let items: Vec<(i32, u32)> = (0..28).map(|index| (3000 + index, 1)).collect();
        let frames = (0..=1)
            .map(|tick| {
                let mut native = status(&identity);
                native.phase = StatusPhase::InventoryBlocked;
                native.blocked_reason = Some(BlockedReason::NoSelectedItemToDeposit);
                frame(&identity, tick, &items, &[], native, tick + 1)
            })
            .collect();
        let pre_start_inventory = pre_start_observation(&identity, &items, 28);
        let setup_seed_actions = vec![setup_seed_action(
            &identity.actor,
            2,
            &pre_start_inventory.inventory,
        )];
        InventoryBlockedTrace {
            pre_start_inventory,
            setup_seed_actions,
            kind: CellKind::CInventoryBlocked,
            engine: unmodified_scratch(),
            authority: authority(false, false, false, true),
            terminal_stop: TerminalNativeStop {
                run_slot: 1,
                run_number: 1,
                native_session: 1,
                card: "Grinch".into(),
                native_phase: "Blocked".into(),
                failure_code: "inventory-blocked".into(),
                failure_message: "inventory is full and contains no selected item to deposit"
                    .into(),
                status_read_observation_sequence: 4,
                generation_before_start: 0,
                generation_after_stop: 1,
                active_run_key_after_stop: None,
            },
            identity,
            selected_ids: set([PUMPKIN_ID]),
            frames,
            bank_actions: Vec::new(),
        }
    }

    fn c_cell_fixture() -> CellC {
        CellC {
            deposit_28: c_bank_fixture("c-deposit-28", 27, None, 28),
            deposit_27_preserve_item: c_bank_fixture(
                "c-deposit-27",
                26,
                Some(ORDINARY_UNSELECTED_ID),
                27,
            ),
            inventory_blocked: c_inventory_blocked_fixture(),
        }
    }

    fn c_same_edge_bank_packets_fixture() -> CellC {
        let mut cell = c_cell_fixture();
        for trace in [&mut cell.deposit_28, &mut cell.deposit_27_preserve_item] {
            trace.receipts.open.as_mut().unwrap().tick = 111;
            trace.bank_actions[0].tick = 111;
            let modal = trace.packet_order.modal.as_mut().unwrap();
            modal.ordinal = Some(BankPacketOrdinal {
                tick: 111,
                position: 1,
            });
            modal.callback_sequence = 1110;
            let table = trace.packet_order.table.as_mut().unwrap();
            table.ordinal = Some(BankPacketOrdinal {
                tick: 111,
                position: 2,
            });
            table.callback_sequence = 1110;
            trace
                .packet_order
                .table_binding
                .as_mut()
                .unwrap()
                .callback_sequence = 1110;
            trace.packet_order.deposit.as_mut().unwrap().ordinal = Some(BankPacketOrdinal {
                tick: 111,
                position: 3,
            });
            for frame in &mut trace.frames[110..=111] {
                frame.bank_open = Some(false);
                frame.bank_loaded = Some(false);
                frame.bank = None;
            }
        }
        cell
    }

    #[test]
    fn cell_a_complete_fixture_passes_all_drop_and_visibility_variants() {
        check_cell_a(&a_cell_fixture()).expect("complete A fixture");
    }

    fn sync_helper_take_sends(run: &mut PlayerRunTrace) {
        let take_ticks: Vec<u64> = run.takes.iter().map(|take| take.tick).collect();
        for frame in &mut run.frames {
            frame.status.as_mut().unwrap().take_sends = take_ticks
                .iter()
                .filter(|tick| **tick < frame.tick)
                .count() as u32;
        }
    }

    fn negative_candidates_with_take(kind: NegativeCandidateKind) -> NegativeDropTrace {
        let mut trace = a_negative_candidates_fixture();
        let candidate = trace
            .candidates
            .iter()
            .find(|candidate| candidate.kind == kind)
            .unwrap();
        let item_id = candidate.item_id;
        let tile = candidate.tile;
        let identity = trace.run.identity.clone();
        trace
            .run
            .takes
            .push(take(&identity, item_id, tile, 103));
        sync_helper_take_sends(&mut trace.run);
        trace
    }

    #[test]
    fn cell_a_green_mask_rejects_selection_of_another_mask() {
        let mut trace = a_green_mask_fixture();
        trace.run.selected_ids.insert(BLUE_MASK_ID);
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "green-mask variant selected more than green 1053"
        );
    }

    #[test]
    fn cell_a_green_mask_rejects_missing_green_take() {
        let mut trace = a_green_mask_fixture();
        trace.run.takes.clear();
        sync_helper_take_sends(&mut trace.run);
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "mask Take ledger was not green-only"
        );
    }

    #[test]
    fn cell_a_green_mask_rejects_blue_take() {
        let mut trace = a_green_mask_fixture();
        let identity = trace.run.identity.clone();
        trace
            .run
            .takes
            .push(take(&identity, BLUE_MASK_ID, identity.anchor, 102));
        sync_helper_take_sends(&mut trace.run);
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "Take dispatch used an unselected ID"
        );
    }

    #[test]
    fn cell_a_green_mask_rejects_red_take() {
        let mut trace = a_green_mask_fixture();
        let identity = trace.run.identity.clone();
        trace
            .run
            .takes
            .push(take(&identity, RED_MASK_ID, identity.anchor, 103));
        sync_helper_take_sends(&mut trace.run);
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "Take dispatch used an unselected ID"
        );
    }

    #[test]
    fn cell_a_green_mask_rejects_an_unselected_mask_pickup() {
        let mut trace = a_green_mask_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 130) {
            let inventory = frame.inventory.as_mut().unwrap();
            inventory.quantities.insert(BLUE_MASK_ID, 1);
            inventory.occupied_slots = Some(2);
            frame.status.as_mut().unwrap().pickup_units = 2;
        }
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "mask pack delta was not green-only"
        );
    }

    #[test]
    fn cell_a_green_mask_requires_blue_to_remain_posted() {
        let mut trace = a_green_mask_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 120) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != BLUE_MASK_ID);
        }
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "blue mask did not remain on its tile"
        );
    }

    #[test]
    fn cell_a_green_mask_requires_red_to_remain_posted() {
        let mut trace = a_green_mask_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 120) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != RED_MASK_ID);
        }
        assert_eq!(
            check_a_green_mask(&trace).unwrap_err(),
            "red mask did not remain on its tile"
        );
    }

    #[test]
    fn cell_a_negative_candidates_rejects_take_of_ordinary_unselected_item() {
        let trace =
            negative_candidates_with_take(NegativeCandidateKind::OrdinaryUnselected);
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "Take dispatch used an unselected ID"
        );
    }

    #[test]
    fn cell_a_negative_candidates_rejects_take_of_certificate() {
        let trace = negative_candidates_with_take(NegativeCandidateKind::Certificate);
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "Take dispatch used an unselected ID"
        );
    }

    #[test]
    fn cell_a_negative_candidates_rejects_take_of_selected_outside_radius_item() {
        let trace =
            negative_candidates_with_take(NegativeCandidateKind::SelectedOutsideRadius);
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "Take dispatch targeted a tile outside the anchor radius or plane"
        );
    }

    #[test]
    fn cell_a_negative_candidates_rejects_a_negative_item_pickup() {
        let mut trace = a_negative_candidates_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 110) {
            let inventory = frame.inventory.as_mut().unwrap();
            inventory.quantities.insert(ORDINARY_UNSELECTED_ID, 1);
            inventory.occupied_slots = Some(1);
            frame.status.as_mut().unwrap().pickup_units = 1;
        }
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "negative-only variant changed pack inventory"
        );
    }

    #[test]
    fn cell_a_negative_candidates_require_ordinary_row_to_remain_posted() {
        let mut trace = a_negative_candidates_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 120) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != ORDINARY_UNSELECTED_ID);
        }
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "negative row 995 disappeared or changed"
        );
    }

    #[test]
    fn cell_a_negative_candidates_require_certificate_to_remain_posted() {
        let mut trace = a_negative_candidates_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 120) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != EASTER_EGG_CERTIFICATE_ID);
        }
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "negative row 1962 disappeared or changed"
        );
    }

    #[test]
    fn cell_a_negative_candidates_require_outside_radius_row_to_remain_posted() {
        let mut trace = a_negative_candidates_fixture();
        for frame in trace.run.frames.iter_mut().filter(|frame| frame.tick >= 120) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != PUMPKIN_ID);
        }
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "negative row 1959 disappeared or changed"
        );
    }

    #[test]
    fn cell_a_negative_candidates_rejects_a_non_waiting_healthy_tail() {
        let mut trace = a_negative_candidates_fixture();
        trace.run.frames.last_mut().unwrap().status.as_mut().unwrap().phase =
            StatusPhase::Working;
        assert_eq!(
            check_a_negative_candidates(&trace).unwrap_err(),
            "idle tail left Waiting status"
        );
    }

    #[test]
    fn cell_b_complete_fixture_passes_respawn_race_and_duplicate_variants() {
        check_cell_b(&b_cell_fixture()).expect("complete B fixture");
    }

    fn sync_static_take_sends(trace: &mut StaticSpawnTrace) {
        let main_actor = trace.identity.actor.clone();
        let take_ticks: Vec<u64> = trace
            .takes
            .iter()
            .filter(|take| take.actor == main_actor)
            .map(|take| take.tick)
            .collect();
        for frame in &mut trace.frames {
            frame.status.as_mut().unwrap().take_sends = take_ticks
                .iter()
                .filter(|tick| **tick < frame.tick)
                .count() as u32;
        }
    }

    #[test]
    fn cell_b_pumpkin_rejects_work_between_respawns_without_a_target() {
        let mut trace = b_pumpkin_fixture();
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| (20..=80).contains(&frame.tick))
        {
            frame.status.as_mut().unwrap().phase = StatusPhase::Working;
        }
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "Grinch did not remain in Waiting when no in-range static target was visible"
        );
    }

    #[test]
    fn cell_b_pumpkin_rejects_foreground_child_between_respawns_without_a_target() {
        let mut trace = b_pumpkin_fixture();
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| (20..=80).contains(&frame.tick))
        {
            frame.status.as_mut().unwrap().foreground_children = 1;
        }
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "Grinch retained foreground work when no in-range static target was visible"
        );
    }

    #[test]
    fn cell_b_pumpkin_rejects_recovery_and_restart_during_the_trace() {
        let mut trace = b_pumpkin_fixture();
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 50) {
            let status = frame.status.as_mut().unwrap();
            status.recovery_count = 1;
            status.restart_count = 1;
        }
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "scratch respawn cell recovered or restarted"
        );
    }

    #[test]
    fn cell_b_contested_rejects_recovery_and_restart_during_the_trace() {
        let mut trace = b_contested_fixture();
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 50) {
            let status = frame.status.as_mut().unwrap();
            status.recovery_count = 1;
            status.restart_count = 1;
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "scratch respawn cell recovered or restarted"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_missing_content_refusal() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        trace.refusals.clear();
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate variant requires exactly one content refusal"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_wrong_typed_content_refusal() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        trace.refusals[0].kind = RefusalKind::BunnyEarsAlreadyOwned;
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "second-copy refusal lacks the exact typed content refusal receipt"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_missing_scythe_exclusion() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        let refusal_tick = trace.refusals[0].tick;
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= refusal_tick)
        {
            frame.status.as_mut().unwrap().id_exclusions.clear();
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate item ID exclusion is absent or incorrect"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_missing_bunny_ears_exclusion() {
        let mut trace = b_single_item_fixture(BUNNY_EARS_ID, CellKind::BBunnyEarsDuplicate);
        let refusal_tick = trace.refusals[0].tick;
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= refusal_tick)
        {
            frame.status.as_mut().unwrap().id_exclusions.clear();
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, BUNNY_EARS_ID).unwrap_err(),
            "duplicate item ID exclusion is absent or incorrect"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_exclusion_before_refusal() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= 50)
        {
            frame
                .status
                .as_mut()
                .unwrap()
                .id_exclusions
                .insert(SCYTHE_ID);
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate ID exclusion appeared before refusal"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_refusal_outside_take_settlement() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        trace.refusals[0].tick += MAX_TAKE_SETTLEMENT_TICKS + 1;
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate refusal was not observed within the Take settlement fence"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_an_extra_take_after_exclusion() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        let refusal_tick = trace.refusals[0].tick;
        let actor = trace.identity.actor.clone();
        let tile = trace.identity.anchor;
        let next_tick = refusal_tick + 1;
        trace.takes.push(scratch_take(
            &actor,
            SCYTHE_ID,
            tile,
            next_tick,
            1001 + next_tick,
        ));
        sync_static_take_sends(&mut trace);
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "Grinch sent another Take after duplicate ID exclusion"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_a_removed_refused_row() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick > 100) {
            frame
                .ground
                .as_mut()
                .unwrap()
                .rows
                .retain(|row| row.item_id != SCYTHE_ID);
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "refused duplicate row did not remain unchanged"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_non_waiting_after_refusal() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        let refusal_tick = trace.refusals[0].tick;
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= refusal_tick)
        {
            frame.status.as_mut().unwrap().phase = StatusPhase::Working;
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate exclusion did not return to Waiting"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_a_short_waiting_tail() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        let refusal_tick = trace.refusals[0].tick;
        trace.frames.truncate(refusal_tick as usize + 40);
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "healthy Waiting tail started before the required work completed"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_foreground_work_in_waiting_tail() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 110) {
            frame.status.as_mut().unwrap().foreground_children = 1;
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "idle tail retained foreground work"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_early_refusal_counter() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= 50)
        {
            frame.status.as_mut().unwrap().duplicate_refusals = 1;
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate refusal counter advanced before refusal"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_an_extra_refusal_count() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        trace.frames.last_mut().unwrap().status.as_mut().unwrap().duplicate_refusals = 2;
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "native duplicate refusal count is not one"
        );
    }

    #[test]
    fn cell_b_duplicate_rejects_an_unrelated_failed_key() {
        let mut trace = b_single_item_fixture(SCYTHE_ID, CellKind::BScytheDuplicate);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 100) {
            frame.status.as_mut().unwrap().failed_key_attempts = 1;
        }
        assert_eq!(
            check_b_duplicate_untradeable(&trace, SCYTHE_ID).unwrap_err(),
            "duplicate variant contains an unrelated failure, avoidance, or exclusion"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_an_invented_duplicate_refusal() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        let take = trace.takes[1].clone();
        trace.refusals.push(RefusalReceipt {
            request_key: take.request_key,
            item_id: YO_YO_ID,
            tile: take.tile,
            tick: take.tick + 1,
            kind: RefusalKind::ScytheAlreadyOwned,
        });
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "Yo-yo trace invented a duplicate refusal"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_an_invented_id_exclusion() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 50) {
            frame.status.as_mut().unwrap().id_exclusions.insert(YO_YO_ID);
        }
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "Yo-yo acquired an invented ID exclusion"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_a_missing_second_take() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        trace.takes.retain(|take| take.tick == 0);
        sync_static_take_sends(&mut trace);
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "Yo-yo must have one fresh Take per appearance"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_an_extra_take_for_one_appearance() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        let actor = trace.identity.actor.clone();
        let mut extra = scratch_take(
            &actor,
            YO_YO_ID,
            trace.identity.anchor,
            RESPAWN_OUTPUT_TICK_GAP,
            1001 + RESPAWN_OUTPUT_TICK_GAP,
        );
        extra.request_key = "extra-yoyo-take".into();
        trace.takes.push(extra);
        sync_static_take_sends(&mut trace);
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "Yo-yo must have one fresh Take per appearance"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_fewer_than_two_landed_items() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick > RESPAWN_OUTPUT_TICK_GAP) {
            let inventory = frame.inventory.as_mut().unwrap();
            inventory.quantities.insert(YO_YO_ID, 1);
            inventory.occupied_slots = Some(1);
            frame.status.as_mut().unwrap().pickup_units = 1;
        }
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "Yo-yo did not land exactly twice"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_a_failed_key_attempt() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 50) {
            frame.status.as_mut().unwrap().failed_key_attempts = 1;
        }
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "trace contains a failure, avoidance entry, or run exclusion"
        );
    }

    #[test]
    fn cell_b_yoyo_rejects_an_unexpected_bank_trip() {
        let mut trace = b_single_item_fixture(YO_YO_ID, CellKind::BYoYoRepeat);
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 50) {
            frame.status.as_mut().unwrap().bank_trips = 1;
        }
        assert_eq!(
            check_b_yoyo_repeat(&trace).unwrap_err(),
            "unexpected bank trip"
        );
    }

    #[test]
    fn cell_b_contested_uses_each_participants_own_tick_clock() {
        for offset in [21, 121] {
            let trace = b_contested_fixture_with_competitor_tick_offset(offset);
            check_b_contested_pumpkin(&trace)
                .unwrap_or_else(|error| panic!("competitor tick offset {offset}: {error}"));
        }
    }

    #[test]
    fn cell_b_contested_r5_receipt_passes_three_races_then_two_grinch_wins() {
        let trace = b_contested_r5_receipt_fixture();
        let tile = trace.identity.anchor;
        assert_eq!(
            spawn_appearances(&trace.frames, PUMPKIN_ID, tile)
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            spawn_appearances(&trace.competitor_frames, PUMPKIN_ID, tile)
                .unwrap()
                .len(),
            4
        );
        check_b_contested_pumpkin(&trace).expect("R5 receipt follows design B:290-300");
    }

    #[test]
    fn cell_b_contested_requires_three_fresh_competitor_races() {
        let mut trace = b_contested_r5_receipt_fixture();
        for frame in &mut trace.competitor_frames {
            if matches!(frame.server_cycle, Some(279 | 378)) {
                frame.ground.as_mut().unwrap().rows.clear();
            }
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "competitor did not observe three fresh race appearances"
        );
    }

    #[test]
    fn cell_b_contested_r5_receipt_accepts_four_own_edge_settlement() {
        let mut trace = b_contested_r5_receipt_fixture();
        let rows = trace.competitor_frames[0].ground.clone();
        for frame in &mut trace.competitor_frames {
            if frame.server_cycle == Some(477) {
                frame.ground = rows.clone();
            }
            if frame.tick == 34 {
                frame.inventory = Some(item_snapshot(&[], 0, 28));
            }
        }
        check_b_contested_pumpkin(&trace).expect("four own PI edges remain inside the bound");
    }

    #[test]
    fn cell_b_contested_rejects_fifth_own_edge_settlement() {
        let mut trace = b_contested_r5_receipt_fixture();
        for frame in &mut trace.competitor_frames {
            if (34..=35).contains(&frame.tick) {
                frame.inventory = Some(item_snapshot(&[], 0, 28));
            }
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "competitor pack did not increase within four own PLAYER_INFO frames"
        );
    }

    #[test]
    fn cell_b_contested_rejects_cropped_first_send_owner() {
        let mut trace = b_contested_r5_receipt_fixture();
        trace.competitor_frames.remove(0);
        trace.competitor.as_mut().unwrap().start_tick = 32;
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "fresh static copy lacks exactly one Take dispatch"
        );
    }

    #[test]
    fn cell_b_contested_rejects_competitor_inactivity_after_three_wins() {
        let mut trace = b_contested_r5_receipt_fixture();
        let competitor = &trace.competitor.as_ref().unwrap().actor;
        trace
            .cycles
            .last_mut()
            .unwrap()
            .active_actors
            .iter_mut()
            .find(|presence| presence.actor == *competitor)
            .unwrap()
            .active = false;
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "scratch actor was not continuously active"
        );
    }

    #[test]
    fn cell_b_contested_rejects_grinch_pack_increase_in_a_lost_race() {
        let mut trace = b_contested_r5_receipt_fixture();
        let frame = trace
            .frames
            .iter_mut()
            .find(|frame| frame.tick == 15)
            .unwrap();
        frame.inventory = Some(item_snapshot(&[(PUMPKIN_ID, 1)], 1, 28));
        for frame in trace.frames.iter_mut().filter(|frame| frame.tick >= 15) {
            frame.status.as_mut().unwrap().pickup_units = 1;
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "Grinch pack changed during a competitor win"
        );
    }
    #[test]
    fn cell_b_contested_rejects_competitor_take_after_twenty_own_edges() {
        let mut trace = b_contested_fixture_with_competitor_tick_offset(21);
        let competitor = &trace.competitor.as_ref().unwrap().actor;
        trace
            .takes
            .iter_mut()
            .find(|take| take.actor == *competitor)
            .unwrap()
            .tick += 21;
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "item 1959 Take was outside the fresh-row 20-tick bound"
        );
    }

    fn assert_lifecycle_gap_rejected(gap: u64) {
        let mut trace = b_pumpkin_fixture();
        let anchor = trace.identity.anchor;
        let previous_take = trace
            .takes
            .iter()
            .find(|take| take.tile == anchor && take.tick == 0)
            .unwrap()
            .clone();
        let removal_cycle = trace
            .removals
            .iter()
            .find(|receipt| receipt.tile == anchor)
            .unwrap()
            .cycle;
        let appearances = spawn_appearances(&trace.frames, PUMPKIN_ID, anchor).unwrap();
        let mut fresh = trace.frames[appearances[1]].clone();
        let respawn = trace
            .respawns
            .iter_mut()
            .find(|receipt| receipt.tile == anchor)
            .unwrap();
        respawn.cycle = removal_cycle + gap;
        // Test the lifecycle boundary directly, including a matching later
        // client-output sample when the deliberately wrong event is late.
        fresh.server_cycle = Some(fresh.server_cycle.unwrap().max(respawn.cycle));
        fresh.object_turn = Some(
            cycle(&trace.cycles, fresh.server_cycle.unwrap())
                .unwrap()
                .object_turn,
        );
        let error = require_respawn_gap(&trace, 1, &previous_take, &fresh).unwrap_err();
        assert_eq!(
            error,
            format!("respawn lifecycle gap was {gap}, expected exactly 98 server cycles")
        );
    }

    #[test]
    fn cell_b_rejects_97_cycle_actual_lifecycle_with_a_named_reason() {
        assert_lifecycle_gap_rejected(97);
    }

    #[test]
    fn cell_b_rejects_99_cycle_actual_lifecycle_with_a_named_reason() {
        assert_lifecycle_gap_rejected(99);
    }

    #[test]
    fn cell_b_rejects_missing_removal_receipt_with_a_named_reason() {
        let mut trace = b_pumpkin_fixture();
        let anchor = trace.identity.anchor;
        trace.removals.retain(|receipt| receipt.tile != anchor);
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "missing removal receipt for previous Take"
        );
    }

    #[test]
    fn cell_b_rejects_missing_respawn_receipt_with_a_named_reason() {
        let mut trace = b_pumpkin_fixture();
        let anchor = trace.identity.anchor;
        trace.respawns.retain(|receipt| receipt.tile != anchor);
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "missing respawn receipt for first fresh row"
        );
    }

    #[test]
    fn cell_b_rejects_map_patch_that_changes_an_attested_pack() {
        let mut trace = b_pumpkin_fixture();
        trace
            .engine
            .scratch
            .content_patch
            .as_mut()
            .unwrap()
            .attested_pack_hashes_after
            .insert("server/obj.dat".into(), "e".repeat(64));
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "map-only scratch patch changed attested pack server/obj.dat"
        );
    }

    #[test]
    fn cell_b_same_tick_later_take_does_not_claim_an_earlier_inventory_delta() {
        check_b_pumpkin_respawns(&b_pumpkin_fixture())
            .expect("sequential identical-ID Takes retain their causal inventory deltas");
    }

    #[test]
    fn cell_b_rejects_premature_identical_copy_respawn_with_a_named_reason() {
        let mut trace = b_pumpkin_fixture();
        let anchor = trace.identity.anchor;
        let removal_cycle = trace
            .removals
            .iter()
            .find(|receipt| receipt.tile == anchor)
            .unwrap()
            .cycle;
        let respawn = trace
            .respawns
            .iter_mut()
            .find(|receipt| receipt.tile == anchor)
            .unwrap();
        respawn.cycle = removal_cycle + 97;
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "respawn lifecycle gap was 97, expected exactly 98 server cycles"
        );
    }

    #[test]
    fn cell_b_rejects_omitted_fresh_identical_copy_take() {
        let mut trace = b_pumpkin_fixture();
        let anchor = trace.identity.anchor;
        trace.takes.retain(|take| {
            !(take.item_id == PUMPKIN_ID
                && take.tile == anchor
                && take.tick == RESPAWN_OUTPUT_TICK_GAP)
        });
        let take_ticks: Vec<u64> = trace.takes.iter().map(|take| take.tick).collect();
        for frame in &mut trace.frames {
            frame.status.as_mut().unwrap().take_sends =
                take_ticks.iter().filter(|tick| **tick < frame.tick).count() as u32;
        }
        assert_eq!(
            check_b_pumpkin_respawns(&trace).unwrap_err(),
            "fresh static copy lacks exactly one Take dispatch"
        );
    }

    #[test]
    fn cell_b_rejects_race_derived_contested_tile_exclusion() {
        let mut trace = b_contested_fixture();
        let key = GroundKey {
            item_id: PUMPKIN_ID,
            tile: trace.identity.anchor,
        };
        for frame in &mut trace.frames {
            if frame.tick >= 2 * RESPAWN_OUTPUT_TICK_GAP + 2 {
                frame
                    .status
                    .as_mut()
                    .unwrap()
                    .failed_key_exclusions
                    .insert(key.clone());
            }
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "trace contains a failure, avoidance entry, or run exclusion"
        );
    }

    #[test]
    fn cell_b_rejects_race_derived_avoidance_entry() {
        let mut trace = b_contested_fixture();
        for frame in trace
            .frames
            .iter_mut()
            .filter(|frame| frame.tick >= 2 * RESPAWN_OUTPUT_TICK_GAP + 2)
        {
            frame.status.as_mut().unwrap().avoidance_entries = 1;
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "trace contains a failure, avoidance entry, or run exclusion"
        );
    }

    #[test]
    fn cell_b_rejects_fresh_take_delayed_beyond_20_ticks() {
        let mut trace = b_contested_fixture();
        let main_actor = trace.identity.actor.clone();
        let delayed_appearance_tick = 3 * RESPAWN_OUTPUT_TICK_GAP;
        let delayed_take_tick = delayed_appearance_tick + 21;
        let delayed = trace
            .takes
            .iter_mut()
            .find(|take| take.actor == main_actor && take.tick == delayed_appearance_tick)
            .unwrap();
        delayed.tick = delayed_take_tick;
        delayed.server_cycle = Some(1001 + delayed_take_tick);
        let main_take_ticks: Vec<u64> = trace
            .takes
            .iter()
            .filter(|take| take.actor == main_actor)
            .map(|take| take.tick)
            .collect();
        let tile = trace.identity.anchor;
        for frame in trace.frames.iter_mut().filter(|frame| {
            ((delayed_appearance_tick + 1)..=delayed_take_tick).contains(&frame.tick)
        }) {
            frame.ground.as_mut().unwrap().rows.push(GroundRow {
                item_id: PUMPKIN_ID,
                tile,
                quantity: 1,
            });
        }
        for frame in &mut trace.frames {
            let pickups = (frame.tick > delayed_take_tick) as u32
                + (frame.tick > 4 * RESPAWN_OUTPUT_TICK_GAP) as u32;
            let inventory = frame.inventory.as_mut().unwrap();
            inventory
                .quantities
                .retain(|item_id, _| *item_id != PUMPKIN_ID);
            if pickups > 0 {
                inventory.quantities.insert(PUMPKIN_ID, pickups);
            }
            inventory.occupied_slots = Some(pickups as u16);
            let status = frame.status.as_mut().unwrap();
            status.pickup_units = pickups;
            status.take_sends = main_take_ticks
                .iter()
                .filter(|tick| **tick < frame.tick)
                .count() as u32;
        }
        assert_eq!(
            check_b_contested_pumpkin(&trace).unwrap_err(),
            "item 1959 Take was outside the fresh-row 20-tick bound"
        );
    }

    #[test]
    fn cell_c_complete_fixture_passes_deposit_return_and_blocked_variants() {
        assert!(check_cell_c(&c_cell_fixture()).is_ok());
    }

    #[test]
    fn cell_c_actual_ui_then_deposit_order_passes_without_relabeling_pi() {
        check_cell_c(&c_same_edge_bank_packets_fixture())
            .expect("real same-edge UI/table packets precede the accepted click");
    }

    fn c_table_before_modal_binding_fixture() -> CellC {
        let mut cell = c_same_edge_bank_packets_fixture();
        for trace in [&mut cell.deposit_28, &mut cell.deposit_27_preserve_item] {
            let proof = &mut trace.packet_order;
            proof.modal.as_mut().unwrap().arrival_bank_generation = 5;
            let table = proof.table.as_mut().unwrap();
            table.arrival_bank_generation = 4;
            table.callback_sequence = 1109;
            table.packet_observation = 9;
            table.player_info_changed = true;
            table.ordinal = None;
            table.bank_snapshot = None;
            let binding = proof.table_binding.as_mut().unwrap();
            binding.bound_to_generation = 5;
            binding.packet_observation = 9;
            let deposit = proof.deposit.as_mut().unwrap();
            deposit.bank_generation = 5;
            deposit.observed_table_stamp = 9;
            deposit.ordinal.as_mut().unwrap().position = 2;
            proof.ambiguous_ui_batches = 1;
        }
        cell
    }

    #[test]
    fn cell_c_equal_stamp_binding_passes_both_deposits_without_relabeling_arrival() {
        let cell = c_table_before_modal_binding_fixture();
        check_cell_c(&cell).expect("the same real table binds before both generation-5 clicks");
        for proof in [
            &cell.deposit_28.packet_order,
            &cell.deposit_27_preserve_item.packet_order,
        ] {
            let table = proof.table.as_ref().unwrap();
            let binding = proof.table_binding.as_ref().unwrap();
            assert_eq!(table.arrival_bank_generation, 4);
            assert!(table.ordinal.is_none());
            assert!(table.bank_snapshot.is_none());
            assert_eq!(binding.bound_to_generation, 5);
            assert_eq!(binding.packet_observation, table.packet_observation);
            assert!(binding.bank_snapshot.as_ref().unwrap().complete);
            assert_eq!(proof.ambiguous_ui_batches, 1);
        }
    }

    #[test]
    fn cell_c_newer_table_before_deposit_fails_with_named_reason() {
        let mut cell = c_table_before_modal_binding_fixture();
        cell.deposit_28
            .packet_order
            .deposit
            .as_mut()
            .unwrap()
            .observed_table_stamp += 1;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "newer bank table packet arrived before the accepted deposit"
        );
    }

    #[test]
    fn cell_c_different_first_loaded_table_stamp_fails_with_named_binding_reason() {
        let mut cell = c_table_before_modal_binding_fixture();
        cell.deposit_28
            .packet_order
            .table_binding
            .as_mut()
            .unwrap()
            .packet_observation += 1;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "bank table binding does not prove its exact first-loaded generation"
        );
    }

    #[test]
    fn cell_c_missing_generation_5_snapshot_fails_with_named_reason() {
        let mut cell = c_table_before_modal_binding_fixture();
        cell.deposit_28
            .packet_order
            .table_binding
            .as_mut()
            .unwrap()
            .bank_snapshot = None;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "bound bank table lacks its complete generation snapshot"
        );
    }

    #[test]
    fn cell_c_table_ambiguity_that_can_change_the_order_still_fails_closed() {
        let mut cell = c_table_before_modal_binding_fixture();
        let proof = &mut cell.deposit_28.packet_order;
        proof.table.as_mut().unwrap().packet_observation =
            proof.modal.as_ref().unwrap().packet_observation + 1;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "bank UI packet arrival order is ambiguous across a PLAYER_INFO edge"
        );
    }

    #[test]
    fn cell_c_deposit_before_open_packet_fails_with_named_reason() {
        let mut cell = c_same_edge_bank_packets_fixture();
        let proof = &mut cell.deposit_28.packet_order;
        proof
            .modal
            .as_mut()
            .unwrap()
            .ordinal
            .as_mut()
            .unwrap()
            .position = 2;
        proof
            .table
            .as_mut()
            .unwrap()
            .ordinal
            .as_mut()
            .unwrap()
            .position = 3;
        proof
            .deposit
            .as_mut()
            .unwrap()
            .ordinal
            .as_mut()
            .unwrap()
            .position = 1;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "deposit packet did not strictly follow bank interface-open packet"
        );
    }

    #[test]
    fn cell_c_deposit_without_open_packet_fails_with_named_reason() {
        let mut cell = c_same_edge_bank_packets_fixture();
        cell.deposit_28.packet_order.modal = None;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "missing bank interface-open packet receipt"
        );
    }

    #[test]
    fn cell_c_open_packet_from_another_generation_fails_with_named_reason() {
        for different_session in [false, true] {
            let mut cell = c_same_edge_bank_packets_fixture();
            let modal = cell.deposit_28.packet_order.modal.as_mut().unwrap();
            if different_session {
                modal.session_generation += 1;
            } else {
                modal.arrival_bank_generation += 1;
            }
            assert_eq!(
                check_cell_c(&cell).unwrap_err(),
                "bank interface-open packet belongs to another session or bank generation"
            );
        }
    }

    #[test]
    fn cell_c_equal_deposit_and_open_ordinals_fail_with_named_reason() {
        let mut cell = c_same_edge_bank_packets_fixture();
        let proof = &mut cell.deposit_28.packet_order;
        proof.deposit.as_mut().unwrap().ordinal = proof.modal.as_ref().unwrap().ordinal;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "deposit packet did not strictly follow bank interface-open packet"
        );
    }

    #[test]
    fn cell_c_deposit_before_table_packet_fails_with_named_reason() {
        let mut cell = c_same_edge_bank_packets_fixture();
        let proof = &mut cell.deposit_28.packet_order;
        proof.deposit.as_mut().unwrap().ordinal = proof.table.as_ref().unwrap().ordinal;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "deposit packet did not strictly follow bank table packet"
        );
    }

    #[test]
    fn cell_c_missing_ui_ordinals_fail_with_named_reasons() {
        for missing_modal in [false, true] {
            let mut cell = c_same_edge_bank_packets_fixture();
            let proof = &mut cell.deposit_28.packet_order;
            let name = if missing_modal {
                proof.modal.as_mut().unwrap().ordinal = None;
                "interface-open"
            } else {
                proof.table.as_mut().unwrap().ordinal = None;
                "table"
            };
            assert_eq!(
                check_cell_c(&cell).unwrap_err(),
                format!("missing bank {name} packet arrival ordinal")
            );
        }
    }

    #[test]
    fn cell_c_missing_table_packet_fails_with_named_reason() {
        let mut cell = c_same_edge_bank_packets_fixture();
        cell.deposit_28.packet_order.table = None;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "missing bank table packet receipt"
        );
    }

    #[test]
    fn cell_c_ui_change_with_pi_fails_closed_with_named_ambiguity() {
        let mut cell = c_same_edge_bank_packets_fixture();
        let proof = &mut cell.deposit_28.packet_order;
        proof.modal.as_mut().unwrap().player_info_changed = true;
        proof.modal.as_mut().unwrap().ordinal = None;
        proof.ambiguous_ui_batches = 1;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "bank UI packet arrival order is ambiguous across a PLAYER_INFO edge"
        );
    }

    #[test]
    fn cell_c_requires_the_complete_actual_opening_table_baseline() {
        for invalid in 0..3 {
            let mut cell = c_same_edge_bank_packets_fixture();
            let table = cell.deposit_28.packet_order.table_binding.as_mut().unwrap();
            match invalid {
                0 => table.bank_snapshot = None,
                1 => table.bank_snapshot.as_mut().unwrap().complete = false,
                _ => {
                    table
                        .bank_snapshot
                        .as_mut()
                        .unwrap()
                        .quantities
                        .insert(PUMPKIN_ID, 6);
                }
            }
            assert_eq!(
                check_cell_c(&cell).unwrap_err(),
                if invalid == 2 {
                    "bank-open table packet does not match the observed bank baseline"
                } else {
                    "bound bank table lacks its complete generation snapshot"
                }
            );
        }
    }

    #[test]
    fn cell_c_deposit_requires_its_callback_to_have_observed_both_packets() {
        for invalid in 0..3 {
            let mut cell = c_same_edge_bank_packets_fixture();
            let deposit = cell.deposit_28.packet_order.deposit.as_mut().unwrap();
            match invalid {
                0 => deposit.observed_modal_stamp = 0,
                1 => deposit.observed_table_stamp = 0,
                _ => deposit.host_log_ordinal_fence = deposit.click_host_log_ordinal,
            }
            assert_eq!(
                check_cell_c(&cell).unwrap_err(),
                "deposit callback had not already observed both packets in its bank generation"
            );
        }
    }

    #[test]
    fn cell_c_rejects_absent_exact_anchor_return_receipt() {
        let mut cell = c_cell_fixture();
        cell.deposit_28.receipts.return_to_anchor = None;
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "missing exact-anchor return receipt"
        );
    }

    #[test]
    fn cell_c_rejects_receiver_seed_not_matching_pre_start_pack() {
        let mut cell = c_cell_fixture();
        let trace = &mut cell.deposit_27_preserve_item;
        let receiver = trace.identity.actor.clone();
        let action = trace
            .setup_seed_actions
            .iter_mut()
            .find(|action| action.actor == receiver)
            .unwrap();
        action.inventory.quantities.insert(PUMPKIN_ID, 25);
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "setup seed action inventory does not match its observed account baseline"
        );
    }

    #[test]
    fn cell_c_rejects_full_pack_without_receiver_seed_provenance() {
        let mut cell = c_cell_fixture();
        cell.inventory_blocked.setup_seed_actions.clear();
        assert_eq!(
            check_cell_c(&cell).unwrap_err(),
            "inventory-blocked setup must contain exactly one receiver seed action"
        );
    }

    #[test]
    fn cell_c_rejects_an_invented_active_key_or_unrelated_terminal_diagnostic() {
        let baseline = c_inventory_blocked_fixture();
        check_c_inventory_blocked(&baseline).expect("actual terminal stop fixture");
        let mut trace = baseline.clone();
        trace.terminal_stop.active_run_key_after_stop = Some("invented-live-key".into());
        assert_eq!(
            check_c_inventory_blocked(&trace).unwrap_err(),
            "inventory-blocked stop lacks its observed Start generation and retired active key"
        );
        let mut trace = baseline.clone();
        trace.terminal_stop.generation_after_stop += 1;
        assert_eq!(
            check_c_inventory_blocked(&trace).unwrap_err(),
            "inventory-blocked stop lacks its observed Start generation and retired active key"
        );
        let mut trace = baseline.clone();
        trace.terminal_stop.run_number += 1;
        assert_eq!(
            check_c_inventory_blocked(&trace).unwrap_err(),
            "inventory-blocked stop lacks its exact retained native diagnostic"
        );
        let mut trace = baseline.clone();
        trace.terminal_stop.failure_code = "different-failure".into();
        assert_eq!(
            check_c_inventory_blocked(&trace).unwrap_err(),
            "inventory-blocked stop lacks its exact retained native diagnostic"
        );
        let mut trace = baseline;
        trace.terminal_stop.native_phase = "Working".into();
        assert_eq!(
            check_c_inventory_blocked(&trace).unwrap_err(),
            "inventory-blocked stop lacks its exact retained native diagnostic"
        );
    }
}

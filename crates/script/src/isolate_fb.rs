//! FlatBuffers wire format for isolate IPC — schema: `crates/script/
//! schema/isolate.fbs`. Table codecs are generated from that schema into
//! `schema/generated/` (checked in; operators never need `flatc` at
//! `cargo test` time). Domain types, delta masking and [`IsolateBuf`]
//! (`reset`, not a fresh builder) stay here. The PLAYER_INFO snapshot
//! posted into each JS isolate and the shim interact batches forwarded
//! back are FlatBuffers, not JSON: a 50+ isolate wall never stringifies
//! or parses a JSON document per tick. Recorded paint frames cross that
//! channel as typed `ScriptPaint` values (both ends are this crate in
//! this process) and are capped by [`cap_paint`] before they are shared.
//!
//! Both thread-boundary messages are fully verified once on decode with the
//! bounded [`VerifierOptions`], so truncated or malicious buffers fail closed
//! instead of panicking or reading out of bounds. Nested reads then borrow the
//! already verified buffer without another pass.
//!
//! Posts are deltas (schema: `Snapshot`): `tick` is always carried, other
//! fields only when they changed vs the last post — an omitted vector is
//! absent, never empty, and the isolate keeps its last JS value for it.
//! The per-slot last-post [`SnapshotFingerprint`] is compared by value
//! (equality, not a hash) once per slot per tick.

use api::snapshot::{ActorKind, ProjectileView, MAX_PROJECTILES_PER_SNAPSHOT};
use flatbuffers::{
    root_with_opts, FlatBufferBuilder, InvalidFlatbuffer, VerifierOptions, WIPOffset,
};
use std::sync::Arc;

#[path = "../schema/generated/isolate_generated.rs"]
#[rustfmt::skip]
#[allow(dead_code, clippy::all, rustdoc::all)]
pub(crate) mod generated;
use generated::rs_2b_0t::isolate::*;
pub use generated::rs_2b_0t::isolate::{
    ApiGather, ApiGatherOutcome, ApiProgress, AvoidRect, BankApproach, BankStand, Booth, Carry,
    ChatLine, ChatOption, Collision, CombatProjectile, CombatStyle, InspectHop, Interact,
    InteractBatch, MainModalTexts, MakeButton, MakeProduct, NearestBooth, NpcBox, ProgressFlagRow,
    PuzzleBoard, QuestProgressRow, QuestStatus, Reach, Row, SceneEntity, SettingRow, SideTabIface,
    Snapshot, Stat, StatusField, Tile, Varp, WalkCancelReason, WidgetText,
};

fn isolate_verify_opts() -> VerifierOptions {
    VerifierOptions {
        max_depth: 64,
        max_tables: 10_000,
        max_apparent_size: 16 * 1024 * 1024,
        ignore_missing_null_terminator: false,
    }
}

macro_rules! presence_methods {
    ($($method:ident => $slot:ident),+ $(,)?) => {
        $(
            #[inline]
            pub fn $method(&self) -> bool {
                self._tab.vtable().get(Self::$slot) != 0
            }
        )+
    };
}

/// Max shim interact rows per tick (isolate→host).
const MAX_INTERACT_REQS: usize = 256;
/// Max paint lines per frame (isolate→host).
const MAX_PAINT_LINES: usize = 512;
/// Max advertised paint buttons per frame (isolate→host).
const MAX_PAINT_BUTTONS: usize = 32;
/// Max advertised tabs bands per paint frame (isolate→host).
const MAX_PAINT_TABS: usize = 16;
/// Max names on one strip/rail/tabs band.
const MAX_CHROME_NAMES: usize = 32;

/// Logical applet posted as `canvasRect`. Bound to `api::native_input::APPLET_*`.
pub const SNAPSHOT_CANVAS_W: i32 = api::native_input::APPLET_W;
pub const SNAPSHOT_CANVAS_H: i32 = api::native_input::APPLET_H;

const RUN_OPTION_ABSENT: u8 = 0;
const RUN_OPTION_FALSE_OR_FLOOR: u8 = 1;
const RUN_OPTION_TRUE_OR_NAN: u8 = 2;

/// A game tile `{x, z, level}`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TileInput {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// Compact native reach query posted on the isolate snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReachViewInput<'a> {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub walkable: &'a [u32],
    pub reachable: &'a [u32],
    pub reachable_adj: &'a [u32],
    pub exact_rank: &'a [u16],
    pub adjacent_rank: &'a [u16],
    pub step: &'a [u8],
    pub canlight: &'a [u32],
    /// Non-zero: fingerprint compares this stamp instead of copying vectors.
    pub stamp: u64,
}

impl ReachViewInput<'static> {
    /// Fail-closed view: no scene, empty dims, empty bitsets.
    pub const UNAVAILABLE: Self = Self {
        available: false,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: 0,
        height: 0,
        walkable: &[],
        reachable: &[],
        reachable_adj: &[],
        exact_rank: &[],
        adjacent_rank: &[],
        step: &[],
        canlight: &[],
        stamp: 0,
    };
}

/// One current-plane raw i32 collision grid posted on the isolate snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollisionViewInput<'a> {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub flags: &'a [i32],
}

impl CollisionViewInput<'static> {
    pub const UNAVAILABLE: Self = Self {
        available: false,
        base_x: 0,
        base_z: 0,
        level: 0,
        width: 0,
        height: 0,
        flags: &[],
    };
}

/// One skill row: the snapshot's stat index, name, xp, base, and effective.
#[derive(Clone, Copy)]
pub struct StatInput<'a> {
    pub index: i32,
    pub name: &'a str,
    pub xp: i32,
    pub base: i32,
    pub effective: i32,
}

/// A scene entity view posted into the isolate (npc/loc/player/ground).
#[derive(Clone, Copy)]
pub struct SceneEntityInput<'a> {
    pub index: i32,
    pub id: i32,
    pub name: Option<&'a str>,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
    pub health: i32,
    pub max_health: i32,
    pub in_combat: bool,
    pub animating: bool,
    pub actions: &'a [String],
    pub reachable: bool,
    pub reachable_adj: bool,
    pub combat_level: i32,
    /// `0` none, `1` npc, `2` player.
    pub target_kind: i32,
    /// `-1` when not facing anyone.
    pub target_index: i32,
    /// NPC packed size in tiles. `0` omits the new slots (loc/player/ground).
    pub size: i32,
    /// Path-head network SW x. Packed only with `size >= 1`.
    pub nx: i32,
    /// Path-head network SW z. Packed only with `size >= 1`.
    pub nz: i32,
    /// Placed loc shape; `0` for non-loc rows and omitted from their wire table.
    pub shape: i32,
    /// Placed loc angle; `0` for non-loc rows and omitted from their wire table.
    pub angle: i32,
}

/// One chat modal BUTTON_OK choice, including the exact component direct
/// catalog callers pass to `actions.ifButton`.
#[derive(Clone, Copy)]
pub struct ChatOptionInput<'a> {
    pub text: &'a str,
    pub com_id: i32,
}

/// One inv/bank/equipment row posted from the live snapshot.
#[derive(Clone, Copy)]
pub struct ItemRowInput<'a> {
    pub name: Option<&'a str>,
    pub count: i32,
    pub id: i32,
    pub ops: &'a [String],
    pub noted: bool,
    pub cert: i32,
    pub component_id: i32,
    pub slot: i32,
}

impl<'a> ItemRowInput<'a> {
    pub const fn nc(name: Option<&'a str>, count: i32) -> Self {
        Self {
            name,
            count,
            id: 0,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot: -1,
        }
    }
}

/// Posted side-tab root component id (`reader.sideTabInterface`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SideTabIfaceInput {
    pub index: i32,
    pub id: i32,
}

/// Posted game-chat ring line.
#[derive(Clone, Copy)]
pub struct ChatLineInput<'a> {
    pub seq: i32,
    pub text: &'a str,
    pub type_: i32,
    pub username: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub struct MakeButtonInput {
    pub qty: i32,
    pub com_id: i32,
}

#[derive(Clone, Copy)]
pub struct MakeProductInput<'a> {
    pub object_id: i32,
    pub name: &'a str,
    pub buttons: &'a [MakeButtonInput],
}

/// One combat-style varp-select button with its label.
#[derive(Clone, Copy)]
pub struct CombatStyleInput<'a> {
    pub mode: i32,
    pub label: &'a str,
    pub component_id: i32,
}

/// One varp index/value pair.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VarpInput {
    pub index: i32,
    pub value: i32,
}

/// A packed bank stand the shim walks to / opens. `kind` is `"booth"` or
/// `"npc"`; `op` is the stand's 1-based access op slot; `choose` is the
/// teller dialog option (deferred), `None` for a booth.
#[derive(Clone, Copy)]
pub struct BankStandInput<'a> {
    pub name: &'a str,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub kind: &'a str,
    pub op: i32,
    pub choose: Option<&'a str>,
}

/// The Rust-picked nearest Use-quickly booth on the player's plane.
pub struct NearestBoothInput<'a> {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub id: i32,
    pub name: &'a str,
    pub op: &'a str,
}

/// The host-observed PLAYER_INFO facts sent to the isolate, plus correlated
/// movement intent and walk-outcome reason. No cloned World. `inv`/`bank`/
/// `bank_side` rows carry the resolved obj name (`None` when the host table
/// has no name for the id — a script query never matches); `stats` the stat
/// index/name/xp; `booths` the scene locs with a `Use-quickly` action; `banks`
/// the packed stands; `hold`/`ours` the guardian's status for
/// `EventSignal.pending()`.
pub struct SnapshotInput<'a> {
    pub tick: u64,
    pub here: Option<TileInput>,
    pub ingame: bool,
    pub inv: &'a [ItemRowInput<'a>],
    /// The inv tab's slot count (28 bound, 0 while tutorial-locked) — the
    /// `reader.inventorySize()` read a script's onStart gates on.
    pub inv_size: i32,
    pub stats: &'a [StatInput<'a>],
    pub booths: &'a [TileInput],
    pub nearest_booth: Option<NearestBoothInput<'a>>,
    pub banks: &'a [BankStandInput<'a>],
    pub bank: &'a [ItemRowInput<'a>],
    pub bank_side: &'a [ItemRowInput<'a>],
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    pub count_dialog_open: bool,
    pub withdraw_x_result_seq: u64,
    pub withdraw_x_result: bool,
    pub hold: bool,
    pub ours: bool,
    pub npcs: &'a [SceneEntityInput<'a>],
    pub withdraw_load_result_seq: u64,
    pub withdraw_load_result: bool,
    pub bank_op_result_seq: u64,
    pub bank_op_result: bool,
    pub locs: &'a [SceneEntityInput<'a>],
    pub players: &'a [SceneEntityInput<'a>],
    pub ground: &'a [SceneEntityInput<'a>],
    pub equipment: &'a [ItemRowInput<'a>],
    pub chat_open: bool,
    pub chat_continue: bool,
    pub chat_text: Option<&'a str>,
    pub chat_options: &'a [ChatOptionInput<'a>],
    pub side_tab: i32,
    pub varps: &'a [VarpInput],
    pub combat_styles: &'a [CombatStyleInput<'a>],
    pub run_energy: i32,
    pub run_enabled: bool,
    pub retaliate_enabled: bool,
    pub my_name: Option<&'a str>,
    pub in_combat: bool,
    pub animating: bool,
    pub main_modal_id: i32,
    pub chat_modal_id: i32,
    pub make_products: &'a [MakeProductInput<'a>],
    pub side_tab_ifaces: &'a [SideTabIfaceInput],
    pub spell_buttons: &'a [CombatStyleInput<'a>],
    pub chat_lines: &'a [ChatLineInput<'a>],
    /// The bank Note toggle component id (-1 when absent).
    pub bank_note_on: i32,
    /// The bank Item toggle component id (-1 when absent).
    pub bank_note_off: i32,
    /// Client `GameSnapshot::scene_state` (2 = 3D ready).
    pub scene_state: i32,
    /// Local player run weight from `GameSnapshot::local_player`.
    pub weight: i32,
    /// Local player combat level from `GameSnapshot::local_player`.
    pub combat_level: i32,
    /// Orbit camera yaw (`CameraView::orbit_yaw`).
    pub camera_yaw: i32,
    /// Orbit camera pitch (`CameraView::orbit_pitch`).
    pub camera_pitch: i32,
    /// Whether packed nav last armed with `allow_teleports` (default off).
    pub teleports_enabled: bool,
    /// Local player table index (`GameSnapshot::self_slot`).
    pub self_slot: i32,
    pub trade_offer_open: bool,
    pub trade_confirm_open: bool,
    pub trade_partner: Option<&'a str>,
    pub trade_mine: &'a [ItemRowInput<'a>],
    pub trade_theirs: &'a [ItemRowInput<'a>],
    pub trade_side: &'a [ItemRowInput<'a>],
    pub trade_accept_id: i32,
    pub trade_decline_id: i32,
    pub shop_open: bool,
    pub shop_stock: &'a [ItemRowInput<'a>],
    /// Compact native reach query view. Always present on a keyframe;
    /// unavailable when `here` is missing or the scene is not available.
    pub reach: ReachViewInput<'a>,
    /// Local `Game.attackedByPlayer`: `face_entity >= PLAYER_FACE_BASE`.
    pub attacked_by_player: bool,
    /// Local decoded face: `0` none, `1` npc, `2` player.
    pub self_target_kind: i32,
    /// Local decoded face index. `-1` when kind is none.
    pub self_target_index: i32,
    /// Selected-world widget text rows. Absent id is not a stale IfType label.
    pub widgets: &'a [WidgetTextInput<'a>],
    /// Latest qualifying user movement intent, independent of a walk terminal.
    pub user_move_intent_seq: u64,
    /// Bounded cancellation reason for the latest posted walk outcome.
    pub walk_outcome_cancel_reason: WalkCancelReason,
}

/// Optional native facts appended to the isolate snapshot. Kept separate
/// from [`SnapshotInput`] so callers can omit these facts when unused.
#[derive(Clone, Copy, Default)]
pub struct NativeFactsInput<'a> {
    pub self_chat: Option<&'a str>,
    pub hint_tile: Option<(i32, i32)>,
    pub retaliate_controls: Option<(i32, i32)>,
    pub quest_statuses: Option<&'a [QuestStatusInput<'a>]>,
    /// The main modal's paired text walk. `None` = not supplied this post
    /// (omit the slot — the isolate keeps its last pair). `Some` with
    /// `root: -1, texts: []` = observed closed, a present table. The two
    /// are not equal, and an empty vector is still a supplied walk.
    pub main_modal_texts: Option<MainModalTextsInput<'a>>,
    /// The open puzzle board (rows + session generation). `None` = not
    /// supplied this post (omit both slots — the isolate keeps its last
    /// board); `Some` with `component_id: -1` = observed closed, a present
    /// table. The two are not equal. A present board always posts the
    /// generation in the same buffer.
    pub puzzle_board: Option<PuzzleBoardInput<'a>>,
    pub npc_boxes: Option<&'a [NpcBoxInput]>,
    /// The shop side interface's player pack rows (`shop_template_side:inv`,
    /// 3823). `None` = that container was not decoded this rebuild: Sell must
    /// fail closed rather than act on an empty stand-in. `Some([])` = decoded
    /// and empty.
    pub shop_player: Option<&'a [ItemRowInput<'a>]>,
    /// Main-modal skill-multi TYPE_INV rows. `None` = that panel was not
    /// decoded this rebuild (anvil ops fail closed). `Some([])` = decoded
    /// and empty.
    pub main_make: Option<&'a [ItemRowInput<'a>]>,
    /// Compact bank dest/readiness. `None` omits the field (delta keep /
    /// tests without a model). `Some([])` is an explicit empty table.
    pub bank_approaches: Option<&'a [BankApproachInput]>,
    /// Host-published walk outcome sequence. `0` on old buffers / never published.
    pub walk_outcome_seq: u64,
    pub walk_outcome_generation: u64,
    pub walk_outcome_failed: bool,
    pub walk_outcome_x: i32,
    pub walk_outcome_z: i32,
    pub walk_outcome_level: i32,
    pub walk_outcome_radius: i32,
    pub walk_outcome_allow_teleports: bool,
    pub walk_outcome_request_id: u64,
    /// The settled route end is frozen `'blocked'`: the player stands next
    /// to a last tile the live scene refuses. Part of the walk outcome.
    pub walk_outcome_blocked: bool,
    /// The walk outcome's navigator-named gate shorts. ALWAYS supplied — an
    /// empty slice is the observed "this outcome names no short", and the pack
    /// posts it with the family above in the same buffer, so a clear is never
    /// omitted. One family, not two: the rows ride inside
    /// [`Self::walk_outcome_seq`]'s delta.
    pub walk_missing_carry: &'a [CarryInput<'a>],
    pub route_inspect: RouteInspectFactsInput<'a>,
    /// Posted collision family. `None` omits the table (old callers / first
    /// post without Collision). `Some(UNAVAILABLE)` posts a clear.
    pub collision: Option<CollisionViewInput<'a>>,
    pub bank_selection: BankSelectionInput,
    /// The local player's primary animation id (frozen `reader.selfAnim()`,
    /// `-1` idle or no local player). `None` omits the slot (callers that
    /// do not observe it); the isolate keeps its last value.
    pub self_anim: Option<i32>,
    /// The host's live gather session. `None` means do not write this delta
    /// page; `Some` has a status only after the card publishes one.
    pub api_gather: Option<&'a crate::api_gather::GatherPage>,
    /// The retained terminal result. Absence means retain any prior outcome.
    pub api_gather_outcome: Option<&'a crate::api_gather::GatherEnd>,
    /// The most recently published progress acknowledgment or terminal.
    /// Absence omits the page and retains the isolate's prior value.
    pub api_progress: Option<&'a crate::api_progress::ProgressPage>,
    /// Bank item packet generation (`-1` while closed), not the open/close
    /// session identity. `None` omits the slot and keeps the last value.
    pub bank_snapshot_generation: Option<i64>,
    /// Live projectiles from the observed snapshot. `None` omits the page;
    /// `Some(&[])` explicitly clears it. Encoding keeps at most
    /// `MAX_PROJECTILES_PER_SNAPSHOT` projectiles targeted at `self_slot`.
    pub projectiles: Option<&'a [ProjectileView]>,
    /// Host-published native fingerprint for the current chat modal page.
    pub chat_page_fingerprint: u64,
}

/// A terminal select-only result. Its ordinal is resolved against Start's
/// immutable bank facts, never a JS-supplied bank object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BankSelectionInput {
    pub request_id: u64,
    pub generation: u64,
    pub bank_index: i32,
    pub kind: u8,
}

impl Default for BankSelectionInput {
    fn default() -> Self {
        Self {
            request_id: 0,
            generation: 0,
            bank_index: -1,
            kind: 0,
        }
    }
}

/// Host-published inspect family on the snapshot. All-zero is omitted / old buffer.
#[derive(Clone, Copy, Default)]
pub struct RouteInspectFactsInput<'a> {
    pub latest: RouteInspectTerminalInput<'a>,
    pub prev: RouteInspectTerminalInput<'a>,
    pub running_id: u64,
    pub pending_id: u64,
    pub accepted_id: u64,
    pub replaced_id: u64,
    pub replaced_prev_id: u64,
    pub refused_id: u64,
    pub refused_id_2: u64,
    pub refused_id_3: u64,
    pub unobserved: u64,
}

#[derive(Clone, Copy, Default)]
pub struct RouteInspectTerminalInput<'a> {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: Option<&'a str>,
    pub bank_planned: bool,
    pub ticks: f64,
    pub hops: &'a [InspectHopInput<'a>],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectHopInput<'a> {
    pub kind: &'a str,
    pub loc_id: i32,
    pub loc_name: &'a str,
    pub action: &'a str,
    pub option: i32,
    pub from_x: i32,
    pub from_z: i32,
    pub from_level: i32,
    pub to_x: i32,
    pub to_z: i32,
    pub to_level: i32,
    pub ticks: i32,
}

/// One native bank-booth dest + readiness row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BankApproachInput {
    pub loc_id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub can_operate: bool,
    pub dest_ok: bool,
    pub dest_x: i32,
    pub dest_z: i32,
    pub dest_level: i32,
}

/// One native NPC projection in overlay-canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpcBoxInput {
    pub index: i32,
    pub points: [(i32, i32); 8],
}

/// One quest-tab row after Rust resolves the native display colour.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct QuestStatusInput<'a> {
    pub name: &'a str,
    pub status: &'a str,
    /// The walked TYPE_TEXT id — the row's click target. `None` omits the
    /// slot (an old buffer / a row the walk had no id for); a present `0`
    /// is a real id and is posted. Not a sentinel: `-1` is never written
    /// for an absent id.
    pub component_id: Option<i32>,
}

/// The main modal's paired text walk: the root the walk used and its
/// TYPE_TEXT lines in walk order. Kept off [`SnapshotInput`] (like
/// [`NativeFactsInput`]) so one-shot callers that post `main_modal_id`
/// alone do not gain a pair they never walked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MainModalTextsInput<'a> {
    /// The root this walk was taken from — the same integer the buffer
    /// posts as `main_modal_id`. `-1` is an observed closed modal.
    pub root: i32,
    /// TYPE_TEXT lines in walk order, tags intact. Empty is a real walk
    /// (a closed modal, or an open one whose tree has no text).
    pub texts: &'a [String],
}

/// The open puzzle board's piece container, the rows it stores and its
/// session generation. ONE value: a present board always writes
/// `component_id`, `size` and `items` together, and the generation is
/// written in the same buffer (never without the table), so a buffer can
/// never carry one session's rows beside another session's generation.
/// Kept off [`SnapshotInput`] (like [`NativeFactsInput`]) so one-shot
/// callers that never walked a board do not gain an observation they did
/// not make.
#[derive(Clone, Copy)]
pub struct PuzzleBoardInput<'a> {
    /// The identified TYPE_INV component, or `-1` for an observed closed
    /// board (posted as `{ -1, 0, [] }` plus the generation, NOT omitted —
    /// an omitted slot keeps the isolate's last board).
    pub component_id: i32,
    /// The component's `link_obj_type` slot count, not `items.len()`: a
    /// wrong size is an observation and is posted as observed.
    pub size: i32,
    /// The identified widget's stored rows, sparse (an empty slot
    /// contributes no row). Empty is a real board with no pieces.
    pub items: &'a [ItemRowInput<'a>],
    /// Session identity of the board family. Bumps on session open, close
    /// or a new component id — never on a piece move.
    pub generation: u64,
}

/// One navigator-named gate short: an `item_req` stack the strict route
/// needed and the player's posted pack could not prove. `name` is the host obj
/// table's display name for `id` and is `None` when that table has none — the
/// join is always the id, so a row without a name is still posted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CarryInput<'a> {
    pub id: i32,
    pub count: i32,
    pub name: Option<&'a str>,
}

/// One currently posted widget text row (`reader.ifText`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WidgetTextInput<'a> {
    pub component_id: i32,
    pub text: &'a str,
    /// Number of observed TYPE_INV rows; `-1` for non-inventory widgets.
    pub item_count: i32,
}
/// Decode `buf` as a root-`Snapshot` FlatBuffer after bounded verification.
pub fn decode_snapshot(buf: &[u8]) -> Result<Snapshot<'_>, String> {
    Snapshot::from_bytes(buf)
}

impl<'a> Snapshot<'a> {
    pub fn from_bytes(buf: &'a [u8]) -> Result<Self, String> {
        let snapshot = root_as_snapshot_with_opts(&isolate_verify_opts(), buf)
            .map_err(|err: InvalidFlatbuffer| err.to_string())?;
        if snapshot.has_walk_outcome_cancel_reason()
            && !matches!(
                snapshot.walk_outcome_cancel_reason(),
                WalkCancelReason::None | WalkCancelReason::UserInput
            )
        {
            return Err("unknown walk outcome cancellation reason".to_string());
        }
        Ok(snapshot)
    }

    presence_methods! {
        has_tick => VT_TICK,
        has_here => VT_HERE,
        has_ingame => VT_INGAME,
        has_inv => VT_INV,
        has_inv_size => VT_INV_SIZE,
        has_stats => VT_STATS,
        has_booths => VT_BOOTHS,
        has_banks => VT_BANKS,
        has_bank => VT_BANK,
        has_bank_side => VT_BANK_SIDE,
        has_bank_open => VT_BANK_OPEN,
        has_bank_loaded => VT_BANK_LOADED,
        has_hold => VT_HOLD,
        has_ours => VT_OURS,
        has_npcs => VT_NPCS,
        has_locs => VT_LOCS,
        has_players => VT_PLAYERS,
        has_ground => VT_GROUND,
        has_equipment => VT_EQUIPMENT,
        has_chat_open => VT_CHAT_OPEN,
        has_chat_continue => VT_CHAT_CONTINUE,
        has_chat_text => VT_CHAT_TEXT,
        has_chat_options => VT_CHAT_OPTIONS,
        has_side_tab => VT_SIDE_TAB,
        has_varps => VT_VARPS,
        has_combat_styles => VT_COMBAT_STYLES,
        has_run_energy => VT_RUN_ENERGY,
        has_run_enabled => VT_RUN_ENABLED,
        has_retaliate_enabled => VT_RETALIATE_ENABLED,
        has_my_name => VT_MY_NAME,
        has_in_combat => VT_IN_COMBAT,
        has_animating => VT_ANIMATING,
        has_main_modal_id => VT_MAIN_MODAL_ID,
        has_chat_modal_id => VT_CHAT_MODAL_ID,
        has_make_products => VT_MAKE_PRODUCTS,
        has_side_tab_ifaces => VT_SIDE_TAB_IFACES,
        has_spell_buttons => VT_SPELL_BUTTONS,
        has_chat_lines => VT_CHAT_LINES,
        has_nearest_booth => VT_NEAREST_BOOTH,
        has_bank_note_on => VT_BANK_NOTE_ON,
        has_bank_note_off => VT_BANK_NOTE_OFF,
        has_scene_state => VT_SCENE_STATE,
        has_weight => VT_WEIGHT,
        has_camera_yaw => VT_CAMERA_YAW,
        has_camera_pitch => VT_CAMERA_PITCH,
        has_teleports_enabled => VT_TELEPORTS_ENABLED,
        has_self_slot => VT_SELF_SLOT,
        has_trade_offer_open => VT_TRADE_OFFER_OPEN,
        has_trade_confirm_open => VT_TRADE_CONFIRM_OPEN,
        has_trade_partner => VT_TRADE_PARTNER,
        has_trade_mine => VT_TRADE_MINE,
        has_trade_theirs => VT_TRADE_THEIRS,
        has_trade_side => VT_TRADE_SIDE,
        has_trade_accept_id => VT_TRADE_ACCEPT_ID,
        has_trade_decline_id => VT_TRADE_DECLINE_ID,
        has_shop_open => VT_SHOP_OPEN,
        has_shop_stock => VT_SHOP_STOCK,
        has_bank_generation => VT_BANK_GENERATION,
        has_count_dialog_open => VT_COUNT_DIALOG_OPEN,
        has_withdraw_x_result_seq => VT_WITHDRAW_X_RESULT_SEQ,
        has_withdraw_x_result => VT_WITHDRAW_X_RESULT,
        has_withdraw_load_result_seq => VT_WITHDRAW_LOAD_RESULT_SEQ,
        has_withdraw_load_result => VT_WITHDRAW_LOAD_RESULT,
        has_bank_op_result_seq => VT_BANK_OP_RESULT_SEQ,
        has_bank_op_result => VT_BANK_OP_RESULT,
        has_reach => VT_REACH,
        has_attacked_by_player => VT_ATTACKED_BY_PLAYER,
        has_widgets => VT_WIDGETS,
        has_self_chat => VT_SELF_CHAT,
        has_hint_tile_x => VT_HINT_TILE_X,
        has_hint_tile_z => VT_HINT_TILE_Z,
        has_retaliate_on_com_id => VT_RETALIATE_ON_COM_ID,
        has_retaliate_off_com_id => VT_RETALIATE_OFF_COM_ID,
        has_quest_statuses => VT_QUEST_STATUSES,
        has_quest_statuses_available => VT_QUEST_STATUSES_AVAILABLE,
        has_npc_boxes => VT_NPC_BOXES,
        has_npc_boxes_available => VT_NPC_BOXES_AVAILABLE,
        has_shop_player => VT_SHOP_PLAYER,
        has_shop_player_available => VT_SHOP_PLAYER_AVAILABLE,
        has_main_make => VT_MAIN_MAKE,
        has_main_make_available => VT_MAIN_MAKE_AVAILABLE,
        has_bank_approaches => VT_BANK_APPROACHES,
        has_user_move_intent_seq => VT_USER_MOVE_INTENT_SEQ,
        has_walk_outcome_cancel_reason => VT_WALK_OUTCOME_CANCEL_REASON,

        has_walk_outcome_seq => VT_WALK_OUTCOME_SEQ,
        has_walk_outcome_generation => VT_WALK_OUTCOME_GENERATION,
        has_walk_outcome_failed => VT_WALK_OUTCOME_FAILED,
        has_walk_outcome_x => VT_WALK_OUTCOME_X,
        has_walk_outcome_z => VT_WALK_OUTCOME_Z,
        has_walk_outcome_level => VT_WALK_OUTCOME_LEVEL,
        has_walk_outcome_radius => VT_WALK_OUTCOME_RADIUS,
        has_walk_outcome_allow_teleports => VT_WALK_OUTCOME_ALLOW_TELEPORTS,
        has_walk_outcome_request_id => VT_WALK_OUTCOME_REQUEST_ID,
        has_canvas_width => VT_CANVAS_WIDTH,
        has_canvas_height => VT_CANVAS_HEIGHT,
        has_combat_level => VT_COMBAT_LEVEL,
        has_route_inspect_seq => VT_ROUTE_INSPECT_SEQ,
        has_route_inspect_generation => VT_ROUTE_INSPECT_GENERATION,
        has_route_inspect_request_id => VT_ROUTE_INSPECT_REQUEST_ID,
        has_route_inspect_ok => VT_ROUTE_INSPECT_OK,
        has_route_inspect_reason => VT_ROUTE_INSPECT_REASON,
        has_route_inspect_bank_planned => VT_ROUTE_INSPECT_BANK_PLANNED,
        has_route_inspect_ticks => VT_ROUTE_INSPECT_TICKS,
        has_route_inspect_hops => VT_ROUTE_INSPECT_HOPS,
        has_route_inspect_prev_seq => VT_ROUTE_INSPECT_PREV_SEQ,
        has_route_inspect_prev_generation => VT_ROUTE_INSPECT_PREV_GENERATION,
        has_route_inspect_prev_request_id => VT_ROUTE_INSPECT_PREV_REQUEST_ID,
        has_route_inspect_prev_ok => VT_ROUTE_INSPECT_PREV_OK,
        has_route_inspect_prev_reason => VT_ROUTE_INSPECT_PREV_REASON,
        has_route_inspect_prev_bank_planned => VT_ROUTE_INSPECT_PREV_BANK_PLANNED,
        has_route_inspect_prev_ticks => VT_ROUTE_INSPECT_PREV_TICKS,
        has_route_inspect_prev_hops => VT_ROUTE_INSPECT_PREV_HOPS,
        has_route_inspect_running_id => VT_ROUTE_INSPECT_RUNNING_ID,
        has_route_inspect_pending_id => VT_ROUTE_INSPECT_PENDING_ID,
        has_route_inspect_accepted_id => VT_ROUTE_INSPECT_ACCEPTED_ID,
        has_route_inspect_replaced_id => VT_ROUTE_INSPECT_REPLACED_ID,
        has_route_inspect_replaced_prev_id => VT_ROUTE_INSPECT_REPLACED_PREV_ID,
        has_route_inspect_refused_id => VT_ROUTE_INSPECT_REFUSED_ID,
        has_route_inspect_refused_id_2 => VT_ROUTE_INSPECT_REFUSED_ID_2,
        has_route_inspect_refused_id_3 => VT_ROUTE_INSPECT_REFUSED_ID_3,
        has_route_inspect_unobserved => VT_ROUTE_INSPECT_UNOBSERVED,
        has_collision => VT_COLLISION,
        has_self_target_kind => VT_SELF_TARGET_KIND,
        has_self_target_index => VT_SELF_TARGET_INDEX,
        has_main_modal_texts => VT_MAIN_MODAL_TEXTS,
        has_puzzle_board => VT_PUZZLE_BOARD,
        has_puzzle_board_generation => VT_PUZZLE_BOARD_GENERATION,
        has_walk_missing_carry => VT_WALK_MISSING_CARRY,
        has_bank_selection_request_id => VT_BANK_SELECTION_REQUEST_ID,
        has_bank_selection_generation => VT_BANK_SELECTION_GENERATION,
        has_bank_selection_index => VT_BANK_SELECTION_INDEX,
        has_bank_selection_kind => VT_BANK_SELECTION_KIND,
        has_self_anim => VT_SELF_ANIM,
        has_walk_outcome_blocked => VT_WALK_OUTCOME_BLOCKED,
        has_bank_snapshot_generation => VT_BANK_SNAPSHOT_GENERATION,
        has_api_gather => VT_API_GATHER,
        has_api_gather_outcome => VT_API_GATHER_OUTCOME,
        has_api_progress => VT_API_PROGRESS,
        has_chat_page_fingerprint => VT_CHAT_PAGE_FINGERPRINT,
    }

    pub fn bank_selection(&self) -> Option<BankSelectionInput> {
        self.has_bank_selection_request_id()
            .then(|| BankSelectionInput {
                request_id: self.bank_selection_request_id(),
                generation: self.bank_selection_generation(),
                bank_index: self.bank_selection_index(),
                kind: self.bank_selection_kind(),
            })
    }

    pub fn has_hint_tile(&self) -> bool {
        self.has_hint_tile_x() || self.has_hint_tile_z()
    }

    pub fn hint_tile(&self) -> Option<(i32, i32)> {
        let x = self.hint_tile_x();
        let z = self.hint_tile_z();
        (x >= 0 && z >= 0).then_some((x, z))
    }

    pub fn has_retaliate_controls(&self) -> bool {
        self.has_retaliate_on_com_id() || self.has_retaliate_off_com_id()
    }

    pub fn retaliate_controls(&self) -> Option<(i32, i32)> {
        let on = self.retaliate_on_com_id();
        let off = self.retaliate_off_com_id();
        (on >= 0 && off >= 0).then_some((on, off))
    }
}

impl Row<'_> {
    presence_methods! {
        has_component_id => VT_COMPONENT_ID,
        has_slot => VT_SLOT,
    }
}

impl Carry<'_> {
    presence_methods! {
        has_name => VT_NAME,
    }
}

impl Interact<'_> {
    presence_methods! {
        has_x => VT_X,
        has_z => VT_Z,
        has_level => VT_LEVEL,
    }
}

impl<'a> InteractBatch<'a> {
    pub fn from_bytes(buf: &'a [u8]) -> Result<Self, String> {
        root_with_opts::<Self>(&isolate_verify_opts(), buf)
            .map_err(|err: InvalidFlatbuffer| err.to_string())
    }
}

/// One packed bank stand as an owned fingerprint row (same fields as
/// [`BankStandInput`], names cloned).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BankStandFp {
    pub name: String,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub kind: String,
    pub op: i32,
    pub choose: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SceneEntityFp {
    pub index: i32,
    pub id: i32,
    pub name: Option<String>,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
    pub health: i32,
    pub max_health: i32,
    pub in_combat: bool,
    pub animating: bool,
    pub actions: Vec<String>,
    pub reachable: bool,
    pub reachable_adj: bool,
    pub combat_level: i32,
    pub target_kind: i32,
    pub target_index: i32,
    pub size: i32,
    pub nx: i32,
    pub nz: i32,
    pub shape: i32,
    pub angle: i32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ItemRowFp {
    pub name: Option<String>,
    pub count: i32,
    pub id: i32,
    pub ops: Vec<String>,
    pub noted: bool,
    pub cert: i32,
    pub component_id: i32,
    pub slot: i32,
}

/// One posted puzzle board as an owned fingerprint row. The generation is a
/// member, not a sibling: the board and its session generation are ONE delta
/// family, so a comparison can never move one without the other.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PuzzleBoardFp {
    pub component_id: i32,
    pub size: i32,
    pub items: Vec<ItemRowFp>,
    pub generation: u64,
}

/// One navigator-named gate short as an owned fingerprint row. The name is a
/// member: the display it resolves is part of the posted observation, and a
/// row the host obj table stops naming still re-posts its id and count.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CarryFp {
    pub id: i32,
    pub count: i32,
    pub name: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CombatStyleFp {
    pub mode: i32,
    pub label: String,
    pub component_id: i32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MakeButtonFp {
    pub qty: i32,
    pub com_id: i32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MakeProductFp {
    pub object_id: i32,
    pub name: String,
    pub buttons: Vec<MakeButtonFp>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NearestBoothFp {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub id: i32,
    pub name: String,
    pub op: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CollisionViewFp {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub flags: Arc<[i32]>,
}

fn collision_fp(
    last: Option<&CollisionViewFp>,
    input: Option<CollisionViewInput<'_>>,
) -> CollisionViewFp {
    let Some(input) = input else {
        return last.cloned().unwrap_or_default();
    };
    if let Some(prev) = last {
        if prev.available == input.available
            && prev.base_x == input.base_x
            && prev.base_z == input.base_z
            && prev.level == input.level
            && prev.width == input.width
            && prev.height == input.height
            && prev.flags.as_ref() == input.flags
        {
            return prev.clone();
        }
    }
    CollisionViewFp {
        available: input.available,
        base_x: input.base_x,
        base_z: input.base_z,
        level: input.level,
        width: input.width,
        height: input.height,
        flags: Arc::from(input.flags),
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ReachViewFp {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub walkable: Vec<u32>,
    pub reachable: Vec<u32>,
    pub reachable_adj: Vec<u32>,
    pub exact_rank: Vec<u16>,
    pub adjacent_rank: Vec<u16>,
    pub step: Vec<u8>,
    pub canlight: Vec<u32>,
    pub stamp: u64,
}

fn reach_fp(r: &ReachViewInput<'_>) -> ReachViewFp {
    if r.stamp != 0 {
        return ReachViewFp {
            available: r.available,
            base_x: r.base_x,
            base_z: r.base_z,
            level: r.level,
            width: r.width,
            height: r.height,
            stamp: r.stamp,
            ..ReachViewFp::default()
        };
    }
    ReachViewFp {
        available: r.available,
        base_x: r.base_x,
        base_z: r.base_z,
        level: r.level,
        width: r.width,
        height: r.height,
        walkable: r.walkable.to_vec(),
        reachable: r.reachable.to_vec(),
        reachable_adj: r.reachable_adj.to_vec(),
        exact_rank: r.exact_rank.to_vec(),
        adjacent_rank: r.adjacent_rank.to_vec(),
        step: r.step.to_vec(),
        canlight: r.canlight.to_vec(),
        stamp: 0,
    }
}
/// The live gather page's delta identity. Keeping the status `Arc` alive is
/// deliberate: pointer identity cannot be confused by allocator address reuse.
#[derive(Clone)]
pub struct ApiGatherFp {
    token: u64,
    phase: u8,
    status: Option<Arc<crate::native::ScriptStatus>>,
}

impl PartialEq for ApiGatherFp {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token
            && self.phase == other.phase
            && match (&self.status, &other.status) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}

impl Eq for ApiGatherFp {}

impl From<&crate::api_gather::GatherPage> for ApiGatherFp {
    fn from(page: &crate::api_gather::GatherPage) -> Self {
        Self {
            token: page.token,
            phase: match page.phase {
                crate::api_gather::GatherPhase::Preparing => 1,
                crate::api_gather::GatherPhase::Running => 2,
            },
            status: page.status.clone(),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ProjectileFingerprintRow {
    spotanim: i32,
    target_player_index: i32,
}

/// Allocation-free fingerprint for the optional bounded projectile page.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectilePageFingerprint {
    len: usize,
    rows: [ProjectileFingerprintRow; MAX_PROJECTILES_PER_SNAPSHOT],
}

impl ProjectilePageFingerprint {
    fn from_input(rows: &[ProjectileView], local_player_slot: i32) -> Self {
        let mut page = Self::default();
        for projectile in local_target_projectiles(rows, local_player_slot) {
            let index = page.len;
            page.rows[index] = ProjectileFingerprintRow {
                spotanim: projectile.spotanim,
                target_player_index: local_player_slot,
            };
            page.len = index + 1;
        }
        page
    }
}

fn local_target_projectiles<'a>(
    rows: &'a [ProjectileView],
    local_player_slot: i32,
) -> impl Iterator<Item = &'a ProjectileView> + 'a {
    let local_slot = usize::try_from(local_player_slot).ok();
    rows.iter()
        .filter(move |projectile| {
            local_slot.is_some_and(|slot| {
                projectile
                    .target
                    .is_some_and(|target| target.kind == ActorKind::Player && target.index == slot)
            })
        })
        .take(MAX_PROJECTILES_PER_SNAPSHOT)
}

/// The per-slot last-post fingerprint: an owned copy of the snapshot
/// fields the host last posted, compared against the next input to build
/// the delta. Content equality (not a hash) is fine — the tables are
/// small and the compare runs once per slot per tick.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SnapshotFingerprint {
    pub here: Option<TileInput>,
    pub ingame: bool,
    pub inv: Vec<ItemRowFp>,
    pub inv_size: i32,
    pub stats: Vec<(i32, String, i32, i32, i32)>,
    pub booths: Vec<TileInput>,
    pub nearest_booth: Option<NearestBoothFp>,
    pub banks: Vec<BankStandFp>,
    pub bank: Vec<ItemRowFp>,
    pub bank_side: Vec<ItemRowFp>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    pub count_dialog_open: bool,
    pub withdraw_x_result_seq: u64,
    pub withdraw_x_result: bool,
    pub hold: bool,
    pub ours: bool,
    pub npcs: Vec<SceneEntityFp>,
    pub withdraw_load_result_seq: u64,
    pub withdraw_load_result: bool,
    pub bank_op_result_seq: u64,
    pub bank_op_result: bool,
    pub locs: Vec<SceneEntityFp>,
    pub players: Vec<SceneEntityFp>,
    pub ground: Vec<SceneEntityFp>,
    pub equipment: Vec<ItemRowFp>,
    pub chat_open: bool,
    pub chat_continue: bool,
    pub chat_text: Option<String>,
    pub chat_options: Vec<(String, i32)>,
    pub side_tab: i32,
    pub varps: Vec<VarpInput>,
    pub combat_styles: Vec<CombatStyleFp>,
    pub run_energy: i32,
    pub run_enabled: bool,
    pub retaliate_enabled: bool,
    pub my_name: Option<String>,
    pub in_combat: bool,
    pub animating: bool,
    pub main_modal_id: i32,
    pub chat_modal_id: i32,
    pub make_products: Vec<MakeProductFp>,
    pub side_tab_ifaces: Vec<SideTabIfaceInput>,
    pub spell_buttons: Vec<CombatStyleFp>,
    pub chat_lines: Vec<(i32, String, i32, Option<String>)>,
    pub bank_note_on: i32,
    pub bank_note_off: i32,
    pub scene_state: i32,
    pub weight: i32,
    pub combat_level: i32,
    pub camera_yaw: i32,
    pub camera_pitch: i32,
    pub teleports_enabled: bool,
    pub self_slot: i32,
    pub trade_offer_open: bool,
    pub trade_confirm_open: bool,
    pub trade_partner: Option<String>,
    pub trade_mine: Vec<ItemRowFp>,
    pub trade_theirs: Vec<ItemRowFp>,
    pub trade_side: Vec<ItemRowFp>,
    pub trade_accept_id: i32,
    pub trade_decline_id: i32,
    pub shop_open: bool,
    pub shop_stock: Vec<ItemRowFp>,
    pub shop_player: Option<Vec<ItemRowFp>>,
    pub main_make: Option<Vec<ItemRowFp>>,
    pub reach: ReachViewFp,
    pub attacked_by_player: bool,
    pub self_target_kind: i32,
    pub self_target_index: i32,
    pub widgets: Vec<(i32, String, i32)>,
    pub self_chat: Option<String>,
    pub hint_tile: Option<(i32, i32)>,
    pub retaliate_controls: Option<(i32, i32)>,
    /// `(name, status, component_id)` per posted row. The id is part of the
    /// comparison: a name/status match after a quiet interface rebuild
    /// would keep a row whose click target is stale or gone.
    pub quest_statuses: Option<Vec<(String, String, Option<i32>)>>,
    /// `(root, texts)` — the whole pair or nothing. A missing pair is not
    /// a closed modal, so the comparison cannot tear lines off the root.
    pub main_modal_texts: Option<(i32, Vec<String>)>,
    /// The whole board observation (identity, size, rows, generation) or
    /// nothing — one family, so a delta cannot post one half.
    pub puzzle_board: Option<PuzzleBoardFp>,
    pub npc_boxes: Option<Vec<NpcBoxInput>>,
    /// `(spotanim, target player slot)` — only policy-relevant projectile facts.
    pub projectiles: Option<ProjectilePageFingerprint>,
    pub bank_approaches: Option<Vec<BankApproachInput>>,
    pub user_move_intent_seq: u64,

    pub walk_outcome_seq: u64,
    pub walk_outcome_generation: u64,
    pub walk_outcome_failed: bool,
    pub walk_outcome_x: i32,
    pub walk_outcome_z: i32,
    pub walk_outcome_level: i32,
    pub walk_outcome_radius: i32,
    pub walk_outcome_allow_teleports: bool,
    pub walk_outcome_request_id: u64,
    pub walk_outcome_blocked: bool,
    pub walk_outcome_cancel_reason: WalkCancelReason,
    /// The walk outcome's named shorts. Part of that family: a list that moved
    /// without a scalar moving still re-posts the family, so a clear is never
    /// left to a stale keep.
    pub walk_missing_carry: Vec<CarryFp>,
    pub route_inspect: RouteInspectFp,
    pub collision: CollisionViewFp,
    pub bank_selection: BankSelectionInput,
    pub self_anim: Option<i32>,
    /// `(token, phase, status Arc identity)` for the live host session.
    pub api_gather: Option<ApiGatherFp>,
    /// The most recent terminal token; terminals are retained, never cleared
    /// by a delta.
    pub api_gather_outcome: Option<u64>,
    /// The current page identity; only a new `(token, kind)` is posted.
    pub api_progress: Option<(u64, u8)>,
    pub bank_snapshot_generation: Option<i64>,
    pub chat_page_fingerprint: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RouteInspectFp {
    pub latest: RouteInspectTerminalFp,
    pub prev: RouteInspectTerminalFp,
    pub running_id: u64,
    pub pending_id: u64,
    pub accepted_id: u64,
    pub replaced_id: u64,
    pub replaced_prev_id: u64,
    pub refused_id: u64,
    pub refused_id_2: u64,
    pub refused_id_3: u64,
    pub unobserved: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RouteInspectTerminalFp {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: String,
    pub bank_planned: bool,
    pub ticks_bits: u64,
    pub hops: Vec<InspectHopFp>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct InspectHopFp {
    pub kind: String,
    pub loc_id: i32,
    pub loc_name: String,
    pub action: String,
    pub option: i32,
    pub from_x: i32,
    pub from_z: i32,
    pub from_level: i32,
    pub to_x: i32,
    pub to_z: i32,
    pub to_level: i32,
    pub ticks: i32,
}

impl SnapshotFingerprint {
    /// Own the input's field values (names cloned) for later comparison.
    pub fn from_input(input: &SnapshotInput<'_>) -> SnapshotFingerprint {
        Self::from_input_with_native(input, NativeFactsInput::default())
    }

    pub fn from_input_with_native(
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) -> SnapshotFingerprint {
        fn item_row_fp(r: &ItemRowInput<'_>) -> ItemRowFp {
            ItemRowFp {
                name: r.name.map(str::to_string),
                count: r.count,
                id: r.id,
                ops: r.ops.iter().map(|a| a.to_string()).collect(),
                noted: r.noted,
                cert: r.cert,
                component_id: r.component_id,
                slot: r.slot,
            }
        }
        fn entity_fp(e: &SceneEntityInput<'_>) -> SceneEntityFp {
            SceneEntityFp {
                index: e.index,
                id: e.id,
                name: e.name.map(str::to_string),
                x: e.x,
                z: e.z,
                level: e.level,
                distance: e.distance,
                health: e.health,
                max_health: e.max_health,
                in_combat: e.in_combat,
                animating: e.animating,
                actions: e.actions.iter().map(|a| a.to_string()).collect(),
                reachable: e.reachable,
                reachable_adj: e.reachable_adj,
                combat_level: e.combat_level,
                target_kind: e.target_kind,
                target_index: e.target_index,
                size: e.size,
                nx: e.nx,
                nz: e.nz,
                shape: e.shape,
                angle: e.angle,
            }
        }
        SnapshotFingerprint {
            here: input.here,
            ingame: input.ingame,
            inv: input.inv.iter().map(item_row_fp).collect(),
            inv_size: input.inv_size,
            stats: input
                .stats
                .iter()
                .map(|s| (s.index, s.name.to_string(), s.xp, s.base, s.effective))
                .collect(),
            booths: input.booths.to_vec(),
            nearest_booth: input.nearest_booth.as_ref().map(|b| NearestBoothFp {
                x: b.x,
                z: b.z,
                level: b.level,
                id: b.id,
                name: b.name.to_string(),
                op: b.op.to_string(),
            }),
            banks: input
                .banks
                .iter()
                .map(|b| BankStandFp {
                    name: b.name.to_string(),
                    x: b.x,
                    z: b.z,
                    level: b.level,
                    kind: b.kind.to_string(),
                    op: b.op,
                    choose: b.choose.map(str::to_string),
                })
                .collect(),
            bank: input.bank.iter().map(item_row_fp).collect(),
            bank_side: input.bank_side.iter().map(item_row_fp).collect(),
            bank_open: input.bank_open,
            bank_loaded: input.bank_loaded,
            bank_generation: input.bank_generation,
            count_dialog_open: input.count_dialog_open,
            withdraw_x_result_seq: input.withdraw_x_result_seq,
            withdraw_x_result: input.withdraw_x_result,
            withdraw_load_result_seq: input.withdraw_load_result_seq,
            withdraw_load_result: input.withdraw_load_result,
            bank_op_result_seq: input.bank_op_result_seq,
            bank_op_result: input.bank_op_result,
            hold: input.hold,
            ours: input.ours,
            npcs: input.npcs.iter().map(entity_fp).collect(),
            locs: input.locs.iter().map(entity_fp).collect(),
            players: input.players.iter().map(entity_fp).collect(),
            ground: input.ground.iter().map(entity_fp).collect(),
            equipment: input.equipment.iter().map(item_row_fp).collect(),
            chat_open: input.chat_open,
            chat_continue: input.chat_continue,
            chat_text: input.chat_text.map(str::to_string),
            chat_options: input
                .chat_options
                .iter()
                .map(|o| (o.text.to_string(), o.com_id))
                .collect(),
            side_tab: input.side_tab,
            varps: input.varps.to_vec(),
            combat_styles: input
                .combat_styles
                .iter()
                .map(|c| CombatStyleFp {
                    mode: c.mode,
                    label: c.label.to_string(),
                    component_id: c.component_id,
                })
                .collect(),
            run_energy: input.run_energy,
            run_enabled: input.run_enabled,
            retaliate_enabled: input.retaliate_enabled,
            my_name: input.my_name.map(str::to_string),
            in_combat: input.in_combat,
            animating: input.animating,
            main_modal_id: input.main_modal_id,
            chat_modal_id: input.chat_modal_id,
            make_products: input
                .make_products
                .iter()
                .map(|p| MakeProductFp {
                    object_id: p.object_id,
                    name: p.name.to_string(),
                    buttons: p
                        .buttons
                        .iter()
                        .map(|b| MakeButtonFp {
                            qty: b.qty,
                            com_id: b.com_id,
                        })
                        .collect(),
                })
                .collect(),
            side_tab_ifaces: input.side_tab_ifaces.to_vec(),
            spell_buttons: input
                .spell_buttons
                .iter()
                .map(|c| CombatStyleFp {
                    mode: c.mode,
                    label: c.label.to_string(),
                    component_id: c.component_id,
                })
                .collect(),
            chat_lines: input
                .chat_lines
                .iter()
                .map(|l| {
                    (
                        l.seq,
                        l.text.to_string(),
                        l.type_,
                        l.username.map(str::to_string),
                    )
                })
                .collect(),
            bank_note_on: input.bank_note_on,
            bank_note_off: input.bank_note_off,
            scene_state: input.scene_state,
            weight: input.weight,
            combat_level: input.combat_level,
            camera_yaw: input.camera_yaw,
            camera_pitch: input.camera_pitch,
            teleports_enabled: input.teleports_enabled,
            self_slot: input.self_slot,
            trade_offer_open: input.trade_offer_open,
            trade_confirm_open: input.trade_confirm_open,
            trade_partner: input.trade_partner.map(str::to_string),
            trade_mine: input.trade_mine.iter().map(item_row_fp).collect(),
            trade_theirs: input.trade_theirs.iter().map(item_row_fp).collect(),
            trade_side: input.trade_side.iter().map(item_row_fp).collect(),
            trade_accept_id: input.trade_accept_id,
            trade_decline_id: input.trade_decline_id,
            shop_open: input.shop_open,
            shop_stock: input.shop_stock.iter().map(item_row_fp).collect(),
            shop_player: native
                .shop_player
                .map(|rows| rows.iter().map(item_row_fp).collect()),
            main_make: native
                .main_make
                .map(|rows| rows.iter().map(item_row_fp).collect()),
            reach: reach_fp(&input.reach),
            attacked_by_player: input.attacked_by_player,
            self_target_kind: input.self_target_kind,
            self_target_index: input.self_target_index,
            widgets: input
                .widgets
                .iter()
                .map(|w| (w.component_id, w.text.to_string(), w.item_count))
                .collect(),
            self_chat: native.self_chat.map(str::to_string),
            hint_tile: native.hint_tile,
            retaliate_controls: native.retaliate_controls,
            quest_statuses: native.quest_statuses.map(|rows| {
                rows.iter()
                    .map(|q| (q.name.to_string(), q.status.to_string(), q.component_id))
                    .collect()
            }),
            main_modal_texts: native
                .main_modal_texts
                .map(|pair| (pair.root, pair.texts.to_vec())),
            puzzle_board: native.puzzle_board.map(|board| PuzzleBoardFp {
                component_id: board.component_id,
                size: board.size,
                items: board.items.iter().map(item_row_fp).collect(),
                generation: board.generation,
            }),
            npc_boxes: native.npc_boxes.map(<[NpcBoxInput]>::to_vec),
            projectiles: native
                .projectiles
                .map(|rows| ProjectilePageFingerprint::from_input(rows, input.self_slot)),
            bank_approaches: native.bank_approaches.map(<[BankApproachInput]>::to_vec),
            user_move_intent_seq: input.user_move_intent_seq,
            walk_outcome_seq: native.walk_outcome_seq,
            walk_outcome_generation: native.walk_outcome_generation,
            walk_outcome_failed: native.walk_outcome_failed,
            walk_outcome_x: native.walk_outcome_x,
            walk_outcome_z: native.walk_outcome_z,
            walk_outcome_level: native.walk_outcome_level,
            walk_outcome_radius: native.walk_outcome_radius,
            walk_outcome_allow_teleports: native.walk_outcome_allow_teleports,
            walk_outcome_request_id: native.walk_outcome_request_id,
            walk_outcome_blocked: native.walk_outcome_blocked,
            walk_outcome_cancel_reason: input.walk_outcome_cancel_reason,
            walk_missing_carry: native
                .walk_missing_carry
                .iter()
                .map(|row| CarryFp {
                    id: row.id,
                    count: row.count,
                    name: row.name.map(str::to_string),
                })
                .collect(),
            route_inspect: route_inspect_fp(&native.route_inspect),
            collision: collision_fp(None, native.collision),
            bank_selection: native.bank_selection,
            self_anim: native.self_anim,
            bank_snapshot_generation: native.bank_snapshot_generation,
            api_gather: native.api_gather.map(ApiGatherFp::from),
            api_gather_outcome: native.api_gather_outcome.map(|end| end.token()),
            api_progress: native.api_progress.map(|page| (page.token(), page.kind())),
            chat_page_fingerprint: native.chat_page_fingerprint,
        }
    }
}

fn inspect_hop_fp(h: &InspectHopInput<'_>) -> InspectHopFp {
    InspectHopFp {
        kind: h.kind.to_string(),
        loc_id: h.loc_id,
        loc_name: h.loc_name.to_string(),
        action: h.action.to_string(),
        option: h.option,
        from_x: h.from_x,
        from_z: h.from_z,
        from_level: h.from_level,
        to_x: h.to_x,
        to_z: h.to_z,
        to_level: h.to_level,
        ticks: h.ticks,
    }
}

fn route_inspect_terminal_fp(t: &RouteInspectTerminalInput<'_>) -> RouteInspectTerminalFp {
    RouteInspectTerminalFp {
        seq: t.seq,
        generation: t.generation,
        request_id: t.request_id,
        ok: t.ok,
        reason: t.reason.unwrap_or("").to_string(),
        bank_planned: t.bank_planned,
        ticks_bits: t.ticks.to_bits(),
        hops: t.hops.iter().map(inspect_hop_fp).collect(),
    }
}

fn route_inspect_fp(facts: &RouteInspectFactsInput<'_>) -> RouteInspectFp {
    RouteInspectFp {
        latest: route_inspect_terminal_fp(&facts.latest),
        prev: route_inspect_terminal_fp(&facts.prev),
        running_id: facts.running_id,
        pending_id: facts.pending_id,
        accepted_id: facts.accepted_id,
        replaced_id: facts.replaced_id,
        replaced_prev_id: facts.replaced_prev_id,
        refused_id: facts.refused_id,
        refused_id_2: facts.refused_id_2,
        refused_id_3: facts.refused_id_3,
        unobserved: facts.unobserved,
    }
}

/// Which snapshot fields a delta carries. `tick` is always carried; every
/// other field only when it changed vs the last post (or on the keyframe —
/// first post / isolate spawn). `force_banks` re-includes the packed banks
/// when the `NavWorld` identity changed even though the stand list is
/// byte-identical.
#[derive(Clone, Copy, Default)]
pub struct DeltaMask {
    pub here: bool,
    pub ingame: bool,
    pub inv: bool,
    pub inv_size: bool,
    pub stats: bool,
    pub booths: bool,
    pub nearest_booth: bool,
    pub banks: bool,
    pub bank: bool,
    pub bank_side: bool,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: bool,
    pub count_dialog_open: bool,
    pub withdraw_x_result_seq: bool,
    pub withdraw_x_result: bool,
    pub withdraw_load_result_seq: bool,
    pub withdraw_load_result: bool,
    pub bank_op_result_seq: bool,
    pub bank_op_result: bool,
    pub hold: bool,
    pub ours: bool,
    pub npcs: bool,
    pub locs: bool,
    pub projectiles: bool,
    pub players: bool,
    pub ground: bool,
    pub equipment: bool,
    pub chat_open: bool,
    pub chat_continue: bool,
    pub chat_text: bool,
    pub chat_options: bool,
    pub side_tab: bool,
    pub varps: bool,
    pub combat_styles: bool,
    pub run_energy: bool,
    pub run_enabled: bool,
    pub retaliate_enabled: bool,
    pub my_name: bool,
    pub in_combat: bool,
    pub animating: bool,
    pub main_modal_id: bool,
    pub chat_modal_id: bool,
    pub make_products: bool,
    pub side_tab_ifaces: bool,
    pub spell_buttons: bool,
    pub chat_lines: bool,
    pub bank_note_on: bool,
    pub bank_note_off: bool,
    pub scene_state: bool,
    pub weight: bool,
    pub combat_level: bool,
    pub camera_yaw: bool,
    pub camera_pitch: bool,
    pub teleports_enabled: bool,
    pub self_slot: bool,
    pub trade_offer_open: bool,
    pub trade_confirm_open: bool,
    pub trade_partner: bool,
    pub trade_mine: bool,
    pub trade_theirs: bool,
    pub trade_side: bool,
    pub trade_accept_id: bool,
    pub trade_decline_id: bool,
    pub shop_open: bool,
    pub shop_stock: bool,
    pub shop_player: bool,
    pub main_make: bool,
    pub reach: bool,
    pub attacked_by_player: bool,
    pub self_target: bool,
    pub widgets: bool,
    pub self_chat: bool,
    pub hint_tile: bool,
    pub retaliate_controls: bool,
    pub quest_statuses: bool,
    pub npc_boxes: bool,
    pub bank_approaches: bool,
    pub user_move_intent_seq: bool,
    pub walk_outcome: bool,
    pub route_inspect: bool,
    pub collision: bool,
    /// One bit for the `MainModalTexts` pair: `(root, texts)` changes
    /// together so a buffer can never carry lines from one root beside
    /// another root's id. When set, slot 248 and slot 68 post in the same
    /// buffer.
    pub main_modal_texts: bool,
    /// One bit for the whole board family (identity, size, rows AND
    /// generation): when set, slot 250 and slot 252 post in the same
    /// buffer. Never set for the generation alone — the generation is not
    /// a field of its own.
    pub puzzle_board: bool,
    pub bank_selection: bool,
    /// The local player's animation id; written only when supplied.
    pub self_anim: bool,
    pub bank_snapshot_generation: bool,
    /// The live session page, including an explicit zero-id clear.
    pub api_gather: bool,
    /// The retained terminal is only replaced, never cleared by a delta.
    pub api_gather_outcome: bool,
    /// A progress page is replaced by the next request or cleared on teardown.
    pub api_progress: bool,
    pub chat_page_fingerprint: bool,
}

impl DeltaMask {
    /// Every field: the full (keyframe) snapshot.
    fn all() -> DeltaMask {
        DeltaMask {
            here: true,
            ingame: true,
            inv: true,
            inv_size: true,
            stats: true,
            booths: true,
            nearest_booth: true,
            banks: true,
            bank: true,
            bank_side: true,
            bank_open: true,
            bank_loaded: true,
            bank_generation: true,
            count_dialog_open: true,
            withdraw_x_result_seq: true,
            withdraw_x_result: true,
            withdraw_load_result_seq: true,
            withdraw_load_result: true,
            bank_op_result_seq: true,
            bank_op_result: true,
            hold: true,
            ours: true,
            npcs: true,
            locs: true,
            players: true,
            ground: true,
            equipment: true,
            projectiles: true,
            chat_open: true,
            chat_continue: true,
            chat_text: true,
            chat_options: true,
            side_tab: true,
            varps: true,
            combat_styles: true,
            run_energy: true,
            run_enabled: true,
            retaliate_enabled: true,
            my_name: true,
            in_combat: true,
            animating: true,
            main_modal_id: true,
            chat_modal_id: true,
            make_products: true,
            side_tab_ifaces: true,
            spell_buttons: true,
            chat_lines: true,
            bank_note_on: true,
            bank_note_off: true,
            scene_state: true,
            weight: true,
            combat_level: true,
            camera_yaw: true,
            camera_pitch: true,
            teleports_enabled: true,
            self_slot: true,
            trade_offer_open: true,
            trade_confirm_open: true,
            trade_partner: true,
            trade_mine: true,
            trade_theirs: true,
            trade_side: true,
            trade_accept_id: true,
            trade_decline_id: true,
            shop_open: true,
            shop_stock: true,
            shop_player: true,
            main_make: true,
            reach: true,
            attacked_by_player: true,
            self_target: true,
            widgets: true,
            self_chat: true,
            hint_tile: true,
            retaliate_controls: true,
            quest_statuses: true,
            npc_boxes: true,
            bank_approaches: true,
            user_move_intent_seq: true,
            walk_outcome: true,
            route_inspect: true,
            collision: true,
            main_modal_texts: true,
            puzzle_board: true,
            bank_selection: true,
            self_anim: true,
            bank_snapshot_generation: true,
            api_gather: true,
            api_gather_outcome: true,
            api_progress: true,
            chat_page_fingerprint: true,
        }
    }

    /// The fields that differ from `last` (all when there is no last post
    /// — a keyframe). Packed banks are additionally forced by
    /// `force_banks` (a `NavWorld` identity change the list alone cannot
    /// see).
    fn changed(
        last: &SnapshotFingerprint,
        next: &SnapshotFingerprint,
        force_banks: bool,
    ) -> DeltaMask {
        DeltaMask {
            here: next.here != last.here,
            ingame: next.ingame != last.ingame,
            inv: next.inv != last.inv,
            inv_size: next.inv_size != last.inv_size,
            stats: next.stats != last.stats,
            booths: next.booths != last.booths,
            nearest_booth: next.nearest_booth != last.nearest_booth,
            banks: force_banks || next.banks != last.banks,
            bank: next.bank != last.bank,
            bank_side: next.bank_side != last.bank_side,
            bank_open: next.bank_open != last.bank_open,
            bank_loaded: next.bank_loaded != last.bank_loaded,
            bank_generation: next.bank_generation != last.bank_generation,
            count_dialog_open: next.count_dialog_open != last.count_dialog_open,
            withdraw_x_result_seq: next.withdraw_x_result_seq != last.withdraw_x_result_seq,
            withdraw_x_result: next.withdraw_x_result != last.withdraw_x_result,
            withdraw_load_result_seq: next.withdraw_load_result_seq
                != last.withdraw_load_result_seq,
            withdraw_load_result: next.withdraw_load_result != last.withdraw_load_result,
            bank_op_result_seq: next.bank_op_result_seq != last.bank_op_result_seq,
            bank_op_result: next.bank_op_result != last.bank_op_result,
            // SEC-004: re-post hold every tick so JS cannot clear
            // `__rs2b0t_host.hold` in onPaint and unfreeze loop().
            hold: true,
            ours: next.ours != last.ours,
            npcs: next.npcs != last.npcs,
            locs: next.locs != last.locs,
            projectiles: next.projectiles != last.projectiles,
            players: next.players != last.players,
            ground: next.ground != last.ground,
            equipment: next.equipment != last.equipment,
            chat_open: next.chat_open != last.chat_open,
            chat_continue: next.chat_continue != last.chat_continue,
            chat_text: next.chat_text != last.chat_text,
            chat_options: next.chat_options != last.chat_options,
            side_tab: next.side_tab != last.side_tab,
            varps: next.varps != last.varps,
            combat_styles: next.combat_styles != last.combat_styles,
            run_energy: next.run_energy != last.run_energy,
            run_enabled: next.run_enabled != last.run_enabled,
            retaliate_enabled: next.retaliate_enabled != last.retaliate_enabled,
            my_name: next.my_name != last.my_name,
            in_combat: next.in_combat != last.in_combat,
            animating: next.animating != last.animating,
            main_modal_id: next.main_modal_id != last.main_modal_id,
            chat_modal_id: next.chat_modal_id != last.chat_modal_id,
            make_products: next.make_products != last.make_products,
            side_tab_ifaces: next.side_tab_ifaces != last.side_tab_ifaces,
            spell_buttons: next.spell_buttons != last.spell_buttons,
            chat_lines: next.chat_lines != last.chat_lines,
            bank_note_on: next.bank_note_on != last.bank_note_on,
            bank_note_off: next.bank_note_off != last.bank_note_off,
            scene_state: next.scene_state != last.scene_state,
            weight: next.weight != last.weight,
            combat_level: next.combat_level != last.combat_level,
            camera_yaw: next.camera_yaw != last.camera_yaw,
            camera_pitch: next.camera_pitch != last.camera_pitch,
            teleports_enabled: next.teleports_enabled != last.teleports_enabled,
            self_slot: next.self_slot != last.self_slot,
            trade_offer_open: next.trade_offer_open != last.trade_offer_open,
            trade_confirm_open: next.trade_confirm_open != last.trade_confirm_open,
            trade_partner: next.trade_partner != last.trade_partner,
            trade_mine: next.trade_mine != last.trade_mine,
            trade_theirs: next.trade_theirs != last.trade_theirs,
            trade_side: next.trade_side != last.trade_side,
            trade_accept_id: next.trade_accept_id != last.trade_accept_id,
            trade_decline_id: next.trade_decline_id != last.trade_decline_id,
            shop_open: next.shop_open != last.shop_open,
            shop_stock: next.shop_stock != last.shop_stock,
            shop_player: next.shop_player != last.shop_player,
            main_make: next.main_make != last.main_make,
            reach: next.reach != last.reach,
            attacked_by_player: next.attacked_by_player != last.attacked_by_player,
            self_target: next.self_target_kind != last.self_target_kind
                || next.self_target_index != last.self_target_index,
            widgets: next.widgets != last.widgets,
            self_chat: next.self_chat != last.self_chat,
            hint_tile: next.hint_tile != last.hint_tile,
            retaliate_controls: next.retaliate_controls != last.retaliate_controls,
            quest_statuses: next.quest_statuses != last.quest_statuses,
            npc_boxes: next.npc_boxes != last.npc_boxes,
            bank_approaches: next.bank_approaches != last.bank_approaches,
            user_move_intent_seq: next.user_move_intent_seq != last.user_move_intent_seq,
            walk_outcome: next.walk_outcome_seq != last.walk_outcome_seq
                || next.walk_outcome_generation != last.walk_outcome_generation
                || next.walk_outcome_failed != last.walk_outcome_failed
                || next.walk_outcome_x != last.walk_outcome_x
                || next.walk_outcome_z != last.walk_outcome_z
                || next.walk_outcome_level != last.walk_outcome_level
                || next.walk_outcome_radius != last.walk_outcome_radius
                || next.walk_outcome_allow_teleports != last.walk_outcome_allow_teleports
                || next.walk_outcome_request_id != last.walk_outcome_request_id
                || next.walk_outcome_blocked != last.walk_outcome_blocked
                || next.walk_outcome_cancel_reason != last.walk_outcome_cancel_reason
                || next.walk_missing_carry != last.walk_missing_carry,
            route_inspect: next.route_inspect != last.route_inspect,
            collision: next.collision != last.collision,
            main_modal_texts: next.main_modal_texts != last.main_modal_texts,
            // The generation rides inside the board row, so one comparison
            // covers both slots.
            puzzle_board: next.puzzle_board != last.puzzle_board,
            bank_selection: next.bank_selection != last.bank_selection,
            self_anim: next.self_anim != last.self_anim,
            bank_snapshot_generation: next.bank_snapshot_generation
                != last.bank_snapshot_generation,
            api_gather: next.api_gather != last.api_gather,
            api_gather_outcome: next.api_gather_outcome.is_some()
                && next.api_gather_outcome != last.api_gather_outcome,
            api_progress: next.api_progress.is_some() && next.api_progress != last.api_progress,
            chat_page_fingerprint: next.chat_page_fingerprint != last.chat_page_fingerprint,
        }
    }
}

/// One reusable FlatBuffer builder for isolate IPC. Each started JS slot
/// holds one on the host encode path and one on the V8 isolate thread:
/// `reset` keeps the backing allocation so a 50+ isolate wall does not
/// construct a new builder (or a JSON document) per PLAYER_INFO.
pub struct IsolateBuf {
    builder: FlatBufferBuilder<'static>,
}

impl Default for IsolateBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl IsolateBuf {
    pub fn new() -> Self {
        let mut builder = FlatBufferBuilder::new();
        builder.force_defaults(true);
        Self { builder }
    }

    #[cfg(test)]
    pub(crate) fn into_backing_capacity(self) -> usize {
        self.builder.collapse().0.capacity()
    }

    fn copy_finished(&self) -> Vec<u8> {
        self.builder.finished_data().to_vec()
    }

    /// Encode `input` as a root-`Snapshot` FlatBuffer carrying every field
    /// — the keyframe posted on Start / isolate spawn.
    pub fn encode_snapshot(&mut self, input: &SnapshotInput<'_>) -> Vec<u8> {
        self.encode_snapshot_with_native(input, NativeFactsInput::default())
    }

    pub fn encode_snapshot_with_native(
        &mut self,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) -> Vec<u8> {
        self.builder.reset();
        encode_snapshot_masked_into(&mut self.builder, input, native, &DeltaMask::all());
        self.copy_finished()
    }

    /// Encode a delta snapshot: `tick` always; every other field only when
    /// it differs from `last` (all fields when `last` is `None` — the
    /// keyframe). Returns the encoded buffer and the fingerprint of what
    /// was just posted.
    pub fn encode_snapshot_delta(
        &mut self,
        last: Option<&SnapshotFingerprint>,
        input: &SnapshotInput<'_>,
        force_banks: bool,
    ) -> (Vec<u8>, SnapshotFingerprint) {
        self.encode_snapshot_delta_with_native(
            last,
            input,
            NativeFactsInput::default(),
            force_banks,
        )
    }

    pub fn encode_snapshot_delta_with_native(
        &mut self,
        last: Option<&SnapshotFingerprint>,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
    ) -> (Vec<u8>, SnapshotFingerprint) {
        let mut fp = SnapshotFingerprint::from_input_with_native(input, native);
        fp.collision = collision_fp(last.map(|prev| &prev.collision), native.collision);
        let mask = match last {
            None => DeltaMask::all(),
            Some(prev) => DeltaMask::changed(prev, &fp, force_banks),
        };
        self.builder.reset();
        encode_snapshot_masked_into(&mut self.builder, input, native, &mask);
        (self.copy_finished(), fp)
    }

    /// Wake posts do not observe the tick-only screen projection. Retain its
    /// fingerprint without copying it, so the next tick still detects changes.
    /// Inventory is retained only when the wake has no decoded inventory.
    pub fn encode_snapshot_wake_with_native(
        &mut self,
        last: Option<&mut SnapshotFingerprint>,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        force_banks: bool,
        preserve_inv: bool,
    ) -> (Vec<u8>, SnapshotFingerprint) {
        let Some(last) = last else {
            return self.encode_snapshot_delta_with_native(None, input, native, force_banks);
        };
        let mut fp = SnapshotFingerprint::from_input_with_native(input, native);
        fp.collision = collision_fp(Some(&last.collision), native.collision);
        let mut mask = DeltaMask::changed(last, &fp, force_banks);
        mask.npc_boxes = false;
        fp.npc_boxes = last.npc_boxes.take();
        if preserve_inv {
            mask.inv = false;
            mask.inv_size = false;
            fp.inv = std::mem::take(&mut last.inv);
            fp.inv_size = last.inv_size;
        }
        self.builder.reset();
        encode_snapshot_masked_into(&mut self.builder, input, native, &mask);
        (self.copy_finished(), fp)
    }

    /// Encode the tick's shim interact queue as a root-`InteractBatch`.
    pub fn encode_interact_batch(&mut self, reqs: &[crate::shim::InteractReq]) -> Vec<u8> {
        self.builder.reset();
        encode_interact_batch_into(&mut self.builder, reqs);
        self.copy_finished()
    }
}

/// Encode `input` as a root-`Snapshot` FlatBuffer carrying every field —
/// the keyframe posted on Start / isolate spawn. Tests and one-shot
/// callers; the live path uses [`IsolateBuf`].
pub fn encode_snapshot(input: &SnapshotInput<'_>) -> Vec<u8> {
    IsolateBuf::new().encode_snapshot(input)
}

pub fn encode_snapshot_with_native(
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
) -> Vec<u8> {
    IsolateBuf::new().encode_snapshot_with_native(input, native)
}

/// Encode a delta snapshot: `tick` always; every other field only when it
/// differs from `last` (all fields when `last` is `None` — the keyframe).
/// Omitted tables are absent from the buffer, never empty: the isolate
/// keeps its last JS values for them. Packed `banks` are carried on the
/// keyframe and when `force_banks` even if the stand list is unchanged
/// (a `NavWorld` identity change). Returns the encoded buffer and the
/// fingerprint of what was just posted — the caller stores it as the new
/// `last` (per-slot, reset on Start). Tests and one-shot callers; the
/// live path uses [`IsolateBuf`].
pub fn encode_snapshot_delta(
    last: Option<&SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
    force_banks: bool,
) -> (Vec<u8>, SnapshotFingerprint) {
    IsolateBuf::new().encode_snapshot_delta(last, input, force_banks)
}

pub fn encode_snapshot_delta_with_native(
    last: Option<&SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
    force_banks: bool,
) -> (Vec<u8>, SnapshotFingerprint) {
    IsolateBuf::new().encode_snapshot_delta_with_native(last, input, native, force_banks)
}

/// Encode `input` carrying exactly the masked fields (`tick` is always
/// carried).
fn encode_snapshot_masked_into(
    b: &mut FlatBufferBuilder<'_>,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
    mask: &DeltaMask,
) {
    // Children (strings, sub-tables, vectors) are written before the root
    // table's own start — masked-in fields only.
    let here_off = if mask.here {
        input.here.map(|h| tile_off(b, h))
    } else {
        None
    };
    let inv_off = if mask.inv {
        let offs = input.inv.iter().map(|r| row_off(b, r)).collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let stats_off = if mask.stats {
        let offs = input
            .stats
            .iter()
            .map(|s| stat_off(b, s))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let booths_off = if mask.booths {
        let offs = input
            .booths
            .iter()
            .map(|t| booth_off(b, *t))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let banks_off = if mask.banks {
        let offs = input
            .banks
            .iter()
            .map(|s| bank_stand_off(b, s))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let bank_off = if mask.bank {
        let offs = input.bank.iter().map(|r| row_off(b, r)).collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let bank_side_off = if mask.bank_side {
        let offs = input
            .bank_side
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let mut entities_off = |entities: &[SceneEntityInput<'_>]| {
        let offs = entities
            .iter()
            .map(|e| scene_entity_off(b, e))
            .collect::<Vec<_>>();
        b.create_vector(&offs)
    };
    let npcs_off = if mask.npcs {
        Some(entities_off(input.npcs))
    } else {
        None
    };
    let locs_off = if mask.locs {
        Some(entities_off(input.locs))
    } else {
        None
    };
    let players_off = if mask.players {
        Some(entities_off(input.players))
    } else {
        None
    };
    let ground_off = if mask.ground {
        Some(entities_off(input.ground))
    } else {
        None
    };
    let equipment_off = if mask.equipment {
        let offs = input
            .equipment
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let chat_text_off = if mask.chat_text {
        Some(b.create_string(input.chat_text.unwrap_or("")))
    } else {
        None
    };
    let chat_options_off = if mask.chat_options {
        let offs = input
            .chat_options
            .iter()
            .map(|o| chat_option_off(b, o))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let make_products_off = if mask.make_products {
        let offs = input
            .make_products
            .iter()
            .map(|p| make_product_off(b, p))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let varps_off = if mask.varps {
        let offs = input
            .varps
            .iter()
            .map(|v| varp_off(b, v))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let combat_styles_off = if mask.combat_styles {
        let offs = input
            .combat_styles
            .iter()
            .map(|c| combat_style_off(b, c))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let side_tab_ifaces_off = if mask.side_tab_ifaces {
        let offs = input
            .side_tab_ifaces
            .iter()
            .map(|t| side_tab_iface_off(b, *t))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let spell_buttons_off = if mask.spell_buttons {
        let offs = input
            .spell_buttons
            .iter()
            .map(|c| combat_style_off(b, c))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let chat_lines_off = if mask.chat_lines {
        let offs = input
            .chat_lines
            .iter()
            .map(|l| chat_line_off(b, l))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let nearest_booth_table_off = if mask.nearest_booth {
        input
            .nearest_booth
            .as_ref()
            .map(|nb| nearest_booth_table_off(b, nb))
    } else {
        None
    };
    let my_name_off = if mask.my_name {
        Some(b.create_string(input.my_name.unwrap_or("")))
    } else {
        None
    };
    let trade_partner_off = if mask.trade_partner {
        Some(b.create_string(input.trade_partner.unwrap_or("")))
    } else {
        None
    };
    let trade_mine_off = if mask.trade_mine {
        let offs = input
            .trade_mine
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let trade_theirs_off = if mask.trade_theirs {
        let offs = input
            .trade_theirs
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let trade_side_off = if mask.trade_side {
        let offs = input
            .trade_side
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let shop_stock_off = if mask.shop_stock {
        let offs = input
            .shop_stock
            .iter()
            .map(|r| row_off(b, r))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let shop_player_off = if mask.shop_player {
        native.shop_player.map(|rows| {
            let offs = rows.iter().map(|r| row_off(b, r)).collect::<Vec<_>>();
            b.create_vector(&offs)
        })
    } else {
        None
    };
    let main_make_off = if mask.main_make {
        native.main_make.map(|rows| {
            let offs = rows.iter().map(|r| row_off(b, r)).collect::<Vec<_>>();
            b.create_vector(&offs)
        })
    } else {
        None
    };
    let reach_table_off = if mask.reach {
        Some(reach_off(b, &input.reach))
    } else {
        None
    };
    let collision_table_off = if mask.collision {
        native.collision.map(|c| collision_off(b, &c))
    } else {
        None
    };
    // The main modal's paired text walk. ONE table: when it is written both
    // inner slots are written, and an empty `texts` stays `[]` (a supplied
    // walk of a closed or text-less modal). The table is absent only when
    // the native fact was not supplied — that omits the slot, which is not
    // an observed close.
    let main_modal_texts_off = if mask.main_modal_texts {
        native
            .main_modal_texts
            .map(|pair| main_modal_texts_off(b, &pair))
    } else {
        None
    };
    // The open puzzle board. ONE table carrying the identity, the slot count
    // and the rows; all three inner slots are always written, so a present
    // board can never be half-posted. The table is absent only when the
    // native fact was not supplied — that omits both Snapshot slots (a delta
    // keep), which is not an observed close.
    let puzzle_board_slot = if mask.puzzle_board {
        native
            .puzzle_board
            .map(|board| (puzzle_board_off(b, &board), board.generation))
    } else {
        None
    };
    // Slot 68 co-posts with the pair (a text-only change still carries the
    // id those lines belong to). A present table's root wins over
    // `input.main_modal_id` — the co-posted integer must be the root the
    // lines were walked from, so the page can never read one root's lines
    // beside another root's id. Never `-1` merely because `texts` is empty.
    let main_modal_id_slot = if mask.main_modal_texts {
        Some(
            native
                .main_modal_texts
                .map_or(input.main_modal_id, |pair| pair.root),
        )
    } else if mask.main_modal_id {
        Some(input.main_modal_id)
    } else {
        None
    };
    let widgets_off = if mask.widgets {
        let offs = input
            .widgets
            .iter()
            .map(|w| widget_text_off(b, w))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let self_chat_off = if mask.self_chat {
        Some(b.create_string(native.self_chat.unwrap_or("")))
    } else {
        None
    };
    let quest_statuses_off = if mask.quest_statuses {
        native.quest_statuses.map(|rows| {
            let offs = rows
                .iter()
                .map(|q| quest_status_off(b, q))
                .collect::<Vec<_>>();
            b.create_vector(&offs)
        })
    } else {
        None
    };
    let npc_boxes_off = if mask.npc_boxes {
        native.npc_boxes.map(|rows| {
            let offs = rows
                .iter()
                .map(|row| npc_box_off(b, row))
                .collect::<Vec<_>>();
            b.create_vector(&offs)
        })
    } else {
        None
    };
    let bank_approaches_off = if mask.bank_approaches {
        native.bank_approaches.map(|rows| {
            let offs = rows
                .iter()
                .map(|row| bank_approach_off(b, row))
                .collect::<Vec<_>>();
            b.create_vector(&offs)
        })
    } else {
        None
    };
    // The walk outcome's own family: the carry vector is built and pushed with
    // it, never on its own. The rows are always supplied, so an empty vector
    // posts as a present clear and the family never omits one.
    let walk_missing_carry_off = if mask.walk_outcome {
        let offs = native
            .walk_missing_carry
            .iter()
            .map(|row| carry_off(b, row))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let inspect_reason_off = if mask.route_inspect {
        Some(b.create_string(native.route_inspect.latest.reason.unwrap_or("")))
    } else {
        None
    };
    let inspect_prev_reason_off = if mask.route_inspect {
        Some(b.create_string(native.route_inspect.prev.reason.unwrap_or("")))
    } else {
        None
    };
    let inspect_hops_off = if mask.route_inspect {
        let offs = native
            .route_inspect
            .latest
            .hops
            .iter()
            .map(|hop| inspect_hop_off(b, hop))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let inspect_prev_hops_off = if mask.route_inspect {
        let offs = native
            .route_inspect
            .prev
            .hops
            .iter()
            .map(|hop| inspect_hop_off(b, hop))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    // The live page posts an explicit request-id-zero table on keyframes and
    // when a session clears. Terminal outcomes are retained: `None` omits the
    // table even on an ordinary delta.
    let api_gather_slot = mask
        .api_gather
        .then(|| api_gather_off(b, native.api_gather));
    let api_gather_outcome_slot = if mask.api_gather_outcome {
        native
            .api_gather_outcome
            .map(|outcome| api_gather_outcome_off(b, outcome))
    } else {
        None
    };
    // A progress page is present only when supplied and changed. Unlike the
    // live Gatherer page, there is no request-id-zero clear table.
    let api_progress_slot = if mask.api_progress {
        native.api_progress.map(|page| api_progress_off(b, page))
    } else {
        None
    };
    let projectiles_off = if mask.projectiles {
        native.projectiles.map(|rows| {
            let mut offsets = [WIPOffset::new(0); MAX_PROJECTILES_PER_SNAPSHOT];
            let mut len = 0;
            for projectile in local_target_projectiles(rows, input.self_slot) {
                offsets[len] = combat_projectile_off(b, projectile);
                len += 1;
            }
            b.create_vector(&offsets[..len])
        })
    } else {
        None
    };
    let mut table = SnapshotBuilder::new(b);
    table.add_tick(input.tick);
    if mask.here {
        if let Some(off) = here_off {
            table.add_here(off);
        }
    }
    if mask.ingame {
        table.add_ingame(input.ingame);
    }
    if mask.inv {
        table.add_inv(inv_off.expect("mask checked"));
    }
    if mask.inv_size {
        table.add_inv_size(input.inv_size);
    }
    if mask.stats {
        table.add_stats(stats_off.expect("mask checked"));
    }
    if mask.booths {
        table.add_booths(booths_off.expect("mask checked"));
    }
    if mask.nearest_booth {
        if let Some(off) = nearest_booth_table_off {
            table.add_nearest_booth(off);
        }
    }
    if mask.banks {
        table.add_banks(banks_off.expect("mask checked"));
    }
    if mask.bank {
        table.add_bank(bank_off.expect("mask checked"));
    }
    if mask.bank_side {
        table.add_bank_side(bank_side_off.expect("mask checked"));
    }
    if mask.bank_open {
        table.add_bank_open(input.bank_open);
    }
    if mask.bank_loaded {
        table.add_bank_loaded(input.bank_loaded);
    }
    if mask.bank_generation {
        table.add_bank_generation(input.bank_generation);
    }
    if mask.count_dialog_open {
        table.add_count_dialog_open(input.count_dialog_open);
    }
    if mask.withdraw_x_result_seq {
        table.add_withdraw_x_result_seq(input.withdraw_x_result_seq);
    }
    if mask.withdraw_x_result {
        table.add_withdraw_x_result(input.withdraw_x_result);
    }
    if mask.withdraw_load_result_seq {
        table.add_withdraw_load_result_seq(input.withdraw_load_result_seq);
    }
    if mask.withdraw_load_result {
        table.add_withdraw_load_result(input.withdraw_load_result);
    }
    if mask.bank_op_result_seq {
        table.add_bank_op_result_seq(input.bank_op_result_seq);
    }
    if mask.bank_op_result {
        table.add_bank_op_result(input.bank_op_result);
    }
    if mask.hold {
        table.add_hold(input.hold);
    }
    if mask.ours {
        table.add_ours(input.ours);
    }
    if mask.npcs {
        table.add_npcs(npcs_off.expect("mask checked"));
    }
    if mask.locs {
        table.add_locs(locs_off.expect("mask checked"));
    }
    if mask.players {
        table.add_players(players_off.expect("mask checked"));
    }
    if mask.ground {
        table.add_ground(ground_off.expect("mask checked"));
    }
    if mask.equipment {
        table.add_equipment(equipment_off.expect("mask checked"));
    }
    if mask.chat_open {
        table.add_chat_open(input.chat_open);
    }
    if mask.chat_continue {
        table.add_chat_continue(input.chat_continue);
    }
    if mask.chat_text {
        table.add_chat_text(chat_text_off.expect("mask checked"));
    }
    if mask.chat_options {
        table.add_chat_options(chat_options_off.expect("mask checked"));
    }
    if mask.side_tab {
        table.add_side_tab(input.side_tab);
    }
    if mask.varps {
        table.add_varps(varps_off.expect("mask checked"));
    }
    if mask.combat_styles {
        table.add_combat_styles(combat_styles_off.expect("mask checked"));
    }
    if mask.run_energy {
        table.add_run_energy(input.run_energy);
    }
    if mask.run_enabled {
        table.add_run_enabled(input.run_enabled);
    }
    if mask.retaliate_enabled {
        table.add_retaliate_enabled(input.retaliate_enabled);
    }
    if mask.my_name {
        table.add_my_name(my_name_off.expect("mask checked"));
    }
    if mask.in_combat {
        table.add_in_combat(input.in_combat);
    }
    if mask.animating {
        table.add_animating(input.animating);
    }
    if let Some(main_modal_id) = main_modal_id_slot {
        table.add_main_modal_id(main_modal_id);
    }
    if mask.chat_modal_id {
        table.add_chat_modal_id(input.chat_modal_id);
    }
    if mask.make_products {
        table.add_make_products(make_products_off.expect("mask checked"));
    }
    if mask.side_tab_ifaces {
        table.add_side_tab_ifaces(side_tab_ifaces_off.expect("mask checked"));
    }
    if mask.spell_buttons {
        table.add_spell_buttons(spell_buttons_off.expect("mask checked"));
    }
    if mask.chat_lines {
        table.add_chat_lines(chat_lines_off.expect("mask checked"));
    }
    if mask.bank_note_on {
        table.add_bank_note_on(input.bank_note_on);
    }
    if mask.bank_note_off {
        table.add_bank_note_off(input.bank_note_off);
    }
    if mask.scene_state {
        table.add_scene_state(input.scene_state);
    }
    if mask.weight {
        table.add_weight(input.weight);
    }
    if mask.combat_level {
        table.add_combat_level(input.combat_level);
    }
    if mask.camera_yaw {
        table.add_camera_yaw(input.camera_yaw);
    }
    if mask.camera_pitch {
        table.add_camera_pitch(input.camera_pitch);
    }
    if mask.teleports_enabled {
        table.add_teleports_enabled(input.teleports_enabled);
    }
    if mask.self_slot {
        table.add_self_slot(input.self_slot);
    }
    if mask.trade_offer_open {
        table.add_trade_offer_open(input.trade_offer_open);
    }
    if mask.trade_confirm_open {
        table.add_trade_confirm_open(input.trade_confirm_open);
    }
    if mask.trade_partner {
        table.add_trade_partner(trade_partner_off.expect("mask checked"));
    }
    if mask.trade_mine {
        table.add_trade_mine(trade_mine_off.expect("mask checked"));
    }
    if mask.trade_theirs {
        table.add_trade_theirs(trade_theirs_off.expect("mask checked"));
    }
    if mask.trade_side {
        table.add_trade_side(trade_side_off.expect("mask checked"));
    }
    if mask.trade_accept_id {
        table.add_trade_accept_id(input.trade_accept_id);
    }
    if mask.trade_decline_id {
        table.add_trade_decline_id(input.trade_decline_id);
    }
    if mask.shop_open {
        table.add_shop_open(input.shop_open);
    }
    if mask.shop_stock {
        table.add_shop_stock(shop_stock_off.expect("mask checked"));
    }
    if mask.shop_player {
        table.add_shop_player_available(native.shop_player.is_some());
        if let Some(off) = shop_player_off {
            table.add_shop_player(off);
        }
    }
    if mask.main_make {
        table.add_main_make_available(native.main_make.is_some());
        if let Some(off) = main_make_off {
            table.add_main_make(off);
        }
    }
    if mask.reach {
        table.add_reach(reach_table_off.expect("mask checked"));
    }
    if mask.attacked_by_player {
        table.add_attacked_by_player(input.attacked_by_player);
    }
    if mask.self_target {
        table.add_self_target_kind(input.self_target_kind);
        table.add_self_target_index(input.self_target_index);
    }
    if mask.widgets {
        table.add_widgets(widgets_off.expect("mask checked"));
    }
    if mask.bank_selection {
        table.add_bank_selection_request_id(native.bank_selection.request_id);
        table.add_bank_selection_generation(native.bank_selection.generation);
        table.add_bank_selection_index(native.bank_selection.bank_index);
        table.add_bank_selection_kind(native.bank_selection.kind);
    }
    if let (true, Some(anim)) = (mask.self_anim, native.self_anim) {
        table.add_self_anim(anim);
    }
    if let (true, Some(generation)) = (
        mask.bank_snapshot_generation,
        native.bank_snapshot_generation,
    ) {
        table.add_bank_snapshot_generation(generation);
    }
    if mask.self_chat {
        table.add_self_chat(self_chat_off.expect("mask checked"));
    }
    if mask.hint_tile {
        let (x, z) = native.hint_tile.unwrap_or((-1, -1));
        table.add_hint_tile_x(x);
        table.add_hint_tile_z(z);
    }
    if mask.retaliate_controls {
        let (on, off) = native.retaliate_controls.unwrap_or((-1, -1));
        table.add_retaliate_on_com_id(on);
        table.add_retaliate_off_com_id(off);
    }
    if mask.quest_statuses {
        table.add_quest_statuses_available(native.quest_statuses.is_some());
        if let Some(off) = quest_statuses_off {
            table.add_quest_statuses(off);
        }
    }
    if mask.npc_boxes {
        table.add_npc_boxes_available(native.npc_boxes.is_some());
        if let Some(off) = npc_boxes_off {
            table.add_npc_boxes(off);
        }
    }
    if mask.bank_approaches {
        if let Some(off) = bank_approaches_off {
            table.add_bank_approaches(off);
        }
    }
    if mask.user_move_intent_seq {
        table.add_user_move_intent_seq(input.user_move_intent_seq);
    }
    if mask.walk_outcome {
        table.add_walk_outcome_seq(native.walk_outcome_seq);
        table.add_walk_outcome_generation(native.walk_outcome_generation);
        table.add_walk_outcome_failed(native.walk_outcome_failed);
        table.add_walk_outcome_x(native.walk_outcome_x);
        table.add_walk_outcome_z(native.walk_outcome_z);
        table.add_walk_outcome_level(native.walk_outcome_level);
        table.add_walk_outcome_radius(native.walk_outcome_radius);
        table.add_walk_outcome_allow_teleports(native.walk_outcome_allow_teleports);
        table.add_walk_outcome_request_id(native.walk_outcome_request_id);
        table.add_walk_outcome_blocked(native.walk_outcome_blocked);
        table.add_walk_outcome_cancel_reason(input.walk_outcome_cancel_reason);
        // the observed "no named short", so a clear is never omitted. Only a
        // caller that supplied no list at all omits the slot.
        if let Some(off) = walk_missing_carry_off {
            table.add_walk_missing_carry(off);
        }
    }
    if mask.route_inspect {
        let facts = &native.route_inspect;
        table.add_route_inspect_seq(facts.latest.seq);
        table.add_route_inspect_generation(facts.latest.generation);
        table.add_route_inspect_request_id(facts.latest.request_id);
        table.add_route_inspect_ok(facts.latest.ok);
        if let Some(off) = inspect_reason_off {
            table.add_route_inspect_reason(off);
        }
        table.add_route_inspect_bank_planned(facts.latest.bank_planned);
        table.add_route_inspect_ticks(facts.latest.ticks);
        if let Some(off) = inspect_hops_off {
            table.add_route_inspect_hops(off);
        }
        table.add_route_inspect_prev_seq(facts.prev.seq);
        table.add_route_inspect_prev_generation(facts.prev.generation);
        table.add_route_inspect_prev_request_id(facts.prev.request_id);
        table.add_route_inspect_prev_ok(facts.prev.ok);
        if let Some(off) = inspect_prev_reason_off {
            table.add_route_inspect_prev_reason(off);
        }
        table.add_route_inspect_prev_bank_planned(facts.prev.bank_planned);
        table.add_route_inspect_prev_ticks(facts.prev.ticks);
        if let Some(off) = inspect_prev_hops_off {
            table.add_route_inspect_prev_hops(off);
        }
        table.add_route_inspect_running_id(facts.running_id);
        table.add_route_inspect_pending_id(facts.pending_id);
        table.add_route_inspect_accepted_id(facts.accepted_id);
        table.add_route_inspect_replaced_id(facts.replaced_id);
        table.add_route_inspect_replaced_prev_id(facts.replaced_prev_id);
        table.add_route_inspect_refused_id(facts.refused_id);
        table.add_route_inspect_refused_id_2(facts.refused_id_2);
        table.add_route_inspect_refused_id_3(facts.refused_id_3);
        table.add_route_inspect_unobserved(facts.unobserved);
    }
    if mask.collision {
        if let Some(off) = collision_table_off {
            table.add_collision(off);
        }
    }
    if mask.main_modal_texts {
        if let Some(off) = main_modal_texts_off {
            table.add_main_modal_texts(off);
        }
    }
    // Both slots or neither: the generation is pushed only from the same
    // `Some` that produced the table, so no buffer can carry a generation
    // without a board (or a board without its generation).
    if let Some((off, generation)) = puzzle_board_slot {
        table.add_puzzle_board(off);
        table.add_puzzle_board_generation(generation);
    }
    if let Some(off) = api_gather_slot {
        table.add_api_gather(off);
    }
    if let Some(off) = api_gather_outcome_slot {
        table.add_api_gather_outcome(off);
    }
    if let Some(off) = api_progress_slot {
        table.add_api_progress(off);
    }
    if let Some(off) = projectiles_off {
        table.add_projectiles(off);
    }
    if mask.chat_page_fingerprint {
        table.add_chat_page_fingerprint(native.chat_page_fingerprint);
    }
    table.add_canvas_width(SNAPSHOT_CANVAS_W);
    table.add_canvas_height(SNAPSHOT_CANVAS_H);
    let root = table.finish();
    b.finish(root, None);
}

fn tile_off<'b>(b: &mut FlatBufferBuilder<'b>, t: TileInput) -> WIPOffset<Tile<'b>> {
    let mut table = TileBuilder::new(b);
    table.add_x(t.x);
    table.add_z(t.z);
    table.add_level(t.level);
    table.finish()
}
fn combat_projectile_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    projectile: &ProjectileView,
) -> WIPOffset<CombatProjectile<'b>> {
    let mut table = CombatProjectileBuilder::new(b);
    table.add_spotanim(projectile.spotanim);
    if let Some(target) = projectile
        .target
        .filter(|target| target.kind == ActorKind::Player)
    {
        if let Ok(index) = i32::try_from(target.index) {
            table.add_target_player_index(index);
        }
    }
    table.finish()
}
/// The largest encoded settings vector admitted for `gather-run`.
pub(crate) const GATHER_SETTINGS_MAX_BYTES: usize = 4 * 1024;
const MAX_GATHER_SETTING_ROWS: usize = 256;
const MAX_GATHER_SETTING_LIST_ITEMS: usize = 512;

/// Whether `bag` has supported typed values whose exact FlatBuffer vector
/// fits the isolate wire limit.
pub(crate) fn gather_settings_within_limit(bag: &crate::native::SettingsBag) -> bool {
    let mut builder = FlatBufferBuilder::new();
    let Ok(settings) = settings_vector_off(&mut builder, bag) else {
        return false;
    };
    builder.finish(settings, None);
    builder.finished_data().len() <= GATHER_SETTINGS_MAX_BYTES
}

fn setting_tile(value: &serde_json::Value, key: &str) -> Result<TileInput, String> {
    let serde_json::Value::Object(fields) = value else {
        return Err(format!("setting {key:?} is not a tile"));
    };
    if fields.len() != 3 {
        return Err(format!("setting {key:?} is not a tile"));
    }
    let coordinate = |name| {
        fields
            .get(name)
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| format!("setting {key:?} has an invalid tile"))
    };
    Ok(TileInput {
        x: coordinate("x")?,
        z: coordinate("z")?,
        level: coordinate("level")?,
    })
}
fn setting_text_bytes(key: &str, value: &serde_json::Value) -> Result<usize, String> {
    let mut bytes = key.len();
    match value {
        serde_json::Value::String(text) => bytes = bytes.saturating_add(text.len()),
        serde_json::Value::Number(number) => {
            if number.as_i64().is_none() {
                return Err(format!("setting {key:?} is not an integer"));
            }
        }
        serde_json::Value::Bool(_) => {}
        serde_json::Value::Array(items) => {
            if items.len() > MAX_GATHER_SETTING_LIST_ITEMS {
                return Err("gather-run settings exceed cap".into());
            }
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(format!("setting {key:?} is not a string list"));
                };
                bytes = bytes.saturating_add(text.len());
            }
        }
        serde_json::Value::Object(_) => {
            setting_tile(value, key)?;
        }
        serde_json::Value::Null => {
            return Err(format!("setting {key:?} has an unsupported value"));
        }
    }
    if bytes > GATHER_SETTINGS_MAX_BYTES {
        return Err("gather-run settings exceed cap".into());
    }
    Ok(bytes)
}

fn setting_row_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    key: &str,
    value: &serde_json::Value,
) -> Result<WIPOffset<SettingRow<'b>>, String> {
    let key_off = b.create_string(key);
    match value {
        serde_json::Value::String(text) => {
            let text_off = b.create_string(text);
            // FlatBuffer child offsets must be built before their parent.
            let mut table = SettingRowBuilder::new(b);
            table.add_key(key_off);
            table.add_kind(1);
            table.add_text(text_off);
            Ok(table.finish())
        }
        serde_json::Value::Number(number) => {
            let integer = number
                .as_i64()
                .ok_or_else(|| format!("setting {key:?} is not an integer"))?;
            let mut table = SettingRowBuilder::new(b);
            table.add_key(key_off);
            table.add_kind(2);
            table.add_integer(integer);
            Ok(table.finish())
        }
        serde_json::Value::Bool(flag) => {
            let mut table = SettingRowBuilder::new(b);
            table.add_key(key_off);
            table.add_kind(3);
            table.add_flag(*flag);
            Ok(table.finish())
        }
        serde_json::Value::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(format!("setting {key:?} is not a string list"));
                };
                values.push(b.create_string(text));
            }
            let list = b.create_vector(&values);
            let mut table = SettingRowBuilder::new(b);
            table.add_key(key_off);
            table.add_kind(4);
            table.add_list(list);
            Ok(table.finish())
        }
        serde_json::Value::Object(_) => {
            let tile = setting_tile(value, key)?;
            let tile = tile_off(b, tile);
            let mut table = SettingRowBuilder::new(b);
            table.add_key(key_off);
            table.add_kind(5);
            table.add_tile(tile);
            Ok(table.finish())
        }
        serde_json::Value::Null => Err(format!("setting {key:?} has an unsupported value")),
    }
}

fn settings_vector_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    bag: &crate::native::SettingsBag,
) -> Result<WIPOffset<flatbuffers::Vector<'b, flatbuffers::ForwardsUOffset<SettingRow<'b>>>>, String>
{
    if bag.len() > MAX_GATHER_SETTING_ROWS {
        return Err("gather-run settings exceed cap".into());
    }
    let list_items = bag
        .values()
        .filter_map(serde_json::Value::as_array)
        .fold(0usize, |sum, values| sum.saturating_add(values.len()));
    if list_items > MAX_GATHER_SETTING_LIST_ITEMS {
        return Err("gather-run settings exceed cap".into());
    }
    let _text_bytes = bag.iter().try_fold(0usize, |sum, (key, value)| {
        let field_bytes = setting_text_bytes(key, value)?;
        let total = sum.saturating_add(field_bytes);
        (total <= GATHER_SETTINGS_MAX_BYTES)
            .then_some(total)
            .ok_or_else(|| "gather-run settings exceed cap".to_string())
    })?;
    let rows = bag
        .iter()
        .map(|(key, value)| setting_row_off(b, key, value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(b.create_vector(&rows))
}

fn validate_gather_settings_rows<'a>(
    rows: flatbuffers::Vector<'a, flatbuffers::ForwardsUOffset<SettingRow<'a>>>,
) -> Result<(), String> {
    if rows.len() > MAX_GATHER_SETTING_ROWS {
        return Err("gather-run settings exceed cap".into());
    }
    let mut encoded_text_bytes = 0usize;
    let mut list_items = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let key = row
            .key()
            .ok_or_else(|| "gather-run setting has no key".to_string())?;
        if rows
            .iter()
            .take(index)
            .any(|previous| previous.key() == Some(key))
        {
            return Err("gather-run settings contain a duplicate key".into());
        }
        encoded_text_bytes = encoded_text_bytes.saturating_add(key.len());
        if encoded_text_bytes > GATHER_SETTINGS_MAX_BYTES {
            return Err("gather-run settings exceed cap".into());
        }
        match row.kind() {
            1 => {
                let text = row
                    .text()
                    .ok_or_else(|| "gather-run text setting has no value".to_string())?;
                encoded_text_bytes = encoded_text_bytes.saturating_add(text.len());
            }
            2 | 3 => {}
            4 => {
                let list = row
                    .list()
                    .ok_or_else(|| "gather-run list setting has no value".to_string())?;
                list_items = list_items.saturating_add(list.len());
                if list_items > MAX_GATHER_SETTING_LIST_ITEMS {
                    return Err("gather-run settings exceed cap".into());
                }
                for text in list.iter() {
                    encoded_text_bytes = encoded_text_bytes.saturating_add(text.len());
                    if encoded_text_bytes > GATHER_SETTINGS_MAX_BYTES {
                        return Err("gather-run settings exceed cap".into());
                    }
                }
            }
            5 => {
                if row.tile().is_none() {
                    return Err("gather-run tile setting has no value".into());
                }
            }
            kind => return Err(format!("gather-run setting has unknown kind {kind}")),
        }
        if encoded_text_bytes > GATHER_SETTINGS_MAX_BYTES {
            return Err("gather-run settings exceed cap".into());
        }
    }
    Ok(())
}

fn decode_gather_settings<'a>(
    rows: flatbuffers::Vector<'a, flatbuffers::ForwardsUOffset<SettingRow<'a>>>,
) -> Result<crate::native::SettingsBag, String> {
    validate_gather_settings_rows(rows)?;
    let mut bag = crate::native::SettingsBag::new();
    for row in rows.iter() {
        let key = row
            .key()
            .ok_or_else(|| "gather-run setting has no key".to_string())?;
        let value = match row.kind() {
            1 => serde_json::Value::String(
                row.text()
                    .ok_or_else(|| "gather-run text setting has no value".to_string())?
                    .to_string(),
            ),
            2 => serde_json::Value::from(row.integer()),
            3 => serde_json::Value::Bool(row.flag()),
            4 => {
                let list = row
                    .list()
                    .ok_or_else(|| "gather-run list setting has no value".to_string())?;
                let values = list
                    .iter()
                    .map(|text| serde_json::Value::String(text.to_string()))
                    .collect();
                serde_json::Value::Array(values)
            }
            5 => {
                let tile = row
                    .tile()
                    .ok_or_else(|| "gather-run tile setting has no value".to_string())?;
                serde_json::json!({
                    "x": tile.x(),
                    "z": tile.z(),
                    "level": tile.level(),
                })
            }
            kind => return Err(format!("gather-run setting has unknown kind {kind}")),
        };
        if bag.insert(key.to_string(), value).is_some() {
            return Err("gather-run settings contain a duplicate key".into());
        }
    }
    if !gather_settings_within_limit(&bag) {
        return Err("gather-run settings exceed cap".into());
    }
    Ok(bag)
}

fn status_field_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    field: &crate::native::StatusField,
) -> Option<WIPOffset<StatusField<'b>>> {
    use crate::native::StatusValue;
    if matches!(&field.value, crate::native::StatusValue::Quest(_)) {
        return None;
    }
    let key = b.create_string(field.key);
    match &field.value {
        StatusValue::Text(value) => {
            let value = b.create_string(value);
            let mut table = StatusFieldBuilder::new(b);
            table.add_key(key);
            table.add_kind(1);
            table.add_text(value);
            Some(table.finish())
        }
        StatusValue::Integer(value) => {
            let mut table = StatusFieldBuilder::new(b);
            table.add_key(key);
            table.add_kind(2);
            table.add_integer(*value);
            Some(table.finish())
        }
        StatusValue::Tile(value) => {
            let tile = tile_off(
                b,
                TileInput {
                    x: value.x,
                    z: value.z,
                    level: value.level,
                },
            );
            let mut table = StatusFieldBuilder::new(b);
            table.add_key(key);
            table.add_kind(3);
            table.add_tile(tile);
            Some(table.finish())
        }
        StatusValue::Truth(value) => {
            let truth = match value {
                api::selected::Truth::True => 1,
                api::selected::Truth::False => 2,
                api::selected::Truth::Unknown => 3,
            };
            let mut table = StatusFieldBuilder::new(b);
            table.add_key(key);
            table.add_kind(4);
            table.add_truth(truth);
            Some(table.finish())
        }
        StatusValue::Quest(_) => None,
    }
}

fn api_gather_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    page: Option<&crate::api_gather::GatherPage>,
) -> WIPOffset<ApiGather<'b>> {
    let (request_id, phase, has_status) = match page {
        Some(page) => (
            page.token,
            match page.phase {
                crate::api_gather::GatherPhase::Preparing => 1,
                crate::api_gather::GatherPhase::Running => 2,
            },
            page.status.is_some(),
        ),
        None => (0, 0, false),
    };
    let fields = page.and_then(|page| page.status.as_deref()).map(|status| {
        let rows = status
            .fields
            .iter()
            .filter_map(|field| status_field_off(b, field))
            .collect::<Vec<_>>();
        b.create_vector(&rows)
    });
    let mut table = ApiGatherBuilder::new(b);
    table.add_request_id(request_id);
    table.add_phase(phase);
    table.add_has_status(has_status);
    if let Some(fields) = fields {
        table.add_fields(fields);
    }
    table.finish()
}

fn api_gather_outcome_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    outcome: &crate::api_gather::GatherEnd,
) -> WIPOffset<ApiGatherOutcome<'b>> {
    use crate::api_gather::GatherEnd;
    let (end, code, message, retryable, counts) = match outcome {
        GatherEnd::Stopped { counts, .. } => (1, None, None, false, *counts),
        GatherEnd::Blocked {
            failure, counts, ..
        } => (
            2,
            Some(failure.code.as_ref()),
            Some(failure.message.as_ref()),
            failure.retryable,
            *counts,
        ),
        GatherEnd::Refused { reason, .. } => {
            (3, None, Some(reason.as_ref()), false, Default::default())
        }
        GatherEnd::Failed { reason, counts, .. } => {
            (4, None, Some(reason.as_ref()), false, *counts)
        }
    };
    let code = code.map(|value| b.create_string(value));
    let message = message.map(|value| b.create_string(value));
    let mut table = ApiGatherOutcomeBuilder::new(b);
    table.add_request_id(outcome.token());
    table.add_end(end);
    if let Some(code) = code {
        table.add_code(code);
    }
    if let Some(message) = message {
        table.add_message(message);
    }
    table.add_retryable(retryable);
    table.add_yielded(counts.yielded);
    table.add_dropped(counts.dropped);
    table.add_deposited(counts.deposited);
    table.add_trips(counts.trips);
    table.add_xp(counts.xp);
    table.finish()
}
pub(crate) const MAX_PROGRESS_FLAG_COUNT: u32 = 999_999_999;

fn progress_truth_code(truth: api::selected::Truth) -> u8 {
    match truth {
        api::selected::Truth::True => 1,
        api::selected::Truth::False => 2,
        api::selected::Truth::Unknown => 3,
    }
}

fn progress_colour_code(colour: api::snapshot::QuestListStatus) -> u8 {
    match colour {
        api::snapshot::QuestListStatus::NotStarted => 1,
        api::snapshot::QuestListStatus::InProgress => 2,
        api::snapshot::QuestListStatus::Complete => 3,
        api::snapshot::QuestListStatus::Unknown => 4,
    }
}

fn knowledge_parts(value: &api::selected::Knowledge<Arc<str>>) -> (&str, &str) {
    match value {
        api::selected::Knowledge::Known(value) => (value, ""),
        api::selected::Knowledge::Partial { known, gaps } => {
            (known, gaps.first().map_or("", |gap| gap.code.as_ref()))
        }
        api::selected::Knowledge::Unknown(gap) => ("", gap.code.as_ref()),
    }
}

fn progress_row_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    row: &crate::api_progress::QuestProgressRow,
) -> WIPOffset<QuestProgressRow<'b>> {
    let quest = b.create_string(&row.quest);
    let display = b.create_string(&row.display);
    let (stage_value, stage_gap) = knowledge_parts(&row.stage);
    let stage = b.create_string(stage_value);
    let stage_gap = b.create_string(stage_gap);
    let (rule_value, rule_gap) = knowledge_parts(&row.rule);
    let rule = b.create_string(rule_value);
    let rule_gap = b.create_string(rule_gap);
    let binding = b.create_string(&row.binding);
    let role = row.role.as_deref().map(|value| b.create_string(value));
    let flags = row
        .flags
        .iter()
        .map(|flag| {
            let name = b.create_string(flag.flag.0.as_ref());
            let count = flag.count.map_or(-1, |count| {
                assert!(
                    count <= MAX_PROGRESS_FLAG_COUNT,
                    "quest progress flag count exceeds the nine-digit wire bound"
                );
                count as i32
            });
            let mut table = ProgressFlagRowBuilder::new(b);
            table.add_flag(name);
            table.add_truth(progress_truth_code(flag.truth));
            table.add_count(count);
            table.finish()
        })
        .collect::<Vec<_>>();
    let flags = b.create_vector(&flags);

    let mut table = QuestProgressRowBuilder::new(b);
    table.add_quest(quest);
    table.add_display(display);
    table.add_colour(progress_colour_code(row.colour));
    table.add_stage(stage);
    table.add_stage_gap(stage_gap);
    table.add_complete(progress_truth_code(row.complete));
    table.add_rule(rule);
    table.add_rule_gap(rule_gap);
    table.add_flags(flags);
    table.add_evidence_run(row.evidence.run.run);
    table.add_evidence_session(row.evidence.run.session);
    table.add_evidence_tick(row.evidence.tick);
    table.add_evidence_sequence(row.evidence.sequence);
    table.add_journal_read(row.journal_read);
    table.add_binding(binding);
    if let Some(role) = role {
        table.add_role(role);
    }
    table.finish()
}

fn api_progress_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    page: &crate::api_progress::ProgressPage,
) -> WIPOffset<ApiProgress<'b>> {
    use crate::api_progress::ProgressPage;
    let (reason, row) = match page {
        ProgressPage::Reading { .. } => (None, None),
        ProgressPage::Done { row, .. } => (None, Some(progress_row_off(b, row))),
        ProgressPage::Refused { reason, .. } => (Some(b.create_string(reason)), None),
    };
    let mut table = ApiProgressBuilder::new(b);
    table.add_request_id(page.token());
    table.add_kind(page.kind());
    if let Some(reason) = reason {
        table.add_reason(reason);
    }
    if let Some(row) = row {
        table.add_row(row);
    }
    table.finish()
}

fn booth_off<'b>(b: &mut FlatBufferBuilder<'b>, t: TileInput) -> WIPOffset<Booth<'b>> {
    let mut table = BoothBuilder::new(b);
    table.add_x(t.x);
    table.add_z(t.z);
    table.add_level(t.level);
    table.finish()
}

fn avoid_rect_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    level: Option<i32>,
) -> WIPOffset<AvoidRect<'b>> {
    let mut table = AvoidRectBuilder::new(b);
    table.add_min_x(min_x);
    table.add_max_x(max_x);
    table.add_min_z(min_z);
    table.add_max_z(max_z);
    table.add_level(level.unwrap_or(-1));
    table.finish()
}
fn avoid_catalog_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    catalog_id: &str,
) -> WIPOffset<AvoidRect<'b>> {
    let catalog_id = b.create_string(catalog_id);
    let mut table = AvoidRectBuilder::new(b);
    table.add_catalog_id(catalog_id);
    table.finish()
}

fn avoid_invalid_off<'b>(b: &mut FlatBufferBuilder<'b>) -> WIPOffset<AvoidRect<'b>> {
    avoid_catalog_off(b, "")
}

fn inspect_hop_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    hop: &InspectHopInput<'_>,
) -> WIPOffset<InspectHop<'b>> {
    let kind = b.create_string(hop.kind);
    let loc_name = b.create_string(hop.loc_name);
    let action = b.create_string(hop.action);
    let mut table = InspectHopBuilder::new(b);
    table.add_kind(kind);
    table.add_loc_id(hop.loc_id);
    table.add_loc_name(loc_name);
    table.add_action(action);
    table.add_option(hop.option);
    table.add_from_x(hop.from_x);
    table.add_from_z(hop.from_z);
    table.add_from_level(hop.from_level);
    table.add_to_x(hop.to_x);
    table.add_to_z(hop.to_z);
    table.add_to_level(hop.to_level);
    table.add_ticks(hop.ticks);
    table.finish()
}

fn bank_approach_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    row: &BankApproachInput,
) -> WIPOffset<BankApproach<'b>> {
    let mut table = BankApproachBuilder::new(b);
    table.add_loc_id(row.loc_id);
    table.add_x(row.x);
    table.add_z(row.z);
    table.add_level(row.level);
    table.add_can_operate(row.can_operate);
    table.add_dest_ok(row.dest_ok);
    table.add_dest_x(row.dest_x);
    table.add_dest_z(row.dest_z);
    table.add_dest_level(row.dest_level);
    table.finish()
}

fn collision_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    c: &CollisionViewInput<'_>,
) -> WIPOffset<Collision<'b>> {
    let flags = b.create_vector(c.flags);
    let mut table = CollisionBuilder::new(b);
    table.add_available(c.available);
    table.add_base_x(c.base_x);
    table.add_base_z(c.base_z);
    table.add_level(c.level);
    table.add_width(c.width);
    table.add_height(c.height);
    table.add_flags(flags);
    table.finish()
}

fn reach_off<'b>(b: &mut FlatBufferBuilder<'b>, r: &ReachViewInput<'_>) -> WIPOffset<Reach<'b>> {
    let walkable = b.create_vector(r.walkable);
    let reachable = b.create_vector(r.reachable);
    let reachable_adj = b.create_vector(r.reachable_adj);
    let step = b.create_vector(r.step);
    let exact_rank = b.create_vector(r.exact_rank);
    let adjacent_rank = b.create_vector(r.adjacent_rank);
    let canlight = b.create_vector(r.canlight);
    let mut table = ReachBuilder::new(b);
    table.add_available(r.available);
    table.add_base_x(r.base_x);
    table.add_base_z(r.base_z);
    table.add_level(r.level);
    table.add_width(r.width);
    table.add_height(r.height);
    table.add_walkable(walkable);
    table.add_reachable(reachable);
    table.add_reachable_adj(reachable_adj);
    table.add_step(step);
    table.add_exact_rank(exact_rank);
    table.add_adjacent_rank(adjacent_rank);
    table.add_canlight(canlight);
    table.finish()
}

fn row_off<'b>(b: &mut FlatBufferBuilder<'b>, r: &ItemRowInput<'_>) -> WIPOffset<Row<'b>> {
    let name_off = r.name.map(|n| b.create_string(n));
    let ops_offs: Vec<_> = r.ops.iter().map(|a| b.create_string(a)).collect();
    let ops_off = b.create_vector(&ops_offs);
    let mut table = RowBuilder::new(b);
    if let Some(off) = name_off {
        table.add_name(off);
    }
    table.add_count(r.count);
    table.add_id(r.id);
    table.add_ops(ops_off);
    table.add_noted(r.noted);
    table.add_cert(r.cert);
    table.add_component_id(r.component_id);
    if r.slot >= 0 {
        table.add_slot(r.slot);
    }
    table.finish()
}

fn stat_off<'b>(b: &mut FlatBufferBuilder<'b>, s: &StatInput<'_>) -> WIPOffset<Stat<'b>> {
    let name_off = b.create_string(s.name);
    let mut table = StatBuilder::new(b);
    table.add_index(s.index);
    table.add_name(name_off);
    table.add_xp(s.xp);
    table.add_base(s.base);
    table.add_effective(s.effective);
    table.finish()
}

fn scene_entity_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    e: &SceneEntityInput<'_>,
) -> WIPOffset<SceneEntity<'b>> {
    let name_off = e.name.map(|n| b.create_string(n));
    let action_offs: Vec<_> = e.actions.iter().map(|a| b.create_string(a)).collect();
    let actions_off = b.create_vector(&action_offs);
    let mut table = SceneEntityBuilder::new(b);
    table.add_index(e.index);
    table.add_id(e.id);
    if let Some(off) = name_off {
        table.add_name(off);
    }
    table.add_x(e.x);
    table.add_z(e.z);
    table.add_level(e.level);
    table.add_distance(e.distance);
    table.add_health(e.health);
    table.add_max_health(e.max_health);
    table.add_in_combat(e.in_combat);
    table.add_animating(e.animating);
    table.add_actions(actions_off);
    table.add_reachable(e.reachable);
    table.add_reachable_adj(e.reachable_adj);
    table.add_combat_level(e.combat_level);
    table.add_target_kind(e.target_kind);
    table.add_target_index(e.target_index);
    if e.size >= 1 {
        table.add_size(e.size);
        table.add_nx(e.nx);
        table.add_nz(e.nz);
    }
    if e.shape != 0 {
        table.add_shape(e.shape);
    }
    if e.angle != 0 {
        table.add_angle(e.angle);
    }
    table.finish()
}

fn chat_option_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    o: &ChatOptionInput<'_>,
) -> WIPOffset<ChatOption<'b>> {
    let text_off = b.create_string(o.text);
    let mut table = ChatOptionBuilder::new(b);
    table.add_text(text_off);
    table.add_com_id(o.com_id);
    table.finish()
}

fn chat_line_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    l: &ChatLineInput<'_>,
) -> WIPOffset<ChatLine<'b>> {
    let text_off = b.create_string(l.text);
    let username_off = l.username.map(|name| b.create_string(name));
    let mut table = ChatLineBuilder::new(b);
    table.add_seq(l.seq);
    table.add_text(text_off);
    table.add_type_(l.type_);
    if let Some(off) = username_off {
        table.add_username(off);
    }
    table.finish()
}

fn widget_text_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    w: &WidgetTextInput<'_>,
) -> WIPOffset<WidgetText<'b>> {
    let text_off = b.create_string(w.text);
    let mut table = WidgetTextBuilder::new(b);
    table.add_component_id(w.component_id);
    table.add_text(text_off);
    table.add_item_count(w.item_count);
    table.finish()
}

fn main_modal_texts_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    pair: &MainModalTextsInput<'_>,
) -> WIPOffset<MainModalTexts<'b>> {
    let text_offs = pair
        .texts
        .iter()
        .map(|line| b.create_string(line))
        .collect::<Vec<_>>();
    let texts_off = b.create_vector(&text_offs);
    let mut table = MainModalTextsBuilder::new(b);
    table.add_root(pair.root);
    table.add_texts(texts_off);
    table.finish()
}

/// The puzzle board table. All three inner slots are written unconditionally
/// (including a closed `{ -1, 0, [] }`): the table's presence is the
/// observation, and a half-written board would let the page read rows under
/// a stale identity.
fn puzzle_board_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    board: &PuzzleBoardInput<'_>,
) -> WIPOffset<PuzzleBoard<'b>> {
    let item_offs = board
        .items
        .iter()
        .map(|row| row_off(b, row))
        .collect::<Vec<_>>();
    let items_off = b.create_vector(&item_offs);
    let mut table = PuzzleBoardBuilder::new(b);
    table.add_component_id(board.component_id);
    table.add_size(board.size);
    table.add_items(items_off);
    table.finish()
}

/// One navigator-named gate short. `id` and `count` are always written — the
/// row's identity is the id — and the name only when the host obj table
/// resolved one, so a nameless short is never dropped and never invented.
fn carry_off<'b>(b: &mut FlatBufferBuilder<'b>, row: &CarryInput<'_>) -> WIPOffset<Carry<'b>> {
    let name_off = row.name.map(|name| b.create_string(name));
    let mut table = CarryBuilder::new(b);
    table.add_id(row.id);
    table.add_count(row.count);
    if let Some(off) = name_off {
        table.add_name(off);
    }
    table.finish()
}

fn quest_status_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    q: &QuestStatusInput<'_>,
) -> WIPOffset<QuestStatus<'b>> {
    let name_off = b.create_string(q.name);
    let status_off = b.create_string(q.status);
    let mut table = QuestStatusBuilder::new(b);
    table.add_name(name_off);
    table.add_status(status_off);
    // Only a supplied id is written. An absent id omits the slot so the
    // page can tell "no click target" from a real component `0`; never
    // write a sentinel for it.
    if let Some(component_id) = q.component_id {
        table.add_component_id(component_id);
    }
    table.finish()
}

fn npc_box_off<'b>(b: &mut FlatBufferBuilder<'b>, row: &NpcBoxInput) -> WIPOffset<NpcBox<'b>> {
    let points = row
        .points
        .iter()
        .flat_map(|&(x, y)| [x, y])
        .collect::<Vec<_>>();
    let points_off = b.create_vector(&points);
    let mut table = NpcBoxBuilder::new(b);
    table.add_index(row.index);
    table.add_points(points_off);
    table.finish()
}

fn side_tab_iface_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    t: SideTabIfaceInput,
) -> WIPOffset<SideTabIface<'b>> {
    let mut table = SideTabIfaceBuilder::new(b);
    table.add_index(t.index);
    table.add_id(t.id);
    table.finish()
}

fn make_button_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    btn: &MakeButtonInput,
) -> WIPOffset<MakeButton<'b>> {
    let mut table = MakeButtonBuilder::new(b);
    table.add_qty(btn.qty);
    table.add_com_id(btn.com_id);
    table.finish()
}

fn make_product_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    p: &MakeProductInput<'_>,
) -> WIPOffset<MakeProduct<'b>> {
    let name_off = b.create_string(p.name);
    let btn_offs = p
        .buttons
        .iter()
        .map(|btn| make_button_off(b, btn))
        .collect::<Vec<_>>();
    let buttons_off = b.create_vector(&btn_offs);
    let mut table = MakeProductBuilder::new(b);
    table.add_object_id(p.object_id);
    table.add_name(name_off);
    table.add_buttons(buttons_off);
    table.finish()
}

fn combat_style_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    c: &CombatStyleInput<'_>,
) -> WIPOffset<CombatStyle<'b>> {
    let label_off = b.create_string(c.label);
    let mut table = CombatStyleBuilder::new(b);
    table.add_mode(c.mode);
    table.add_label(label_off);
    table.add_component_id(c.component_id);
    table.finish()
}

fn varp_off<'b>(b: &mut FlatBufferBuilder<'b>, v: &VarpInput) -> WIPOffset<Varp<'b>> {
    let mut table = VarpBuilder::new(b);
    table.add_index(v.index);
    table.add_value(v.value);
    table.finish()
}

fn bank_stand_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    s: &BankStandInput<'_>,
) -> WIPOffset<BankStand<'b>> {
    let name_off = b.create_string(s.name);
    let kind_off = b.create_string(s.kind);
    let choose_off = s.choose.map(|c| b.create_string(c));
    let mut table = BankStandBuilder::new(b);
    table.add_name(name_off);
    table.add_x(s.x);
    table.add_z(s.z);
    table.add_level(s.level);
    table.add_kind(kind_off);
    table.add_op(s.op);
    if let Some(off) = choose_off {
        table.add_choose(off);
    }
    table.finish()
}

fn nearest_booth_table_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    s: &NearestBoothInput<'_>,
) -> WIPOffset<NearestBooth<'b>> {
    let name_off = b.create_string(s.name);
    let op_off = b.create_string(s.op);
    let mut table = NearestBoothBuilder::new(b);
    table.add_x(s.x);
    table.add_z(s.z);
    table.add_level(s.level);
    table.add_name(name_off);
    table.add_op(op_off);
    table.add_id(s.id);
    table.finish()
}
/// Encode the tick's shim interact queue as a root-`InteractBatch`
/// FlatBuffer. Tests and one-shot callers; the live path uses
/// [`IsolateBuf`].
pub fn encode_interact_batch(reqs: &[crate::shim::InteractReq]) -> Vec<u8> {
    IsolateBuf::new().encode_interact_batch(reqs)
}

fn encode_interact_batch_into(b: &mut FlatBufferBuilder<'_>, reqs: &[crate::shim::InteractReq]) {
    let offs = reqs
        .iter()
        .map(|req| interact_off(b, req))
        .collect::<Vec<_>>();
    let reqs_off = b.create_vector(&offs);
    let mut table = InteractBatchBuilder::new(b);
    table.add_reqs(reqs_off);
    let root = table.finish();
    b.finish(root, None);
}

/// The isolate→host paint limits for the structured half of a frame: rows,
/// buttons, tab bands and the names on one chrome band. The frame comes from
/// the JS-writable `__rs2b0t_host.paint`, so the caps apply where the frame is
/// built; a frame over a cap is dropped and logged, never forwarded. Their
/// first failure was the wire decoder's, and this is that check in its new
/// place. The canvas half is capped by the recorder itself.
pub fn cap_paint(paint: &crate::shim::ScriptPaint) -> Result<(), String> {
    if paint.lines.len() > MAX_PAINT_LINES {
        return Err(format!(
            "vector length {} exceeds cap {MAX_PAINT_LINES}",
            paint.lines.len()
        ));
    }
    if paint.buttons.len() > MAX_PAINT_BUTTONS {
        return Err(format!(
            "vector length {} exceeds cap {MAX_PAINT_BUTTONS}",
            paint.buttons.len()
        ));
    }
    if paint.tabs.len() > MAX_PAINT_TABS {
        return Err(format!(
            "vector length {} exceeds cap {MAX_PAINT_TABS}",
            paint.tabs.len()
        ));
    }
    for band in paint
        .strip
        .iter()
        .chain(paint.rail.iter())
        .chain(paint.tabs.iter())
    {
        if band.names.len() > MAX_CHROME_NAMES {
            return Err(format!(
                "vector length {} exceeds cap {MAX_CHROME_NAMES}",
                band.names.len()
            ));
        }
    }
    Ok(())
}

/// Decode a root-`InteractBatch` into the shim's request type. A row with
/// a missing/unknown `op` (or a request missing a required field) fails
/// the whole batch — the host logs it and drops the batch, never fatal,
/// exactly like the old JSON parse.
/// A row's typed avoid rectangles (inspect route and world walks).
fn decoded_avoid(row: &Interact<'_>) -> Vec<crate::shim::InspectAvoidWire> {
    let Some(rects) = row.avoid() else {
        return Vec::new();
    };
    rects
        .iter()
        .map(|rect| match rect.catalog_id() {
            Some("") => crate::shim::InspectAvoidWire::Unsupported,
            Some(id) => crate::shim::InspectAvoidWire::Catalog(id.to_string()),
            None => crate::shim::InspectAvoidWire::Rect {
                min_x: rect.min_x(),
                max_x: rect.max_x(),
                min_z: rect.min_z(),
                max_z: rect.max_z(),
                level: (rect.level() >= 0).then(|| rect.level()),
            },
        })
        .collect()
}
fn decoded_run_policy(
    row: &Interact<'_>,
) -> Result<Option<api::run_policy::RunPolicyOverride>, String> {
    use api::run_policy::{RunEnergyMin, RunPolicyOverride};

    let run_auto = match row.run_auto_kind() {
        RUN_OPTION_ABSENT => None,
        RUN_OPTION_FALSE_OR_FLOOR => Some(false),
        RUN_OPTION_TRUE_OR_NAN => Some(true),
        kind => return Err(format!("run-policy has unknown run_auto kind {kind}")),
    };
    let energy_min = match row.run_energy_kind() {
        RUN_OPTION_ABSENT => None,
        RUN_OPTION_FALSE_OR_FLOOR => Some(RunEnergyMin::Floor(row.run_energy_min())),
        RUN_OPTION_TRUE_OR_NAN => Some(RunEnergyMin::NotANumber),
        kind => return Err(format!("run-policy has unknown energy kind {kind}")),
    };
    if row.run_policy_clear() {
        if run_auto.is_some() || energy_min.is_some() {
            return Err("run-policy clear carries fields".to_string());
        }
        Ok(None)
    } else {
        Ok(Some(RunPolicyOverride {
            run_auto,
            energy_min,
        }))
    }
}

pub fn decode_interact_batch(buf: &[u8]) -> Result<Vec<crate::shim::InteractReq>, String> {
    let batch = InteractBatch::from_bytes(buf)?;
    let Some(rows) = batch.reqs() else {
        return Ok(Vec::new());
    };
    if rows.len() > MAX_INTERACT_REQS {
        return Err(format!(
            "vector length {} exceeds cap {MAX_INTERACT_REQS}",
            rows.len()
        ));
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        let op = row
            .op()
            .ok_or_else(|| "interact row has no op".to_string())?;
        match op {
            "open-booth" => out.push(crate::shim::InteractReq::OpenBooth {
                x: row
                    .has_x()
                    .then(|| row.x())
                    .ok_or_else(|| "open-booth has no x".to_string())?,
                z: row
                    .has_z()
                    .then(|| row.z())
                    .ok_or_else(|| "open-booth has no z".to_string())?,
                level: row
                    .has_level()
                    .then(|| row.level())
                    .ok_or_else(|| "open-booth has no level".to_string())?,
                id: row
                    .index()
                    .ok_or_else(|| "open-booth has no id".to_string())?,
                name: row.name().map(str::to_string),
                action: row.action().map(str::to_string),
            }),
            "open-stand" => out.push(crate::shim::InteractReq::OpenStand {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                kind: row.kind().unwrap_or("").to_string(),
                name: row.name().map(str::to_string),
                stand_op: row.stand_op(),
                choose: row.choose().map(str::to_string),
            }),
            "walk" => out.push(crate::shim::InteractReq::Walk {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                allow_teleports: row.action().is_some_and(|a| a == "tele" || a == "on"),
                allow_wilderness: row.allow_wilderness(),
                allow_bank_fetch: row.allow_bank_fetch(),
                request_id: row.request_id(),
                avoid: decoded_avoid(&row),
                cross: row
                    .cross()
                    .map(|names| names.iter().map(str::to_string).collect())
                    .unwrap_or_default(),
            }),
            "walk-near" => out.push(crate::shim::InteractReq::WalkNear {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                radius: row.index().unwrap_or(0),
                allow_teleports: row.action().is_some_and(|a| a == "tele" || a == "on"),
                allow_wilderness: row.allow_wilderness(),
                allow_bank_fetch: row.allow_bank_fetch(),
                request_id: row.request_id(),
                avoid: decoded_avoid(&row),
                cross: row
                    .cross()
                    .map(|names| names.iter().map(str::to_string).collect())
                    .unwrap_or_default(),
            }),
            "walk-nearest-bank" => out.push(crate::shim::InteractReq::WalkNearestBank),
            "select-bank" => out.push(crate::shim::InteractReq::SelectBank {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                allow_wilderness: row.allow_wilderness(),
                use_mage_bank: row.use_mage_bank(),
                use_zanaris_bank: row.use_zanaris_bank(),
                request_id: row.request_id(),
            }),
            "gather-run" => {
                let request_id = row.request_id();
                if request_id == 0 {
                    return Err("gather-run has no request_id".into());
                }
                let settings = row
                    .settings()
                    .map(decode_gather_settings)
                    .transpose()?
                    .unwrap_or_default();
                out.push(crate::shim::InteractReq::GatherRun {
                    request_id,
                    settings: Arc::new(settings),
                });
            }
            "gather-stop" => {
                let request_id = row.request_id();
                if request_id == 0 {
                    return Err("gather-stop has no request_id".into());
                }
                out.push(crate::shim::InteractReq::GatherStop { request_id });
            }
            "progress-read" => {
                let request_id = row.request_id();
                if request_id == 0 {
                    return Err("progress-read has no request_id".into());
                }
                let name = row
                    .name()
                    .ok_or_else(|| "progress-read has no quest id".to_string())?;
                out.push(crate::shim::InteractReq::ProgressRead {
                    request_id,
                    name: name.to_string(),
                });
            }
            "abort-walk" => out.push(crate::shim::InteractReq::AbortWalk {
                request_id: row.request_id(),
            }),
            "inspect-route" => out.push(crate::shim::InteractReq::InspectRoute {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                from_x: row.from_x(),
                from_z: row.from_z(),
                from_level: row.from_level(),
                allow_teleports: row.allow_teleports(),
                allow_wilderness: row.allow_wilderness(),
                allow_bank_fetch: row.allow_bank_fetch(),
                avoid: decoded_avoid(&row),
                cross: row
                    .cross()
                    .map(|names| names.iter().map(str::to_string).collect())
                    .unwrap_or_default(),
                request_id: row.request_id(),
            }),
            "inspect-ack" => out.push(crate::shim::InteractReq::InspectAck {
                seq: row.inspect_ack_seq(),
                generation: row.inspect_ack_generation(),
            }),
            "walk-to" => out.push(crate::shim::InteractReq::WalkTo {
                x: row.x(),
                z: row.z(),
                level: row.level(),
            }),
            "deposit" => out.push(crate::shim::InteractReq::Deposit {
                name: row
                    .name()
                    .ok_or_else(|| "deposit has no name".to_string())?
                    .to_string(),
            }),
            "withdraw" => out.push(crate::shim::InteractReq::Withdraw {
                name: row
                    .name()
                    .ok_or_else(|| "withdraw has no name".to_string())?
                    .to_string(),
                action: row
                    .action()
                    .ok_or_else(|| "withdraw has no action".to_string())?
                    .to_string(),
            }),
            "withdraw-x" => out.push(crate::shim::InteractReq::WithdrawX {
                name: row
                    .name()
                    .ok_or_else(|| "withdraw-x has no name".to_string())?
                    .to_string(),
                count: row.x(),
                bank_item_id: row
                    .bank_item_id()
                    .ok_or_else(|| "withdraw-x has no bank_item_id".to_string())?,
                lands_as_id: row
                    .lands_as_id()
                    .ok_or_else(|| "withdraw-x has no lands_as_id".to_string())?,
                action: row
                    .action()
                    .ok_or_else(|| "withdraw-x has no action".to_string())?
                    .to_string(),
                bank_generation: row
                    .bank_generation()
                    .ok_or_else(|| "withdraw-x has no bank_generation".to_string())?,
            }),
            "withdraw-load" => out.push(crate::shim::InteractReq::WithdrawLoad {
                name: row
                    .name()
                    .ok_or_else(|| "withdraw-load has no name".to_string())?
                    .to_string(),
                bank_generation: row
                    .bank_generation()
                    .ok_or_else(|| "withdraw-load has no bank_generation".to_string())?,
            }),
            "held" => out.push(crate::shim::InteractReq::Held {
                name: row
                    .name()
                    .ok_or_else(|| "held has no name".to_string())?
                    .to_string(),
                action: row
                    .action()
                    .ok_or_else(|| "held has no action".to_string())?
                    .to_string(),
                slot: None,
            }),
            "inv-button" => out.push(crate::shim::InteractReq::InvButton {
                id: row
                    .bank_item_id()
                    .ok_or_else(|| "inv-button has no id".to_string())?,
                slot: row
                    .source_item_slot()
                    .ok_or_else(|| "inv-button has no slot".to_string())?,
                component: row
                    .component_id()
                    .ok_or_else(|| "inv-button has no component".to_string())?,
                operation: row
                    .stand_op()
                    .ok_or_else(|| "inv-button has no operation".to_string())?,
                bank_generation: row.bank_generation().unwrap_or(0),
            }),
            "puzzle-move" => out.push(crate::shim::InteractReq::PuzzleMove {
                id: row
                    .bank_item_id()
                    .ok_or_else(|| "puzzle-move has no id".to_string())?,
                slot: row
                    .source_item_slot()
                    .ok_or_else(|| "puzzle-move has no slot".to_string())?,
                component: row
                    .component_id()
                    .ok_or_else(|| "puzzle-move has no component".to_string())?,
                generation: row
                    .bank_generation()
                    .ok_or_else(|| "puzzle-move has no generation".to_string())?,
            }),
            "shop-button" => out.push(crate::shim::InteractReq::ShopButton {
                kind: row
                    .kind()
                    .ok_or_else(|| "shop-button has no kind".to_string())?
                    .to_string(),
                name: row
                    .name()
                    .ok_or_else(|| "shop-button has no name".to_string())?
                    .to_string(),
                id: row
                    .bank_item_id()
                    .ok_or_else(|| "shop-button has no id".to_string())?,
                slot: row
                    .index()
                    .ok_or_else(|| "shop-button has no slot".to_string())?,
                component: row
                    .component_id()
                    .ok_or_else(|| "shop-button has no component".to_string())?,
                chunk: row
                    .stand_op()
                    .ok_or_else(|| "shop-button has no chunk".to_string())?,
            }),
            "make-panel" => out.push(crate::shim::InteractReq::MakePanel {
                id: row
                    .bank_item_id()
                    .ok_or_else(|| "make-panel has no id".to_string())?,
                slot: row
                    .source_item_slot()
                    .ok_or_else(|| "make-panel has no slot".to_string())?,
                component: row
                    .component_id()
                    .ok_or_else(|| "make-panel has no component".to_string())?,
                operation: row
                    .stand_op()
                    .ok_or_else(|| "make-panel has no operation".to_string())?,
            }),
            "close" => out.push(crate::shim::InteractReq::Close),
            "npc" => out.push(crate::shim::InteractReq::Npc {
                name: row
                    .name()
                    .ok_or_else(|| "npc has no name".to_string())?
                    .to_string(),
                action: row
                    .action()
                    .ok_or_else(|| "npc has no action".to_string())?
                    .to_string(),
                index: row.index(),
            }),
            "loc" => out.push(crate::shim::InteractReq::Loc {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                action: row
                    .action()
                    .ok_or_else(|| "loc has no action".to_string())?
                    .to_string(),
                id: row.index(),
            }),
            "obj" => out.push(crate::shim::InteractReq::Obj {
                x: row.x(),
                z: row.z(),
                level: row.level(),
                name: row.name().map(str::to_string),
                action: row
                    .action()
                    .ok_or_else(|| "obj has no action".to_string())?
                    .to_string(),
            }),
            "player" => out.push(crate::shim::InteractReq::Player {
                name: row
                    .name()
                    .ok_or_else(|| "player has no name".to_string())?
                    .to_string(),
                action: row
                    .action()
                    .ok_or_else(|| "player has no action".to_string())?
                    .to_string(),
            }),
            "use-on" => out.push(crate::shim::InteractReq::UseOn {
                name: row
                    .name()
                    .ok_or_else(|| "use-on has no name".to_string())?
                    .to_string(),
                kind: row.kind().unwrap_or("").to_string(),
                target_name: row.choose().map(str::to_string),
                x: row.x(),
                z: row.z(),
                level: row.level(),
                index: row.index(),
                source_item_id: row.source_item_id(),
                source_item_slot: row.source_item_slot(),
                target_item_id: row.target_item_id(),
                target_item_slot: row.target_item_slot(),
            }),
            "use-widget-on" => out.push(crate::shim::InteractReq::UseWidgetOn {
                component_id: row
                    .component_id()
                    .ok_or_else(|| "use-widget-on has no component_id".to_string())?,
                kind: row.kind().unwrap_or("").to_string(),
                target_name: row.choose().map(str::to_string),
                x: row.x(),
                z: row.z(),
                level: row.level(),
                index: row.index(),
            }),
            "continue" => out.push(crate::shim::InteractReq::ContinueDialog),
            "answer" => out.push(crate::shim::InteractReq::Answer {
                option: row
                    .stand_op()
                    .ok_or_else(|| "answer has no option".to_string())?,
            }),
            "answer-count" => out.push(crate::shim::InteractReq::AnswerCount { value: row.x() }),
            "if-button" => out.push(crate::shim::InteractReq::IfButton {
                component_id: row
                    .component_id()
                    .ok_or_else(|| "if-button has no component_id".to_string())?,
            }),
            "duel-accept" => out.push(crate::shim::InteractReq::DuelAccept {
                screen: row
                    .action()
                    .ok_or_else(|| "duel-accept has no screen".to_string())?
                    .to_string(),
                partner: row
                    .name()
                    .ok_or_else(|| "duel-accept has no partner".to_string())?
                    .to_string(),
                rules: row.x(),
            }),
            "close-modal" => out.push(crate::shim::InteractReq::CloseModal),
            "side-tab" => out.push(crate::shim::InteractReq::SideTab {
                tab: row
                    .stand_op()
                    .ok_or_else(|| "side-tab has no tab".to_string())?,
            }),
            "wear" => out.push(crate::shim::InteractReq::Wear {
                name: row
                    .name()
                    .ok_or_else(|| "wear has no name".to_string())?
                    .to_string(),
            }),
            "unequip" => out.push(crate::shim::InteractReq::Unequip {
                name: row
                    .name()
                    .ok_or_else(|| "unequip has no name".to_string())?
                    .to_string(),
            }),
            "set-run" => out.push(crate::shim::InteractReq::SetRun {
                on: row.action().is_some_and(|a| a == "on" || a == "true"),
            }),
            "set-retaliate" => out.push(crate::shim::InteractReq::SetRetaliate {
                on: row.action().is_some_and(|a| a == "on" || a == "true"),
            }),
            "set-note-mode" => out.push(crate::shim::InteractReq::SetNoteMode {
                on: row.action().is_some_and(|a| a == "on" || a == "true"),
            }),
            "set-camera-yaw" => out.push(crate::shim::InteractReq::SetCameraYaw { yaw: row.x() }),
            "note-progress" => out.push(crate::shim::InteractReq::NoteProgress),
            "loop-settled" => out.push(crate::shim::InteractReq::LoopSettled),
            "wait-enqueued" => out.push(crate::shim::InteractReq::WaitEnqueued),
            "wait-settled" => out.push(crate::shim::InteractReq::WaitSettled),
            "recovery-anchor" => out.push(crate::shim::InteractReq::RecoveryAnchor {
                x: row.x(),
                z: row.z(),
                level: row.level(),
            }),
            "recovery-anchor-none" => out.push(crate::shim::InteractReq::RecoveryAnchorNone),
            "channel-open" => out.push(crate::shim::InteractReq::ChannelOpen {
                channel_id: row.channel_id(),
                name: row
                    .name()
                    .ok_or_else(|| "channel-open has no name".to_string())?
                    .to_string(),
            }),
            "channel-post" => {
                let data = row.data();
                if data.as_ref().map_or(0, |bytes| bytes.len()) > crate::channel::MAX_CHANNEL_BYTES
                {
                    return Err("channel-post data exceeds cap".into());
                }
                let data = data.map(|bytes| bytes.iter().collect()).unwrap_or_default();
                out.push(crate::shim::InteractReq::ChannelPost {
                    channel_id: row.channel_id(),
                    name: row
                        .name()
                        .ok_or_else(|| "channel-post has no name".to_string())?
                        .to_string(),
                    data,
                });
            }
            "channel-close" => out.push(crate::shim::InteractReq::ChannelClose {
                channel_id: row.channel_id(),
                name: row
                    .name()
                    .ok_or_else(|| "channel-close has no name".to_string())?
                    .to_string(),
            }),
            "channel-message" => {
                let data = row.data();
                if data.as_ref().map_or(0, |bytes| bytes.len()) > crate::channel::MAX_CHANNEL_BYTES
                {
                    return Err("channel-message data exceeds cap".into());
                }
                let data = data.map(|bytes| bytes.iter().collect()).unwrap_or_default();
                out.push(crate::shim::InteractReq::ChannelMessage {
                    channel_id: row.channel_id(),
                    sender: row
                        .name()
                        .ok_or_else(|| "channel-message has no sender".to_string())?
                        .to_string(),
                    seq: row.seq(),
                    data,
                });
            }
            "channel-status" => out.push(crate::shim::InteractReq::ChannelStatus {
                channel_id: row.channel_id(),
                message: row
                    .action()
                    .ok_or_else(|| "channel-status has no message".to_string())?
                    .to_string(),
            }),
            "key" => out.push(crate::shim::InteractReq::Key {
                down: row.index() == Some(1),
                key: row
                    .action()
                    .ok_or_else(|| "key has no key".to_string())?
                    .to_string(),
                code: row.kind().unwrap_or("").to_string(),
            }),
            "mouse" => out.push(crate::shim::InteractReq::Mouse {
                down: row.index() == Some(1),
                x: row.xf().unwrap_or(f64::NAN),
                y: row.yf().unwrap_or(f64::NAN),
                button: row.level(),
                identity: row.input_identity(),
            }),
            "run-policy" => out.push(crate::shim::InteractReq::RunPolicyOverride {
                policy: decoded_run_policy(&row)?,
            }),
            other => return Err(format!("unknown interact op: {other}")),
        }
    }
    Ok(out)
}

fn interact_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    req: &crate::shim::InteractReq,
) -> WIPOffset<Interact<'b>> {
    use crate::shim::InteractReq;
    let op_off = b.create_string(match req {
        InteractReq::OpenBooth { .. } => "open-booth",
        InteractReq::OpenStand { .. } => "open-stand",
        InteractReq::Walk { .. } => "walk",
        InteractReq::WalkNear { .. } => "walk-near",
        InteractReq::WalkNearestBank => "walk-nearest-bank",
        InteractReq::SelectBank { .. } => "select-bank",
        InteractReq::GatherRun { .. } => "gather-run",
        InteractReq::GatherStop { .. } => "gather-stop",
        InteractReq::ProgressRead { .. } => "progress-read",
        InteractReq::AbortWalk { .. } => "abort-walk",
        InteractReq::InspectRoute { .. } => "inspect-route",
        InteractReq::InspectAck { .. } => "inspect-ack",
        InteractReq::WalkTo { .. } => "walk-to",
        InteractReq::Deposit { .. } => "deposit",
        InteractReq::Withdraw { .. } => "withdraw",
        InteractReq::WithdrawX { .. } => "withdraw-x",
        InteractReq::WithdrawLoad { .. } => "withdraw-load",
        InteractReq::Held { .. } => "held",
        InteractReq::InvButton { .. } => "inv-button",
        InteractReq::PuzzleMove { .. } => "puzzle-move",
        InteractReq::ShopButton { .. } => "shop-button",
        InteractReq::MakePanel { .. } => "make-panel",
        InteractReq::Close => "close",
        InteractReq::Npc { .. } => "npc",
        InteractReq::Loc { .. } => "loc",
        InteractReq::Obj { .. } => "obj",
        InteractReq::Player { .. } => "player",
        InteractReq::UseOn { .. } => "use-on",
        InteractReq::UseWidgetOn { .. } => "use-widget-on",
        InteractReq::ContinueDialog => "continue",
        InteractReq::Answer { .. } => "answer",
        InteractReq::AnswerCount { .. } => "answer-count",
        InteractReq::IfButton { .. } => "if-button",
        InteractReq::DuelAccept { .. } => "duel-accept",
        InteractReq::CloseModal => "close-modal",
        InteractReq::SideTab { .. } => "side-tab",
        InteractReq::Wear { .. } => "wear",
        InteractReq::Unequip { .. } => "unequip",
        InteractReq::SetRun { .. } => "set-run",
        InteractReq::SetRetaliate { .. } => "set-retaliate",
        InteractReq::SetNoteMode { .. } => "set-note-mode",
        InteractReq::SetCameraYaw { .. } => "set-camera-yaw",
        InteractReq::NoteProgress => "note-progress",
        InteractReq::LoopSettled => "loop-settled",
        InteractReq::WaitEnqueued => "wait-enqueued",
        InteractReq::WaitSettled => "wait-settled",
        InteractReq::RecoveryAnchor { .. } => "recovery-anchor",
        InteractReq::RecoveryAnchorNone => "recovery-anchor-none",
        InteractReq::ChannelOpen { .. } => "channel-open",
        InteractReq::ChannelPost { .. } => "channel-post",
        InteractReq::ChannelClose { .. } => "channel-close",
        InteractReq::ChannelMessage { .. } => "channel-message",
        InteractReq::ChannelStatus { .. } => "channel-status",
        InteractReq::Key { .. } => "key",
        InteractReq::Mouse { .. } => "mouse",
        InteractReq::RunPolicyOverride { .. } => "run-policy",
    });
    let kind_off = match req {
        InteractReq::OpenStand { kind, .. }
        | InteractReq::UseOn { kind, .. }
        | InteractReq::UseWidgetOn { kind, .. }
        | InteractReq::ShopButton { kind, .. } => Some(b.create_string(kind)),
        InteractReq::Key { code, .. } if !code.is_empty() => Some(b.create_string(code)),
        _ => None,
    };
    let name_off = match req {
        InteractReq::OpenBooth { name, .. } | InteractReq::OpenStand { name, .. } => {
            name.as_deref().map(|n| b.create_string(n))
        }
        InteractReq::Deposit { name }
        | InteractReq::Withdraw { name, .. }
        | InteractReq::WithdrawX { name, .. }
        | InteractReq::WithdrawLoad { name, .. }
        | InteractReq::Held { name, .. }
        | InteractReq::Npc { name, .. }
        | InteractReq::Player { name, .. }
        | InteractReq::UseOn { name, .. }
        | InteractReq::ShopButton { name, .. }
        | InteractReq::Wear { name }
        | InteractReq::Unequip { name }
        | InteractReq::ChannelOpen { name, .. }
        | InteractReq::ChannelPost { name, .. }
        | InteractReq::ChannelClose { name, .. }
        | InteractReq::ChannelMessage { sender: name, .. }
        | InteractReq::DuelAccept { partner: name, .. } => Some(b.create_string(name)),
        InteractReq::ProgressRead { name, .. } => Some(b.create_string(name)),
        InteractReq::Obj { name, .. } => name.as_deref().map(|n| b.create_string(n)),
        _ => None,
    };
    let choose_off = match req {
        InteractReq::OpenStand { choose, .. } => choose.as_deref().map(|c| b.create_string(c)),
        InteractReq::UseOn { target_name, .. } | InteractReq::UseWidgetOn { target_name, .. } => {
            target_name.as_deref().map(|n| b.create_string(n))
        }
        _ => None,
    };
    let action_off = match req {
        InteractReq::OpenBooth { action, .. } => {
            action.as_deref().map(|action| b.create_string(action))
        }
        InteractReq::Withdraw { action, .. }
        | InteractReq::WithdrawX { action, .. }
        | InteractReq::Held { action, .. } => Some(b.create_string(action)),
        InteractReq::Npc { action, .. }
        | InteractReq::Loc { action, .. }
        | InteractReq::Obj { action, .. }
        | InteractReq::Player { action, .. } => Some(b.create_string(action)),
        InteractReq::SetRun { on }
        | InteractReq::SetRetaliate { on }
        | InteractReq::SetNoteMode { on } => Some(b.create_string(if *on { "on" } else { "off" })),
        InteractReq::Walk {
            allow_teleports: true,
            ..
        }
        | InteractReq::WalkNear {
            allow_teleports: true,
            ..
        } => Some(b.create_string("tele")),
        InteractReq::Key { key, .. } => Some(b.create_string(key)),
        InteractReq::ChannelStatus { message, .. } => Some(b.create_string(message)),
        InteractReq::DuelAccept { screen, .. } => Some(b.create_string(screen)),
        _ => None,
    };
    let avoid_off = match req {
        InteractReq::InspectRoute { avoid, .. }
        | InteractReq::Walk { avoid, .. }
        | InteractReq::WalkNear { avoid, .. }
            if !avoid.is_empty() || matches!(req, InteractReq::InspectRoute { .. }) =>
        {
            let offs: Vec<_> = avoid
                .iter()
                .map(|entry| match entry {
                    crate::shim::InspectAvoidWire::Rect {
                        min_x,
                        max_x,
                        min_z,
                        max_z,
                        level,
                    } => avoid_rect_off(b, *min_x, *max_x, *min_z, *max_z, *level),
                    crate::shim::InspectAvoidWire::Catalog(id) => avoid_catalog_off(b, id),
                    crate::shim::InspectAvoidWire::Unsupported => avoid_invalid_off(b),
                })
                .collect();
            Some(b.create_vector(&offs))
        }
        _ => None,
    };
    let cross_off = match req {
        InteractReq::Walk { cross, .. }
        | InteractReq::WalkNear { cross, .. }
        | InteractReq::InspectRoute { cross, .. }
            if !cross.is_empty() =>
        {
            let names: Vec<_> = cross.iter().map(|name| b.create_string(name)).collect();
            Some(b.create_vector(&names))
        }
        _ => None,
    };
    let data_off = match req {
        InteractReq::ChannelPost { data, .. } | InteractReq::ChannelMessage { data, .. } => {
            Some(b.create_vector(data))
        }
        _ => None,
    };
    let settings_off = match req {
        InteractReq::GatherRun { settings, .. } => {
            assert!(
                gather_settings_within_limit(settings),
                "gather-run settings must pass wire validation before encoding"
            );
            Some(
                settings_vector_off(b, settings)
                    .expect("gather-run settings passed the wire validation"),
            )
        }
        _ => None,
    };
    let mut table = InteractBuilder::new(b);
    table.add_op(op_off);
    match req {
        InteractReq::GatherRun { request_id, .. } => {
            table.add_request_id(*request_id);
            table.add_settings(settings_off.expect("gather-run settings encoded"));
        }
        InteractReq::GatherStop { request_id } => {
            table.add_request_id(*request_id);
        }
        InteractReq::ProgressRead { request_id, .. } => {
            table.add_request_id(*request_id);
            table.add_name(name_off.expect("progress-read quest id encoded"));
        }
        InteractReq::ChannelOpen { channel_id, .. }
        | InteractReq::ChannelClose { channel_id, .. } => {
            table.add_channel_id(*channel_id);
            table.add_name(name_off.unwrap());
        }
        InteractReq::ChannelPost { channel_id, .. } => {
            table.add_channel_id(*channel_id);
            table.add_name(name_off.unwrap());
            table.add_data(data_off.unwrap());
        }
        InteractReq::ChannelMessage {
            channel_id, seq, ..
        } => {
            table.add_channel_id(*channel_id);
            table.add_name(name_off.unwrap());
            table.add_seq(*seq);
            table.add_data(data_off.unwrap());
        }
        InteractReq::ChannelStatus { channel_id, .. } => {
            table.add_channel_id(*channel_id);
            table.add_action(action_off.unwrap());
        }
        InteractReq::DuelAccept { rules, .. } => {
            table.add_name(name_off.unwrap());
            table.add_action(action_off.unwrap());
            table.add_x(*rules);
        }
        InteractReq::ShopButton {
            id,
            slot,
            component,
            chunk,
            ..
        } => {
            table.add_kind(kind_off.unwrap());
            table.add_name(name_off.unwrap());
            table.add_index(*slot);
            table.add_bank_item_id(*id);
            table.add_component_id(*component);
            table.add_stand_op(*chunk);
        }
        InteractReq::OpenBooth {
            x, z, level, id, ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_index(*id);
            if let Some(off) = name_off {
                table.add_name(off);
            }
            if let Some(off) = action_off {
                table.add_action(off);
            }
        }
        InteractReq::OpenStand {
            x,
            z,
            level,
            stand_op,
            ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_kind(kind_off.unwrap());
            if let Some(off) = name_off {
                table.add_name(off);
            }
            if let Some(op) = stand_op {
                table.add_stand_op(*op);
            }
            if let Some(off) = choose_off {
                table.add_choose(off);
            }
        }
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            request_id,
            allow_wilderness,
            allow_bank_fetch,
            ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_index(*radius);
            if *request_id != 0 {
                table.add_request_id(*request_id);
            }
            if let Some(off) = action_off {
                table.add_action(off);
            }
            if *allow_wilderness {
                table.add_allow_wilderness(true);
            }
            if *allow_bank_fetch {
                table.add_allow_bank_fetch(true);
            }
        }
        InteractReq::WalkNearestBank => {}
        InteractReq::SelectBank {
            x,
            z,
            level,
            allow_wilderness,
            use_mage_bank,
            use_zanaris_bank,
            request_id,
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_allow_wilderness(*allow_wilderness);
            table.add_use_mage_bank(*use_mage_bank);
            table.add_use_zanaris_bank(*use_zanaris_bank);
            table.add_request_id(*request_id);
        }
        InteractReq::AbortWalk { request_id } => {
            table.add_request_id(*request_id);
        }
        InteractReq::InspectRoute {
            x,
            z,
            level,
            from_x,
            from_z,
            from_level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
            ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_from_x(*from_x);
            table.add_from_z(*from_z);
            table.add_from_level(*from_level);
            if *allow_teleports {
                table.add_allow_teleports(true);
            }
            if *allow_wilderness {
                table.add_allow_wilderness(true);
            }
            if *allow_bank_fetch {
                table.add_allow_bank_fetch(true);
            }
            if *request_id != 0 {
                table.add_request_id(*request_id);
            }
        }
        InteractReq::InspectAck { seq, generation } => {
            if *seq != 0 {
                table.add_inspect_ack_seq(*seq);
            }
            table.add_inspect_ack_generation(*generation);
        }
        InteractReq::Walk {
            x,
            z,
            level,
            request_id,
            allow_wilderness,
            allow_bank_fetch,
            ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            if *request_id != 0 {
                table.add_request_id(*request_id);
            }
            if let Some(off) = action_off {
                table.add_action(off);
            }
            if *allow_wilderness {
                table.add_allow_wilderness(true);
            }
            if *allow_bank_fetch {
                table.add_allow_bank_fetch(true);
            }
        }
        InteractReq::WalkTo { x, z, level } | InteractReq::RecoveryAnchor { x, z, level } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            if let Some(off) = action_off {
                table.add_action(off);
            }
        }
        InteractReq::Deposit { .. } => {
            table.add_name(name_off.unwrap());
        }
        InteractReq::Withdraw { .. } => {
            table.add_name(name_off.unwrap());
            table.add_action(action_off.unwrap());
        }
        InteractReq::WithdrawX {
            count,
            bank_item_id,
            lands_as_id,
            bank_generation,
            ..
        } => {
            table.add_name(name_off.unwrap());
            table.add_x(*count);
            table.add_bank_item_id(*bank_item_id);
            table.add_lands_as_id(*lands_as_id);
            table.add_action(action_off.unwrap());
            table.add_bank_generation(*bank_generation);
        }
        InteractReq::WithdrawLoad {
            bank_generation, ..
        } => {
            table.add_name(name_off.unwrap());
            table.add_bank_generation(*bank_generation);
        }
        InteractReq::Held { slot, .. } => {
            debug_assert!(slot.is_none(), "slot-exact Held is native-only");
            table.add_name(name_off.unwrap());
            table.add_action(action_off.unwrap());
        }
        InteractReq::InvButton {
            id,
            slot,
            component,
            operation,
            bank_generation,
        } => {
            table.add_bank_item_id(*id);
            table.add_source_item_slot(*slot);
            table.add_component_id(*component);
            table.add_stand_op(*operation);
            table.add_bank_generation(*bank_generation);
        }
        InteractReq::MakePanel {
            id,
            slot,
            component,
            operation,
        } => {
            table.add_bank_item_id(*id);
            table.add_source_item_slot(*slot);
            table.add_component_id(*component);
            table.add_stand_op(*operation);
        }
        InteractReq::PuzzleMove {
            id,
            slot,
            component,
            generation,
        } => {
            table.add_bank_item_id(*id);
            table.add_source_item_slot(*slot);
            table.add_component_id(*component);
            table.add_bank_generation(*generation);
        }
        InteractReq::Close => {}
        InteractReq::Npc { index, .. } => {
            table.add_name(name_off.unwrap());
            table.add_action(action_off.unwrap());
            if let Some(idx) = index {
                table.add_index(*idx);
            }
        }
        InteractReq::Loc {
            x, z, level, id, ..
        } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            table.add_action(action_off.unwrap());
            if let Some(id) = id {
                table.add_index(*id);
            }
        }
        InteractReq::Obj { x, z, level, .. } => {
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            if let Some(off) = name_off {
                table.add_name(off);
            }
            table.add_action(action_off.unwrap());
        }
        InteractReq::Player { .. } => {
            table.add_name(name_off.unwrap());
            table.add_action(action_off.unwrap());
        }
        InteractReq::UseOn {
            x,
            z,
            level,
            index,
            source_item_id,
            source_item_slot,
            target_item_id,
            target_item_slot,
            ..
        } => {
            table.add_name(name_off.unwrap());
            table.add_kind(kind_off.unwrap());
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            if let Some(off) = choose_off {
                table.add_choose(off);
            }
            if let Some(idx) = index {
                table.add_index(*idx);
            }
            if let Some(id) = source_item_id {
                table.add_source_item_id(*id);
            }
            if let Some(slot) = source_item_slot {
                table.add_source_item_slot(*slot);
            }
            if let Some(id) = target_item_id {
                table.add_target_item_id(*id);
            }
            if let Some(slot) = target_item_slot {
                table.add_target_item_slot(*slot);
            }
        }
        InteractReq::UseWidgetOn {
            component_id,
            x,
            z,
            level,
            index,
            ..
        } => {
            table.add_component_id(*component_id);
            table.add_kind(kind_off.unwrap());
            table.add_x(*x);
            table.add_z(*z);
            table.add_level(*level);
            if let Some(off) = choose_off {
                table.add_choose(off);
            }
            if let Some(idx) = index {
                table.add_index(*idx);
            }
        }
        InteractReq::ContinueDialog
        | InteractReq::CloseModal
        | InteractReq::NoteProgress
        | InteractReq::LoopSettled
        | InteractReq::WaitEnqueued
        | InteractReq::WaitSettled
        | InteractReq::RecoveryAnchorNone => {}
        InteractReq::Answer { option } => {
            table.add_stand_op(*option);
        }
        InteractReq::AnswerCount { value } => {
            table.add_x(*value);
        }
        InteractReq::IfButton { component_id } => {
            table.add_component_id(*component_id);
        }
        InteractReq::SideTab { tab } => {
            table.add_stand_op(*tab);
        }
        InteractReq::Wear { .. } | InteractReq::Unequip { .. } => {
            table.add_name(name_off.unwrap());
        }
        InteractReq::SetRun { .. }
        | InteractReq::SetRetaliate { .. }
        | InteractReq::SetNoteMode { .. } => {
            table.add_action(action_off.unwrap());
        }
        InteractReq::SetCameraYaw { yaw } => {
            table.add_x(*yaw);
        }
        InteractReq::RunPolicyOverride { policy } => match policy {
            None => table.add_run_policy_clear(true),
            Some(policy) => {
                if let Some(run_auto) = policy.run_auto {
                    table.add_run_auto_kind(if run_auto {
                        RUN_OPTION_TRUE_OR_NAN
                    } else {
                        RUN_OPTION_FALSE_OR_FLOOR
                    });
                }
                match policy.energy_min {
                    Some(api::run_policy::RunEnergyMin::Floor(energy_min)) => {
                        table.add_run_energy_kind(RUN_OPTION_FALSE_OR_FLOOR);
                        table.add_run_energy_min(energy_min);
                    }
                    Some(api::run_policy::RunEnergyMin::NotANumber) => {
                        table.add_run_energy_kind(RUN_OPTION_TRUE_OR_NAN);
                    }
                    None => {}
                }
            }
        },
        InteractReq::Key { down, .. } => {
            table.add_action(action_off.unwrap());
            if let Some(off) = kind_off {
                table.add_kind(off);
            }
            table.add_index(if *down { 1 } else { 0 });
        }
        InteractReq::Mouse {
            down,
            x,
            y,
            button,
            identity,
        } => {
            table.add_index(if *down { 1 } else { 0 });
            table.add_level(*button);
            table.add_xf(*x);
            table.add_yf(*y);
            if *identity != 0 {
                table.add_input_identity(*identity);
            }
        }
    }
    // Inspect routes and world walks carry their avoid rectangles.
    if let Some(off) = avoid_off {
        table.add_avoid(off);
    }
    if let Some(off) = cross_off {
        table.add_cross(off);
    }
    table.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::shim::InteractReq;
    use api::snapshot::{ActorTargetView, WorldTile};

    pub(crate) fn empty_input(tick: u64) -> SnapshotInput<'static> {
        SnapshotInput {
            tick,
            here: None,
            ingame: true,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
            nearest_booth: None,
            banks: &[],
            bank: &[],
            bank_side: &[],
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            count_dialog_open: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            bank_op_result_seq: 0,
            bank_op_result: false,
            hold: false,
            ours: false,
            npcs: &[],
            locs: &[],
            players: &[],
            ground: &[],
            equipment: &[],
            chat_open: false,
            chat_continue: false,
            chat_text: None,
            chat_options: &[],
            side_tab: -1,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: None,
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 0,
            weight: 0,
            combat_level: 0,
            camera_yaw: 0,
            camera_pitch: 0,
            teleports_enabled: false,
            self_slot: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_mine: &[],
            trade_theirs: &[],
            trade_side: &[],
            trade_accept_id: -1,
            trade_decline_id: -1,
            shop_open: false,
            shop_stock: &[],
            reach: ReachViewInput::UNAVAILABLE,
            attacked_by_player: false,
            self_target_kind: 0,
            self_target_index: -1,
            widgets: &[],
            user_move_intent_seq: 0,
            walk_outcome_cancel_reason: Default::default(),
        }
    }

    fn projectile(spotanim: i32, target: Option<ActorTargetView>, t1: i32) -> ProjectileView {
        ProjectileView {
            spotanim,
            level: 0,
            src: WorldTile {
                x: t1,
                z: 20,
                level: 0,
            },
            target,
            t1,
            t2: t1 + 10,
        }
    }

    #[test]
    fn omitted_walk_outcome_fields_default_safe() {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(7);
        let root = snapshot.finish();
        b.finish(root, None);
        let view = Snapshot::from_bytes(b.finished_data()).expect("old snapshot");
        assert!(!view.has_walk_outcome_seq());
        assert_eq!(view.walk_outcome_seq(), 0);
        assert_eq!(view.walk_outcome_generation(), 0);
        assert!(!view.walk_outcome_failed());
        assert_eq!(view.walk_outcome_x(), 0);
        assert_eq!(view.walk_outcome_z(), 0);
        assert_eq!(view.walk_outcome_level(), 0);
        assert_eq!(view.walk_outcome_radius(), 0);
        assert!(!view.walk_outcome_allow_teleports());
        assert_eq!(view.walk_outcome_request_id(), 0);
        assert!(!view.has_route_inspect_seq());
        assert_eq!(view.route_inspect_seq(), 0);
        assert_eq!(view.route_inspect_request_id(), 0);
        assert!(!view.route_inspect_ok());
        assert_eq!(view.route_inspect_reason(), None);
        assert!(!view.route_inspect_bank_planned());
        assert_eq!(view.route_inspect_ticks(), 0.0);
        assert!(view.route_inspect_hops().is_none());
        assert_eq!(view.route_inspect_prev_seq(), 0);
        assert_eq!(view.route_inspect_prev_request_id(), 0);
        assert_eq!(view.route_inspect_running_id(), 0);
        assert_eq!(view.route_inspect_pending_id(), 0);
        assert_eq!(view.route_inspect_accepted_id(), 0);
        assert_eq!(view.route_inspect_replaced_id(), 0);
        assert_eq!(view.route_inspect_replaced_prev_id(), 0);
        assert_eq!(view.route_inspect_refused_id(), 0);
        assert_eq!(view.route_inspect_refused_id_2(), 0);
        assert_eq!(view.route_inspect_refused_id_3(), 0);
        assert_eq!(view.route_inspect_unobserved(), 0);
    }

    #[test]
    fn chat_page_fingerprint_round_trips_as_a_delta_scalar() {
        let native = |chat_page_fingerprint| NativeFactsInput {
            chat_page_fingerprint,
            ..NativeFactsInput::default()
        };
        crate::observed::on_reset();
        let mut input = empty_input(1);
        let (keyframe_bytes, fp) =
            encode_snapshot_delta_with_native(None, &input, NativeFactsInput::default(), false);
        let keyframe = decode_snapshot(&keyframe_bytes).expect("keyframe");
        assert!(
            keyframe.has_chat_page_fingerprint(),
            "a zero-valued keyframe scalar is still present"
        );
        assert_eq!(keyframe.chat_page_fingerprint(), 0);
        crate::observed::apply(&keyframe);

        let fingerprint = 0xfedc_ba98_7654_3210;
        input.tick = 2;
        let (changed_bytes, fp) =
            encode_snapshot_delta_with_native(Some(&fp), &input, native(fingerprint), false);
        let changed = decode_snapshot(&changed_bytes).expect("changed delta");
        assert!(changed.has_chat_page_fingerprint());
        assert_eq!(changed.chat_page_fingerprint(), fingerprint);
        assert!(
            !changed.has_ingame(),
            "only the fingerprint changed apart from always-posted fields"
        );
        crate::observed::apply(&changed);
        crate::observed::with(|scene| {
            assert_eq!(scene.latest().chat_page_fingerprint(), Some(fingerprint));
            assert_eq!(
                scene.since_login().chat_page_fingerprint(),
                Some(fingerprint)
            );
        });

        input.tick = 3;
        let (unchanged_bytes, fp) =
            encode_snapshot_delta_with_native(Some(&fp), &input, native(fingerprint), false);
        let unchanged = decode_snapshot(&unchanged_bytes).expect("unchanged delta");
        assert!(
            !unchanged.has_chat_page_fingerprint(),
            "an unchanged scalar is omitted"
        );
        crate::observed::apply(&unchanged);
        crate::observed::with(|scene| {
            assert_eq!(
                scene.latest().chat_page_fingerprint(),
                Some(fingerprint),
                "omission retains the latest observed scalar"
            );
            assert_eq!(
                scene.since_login().chat_page_fingerprint(),
                Some(fingerprint)
            );
        });

        input.tick = 4;
        input.ingame = false;
        let (logout_bytes, fp) =
            encode_snapshot_delta_with_native(Some(&fp), &input, native(fingerprint), false);
        let logout = decode_snapshot(&logout_bytes).expect("logout delta");
        assert!(logout.has_ingame());
        assert!(
            !logout.has_chat_page_fingerprint(),
            "logout does not need to resend an unchanged fingerprint"
        );
        crate::observed::apply(&logout);
        crate::observed::with(|scene| {
            assert_eq!(scene.latest().chat_page_fingerprint(), Some(fingerprint));
            assert_eq!(
                scene.since_login().chat_page_fingerprint(),
                None,
                "logout hides the prior session's scalar"
            );
        });

        input.tick = 5;
        input.ingame = true;
        let (login_bytes, fp) =
            encode_snapshot_delta_with_native(Some(&fp), &input, native(fingerprint), false);
        let login = decode_snapshot(&login_bytes).expect("login delta");
        assert!(!login.has_chat_page_fingerprint());
        crate::observed::apply(&login);
        crate::observed::with(|scene| {
            assert_eq!(
                scene.since_login().chat_page_fingerprint(),
                None,
                "an omitted scalar stays unobserved in the new session"
            );
        });

        input.tick = 6;
        let (clear_bytes, _) = encode_snapshot_delta_with_native(
            Some(&fp),
            &input,
            NativeFactsInput::default(),
            false,
        );
        let clear = decode_snapshot(&clear_bytes).expect("zero clear");
        assert!(
            clear.has_chat_page_fingerprint(),
            "zero is an explicit clear"
        );
        assert_eq!(clear.chat_page_fingerprint(), 0);
        assert!(!clear.has_ingame());
        crate::observed::apply(&clear);
        crate::observed::with(|scene| {
            assert_eq!(scene.latest().chat_page_fingerprint(), Some(0));
            assert_eq!(scene.since_login().chat_page_fingerprint(), Some(0));
        });
    }

    /// Stats rows carry base + effective (+ xp/name/index) through the blob.
    #[test]
    fn encode_decode_stat_base_and_effective() {
        let mut input = empty_input(3);
        let stats = [StatInput {
            index: 3,
            name: "hitpoints",
            xp: 1500,
            base: 10,
            effective: 7,
        }];
        input.stats = &stats;
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot decodes");
        assert!(view.has_stats());
        let got = view.stats().expect("stats vector");
        assert_eq!(got.len(), 1);
        let stat = got.get(0);
        assert_eq!(stat.index(), 3);
        assert_eq!(stat.name(), Some("hitpoints"));
        assert_eq!(stat.xp(), 1500);
        assert_eq!(stat.base(), 10);
        assert_eq!(stat.effective(), 7);
    }

    /// Task 8 — an npc SceneEntity view round-trips through encode/decode.
    #[test]
    fn encode_decode_npc_view_round_trips() {
        let actions = ["Attack".to_string(), "Pick-up".to_string()];
        let npc = SceneEntityInput {
            index: 7,
            id: 41,
            name: Some("Chicken"),
            x: 3222,
            z: 3295,
            level: 0,
            distance: 3,
            health: 3,
            max_health: 3,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 1,
            target_kind: 0,
            target_index: -1,
            size: 4,
            nx: 2832,
            nz: 9825,
            shape: 0,
            angle: 0,
        };
        let mut input = empty_input(9);
        let npcs = [npc];
        input.npcs = &npcs;
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot decodes");
        assert!(view.has_npcs(), "keyframe carries npcs");
        let got = view.npcs().expect("npc vector");
        assert_eq!(got.len(), 1);
        let npc = got.get(0);
        assert_eq!(npc.index(), 7);
        assert_eq!(npc.id(), 41);
        assert_eq!(npc.name(), Some("Chicken"));
        assert_eq!((npc.x(), npc.z(), npc.level()), (3222, 3295, 0));
        assert_eq!((npc.size(), npc.nx(), npc.nz()), (4, 2832, 9825));
        assert_eq!(
            npc.actions()
                .expect("npc actions")
                .iter()
                .collect::<Vec<_>>(),
            vec!["Attack", "Pick-up"]
        );
        assert!(
            unsafe { npc._tab.get::<i32>(SceneEntity::VT_SHAPE, None) }.is_none(),
            "non-loc rows omit zero-valued loc geometry slots"
        );
        assert!(
            unsafe { npc._tab.get::<i32>(SceneEntity::VT_ANGLE, None) }.is_none(),
            "non-loc rows omit zero-valued loc geometry slots"
        );
    }

    /// Task 8 — an omitted npc table is absent, not an empty vector.
    #[test]
    fn omitted_npc_table_is_absent_not_empty() {
        let actions = ["Attack".to_string()];
        let npc = SceneEntityInput {
            index: 1,
            id: 2,
            name: Some("Goblin"),
            x: 100,
            z: 100,
            level: 0,
            distance: 1,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 0,
            nx: 0,
            nz: 0,
            shape: 0,
            angle: 0,
        };
        let mut input = empty_input(1);
        let npcs = [npc];
        input.npcs = &npcs;
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert!(kf.has_npcs());
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(!view.has_npcs(), "unchanged npcs omitted from delta");
        assert!(view.npcs().is_none(), "absent reads as None");
    }

    #[test]
    fn old_scene_entity_defaults_size_nx_nz_zero() {
        let mut snap_b = flatbuffers::FlatBufferBuilder::new();
        let ent_off = {
            let mut entity = SceneEntityBuilder::new(&mut snap_b);
            entity.add_index(7);
            entity.add_x(10);
            entity.add_z(20);
            entity.finish()
        };
        let entities = snap_b.create_vector(&[ent_off]);
        let root = {
            let mut snapshot = SnapshotBuilder::new(&mut snap_b);
            snapshot.add_tick(1);
            snapshot.add_npcs(entities);
            snapshot.finish()
        };
        snap_b.finish(root, None);
        let snap = Snapshot::from_bytes(snap_b.finished_data()).expect("old snap");
        let got = snap.npcs().expect("npc vector");
        assert_eq!(got.len(), 1);
        let npc = got.get(0);
        assert_eq!(npc.size(), 0);
        assert_eq!(npc.nx(), 0);
        assert_eq!(npc.nz(), 0);
        assert_eq!(npc.target_index(), -1);
    }

    #[test]
    fn old_snapshot_self_target_defaults_none() {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(7);
        let root = snapshot.finish();
        b.finish(root, None);
        let view = Snapshot::from_bytes(b.finished_data()).expect("old snapshot");
        assert!(!view.has_self_target_kind());
        assert!(!view.has_self_target_index());
        assert_eq!(view.self_target_kind(), 0);
        assert_eq!(view.self_target_index(), -1);
    }

    #[test]
    fn packed_size_zero_world_is_present_when_size_at_least_one() {
        let actions: [String; 0] = [];
        let npc = SceneEntityInput {
            index: 1,
            id: 1,
            name: Some("Man"),
            x: 0,
            z: 0,
            level: 0,
            distance: 0,
            health: 1,
            max_health: 1,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: 0,
            nz: 0,
            shape: 0,
            angle: 0,
        };
        let mut input = empty_input(2);
        let npcs = [npc];
        input.npcs = &npcs;
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot");
        let npc = view.npcs().expect("npc vector").get(0);
        assert_eq!(npc.size(), 1);
        assert_eq!((npc.nx(), npc.nz()), (0, 0));
    }

    #[test]
    fn loc_row_omits_size_slots() {
        let actions: [String; 0] = [];
        let loc = SceneEntityInput {
            index: 9,
            id: 9,
            name: Some("Tree"),
            x: 10,
            z: 10,
            level: 0,
            distance: 1,
            health: -1,
            max_health: -1,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 0,
            nx: 0,
            nz: 0,
            shape: 9,
            angle: 1,
        };
        let mut input = empty_input(3);
        let locs = [loc];
        input.locs = &locs;
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot");
        let loc = view.locs().expect("loc vector").get(0);
        assert_eq!(loc.size(), 0);
        assert_eq!((loc.nx(), loc.nz()), (0, 0));
        assert_eq!((loc.shape(), loc.angle()), (9, 1));
    }

    #[test]
    fn fingerprint_changes_when_size_or_network_origin_changes() {
        let actions = ["Attack".to_string()];
        let mut npc = SceneEntityInput {
            index: 1,
            id: 9,
            name: Some("Goblin"),
            x: 10,
            z: 10,
            level: 0,
            distance: 1,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 2,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: 10,
            nz: 10,
            shape: 0,
            angle: 0,
        };
        let mut input = empty_input(4);
        let npcs = [npc];
        input.npcs = &npcs;
        let fp1 = SnapshotFingerprint::from_input(&input);
        npc.size = 4;
        let npcs = [npc];
        input.npcs = &npcs;
        let fp2 = SnapshotFingerprint::from_input(&input);
        assert_ne!(fp1.npcs, fp2.npcs);
        npc.size = 4;
        npc.nx = 11;
        let npcs = [npc];
        input.npcs = &npcs;
        let fp3 = SnapshotFingerprint::from_input(&input);
        assert_ne!(fp2.npcs, fp3.npcs);
        npc.nx = 11;
        npc.nz = 12;
        let npcs = [npc];
        input.npcs = &npcs;
        let fp4 = SnapshotFingerprint::from_input(&input);
        assert_ne!(fp3.npcs, fp4.npcs);
    }

    #[test]
    fn self_target_delta_omits_when_unchanged_and_posts_on_change() {
        let mut input = empty_input(5);
        input.self_target_kind = 1;
        input.self_target_index = 42;
        let (kf, fp) = encode_snapshot_delta(None, &input, false);
        let kf_view = decode_snapshot(&kf).expect("kf");
        assert_eq!(kf_view.self_target_kind(), 1);
        assert_eq!(kf_view.self_target_index(), 42);
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let d = decode_snapshot(&delta).expect("delta");
        assert!(!d.has_self_target_kind());
        assert!(!d.has_self_target_index());
        input.self_target_kind = 2;
        input.self_target_index = 7;
        let (delta2, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let d2 = decode_snapshot(&delta2).expect("delta2");
        assert!(d2.has_self_target_kind());
        assert_eq!(d2.self_target_kind(), 2);
        assert_eq!(d2.self_target_index(), 7);
    }

    #[test]
    fn wake_retains_unobserved_projections_until_a_tick_explicitly_clears_them() {
        let mut buf = IsolateBuf::new();
        let mut input = empty_input(5);
        let inventory = [ItemRowInput::nc(Some("Coins"), 42)];
        input.inv = &inventory;
        input.inv_size = 28;
        let boxes = [NpcBoxInput {
            index: 7,
            points: [(12, 34); 8],
        }];
        let (_, mut fp) = buf.encode_snapshot_delta_with_native(
            None,
            &input,
            NativeFactsInput {
                npc_boxes: Some(&boxes),
                ..NativeFactsInput::default()
            },
            false,
        );
        input.inv_size = 0;
        input.inv = &[];
        for _ in 0..2 {
            let (bytes, next) = buf.encode_snapshot_wake_with_native(
                Some(&mut fp),
                &input,
                NativeFactsInput::default(),
                false,
                true,
            );
            let wake = decode_snapshot(&bytes).unwrap();
            assert!(!wake.has_npc_boxes_available());
            assert!(!wake.has_npc_boxes());
            assert!(!wake.has_inv());
            assert!(!wake.has_inv_size());
            fp = next;
        }
        let (bytes, _) = buf.encode_snapshot_delta_with_native(
            Some(&fp),
            &input,
            NativeFactsInput::default(),
            false,
        );
        let tick = decode_snapshot(&bytes).unwrap();
        assert!(tick.has_npc_boxes_available());
        assert!(!tick.npc_boxes_available());
        assert!(tick.has_inv_size());
        assert_eq!(tick.inv_size(), 0);
        assert!(tick.has_inv());
        assert_eq!(tick.inv().unwrap().len(), 0);
    }

    /// The local player's animation id rides the keyframe, is omitted while
    /// unchanged, posts on a change (idle `-1` included), and an old or
    /// unsupplied buffer reads as absent `-1`.
    #[test]
    fn self_anim_delta_omits_when_unchanged_and_posts_on_change() {
        let input = empty_input(5);
        let native = |anim| NativeFactsInput {
            self_anim: Some(anim),
            ..NativeFactsInput::default()
        };
        let (kf, fp) = encode_snapshot_delta_with_native(None, &input, native(390), false);
        let kf_view = decode_snapshot(&kf).expect("kf");
        assert!(kf_view.has_self_anim());
        assert_eq!(kf_view.self_anim(), 390);
        let (same, _) = encode_snapshot_delta_with_native(Some(&fp), &input, native(390), false);
        assert!(!decode_snapshot(&same).expect("same").has_self_anim());
        let (idle, _) = encode_snapshot_delta_with_native(Some(&fp), &input, native(-1), false);
        let idle = decode_snapshot(&idle).expect("idle");
        assert!(idle.has_self_anim());
        assert_eq!(idle.self_anim(), -1);
        let (old, _) = encode_snapshot_delta(None, &input, false);
        let old = decode_snapshot(&old).expect("old");
        assert!(!old.has_self_anim());
        assert_eq!(old.self_anim(), -1);
    }

    #[test]
    fn projectile_encoder_filters_to_self_and_ignores_irrelevant_changes() {
        let mut input = empty_input(5);
        input.self_slot = 4;
        let local = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 4,
        });
        let other = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 3,
        });
        let npc = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 7,
        });
        let initial = [
            projectile(91, other, 10),
            projectile(92, npc, 20),
            projectile(93, None, 30),
            projectile(44, local, 40),
        ];
        let native = NativeFactsInput {
            projectiles: Some(&initial),
            ..NativeFactsInput::default()
        };
        let (keyframe, fingerprint) =
            encode_snapshot_delta_with_native(None, &input, native, false);
        let page = decode_snapshot(&keyframe)
            .expect("keyframe")
            .projectiles()
            .expect("present projectile page");
        assert_eq!(page.len(), 1);
        assert_eq!(page.get(0).spotanim(), 44);
        assert_eq!(page.get(0).target_player_index(), Some(4));

        // Other actors and source/timing changes are not consumed by policy.
        let irrelevant_changes = [
            projectile(191, other, 110),
            projectile(192, npc, 120),
            projectile(193, None, 130),
            projectile(44, local, 140),
        ];
        let native = NativeFactsInput {
            projectiles: Some(&irrelevant_changes),
            ..NativeFactsInput::default()
        };
        let (delta, next_fingerprint) =
            encode_snapshot_delta_with_native(Some(&fingerprint), &input, native, false);
        assert!(decode_snapshot(&delta)
            .expect("unchanged delta")
            .projectiles()
            .is_none());

        let relevant_change = [projectile(45, local, 140)];
        let native = NativeFactsInput {
            projectiles: Some(&relevant_change),
            ..NativeFactsInput::default()
        };
        let (delta, _) =
            encode_snapshot_delta_with_native(Some(&next_fingerprint), &input, native, false);
        let page = decode_snapshot(&delta)
            .expect("changed delta")
            .projectiles()
            .expect("changed projectile page");
        assert_eq!(page.len(), 1);
        assert_eq!(page.get(0).spotanim(), 45);
    }

    #[test]
    fn projectile_encoder_caps_filtered_synthetic_rows() {
        let mut input = empty_input(5);
        input.self_slot = 4;
        let local = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 4,
        });
        let other = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 3,
        });
        let npc = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 7,
        });
        let mut rows = Vec::with_capacity(MAX_PROJECTILES_PER_SNAPSHOT + 3);
        rows.extend([
            projectile(1, other, 1),
            projectile(2, npc, 2),
            projectile(3, None, 3),
        ]);
        for index in 0..MAX_PROJECTILES_PER_SNAPSHOT + 5 {
            let index = i32::try_from(index).expect("small capped test index");
            rows.push(projectile(700 + index, local, index));
            rows.push(projectile(900 + index, other, index));
        }

        let native = NativeFactsInput {
            projectiles: Some(&rows),
            ..NativeFactsInput::default()
        };
        let (keyframe, _) = encode_snapshot_delta_with_native(None, &input, native, false);
        let page = decode_snapshot(&keyframe)
            .expect("keyframe")
            .projectiles()
            .expect("present projectile page");
        assert_eq!(page.len(), MAX_PROJECTILES_PER_SNAPSHOT);
        for index in 0..MAX_PROJECTILES_PER_SNAPSHOT {
            let spotanim = 700 + i32::try_from(index).expect("small capped test index");
            assert_eq!(page.get(index).spotanim(), spotanim);
            assert_eq!(page.get(index).target_player_index(), Some(4));
        }
    }

    #[test]
    fn projectile_empty_vector_delta_clears_but_none_omits() {
        let input = empty_input(5);
        let rows = [projectile(
            44,
            Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            40,
        )];
        let native = NativeFactsInput {
            projectiles: Some(&rows),
            ..NativeFactsInput::default()
        };
        let (keyframe, fingerprint) =
            encode_snapshot_delta_with_native(None, &input, native, false);
        assert_eq!(
            decode_snapshot(&keyframe)
                .expect("keyframe")
                .projectiles()
                .expect("present page")
                .len(),
            1
        );

        let (omitted, omitted_fingerprint) = encode_snapshot_delta_with_native(
            Some(&fingerprint),
            &input,
            NativeFactsInput::default(),
            false,
        );
        assert!(decode_snapshot(&omitted)
            .expect("omitted delta")
            .projectiles()
            .is_none());

        let native = NativeFactsInput {
            projectiles: Some(&[]),
            ..NativeFactsInput::default()
        };
        let (clear, _) =
            encode_snapshot_delta_with_native(Some(&omitted_fingerprint), &input, native, false);
        assert_eq!(
            decode_snapshot(&clear)
                .expect("empty-page delta")
                .projectiles()
                .expect("explicit clear page")
                .len(),
            0
        );
    }

    #[test]
    fn reset_posts_empty_npcs_and_none_target() {
        let actions = ["Attack".to_string()];
        let npc = SceneEntityInput {
            index: 1,
            id: 2,
            name: Some("Goblin"),
            x: 100,
            z: 100,
            level: 0,
            distance: 1,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: 100,
            nz: 100,
            shape: 0,
            angle: 0,
        };
        let mut input = empty_input(6);
        let npcs = [npc];
        input.npcs = &npcs;
        input.self_target_kind = 1;
        input.self_target_index = 1;
        let (_, fp) = encode_snapshot_delta(None, &input, false);
        let mut reset = empty_input(7);
        reset.self_target_kind = 0;
        reset.self_target_index = -1;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &reset, false);
        let view = decode_snapshot(&delta).expect("reset");
        assert!(view.has_npcs());
        assert!(view.npcs().expect("cleared npc vector").is_empty());
        assert_eq!(view.self_target_kind(), 0);
        assert_eq!(view.self_target_index(), -1);
    }

    /// Task 8 — interact `npc` + label round-trips through the batch codec.
    #[test]
    fn encode_decode_interact_npc_pick_round_trips() {
        let reqs = vec![InteractReq::Npc {
            name: "Chicken".into(),
            action: "Pick".into(),
            index: None,
        }];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("interact batch decodes");
        assert_eq!(got, reqs);
    }

    #[test]
    fn encode_decode_run_policy_replacements_round_trip() {
        use api::run_policy::{RunEnergyMin, RunPolicyOverride};

        let reqs = vec![
            InteractReq::RunPolicyOverride {
                policy: Some(RunPolicyOverride {
                    run_auto: Some(false),
                    energy_min: Some(RunEnergyMin::Floor(80)),
                }),
            },
            InteractReq::RunPolicyOverride {
                policy: Some(RunPolicyOverride {
                    run_auto: Some(true),
                    energy_min: Some(RunEnergyMin::NotANumber),
                }),
            },
            InteractReq::RunPolicyOverride {
                policy: Some(RunPolicyOverride::default()),
            },
            InteractReq::RunPolicyOverride { policy: None },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("run-policy batch decodes");
        assert_eq!(got, reqs);
    }

    /// `wear` and `unequip` share the name slot; the op string keeps a
    /// removal from decoding as a wear of the same item.
    #[test]
    fn encode_decode_interact_wear_and_unequip_round_trip() {
        let reqs = vec![
            InteractReq::Wear {
                name: "Iron chainbody".into(),
            },
            InteractReq::Unequip {
                name: "Iron chainbody".into(),
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("interact batch decodes");
        assert_eq!(got, reqs);
    }

    /// `abort-walk` crosses the wire as its own op, in batch order, so the
    /// host stops the follow before the click queued after it.
    #[test]
    fn encode_decode_interact_abort_walk_keeps_its_place_in_the_batch() {
        let reqs = vec![
            InteractReq::AbortWalk { request_id: 42 },
            InteractReq::Loc {
                x: 7,
                z: 5,
                level: 0,
                action: "Open".into(),
                id: Some(1530),
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("interact batch decodes");
        assert_eq!(got, reqs);
    }

    /// `puzzle-move` round-trips on the reused Interact slots (id / slot /
    /// component / generation) even when a bank row carries the same four
    /// numbers: the op string is the discriminator, and the operation slot
    /// the component family needs stays unset for the board row.
    #[test]
    fn encode_decode_interact_puzzle_move_round_trips_beside_a_bank_row() {
        let reqs = vec![
            InteractReq::PuzzleMove {
                id: 2749,
                slot: 0,
                component: 6600,
                generation: 3,
            },
            InteractReq::InvButton {
                id: 2749,
                slot: 0,
                component: 6600,
                operation: 5,
                bank_generation: 3,
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("interact batch decodes");
        assert_eq!(
            got, reqs,
            "the op string decides the variant, not the slots"
        );
    }

    #[test]
    fn encode_decode_interact_key_down_and_enter_up_round_trips() {
        let reqs = vec![
            InteractReq::Key {
                down: true,
                key: "2".into(),
                code: "2".into(),
            },
            InteractReq::Key {
                down: false,
                key: "Enter".into(),
                code: "Enter".into(),
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("key batch decodes");
        assert_eq!(got, reqs);
    }

    #[test]
    fn decode_interact_unknown_op_still_fails_the_batch() {
        let bytes = encode_interact_batch(&[InteractReq::Key {
            down: true,
            key: "2".into(),
            code: String::new(),
        }]);
        let got = decode_interact_batch(&bytes).expect("empty code still decodes");
        assert_eq!(
            got,
            vec![InteractReq::Key {
                down: true,
                key: "2".into(),
                code: String::new(),
            }]
        );
        assert!(
            decode_interact_batch(b"not a batch").is_err(),
            "unknown bytes fail closed"
        );
    }

    #[test]
    fn old_gather_run_without_settings_decodes_as_default_bag() {
        let mut builder = FlatBufferBuilder::new();
        let op = builder.create_string("gather-run");
        let mut row = InteractBuilder::new(&mut builder);
        row.add_op(op);
        row.add_request_id(42);
        let row = row.finish();
        let reqs = builder.create_vector(&[row]);
        let mut batch = InteractBatchBuilder::new(&mut builder);
        batch.add_reqs(reqs);
        let root = batch.finish();
        builder.finish(root, None);

        let decoded =
            decode_interact_batch(builder.finished_data()).expect("older gather-run decodes");
        let [InteractReq::GatherRun {
            request_id,
            settings,
        }] = decoded.as_slice()
        else {
            panic!("older gather-run row retained its operation");
        };
        assert_eq!(*request_id, 42);
        assert!(settings.is_empty());
    }

    #[test]
    fn gather_settings_guard_enforces_encoded_size_and_typed_limits() {
        let mut small = crate::native::SettingsBag::new();
        small.insert("skill".into(), serde_json::json!("Mining"));
        assert!(gather_settings_within_limit(&small));

        let mut oversized = crate::native::SettingsBag::new();
        oversized.insert(
            "text".into(),
            serde_json::Value::String("x".repeat(GATHER_SETTINGS_MAX_BYTES * 2)),
        );
        assert!(!gather_settings_within_limit(&oversized));

        let mut too_many_items = crate::native::SettingsBag::new();
        too_many_items.insert("list".into(), serde_json::json!(vec![""; 513]));
        assert!(!gather_settings_within_limit(&too_many_items));

        for value in [
            serde_json::Value::Null,
            serde_json::json!(12.5),
            serde_json::json!({"x": 3200, "z": 3210}),
        ] {
            let mut unsupported = crate::native::SettingsBag::new();
            unsupported.insert("value".into(), value);
            assert!(!gather_settings_within_limit(&unsupported));
        }
    }

    #[test]
    fn old_snapshot_without_gather_tables_remains_valid() {
        let mut builder = FlatBufferBuilder::new();
        let mut snapshot = SnapshotBuilder::new(&mut builder);
        snapshot.add_tick(3);
        let root = snapshot.finish();
        builder.finish(root, None);

        let decoded = Snapshot::from_bytes(builder.finished_data()).expect("old snapshot verifies");
        assert_eq!(decoded.tick(), 3);
        assert!(!decoded.has_api_gather());
        assert!(!decoded.has_api_gather_outcome());
    }

    #[test]
    fn encode_decode_interact_mouse_center_and_up_round_trips() {
        let reqs = vec![
            InteractReq::Mouse {
                down: true,
                x: 382.5,
                y: 251.5,
                button: 0,
                identity: 7,
            },
            InteractReq::Mouse {
                down: false,
                x: 382.5,
                y: 251.5,
                button: 0,
                identity: 7,
            },
            InteractReq::Key {
                down: true,
                key: "2".into(),
                code: "2".into(),
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("mouse batch decodes");
        assert_eq!(got, reqs);
    }

    #[test]
    fn encode_decode_mouse_preserves_negative_fraction() {
        let reqs = vec![InteractReq::Mouse {
            down: true,
            x: -0.25,
            y: 10.0,
            button: 0,
            identity: 0,
        }];
        let bytes = encode_interact_batch(&reqs);
        let got = decode_interact_batch(&bytes).expect("neg fraction decodes");
        match &got[0] {
            InteractReq::Mouse { x, y, .. } => {
                assert!((*x - -0.25).abs() < 1e-12, "{x}");
                assert!((*y - 10.0).abs() < 1e-12, "{y}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Task 8 fix — delta with mask set clears optional `chat_text`.
    #[test]
    fn optional_string_delta_clears_chat_text() {
        let mut input = empty_input(1);
        input.chat_text = Some("hi");
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert_eq!(kf.chat_text(), Some("hi"));

        input.chat_text = None;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(
            view.has_chat_text(),
            "cleared chat_text is present in delta"
        );
        assert_eq!(view.chat_text(), Some(""), "None encodes as empty string");
    }

    /// Task 8 fix — delta with mask set clears optional `my_name`.
    #[test]
    fn optional_string_delta_clears_my_name() {
        let mut input = empty_input(1);
        input.my_name = Some("Alice");
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert_eq!(kf.my_name(), Some("Alice"));

        input.my_name = None;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_my_name(), "cleared my_name is present in delta");
        assert_eq!(view.my_name(), Some(""), "None encodes as empty string");
    }

    #[test]
    fn held_wire_roundtrip_retains_legacy_unspecified_slot() {
        let request = InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: None,
        };
        assert_eq!(
            decode_interact_batch(&encode_interact_batch(std::slice::from_ref(&request))).unwrap(),
            vec![request]
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "slot-exact Held is native-only")]
    fn slot_exact_held_cannot_cross_the_isolate_wire() {
        encode_interact_batch(&[InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: Some(7),
        }]);
    }

    /// One reusable builder encodes snapshot, then paint, then interact —
    /// the per-slot / per-V8 buffer, reset between messages, never a
    /// JSON document and never `FlatBufferBuilder::new()` per tick.
    #[test]
    fn one_isolate_buf_encodes_snapshot_then_paint_then_interact() {
        let mut buf = IsolateBuf::new();
        let bytes = buf.encode_snapshot(&empty_input(1));
        let snap = Snapshot::from_bytes(&bytes).expect("snapshot");
        assert_eq!(snap.tick(), 1);

        let reqs = vec![
            InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into(),
                slot: None,
            },
            InteractReq::Walk {
                x: 1,
                z: 2,
                level: 0,
                allow_teleports: true,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 9,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
            InteractReq::WalkNear {
                x: 2656,
                z: 3286,
                level: 0,
                radius: 3,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 0,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
            InteractReq::WalkTo {
                x: 3,
                z: 4,
                level: 0,
            },
            InteractReq::NoteProgress,
            InteractReq::LoopSettled,
            InteractReq::WaitEnqueued,
            InteractReq::WaitSettled,
            InteractReq::RecoveryAnchor {
                x: 10,
                z: 20,
                level: 1,
            },
            InteractReq::RecoveryAnchorNone,
        ];
        let ibytes = buf.encode_interact_batch(&reqs);
        let got = decode_interact_batch(&ibytes).expect("interact");
        assert_eq!(got, reqs);
    }

    #[test]
    fn walk_find_options_roundtrip_through_interact_batch() {
        let reqs = vec![
            InteractReq::Walk {
                x: 3100,
                z: 3525,
                level: 1,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: 11,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
            InteractReq::WalkNear {
                x: 3222,
                z: 3222,
                level: 0,
                radius: 2,
                allow_teleports: true,
                allow_wilderness: false,
                allow_bank_fetch: true,
                request_id: 12,
                avoid: Vec::new(),
                cross: Vec::new(),
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        assert_eq!(decode_interact_batch(&bytes).expect("decode"), reqs);
    }

    #[test]
    fn inspect_ack_roundtrip_preserves_seq_and_generation() {
        let reqs = vec![InteractReq::InspectAck {
            seq: 4,
            generation: 2,
        }];
        let bytes = encode_interact_batch(&reqs);
        assert_eq!(decode_interact_batch(&bytes).expect("ack"), reqs);
    }

    #[test]
    fn old_walk_buffers_default_new_find_options_false() {
        let mut b = FlatBufferBuilder::new();
        let op_off = b.create_string("walk");
        let row = {
            let mut interact = InteractBuilder::new(&mut b);
            interact.add_op(op_off);
            interact.add_x(1);
            interact.add_z(2);
            interact.add_level(0);
            interact.add_request_id(7);
            interact.finish()
        };
        let reqs = b.create_vector(&[row]);
        let root = {
            let mut batch = InteractBatchBuilder::new(&mut b);
            batch.add_reqs(reqs);
            batch.finish()
        };
        b.finish(root, None);
        let got = decode_interact_batch(b.finished_data()).expect("old walk");
        assert_eq!(
            got,
            vec![InteractReq::Walk {
                x: 1,
                z: 2,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 7,
                avoid: Vec::new(),
                cross: Vec::new(),
            }]
        );
    }

    /// Truncated isolate→host buffers must not panic; invalid roots err.
    #[test]
    fn truncated_paint_and_interact_buffers_return_err() {
        let reqs = vec![InteractReq::Close];
        let ibytes = IsolateBuf::new().encode_interact_batch(&reqs);
        for cut in 1..ibytes.len() {
            let _ = decode_interact_batch(&ibytes[..cut]);
        }
        let mid = ibytes.len().saturating_sub(4);
        assert!(
            decode_interact_batch(&ibytes[..mid]).is_err(),
            "interact truncated mid-payload should err"
        );
        let mut bad_root = ibytes.clone();
        bad_root[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            decode_interact_batch(&bad_root).is_err(),
            "huge interact root offset should err"
        );
    }

    /// Resetting the same builder must not leave the previous root's tick
    /// in the finished bytes.
    #[test]
    fn reused_isolate_buf_second_snapshot_does_not_keep_first_tick() {
        let mut buf = IsolateBuf::new();
        let _ = buf.encode_snapshot(&empty_input(1));
        let bytes = buf.encode_snapshot(&empty_input(2));
        let snap = Snapshot::from_bytes(&bytes).expect("snapshot");
        assert_eq!(snap.tick(), 2);
    }

    fn reach_words(bits: &[usize]) -> Vec<u32> {
        let mut words = vec![0u32; 3];
        for &i in bits {
            words[i / 32] |= 1u32 << (i % 32);
        }
        words
    }

    #[test]
    fn encode_decode_reach_view_round_trips() {
        let walkable = reach_words(&[0, 31, 32, 53, 63, 64]);
        let reachable = reach_words(&[0, 32, 53]);
        let adj = reach_words(&[0, 31, 32, 53, 64]);
        let mut exact_rank = vec![u16::MAX; 9 * 8];
        exact_rank[0] = 0;
        exact_rank[32] = 7;
        exact_rank[53] = 11;
        let mut adjacent_rank = exact_rank.clone();
        adjacent_rank[31] = 6;
        adjacent_rank[64] = 13;
        let mut input = empty_input(4);
        input.reach = ReachViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 9,
            height: 8,
            walkable: &walkable,
            reachable: &reachable,
            reachable_adj: &adj,
            exact_rank: &exact_rank,
            adjacent_rank: &adjacent_rank,
            step: &[],
            canlight: &walkable,
            stamp: 0,
        };
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot decodes");
        assert!(view.has_reach(), "keyframe carries reach");
        let reach = view.reach().expect("reach table");
        assert!(reach.available());
        assert_eq!(
            (
                reach.base_x(),
                reach.base_z(),
                reach.level(),
                reach.width(),
                reach.height()
            ),
            (3200, 3200, 0, 9, 8)
        );
        let got_walkable = reach.walkable().expect("walkable bits");
        assert_eq!(got_walkable.iter().collect::<Vec<_>>(), walkable);
        assert_eq!(
            reach
                .reachable()
                .expect("reachable bits")
                .iter()
                .collect::<Vec<_>>(),
            reachable
        );
        assert_eq!(
            reach
                .reachable_adj()
                .expect("adjacent bits")
                .iter()
                .collect::<Vec<_>>(),
            adj
        );
        assert_eq!(
            reach
                .exact_rank()
                .expect("exact ranks")
                .iter()
                .collect::<Vec<_>>(),
            exact_rank
        );
        assert_eq!(
            reach
                .adjacent_rank()
                .expect("adjacent ranks")
                .iter()
                .collect::<Vec<_>>(),
            adjacent_rank
        );
        assert!(reach.step().expect("step vector").is_empty());
        assert_eq!(
            reach
                .canlight()
                .expect("lighting bits")
                .iter()
                .collect::<Vec<_>>(),
            walkable
        );
        assert_eq!(got_walkable.get(0) & (1 << 31), 1 << 31, "bit 31 in word 0");
        assert_eq!(got_walkable.get(1) & 1, 1, "bit 32 in word 1");
        assert_eq!(got_walkable.get(1) & (1 << 21), 1 << 21, "bit 53 in word 1");
        assert_eq!(got_walkable.get(1) & (1 << 31), 1 << 31, "bit 63 in word 1");
        assert_eq!(got_walkable.get(2) & 1, 1, "bit 64 in word 2");
    }

    #[test]
    fn omitted_reach_is_absent_when_unchanged() {
        let mut input = empty_input(1);
        let walkable = reach_words(&[1]);
        let step = vec![0u8, 2, 0];
        let rank = vec![0u16, 1, u16::MAX];
        input.reach = ReachViewInput {
            available: true,
            base_x: 1,
            base_z: 2,
            level: 0,
            width: 9,
            height: 8,
            walkable: &walkable,
            reachable: &walkable,
            reachable_adj: &walkable,
            exact_rank: &rank,
            adjacent_rank: &rank,
            step: &step,
            canlight: &[],
            stamp: 0,
        };
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert!(kf.has_reach());
        assert_eq!(
            kf.reach()
                .expect("reach")
                .step()
                .expect("step vector")
                .iter()
                .collect::<Vec<_>>(),
            step
        );
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(!view.has_reach(), "unchanged reach omitted from delta");
        assert!(view.reach().is_none());
    }

    #[test]
    fn unavailable_reach_is_posted_when_cleared() {
        let walkable = reach_words(&[0]);
        let rank = vec![0u16];
        let mut input = empty_input(1);
        input.reach = ReachViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 9,
            height: 8,
            walkable: &walkable,
            reachable: &walkable,
            reachable_adj: &walkable,
            exact_rank: &rank,
            adjacent_rank: &rank,
            step: &[2],
            canlight: &[],
            stamp: 0,
        };
        let (_keyframe, fp) = encode_snapshot_delta(None, &input, false);
        input.reach = ReachViewInput::UNAVAILABLE;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_reach(), "available→unavailable must post reach");
        let reach = view.reach().expect("cleared view");
        assert!(!reach.available());
        assert_eq!(reach.width(), 0);
        assert!(reach.walkable().expect("walkable bits").is_empty());
        assert!(reach.exact_rank().expect("exact ranks").is_empty());
        assert!(reach.adjacent_rank().expect("adjacent ranks").is_empty());
        assert!(reach.step().expect("step vector").is_empty());
        assert!(reach.canlight().expect("lighting bits").is_empty());
    }

    #[test]
    fn canlight_present_zeros_are_not_omitted_and_unavailable_clears_stale_bits() {
        let walkable = reach_words(&[0]);
        let rank = vec![0u16];
        let zeros = vec![0u32; 1];
        let lit = vec![1u32];
        let mut input = empty_input(1);
        input.reach = ReachViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 1,
            width: 9,
            height: 8,
            walkable: &walkable,
            reachable: &walkable,
            reachable_adj: &walkable,
            exact_rank: &rank,
            adjacent_rank: &rank,
            step: &[2],
            canlight: &lit,
            stamp: 0,
        };
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert_eq!(
            kf.reach()
                .expect("reach")
                .canlight()
                .expect("lighting bits")
                .iter()
                .collect::<Vec<_>>(),
            lit
        );

        input.reach.canlight = &zeros;
        let (delta, fp2) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_reach(), "canlight change must post reach");
        assert_eq!(
            view.reach()
                .expect("reach")
                .canlight()
                .expect("lighting bits")
                .iter()
                .collect::<Vec<_>>(),
            zeros
        );

        input.reach = ReachViewInput::UNAVAILABLE;
        let (cleared, _) = encode_snapshot_delta(Some(&fp2), &input, false);
        let view = decode_snapshot(&cleared).expect("cleared");
        assert!(view.has_reach());
        assert!(view
            .reach()
            .expect("cleared")
            .canlight()
            .expect("lighting bits")
            .is_empty());
    }

    #[test]
    fn collision_keyframe_posts_and_unchanged_delta_omits() {
        let flags = vec![0i32, 1, 2, 3];
        let mut native = NativeFactsInput {
            collision: Some(CollisionViewInput {
                available: true,
                base_x: 3200,
                base_z: 3200,
                level: 0,
                width: 2,
                height: 2,
                flags: &flags,
            }),
            ..NativeFactsInput::default()
        };
        let input = empty_input(1);
        let (keyframe, fp) = encode_snapshot_delta_with_native(None, &input, native, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert!(kf.has_collision());
        let c = kf.collision().expect("collision");
        assert!(c.available());
        assert_eq!(
            c.flags()
                .expect("collision flags")
                .iter()
                .collect::<Vec<_>>(),
            flags
        );
        let (delta, fp2) = encode_snapshot_delta_with_native(Some(&fp), &input, native, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(!view.has_collision(), "unchanged collision omitted");
        assert!(
            std::sync::Arc::ptr_eq(&fp.collision.flags, &fp2.collision.flags),
            "unchanged tick reuses the last flags Arc"
        );

        native.collision = Some(CollisionViewInput::UNAVAILABLE);
        let (cleared, _) = encode_snapshot_delta_with_native(Some(&fp2), &input, native, false);
        let view = decode_snapshot(&cleared).expect("cleared");
        assert!(view.has_collision(), "available→false must post");
        let c = view.collision().expect("cleared");
        assert!(!c.available());
        assert!(c.flags().expect("collision flags").is_empty());
    }

    #[test]
    fn first_post_without_collision_table_omits_field() {
        let input = empty_input(1);
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot");
        assert!(!view.has_collision());
    }

    fn board_row(ops: &[String], id: i32, slot: i32, component_id: i32) -> ItemRowInput<'_> {
        ItemRowInput {
            name: Some("Piece"),
            count: 1,
            id,
            ops,
            noted: false,
            cert: -1,
            component_id,
            slot,
        }
    }

    fn board_native<'a>(
        rows: &'a [ItemRowInput<'a>],
        size: i32,
        generation: u64,
    ) -> NativeFactsInput<'a> {
        NativeFactsInput {
            puzzle_board: Some(PuzzleBoardInput {
                component_id: 6600,
                size,
                items: rows,
                generation,
            }),
            ..NativeFactsInput::default()
        }
    }

    #[test]
    fn omitted_puzzle_board_is_absent_on_an_old_buffer() {
        // A buffer written before the board slots existed (append-only
        // schema): both slots are absent and the generation reads 0.
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let mut snapshot = SnapshotBuilder::new(&mut b);
        snapshot.add_tick(7);
        let root = snapshot.finish();
        b.finish(root, None);
        let view = Snapshot::from_bytes(b.finished_data()).expect("old snapshot");
        assert!(!view.has_puzzle_board());
        assert!(view.puzzle_board().is_none());
        assert!(!view.has_puzzle_board_generation());
        assert_eq!(view.puzzle_board_generation(), 0);
    }

    #[test]
    fn keyframe_without_a_board_posts_neither_slot() {
        let bytes = encode_snapshot(&empty_input(1));
        let view = decode_snapshot(&bytes).expect("keyframe");
        assert!(!view.has_puzzle_board());
        assert!(!view.has_puzzle_board_generation());
    }

    #[test]
    fn present_puzzle_board_writes_identity_size_and_rows_in_one_table() {
        let ops = vec!["Take".to_string()];
        let rows = [
            board_row(&ops, 2201, 3, 6600),
            board_row(&[], 2202, 4, 6600),
        ];
        let native = board_native(&rows, 25, 4);
        let (bytes, _) = encode_snapshot_delta_with_native(None, &empty_input(1), native, false);
        let view = decode_snapshot(&bytes).expect("keyframe");
        let board = view.puzzle_board().expect("present board");
        assert_eq!(board.component_id(), 6600);
        assert_eq!(board.size(), 25);
        let items = board.items().expect("board items");
        assert_eq!(items.len(), 2, "one row per stored slot, no gap rows");
        let first = items.get(0);
        assert_eq!(first.id(), 2201);
        assert_eq!(first.slot(), 3);
        assert_eq!(first.component_id(), 6600);
        assert_eq!(
            first
                .ops()
                .expect("item operations")
                .iter()
                .collect::<Vec<_>>(),
            vec!["Take"]
        );
        let second = items.get(1);
        assert_eq!(second.id(), 2202);
        assert_eq!(second.slot(), 4);
        assert!(second.ops().expect("item operations").is_empty());
        assert_eq!(view.puzzle_board_generation(), 4);
    }

    #[test]
    fn wrong_size_is_posted_as_observed_not_filled() {
        // Nine slots holding two pieces: the link_obj_type length is the
        // observation. Not items.len(), and never filled to a panel size.
        let rows = [board_row(&[], 2201, 0, 6600), board_row(&[], 2202, 7, 6600)];
        let native = board_native(&rows, 9, 1);
        let (bytes, _) = encode_snapshot_delta_with_native(None, &empty_input(1), native, false);
        let view = decode_snapshot(&bytes).expect("keyframe");
        let board = view.puzzle_board().expect("present board");
        assert_eq!(board.size(), 9);
        let items = board.items().expect("board items");
        assert_eq!(items.len(), 2, "rows are the stored slots only");
        assert_eq!(items.get(1).slot(), 7, "the empty slot 6 is not a row");
    }

    #[test]
    fn closed_puzzle_board_is_a_present_empty_observation() {
        let native = NativeFactsInput {
            puzzle_board: Some(PuzzleBoardInput {
                component_id: -1,
                size: 0,
                items: &[],
                generation: 9,
            }),
            ..NativeFactsInput::default()
        };
        let (bytes, _) = encode_snapshot_delta_with_native(None, &empty_input(1), native, false);
        let view = decode_snapshot(&bytes).expect("keyframe");
        let board = view.puzzle_board().expect("a close is a present object");
        assert_eq!(board.component_id(), -1);
        assert_eq!(board.size(), 0);
        assert!(board.items().expect("board items").is_empty());
        assert_eq!(view.puzzle_board_generation(), 9);
    }

    #[test]
    fn unchanged_puzzle_board_omits_both_slots() {
        let rows = [board_row(&[], 2201, 0, 6600)];
        let native = board_native(&rows, 25, 1);
        let (keyframe, fp) =
            encode_snapshot_delta_with_native(None, &empty_input(1), native, false);
        assert!(decode_snapshot(&keyframe)
            .expect("keyframe")
            .has_puzzle_board());
        let (delta, _) =
            encode_snapshot_delta_with_native(Some(&fp), &empty_input(2), native, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(!view.has_puzzle_board(), "unchanged board omitted (keep)");
        assert!(
            !view.has_puzzle_board_generation(),
            "the generation never posts without the table"
        );
    }

    #[test]
    fn board_or_generation_change_posts_both_slots_in_one_buffer() {
        let rows = [board_row(&[], 2201, 0, 6600)];
        let base = board_native(&rows, 25, 1);
        let (_, fp) = encode_snapshot_delta_with_native(None, &empty_input(1), base, false);

        // A piece move: the rows change, the session generation holds.
        let moved = [board_row(&[], 2202, 1, 6600)];
        let moved_native = board_native(&moved, 25, 1);
        let (delta, fp2) =
            encode_snapshot_delta_with_native(Some(&fp), &empty_input(2), moved_native, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_puzzle_board());
        assert!(
            view.has_puzzle_board_generation(),
            "a row change co-posts the generation"
        );
        assert_eq!(view.puzzle_board_generation(), 1);
        assert_eq!(
            view.puzzle_board()
                .expect("board")
                .items()
                .expect("board items")
                .get(0)
                .id(),
            2202
        );

        // A session edge: only the generation moves.
        let bumped = board_native(&moved, 25, 2);
        let (delta, _) =
            encode_snapshot_delta_with_native(Some(&fp2), &empty_input(3), bumped, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(
            view.has_puzzle_board(),
            "a generation-only change still carries the table"
        );
        assert_eq!(view.puzzle_board_generation(), 2);
        assert_eq!(view.puzzle_board().expect("board").component_id(), 6600);
    }

    #[test]
    fn generation_alone_flips_the_board_delta_bit() {
        // The fingerprint carries both halves, so a post that changes only
        // the session identity can never be mistaken for an unchanged board.
        let rows = [board_row(&[], 2201, 0, 6600)];
        let (_, fp) = encode_snapshot_delta_with_native(
            None,
            &empty_input(1),
            board_native(&rows, 25, 1),
            false,
        );
        let same = SnapshotFingerprint::from_input_with_native(
            &empty_input(2),
            board_native(&rows, 25, 1),
        );
        let bumped = SnapshotFingerprint::from_input_with_native(
            &empty_input(2),
            board_native(&rows, 25, 2),
        );
        assert!(
            !DeltaMask::changed(&fp, &same, false).puzzle_board,
            "an unchanged board posts nothing"
        );
        assert!(
            DeltaMask::changed(&fp, &bumped, false).puzzle_board,
            "a session bump alone re-posts the whole family"
        );
        assert_eq!(bumped.puzzle_board.as_ref().expect("board").generation, 2);
    }

    fn carry_native<'a>(rows: &'a [CarryInput<'a>], seq: u64) -> NativeFactsInput<'a> {
        NativeFactsInput {
            walk_outcome_seq: seq,
            walk_missing_carry: rows,
            ..NativeFactsInput::default()
        }
    }

    #[test]
    fn unknown_walk_cancel_reason_is_rejected() {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let root = {
            let mut snapshot = SnapshotBuilder::new(&mut b);
            snapshot.add_tick(1);
            snapshot.add_walk_outcome_cancel_reason(WalkCancelReason(2));
            snapshot.finish()
        };
        b.finish(root, None);
        let error = match Snapshot::from_bytes(b.finished_data()) {
            Err(error) => error,
            Ok(_) => panic!("unknown bounded enum was accepted"),
        };
        assert!(error.contains("unknown walk outcome cancellation reason"));
    }

    #[test]
    fn omitted_walk_missing_carry_is_absent_on_an_old_buffer() {
        // A buffer written before slot 254 existed (append-only schema): the
        // slot is absent and the reader reports nothing rather than a clear.
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let root = {
            let mut snapshot = SnapshotBuilder::new(&mut b);
            snapshot.add_tick(7);
            snapshot.add_walk_outcome_seq(3);
            snapshot.finish()
        };
        b.finish(root, None);
        let view = Snapshot::from_bytes(b.finished_data()).expect("old snapshot");
        assert!(view.has_walk_outcome_seq());
        assert!(!view.has_user_move_intent_seq());
        assert!(!view.has_walk_outcome_cancel_reason());
        assert_eq!(view.walk_outcome_cancel_reason(), WalkCancelReason::None);
        assert!(!view.has_walk_missing_carry());
        assert!(view.walk_missing_carry().is_none());
    }

    #[test]
    fn walk_outcome_family_posts_named_shorts_with_and_without_a_name() {
        // The name is corroboration: a row the host obj table has no name for
        // still posts its id and count.
        let rows = [
            CarryInput {
                id: 1854,
                count: 1,
                name: Some("Shantay pass"),
            },
            CarryInput {
                id: 995,
                count: 10,
                name: None,
            },
        ];
        let (bytes, _) =
            encode_snapshot_delta_with_native(None, &empty_input(1), carry_native(&rows, 1), false);
        let view = decode_snapshot(&bytes).expect("keyframe");
        assert!(view.has_walk_missing_carry());
        let posted = view.walk_missing_carry().expect("missing carry rows");
        assert_eq!(posted.len(), 2, "posted order is the host's");
        let first = posted.get(0);
        assert_eq!(first.id(), 1854);
        assert_eq!(first.count(), 1);
        assert_eq!(first.name(), Some("Shantay pass"));
        let second = posted.get(1);
        assert_eq!(second.id(), 995);
        assert_eq!(second.count(), 10);
        assert_eq!(second.name(), None);
    }

    #[test]
    fn a_routed_outcome_posts_an_empty_vector_and_never_omits_the_clear() {
        let rows = [CarryInput {
            id: 1854,
            count: 1,
            name: Some("Shantay pass"),
        }];
        let (keyframe, fp) =
            encode_snapshot_delta_with_native(None, &empty_input(1), carry_native(&rows, 1), false);
        assert_eq!(
            decode_snapshot(&keyframe)
                .expect("keyframe")
                .walk_missing_carry()
                .expect("missing carry rows")
                .len(),
            1
        );
        // The next outcome is a route: seq bumped, no named short. The family
        // posts a PRESENT empty vector — the clear rides it, never omitted.
        let (delta, _) = encode_snapshot_delta_with_native(
            Some(&fp),
            &empty_input(2),
            carry_native(&[], 2),
            false,
        );
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_walk_outcome_seq(), "the family re-posts");
        assert!(view.has_walk_missing_carry(), "the clear is never omitted");
        assert!(view
            .walk_missing_carry()
            .expect("cleared missing carry rows")
            .is_empty());
    }

    #[test]
    fn unchanged_walk_outcome_family_omits_the_vector_too() {
        // A keep: the vector rides the family, so an unchanged family posts
        // neither the scalars nor a vector a page could read as a fresh clear.
        let rows = [CarryInput {
            id: 1854,
            count: 1,
            name: None,
        }];
        let native = carry_native(&rows, 4);
        let (keyframe, fp) =
            encode_snapshot_delta_with_native(None, &empty_input(1), native, false);
        assert!(decode_snapshot(&keyframe)
            .expect("keyframe")
            .has_walk_missing_carry());
        let (delta, _) =
            encode_snapshot_delta_with_native(Some(&fp), &empty_input(2), native, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(!view.has_walk_outcome_seq());
        assert!(!view.has_walk_missing_carry());
    }

    #[test]
    fn a_shopping_list_change_alone_flips_the_walk_outcome_family() {
        // The vector rides the family, so a post that moved only the named
        // shorts still carries it: an outcome that failed for a new reason is
        // not an unchanged outcome.
        let rows = [CarryInput {
            id: 1854,
            count: 1,
            name: None,
        }];
        let (_, fp) =
            encode_snapshot_delta_with_native(None, &empty_input(1), carry_native(&rows, 1), false);
        let same =
            SnapshotFingerprint::from_input_with_native(&empty_input(2), carry_native(&rows, 1));
        let other =
            SnapshotFingerprint::from_input_with_native(&empty_input(2), carry_native(&[], 1));
        assert!(!DeltaMask::changed(&fp, &same, false).walk_outcome);
        assert!(
            DeltaMask::changed(&fp, &other, false).walk_outcome,
            "the family bit covers the named shorts"
        );
    }

    #[test]
    fn a_blocked_route_end_rides_the_walk_outcome_family() {
        let blocked = NativeFactsInput {
            walk_outcome_blocked: true,
            ..carry_native(&[], 3)
        };
        let (keyframe, fp) =
            encode_snapshot_delta_with_native(None, &empty_input(1), blocked, false);
        assert!(decode_snapshot(&keyframe)
            .expect("keyframe")
            .walk_outcome_blocked());
        let (delta, _) = encode_snapshot_delta_with_native(
            Some(&fp),
            &empty_input(2),
            carry_native(&[], 3),
            false,
        );
        let view = decode_snapshot(&delta).expect("delta");
        assert!(
            view.has_walk_outcome_seq(),
            "the flag alone re-posts the family"
        );
        assert!(!view.walk_outcome_blocked());
    }

    include!("isolate_fb_schema_roundtrip.rs");
}

//! FlatBuffers wire format for isolate IPC — schema: `crates/script/
//! schema/isolate.fbs`. The builder and reader are hand-written against
//! that schema (operators never need `flatc` at `cargo test` time); keep
//! the two in sync. The PLAYER_INFO snapshot posted into each JS isolate
//! and the shim interact / paint frames forwarded back are FlatBuffers,
//! not JSON: a 50+ isolate wall never stringifies or parses a JSON
//! document per tick. Each slot's host encode path and each V8 isolate
//! thread reuse one [`IsolateBuf`] (`reset`, not a fresh builder).
//!
//! The wire format is produced and consumed only by 274bot code. Host-encoded
//! snapshots are trusted; isolate→host interact/paint bytes are verified on
//! decode (`flatbuffers::root_with_opts`) so truncated or malicious buffers
//! fail closed instead of panicking or reading out of bounds.
//!
//! Posts are deltas (schema: `Snapshot`): `tick` is always carried, other
//! fields only when they changed vs the last post — an omitted vector is
//! absent, never empty, and the isolate keeps its last JS value for it.
//! The per-slot last-post [`SnapshotFingerprint`] is compared by value
//! (equality, not a hash) once per slot per tick.

use flatbuffers::{
    root_with_opts, FlatBufferBuilder, Follow, ForwardsUOffset, InvalidFlatbuffer, Table, VOffsetT,
    Vector, Verifiable, Verifier, VerifierOptions, WIPOffset,
};

/// Max shim interact rows per tick (isolate→host).
const MAX_INTERACT_REQS: usize = 256;
/// Max paint lines per frame (isolate→host).
const MAX_PAINT_LINES: usize = 512;
/// Max advertised paint buttons per frame (isolate→host).
const MAX_PAINT_BUTTONS: usize = 32;
/// Max canvas ops per paint frame (isolate→host).
const MAX_CANVAS_OPS: usize = crate::canvas::MAX_CANVAS_OPS;
/// Max UTF-8 bytes per canvas fillText string.
const MAX_PAINT_TEXT: usize = crate::canvas::MAX_PAINT_TEXT;
const MAX_PATH_SEGS_PER_OP: usize = crate::canvas::MAX_PATH_SEGS_PER_OP;
const MAX_PATH_SEGS_PER_FRAME: usize = crate::canvas::MAX_PATH_SEGS_PER_FRAME;
const MAX_GRADIENT_STOPS: usize = crate::canvas::MAX_GRADIENT_STOPS;
const MAX_CLIP_PATHS: usize = crate::canvas::MAX_CLIP_PATHS;
const MAX_LINE_WIDTH: f32 = crate::canvas::MAX_LINE_WIDTH;
const MAX_SHADOW_BLUR: f32 = crate::canvas::MAX_SHADOW_BLUR;
const MAX_BUYOUT_STOCK: usize = 256;
const MAX_BUYOUT_CHOSEN: usize = 256;
const MAX_BUYOUT_ITEMS: usize = 256;

fn isolate_verify_opts() -> VerifierOptions {
    VerifierOptions {
        max_depth: 64,
        max_tables: 10_000,
        max_apparent_size: 16 * 1024 * 1024,
        ignore_missing_null_terminator: false,
    }
}

fn verified_root<'buf, T>(buf: &'buf [u8]) -> Result<T::Inner, String>
where
    T: 'buf + Follow<'buf> + Verifiable,
{
    root_with_opts::<T>(&isolate_verify_opts(), buf).map_err(|e: InvalidFlatbuffer| e.to_string())
}

// Field slot offsets are vtable byte offsets: field id N sits at
// `(N + 2) * SIZE_VOFFSET` (`SIZE_VOFFSET = 2`), so id 0 -> 4, 1 -> 6, ...

// Tile / Booth: { x: int, z: int, level: int }
const VT_TILE_X: VOffsetT = 4;
const VT_TILE_Z: VOffsetT = 6;
const VT_TILE_LEVEL: VOffsetT = 8;

// Row: { name: string, count: int, id, ops, noted, cert, component_id, slot }
const VT_ROW_NAME: VOffsetT = 4;
const VT_ROW_COUNT: VOffsetT = 6;
const VT_ROW_ID: VOffsetT = 8;
const VT_ROW_OPS: VOffsetT = 10;
const VT_ROW_NOTED: VOffsetT = 12;
const VT_ROW_CERT: VOffsetT = 14;
const VT_ROW_COMPONENT: VOffsetT = 16;
const VT_ROW_SLOT: VOffsetT = 18;

// Stat: { index, name, xp, base, effective }
const VT_STAT_INDEX: VOffsetT = 4;
const VT_STAT_NAME: VOffsetT = 6;
const VT_STAT_XP: VOffsetT = 8;
const VT_STAT_BASE: VOffsetT = 10;
const VT_STAT_EFFECTIVE: VOffsetT = 12;

// BankStand: { name, x, z, level, kind, op, choose }
const VT_BANK_NAME: VOffsetT = 4;
const VT_BANK_X: VOffsetT = 6;
const VT_BANK_Z: VOffsetT = 8;
const VT_BANK_LEVEL: VOffsetT = 10;
const VT_BANK_KIND: VOffsetT = 12;
const VT_BANK_OP: VOffsetT = 14;
const VT_BANK_CHOOSE: VOffsetT = 16;

// NearestBooth: { x, z, level, name, op }
const VT_NEAREST_X: VOffsetT = 4;
const VT_NEAREST_Z: VOffsetT = 6;
const VT_NEAREST_LEVEL: VOffsetT = 8;
const VT_NEAREST_NAME: VOffsetT = 10;
const VT_NEAREST_OP: VOffsetT = 12;
const VT_NEAREST_ID: VOffsetT = 14;

// Snapshot: { tick, here, ingame, inv, inv_size, stats, booths, nearest_booth,
//             bank, bank_side, bank_open, bank_loaded, hold, ours }
const VT_SNAP_TICK: VOffsetT = 4;
const VT_SNAP_HERE: VOffsetT = 6;
const VT_SNAP_INGAME: VOffsetT = 8;
const VT_SNAP_INV: VOffsetT = 10;
const VT_SNAP_INV_SIZE: VOffsetT = 12;
const VT_SNAP_STATS: VOffsetT = 14;
const VT_SNAP_BOOTHS: VOffsetT = 16;
const VT_SNAP_BANKS: VOffsetT = 18;
const VT_SNAP_BANK: VOffsetT = 20;
const VT_SNAP_BANK_SIDE: VOffsetT = 22;
const VT_SNAP_BANK_OPEN: VOffsetT = 24;
const VT_SNAP_BANK_LOADED: VOffsetT = 26;
const VT_SNAP_HOLD: VOffsetT = 28;
const VT_SNAP_OURS: VOffsetT = 30;
const VT_SNAP_NPCS: VOffsetT = 32;
const VT_SNAP_LOCS: VOffsetT = 34;
const VT_SNAP_PLAYERS: VOffsetT = 36;
const VT_SNAP_GROUND: VOffsetT = 38;
const VT_SNAP_EQUIPMENT: VOffsetT = 40;
const VT_SNAP_CHAT_OPEN: VOffsetT = 42;
const VT_SNAP_CHAT_CONTINUE: VOffsetT = 44;
const VT_SNAP_CHAT_TEXT: VOffsetT = 46;
const VT_SNAP_CHAT_OPTIONS: VOffsetT = 48;
const VT_SNAP_SIDE_TAB: VOffsetT = 50;
const VT_SNAP_VARPS: VOffsetT = 52;
const VT_SNAP_COMBAT_STYLES: VOffsetT = 54;
const VT_SNAP_RUN_ENERGY: VOffsetT = 56;
const VT_SNAP_RUN_ENABLED: VOffsetT = 58;
const VT_SNAP_RETALIATE: VOffsetT = 60;
const VT_SNAP_MY_NAME: VOffsetT = 62;
const VT_SNAP_IN_COMBAT: VOffsetT = 64;
const VT_SNAP_ANIMATING: VOffsetT = 66;
const VT_SNAP_MAIN_MODAL: VOffsetT = 68;
const VT_SNAP_CHAT_MODAL: VOffsetT = 70;
const VT_SNAP_MAKE_PRODUCTS: VOffsetT = 72;
const VT_SNAP_SIDE_TAB_IFACES: VOffsetT = 74;
const VT_SNAP_SPELL_BUTTONS: VOffsetT = 76;
const VT_SNAP_CHAT_LINES: VOffsetT = 78;
const VT_SNAP_NEAREST_BOOTH: VOffsetT = 80;
const VT_SNAP_BANK_NOTE_ON: VOffsetT = 82;
const VT_SNAP_BANK_NOTE_OFF: VOffsetT = 84;
const VT_SNAP_SCENE_STATE: VOffsetT = 86;
const VT_SNAP_WEIGHT: VOffsetT = 88;
const VT_SNAP_CAMERA_YAW: VOffsetT = 90;
const VT_SNAP_CAMERA_PITCH: VOffsetT = 92;
const VT_SNAP_TELEPORTS_ENABLED: VOffsetT = 94;
const VT_SNAP_SELF_SLOT: VOffsetT = 96;
const VT_SNAP_TRADE_OFFER_OPEN: VOffsetT = 98;
const VT_SNAP_TRADE_CONFIRM_OPEN: VOffsetT = 100;
const VT_SNAP_TRADE_PARTNER: VOffsetT = 102;
const VT_SNAP_TRADE_MINE: VOffsetT = 104;
const VT_SNAP_TRADE_THEIRS: VOffsetT = 106;
const VT_SNAP_TRADE_SIDE: VOffsetT = 108;
const VT_SNAP_TRADE_ACCEPT_ID: VOffsetT = 110;
const VT_SNAP_TRADE_DECLINE_ID: VOffsetT = 112;
const VT_SNAP_SHOP_OPEN: VOffsetT = 114;
const VT_SNAP_SHOP_STOCK: VOffsetT = 116;
const VT_SNAP_BANK_GENERATION: VOffsetT = 118;
const VT_SNAP_COUNT_DIALOG_OPEN: VOffsetT = 120;
const VT_SNAP_WITHDRAW_X_RESULT_SEQ: VOffsetT = 122;
const VT_SNAP_WITHDRAW_X_RESULT: VOffsetT = 124;
const VT_SNAP_WITHDRAW_LOAD_RESULT_SEQ: VOffsetT = 126;
const VT_SNAP_WITHDRAW_LOAD_RESULT: VOffsetT = 128;
const VT_SNAP_BANK_OP_RESULT_SEQ: VOffsetT = 130;
const VT_SNAP_BANK_OP_RESULT: VOffsetT = 132;
const VT_SNAP_REACH: VOffsetT = 134;
const VT_SNAP_ATTACKED_BY_PLAYER: VOffsetT = 136;
const VT_SNAP_WIDGETS: VOffsetT = 138;
const VT_SNAP_SELF_CHAT: VOffsetT = 140;
const VT_SNAP_HINT_TILE_X: VOffsetT = 142;
const VT_SNAP_HINT_TILE_Z: VOffsetT = 144;
const VT_SNAP_RETALIATE_ON_COM_ID: VOffsetT = 146;
const VT_SNAP_RETALIATE_OFF_COM_ID: VOffsetT = 148;
const VT_SNAP_QUEST_STATUSES: VOffsetT = 150;
const VT_SNAP_QUEST_STATUSES_AVAILABLE: VOffsetT = 152;
const VT_SNAP_NPC_BOXES: VOffsetT = 154;
const VT_SNAP_NPC_BOXES_AVAILABLE: VOffsetT = 156;
const VT_SNAP_SHOP_PLAYER: VOffsetT = 158;
const VT_SNAP_SHOP_PLAYER_AVAILABLE: VOffsetT = 160;
const VT_SNAP_MAIN_MAKE: VOffsetT = 162;
const VT_SNAP_MAIN_MAKE_AVAILABLE: VOffsetT = 164;
const VT_SNAP_BANK_APPROACHES: VOffsetT = 166;
const VT_SNAP_WALK_OUTCOME_SEQ: VOffsetT = 168;
const VT_SNAP_WALK_OUTCOME_GENERATION: VOffsetT = 170;
const VT_SNAP_WALK_OUTCOME_FAILED: VOffsetT = 172;
const VT_SNAP_WALK_OUTCOME_X: VOffsetT = 174;
const VT_SNAP_WALK_OUTCOME_Z: VOffsetT = 176;
const VT_SNAP_WALK_OUTCOME_LEVEL: VOffsetT = 178;
const VT_SNAP_WALK_OUTCOME_RADIUS: VOffsetT = 180;
const VT_SNAP_WALK_OUTCOME_ALLOW_TELEPORTS: VOffsetT = 182;
const VT_SNAP_WALK_OUTCOME_REQUEST_ID: VOffsetT = 184;
const VT_SNAP_CANVAS_WIDTH: VOffsetT = 186;
const VT_SNAP_CANVAS_HEIGHT: VOffsetT = 188;

/// Logical applet posted as `canvasRect`. Bound to `api::native_input::APPLET_*`.
pub const SNAPSHOT_CANVAS_W: i32 = api::native_input::APPLET_W;
pub const SNAPSHOT_CANVAS_H: i32 = api::native_input::APPLET_H;

// BankApproach: { loc_id, x, z, level, can_operate, dest_ok, dest_x, dest_z, dest_level }
const VT_BA_LOC_ID: VOffsetT = 4;
const VT_BA_X: VOffsetT = 6;
const VT_BA_Z: VOffsetT = 8;
const VT_BA_LEVEL: VOffsetT = 10;
const VT_BA_CAN_OPERATE: VOffsetT = 12;
const VT_BA_DEST_OK: VOffsetT = 14;
const VT_BA_DEST_X: VOffsetT = 16;
const VT_BA_DEST_Z: VOffsetT = 18;
const VT_BA_DEST_LEVEL: VOffsetT = 20;

// WidgetText: { component_id, text }
const VT_WT_COMPONENT: VOffsetT = 4;
const VT_WT_TEXT: VOffsetT = 6;

// QuestStatus: { name, status }
const VT_QUEST_NAME: VOffsetT = 4;
const VT_QUEST_STATUS: VOffsetT = 6;

// NpcBox: { index, points }
const VT_NPC_BOX_INDEX: VOffsetT = 4;
const VT_NPC_BOX_POINTS: VOffsetT = 6;

// Reach: { available, base_x, base_z, level, width, height, walkable,
//          reachable, reachable_adj, step, exact_rank, adjacent_rank }
const VT_REACH_AVAILABLE: VOffsetT = 4;
const VT_REACH_BASE_X: VOffsetT = 6;
const VT_REACH_BASE_Z: VOffsetT = 8;
const VT_REACH_LEVEL: VOffsetT = 10;
const VT_REACH_WIDTH: VOffsetT = 12;
const VT_REACH_HEIGHT: VOffsetT = 14;
const VT_REACH_WALKABLE: VOffsetT = 16;
const VT_REACH_REACHABLE: VOffsetT = 18;
const VT_REACH_REACHABLE_ADJ: VOffsetT = 20;
const VT_REACH_STEP: VOffsetT = 22;
const VT_REACH_EXACT_RANK: VOffsetT = 24;
const VT_REACH_ADJACENT_RANK: VOffsetT = 26;
const VT_REACH_CANLIGHT: VOffsetT = 28;

// SideTabIface: { index, id }
const VT_STI_INDEX: VOffsetT = 4;
const VT_STI_ID: VOffsetT = 6;

// ChatLine: { seq, text, type, username }
const VT_CL_SEQ: VOffsetT = 4;
const VT_CL_TEXT: VOffsetT = 6;
const VT_CL_TYPE: VOffsetT = 8;
const VT_CL_USERNAME: VOffsetT = 10;

// SceneEntity: { index, id, name, x, z, level, distance, health,
//               max_health, in_combat, animating, actions }
const VT_ENT_INDEX: VOffsetT = 4;
const VT_ENT_ID: VOffsetT = 6;
const VT_ENT_NAME: VOffsetT = 8;
const VT_ENT_X: VOffsetT = 10;
const VT_ENT_Z: VOffsetT = 12;
const VT_ENT_LEVEL: VOffsetT = 14;
const VT_ENT_DISTANCE: VOffsetT = 16;
const VT_ENT_HEALTH: VOffsetT = 18;
const VT_ENT_MAX_HEALTH: VOffsetT = 20;
const VT_ENT_IN_COMBAT: VOffsetT = 22;
const VT_ENT_ANIMATING: VOffsetT = 24;
const VT_ENT_ACTIONS: VOffsetT = 26;
const VT_ENT_REACHABLE: VOffsetT = 28;
const VT_ENT_REACHABLE_ADJ: VOffsetT = 30;
const VT_ENT_COMBAT_LEVEL: VOffsetT = 32;
const VT_ENT_TARGET_KIND: VOffsetT = 34;
const VT_ENT_TARGET_INDEX: VOffsetT = 36;

// ChatOption: { text }
const VT_CHAT_OPT_TEXT: VOffsetT = 4;

// MakeButton: { qty, com_id }
const VT_MAKE_BTN_QTY: VOffsetT = 4;
const VT_MAKE_BTN_COM: VOffsetT = 6;

// MakeProduct: { object_id, name, buttons }
const VT_MAKE_PROD_OID: VOffsetT = 4;
const VT_MAKE_PROD_NAME: VOffsetT = 6;
const VT_MAKE_PROD_BTNS: VOffsetT = 8;

// CombatStyle: { mode, label, component_id }
const VT_CS_MODE: VOffsetT = 4;
const VT_CS_LABEL: VOffsetT = 6;
const VT_CS_COMPONENT: VOffsetT = 8;

// Varp: { index, value }
const VT_VARP_INDEX: VOffsetT = 4;
const VT_VARP_VALUE: VOffsetT = 6;

// Interact: { op, x, z, level, kind, name, stand_op, choose, action,
//             index, component_id, bank_generation, bank_item_id, lands_as_id,
//             source_item_id, source_item_slot, target_item_id, target_item_slot,
//             request_id, xf, yf, input_identity, allow_wilderness, allow_bank_fetch }
const VT_IN_OP: VOffsetT = 4;
const VT_IN_X: VOffsetT = 6;
const VT_IN_Z: VOffsetT = 8;
const VT_IN_LEVEL: VOffsetT = 10;
const VT_IN_KIND: VOffsetT = 12;
const VT_IN_NAME: VOffsetT = 14;
const VT_IN_STAND_OP: VOffsetT = 16;
const VT_IN_CHOOSE: VOffsetT = 18;
const VT_IN_ACTION: VOffsetT = 20;
const VT_IN_INDEX: VOffsetT = 22;
const VT_IN_COMPONENT_ID: VOffsetT = 24;
const VT_IN_BANK_GENERATION: VOffsetT = 26;
const VT_IN_BANK_ITEM_ID: VOffsetT = 28;
const VT_IN_LANDS_AS_ID: VOffsetT = 30;
const VT_IN_SOURCE_ITEM_ID: VOffsetT = 32;
const VT_IN_SOURCE_ITEM_SLOT: VOffsetT = 34;
const VT_IN_TARGET_ITEM_ID: VOffsetT = 36;
const VT_IN_TARGET_ITEM_SLOT: VOffsetT = 38;
const VT_IN_REQUEST_ID: VOffsetT = 40;
const VT_IN_XF: VOffsetT = 42;
const VT_IN_YF: VOffsetT = 44;
const VT_IN_INPUT_IDENTITY: VOffsetT = 46;
const VT_IN_ALLOW_WILDERNESS: VOffsetT = 48;
const VT_IN_ALLOW_BANK_FETCH: VOffsetT = 50;

// InteractBatch: { reqs: [Interact] }
const VT_REQS: VOffsetT = 4;

// PaintButton: { id: string, label: string }
const VT_PAINT_BTN_ID: VOffsetT = 4;
const VT_PAINT_BTN_LABEL: VOffsetT = 6;

// Paint: { title, accent, lines, buttons, canvas }
const VT_PAINT_TITLE: VOffsetT = 4;
const VT_PAINT_ACCENT: VOffsetT = 6;
const VT_PAINT_LINES: VOffsetT = 8;
const VT_PAINT_BUTTONS: VOffsetT = 10;
const VT_PAINT_CANVAS: VOffsetT = 12;

// CanvasOp: { kind, x, y, w, h, color, text, font_px, mono, segs, clips, stops, ... }
const VT_CANVAS_KIND: VOffsetT = 4;
const VT_CANVAS_X: VOffsetT = 6;
const VT_CANVAS_Y: VOffsetT = 8;
const VT_CANVAS_W: VOffsetT = 10;
const VT_CANVAS_H: VOffsetT = 12;
const VT_CANVAS_COLOR: VOffsetT = 14;
const VT_CANVAS_TEXT: VOffsetT = 16;
const VT_CANVAS_FONT_PX: VOffsetT = 18;
const VT_CANVAS_MONO: VOffsetT = 20;
const VT_CANVAS_SEGS: VOffsetT = 22;
const VT_CANVAS_CLIPS: VOffsetT = 24;
const VT_CANVAS_STOPS: VOffsetT = 26;
const VT_CANVAS_GX0: VOffsetT = 28;
const VT_CANVAS_GY0: VOffsetT = 30;
const VT_CANVAS_GX1: VOffsetT = 32;
const VT_CANVAS_GY1: VOffsetT = 34;
const VT_CANVAS_R0: VOffsetT = 36;
const VT_CANVAS_R1: VOffsetT = 38;
const VT_CANVAS_GRAD_KIND: VOffsetT = 40;
const VT_CANVAS_LINE_WIDTH: VOffsetT = 42;
const VT_CANVAS_LINE_JOIN: VOffsetT = 44;
const VT_CANVAS_SHADOW_COLOR: VOffsetT = 46;
const VT_CANVAS_SHADOW_BLUR: VOffsetT = 48;
const VT_CANVAS_SHADOW_X: VOffsetT = 50;
const VT_CANVAS_SHADOW_Y: VOffsetT = 52;
const VT_CANVAS_ALIGN: VOffsetT = 54;
const VT_CANVAS_BASELINE: VOffsetT = 56;

const VT_SEG_KIND: VOffsetT = 4;
const VT_SEG_X: VOffsetT = 6;
const VT_SEG_Y: VOffsetT = 8;
const VT_SEG_C1X: VOffsetT = 10;
const VT_SEG_C1Y: VOffsetT = 12;
const VT_SEG_C2X: VOffsetT = 14;
const VT_SEG_C2Y: VOffsetT = 16;

const VT_GSTOP_OFFSET: VOffsetT = 4;
const VT_GSTOP_COLOR: VOffsetT = 6;

const VT_CLIP_SEGS: VOffsetT = 4;

// BuyoutStock: { obj, count }
const VT_BUYOUT_STOCK_OBJ: VOffsetT = 4;
const VT_BUYOUT_STOCK_COUNT: VOffsetT = 6;

// BuyoutPlanRequest: { inv, keeper, coins, stock, chosen }
const VT_BUYOUT_REQ_INV: VOffsetT = 4;
const VT_BUYOUT_REQ_KEEPER: VOffsetT = 6;
const VT_BUYOUT_REQ_COINS: VOffsetT = 8;
const VT_BUYOUT_REQ_STOCK: VOffsetT = 10;
const VT_BUYOUT_REQ_CHOSEN: VOffsetT = 12;

// BuyoutPlanItem: { obj, name, units, est_cost }
const VT_BUYOUT_ITEM_OBJ: VOffsetT = 4;
const VT_BUYOUT_ITEM_NAME: VOffsetT = 6;
const VT_BUYOUT_ITEM_UNITS: VOffsetT = 8;
const VT_BUYOUT_ITEM_EST_COST: VOffsetT = 10;

// BuyoutPlanResult: { ok, reason, items }
const VT_BUYOUT_RES_OK: VOffsetT = 4;
const VT_BUYOUT_RES_REASON: VOffsetT = 6;
const VT_BUYOUT_RES_ITEMS: VOffsetT = 8;

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
}

/// One chat modal BUTTON_OK choice.
#[derive(Clone, Copy)]
pub struct ChatOptionInput<'a> {
    pub text: &'a str,
}

/// One inv/bank/equipment row posted from `ItemView`.
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

/// The snapshot fields the host observed this PLAYER_INFO — exactly the
/// set the shim Game/Inventory/Skills/Bank/Banking/EventSignal read, no
/// World clone. `inv`/`bank`/`bank_side` rows carry the resolved obj name
/// (`None` when the host table has no name for the id — a script query
/// never matches); `stats` the stat index/name/xp; `booths` the scene
/// locs with a `Use-quickly` action; `banks` the packed stands; `hold`/
/// `ours` the guardian's status for `EventSignal.pending()`.
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
    /// Selected-world widget text rows. Absent id is not a stale IfType label.
    pub widgets: &'a [WidgetTextInput<'a>],
}

/// Optional native facts appended to the isolate snapshot. Kept separate
/// from [`SnapshotInput`] so existing one-shot callers remain source-compatible.
#[derive(Clone, Copy, Default)]
pub struct NativeFactsInput<'a> {
    pub self_chat: Option<&'a str>,
    pub hint_tile: Option<(i32, i32)>,
    pub retaliate_controls: Option<(i32, i32)>,
    pub quest_statuses: Option<&'a [QuestStatusInput<'a>]>,
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
}

/// One currently posted widget text row (`reader.ifText`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WidgetTextInput<'a> {
    pub component_id: i32,
    pub text: &'a str,
}

/// A `{x, z, level}` tile as decoded from a buffer.
#[derive(Clone, Copy)]
pub struct TileReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for TileReader<'a> {
    type Inner = TileReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl TileReader<'_> {
    pub fn x(&self) -> i32 {
        // Safety: the buffer was produced by our encoder (root checked).
        unsafe { self.tab.get::<i32>(VT_TILE_X, None) }.unwrap_or(0)
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_TILE_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_TILE_LEVEL, None) }.unwrap_or(0)
    }
}

impl Verifiable for TileReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("x", VT_TILE_X, false)?
            .visit_field::<i32>("z", VT_TILE_Z, false)?
            .visit_field::<i32>("level", VT_TILE_LEVEL, false)?
            .finish();
        Ok(())
    }
}

/// Compact reach query as decoded from a buffer.
#[derive(Clone, Copy)]
pub struct ReachReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for ReachReader<'a> {
    type Inner = ReachReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl ReachReader<'_> {
    pub fn available(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_REACH_AVAILABLE, None) }.unwrap_or(false)
    }
    pub fn base_x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_REACH_BASE_X, None) }.unwrap_or(0)
    }
    pub fn base_z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_REACH_BASE_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_REACH_LEVEL, None) }.unwrap_or(0)
    }
    pub fn width(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_REACH_WIDTH, None) }.unwrap_or(0)
    }
    pub fn height(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_REACH_HEIGHT, None) }.unwrap_or(0)
    }
    pub fn walkable(&self) -> Vec<u32> {
        u32_vec(&self.tab, VT_REACH_WALKABLE)
    }
    pub fn reachable(&self) -> Vec<u32> {
        u32_vec(&self.tab, VT_REACH_REACHABLE)
    }
    pub fn reachable_adj(&self) -> Vec<u32> {
        u32_vec(&self.tab, VT_REACH_REACHABLE_ADJ)
    }
    pub fn exact_rank(&self) -> Vec<u16> {
        u16_vec(&self.tab, VT_REACH_EXACT_RANK)
    }
    pub fn adjacent_rank(&self) -> Vec<u16> {
        u16_vec(&self.tab, VT_REACH_ADJACENT_RANK)
    }
    pub fn step(&self) -> Vec<u8> {
        u8_vec(&self.tab, VT_REACH_STEP)
    }
    pub fn canlight(&self) -> Vec<u32> {
        u32_vec(&self.tab, VT_REACH_CANLIGHT)
    }
}

impl Verifiable for ReachReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<bool>("available", VT_REACH_AVAILABLE, false)?
            .visit_field::<i32>("base_x", VT_REACH_BASE_X, false)?
            .visit_field::<i32>("base_z", VT_REACH_BASE_Z, false)?
            .visit_field::<i32>("level", VT_REACH_LEVEL, false)?
            .visit_field::<i32>("width", VT_REACH_WIDTH, false)?
            .visit_field::<i32>("height", VT_REACH_HEIGHT, false)?
            .visit_field::<ForwardsUOffset<Vector<u32>>>("walkable", VT_REACH_WALKABLE, false)?
            .visit_field::<ForwardsUOffset<Vector<u32>>>("reachable", VT_REACH_REACHABLE, false)?
            .visit_field::<ForwardsUOffset<Vector<u32>>>(
                "reachable_adj",
                VT_REACH_REACHABLE_ADJ,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<u8>>>("step", VT_REACH_STEP, false)?
            .visit_field::<ForwardsUOffset<Vector<u16>>>("exact_rank", VT_REACH_EXACT_RANK, false)?
            .visit_field::<ForwardsUOffset<Vector<u16>>>(
                "adjacent_rank",
                VT_REACH_ADJACENT_RANK,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<u32>>>("canlight", VT_REACH_CANLIGHT, false)?
            .finish();
        Ok(())
    }
}

/// One inventory/bank row as decoded: the resolved obj name (`None` =
/// unknown id), count, and ItemView fields (id/ops/noted/cert).
#[derive(Clone, Copy)]
pub struct RowReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for RowReader<'a> {
    type Inner = RowReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl RowReader<'_> {
    pub fn name(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_ROW_NAME, None) }
    }
    pub fn count(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ROW_COUNT, None) }.unwrap_or(0)
    }
    pub fn id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ROW_ID, None) }.unwrap_or(0)
    }
    pub fn ops(&self) -> Vec<&str> {
        match unsafe {
            self.tab
                .get::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(VT_ROW_OPS, None)
        } {
            Some(v) => v.iter().collect(),
            None => Vec::new(),
        }
    }
    pub fn noted(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_ROW_NOTED, None) }.unwrap_or(false)
    }
    pub fn cert(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ROW_CERT, None) }.unwrap_or(-1)
    }
    pub fn has_component_id(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_ROW_COMPONENT, None).is_some() }
    }
    pub fn component_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ROW_COMPONENT, None) }.unwrap_or(-1)
    }
    pub fn has_slot(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_ROW_SLOT, None).is_some() }
    }
    pub fn slot(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ROW_SLOT, None) }.unwrap_or(-1)
    }
}

impl Verifiable for RowReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_ROW_NAME, false)?
            .visit_field::<i32>("count", VT_ROW_COUNT, false)?
            .visit_field::<i32>("id", VT_ROW_ID, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(
                "ops", VT_ROW_OPS, false,
            )?
            .visit_field::<bool>("noted", VT_ROW_NOTED, false)?
            .visit_field::<i32>("cert", VT_ROW_CERT, false)?
            .visit_field::<i32>("component_id", VT_ROW_COMPONENT, false)?
            .visit_field::<i32>("slot", VT_ROW_SLOT, false)?
            .finish();
        Ok(())
    }
}

/// One skill row as decoded: stat index, name, and xp.
#[derive(Clone, Copy)]
pub struct StatReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for StatReader<'a> {
    type Inner = StatReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl StatReader<'_> {
    pub fn index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STAT_INDEX, None) }.unwrap_or(0)
    }
    pub fn name(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_STAT_NAME, None) }.unwrap_or("")
    }
    pub fn xp(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STAT_XP, None) }.unwrap_or(0)
    }
    pub fn base(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STAT_BASE, None) }.unwrap_or(0)
    }
    pub fn effective(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STAT_EFFECTIVE, None) }.unwrap_or(0)
    }
}

impl Verifiable for StatReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_STAT_INDEX, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_STAT_NAME, false)?
            .visit_field::<i32>("xp", VT_STAT_XP, false)?
            .visit_field::<i32>("base", VT_STAT_BASE, false)?
            .visit_field::<i32>("effective", VT_STAT_EFFECTIVE, false)?
            .finish();
        Ok(())
    }
}

/// One packed bank stand as decoded.
#[derive(Clone, Copy)]
pub struct BankStandReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for BankStandReader<'a> {
    type Inner = BankStandReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl BankStandReader<'_> {
    pub fn name(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_BANK_NAME, None) }.unwrap_or("")
    }
    pub fn x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BANK_X, None) }.unwrap_or(0)
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BANK_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BANK_LEVEL, None) }.unwrap_or(0)
    }
    pub fn kind(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_BANK_KIND, None) }.unwrap_or("")
    }
    pub fn op(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BANK_OP, None) }.unwrap_or(0)
    }
    pub fn choose(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_BANK_CHOOSE, None) }
    }
}

impl Verifiable for BankStandReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_BANK_NAME, false)?
            .visit_field::<i32>("x", VT_BANK_X, false)?
            .visit_field::<i32>("z", VT_BANK_Z, false)?
            .visit_field::<i32>("level", VT_BANK_LEVEL, false)?
            .visit_field::<ForwardsUOffset<&str>>("kind", VT_BANK_KIND, false)?
            .visit_field::<i32>("op", VT_BANK_OP, false)?
            .visit_field::<ForwardsUOffset<&str>>("choose", VT_BANK_CHOOSE, false)?
            .finish();
        Ok(())
    }
}

/// The Rust-picked nearest Use-quickly booth as decoded.
#[derive(Clone, Copy)]
pub struct NearestBoothReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for NearestBoothReader<'a> {
    type Inner = NearestBoothReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl NearestBoothReader<'_> {
    pub fn x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_NEAREST_X, None) }.unwrap_or(0)
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_NEAREST_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_NEAREST_LEVEL, None) }.unwrap_or(0)
    }
    pub fn name(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_NEAREST_NAME, None) }.unwrap_or("")
    }
    pub fn op(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_NEAREST_OP, None) }.unwrap_or("")
    }
    pub fn id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_NEAREST_ID, None) }.unwrap_or(-1)
    }
}

impl Verifiable for NearestBoothReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("x", VT_NEAREST_X, false)?
            .visit_field::<i32>("z", VT_NEAREST_Z, false)?
            .visit_field::<i32>("level", VT_NEAREST_LEVEL, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_NEAREST_NAME, false)?
            .visit_field::<ForwardsUOffset<&str>>("op", VT_NEAREST_OP, false)?
            .visit_field::<i32>("id", VT_NEAREST_ID, false)?
            .finish();
        Ok(())
    }
}

/// One compact bank dest/readiness row as decoded.
#[derive(Clone, Copy)]
pub struct BankApproachReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for BankApproachReader<'a> {
    type Inner = BankApproachReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl BankApproachReader<'_> {
    pub fn loc_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_LOC_ID, None) }.unwrap_or(0)
    }
    pub fn x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_X, None) }.unwrap_or(0)
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_LEVEL, None) }.unwrap_or(0)
    }
    pub fn can_operate(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_BA_CAN_OPERATE, None) }.unwrap_or(false)
    }
    pub fn dest_ok(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_BA_DEST_OK, None) }.unwrap_or(false)
    }
    pub fn dest_x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_DEST_X, None) }.unwrap_or(0)
    }
    pub fn dest_z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_DEST_Z, None) }.unwrap_or(0)
    }
    pub fn dest_level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BA_DEST_LEVEL, None) }.unwrap_or(0)
    }
}

impl Verifiable for BankApproachReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("loc_id", VT_BA_LOC_ID, false)?
            .visit_field::<i32>("x", VT_BA_X, false)?
            .visit_field::<i32>("z", VT_BA_Z, false)?
            .visit_field::<i32>("level", VT_BA_LEVEL, false)?
            .visit_field::<bool>("can_operate", VT_BA_CAN_OPERATE, false)?
            .visit_field::<bool>("dest_ok", VT_BA_DEST_OK, false)?
            .visit_field::<i32>("dest_x", VT_BA_DEST_X, false)?
            .visit_field::<i32>("dest_z", VT_BA_DEST_Z, false)?
            .visit_field::<i32>("dest_level", VT_BA_DEST_LEVEL, false)?
            .finish();
        Ok(())
    }
}

/// The PLAYER_INFO snapshot as decoded: read-only access to the same
/// fields `script_snapshot_fb` encodes.
pub struct SnapshotReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for SnapshotReader<'a> {
    type Inner = SnapshotReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for SnapshotReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<u64>("tick", VT_SNAP_TICK, false)?
            .visit_field::<ForwardsUOffset<TileReader>>("here", VT_SNAP_HERE, false)?
            .visit_field::<bool>("ingame", VT_SNAP_INGAME, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "inv",
                VT_SNAP_INV,
                false,
            )?
            .visit_field::<i32>("inv_size", VT_SNAP_INV_SIZE, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<StatReader>>>>(
                "stats",
                VT_SNAP_STATS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<TileReader>>>>(
                "booths",
                VT_SNAP_BOOTHS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<BankStandReader>>>>(
                "banks",
                VT_SNAP_BANKS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "bank",
                VT_SNAP_BANK,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "bank_side",
                VT_SNAP_BANK_SIDE,
                false,
            )?
            .visit_field::<bool>("bank_open", VT_SNAP_BANK_OPEN, false)?
            .visit_field::<bool>("bank_loaded", VT_SNAP_BANK_LOADED, false)?
            .visit_field::<bool>("hold", VT_SNAP_HOLD, false)?
            .visit_field::<bool>("ours", VT_SNAP_OURS, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<SceneEntityReader>>>>(
                "npcs",
                VT_SNAP_NPCS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<SceneEntityReader>>>>(
                "locs",
                VT_SNAP_LOCS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<SceneEntityReader>>>>(
                "players",
                VT_SNAP_PLAYERS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<SceneEntityReader>>>>(
                "ground",
                VT_SNAP_GROUND,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "equipment",
                VT_SNAP_EQUIPMENT,
                false,
            )?
            .visit_field::<bool>("chat_open", VT_SNAP_CHAT_OPEN, false)?
            .visit_field::<bool>("chat_continue", VT_SNAP_CHAT_CONTINUE, false)?
            .visit_field::<ForwardsUOffset<&str>>("chat_text", VT_SNAP_CHAT_TEXT, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<ChatOptionReader>>>>(
                "chat_options",
                VT_SNAP_CHAT_OPTIONS,
                false,
            )?
            .visit_field::<i32>("side_tab", VT_SNAP_SIDE_TAB, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<VarpReader>>>>(
                "varps",
                VT_SNAP_VARPS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<CombatStyleReader>>>>(
                "combat_styles",
                VT_SNAP_COMBAT_STYLES,
                false,
            )?
            .visit_field::<i32>("run_energy", VT_SNAP_RUN_ENERGY, false)?
            .visit_field::<bool>("run_enabled", VT_SNAP_RUN_ENABLED, false)?
            .visit_field::<bool>("retaliate_enabled", VT_SNAP_RETALIATE, false)?
            .visit_field::<ForwardsUOffset<&str>>("my_name", VT_SNAP_MY_NAME, false)?
            .visit_field::<bool>("in_combat", VT_SNAP_IN_COMBAT, false)?
            .visit_field::<bool>("animating", VT_SNAP_ANIMATING, false)?
            .visit_field::<i32>("main_modal_id", VT_SNAP_MAIN_MODAL, false)?
            .visit_field::<i32>("chat_modal_id", VT_SNAP_CHAT_MODAL, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<MakeProductReader>>>>(
                "make_products",
                VT_SNAP_MAKE_PRODUCTS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<SideTabIfaceReader>>>>(
                "side_tab_ifaces",
                VT_SNAP_SIDE_TAB_IFACES,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<CombatStyleReader>>>>(
                "spell_buttons",
                VT_SNAP_SPELL_BUTTONS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<ChatLineReader>>>>(
                "chat_lines",
                VT_SNAP_CHAT_LINES,
                false,
            )?
            .visit_field::<ForwardsUOffset<NearestBoothReader>>(
                "nearest_booth",
                VT_SNAP_NEAREST_BOOTH,
                false,
            )?
            .visit_field::<i32>("bank_note_on", VT_SNAP_BANK_NOTE_ON, false)?
            .visit_field::<i32>("bank_note_off", VT_SNAP_BANK_NOTE_OFF, false)?
            .visit_field::<i32>("scene_state", VT_SNAP_SCENE_STATE, false)?
            .visit_field::<i32>("weight", VT_SNAP_WEIGHT, false)?
            .visit_field::<i32>("camera_yaw", VT_SNAP_CAMERA_YAW, false)?
            .visit_field::<i32>("camera_pitch", VT_SNAP_CAMERA_PITCH, false)?
            .visit_field::<bool>("teleports_enabled", VT_SNAP_TELEPORTS_ENABLED, false)?
            .visit_field::<i32>("self_slot", VT_SNAP_SELF_SLOT, false)?
            .visit_field::<bool>("trade_offer_open", VT_SNAP_TRADE_OFFER_OPEN, false)?
            .visit_field::<bool>("trade_confirm_open", VT_SNAP_TRADE_CONFIRM_OPEN, false)?
            .visit_field::<ForwardsUOffset<&str>>("trade_partner", VT_SNAP_TRADE_PARTNER, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "trade_mine",
                VT_SNAP_TRADE_MINE,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "trade_theirs",
                VT_SNAP_TRADE_THEIRS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "trade_side",
                VT_SNAP_TRADE_SIDE,
                false,
            )?
            .visit_field::<i32>("trade_accept_id", VT_SNAP_TRADE_ACCEPT_ID, false)?
            .visit_field::<i32>("trade_decline_id", VT_SNAP_TRADE_DECLINE_ID, false)?
            .visit_field::<bool>("shop_open", VT_SNAP_SHOP_OPEN, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "shop_stock",
                VT_SNAP_SHOP_STOCK,
                false,
            )?
            .visit_field::<u64>("bank_generation", VT_SNAP_BANK_GENERATION, false)?
            .visit_field::<bool>("count_dialog_open", VT_SNAP_COUNT_DIALOG_OPEN, false)?
            .visit_field::<u64>(
                "withdraw_x_result_seq",
                VT_SNAP_WITHDRAW_X_RESULT_SEQ,
                false,
            )?
            .visit_field::<bool>("withdraw_x_result", VT_SNAP_WITHDRAW_X_RESULT, false)?
            .visit_field::<u64>(
                "withdraw_load_result_seq",
                VT_SNAP_WITHDRAW_LOAD_RESULT_SEQ,
                false,
            )?
            .visit_field::<bool>("withdraw_load_result", VT_SNAP_WITHDRAW_LOAD_RESULT, false)?
            .visit_field::<u64>("bank_op_result_seq", VT_SNAP_BANK_OP_RESULT_SEQ, false)?
            .visit_field::<bool>("bank_op_result", VT_SNAP_BANK_OP_RESULT, false)?
            .visit_field::<ForwardsUOffset<ReachReader>>("reach", VT_SNAP_REACH, false)?
            .visit_field::<bool>("attacked_by_player", VT_SNAP_ATTACKED_BY_PLAYER, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<WidgetTextReader>>>>(
                "widgets",
                VT_SNAP_WIDGETS,
                false,
            )?
            .visit_field::<ForwardsUOffset<&str>>("self_chat", VT_SNAP_SELF_CHAT, false)?
            .visit_field::<i32>("hint_tile_x", VT_SNAP_HINT_TILE_X, false)?
            .visit_field::<i32>("hint_tile_z", VT_SNAP_HINT_TILE_Z, false)?
            .visit_field::<i32>("retaliate_on_com_id", VT_SNAP_RETALIATE_ON_COM_ID, false)?
            .visit_field::<i32>("retaliate_off_com_id", VT_SNAP_RETALIATE_OFF_COM_ID, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<QuestStatusReader>>>>(
                "quest_statuses",
                VT_SNAP_QUEST_STATUSES,
                false,
            )?
            .visit_field::<bool>(
                "quest_statuses_available",
                VT_SNAP_QUEST_STATUSES_AVAILABLE,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<NpcBoxReader>>>>(
                "npc_boxes",
                VT_SNAP_NPC_BOXES,
                false,
            )?
            .visit_field::<bool>("npc_boxes_available", VT_SNAP_NPC_BOXES_AVAILABLE, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "shop_player",
                VT_SNAP_SHOP_PLAYER,
                false,
            )?
            .visit_field::<bool>(
                "shop_player_available",
                VT_SNAP_SHOP_PLAYER_AVAILABLE,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<RowReader>>>>(
                "main_make",
                VT_SNAP_MAIN_MAKE,
                false,
            )?
            .visit_field::<bool>("main_make_available", VT_SNAP_MAIN_MAKE_AVAILABLE, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<BankApproachReader>>>>(
                "bank_approaches",
                VT_SNAP_BANK_APPROACHES,
                false,
            )?
            .visit_field::<u64>("walk_outcome_seq", VT_SNAP_WALK_OUTCOME_SEQ, false)?
            .visit_field::<u64>(
                "walk_outcome_generation",
                VT_SNAP_WALK_OUTCOME_GENERATION,
                false,
            )?
            .visit_field::<bool>("walk_outcome_failed", VT_SNAP_WALK_OUTCOME_FAILED, false)?
            .visit_field::<i32>("walk_outcome_x", VT_SNAP_WALK_OUTCOME_X, false)?
            .visit_field::<i32>("walk_outcome_z", VT_SNAP_WALK_OUTCOME_Z, false)?
            .visit_field::<i32>("walk_outcome_level", VT_SNAP_WALK_OUTCOME_LEVEL, false)?
            .visit_field::<i32>("walk_outcome_radius", VT_SNAP_WALK_OUTCOME_RADIUS, false)?
            .visit_field::<bool>(
                "walk_outcome_allow_teleports",
                VT_SNAP_WALK_OUTCOME_ALLOW_TELEPORTS,
                false,
            )?
            .visit_field::<u64>(
                "walk_outcome_request_id",
                VT_SNAP_WALK_OUTCOME_REQUEST_ID,
                false,
            )?
            .visit_field::<i32>("canvas_width", VT_SNAP_CANVAS_WIDTH, false)?
            .visit_field::<i32>("canvas_height", VT_SNAP_CANVAS_HEIGHT, false)?
            .finish();
        Ok(())
    }
}

impl SnapshotReader<'_> {
    /// Interpret `buf` as a root-`Snapshot` FlatBuffer after verification.
    pub fn from_bytes(buf: &[u8]) -> Result<SnapshotReader<'_>, String> {
        verified_root::<SnapshotReader>(buf)
    }

    pub fn tick(&self) -> u64 {
        // Safety: the buffer was produced by our encoder (root checked).
        unsafe { self.tab.get::<u64>(VT_SNAP_TICK, None) }.unwrap_or(0)
    }
    /// Whether the buffer carries the `here` tile. A delta omits the
    /// fields that did not change since the last post — absent is distinct
    /// from empty, and the isolate keeps its last JS value.
    pub fn has_here(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<TileReader>>(VT_SNAP_HERE, None)
                .is_some()
        }
    }
    pub fn here(&self) -> Option<TileReader<'_>> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<TileReader>>(VT_SNAP_HERE, None)
        }
    }
    pub fn has_ingame(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_INGAME, None).is_some() }
    }
    pub fn ingame(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_INGAME, None) }.unwrap_or(false)
    }
    pub fn has_inv(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_INV)
    }
    pub fn inv(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_INV)
    }
    pub fn has_inv_size(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_INV_SIZE, None).is_some() }
    }
    pub fn inv_size(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_INV_SIZE, None) }.unwrap_or(0)
    }
    pub fn has_stats(&self) -> bool {
        rows_present::<StatReader>(&self.tab, VT_SNAP_STATS)
    }
    pub fn stats(&self) -> Vec<StatReader<'_>> {
        rows::<StatReader>(&self.tab, VT_SNAP_STATS)
    }
    pub fn has_booths(&self) -> bool {
        rows_present::<TileReader>(&self.tab, VT_SNAP_BOOTHS)
    }
    pub fn booths(&self) -> Vec<TileReader<'_>> {
        rows::<TileReader>(&self.tab, VT_SNAP_BOOTHS)
    }
    pub fn has_banks(&self) -> bool {
        rows_present::<BankStandReader>(&self.tab, VT_SNAP_BANKS)
    }
    pub fn banks(&self) -> Vec<BankStandReader<'_>> {
        rows::<BankStandReader>(&self.tab, VT_SNAP_BANKS)
    }
    pub fn has_bank(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_BANK)
    }
    pub fn bank(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_BANK)
    }
    pub fn has_bank_side(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_BANK_SIDE)
    }
    pub fn bank_side(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_BANK_SIDE)
    }
    pub fn has_bank_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_OPEN, None).is_some() }
    }
    pub fn bank_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_bank_loaded(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_LOADED, None).is_some() }
    }
    pub fn bank_loaded(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_LOADED, None) }.unwrap_or(false)
    }
    pub fn has_bank_generation(&self) -> bool {
        unsafe { self.tab.get::<u64>(VT_SNAP_BANK_GENERATION, None).is_some() }
    }
    pub fn bank_generation(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_BANK_GENERATION, None) }.unwrap_or(0)
    }
    pub fn has_count_dialog_open(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_COUNT_DIALOG_OPEN, None)
                .is_some()
        }
    }
    pub fn count_dialog_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_COUNT_DIALOG_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_withdraw_x_result_seq(&self) -> bool {
        unsafe {
            self.tab
                .get::<u64>(VT_SNAP_WITHDRAW_X_RESULT_SEQ, None)
                .is_some()
        }
    }
    pub fn withdraw_x_result_seq(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_WITHDRAW_X_RESULT_SEQ, None) }.unwrap_or(0)
    }
    pub fn has_withdraw_x_result(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_WITHDRAW_X_RESULT, None)
                .is_some()
        }
    }
    pub fn withdraw_x_result(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_WITHDRAW_X_RESULT, None) }.unwrap_or(false)
    }
    pub fn has_withdraw_load_result_seq(&self) -> bool {
        unsafe {
            self.tab
                .get::<u64>(VT_SNAP_WITHDRAW_LOAD_RESULT_SEQ, None)
                .is_some()
        }
    }
    pub fn withdraw_load_result_seq(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_WITHDRAW_LOAD_RESULT_SEQ, None) }.unwrap_or(0)
    }
    pub fn has_withdraw_load_result(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_WITHDRAW_LOAD_RESULT, None)
                .is_some()
        }
    }
    pub fn withdraw_load_result(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_WITHDRAW_LOAD_RESULT, None) }.unwrap_or(false)
    }
    pub fn has_bank_op_result_seq(&self) -> bool {
        unsafe {
            self.tab
                .get::<u64>(VT_SNAP_BANK_OP_RESULT_SEQ, None)
                .is_some()
        }
    }
    pub fn bank_op_result_seq(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_BANK_OP_RESULT_SEQ, None) }.unwrap_or(0)
    }
    pub fn has_bank_op_result(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_OP_RESULT, None).is_some() }
    }
    pub fn bank_op_result(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_BANK_OP_RESULT, None) }.unwrap_or(false)
    }
    pub fn has_bank_note_on(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_BANK_NOTE_ON, None).is_some() }
    }
    pub fn bank_note_on(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_BANK_NOTE_ON, None) }.unwrap_or(-1)
    }
    pub fn has_bank_note_off(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_BANK_NOTE_OFF, None).is_some() }
    }
    pub fn bank_note_off(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_BANK_NOTE_OFF, None) }.unwrap_or(-1)
    }
    pub fn has_scene_state(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_SCENE_STATE, None).is_some() }
    }
    pub fn scene_state(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_SCENE_STATE, None) }.unwrap_or(0)
    }
    pub fn has_weight(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_WEIGHT, None).is_some() }
    }
    pub fn weight(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_WEIGHT, None) }.unwrap_or(0)
    }
    pub fn has_camera_yaw(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_CAMERA_YAW, None).is_some() }
    }
    pub fn camera_yaw(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_CAMERA_YAW, None) }.unwrap_or(0)
    }
    pub fn has_camera_pitch(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_CAMERA_PITCH, None).is_some() }
    }
    pub fn camera_pitch(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_CAMERA_PITCH, None) }.unwrap_or(0)
    }
    pub fn has_teleports_enabled(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_TELEPORTS_ENABLED, None)
                .is_some()
        }
    }
    pub fn teleports_enabled(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_TELEPORTS_ENABLED, None) }.unwrap_or(false)
    }
    pub fn has_self_slot(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_SELF_SLOT, None).is_some() }
    }
    pub fn self_slot(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_SELF_SLOT, None) }.unwrap_or(0)
    }
    pub fn has_trade_offer_open(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_TRADE_OFFER_OPEN, None)
                .is_some()
        }
    }
    pub fn trade_offer_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_TRADE_OFFER_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_trade_confirm_open(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_TRADE_CONFIRM_OPEN, None)
                .is_some()
        }
    }
    pub fn trade_confirm_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_TRADE_CONFIRM_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_trade_partner(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_TRADE_PARTNER, None)
                .is_some()
        }
    }
    pub fn trade_partner(&self) -> Option<&str> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_TRADE_PARTNER, None)
        }
    }
    pub fn has_trade_mine(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_TRADE_MINE)
    }
    pub fn trade_mine(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_TRADE_MINE)
    }
    pub fn has_trade_theirs(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_TRADE_THEIRS)
    }
    pub fn trade_theirs(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_TRADE_THEIRS)
    }
    pub fn has_trade_side(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_TRADE_SIDE)
    }
    pub fn trade_side(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_TRADE_SIDE)
    }
    pub fn has_trade_accept_id(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_TRADE_ACCEPT_ID, None).is_some() }
    }
    pub fn trade_accept_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_TRADE_ACCEPT_ID, None) }.unwrap_or(-1)
    }
    pub fn has_trade_decline_id(&self) -> bool {
        unsafe {
            self.tab
                .get::<i32>(VT_SNAP_TRADE_DECLINE_ID, None)
                .is_some()
        }
    }
    pub fn trade_decline_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_TRADE_DECLINE_ID, None) }.unwrap_or(-1)
    }
    pub fn has_shop_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_SHOP_OPEN, None).is_some() }
    }
    pub fn shop_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_SHOP_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_shop_stock(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_SHOP_STOCK)
    }
    pub fn shop_stock(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_SHOP_STOCK)
    }
    pub fn has_shop_player(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_SHOP_PLAYER)
    }
    pub fn shop_player(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_SHOP_PLAYER)
    }
    pub fn has_shop_player_available(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_SHOP_PLAYER_AVAILABLE, None)
                .is_some()
        }
    }
    pub fn shop_player_available(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_SHOP_PLAYER_AVAILABLE, None) }.unwrap_or(false)
    }
    pub fn has_main_make(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_MAIN_MAKE)
    }
    pub fn main_make(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_MAIN_MAKE)
    }
    pub fn has_main_make_available(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_MAIN_MAKE_AVAILABLE, None)
                .is_some()
        }
    }
    pub fn main_make_available(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_MAIN_MAKE_AVAILABLE, None) }.unwrap_or(false)
    }
    pub fn has_bank_approaches(&self) -> bool {
        rows_present::<BankApproachReader>(&self.tab, VT_SNAP_BANK_APPROACHES)
    }
    pub fn bank_approaches(&self) -> Vec<BankApproachReader<'_>> {
        rows::<BankApproachReader>(&self.tab, VT_SNAP_BANK_APPROACHES)
    }
    pub fn has_walk_outcome_seq(&self) -> bool {
        unsafe {
            self.tab
                .get::<u64>(VT_SNAP_WALK_OUTCOME_SEQ, None)
                .is_some()
        }
    }
    pub fn walk_outcome_seq(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_WALK_OUTCOME_SEQ, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_generation(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_WALK_OUTCOME_GENERATION, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_failed(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_WALK_OUTCOME_FAILED, None) }.unwrap_or(false)
    }
    pub fn walk_outcome_x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_WALK_OUTCOME_X, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_WALK_OUTCOME_Z, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_WALK_OUTCOME_LEVEL, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_radius(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_WALK_OUTCOME_RADIUS, None) }.unwrap_or(0)
    }
    pub fn walk_outcome_allow_teleports(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_WALK_OUTCOME_ALLOW_TELEPORTS, None)
        }
        .unwrap_or(false)
    }
    pub fn walk_outcome_request_id(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_SNAP_WALK_OUTCOME_REQUEST_ID, None) }.unwrap_or(0)
    }
    pub fn has_canvas_width(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_CANVAS_WIDTH, None).is_some() }
    }
    pub fn canvas_width(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_CANVAS_WIDTH, None) }.unwrap_or(0)
    }
    pub fn canvas_height(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_CANVAS_HEIGHT, None) }.unwrap_or(0)
    }
    pub fn has_reach(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<ReachReader>>(VT_SNAP_REACH, None)
                .is_some()
        }
    }
    pub fn reach(&self) -> Option<ReachReader<'_>> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<ReachReader>>(VT_SNAP_REACH, None)
        }
    }
    pub fn has_attacked_by_player(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_ATTACKED_BY_PLAYER, None)
                .is_some()
        }
    }
    pub fn attacked_by_player(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_ATTACKED_BY_PLAYER, None) }.unwrap_or(false)
    }
    pub fn has_widgets(&self) -> bool {
        rows_present::<WidgetTextReader>(&self.tab, VT_SNAP_WIDGETS)
    }
    pub fn widgets(&self) -> Vec<WidgetTextReader<'_>> {
        rows::<WidgetTextReader>(&self.tab, VT_SNAP_WIDGETS)
    }
    pub fn has_self_chat(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_SELF_CHAT, None)
                .is_some()
        }
    }
    pub fn self_chat(&self) -> Option<&str> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_SELF_CHAT, None)
        }
    }
    pub fn has_hint_tile(&self) -> bool {
        unsafe {
            self.tab.get::<i32>(VT_SNAP_HINT_TILE_X, None).is_some()
                || self.tab.get::<i32>(VT_SNAP_HINT_TILE_Z, None).is_some()
        }
    }
    pub fn hint_tile(&self) -> Option<(i32, i32)> {
        let x = unsafe { self.tab.get::<i32>(VT_SNAP_HINT_TILE_X, None) }.unwrap_or(-1);
        let z = unsafe { self.tab.get::<i32>(VT_SNAP_HINT_TILE_Z, None) }.unwrap_or(-1);
        (x >= 0 && z >= 0).then_some((x, z))
    }
    pub fn has_retaliate_controls(&self) -> bool {
        unsafe {
            self.tab
                .get::<i32>(VT_SNAP_RETALIATE_ON_COM_ID, None)
                .is_some()
                || self
                    .tab
                    .get::<i32>(VT_SNAP_RETALIATE_OFF_COM_ID, None)
                    .is_some()
        }
    }
    pub fn retaliate_controls(&self) -> Option<(i32, i32)> {
        let on = unsafe { self.tab.get::<i32>(VT_SNAP_RETALIATE_ON_COM_ID, None) }.unwrap_or(-1);
        let off = unsafe { self.tab.get::<i32>(VT_SNAP_RETALIATE_OFF_COM_ID, None) }.unwrap_or(-1);
        (on >= 0 && off >= 0).then_some((on, off))
    }
    pub fn has_quest_statuses(&self) -> bool {
        rows_present::<QuestStatusReader>(&self.tab, VT_SNAP_QUEST_STATUSES)
    }
    pub fn quest_statuses(&self) -> Vec<QuestStatusReader<'_>> {
        rows::<QuestStatusReader>(&self.tab, VT_SNAP_QUEST_STATUSES)
    }
    pub fn has_quest_statuses_update(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_QUEST_STATUSES_AVAILABLE, None)
                .is_some()
        }
    }
    pub fn quest_statuses_available(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_QUEST_STATUSES_AVAILABLE, Some(false))
        }
        .unwrap_or(false)
    }
    pub fn has_npc_boxes(&self) -> bool {
        rows_present::<NpcBoxReader>(&self.tab, VT_SNAP_NPC_BOXES)
    }
    pub fn npc_boxes(&self) -> Vec<NpcBoxReader<'_>> {
        rows::<NpcBoxReader>(&self.tab, VT_SNAP_NPC_BOXES)
    }
    pub fn has_npc_boxes_update(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_NPC_BOXES_AVAILABLE, None)
                .is_some()
        }
    }
    pub fn npc_boxes_available(&self) -> bool {
        unsafe {
            self.tab
                .get::<bool>(VT_SNAP_NPC_BOXES_AVAILABLE, Some(false))
        }
        .unwrap_or(false)
    }
    pub fn has_hold(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_HOLD, None).is_some() }
    }
    pub fn hold(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_HOLD, None) }.unwrap_or(false)
    }
    pub fn has_ours(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_OURS, None).is_some() }
    }
    pub fn ours(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_OURS, None) }.unwrap_or(false)
    }
    pub fn has_npcs(&self) -> bool {
        rows_present::<SceneEntityReader>(&self.tab, VT_SNAP_NPCS)
    }
    pub fn npcs(&self) -> Vec<SceneEntityReader<'_>> {
        rows::<SceneEntityReader>(&self.tab, VT_SNAP_NPCS)
    }
    pub fn has_locs(&self) -> bool {
        rows_present::<SceneEntityReader>(&self.tab, VT_SNAP_LOCS)
    }
    pub fn locs(&self) -> Vec<SceneEntityReader<'_>> {
        rows::<SceneEntityReader>(&self.tab, VT_SNAP_LOCS)
    }
    pub fn has_players(&self) -> bool {
        rows_present::<SceneEntityReader>(&self.tab, VT_SNAP_PLAYERS)
    }
    pub fn players(&self) -> Vec<SceneEntityReader<'_>> {
        rows::<SceneEntityReader>(&self.tab, VT_SNAP_PLAYERS)
    }
    pub fn has_ground(&self) -> bool {
        rows_present::<SceneEntityReader>(&self.tab, VT_SNAP_GROUND)
    }
    pub fn ground(&self) -> Vec<SceneEntityReader<'_>> {
        rows::<SceneEntityReader>(&self.tab, VT_SNAP_GROUND)
    }
    pub fn has_equipment(&self) -> bool {
        rows_present::<RowReader>(&self.tab, VT_SNAP_EQUIPMENT)
    }
    pub fn equipment(&self) -> Vec<RowReader<'_>> {
        rows::<RowReader>(&self.tab, VT_SNAP_EQUIPMENT)
    }
    pub fn has_chat_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_CHAT_OPEN, None).is_some() }
    }
    pub fn chat_open(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_CHAT_OPEN, None) }.unwrap_or(false)
    }
    pub fn has_chat_continue(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_CHAT_CONTINUE, None).is_some() }
    }
    pub fn chat_continue(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_CHAT_CONTINUE, None) }.unwrap_or(false)
    }
    pub fn has_chat_text(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_CHAT_TEXT, None)
                .is_some()
        }
    }
    pub fn chat_text(&self) -> Option<&str> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_CHAT_TEXT, None)
        }
    }
    pub fn has_chat_options(&self) -> bool {
        rows_present::<ChatOptionReader>(&self.tab, VT_SNAP_CHAT_OPTIONS)
    }
    pub fn chat_options(&self) -> Vec<ChatOptionReader<'_>> {
        rows::<ChatOptionReader>(&self.tab, VT_SNAP_CHAT_OPTIONS)
    }
    pub fn has_side_tab(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_SIDE_TAB, None).is_some() }
    }
    pub fn side_tab(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_SIDE_TAB, None) }.unwrap_or(-1)
    }
    pub fn has_varps(&self) -> bool {
        rows_present::<VarpReader>(&self.tab, VT_SNAP_VARPS)
    }
    pub fn varps(&self) -> Vec<VarpReader<'_>> {
        rows::<VarpReader>(&self.tab, VT_SNAP_VARPS)
    }
    pub fn has_combat_styles(&self) -> bool {
        rows_present::<CombatStyleReader>(&self.tab, VT_SNAP_COMBAT_STYLES)
    }
    pub fn combat_styles(&self) -> Vec<CombatStyleReader<'_>> {
        rows::<CombatStyleReader>(&self.tab, VT_SNAP_COMBAT_STYLES)
    }
    pub fn has_run_energy(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_RUN_ENERGY, None).is_some() }
    }
    pub fn run_energy(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_RUN_ENERGY, None) }.unwrap_or(0)
    }
    pub fn has_run_enabled(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_RUN_ENABLED, None).is_some() }
    }
    pub fn run_enabled(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_RUN_ENABLED, None) }.unwrap_or(false)
    }
    pub fn has_retaliate_enabled(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_RETALIATE, None).is_some() }
    }
    pub fn retaliate_enabled(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_RETALIATE, None) }.unwrap_or(false)
    }
    pub fn has_my_name(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_SNAP_MY_NAME, None)
                .is_some()
        }
    }
    pub fn my_name(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_SNAP_MY_NAME, None) }
    }
    pub fn has_in_combat(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_IN_COMBAT, None).is_some() }
    }
    pub fn in_combat(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_IN_COMBAT, None) }.unwrap_or(false)
    }
    pub fn has_animating(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_ANIMATING, None).is_some() }
    }
    pub fn animating(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_SNAP_ANIMATING, None) }.unwrap_or(false)
    }
    pub fn has_main_modal_id(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_MAIN_MODAL, None).is_some() }
    }
    pub fn main_modal_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_MAIN_MODAL, None) }.unwrap_or(-1)
    }
    pub fn has_chat_modal_id(&self) -> bool {
        unsafe { self.tab.get::<i32>(VT_SNAP_CHAT_MODAL, None).is_some() }
    }
    pub fn chat_modal_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_SNAP_CHAT_MODAL, None) }.unwrap_or(-1)
    }
    pub fn has_make_products(&self) -> bool {
        rows_present::<MakeProductReader>(&self.tab, VT_SNAP_MAKE_PRODUCTS)
    }
    pub fn make_products(&self) -> Vec<MakeProductReader<'_>> {
        rows::<MakeProductReader>(&self.tab, VT_SNAP_MAKE_PRODUCTS)
    }
    pub fn has_side_tab_ifaces(&self) -> bool {
        rows_present::<SideTabIfaceReader>(&self.tab, VT_SNAP_SIDE_TAB_IFACES)
    }
    pub fn side_tab_ifaces(&self) -> Vec<SideTabIfaceReader<'_>> {
        rows::<SideTabIfaceReader>(&self.tab, VT_SNAP_SIDE_TAB_IFACES)
    }
    pub fn has_spell_buttons(&self) -> bool {
        rows_present::<CombatStyleReader>(&self.tab, VT_SNAP_SPELL_BUTTONS)
    }
    pub fn spell_buttons(&self) -> Vec<CombatStyleReader<'_>> {
        rows::<CombatStyleReader>(&self.tab, VT_SNAP_SPELL_BUTTONS)
    }
    pub fn has_chat_lines(&self) -> bool {
        rows_present::<ChatLineReader>(&self.tab, VT_SNAP_CHAT_LINES)
    }
    pub fn chat_lines(&self) -> Vec<ChatLineReader<'_>> {
        rows::<ChatLineReader>(&self.tab, VT_SNAP_CHAT_LINES)
    }
    pub fn has_nearest_booth(&self) -> bool {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<NearestBoothReader>>(VT_SNAP_NEAREST_BOOTH, None)
                .is_some()
        }
    }
    pub fn nearest_booth(&self) -> Option<NearestBoothReader<'_>> {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<NearestBoothReader>>(VT_SNAP_NEAREST_BOOTH, None)
        }
    }
}

/// Decode `buf` as a root-`Snapshot` FlatBuffer (produced by our own
/// encoder; only the root offset is bounds-checked).
pub fn decode_snapshot(buf: &[u8]) -> Result<SnapshotReader<'_>, String> {
    SnapshotReader::from_bytes(buf)
}

/// Read a vector of tables at `slot` from `tab` (empty when absent).
fn rows<'a, T>(tab: &Table<'a>, slot: VOffsetT) -> Vec<T::Inner>
where
    T: flatbuffers::Follow<'a> + 'a,
{
    // Safety: the buffer was produced by our encoder (root checked).
    match unsafe { tab.get::<ForwardsUOffset<Vector<'a, ForwardsUOffset<T>>>>(slot, None) } {
        Some(v) => v.iter().collect(),
        None => Vec::new(),
    }
}

fn u32_vec(tab: &Table<'_>, slot: VOffsetT) -> Vec<u32> {
    match unsafe { tab.get::<ForwardsUOffset<Vector<u32>>>(slot, None) } {
        Some(v) => v.iter().collect(),
        None => Vec::new(),
    }
}

fn u16_vec(tab: &Table<'_>, slot: VOffsetT) -> Vec<u16> {
    match unsafe { tab.get::<ForwardsUOffset<Vector<u16>>>(slot, None) } {
        Some(v) => v.iter().collect(),
        None => Vec::new(),
    }
}

fn u8_vec(tab: &Table<'_>, slot: VOffsetT) -> Vec<u8> {
    match unsafe { tab.get::<ForwardsUOffset<Vector<u8>>>(slot, None) } {
        Some(v) => v.iter().collect(),
        None => Vec::new(),
    }
}

/// Whether the buffer carries the vector at `slot` — a delta omits an
/// unchanged table entirely, and absent must stay distinct from empty
/// (the isolate keeps its last JS rows for an omitted table).
fn rows_present<'a, T>(tab: &Table<'a>, slot: VOffsetT) -> bool
where
    T: flatbuffers::Follow<'a> + 'a,
{
    // Safety: verified before any accessor use.
    unsafe {
        tab.get::<ForwardsUOffset<Vector<'a, ForwardsUOffset<T>>>>(slot, None)
            .is_some()
    }
}

/// Like [`rows`], but rejects vectors longer than `max_len`.
fn rows_capped<'a, T>(
    tab: &Table<'a>,
    slot: VOffsetT,
    max_len: usize,
) -> Result<Vec<T::Inner>, String>
where
    T: flatbuffers::Follow<'a> + 'a,
{
    // Safety: verified before any accessor use.
    match unsafe { tab.get::<ForwardsUOffset<Vector<'a, ForwardsUOffset<T>>>>(slot, None) } {
        Some(v) => {
            let len = v.len();
            if len > max_len {
                return Err(format!("vector length {len} exceeds cap {max_len}"));
            }
            Ok(v.iter().collect())
        }
        None => Ok(Vec::new()),
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
    pub chat_options: Vec<String>,
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
    pub widgets: Vec<(i32, String)>,
    pub self_chat: Option<String>,
    pub hint_tile: Option<(i32, i32)>,
    pub retaliate_controls: Option<(i32, i32)>,
    pub quest_statuses: Option<Vec<(String, String)>>,
    pub npc_boxes: Option<Vec<NpcBoxInput>>,
    pub bank_approaches: Option<Vec<BankApproachInput>>,
    pub walk_outcome_seq: u64,
    pub walk_outcome_generation: u64,
    pub walk_outcome_failed: bool,
    pub walk_outcome_x: i32,
    pub walk_outcome_z: i32,
    pub walk_outcome_level: i32,
    pub walk_outcome_radius: i32,
    pub walk_outcome_allow_teleports: bool,
    pub walk_outcome_request_id: u64,
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
                .map(|o| o.text.to_string())
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
            reach: ReachViewFp {
                available: input.reach.available,
                base_x: input.reach.base_x,
                base_z: input.reach.base_z,
                level: input.reach.level,
                width: input.reach.width,
                height: input.reach.height,
                walkable: input.reach.walkable.to_vec(),
                reachable: input.reach.reachable.to_vec(),
                reachable_adj: input.reach.reachable_adj.to_vec(),
                exact_rank: input.reach.exact_rank.to_vec(),
                adjacent_rank: input.reach.adjacent_rank.to_vec(),
                step: input.reach.step.to_vec(),
                canlight: input.reach.canlight.to_vec(),
            },
            attacked_by_player: input.attacked_by_player,
            widgets: input
                .widgets
                .iter()
                .map(|w| (w.component_id, w.text.to_string()))
                .collect(),
            self_chat: native.self_chat.map(str::to_string),
            hint_tile: native.hint_tile,
            retaliate_controls: native.retaliate_controls,
            quest_statuses: native.quest_statuses.map(|rows| {
                rows.iter()
                    .map(|q| (q.name.to_string(), q.status.to_string()))
                    .collect()
            }),
            npc_boxes: native.npc_boxes.map(<[NpcBoxInput]>::to_vec),
            bank_approaches: native.bank_approaches.map(<[BankApproachInput]>::to_vec),
            walk_outcome_seq: native.walk_outcome_seq,
            walk_outcome_generation: native.walk_outcome_generation,
            walk_outcome_failed: native.walk_outcome_failed,
            walk_outcome_x: native.walk_outcome_x,
            walk_outcome_z: native.walk_outcome_z,
            walk_outcome_level: native.walk_outcome_level,
            walk_outcome_radius: native.walk_outcome_radius,
            walk_outcome_allow_teleports: native.walk_outcome_allow_teleports,
            walk_outcome_request_id: native.walk_outcome_request_id,
        }
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
    pub widgets: bool,
    pub self_chat: bool,
    pub hint_tile: bool,
    pub retaliate_controls: bool,
    pub quest_statuses: bool,
    pub npc_boxes: bool,
    pub bank_approaches: bool,
    pub walk_outcome: bool,
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
            widgets: true,
            self_chat: true,
            hint_tile: true,
            retaliate_controls: true,
            quest_statuses: true,
            npc_boxes: true,
            bank_approaches: true,
            walk_outcome: true,
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
            widgets: next.widgets != last.widgets,
            self_chat: next.self_chat != last.self_chat,
            hint_tile: next.hint_tile != last.hint_tile,
            retaliate_controls: next.retaliate_controls != last.retaliate_controls,
            quest_statuses: next.quest_statuses != last.quest_statuses,
            npc_boxes: next.npc_boxes != last.npc_boxes,
            bank_approaches: next.bank_approaches != last.bank_approaches,
            walk_outcome: next.walk_outcome_seq != last.walk_outcome_seq
                || next.walk_outcome_generation != last.walk_outcome_generation
                || next.walk_outcome_failed != last.walk_outcome_failed
                || next.walk_outcome_x != last.walk_outcome_x
                || next.walk_outcome_z != last.walk_outcome_z
                || next.walk_outcome_level != last.walk_outcome_level
                || next.walk_outcome_radius != last.walk_outcome_radius
                || next.walk_outcome_allow_teleports != last.walk_outcome_allow_teleports
                || next.walk_outcome_request_id != last.walk_outcome_request_id,
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
        Self {
            builder: FlatBufferBuilder::new(),
        }
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
        let fp = SnapshotFingerprint::from_input_with_native(input, native);
        let mask = match last {
            None => DeltaMask::all(),
            Some(prev) => DeltaMask::changed(prev, &fp, force_banks),
        };
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

    /// Encode one recorded paint frame as a root-`Paint` FlatBuffer.
    pub fn encode_paint(&mut self, paint: &crate::shim::ScriptPaint) -> Vec<u8> {
        self.builder.reset();
        encode_paint_into(&mut self.builder, paint);
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
            .map(|t| tile_off(b, *t))
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
    let tab = b.start_table();
    b.push_slot_always(VT_SNAP_TICK, input.tick);
    if mask.here {
        if let Some(off) = here_off {
            b.push_slot_always(VT_SNAP_HERE, off);
        }
    }
    if mask.ingame {
        b.push_slot_always(VT_SNAP_INGAME, input.ingame);
    }
    if mask.inv {
        b.push_slot_always(VT_SNAP_INV, inv_off.expect("mask checked"));
    }
    if mask.inv_size {
        b.push_slot_always(VT_SNAP_INV_SIZE, input.inv_size);
    }
    if mask.stats {
        b.push_slot_always(VT_SNAP_STATS, stats_off.expect("mask checked"));
    }
    if mask.booths {
        b.push_slot_always(VT_SNAP_BOOTHS, booths_off.expect("mask checked"));
    }
    if mask.nearest_booth {
        if let Some(off) = nearest_booth_table_off {
            b.push_slot_always(VT_SNAP_NEAREST_BOOTH, off);
        }
    }
    if mask.banks {
        b.push_slot_always(VT_SNAP_BANKS, banks_off.expect("mask checked"));
    }
    if mask.bank {
        b.push_slot_always(VT_SNAP_BANK, bank_off.expect("mask checked"));
    }
    if mask.bank_side {
        b.push_slot_always(VT_SNAP_BANK_SIDE, bank_side_off.expect("mask checked"));
    }
    if mask.bank_open {
        b.push_slot_always(VT_SNAP_BANK_OPEN, input.bank_open);
    }
    if mask.bank_loaded {
        b.push_slot_always(VT_SNAP_BANK_LOADED, input.bank_loaded);
    }
    if mask.bank_generation {
        b.push_slot_always(VT_SNAP_BANK_GENERATION, input.bank_generation);
    }
    if mask.count_dialog_open {
        b.push_slot_always(VT_SNAP_COUNT_DIALOG_OPEN, input.count_dialog_open);
    }
    if mask.withdraw_x_result_seq {
        b.push_slot_always(VT_SNAP_WITHDRAW_X_RESULT_SEQ, input.withdraw_x_result_seq);
    }
    if mask.withdraw_x_result {
        b.push_slot_always(VT_SNAP_WITHDRAW_X_RESULT, input.withdraw_x_result);
    }
    if mask.withdraw_load_result_seq {
        b.push_slot_always(
            VT_SNAP_WITHDRAW_LOAD_RESULT_SEQ,
            input.withdraw_load_result_seq,
        );
    }
    if mask.withdraw_load_result {
        b.push_slot_always(VT_SNAP_WITHDRAW_LOAD_RESULT, input.withdraw_load_result);
    }
    if mask.bank_op_result_seq {
        b.push_slot_always(VT_SNAP_BANK_OP_RESULT_SEQ, input.bank_op_result_seq);
    }
    if mask.bank_op_result {
        b.push_slot_always(VT_SNAP_BANK_OP_RESULT, input.bank_op_result);
    }
    if mask.hold {
        b.push_slot_always(VT_SNAP_HOLD, input.hold);
    }
    if mask.ours {
        b.push_slot_always(VT_SNAP_OURS, input.ours);
    }
    if mask.npcs {
        b.push_slot_always(VT_SNAP_NPCS, npcs_off.expect("mask checked"));
    }
    if mask.locs {
        b.push_slot_always(VT_SNAP_LOCS, locs_off.expect("mask checked"));
    }
    if mask.players {
        b.push_slot_always(VT_SNAP_PLAYERS, players_off.expect("mask checked"));
    }
    if mask.ground {
        b.push_slot_always(VT_SNAP_GROUND, ground_off.expect("mask checked"));
    }
    if mask.equipment {
        b.push_slot_always(VT_SNAP_EQUIPMENT, equipment_off.expect("mask checked"));
    }
    if mask.chat_open {
        b.push_slot_always(VT_SNAP_CHAT_OPEN, input.chat_open);
    }
    if mask.chat_continue {
        b.push_slot_always(VT_SNAP_CHAT_CONTINUE, input.chat_continue);
    }
    if mask.chat_text {
        b.push_slot_always(VT_SNAP_CHAT_TEXT, chat_text_off.expect("mask checked"));
    }
    if mask.chat_options {
        b.push_slot_always(
            VT_SNAP_CHAT_OPTIONS,
            chat_options_off.expect("mask checked"),
        );
    }
    if mask.side_tab {
        b.push_slot_always(VT_SNAP_SIDE_TAB, input.side_tab);
    }
    if mask.varps {
        b.push_slot_always(VT_SNAP_VARPS, varps_off.expect("mask checked"));
    }
    if mask.combat_styles {
        b.push_slot_always(
            VT_SNAP_COMBAT_STYLES,
            combat_styles_off.expect("mask checked"),
        );
    }
    if mask.run_energy {
        b.push_slot_always(VT_SNAP_RUN_ENERGY, input.run_energy);
    }
    if mask.run_enabled {
        b.push_slot_always(VT_SNAP_RUN_ENABLED, input.run_enabled);
    }
    if mask.retaliate_enabled {
        b.push_slot_always(VT_SNAP_RETALIATE, input.retaliate_enabled);
    }
    if mask.my_name {
        b.push_slot_always(VT_SNAP_MY_NAME, my_name_off.expect("mask checked"));
    }
    if mask.in_combat {
        b.push_slot_always(VT_SNAP_IN_COMBAT, input.in_combat);
    }
    if mask.animating {
        b.push_slot_always(VT_SNAP_ANIMATING, input.animating);
    }
    if mask.main_modal_id {
        b.push_slot_always(VT_SNAP_MAIN_MODAL, input.main_modal_id);
    }
    if mask.chat_modal_id {
        b.push_slot_always(VT_SNAP_CHAT_MODAL, input.chat_modal_id);
    }
    if mask.make_products {
        b.push_slot_always(
            VT_SNAP_MAKE_PRODUCTS,
            make_products_off.expect("mask checked"),
        );
    }
    if mask.side_tab_ifaces {
        b.push_slot_always(
            VT_SNAP_SIDE_TAB_IFACES,
            side_tab_ifaces_off.expect("mask checked"),
        );
    }
    if mask.spell_buttons {
        b.push_slot_always(
            VT_SNAP_SPELL_BUTTONS,
            spell_buttons_off.expect("mask checked"),
        );
    }
    if mask.chat_lines {
        b.push_slot_always(VT_SNAP_CHAT_LINES, chat_lines_off.expect("mask checked"));
    }
    if mask.bank_note_on {
        b.push_slot_always(VT_SNAP_BANK_NOTE_ON, input.bank_note_on);
    }
    if mask.bank_note_off {
        b.push_slot_always(VT_SNAP_BANK_NOTE_OFF, input.bank_note_off);
    }
    if mask.scene_state {
        b.push_slot_always(VT_SNAP_SCENE_STATE, input.scene_state);
    }
    if mask.weight {
        b.push_slot_always(VT_SNAP_WEIGHT, input.weight);
    }
    if mask.camera_yaw {
        b.push_slot_always(VT_SNAP_CAMERA_YAW, input.camera_yaw);
    }
    if mask.camera_pitch {
        b.push_slot_always(VT_SNAP_CAMERA_PITCH, input.camera_pitch);
    }
    if mask.teleports_enabled {
        b.push_slot_always(VT_SNAP_TELEPORTS_ENABLED, input.teleports_enabled);
    }
    if mask.self_slot {
        b.push_slot_always(VT_SNAP_SELF_SLOT, input.self_slot);
    }
    if mask.trade_offer_open {
        b.push_slot_always(VT_SNAP_TRADE_OFFER_OPEN, input.trade_offer_open);
    }
    if mask.trade_confirm_open {
        b.push_slot_always(VT_SNAP_TRADE_CONFIRM_OPEN, input.trade_confirm_open);
    }
    if mask.trade_partner {
        b.push_slot_always(
            VT_SNAP_TRADE_PARTNER,
            trade_partner_off.expect("mask checked"),
        );
    }
    if mask.trade_mine {
        b.push_slot_always(VT_SNAP_TRADE_MINE, trade_mine_off.expect("mask checked"));
    }
    if mask.trade_theirs {
        b.push_slot_always(
            VT_SNAP_TRADE_THEIRS,
            trade_theirs_off.expect("mask checked"),
        );
    }
    if mask.trade_side {
        b.push_slot_always(VT_SNAP_TRADE_SIDE, trade_side_off.expect("mask checked"));
    }
    if mask.trade_accept_id {
        b.push_slot_always(VT_SNAP_TRADE_ACCEPT_ID, input.trade_accept_id);
    }
    if mask.trade_decline_id {
        b.push_slot_always(VT_SNAP_TRADE_DECLINE_ID, input.trade_decline_id);
    }
    if mask.shop_open {
        b.push_slot_always(VT_SNAP_SHOP_OPEN, input.shop_open);
    }
    if mask.shop_stock {
        b.push_slot_always(VT_SNAP_SHOP_STOCK, shop_stock_off.expect("mask checked"));
    }
    if mask.shop_player {
        b.push_slot_always(VT_SNAP_SHOP_PLAYER_AVAILABLE, native.shop_player.is_some());
        if let Some(off) = shop_player_off {
            b.push_slot_always(VT_SNAP_SHOP_PLAYER, off);
        }
    }
    if mask.main_make {
        b.push_slot_always(VT_SNAP_MAIN_MAKE_AVAILABLE, native.main_make.is_some());
        if let Some(off) = main_make_off {
            b.push_slot_always(VT_SNAP_MAIN_MAKE, off);
        }
    }
    if mask.reach {
        b.push_slot_always(VT_SNAP_REACH, reach_table_off.expect("mask checked"));
    }
    if mask.attacked_by_player {
        b.push_slot_always(VT_SNAP_ATTACKED_BY_PLAYER, input.attacked_by_player);
    }
    if mask.widgets {
        b.push_slot_always(VT_SNAP_WIDGETS, widgets_off.expect("mask checked"));
    }
    if mask.self_chat {
        b.push_slot_always(VT_SNAP_SELF_CHAT, self_chat_off.expect("mask checked"));
    }
    if mask.hint_tile {
        let (x, z) = native.hint_tile.unwrap_or((-1, -1));
        b.push_slot_always(VT_SNAP_HINT_TILE_X, x);
        b.push_slot_always(VT_SNAP_HINT_TILE_Z, z);
    }
    if mask.retaliate_controls {
        let (on, off) = native.retaliate_controls.unwrap_or((-1, -1));
        b.push_slot_always(VT_SNAP_RETALIATE_ON_COM_ID, on);
        b.push_slot_always(VT_SNAP_RETALIATE_OFF_COM_ID, off);
    }
    if mask.quest_statuses {
        b.push_slot_always(
            VT_SNAP_QUEST_STATUSES_AVAILABLE,
            native.quest_statuses.is_some(),
        );
        if let Some(off) = quest_statuses_off {
            b.push_slot_always(VT_SNAP_QUEST_STATUSES, off);
        }
    }
    if mask.npc_boxes {
        b.push_slot_always(VT_SNAP_NPC_BOXES_AVAILABLE, native.npc_boxes.is_some());
        if let Some(off) = npc_boxes_off {
            b.push_slot_always(VT_SNAP_NPC_BOXES, off);
        }
    }
    if mask.bank_approaches {
        if let Some(off) = bank_approaches_off {
            b.push_slot_always(VT_SNAP_BANK_APPROACHES, off);
        }
    }
    if mask.walk_outcome {
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_SEQ, native.walk_outcome_seq);
        b.push_slot_always(
            VT_SNAP_WALK_OUTCOME_GENERATION,
            native.walk_outcome_generation,
        );
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_FAILED, native.walk_outcome_failed);
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_X, native.walk_outcome_x);
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_Z, native.walk_outcome_z);
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_LEVEL, native.walk_outcome_level);
        b.push_slot_always(VT_SNAP_WALK_OUTCOME_RADIUS, native.walk_outcome_radius);
        b.push_slot_always(
            VT_SNAP_WALK_OUTCOME_ALLOW_TELEPORTS,
            native.walk_outcome_allow_teleports,
        );
        b.push_slot_always(
            VT_SNAP_WALK_OUTCOME_REQUEST_ID,
            native.walk_outcome_request_id,
        );
    }
    b.push_slot_always(VT_SNAP_CANVAS_WIDTH, SNAPSHOT_CANVAS_W);
    b.push_slot_always(VT_SNAP_CANVAS_HEIGHT, SNAPSHOT_CANVAS_H);
    let root = b.end_table(tab);
    b.finish(root, None);
}

fn tile_off<'b>(b: &mut FlatBufferBuilder<'b>, t: TileInput) -> WIPOffset<TileReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_TILE_X, t.x);
    b.push_slot_always(VT_TILE_Z, t.z);
    b.push_slot_always(VT_TILE_LEVEL, t.level);
    WIPOffset::new(b.end_table(tab).value())
}

fn bank_approach_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    row: &BankApproachInput,
) -> WIPOffset<BankApproachReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_BA_LOC_ID, row.loc_id);
    b.push_slot_always(VT_BA_X, row.x);
    b.push_slot_always(VT_BA_Z, row.z);
    b.push_slot_always(VT_BA_LEVEL, row.level);
    b.push_slot_always(VT_BA_CAN_OPERATE, row.can_operate);
    b.push_slot_always(VT_BA_DEST_OK, row.dest_ok);
    b.push_slot_always(VT_BA_DEST_X, row.dest_x);
    b.push_slot_always(VT_BA_DEST_Z, row.dest_z);
    b.push_slot_always(VT_BA_DEST_LEVEL, row.dest_level);
    WIPOffset::new(b.end_table(tab).value())
}

fn reach_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    r: &ReachViewInput<'_>,
) -> WIPOffset<ReachReader<'b>> {
    let walkable = b.create_vector(r.walkable);
    let reachable = b.create_vector(r.reachable);
    let reachable_adj = b.create_vector(r.reachable_adj);
    let step = b.create_vector(r.step);
    let exact_rank = b.create_vector(r.exact_rank);
    let adjacent_rank = b.create_vector(r.adjacent_rank);
    let canlight = b.create_vector(r.canlight);
    let tab = b.start_table();
    b.push_slot_always(VT_REACH_AVAILABLE, r.available);
    b.push_slot_always(VT_REACH_BASE_X, r.base_x);
    b.push_slot_always(VT_REACH_BASE_Z, r.base_z);
    b.push_slot_always(VT_REACH_LEVEL, r.level);
    b.push_slot_always(VT_REACH_WIDTH, r.width);
    b.push_slot_always(VT_REACH_HEIGHT, r.height);
    b.push_slot_always(VT_REACH_WALKABLE, walkable);
    b.push_slot_always(VT_REACH_REACHABLE, reachable);
    b.push_slot_always(VT_REACH_REACHABLE_ADJ, reachable_adj);
    b.push_slot_always(VT_REACH_STEP, step);
    b.push_slot_always(VT_REACH_EXACT_RANK, exact_rank);
    b.push_slot_always(VT_REACH_ADJACENT_RANK, adjacent_rank);
    b.push_slot_always(VT_REACH_CANLIGHT, canlight);
    WIPOffset::new(b.end_table(tab).value())
}

fn row_off<'b>(b: &mut FlatBufferBuilder<'b>, r: &ItemRowInput<'_>) -> WIPOffset<RowReader<'b>> {
    let name_off = r.name.map(|n| b.create_string(n));
    let ops_offs: Vec<_> = r.ops.iter().map(|a| b.create_string(a)).collect();
    let ops_off = b.create_vector(&ops_offs);
    let tab = b.start_table();
    if let Some(off) = name_off {
        b.push_slot_always(VT_ROW_NAME, off);
    }
    b.push_slot_always(VT_ROW_COUNT, r.count);
    b.push_slot_always(VT_ROW_ID, r.id);
    b.push_slot_always(VT_ROW_OPS, ops_off);
    b.push_slot_always(VT_ROW_NOTED, r.noted);
    b.push_slot_always(VT_ROW_CERT, r.cert);
    b.push_slot_always(VT_ROW_COMPONENT, r.component_id);
    if r.slot >= 0 {
        b.push_slot_always(VT_ROW_SLOT, r.slot);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn stat_off<'b>(b: &mut FlatBufferBuilder<'b>, s: &StatInput<'_>) -> WIPOffset<StatReader<'b>> {
    let name_off = b.create_string(s.name);
    let tab = b.start_table();
    b.push_slot_always(VT_STAT_INDEX, s.index);
    b.push_slot_always(VT_STAT_NAME, name_off);
    b.push_slot_always(VT_STAT_XP, s.xp);
    b.push_slot_always(VT_STAT_BASE, s.base);
    b.push_slot_always(VT_STAT_EFFECTIVE, s.effective);
    WIPOffset::new(b.end_table(tab).value())
}

fn scene_entity_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    e: &SceneEntityInput<'_>,
) -> WIPOffset<SceneEntityReader<'b>> {
    let name_off = e.name.map(|n| b.create_string(n));
    let action_offs: Vec<_> = e.actions.iter().map(|a| b.create_string(a)).collect();
    let actions_off = b.create_vector(&action_offs);
    let tab = b.start_table();
    b.push_slot_always(VT_ENT_INDEX, e.index);
    b.push_slot_always(VT_ENT_ID, e.id);
    if let Some(off) = name_off {
        b.push_slot_always(VT_ENT_NAME, off);
    }
    b.push_slot_always(VT_ENT_X, e.x);
    b.push_slot_always(VT_ENT_Z, e.z);
    b.push_slot_always(VT_ENT_LEVEL, e.level);
    b.push_slot_always(VT_ENT_DISTANCE, e.distance);
    b.push_slot_always(VT_ENT_HEALTH, e.health);
    b.push_slot_always(VT_ENT_MAX_HEALTH, e.max_health);
    b.push_slot_always(VT_ENT_IN_COMBAT, e.in_combat);
    b.push_slot_always(VT_ENT_ANIMATING, e.animating);
    b.push_slot_always(VT_ENT_ACTIONS, actions_off);
    b.push_slot_always(VT_ENT_REACHABLE, e.reachable);
    b.push_slot_always(VT_ENT_REACHABLE_ADJ, e.reachable_adj);
    b.push_slot_always(VT_ENT_COMBAT_LEVEL, e.combat_level);
    b.push_slot_always(VT_ENT_TARGET_KIND, e.target_kind);
    b.push_slot_always(VT_ENT_TARGET_INDEX, e.target_index);
    WIPOffset::new(b.end_table(tab).value())
}

fn chat_option_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    o: &ChatOptionInput<'_>,
) -> WIPOffset<ChatOptionReader<'b>> {
    let text_off = b.create_string(o.text);
    let tab = b.start_table();
    b.push_slot_always(VT_CHAT_OPT_TEXT, text_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn chat_line_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    l: &ChatLineInput<'_>,
) -> WIPOffset<ChatLineReader<'b>> {
    let text_off = b.create_string(l.text);
    let username_off = l.username.map(|name| b.create_string(name));
    let tab = b.start_table();
    b.push_slot_always(VT_CL_SEQ, l.seq);
    b.push_slot_always(VT_CL_TEXT, text_off);
    b.push_slot_always(VT_CL_TYPE, l.type_);
    if let Some(off) = username_off {
        b.push_slot_always(VT_CL_USERNAME, off);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn widget_text_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    w: &WidgetTextInput<'_>,
) -> WIPOffset<WidgetTextReader<'b>> {
    let text_off = b.create_string(w.text);
    let tab = b.start_table();
    b.push_slot_always(VT_WT_COMPONENT, w.component_id);
    b.push_slot_always(VT_WT_TEXT, text_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn quest_status_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    q: &QuestStatusInput<'_>,
) -> WIPOffset<QuestStatusReader<'b>> {
    let name_off = b.create_string(q.name);
    let status_off = b.create_string(q.status);
    let tab = b.start_table();
    b.push_slot_always(VT_QUEST_NAME, name_off);
    b.push_slot_always(VT_QUEST_STATUS, status_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn npc_box_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    row: &NpcBoxInput,
) -> WIPOffset<NpcBoxReader<'b>> {
    let points = row
        .points
        .iter()
        .flat_map(|&(x, y)| [x, y])
        .collect::<Vec<_>>();
    let points_off = b.create_vector(&points);
    let tab = b.start_table();
    b.push_slot_always(VT_NPC_BOX_INDEX, row.index);
    b.push_slot_always(VT_NPC_BOX_POINTS, points_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn side_tab_iface_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    t: SideTabIfaceInput,
) -> WIPOffset<SideTabIfaceReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_STI_INDEX, t.index);
    b.push_slot_always(VT_STI_ID, t.id);
    WIPOffset::new(b.end_table(tab).value())
}

fn make_button_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    btn: &MakeButtonInput,
) -> WIPOffset<MakeButtonReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_MAKE_BTN_QTY, btn.qty);
    b.push_slot_always(VT_MAKE_BTN_COM, btn.com_id);
    WIPOffset::new(b.end_table(tab).value())
}

fn make_product_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    p: &MakeProductInput<'_>,
) -> WIPOffset<MakeProductReader<'b>> {
    let name_off = b.create_string(p.name);
    let btn_offs = p
        .buttons
        .iter()
        .map(|btn| make_button_off(b, btn))
        .collect::<Vec<_>>();
    let buttons_off = b.create_vector(&btn_offs);
    let tab = b.start_table();
    b.push_slot_always(VT_MAKE_PROD_OID, p.object_id);
    b.push_slot_always(VT_MAKE_PROD_NAME, name_off);
    b.push_slot_always(VT_MAKE_PROD_BTNS, buttons_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn combat_style_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    c: &CombatStyleInput<'_>,
) -> WIPOffset<CombatStyleReader<'b>> {
    let label_off = b.create_string(c.label);
    let tab = b.start_table();
    b.push_slot_always(VT_CS_MODE, c.mode);
    b.push_slot_always(VT_CS_LABEL, label_off);
    b.push_slot_always(VT_CS_COMPONENT, c.component_id);
    WIPOffset::new(b.end_table(tab).value())
}

fn varp_off<'b>(b: &mut FlatBufferBuilder<'b>, v: &VarpInput) -> WIPOffset<VarpReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_VARP_INDEX, v.index);
    b.push_slot_always(VT_VARP_VALUE, v.value);
    WIPOffset::new(b.end_table(tab).value())
}

fn bank_stand_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    s: &BankStandInput<'_>,
) -> WIPOffset<BankStandReader<'b>> {
    let name_off = b.create_string(s.name);
    let kind_off = b.create_string(s.kind);
    let choose_off = s.choose.map(|c| b.create_string(c));
    let tab = b.start_table();
    b.push_slot_always(VT_BANK_NAME, name_off);
    b.push_slot_always(VT_BANK_X, s.x);
    b.push_slot_always(VT_BANK_Z, s.z);
    b.push_slot_always(VT_BANK_LEVEL, s.level);
    b.push_slot_always(VT_BANK_KIND, kind_off);
    b.push_slot_always(VT_BANK_OP, s.op);
    if let Some(off) = choose_off {
        b.push_slot_always(VT_BANK_CHOOSE, off);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn nearest_booth_table_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    s: &NearestBoothInput<'_>,
) -> WIPOffset<NearestBoothReader<'b>> {
    let name_off = b.create_string(s.name);
    let op_off = b.create_string(s.op);
    let tab = b.start_table();
    b.push_slot_always(VT_NEAREST_X, s.x);
    b.push_slot_always(VT_NEAREST_Z, s.z);
    b.push_slot_always(VT_NEAREST_LEVEL, s.level);
    b.push_slot_always(VT_NEAREST_NAME, name_off);
    b.push_slot_always(VT_NEAREST_OP, op_off);
    b.push_slot_always(VT_NEAREST_ID, s.id);
    WIPOffset::new(b.end_table(tab).value())
}

/// One scene entity view as decoded.
#[derive(Clone, Copy)]
pub struct SceneEntityReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for SceneEntityReader<'a> {
    type Inner = SceneEntityReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl SceneEntityReader<'_> {
    pub fn index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_INDEX, None) }.unwrap_or(0)
    }
    pub fn id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_ID, None) }.unwrap_or(0)
    }
    pub fn name(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_ENT_NAME, None) }
    }
    pub fn x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_X, None) }.unwrap_or(0)
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_LEVEL, None) }.unwrap_or(0)
    }
    pub fn distance(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_DISTANCE, None) }.unwrap_or(0)
    }
    pub fn health(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_HEALTH, None) }.unwrap_or(-1)
    }
    pub fn max_health(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_MAX_HEALTH, None) }.unwrap_or(-1)
    }
    pub fn in_combat(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_ENT_IN_COMBAT, None) }.unwrap_or(false)
    }
    pub fn animating(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_ENT_ANIMATING, None) }.unwrap_or(false)
    }
    pub fn actions(&self) -> Vec<&str> {
        match unsafe {
            self.tab
                .get::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(VT_ENT_ACTIONS, None)
        } {
            Some(v) => v.iter().collect(),
            None => Vec::new(),
        }
    }
    pub fn reachable(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_ENT_REACHABLE, None) }.unwrap_or(false)
    }
    pub fn reachable_adj(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_ENT_REACHABLE_ADJ, None) }.unwrap_or(false)
    }
    pub fn combat_level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_COMBAT_LEVEL, None) }.unwrap_or(0)
    }
    pub fn target_kind(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_TARGET_KIND, None) }.unwrap_or(0)
    }
    pub fn target_index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_ENT_TARGET_INDEX, None) }.unwrap_or(-1)
    }
}

impl Verifiable for SceneEntityReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_ENT_INDEX, false)?
            .visit_field::<i32>("id", VT_ENT_ID, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_ENT_NAME, false)?
            .visit_field::<i32>("x", VT_ENT_X, false)?
            .visit_field::<i32>("z", VT_ENT_Z, false)?
            .visit_field::<i32>("level", VT_ENT_LEVEL, false)?
            .visit_field::<i32>("distance", VT_ENT_DISTANCE, false)?
            .visit_field::<i32>("health", VT_ENT_HEALTH, false)?
            .visit_field::<i32>("max_health", VT_ENT_MAX_HEALTH, false)?
            .visit_field::<bool>("in_combat", VT_ENT_IN_COMBAT, false)?
            .visit_field::<bool>("animating", VT_ENT_ANIMATING, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(
                "actions",
                VT_ENT_ACTIONS,
                false,
            )?
            .visit_field::<bool>("reachable", VT_ENT_REACHABLE, false)?
            .visit_field::<bool>("reachable_adj", VT_ENT_REACHABLE_ADJ, false)?
            .visit_field::<i32>("combat_level", VT_ENT_COMBAT_LEVEL, false)?
            .visit_field::<i32>("target_kind", VT_ENT_TARGET_KIND, false)?
            .visit_field::<i32>("target_index", VT_ENT_TARGET_INDEX, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct ChatOptionReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for ChatOptionReader<'a> {
    type Inner = ChatOptionReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl ChatOptionReader<'_> {
    pub fn text(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_CHAT_OPT_TEXT, None)
        }
        .unwrap_or("")
    }
}

impl Verifiable for ChatOptionReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("text", VT_CHAT_OPT_TEXT, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct SideTabIfaceReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for SideTabIfaceReader<'a> {
    type Inner = SideTabIfaceReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl SideTabIfaceReader<'_> {
    pub fn index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STI_INDEX, None) }.unwrap_or(0)
    }
    pub fn id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STI_ID, None) }.unwrap_or(-1)
    }
}

impl Verifiable for SideTabIfaceReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_STI_INDEX, false)?
            .visit_field::<i32>("id", VT_STI_ID, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct ChatLineReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for ChatLineReader<'a> {
    type Inner = ChatLineReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl ChatLineReader<'_> {
    pub fn seq(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_CL_SEQ, None) }.unwrap_or(0)
    }
    pub fn text(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_CL_TEXT, None) }.unwrap_or("")
    }
    pub fn type_(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_CL_TYPE, None) }.unwrap_or(0)
    }
    pub fn username(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_CL_USERNAME, None) }
    }
}

impl Verifiable for ChatLineReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("seq", VT_CL_SEQ, false)?
            .visit_field::<ForwardsUOffset<&str>>("text", VT_CL_TEXT, false)?
            .visit_field::<i32>("type", VT_CL_TYPE, false)?
            .visit_field::<ForwardsUOffset<&str>>("username", VT_CL_USERNAME, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct WidgetTextReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for WidgetTextReader<'a> {
    type Inner = WidgetTextReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl WidgetTextReader<'_> {
    pub fn component_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_WT_COMPONENT, None) }.unwrap_or(0)
    }
    pub fn text(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_WT_TEXT, None) }.unwrap_or("")
    }
}

impl Verifiable for WidgetTextReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("component_id", VT_WT_COMPONENT, false)?
            .visit_field::<ForwardsUOffset<&str>>("text", VT_WT_TEXT, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct QuestStatusReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for QuestStatusReader<'a> {
    type Inner = QuestStatusReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl QuestStatusReader<'_> {
    pub fn name(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_QUEST_NAME, None) }.unwrap_or("")
    }
    pub fn status(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_QUEST_STATUS, None) }.unwrap_or("unknown")
    }
}

impl Verifiable for QuestStatusReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_QUEST_NAME, false)?
            .visit_field::<ForwardsUOffset<&str>>("status", VT_QUEST_STATUS, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct NpcBoxReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for NpcBoxReader<'a> {
    type Inner = NpcBoxReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl NpcBoxReader<'_> {
    pub fn index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_NPC_BOX_INDEX, None) }.unwrap_or(-1)
    }

    pub fn points(&self) -> Vec<(i32, i32)> {
        let values = match unsafe {
            self.tab
                .get::<ForwardsUOffset<Vector<i32>>>(VT_NPC_BOX_POINTS, None)
        } {
            Some(values) if values.len() == 16 => values,
            _ => return Vec::new(),
        };
        (0..8)
            .map(|i| (values.get(i * 2), values.get(i * 2 + 1)))
            .collect()
    }
}

impl Verifiable for NpcBoxReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_NPC_BOX_INDEX, false)?
            .visit_field::<ForwardsUOffset<Vector<i32>>>("points", VT_NPC_BOX_POINTS, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct MakeButtonReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for MakeButtonReader<'a> {
    type Inner = MakeButtonReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl MakeButtonReader<'_> {
    pub fn qty(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_MAKE_BTN_QTY, None) }.unwrap_or(0)
    }
    pub fn com_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_MAKE_BTN_COM, None) }.unwrap_or(-1)
    }
}

impl Verifiable for MakeButtonReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("qty", VT_MAKE_BTN_QTY, false)?
            .visit_field::<i32>("com_id", VT_MAKE_BTN_COM, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct MakeProductReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for MakeProductReader<'a> {
    type Inner = MakeProductReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl MakeProductReader<'_> {
    pub fn object_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_MAKE_PROD_OID, None) }.unwrap_or(-1)
    }
    pub fn name(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_MAKE_PROD_NAME, None)
        }
        .unwrap_or("")
    }
    pub fn buttons(&self) -> Vec<MakeButtonReader<'_>> {
        rows::<MakeButtonReader>(&self.tab, VT_MAKE_PROD_BTNS)
    }
}

impl Verifiable for MakeProductReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("object_id", VT_MAKE_PROD_OID, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_MAKE_PROD_NAME, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<MakeButtonReader>>>>(
                "buttons",
                VT_MAKE_PROD_BTNS,
                false,
            )?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct CombatStyleReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for CombatStyleReader<'a> {
    type Inner = CombatStyleReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl CombatStyleReader<'_> {
    pub fn mode(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_CS_MODE, None) }.unwrap_or(0)
    }
    pub fn label(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_CS_LABEL, None) }.unwrap_or("")
    }
    pub fn component_id(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_CS_COMPONENT, None) }.unwrap_or(0)
    }
}

impl Verifiable for CombatStyleReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("mode", VT_CS_MODE, false)?
            .visit_field::<ForwardsUOffset<&str>>("label", VT_CS_LABEL, false)?
            .visit_field::<i32>("component_id", VT_CS_COMPONENT, false)?
            .finish();
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct VarpReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for VarpReader<'a> {
    type Inner = VarpReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl VarpReader<'_> {
    pub fn index(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_VARP_INDEX, None) }.unwrap_or(0)
    }
    pub fn value(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_VARP_VALUE, None) }.unwrap_or(0)
    }
}

impl Verifiable for VarpReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_VARP_INDEX, false)?
            .visit_field::<i32>("value", VT_VARP_VALUE, false)?
            .finish();
        Ok(())
    }
}

/// One shim interact request as decoded: the tagged `op` string plus the
/// request's fields (absent when the request has none).
pub struct InteractReader<'a> {
    tab: Table<'a>,
}

impl<'a> flatbuffers::Follow<'a> for InteractReader<'a> {
    type Inner = InteractReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl InteractReader<'_> {
    pub fn op(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_IN_OP, None) }
    }
    pub fn x(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_IN_X, None) }.unwrap_or(0)
    }
    pub fn required_x(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_X, None) }
    }
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_IN_Z, None) }.unwrap_or(0)
    }
    pub fn required_z(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_Z, None) }
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_IN_LEVEL, None) }.unwrap_or(0)
    }
    pub fn required_level(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_LEVEL, None) }
    }
    pub fn kind(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_IN_KIND, None) }
    }
    pub fn name(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_IN_NAME, None) }
    }
    pub fn stand_op(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_STAND_OP, None) }
    }
    pub fn choose(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_IN_CHOOSE, None) }
    }
    pub fn action(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_IN_ACTION, None) }
    }
    pub fn index(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_INDEX, None) }
    }
    pub fn component_id(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_COMPONENT_ID, None) }
    }
    pub fn bank_generation(&self) -> Option<u64> {
        unsafe { self.tab.get::<u64>(VT_IN_BANK_GENERATION, None) }
    }
    pub fn bank_item_id(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_BANK_ITEM_ID, None) }
    }
    pub fn lands_as_id(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_LANDS_AS_ID, None) }
    }
    pub fn source_item_id(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_SOURCE_ITEM_ID, None) }
    }
    pub fn source_item_slot(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_SOURCE_ITEM_SLOT, None) }
    }
    pub fn target_item_id(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_TARGET_ITEM_ID, None) }
    }
    pub fn target_item_slot(&self) -> Option<i32> {
        unsafe { self.tab.get::<i32>(VT_IN_TARGET_ITEM_SLOT, None) }
    }
    pub fn request_id(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_IN_REQUEST_ID, None) }.unwrap_or(0)
    }
    pub fn xf(&self) -> Option<f64> {
        unsafe { self.tab.get::<f64>(VT_IN_XF, None) }
    }
    pub fn yf(&self) -> Option<f64> {
        unsafe { self.tab.get::<f64>(VT_IN_YF, None) }
    }
    pub fn input_identity(&self) -> u64 {
        unsafe { self.tab.get::<u64>(VT_IN_INPUT_IDENTITY, None) }.unwrap_or(0)
    }
    pub fn allow_wilderness(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_IN_ALLOW_WILDERNESS, None) }.unwrap_or(false)
    }
    pub fn allow_bank_fetch(&self) -> bool {
        unsafe { self.tab.get::<bool>(VT_IN_ALLOW_BANK_FETCH, None) }.unwrap_or(false)
    }
}

impl Verifiable for InteractReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("op", VT_IN_OP, false)?
            .visit_field::<i32>("x", VT_IN_X, false)?
            .visit_field::<i32>("z", VT_IN_Z, false)?
            .visit_field::<i32>("level", VT_IN_LEVEL, false)?
            .visit_field::<ForwardsUOffset<&str>>("kind", VT_IN_KIND, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_IN_NAME, false)?
            .visit_field::<i32>("stand_op", VT_IN_STAND_OP, false)?
            .visit_field::<ForwardsUOffset<&str>>("choose", VT_IN_CHOOSE, false)?
            .visit_field::<ForwardsUOffset<&str>>("action", VT_IN_ACTION, false)?
            .visit_field::<i32>("index", VT_IN_INDEX, false)?
            .visit_field::<i32>("component_id", VT_IN_COMPONENT_ID, false)?
            .visit_field::<u64>("bank_generation", VT_IN_BANK_GENERATION, false)?
            .visit_field::<i32>("bank_item_id", VT_IN_BANK_ITEM_ID, false)?
            .visit_field::<i32>("lands_as_id", VT_IN_LANDS_AS_ID, false)?
            .visit_field::<i32>("source_item_id", VT_IN_SOURCE_ITEM_ID, false)?
            .visit_field::<i32>("source_item_slot", VT_IN_SOURCE_ITEM_SLOT, false)?
            .visit_field::<i32>("target_item_id", VT_IN_TARGET_ITEM_ID, false)?
            .visit_field::<i32>("target_item_slot", VT_IN_TARGET_ITEM_SLOT, false)?
            .visit_field::<u64>("request_id", VT_IN_REQUEST_ID, false)?
            .visit_field::<f64>("xf", VT_IN_XF, false)?
            .visit_field::<f64>("yf", VT_IN_YF, false)?
            .visit_field::<u64>("input_identity", VT_IN_INPUT_IDENTITY, false)?
            .visit_field::<bool>("allow_wilderness", VT_IN_ALLOW_WILDERNESS, false)?
            .visit_field::<bool>("allow_bank_fetch", VT_IN_ALLOW_BANK_FETCH, false)?
            .finish();
        Ok(())
    }
}

/// A batch of shim interact requests as decoded.
pub struct InteractBatchReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for InteractBatchReader<'a> {
    type Inner = InteractBatchReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for InteractBatchReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<InteractReader>>>>(
                "reqs", VT_REQS, false,
            )?
            .finish();
        Ok(())
    }
}

impl InteractBatchReader<'_> {
    /// Interpret `buf` as a root-`InteractBatch` FlatBuffer after verification.
    pub fn from_bytes(buf: &[u8]) -> Result<InteractBatchReader<'_>, String> {
        verified_root::<InteractBatchReader>(buf)
    }

    pub fn reqs(&self) -> Result<Vec<InteractReader<'_>>, String> {
        rows_capped::<InteractReader>(&self.tab, VT_REQS, MAX_INTERACT_REQS)
    }
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
    let tab = b.start_table();
    b.push_slot_always(VT_REQS, reqs_off);
    let root = b.end_table(tab);
    b.finish(root, None);
}

fn paint_button_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    btn: &crate::shim::ScriptPaintButton,
) -> WIPOffset<PaintButtonReader<'b>> {
    let id_off = b.create_string(&btn.id);
    let label_off = b.create_string(&btn.label);
    let tab = b.start_table();
    b.push_slot_always(VT_PAINT_BTN_ID, id_off);
    b.push_slot_always(VT_PAINT_BTN_LABEL, label_off);
    WIPOffset::new(b.end_table(tab).value())
}

fn canvas_seg_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    seg: &crate::canvas::PathSeg,
) -> WIPOffset<CanvasSegReader<'b>> {
    use crate::canvas::PathSeg;
    let (kind, x, y, c1x, c1y, c2x, c2y) = match *seg {
        PathSeg::MoveTo { x, y } => (0i8, x, y, 0.0, 0.0, 0.0, 0.0),
        PathSeg::LineTo { x, y } => (1i8, x, y, 0.0, 0.0, 0.0, 0.0),
        PathSeg::QuadTo { cx, cy, x, y } => (2i8, x, y, cx, cy, 0.0, 0.0),
        PathSeg::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        } => (3i8, x, y, c1x, c1y, c2x, c2y),
        PathSeg::Close => (4i8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    };
    let tab = b.start_table();
    b.push_slot_always(VT_SEG_KIND, kind);
    b.push_slot_always(VT_SEG_X, x);
    b.push_slot_always(VT_SEG_Y, y);
    if c1x != 0.0 {
        b.push_slot_always(VT_SEG_C1X, c1x);
    }
    if c1y != 0.0 {
        b.push_slot_always(VT_SEG_C1Y, c1y);
    }
    if c2x != 0.0 {
        b.push_slot_always(VT_SEG_C2X, c2x);
    }
    if c2y != 0.0 {
        b.push_slot_always(VT_SEG_C2Y, c2y);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn canvas_segs_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    segs: &[crate::canvas::PathSeg],
) -> Option<WIPOffset<flatbuffers::Vector<'b, ForwardsUOffset<CanvasSegReader<'b>>>>> {
    if segs.is_empty() {
        return None;
    }
    let offs: Vec<_> = segs.iter().map(|s| canvas_seg_off(b, s)).collect();
    Some(b.create_vector(&offs))
}

fn clip_path_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    clip: &crate::canvas::ClipPath,
) -> WIPOffset<ClipPathFbReader<'b>> {
    let segs = canvas_segs_off(b, &clip.segs);
    let tab = b.start_table();
    if let Some(off) = segs {
        b.push_slot_always(VT_CLIP_SEGS, off);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn canvas_clips_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    clips: &[crate::canvas::ClipPath],
) -> Option<WIPOffset<flatbuffers::Vector<'b, ForwardsUOffset<ClipPathFbReader<'b>>>>> {
    if clips.is_empty() {
        return None;
    }
    let offs: Vec<_> = clips.iter().map(|c| clip_path_off(b, c)).collect();
    Some(b.create_vector(&offs))
}

fn grad_stop_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    stop: &crate::canvas::GradStop,
) -> WIPOffset<GradStopReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_GSTOP_OFFSET, stop.offset);
    b.push_slot_always(VT_GSTOP_COLOR, stop.color);
    WIPOffset::new(b.end_table(tab).value())
}

struct EncodedFill {
    kind: i8,
    gx0: f32,
    gy0: f32,
    gx1: f32,
    gy1: f32,
    r0: f32,
    r1: f32,
    stops: Vec<crate::canvas::GradStop>,
}

fn encode_fill(fill: &crate::canvas::FillPaint) -> EncodedFill {
    match fill {
        crate::canvas::FillPaint::Solid => EncodedFill {
            kind: 0,
            gx0: 0.0,
            gy0: 0.0,
            gx1: 0.0,
            gy1: 0.0,
            r0: 0.0,
            r1: 0.0,
            stops: Vec::new(),
        },
        crate::canvas::FillPaint::Linear {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => EncodedFill {
            kind: 1,
            gx0: *x0,
            gy0: *y0,
            gx1: *x1,
            gy1: *y1,
            r0: 0.0,
            r1: 0.0,
            stops: stops.clone(),
        },
        crate::canvas::FillPaint::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            stops,
        } => EncodedFill {
            kind: 2,
            gx0: *x0,
            gy0: *y0,
            gx1: *x1,
            gy1: *y1,
            r0: *r0,
            r1: *r1,
            stops: stops.clone(),
        },
    }
}

fn canvas_op_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    op: &crate::canvas::CanvasOp,
) -> WIPOffset<CanvasOpReader<'b>> {
    use crate::canvas::{CanvasOp, LineJoinKind, TextAlign, TextBaseline};
    let (
        kind,
        x,
        y,
        w,
        h,
        color,
        text,
        font_px,
        mono,
        segs,
        extras,
        line_width,
        line_join,
        align,
        baseline,
    ) = match op {
        CanvasOp::FillRect {
            x,
            y,
            w,
            h,
            color,
            extras,
        } => (
            0i8,
            *x,
            *y,
            *w,
            *h,
            *color,
            None,
            0u16,
            false,
            None,
            extras,
            0.0f32,
            LineJoinKind::Miter,
            TextAlign::Left,
            TextBaseline::Alphabetic,
        ),
        CanvasOp::FillText {
            text,
            x,
            y,
            color,
            font_px,
            mono,
            align,
            baseline,
            extras,
        } => (
            1i8,
            *x,
            *y,
            0,
            0,
            *color,
            Some(text.as_str()),
            *font_px,
            *mono,
            None,
            extras,
            0.0,
            LineJoinKind::Miter,
            *align,
            *baseline,
        ),
        CanvasOp::FillPath {
            segs,
            color,
            extras,
        } => (
            2i8,
            0,
            0,
            0,
            0,
            *color,
            None,
            0,
            false,
            Some(segs.as_slice()),
            extras,
            0.0,
            LineJoinKind::Miter,
            TextAlign::Left,
            TextBaseline::Alphabetic,
        ),
        CanvasOp::StrokePath {
            segs,
            color,
            line_width,
            line_join,
            extras,
        } => (
            3i8,
            0,
            0,
            0,
            0,
            *color,
            None,
            0,
            false,
            Some(segs.as_slice()),
            extras,
            *line_width,
            *line_join,
            TextAlign::Left,
            TextBaseline::Alphabetic,
        ),
    };
    let fill = encode_fill(&extras.fill);
    let segs_off = segs.and_then(|s| canvas_segs_off(b, s));
    let clips_off = canvas_clips_off(b, &extras.clips);
    let stops_off = if fill.stops.is_empty() {
        None
    } else {
        let offs: Vec<_> = fill.stops.iter().map(|s| grad_stop_off(b, s)).collect();
        Some(b.create_vector(&offs))
    };
    let text_off = text.map(|s| b.create_string(s));
    let tab = b.start_table();
    b.push_slot_always(VT_CANVAS_KIND, kind);
    if x != 0 {
        b.push_slot_always(VT_CANVAS_X, x);
    }
    if y != 0 {
        b.push_slot_always(VT_CANVAS_Y, y);
    }
    if w != 0 {
        b.push_slot_always(VT_CANVAS_W, w);
    }
    if h != 0 {
        b.push_slot_always(VT_CANVAS_H, h);
    }
    if color != 0 {
        b.push_slot_always(VT_CANVAS_COLOR, color);
    }
    if let Some(off) = text_off {
        b.push_slot_always(VT_CANVAS_TEXT, off);
    }
    if font_px != 0 {
        b.push_slot_always(VT_CANVAS_FONT_PX, font_px);
    }
    if mono {
        b.push_slot_always(VT_CANVAS_MONO, mono);
    }
    if let Some(off) = segs_off {
        b.push_slot_always(VT_CANVAS_SEGS, off);
    }
    if let Some(off) = clips_off {
        b.push_slot_always(VT_CANVAS_CLIPS, off);
    }
    if let Some(off) = stops_off {
        b.push_slot_always(VT_CANVAS_STOPS, off);
    }
    if fill.gx0 != 0.0 {
        b.push_slot_always(VT_CANVAS_GX0, fill.gx0);
    }
    if fill.gy0 != 0.0 {
        b.push_slot_always(VT_CANVAS_GY0, fill.gy0);
    }
    if fill.gx1 != 0.0 {
        b.push_slot_always(VT_CANVAS_GX1, fill.gx1);
    }
    if fill.gy1 != 0.0 {
        b.push_slot_always(VT_CANVAS_GY1, fill.gy1);
    }
    if fill.r0 != 0.0 {
        b.push_slot_always(VT_CANVAS_R0, fill.r0);
    }
    if fill.r1 != 0.0 {
        b.push_slot_always(VT_CANVAS_R1, fill.r1);
    }
    if fill.kind != 0 {
        b.push_slot_always(VT_CANVAS_GRAD_KIND, fill.kind);
    }
    if line_width != 0.0 {
        b.push_slot_always(VT_CANVAS_LINE_WIDTH, line_width);
    }
    if !matches!(line_join, LineJoinKind::Miter) {
        b.push_slot_always(VT_CANVAS_LINE_JOIN, line_join as i8);
    }
    if extras.shadow.color != 0 {
        b.push_slot_always(VT_CANVAS_SHADOW_COLOR, extras.shadow.color);
    }
    if extras.shadow.blur != 0.0 {
        b.push_slot_always(VT_CANVAS_SHADOW_BLUR, extras.shadow.blur);
    }
    if extras.shadow.offset_x != 0.0 {
        b.push_slot_always(VT_CANVAS_SHADOW_X, extras.shadow.offset_x);
    }
    if extras.shadow.offset_y != 0.0 {
        b.push_slot_always(VT_CANVAS_SHADOW_Y, extras.shadow.offset_y);
    }
    if !matches!(align, TextAlign::Left) {
        b.push_slot_always(VT_CANVAS_ALIGN, align as i8);
    }
    if !matches!(baseline, TextBaseline::Alphabetic) {
        b.push_slot_always(VT_CANVAS_BASELINE, baseline as i8);
    }
    WIPOffset::new(b.end_table(tab).value())
}

fn encode_paint_into(b: &mut FlatBufferBuilder<'_>, paint: &crate::shim::ScriptPaint) {
    let title_off = paint.title.as_deref().map(|s| b.create_string(s));
    let accent_off = paint.accent.as_deref().map(|s| b.create_string(s));
    let line_offs: Vec<_> = paint.lines.iter().map(|s| b.create_string(s)).collect();
    let lines_off = b.create_vector(&line_offs);
    let btn_offs: Vec<_> = paint
        .buttons
        .iter()
        .map(|btn| paint_button_off(b, btn))
        .collect();
    let buttons_off = if btn_offs.is_empty() {
        None
    } else {
        Some(b.create_vector(&btn_offs))
    };
    let canvas_offs: Vec<_> = paint.canvas.iter().map(|op| canvas_op_off(b, op)).collect();
    let canvas_off = if canvas_offs.is_empty() {
        None
    } else {
        Some(b.create_vector(&canvas_offs))
    };
    let tab = b.start_table();
    if let Some(off) = title_off {
        b.push_slot_always(VT_PAINT_TITLE, off);
    }
    if let Some(off) = accent_off {
        b.push_slot_always(VT_PAINT_ACCENT, off);
    }
    b.push_slot_always(VT_PAINT_LINES, lines_off);
    if let Some(off) = buttons_off {
        b.push_slot_always(VT_PAINT_BUTTONS, off);
    }
    if let Some(off) = canvas_off {
        b.push_slot_always(VT_PAINT_CANVAS, off);
    }
    let root = b.end_table(tab);
    b.finish(root, None);
}

/// One recorded paint frame as decoded.
pub struct PaintReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for PaintReader<'a> {
    type Inner = PaintReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for PaintReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("title", VT_PAINT_TITLE, false)?
            .visit_field::<ForwardsUOffset<&str>>("accent", VT_PAINT_ACCENT, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(
                "lines",
                VT_PAINT_LINES,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<PaintButtonReader>>>>(
                "buttons",
                VT_PAINT_BUTTONS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<CanvasOpReader>>>>(
                "canvas",
                VT_PAINT_CANVAS,
                false,
            )?
            .finish();
        Ok(())
    }
}

/// One advertised `{id,label}` paint control as decoded.
struct PaintButtonReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for PaintButtonReader<'a> {
    type Inner = PaintButtonReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for PaintButtonReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("id", VT_PAINT_BTN_ID, false)?
            .visit_field::<ForwardsUOffset<&str>>("label", VT_PAINT_BTN_LABEL, false)?
            .finish();
        Ok(())
    }
}

impl PaintButtonReader<'_> {
    fn id(&self) -> &str {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_PAINT_BTN_ID, None) }.unwrap_or("")
    }
    fn label(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_PAINT_BTN_LABEL, None)
        }
        .unwrap_or("")
    }
}

struct CanvasSegReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for CanvasSegReader<'a> {
    type Inner = CanvasSegReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for CanvasSegReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i8>("kind", VT_SEG_KIND, false)?
            .visit_field::<f32>("x", VT_SEG_X, false)?
            .visit_field::<f32>("y", VT_SEG_Y, false)?
            .visit_field::<f32>("c1x", VT_SEG_C1X, false)?
            .visit_field::<f32>("c1y", VT_SEG_C1Y, false)?
            .visit_field::<f32>("c2x", VT_SEG_C2X, false)?
            .visit_field::<f32>("c2y", VT_SEG_C2Y, false)?
            .finish();
        Ok(())
    }
}

impl CanvasSegReader<'_> {
    fn into_seg(&self) -> Result<crate::canvas::PathSeg, String> {
        let kind = unsafe { self.tab.get::<i8>(VT_SEG_KIND, None) }.unwrap_or(0);
        let x = unsafe { self.tab.get::<f32>(VT_SEG_X, None) }.unwrap_or(0.0);
        let y = unsafe { self.tab.get::<f32>(VT_SEG_Y, None) }.unwrap_or(0.0);
        let c1x = unsafe { self.tab.get::<f32>(VT_SEG_C1X, None) }.unwrap_or(0.0);
        let c1y = unsafe { self.tab.get::<f32>(VT_SEG_C1Y, None) }.unwrap_or(0.0);
        let c2x = unsafe { self.tab.get::<f32>(VT_SEG_C2X, None) }.unwrap_or(0.0);
        let c2y = unsafe { self.tab.get::<f32>(VT_SEG_C2Y, None) }.unwrap_or(0.0);
        match kind {
            0 => Ok(crate::canvas::PathSeg::MoveTo { x, y }),
            1 => Ok(crate::canvas::PathSeg::LineTo { x, y }),
            2 => Ok(crate::canvas::PathSeg::QuadTo {
                cx: c1x,
                cy: c1y,
                x,
                y,
            }),
            3 => Ok(crate::canvas::PathSeg::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            }),
            4 => Ok(crate::canvas::PathSeg::Close),
            other => Err(format!("unknown canvas seg kind {other}")),
        }
    }
}

struct GradStopReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for GradStopReader<'a> {
    type Inner = GradStopReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for GradStopReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<f32>("offset", VT_GSTOP_OFFSET, false)?
            .visit_field::<u32>("color", VT_GSTOP_COLOR, false)?
            .finish();
        Ok(())
    }
}

impl GradStopReader<'_> {
    fn into_stop(&self) -> Result<crate::canvas::GradStop, String> {
        let offset = unsafe { self.tab.get::<f32>(VT_GSTOP_OFFSET, None) }.unwrap_or(0.0);
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
            return Err("canvas gradient stop offset out of range".into());
        }
        let color = unsafe { self.tab.get::<u32>(VT_GSTOP_COLOR, None) }.unwrap_or(0);
        Ok(crate::canvas::GradStop { offset, color })
    }
}

struct ClipPathFbReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for ClipPathFbReader<'a> {
    type Inner = ClipPathFbReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for ClipPathFbReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<CanvasSegReader>>>>(
                "segs",
                VT_CLIP_SEGS,
                false,
            )?
            .finish();
        Ok(())
    }
}

impl ClipPathFbReader<'_> {
    fn into_clip(&self) -> Result<crate::canvas::ClipPath, String> {
        let rows = rows_capped::<CanvasSegReader>(&self.tab, VT_CLIP_SEGS, MAX_PATH_SEGS_PER_OP)?;
        if rows.len() > MAX_PATH_SEGS_PER_OP {
            return Err(format!(
                "canvas clip segs {} exceeds cap {MAX_PATH_SEGS_PER_OP}",
                rows.len()
            ));
        }
        Ok(crate::canvas::ClipPath {
            segs: rows
                .into_iter()
                .map(|r| r.into_seg())
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

struct CanvasOpReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for CanvasOpReader<'a> {
    type Inner = CanvasOpReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for CanvasOpReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i8>("kind", VT_CANVAS_KIND, false)?
            .visit_field::<i32>("x", VT_CANVAS_X, false)?
            .visit_field::<i32>("y", VT_CANVAS_Y, false)?
            .visit_field::<i32>("w", VT_CANVAS_W, false)?
            .visit_field::<i32>("h", VT_CANVAS_H, false)?
            .visit_field::<u32>("color", VT_CANVAS_COLOR, false)?
            .visit_field::<ForwardsUOffset<&str>>("text", VT_CANVAS_TEXT, false)?
            .visit_field::<u16>("font_px", VT_CANVAS_FONT_PX, false)?
            .visit_field::<bool>("mono", VT_CANVAS_MONO, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<CanvasSegReader>>>>(
                "segs",
                VT_CANVAS_SEGS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<ClipPathFbReader>>>>(
                "clips",
                VT_CANVAS_CLIPS,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<GradStopReader>>>>(
                "stops",
                VT_CANVAS_STOPS,
                false,
            )?
            .visit_field::<f32>("gx0", VT_CANVAS_GX0, false)?
            .visit_field::<f32>("gy0", VT_CANVAS_GY0, false)?
            .visit_field::<f32>("gx1", VT_CANVAS_GX1, false)?
            .visit_field::<f32>("gy1", VT_CANVAS_GY1, false)?
            .visit_field::<f32>("r0", VT_CANVAS_R0, false)?
            .visit_field::<f32>("r1", VT_CANVAS_R1, false)?
            .visit_field::<i8>("grad_kind", VT_CANVAS_GRAD_KIND, false)?
            .visit_field::<f32>("line_width", VT_CANVAS_LINE_WIDTH, false)?
            .visit_field::<i8>("line_join", VT_CANVAS_LINE_JOIN, false)?
            .visit_field::<u32>("shadow_color", VT_CANVAS_SHADOW_COLOR, false)?
            .visit_field::<f32>("shadow_blur", VT_CANVAS_SHADOW_BLUR, false)?
            .visit_field::<f32>("shadow_x", VT_CANVAS_SHADOW_X, false)?
            .visit_field::<f32>("shadow_y", VT_CANVAS_SHADOW_Y, false)?
            .visit_field::<i8>("align", VT_CANVAS_ALIGN, false)?
            .visit_field::<i8>("baseline", VT_CANVAS_BASELINE, false)?
            .finish();
        Ok(())
    }
}

impl CanvasOpReader<'_> {
    fn decode_segs(&self) -> Result<Vec<crate::canvas::PathSeg>, String> {
        let rows = rows_capped::<CanvasSegReader>(&self.tab, VT_CANVAS_SEGS, MAX_PATH_SEGS_PER_OP)?;
        rows.into_iter().map(|r| r.into_seg()).collect()
    }

    fn decode_extras(&self) -> Result<crate::canvas::DrawExtras, String> {
        use crate::canvas::{DrawExtras, FillPaint, Shadow};
        let clip_rows =
            rows_capped::<ClipPathFbReader>(&self.tab, VT_CANVAS_CLIPS, MAX_CLIP_PATHS)?;
        if clip_rows.len() > MAX_CLIP_PATHS {
            return Err(format!(
                "canvas clips {} exceeds cap {MAX_CLIP_PATHS}",
                clip_rows.len()
            ));
        }
        let clips = clip_rows
            .into_iter()
            .map(|r| r.into_clip())
            .collect::<Result<Vec<_>, _>>()?;
        let stop_rows =
            rows_capped::<GradStopReader>(&self.tab, VT_CANVAS_STOPS, MAX_GRADIENT_STOPS)?;
        if stop_rows.len() > MAX_GRADIENT_STOPS {
            return Err(format!(
                "canvas gradient stops {} exceeds cap {MAX_GRADIENT_STOPS}",
                stop_rows.len()
            ));
        }
        let stops = stop_rows
            .into_iter()
            .map(|r| r.into_stop())
            .collect::<Result<Vec<_>, _>>()?;
        let gx0 = unsafe { self.tab.get::<f32>(VT_CANVAS_GX0, None) }.unwrap_or(0.0);
        let gy0 = unsafe { self.tab.get::<f32>(VT_CANVAS_GY0, None) }.unwrap_or(0.0);
        let gx1 = unsafe { self.tab.get::<f32>(VT_CANVAS_GX1, None) }.unwrap_or(0.0);
        let gy1 = unsafe { self.tab.get::<f32>(VT_CANVAS_GY1, None) }.unwrap_or(0.0);
        let r0 = unsafe { self.tab.get::<f32>(VT_CANVAS_R0, None) }.unwrap_or(0.0);
        let r1 = unsafe { self.tab.get::<f32>(VT_CANVAS_R1, None) }.unwrap_or(0.0);
        let grad_kind = unsafe { self.tab.get::<i8>(VT_CANVAS_GRAD_KIND, None) }.unwrap_or(0);
        let fill = match grad_kind {
            0 => FillPaint::Solid,
            1 => FillPaint::Linear {
                x0: gx0,
                y0: gy0,
                x1: gx1,
                y1: gy1,
                stops,
            },
            2 => FillPaint::Radial {
                x0: gx0,
                y0: gy0,
                r0,
                x1: gx1,
                y1: gy1,
                r1,
                stops,
            },
            other => return Err(format!("unknown canvas grad_kind {other}")),
        };
        let blur = unsafe { self.tab.get::<f32>(VT_CANVAS_SHADOW_BLUR, None) }.unwrap_or(0.0);
        if blur > MAX_SHADOW_BLUR {
            return Err(format!(
                "canvas shadow_blur {blur} exceeds cap {MAX_SHADOW_BLUR}"
            ));
        }
        Ok(DrawExtras {
            clips,
            shadow: Shadow {
                color: unsafe { self.tab.get::<u32>(VT_CANVAS_SHADOW_COLOR, None) }.unwrap_or(0),
                blur,
                offset_x: unsafe { self.tab.get::<f32>(VT_CANVAS_SHADOW_X, None) }.unwrap_or(0.0),
                offset_y: unsafe { self.tab.get::<f32>(VT_CANVAS_SHADOW_Y, None) }.unwrap_or(0.0),
            },
            fill,
        })
    }

    fn into_op(&self) -> Result<crate::canvas::CanvasOp, String> {
        use crate::canvas::{CanvasOp, LineJoinKind, TextAlign, TextBaseline};
        let kind = unsafe { self.tab.get::<i8>(VT_CANVAS_KIND, None) }.unwrap_or(0);
        let x = unsafe { self.tab.get::<i32>(VT_CANVAS_X, None) }.unwrap_or(0);
        let y = unsafe { self.tab.get::<i32>(VT_CANVAS_Y, None) }.unwrap_or(0);
        let w = unsafe { self.tab.get::<i32>(VT_CANVAS_W, None) }.unwrap_or(0);
        let h = unsafe { self.tab.get::<i32>(VT_CANVAS_H, None) }.unwrap_or(0);
        let color = unsafe { self.tab.get::<u32>(VT_CANVAS_COLOR, None) }.unwrap_or(0);
        let extras = self.decode_extras()?;
        match kind {
            0 => Ok(CanvasOp::FillRect {
                x,
                y,
                w,
                h,
                color,
                extras,
            }),
            1 => {
                let text = unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_CANVAS_TEXT, None) }
                    .unwrap_or("");
                if text.len() > MAX_PAINT_TEXT {
                    return Err(format!(
                        "canvas text length {} exceeds cap {MAX_PAINT_TEXT}",
                        text.len()
                    ));
                }
                let font_px = unsafe { self.tab.get::<u16>(VT_CANVAS_FONT_PX, None) }.unwrap_or(10);
                if !crate::canvas::font_px_allowed(font_px) {
                    return Err(format!(
                        "canvas font_px {font_px} exceeds cap {}",
                        crate::canvas::MAX_FONT_PX
                    ));
                }
                let mono = unsafe { self.tab.get::<bool>(VT_CANVAS_MONO, None) }.unwrap_or(false);
                let align = match unsafe { self.tab.get::<i8>(VT_CANVAS_ALIGN, None) }.unwrap_or(0)
                {
                    0 => TextAlign::Left,
                    1 => TextAlign::Center,
                    2 => TextAlign::Right,
                    other => return Err(format!("unknown canvas align {other}")),
                };
                let baseline =
                    match unsafe { self.tab.get::<i8>(VT_CANVAS_BASELINE, None) }.unwrap_or(0) {
                        0 => TextBaseline::Alphabetic,
                        1 => TextBaseline::Top,
                        2 => TextBaseline::Middle,
                        3 => TextBaseline::Bottom,
                        other => return Err(format!("unknown canvas baseline {other}")),
                    };
                Ok(CanvasOp::FillText {
                    text: text.to_string(),
                    x,
                    y,
                    color,
                    font_px,
                    mono,
                    align,
                    baseline,
                    extras,
                })
            }
            2 => {
                let segs = self.decode_segs()?;
                if segs.len() > MAX_PATH_SEGS_PER_OP {
                    return Err(format!(
                        "canvas path segs {} exceeds cap {MAX_PATH_SEGS_PER_OP}",
                        segs.len()
                    ));
                }
                Ok(CanvasOp::FillPath {
                    segs,
                    color,
                    extras,
                })
            }
            3 => {
                let segs = self.decode_segs()?;
                if segs.len() > MAX_PATH_SEGS_PER_OP {
                    return Err(format!(
                        "canvas path segs {} exceeds cap {MAX_PATH_SEGS_PER_OP}",
                        segs.len()
                    ));
                }
                let line_width =
                    unsafe { self.tab.get::<f32>(VT_CANVAS_LINE_WIDTH, None) }.unwrap_or(1.0);
                if line_width > MAX_LINE_WIDTH {
                    return Err(format!(
                        "canvas line_width {line_width} exceeds cap {MAX_LINE_WIDTH}"
                    ));
                }
                let line_join =
                    match unsafe { self.tab.get::<i8>(VT_CANVAS_LINE_JOIN, None) }.unwrap_or(0) {
                        0 => LineJoinKind::Miter,
                        1 => LineJoinKind::Round,
                        2 => LineJoinKind::Bevel,
                        other => return Err(format!("unknown canvas line_join {other}")),
                    };
                Ok(CanvasOp::StrokePath {
                    segs,
                    color,
                    line_width,
                    line_join,
                    extras,
                })
            }
            other => Err(format!("unknown canvas op kind {other}")),
        }
    }
}

impl PaintReader<'_> {
    pub fn title(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_PAINT_TITLE, None) }
    }
    pub fn accent(&self) -> Option<&str> {
        unsafe { self.tab.get::<ForwardsUOffset<&str>>(VT_PAINT_ACCENT, None) }
    }
    pub fn lines(&self) -> Result<Vec<String>, String> {
        // Safety: verified before any accessor use.
        let lines = match unsafe {
            self.tab
                .get::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(VT_PAINT_LINES, None)
        } {
            Some(v) => {
                let len = v.len();
                if len > MAX_PAINT_LINES {
                    return Err(format!("vector length {len} exceeds cap {MAX_PAINT_LINES}"));
                }
                v.iter().map(str::to_string).collect()
            }
            None => Vec::new(),
        };
        Ok(lines)
    }
    fn buttons(&self) -> Result<Vec<crate::shim::ScriptPaintButton>, String> {
        let rows =
            rows_capped::<PaintButtonReader>(&self.tab, VT_PAINT_BUTTONS, MAX_PAINT_BUTTONS)?;
        Ok(rows
            .into_iter()
            .map(|row| crate::shim::ScriptPaintButton {
                id: row.id().to_string(),
                label: row.label().to_string(),
            })
            .collect())
    }
    fn canvas(&self) -> Result<Vec<crate::canvas::CanvasOp>, String> {
        let rows = rows_capped::<CanvasOpReader>(&self.tab, VT_PAINT_CANVAS, MAX_CANVAS_OPS)?;
        let mut out = Vec::with_capacity(rows.len());
        let mut segs = 0usize;
        for row in rows {
            let op = row.into_op()?;
            segs = segs.saturating_add(match &op {
                crate::canvas::CanvasOp::FillPath { segs, extras, .. }
                | crate::canvas::CanvasOp::StrokePath { segs, extras, .. } => {
                    segs.len() + extras.clips.iter().map(|c| c.segs.len()).sum::<usize>()
                }
                crate::canvas::CanvasOp::FillRect { extras, .. }
                | crate::canvas::CanvasOp::FillText { extras, .. } => {
                    extras.clips.iter().map(|c| c.segs.len()).sum::<usize>()
                }
            });
            if segs > MAX_PATH_SEGS_PER_FRAME {
                return Err(format!(
                    "canvas path segs {segs} exceeds cap {MAX_PATH_SEGS_PER_FRAME}"
                ));
            }
            out.push(op);
        }
        Ok(out)
    }
}

/// Decode a root-`Paint` FlatBuffer into the shim's recorded frame.
pub fn decode_paint(buf: &[u8]) -> Result<crate::shim::ScriptPaint, String> {
    let paint = verified_root::<PaintReader>(buf)?;
    Ok(crate::shim::ScriptPaint {
        title: paint.title().map(str::to_string),
        accent: paint.accent().map(str::to_string),
        lines: paint.lines()?,
        buttons: paint.buttons()?,
        canvas: paint.canvas()?,
        generation: 0,
    })
}

/// Decode a root-`InteractBatch` into the shim's request type. A row with
/// a missing/unknown `op` (or a request missing a required field) fails
/// the whole batch — the host logs it and drops the batch, never fatal,
/// exactly like the old JSON parse.
pub fn decode_interact_batch(buf: &[u8]) -> Result<Vec<crate::shim::InteractReq>, String> {
    let batch = InteractBatchReader::from_bytes(buf)?;
    let rows = batch.reqs()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let op = row
            .op()
            .ok_or_else(|| "interact row has no op".to_string())?;
        match op {
            "open-booth" => out.push(crate::shim::InteractReq::OpenBooth {
                x: row
                    .required_x()
                    .ok_or_else(|| "open-booth has no x".to_string())?,
                z: row
                    .required_z()
                    .ok_or_else(|| "open-booth has no z".to_string())?,
                level: row
                    .required_level()
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
            }),
            "walk-nearest-bank" => out.push(crate::shim::InteractReq::WalkNearestBank),
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
            other => return Err(format!("unknown interact op: {other}")),
        }
    }
    Ok(out)
}

fn interact_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    req: &crate::shim::InteractReq,
) -> WIPOffset<InteractReader<'b>> {
    use crate::shim::InteractReq;
    let op_off = b.create_string(match req {
        InteractReq::OpenBooth { .. } => "open-booth",
        InteractReq::OpenStand { .. } => "open-stand",
        InteractReq::Walk { .. } => "walk",
        InteractReq::WalkNear { .. } => "walk-near",
        InteractReq::WalkNearestBank => "walk-nearest-bank",
        InteractReq::WalkTo { .. } => "walk-to",
        InteractReq::Deposit { .. } => "deposit",
        InteractReq::Withdraw { .. } => "withdraw",
        InteractReq::WithdrawX { .. } => "withdraw-x",
        InteractReq::WithdrawLoad { .. } => "withdraw-load",
        InteractReq::Held { .. } => "held",
        InteractReq::InvButton { .. } => "inv-button",
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
        InteractReq::CloseModal => "close-modal",
        InteractReq::SideTab { .. } => "side-tab",
        InteractReq::Wear { .. } => "wear",
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
        InteractReq::Key { .. } => "key",
        InteractReq::Mouse { .. } => "mouse",
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
        | InteractReq::Wear { name } => Some(b.create_string(name)),
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
        _ => None,
    };
    let tab = b.start_table();
    b.push_slot_always(VT_IN_OP, op_off);
    match req {
        InteractReq::ShopButton {
            id,
            slot,
            component,
            chunk,
            ..
        } => {
            b.push_slot_always(VT_IN_KIND, kind_off.unwrap());
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_INDEX, *slot);
            b.push_slot_always(VT_IN_BANK_ITEM_ID, *id);
            b.push_slot_always(VT_IN_COMPONENT_ID, *component);
            b.push_slot_always(VT_IN_STAND_OP, *chunk);
        }
        InteractReq::OpenBooth {
            x, z, level, id, ..
        } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            b.push_slot_always(VT_IN_INDEX, *id);
            if let Some(off) = name_off {
                b.push_slot_always(VT_IN_NAME, off);
            }
            if let Some(off) = action_off {
                b.push_slot_always(VT_IN_ACTION, off);
            }
        }
        InteractReq::OpenStand {
            x,
            z,
            level,
            stand_op,
            ..
        } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            b.push_slot_always(VT_IN_KIND, kind_off.unwrap());
            if let Some(off) = name_off {
                b.push_slot_always(VT_IN_NAME, off);
            }
            if let Some(op) = stand_op {
                b.push_slot_always(VT_IN_STAND_OP, *op);
            }
            if let Some(off) = choose_off {
                b.push_slot_always(VT_IN_CHOOSE, off);
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
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            b.push_slot_always(VT_IN_INDEX, *radius);
            if *request_id != 0 {
                b.push_slot_always(VT_IN_REQUEST_ID, *request_id);
            }
            if let Some(off) = action_off {
                b.push_slot_always(VT_IN_ACTION, off);
            }
            if *allow_wilderness {
                b.push_slot_always(VT_IN_ALLOW_WILDERNESS, true);
            }
            if *allow_bank_fetch {
                b.push_slot_always(VT_IN_ALLOW_BANK_FETCH, true);
            }
        }
        InteractReq::WalkNearestBank => {}
        InteractReq::Walk {
            x,
            z,
            level,
            request_id,
            allow_wilderness,
            allow_bank_fetch,
            ..
        } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            if *request_id != 0 {
                b.push_slot_always(VT_IN_REQUEST_ID, *request_id);
            }
            if let Some(off) = action_off {
                b.push_slot_always(VT_IN_ACTION, off);
            }
            if *allow_wilderness {
                b.push_slot_always(VT_IN_ALLOW_WILDERNESS, true);
            }
            if *allow_bank_fetch {
                b.push_slot_always(VT_IN_ALLOW_BANK_FETCH, true);
            }
        }
        InteractReq::WalkTo { x, z, level } | InteractReq::RecoveryAnchor { x, z, level } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            if let Some(off) = action_off {
                b.push_slot_always(VT_IN_ACTION, off);
            }
        }
        InteractReq::Deposit { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
        }
        InteractReq::Withdraw { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::WithdrawX {
            count,
            bank_item_id,
            lands_as_id,
            bank_generation,
            ..
        } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_X, *count);
            b.push_slot_always(VT_IN_BANK_ITEM_ID, *bank_item_id);
            b.push_slot_always(VT_IN_LANDS_AS_ID, *lands_as_id);
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
            b.push_slot_always(VT_IN_BANK_GENERATION, *bank_generation);
        }
        InteractReq::WithdrawLoad {
            bank_generation, ..
        } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_BANK_GENERATION, *bank_generation);
        }
        InteractReq::Held { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::InvButton {
            id,
            slot,
            component,
            operation,
            bank_generation,
        } => {
            b.push_slot_always(VT_IN_BANK_ITEM_ID, *id);
            b.push_slot_always(VT_IN_SOURCE_ITEM_SLOT, *slot);
            b.push_slot_always(VT_IN_COMPONENT_ID, *component);
            b.push_slot_always(VT_IN_STAND_OP, *operation);
            b.push_slot_always(VT_IN_BANK_GENERATION, *bank_generation);
        }
        InteractReq::MakePanel {
            id,
            slot,
            component,
            operation,
        } => {
            b.push_slot_always(VT_IN_BANK_ITEM_ID, *id);
            b.push_slot_always(VT_IN_SOURCE_ITEM_SLOT, *slot);
            b.push_slot_always(VT_IN_COMPONENT_ID, *component);
            b.push_slot_always(VT_IN_STAND_OP, *operation);
        }
        InteractReq::Close => {}
        InteractReq::Npc { index, .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
            if let Some(idx) = index {
                b.push_slot_always(VT_IN_INDEX, *idx);
            }
        }
        InteractReq::Loc {
            x, z, level, id, ..
        } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
            if let Some(id) = id {
                b.push_slot_always(VT_IN_INDEX, *id);
            }
        }
        InteractReq::Obj { x, z, level, .. } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            if let Some(off) = name_off {
                b.push_slot_always(VT_IN_NAME, off);
            }
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::Player { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
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
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_KIND, kind_off.unwrap());
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            if let Some(off) = choose_off {
                b.push_slot_always(VT_IN_CHOOSE, off);
            }
            if let Some(idx) = index {
                b.push_slot_always(VT_IN_INDEX, *idx);
            }
            if let Some(id) = source_item_id {
                b.push_slot_always(VT_IN_SOURCE_ITEM_ID, *id);
            }
            if let Some(slot) = source_item_slot {
                b.push_slot_always(VT_IN_SOURCE_ITEM_SLOT, *slot);
            }
            if let Some(id) = target_item_id {
                b.push_slot_always(VT_IN_TARGET_ITEM_ID, *id);
            }
            if let Some(slot) = target_item_slot {
                b.push_slot_always(VT_IN_TARGET_ITEM_SLOT, *slot);
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
            b.push_slot_always(VT_IN_COMPONENT_ID, *component_id);
            b.push_slot_always(VT_IN_KIND, kind_off.unwrap());
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            if let Some(off) = choose_off {
                b.push_slot_always(VT_IN_CHOOSE, off);
            }
            if let Some(idx) = index {
                b.push_slot_always(VT_IN_INDEX, *idx);
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
            b.push_slot_always(VT_IN_STAND_OP, *option);
        }
        InteractReq::AnswerCount { value } => {
            b.push_slot_always(VT_IN_X, *value);
        }
        InteractReq::IfButton { component_id } => {
            b.push_slot_always(VT_IN_COMPONENT_ID, *component_id);
        }
        InteractReq::SideTab { tab } => {
            b.push_slot_always(VT_IN_STAND_OP, *tab);
        }
        InteractReq::Wear { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
        }
        InteractReq::SetRun { .. }
        | InteractReq::SetRetaliate { .. }
        | InteractReq::SetNoteMode { .. } => {
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::SetCameraYaw { yaw } => {
            b.push_slot_always(VT_IN_X, *yaw);
        }
        InteractReq::Key { down, .. } => {
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
            if let Some(off) = kind_off {
                b.push_slot_always(VT_IN_KIND, off);
            }
            b.push_slot_always(VT_IN_INDEX, if *down { 1 } else { 0 });
        }
        InteractReq::Mouse {
            down,
            x,
            y,
            button,
            identity,
        } => {
            b.push_slot_always(VT_IN_INDEX, if *down { 1 } else { 0 });
            b.push_slot_always(VT_IN_LEVEL, *button);
            b.push_slot_always(VT_IN_XF, *x);
            b.push_slot_always(VT_IN_YF, *y);
            if *identity != 0 {
                b.push_slot_always(VT_IN_INPUT_IDENTITY, *identity);
            }
        }
    }
    WIPOffset::new(b.end_table(tab).value())
}

/// Owned buyout-plan request after decode (or before encode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyoutPlanRequest {
    pub inv: String,
    pub keeper: String,
    pub coins: i64,
    pub stock: Vec<(String, i32)>,
    pub chosen: Vec<String>,
}

/// One planned purchase row on the FlatBuffer result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyoutPlanItem {
    pub obj: String,
    pub name: String,
    pub units: i32,
    pub est_cost: i64,
}

/// Owned buyout-plan result after decode (or before encode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyoutPlanResult {
    pub ok: bool,
    pub reason: String,
    pub items: Vec<BuyoutPlanItem>,
}

struct BuyoutStockReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for BuyoutStockReader<'a> {
    type Inner = BuyoutStockReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for BuyoutStockReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("obj", VT_BUYOUT_STOCK_OBJ, false)?
            .visit_field::<i32>("count", VT_BUYOUT_STOCK_COUNT, false)?
            .finish();
        Ok(())
    }
}

impl BuyoutStockReader<'_> {
    fn obj(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_STOCK_OBJ, None)
        }
        .unwrap_or("")
    }
    fn count(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BUYOUT_STOCK_COUNT, None) }.unwrap_or(0)
    }
}

struct BuyoutPlanItemReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for BuyoutPlanItemReader<'a> {
    type Inner = BuyoutPlanItemReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for BuyoutPlanItemReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("obj", VT_BUYOUT_ITEM_OBJ, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_BUYOUT_ITEM_NAME, false)?
            .visit_field::<i32>("units", VT_BUYOUT_ITEM_UNITS, false)?
            .visit_field::<i64>("est_cost", VT_BUYOUT_ITEM_EST_COST, false)?
            .finish();
        Ok(())
    }
}

impl BuyoutPlanItemReader<'_> {
    fn obj(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_ITEM_OBJ, None)
        }
        .unwrap_or("")
    }
    fn name(&self) -> &str {
        unsafe {
            self.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_ITEM_NAME, None)
        }
        .unwrap_or("")
    }
    fn units(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_BUYOUT_ITEM_UNITS, None) }.unwrap_or(0)
    }
    fn est_cost(&self) -> i64 {
        unsafe { self.tab.get::<i64>(VT_BUYOUT_ITEM_EST_COST, None) }.unwrap_or(0)
    }
}

struct BuyoutPlanRequestReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for BuyoutPlanRequestReader<'a> {
    type Inner = BuyoutPlanRequestReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for BuyoutPlanRequestReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<ForwardsUOffset<&str>>("inv", VT_BUYOUT_REQ_INV, false)?
            .visit_field::<ForwardsUOffset<&str>>("keeper", VT_BUYOUT_REQ_KEEPER, false)?
            .visit_field::<i64>("coins", VT_BUYOUT_REQ_COINS, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<BuyoutStockReader>>>>(
                "stock",
                VT_BUYOUT_REQ_STOCK,
                false,
            )?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(
                "chosen",
                VT_BUYOUT_REQ_CHOSEN,
                false,
            )?
            .finish();
        Ok(())
    }
}

struct BuyoutPlanResultReader<'a> {
    tab: Table<'a>,
}

impl<'a> Follow<'a> for BuyoutPlanResultReader<'a> {
    type Inner = BuyoutPlanResultReader<'a>;
    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
        Self {
            tab: Table::new(buf, loc),
        }
    }
}

impl Verifiable for BuyoutPlanResultReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<bool>("ok", VT_BUYOUT_RES_OK, false)?
            .visit_field::<ForwardsUOffset<&str>>("reason", VT_BUYOUT_RES_REASON, false)?
            .visit_field::<ForwardsUOffset<Vector<ForwardsUOffset<BuyoutPlanItemReader>>>>(
                "items",
                VT_BUYOUT_RES_ITEMS,
                false,
            )?
            .finish();
        Ok(())
    }
}

fn encode_buyout_plan_request_into(b: &mut FlatBufferBuilder<'_>, req: &BuyoutPlanRequest) {
    let inv = b.create_string(&req.inv);
    let keeper = b.create_string(&req.keeper);
    let stock_offs: Vec<_> = req
        .stock
        .iter()
        .map(|(obj, count)| {
            let obj_off = b.create_string(obj);
            let tab = b.start_table();
            b.push_slot_always(VT_BUYOUT_STOCK_OBJ, obj_off);
            b.push_slot_always(VT_BUYOUT_STOCK_COUNT, *count);
            WIPOffset::<BuyoutStockReader>::new(b.end_table(tab).value())
        })
        .collect();
    let stock_off = b.create_vector(&stock_offs);
    let chosen_offs: Vec<_> = req.chosen.iter().map(|s| b.create_string(s)).collect();
    let chosen_off = b.create_vector(&chosen_offs);
    let tab = b.start_table();
    b.push_slot_always(VT_BUYOUT_REQ_INV, inv);
    b.push_slot_always(VT_BUYOUT_REQ_KEEPER, keeper);
    b.push_slot_always(VT_BUYOUT_REQ_COINS, req.coins);
    b.push_slot_always(VT_BUYOUT_REQ_STOCK, stock_off);
    b.push_slot_always(VT_BUYOUT_REQ_CHOSEN, chosen_off);
    let root = b.end_table(tab);
    b.finish(root, None);
}

fn encode_buyout_plan_result_into(b: &mut FlatBufferBuilder<'_>, result: &BuyoutPlanResult) {
    let reason = b.create_string(&result.reason);
    let item_offs: Vec<_> = result
        .items
        .iter()
        .map(|item| {
            let obj = b.create_string(&item.obj);
            let name = b.create_string(&item.name);
            let tab = b.start_table();
            b.push_slot_always(VT_BUYOUT_ITEM_OBJ, obj);
            b.push_slot_always(VT_BUYOUT_ITEM_NAME, name);
            b.push_slot_always(VT_BUYOUT_ITEM_UNITS, item.units);
            b.push_slot_always(VT_BUYOUT_ITEM_EST_COST, item.est_cost);
            WIPOffset::<BuyoutPlanItemReader>::new(b.end_table(tab).value())
        })
        .collect();
    let items_off = b.create_vector(&item_offs);
    let tab = b.start_table();
    b.push_slot_always(VT_BUYOUT_RES_OK, result.ok);
    b.push_slot_always(VT_BUYOUT_RES_REASON, reason);
    b.push_slot_always(VT_BUYOUT_RES_ITEMS, items_off);
    let root = b.end_table(tab);
    b.finish(root, None);
}

/// Encode a same-tick buyout-plan request as a root FlatBuffer.
pub fn encode_buyout_plan_request(req: &BuyoutPlanRequest) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    encode_buyout_plan_request_into(&mut b, req);
    b.finished_data().to_vec()
}

/// Decode and verify a root `BuyoutPlanRequest` buffer.
pub fn decode_buyout_plan_request(buf: &[u8]) -> Result<BuyoutPlanRequest, String> {
    let req = verified_root::<BuyoutPlanRequestReader>(buf)?;
    let stock = rows_capped::<BuyoutStockReader>(&req.tab, VT_BUYOUT_REQ_STOCK, MAX_BUYOUT_STOCK)?;
    let chosen = match unsafe {
        req.tab
            .get::<ForwardsUOffset<Vector<ForwardsUOffset<&str>>>>(VT_BUYOUT_REQ_CHOSEN, None)
    } {
        Some(v) => {
            if v.len() > MAX_BUYOUT_CHOSEN {
                return Err(format!(
                    "vector length {} exceeds cap {MAX_BUYOUT_CHOSEN}",
                    v.len()
                ));
            }
            v.iter().map(str::to_string).collect()
        }
        None => Vec::new(),
    };
    Ok(BuyoutPlanRequest {
        inv: unsafe {
            req.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_REQ_INV, None)
        }
        .unwrap_or("")
        .to_string(),
        keeper: unsafe {
            req.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_REQ_KEEPER, None)
        }
        .unwrap_or("")
        .to_string(),
        coins: unsafe { req.tab.get::<i64>(VT_BUYOUT_REQ_COINS, None) }.unwrap_or(0),
        stock: stock
            .into_iter()
            .map(|row| (row.obj().to_string(), row.count()))
            .collect(),
        chosen,
    })
}

/// Encode a same-tick buyout-plan result as a root FlatBuffer.
pub fn encode_buyout_plan_result(result: &BuyoutPlanResult) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    encode_buyout_plan_result_into(&mut b, result);
    b.finished_data().to_vec()
}

/// Decode and verify a root `BuyoutPlanResult` buffer.
pub fn decode_buyout_plan_result(buf: &[u8]) -> Result<BuyoutPlanResult, String> {
    let res = verified_root::<BuyoutPlanResultReader>(buf)?;
    let items =
        rows_capped::<BuyoutPlanItemReader>(&res.tab, VT_BUYOUT_RES_ITEMS, MAX_BUYOUT_ITEMS)?;
    Ok(BuyoutPlanResult {
        ok: unsafe { res.tab.get::<bool>(VT_BUYOUT_RES_OK, None) }.unwrap_or(false),
        reason: unsafe {
            res.tab
                .get::<ForwardsUOffset<&str>>(VT_BUYOUT_RES_REASON, None)
        }
        .unwrap_or("")
        .to_string(),
        items: items
            .into_iter()
            .map(|row| BuyoutPlanItem {
                obj: row.obj().to_string(),
                name: row.name().to_string(),
                units: row.units(),
                est_cost: row.est_cost(),
            })
            .collect(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::shim::{InteractReq, ScriptPaint};

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
            widgets: &[],
        }
    }

    #[test]
    fn omitted_walk_outcome_fields_default_safe() {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let tab = b.start_table();
        b.push_slot_always(VT_SNAP_TICK, 7u64);
        let root = b.end_table(tab);
        b.finish(root, None);
        let view = SnapshotReader::from_bytes(b.finished_data()).expect("old snapshot");
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
        let got = view.stats();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].index(), 3);
        assert_eq!(got[0].name(), "hitpoints");
        assert_eq!(got[0].xp(), 1500);
        assert_eq!(got[0].base(), 10);
        assert_eq!(got[0].effective(), 7);
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
        };
        let mut input = empty_input(9);
        let npcs = [npc];
        input.npcs = &npcs;
        let bytes = encode_snapshot(&input);
        let view = decode_snapshot(&bytes).expect("snapshot decodes");
        assert!(view.has_npcs(), "keyframe carries npcs");
        let got = view.npcs();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].index(), 7);
        assert_eq!(got[0].id(), 41);
        assert_eq!(got[0].name(), Some("Chicken"));
        assert_eq!((got[0].x(), got[0].z(), got[0].level()), (3222, 3295, 0));
        assert_eq!(got[0].actions(), vec!["Attack", "Pick-up"]);
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
        assert!(view.npcs().is_empty(), "absent reads as empty vec");
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

    /// One reusable builder encodes snapshot, then paint, then interact —
    /// the per-slot / per-V8 buffer, reset between messages, never a
    /// JSON document and never `FlatBufferBuilder::new()` per tick.
    #[test]
    fn one_isolate_buf_encodes_snapshot_then_paint_then_interact() {
        let mut buf = IsolateBuf::new();
        let bytes = buf.encode_snapshot(&empty_input(1));
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        assert_eq!(snap.tick(), 1);

        let paint = ScriptPaint {
            title: Some("BoneBurier".into()),
            accent: Some("#f3e6a2".into()),
            lines: vec!["Runtime: 1.2m".into(), "".into()],
            buttons: vec![crate::shim::ScriptPaintButton {
                id: "gobank".into(),
                label: "Go bank".into(),
            }],
            generation: 0,
            canvas: Vec::new(),
        };
        let pbytes = buf.encode_paint(&paint);
        let decoded = decode_paint(&pbytes).expect("paint");
        assert_eq!(decoded, paint);

        let reqs = vec![
            InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into(),
            },
            InteractReq::Walk {
                x: 1,
                z: 2,
                level: 0,
                allow_teleports: true,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 9,
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
            },
        ];
        let bytes = encode_interact_batch(&reqs);
        assert_eq!(decode_interact_batch(&bytes).expect("decode"), reqs);
    }

    #[test]
    fn old_walk_buffers_default_new_find_options_false() {
        let mut b = FlatBufferBuilder::new();
        let op_off = b.create_string("walk");
        let tab = b.start_table();
        b.push_slot_always(VT_IN_OP, op_off);
        b.push_slot_always(VT_IN_X, 1);
        b.push_slot_always(VT_IN_Z, 2);
        b.push_slot_always(VT_IN_LEVEL, 0);
        b.push_slot_always(VT_IN_REQUEST_ID, 7u64);
        let row = WIPOffset::<InteractReader>::new(b.end_table(tab).value());
        let reqs = b.create_vector(&[row]);
        let batch = b.start_table();
        b.push_slot_always(VT_REQS, reqs);
        let root = b.end_table(batch);
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
            }]
        );
    }

    /// Truncated isolate→host buffers must not panic; invalid roots err.
    #[test]
    fn truncated_paint_and_interact_buffers_return_err() {
        let paint = ScriptPaint {
            title: Some("t".into()),
            accent: None,
            lines: vec!["line".into()],
            buttons: vec![crate::shim::ScriptPaintButton {
                id: "gobank".into(),
                label: "Go bank".into(),
            }],
            generation: 0,
            canvas: Vec::new(),
        };
        let full = IsolateBuf::new().encode_paint(&paint);
        for cut in 1..full.len() {
            let _ = decode_paint(&full[..cut]);
        }
        let mid = full.len().saturating_sub(8);
        assert!(
            decode_paint(&full[..mid]).is_err(),
            "paint truncated mid-payload should err"
        );
        let mut bad_root = full.clone();
        bad_root[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            decode_paint(&bad_root).is_err(),
            "huge paint root offset should err"
        );

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
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
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
        assert_eq!(reach.walkable(), walkable);
        assert_eq!(reach.reachable(), reachable);
        assert_eq!(reach.reachable_adj(), adj);
        assert_eq!(reach.exact_rank(), exact_rank);
        assert_eq!(reach.adjacent_rank(), adjacent_rank);
        assert_eq!(reach.step(), Vec::<u8>::new());
        assert_eq!(reach.canlight(), walkable);
        assert_eq!(reach.walkable()[0] & (1 << 31), 1 << 31, "bit 31 in word 0");
        assert_eq!(reach.walkable()[1] & 1, 1, "bit 32 in word 1");
        assert_eq!(reach.walkable()[1] & (1 << 21), 1 << 21, "bit 53 in word 1");
        assert_eq!(reach.walkable()[1] & (1 << 31), 1 << 31, "bit 63 in word 1");
        assert_eq!(reach.walkable()[2] & 1, 1, "bit 64 in word 2");
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
        };
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert!(kf.has_reach());
        assert_eq!(kf.reach().expect("reach").step(), step);
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
        };
        let (_keyframe, fp) = encode_snapshot_delta(None, &input, false);
        input.reach = ReachViewInput::UNAVAILABLE;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_reach(), "available→unavailable must post reach");
        let reach = view.reach().expect("cleared view");
        assert!(!reach.available());
        assert_eq!(reach.width(), 0);
        assert!(reach.walkable().is_empty());
        assert!(reach.exact_rank().is_empty());
        assert!(reach.adjacent_rank().is_empty());
        assert!(reach.step().is_empty());
        assert!(reach.canlight().is_empty());
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
        };
        let (keyframe, fp) = encode_snapshot_delta(None, &input, false);
        let kf = decode_snapshot(&keyframe).expect("keyframe");
        assert_eq!(kf.reach().expect("reach").canlight(), lit);

        input.reach.canlight = &zeros;
        let (delta, fp2) = encode_snapshot_delta(Some(&fp), &input, false);
        let view = decode_snapshot(&delta).expect("delta");
        assert!(view.has_reach(), "canlight change must post reach");
        assert_eq!(view.reach().expect("reach").canlight(), zeros);

        input.reach = ReachViewInput::UNAVAILABLE;
        let (cleared, _) = encode_snapshot_delta(Some(&fp2), &input, false);
        let view = decode_snapshot(&cleared).expect("cleared");
        assert!(view.has_reach());
        assert!(view.reach().expect("cleared").canlight().is_empty());
    }

    #[test]
    fn encode_decode_buyout_plan_request_and_result_round_trip() {
        let req = BuyoutPlanRequest {
            inv: "adventurershop".into(),
            keeper: "Aemad".into(),
            coins: 200,
            stock: vec![("vial_water".into(), 500), ("bronze_arrow".into(), 0)],
            chosen: vec!["vial of water".into()],
        };
        let bytes = encode_buyout_plan_request(&req);
        let got = decode_buyout_plan_request(&bytes).expect("request decodes");
        assert_eq!(got, req);

        let result = BuyoutPlanResult {
            ok: true,
            reason: String::new(),
            items: vec![BuyoutPlanItem {
                obj: "vial_water".into(),
                name: "Vial of water".into(),
                units: 2,
                est_cost: 4,
            }],
        };
        let bytes = encode_buyout_plan_result(&result);
        let got = decode_buyout_plan_result(&bytes).expect("result decodes");
        assert_eq!(got, result);
        assert!(decode_buyout_plan_request(b"not a request").is_err());
        assert!(decode_buyout_plan_result(b"not a result").is_err());
    }
}

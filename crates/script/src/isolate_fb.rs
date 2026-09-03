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

// Row: { name: string, count: int, id, ops, noted, cert }
const VT_ROW_NAME: VOffsetT = 4;
const VT_ROW_COUNT: VOffsetT = 6;
const VT_ROW_ID: VOffsetT = 8;
const VT_ROW_OPS: VOffsetT = 10;
const VT_ROW_NOTED: VOffsetT = 12;
const VT_ROW_CERT: VOffsetT = 14;

// Stat: { index, name, xp, level }
const VT_STAT_INDEX: VOffsetT = 4;
const VT_STAT_NAME: VOffsetT = 6;
const VT_STAT_XP: VOffsetT = 8;
const VT_STAT_LEVEL: VOffsetT = 10;

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

// SideTabIface: { index, id }
const VT_STI_INDEX: VOffsetT = 4;
const VT_STI_ID: VOffsetT = 6;

// ChatLine: { seq, text }
const VT_CL_SEQ: VOffsetT = 4;
const VT_CL_TEXT: VOffsetT = 6;

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
//             index, component_id }
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

// InteractBatch: { reqs: [Interact] }
const VT_REQS: VOffsetT = 4;

// Paint: { title: string, accent: string, lines: [string] }
const VT_PAINT_TITLE: VOffsetT = 4;
const VT_PAINT_ACCENT: VOffsetT = 6;
const VT_PAINT_LINES: VOffsetT = 8;

/// A game tile `{x, z, level}`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TileInput {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// One skill row: the snapshot's stat index, name, xp, and effective level.
#[derive(Clone, Copy)]
pub struct StatInput<'a> {
    pub index: i32,
    pub name: &'a str,
    pub xp: i32,
    pub level: i32,
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
    pub hold: bool,
    pub ours: bool,
    pub npcs: &'a [SceneEntityInput<'a>],
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
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_STAT_LEVEL, None) }.unwrap_or(0)
    }
}

impl Verifiable for StatReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("index", VT_STAT_INDEX, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_STAT_NAME, false)?
            .visit_field::<i32>("xp", VT_STAT_XP, false)?
            .visit_field::<i32>("level", VT_STAT_LEVEL, false)?
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
}

impl Verifiable for NearestBoothReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("x", VT_NEAREST_X, false)?
            .visit_field::<i32>("z", VT_NEAREST_Z, false)?
            .visit_field::<i32>("level", VT_NEAREST_LEVEL, false)?
            .visit_field::<ForwardsUOffset<&str>>("name", VT_NEAREST_NAME, false)?
            .visit_field::<ForwardsUOffset<&str>>("op", VT_NEAREST_OP, false)?
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
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ItemRowFp {
    pub name: Option<String>,
    pub count: i32,
    pub id: i32,
    pub ops: Vec<String>,
    pub noted: bool,
    pub cert: i32,
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
    pub name: String,
    pub op: String,
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
    pub stats: Vec<(i32, String, i32, i32)>,
    pub booths: Vec<TileInput>,
    pub nearest_booth: Option<NearestBoothFp>,
    pub banks: Vec<BankStandFp>,
    pub bank: Vec<ItemRowFp>,
    pub bank_side: Vec<ItemRowFp>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub hold: bool,
    pub ours: bool,
    pub npcs: Vec<SceneEntityFp>,
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
    pub chat_lines: Vec<(i32, String)>,
    pub bank_note_on: i32,
    pub bank_note_off: i32,
    pub scene_state: i32,
}

impl SnapshotFingerprint {
    /// Own the input's field values (names cloned) for later comparison.
    pub fn from_input(input: &SnapshotInput<'_>) -> SnapshotFingerprint {
        fn item_row_fp(r: &ItemRowInput<'_>) -> ItemRowFp {
            ItemRowFp {
                name: r.name.map(str::to_string),
                count: r.count,
                id: r.id,
                ops: r.ops.iter().map(|a| a.to_string()).collect(),
                noted: r.noted,
                cert: r.cert,
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
                .map(|s| (s.index, s.name.to_string(), s.xp, s.level))
                .collect(),
            booths: input.booths.to_vec(),
            nearest_booth: input.nearest_booth.as_ref().map(|b| NearestBoothFp {
                x: b.x,
                z: b.z,
                level: b.level,
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
                .map(|l| (l.seq, l.text.to_string()))
                .collect(),
            bank_note_on: input.bank_note_on,
            bank_note_off: input.bank_note_off,
            scene_state: input.scene_state,
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

    fn copy_finished(&self) -> Vec<u8> {
        self.builder.finished_data().to_vec()
    }

    /// Encode `input` as a root-`Snapshot` FlatBuffer carrying every field
    /// — the keyframe posted on Start / isolate spawn.
    pub fn encode_snapshot(&mut self, input: &SnapshotInput<'_>) -> Vec<u8> {
        self.builder.reset();
        encode_snapshot_masked_into(&mut self.builder, input, &DeltaMask::all());
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
        let fp = SnapshotFingerprint::from_input(input);
        let mask = match last {
            None => DeltaMask::all(),
            Some(prev) => DeltaMask::changed(prev, &fp, force_banks),
        };
        self.builder.reset();
        encode_snapshot_masked_into(&mut self.builder, input, &mask);
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

/// Encode `input` carrying exactly the masked fields (`tick` is always
/// carried).
fn encode_snapshot_masked_into(
    b: &mut FlatBufferBuilder<'_>,
    input: &SnapshotInput<'_>,
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
    WIPOffset::new(b.end_table(tab).value())
}

fn stat_off<'b>(b: &mut FlatBufferBuilder<'b>, s: &StatInput<'_>) -> WIPOffset<StatReader<'b>> {
    let name_off = b.create_string(s.name);
    let tab = b.start_table();
    b.push_slot_always(VT_STAT_INDEX, s.index);
    b.push_slot_always(VT_STAT_NAME, name_off);
    b.push_slot_always(VT_STAT_XP, s.xp);
    b.push_slot_always(VT_STAT_LEVEL, s.level);
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
    let tab = b.start_table();
    b.push_slot_always(VT_CL_SEQ, l.seq);
    b.push_slot_always(VT_CL_TEXT, text_off);
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
}

impl Verifiable for ChatLineReader<'_> {
    fn run_verifier(v: &mut Verifier, pos: usize) -> Result<(), InvalidFlatbuffer> {
        v.visit_table(pos)?
            .visit_field::<i32>("seq", VT_CL_SEQ, false)?
            .visit_field::<ForwardsUOffset<&str>>("text", VT_CL_TEXT, false)?
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
    pub fn z(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_IN_Z, None) }.unwrap_or(0)
    }
    pub fn level(&self) -> i32 {
        unsafe { self.tab.get::<i32>(VT_IN_LEVEL, None) }.unwrap_or(0)
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

fn encode_paint_into(b: &mut FlatBufferBuilder<'_>, paint: &crate::shim::ScriptPaint) {
    let title_off = paint.title.as_deref().map(|s| b.create_string(s));
    let accent_off = paint.accent.as_deref().map(|s| b.create_string(s));
    let line_offs: Vec<_> = paint.lines.iter().map(|s| b.create_string(s)).collect();
    let lines_off = b.create_vector(&line_offs);
    let tab = b.start_table();
    if let Some(off) = title_off {
        b.push_slot_always(VT_PAINT_TITLE, off);
    }
    if let Some(off) = accent_off {
        b.push_slot_always(VT_PAINT_ACCENT, off);
    }
    b.push_slot_always(VT_PAINT_LINES, lines_off);
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
            .finish();
        Ok(())
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
}

/// Decode a root-`Paint` FlatBuffer into the shim's recorded frame.
pub fn decode_paint(buf: &[u8]) -> Result<crate::shim::ScriptPaint, String> {
    let paint = verified_root::<PaintReader>(buf)?;
    Ok(crate::shim::ScriptPaint {
        title: paint.title().map(str::to_string),
        accent: paint.accent().map(str::to_string),
        lines: paint.lines()?,
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
                x: row.x(),
                z: row.z(),
                level: row.level(),
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
        InteractReq::WalkTo { .. } => "walk-to",
        InteractReq::Deposit { .. } => "deposit",
        InteractReq::Withdraw { .. } => "withdraw",
        InteractReq::Held { .. } => "held",
        InteractReq::Close => "close",
        InteractReq::Npc { .. } => "npc",
        InteractReq::Loc { .. } => "loc",
        InteractReq::Obj { .. } => "obj",
        InteractReq::Player { .. } => "player",
        InteractReq::UseOn { .. } => "use-on",
        InteractReq::UseWidgetOn { .. } => "use-widget-on",
        InteractReq::ContinueDialog => "continue",
        InteractReq::Answer { .. } => "answer",
        InteractReq::IfButton { .. } => "if-button",
        InteractReq::CloseModal => "close-modal",
        InteractReq::SideTab { .. } => "side-tab",
        InteractReq::Wear { .. } => "wear",
        InteractReq::SetRun { .. } => "set-run",
        InteractReq::SetRetaliate { .. } => "set-retaliate",
        InteractReq::SetNoteMode { .. } => "set-note-mode",
    });
    let kind_off = match req {
        InteractReq::OpenStand { kind, .. }
        | InteractReq::UseOn { kind, .. }
        | InteractReq::UseWidgetOn { kind, .. } => Some(b.create_string(kind)),
        _ => None,
    };
    let name_off = match req {
        InteractReq::OpenStand { name, .. } => name.as_deref().map(|n| b.create_string(n)),
        InteractReq::Deposit { name }
        | InteractReq::Withdraw { name, .. }
        | InteractReq::Held { name, .. }
        | InteractReq::Npc { name, .. }
        | InteractReq::Player { name, .. }
        | InteractReq::UseOn { name, .. }
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
        InteractReq::Withdraw { action, .. } | InteractReq::Held { action, .. } => {
            Some(b.create_string(action))
        }
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
        } => Some(b.create_string("tele")),
        _ => None,
    };
    let tab = b.start_table();
    b.push_slot_always(VT_IN_OP, op_off);
    match req {
        InteractReq::OpenBooth { x, z, level } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
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
        InteractReq::Walk { x, z, level, .. } | InteractReq::WalkTo { x, z, level } => {
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
        InteractReq::Held { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::Close => {}
        InteractReq::Npc { index, .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
            if let Some(idx) = index {
                b.push_slot_always(VT_IN_INDEX, *idx);
            }
        }
        InteractReq::Loc { x, z, level, .. } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
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
            x, z, level, index, ..
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
        InteractReq::ContinueDialog | InteractReq::CloseModal => {}
        InteractReq::Answer { option } => {
            b.push_slot_always(VT_IN_STAND_OP, *option);
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
    }
    WIPOffset::new(b.end_table(tab).value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shim::{InteractReq, ScriptPaint};

    fn empty_input(tick: u64) -> SnapshotInput<'static> {
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
        }
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
            },
            InteractReq::WalkTo {
                x: 3,
                z: 4,
                level: 0,
            },
        ];
        let ibytes = buf.encode_interact_batch(&reqs);
        let got = decode_interact_batch(&ibytes).expect("interact");
        assert_eq!(got, reqs);
    }

    /// Truncated isolate→host buffers must not panic; invalid roots err.
    #[test]
    fn truncated_paint_and_interact_buffers_return_err() {
        let paint = ScriptPaint {
            title: Some("t".into()),
            accent: None,
            lines: vec!["line".into()],
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
}

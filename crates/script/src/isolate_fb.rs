//! FlatBuffers wire format for isolate IPC — schema: `crates/script/
//! schema/isolate.fbs`. The builder and reader are hand-written against
//! that schema (operators never need `flatc` at `cargo test` time); keep
//! the two in sync. The PLAYER_INFO snapshot posted into each JS isolate
//! and the shim interact batch forwarded back are FlatBuffers, not JSON:
//! a 50+ isolate wall never stringifies or parses a JSON document per
//! tick.
//!
//! The wire format is produced and consumed only by 274bot code, so the
//! decoder trusts the buffer (root offset bounds-checked) like
//! `flatbuffers::root_unchecked`; accessors default missing fields to the
//! same fail-closed values the old JSON blob carried.
//!
//! Posts are deltas (schema: `Snapshot`): `tick` is always carried, other
//! fields only when they changed vs the last post — an omitted vector is
//! absent, never empty, and the isolate keeps its last JS value for it.
//! The per-slot last-post [`SnapshotFingerprint`] is compared by value
//! (equality, not a hash) once per slot per tick.

use flatbuffers::{FlatBufferBuilder, ForwardsUOffset, Table, VOffsetT, Vector, WIPOffset};

// Field slot offsets are vtable byte offsets: field id N sits at
// `(N + 2) * SIZE_VOFFSET` (`SIZE_VOFFSET = 2`), so id 0 -> 4, 1 -> 6, ...

// Tile / Booth: { x: int, z: int, level: int }
const VT_TILE_X: VOffsetT = 4;
const VT_TILE_Z: VOffsetT = 6;
const VT_TILE_LEVEL: VOffsetT = 8;

// Row: { name: string, count: int }
const VT_ROW_NAME: VOffsetT = 4;
const VT_ROW_COUNT: VOffsetT = 6;

// Stat: { index: int, name: string, xp: int }
const VT_STAT_INDEX: VOffsetT = 4;
const VT_STAT_NAME: VOffsetT = 6;
const VT_STAT_XP: VOffsetT = 8;

// BankStand: { name, x, z, level, kind, op, choose }
const VT_BANK_NAME: VOffsetT = 4;
const VT_BANK_X: VOffsetT = 6;
const VT_BANK_Z: VOffsetT = 8;
const VT_BANK_LEVEL: VOffsetT = 10;
const VT_BANK_KIND: VOffsetT = 12;
const VT_BANK_OP: VOffsetT = 14;
const VT_BANK_CHOOSE: VOffsetT = 16;

// Snapshot: { tick, here, ingame, inv, stats, booths, banks, bank,
//             bank_side, bank_open, bank_loaded, hold, ours }
const VT_SNAP_TICK: VOffsetT = 4;
const VT_SNAP_HERE: VOffsetT = 6;
const VT_SNAP_INGAME: VOffsetT = 8;
const VT_SNAP_INV: VOffsetT = 10;
const VT_SNAP_STATS: VOffsetT = 12;
const VT_SNAP_BOOTHS: VOffsetT = 14;
const VT_SNAP_BANKS: VOffsetT = 16;
const VT_SNAP_BANK: VOffsetT = 18;
const VT_SNAP_BANK_SIDE: VOffsetT = 20;
const VT_SNAP_BANK_OPEN: VOffsetT = 22;
const VT_SNAP_BANK_LOADED: VOffsetT = 24;
const VT_SNAP_HOLD: VOffsetT = 26;
const VT_SNAP_OURS: VOffsetT = 28;

// Interact: { op, x, z, level, kind, name, stand_op, choose, action }
const VT_IN_OP: VOffsetT = 4;
const VT_IN_X: VOffsetT = 6;
const VT_IN_Z: VOffsetT = 8;
const VT_IN_LEVEL: VOffsetT = 10;
const VT_IN_KIND: VOffsetT = 12;
const VT_IN_NAME: VOffsetT = 14;
const VT_IN_STAND_OP: VOffsetT = 16;
const VT_IN_CHOOSE: VOffsetT = 18;
const VT_IN_ACTION: VOffsetT = 20;

// InteractBatch: { reqs: [Interact] }
const VT_REQS: VOffsetT = 4;

/// A game tile `{x, z, level}`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TileInput {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// One skill row: the snapshot's stat index, name, and xp.
#[derive(Clone, Copy)]
pub struct StatInput<'a> {
    pub index: i32,
    pub name: &'a str,
    pub xp: i32,
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
    pub inv: &'a [(Option<&'a str>, i32)],
    pub stats: &'a [StatInput<'a>],
    pub booths: &'a [TileInput],
    pub banks: &'a [BankStandInput<'a>],
    pub bank: &'a [(Option<&'a str>, i32)],
    pub bank_side: &'a [(Option<&'a str>, i32)],
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub hold: bool,
    pub ours: bool,
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

/// One inventory/bank row as decoded: the resolved obj name (`None` =
/// unknown id) and a count.
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

/// The PLAYER_INFO snapshot as decoded: read-only access to the same
/// fields `script_snapshot_fb` encodes.
pub struct SnapshotReader<'a> {
    tab: Table<'a>,
}

impl SnapshotReader<'_> {
    /// Interpret `buf` as a root-`Snapshot` FlatBuffer. Only the root
    /// offset is bounds-checked: the buffer is produced by our own encoder
    /// (the same trust `flatbuffers::root_unchecked` asks for).
    pub fn from_bytes(buf: &[u8]) -> Result<SnapshotReader<'_>, String> {
        if buf.len() < 4 {
            return Err(format!("snapshot buffer too short: {} bytes", buf.len()));
        }
        let loc = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if loc + 4 > buf.len() {
            return Err(format!("snapshot root out of range: {loc} > {}", buf.len()));
        }
        Ok(SnapshotReader {
            // Safety: `loc` was bounds-checked against `buf`.
            tab: unsafe { Table::new(buf, loc) },
        })
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
    // Safety: the buffer was produced by our encoder (root checked).
    unsafe {
        tab.get::<ForwardsUOffset<Vector<'a, ForwardsUOffset<T>>>>(slot, None)
            .is_some()
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

/// The per-slot last-post fingerprint: an owned copy of the snapshot
/// fields the host last posted, compared against the next input to build
/// the delta. Content equality (not a hash) is fine — the tables are
/// small and the compare runs once per slot per tick.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SnapshotFingerprint {
    pub here: Option<TileInput>,
    pub ingame: bool,
    pub inv: Vec<(Option<String>, i32)>,
    pub stats: Vec<(i32, String, i32)>,
    pub booths: Vec<TileInput>,
    pub banks: Vec<BankStandFp>,
    pub bank: Vec<(Option<String>, i32)>,
    pub bank_side: Vec<(Option<String>, i32)>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub hold: bool,
    pub ours: bool,
}

impl SnapshotFingerprint {
    /// Own the input's field values (names cloned) for later comparison.
    pub fn from_input(input: &SnapshotInput<'_>) -> SnapshotFingerprint {
        SnapshotFingerprint {
            here: input.here,
            ingame: input.ingame,
            inv: input
                .inv
                .iter()
                .map(|(name, count)| (name.map(str::to_string), *count))
                .collect(),
            stats: input
                .stats
                .iter()
                .map(|s| (s.index, s.name.to_string(), s.xp))
                .collect(),
            booths: input.booths.to_vec(),
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
            bank: input
                .bank
                .iter()
                .map(|(name, count)| (name.map(str::to_string), *count))
                .collect(),
            bank_side: input
                .bank_side
                .iter()
                .map(|(name, count)| (name.map(str::to_string), *count))
                .collect(),
            bank_open: input.bank_open,
            bank_loaded: input.bank_loaded,
            hold: input.hold,
            ours: input.ours,
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
    pub stats: bool,
    pub booths: bool,
    pub banks: bool,
    pub bank: bool,
    pub bank_side: bool,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub hold: bool,
    pub ours: bool,
}

impl DeltaMask {
    /// Every field: the full (keyframe) snapshot.
    fn all() -> DeltaMask {
        DeltaMask {
            here: true,
            ingame: true,
            inv: true,
            stats: true,
            booths: true,
            banks: true,
            bank: true,
            bank_side: true,
            bank_open: true,
            bank_loaded: true,
            hold: true,
            ours: true,
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
            stats: next.stats != last.stats,
            booths: next.booths != last.booths,
            banks: force_banks || next.banks != last.banks,
            bank: next.bank != last.bank,
            bank_side: next.bank_side != last.bank_side,
            bank_open: next.bank_open != last.bank_open,
            bank_loaded: next.bank_loaded != last.bank_loaded,
            hold: next.hold != last.hold,
            ours: next.ours != last.ours,
        }
    }
}

/// Encode `input` as a root-`Snapshot` FlatBuffer carrying every field —
/// the keyframe posted on Start / isolate spawn.
pub fn encode_snapshot(input: &SnapshotInput<'_>) -> Vec<u8> {
    encode_snapshot_masked(input, &DeltaMask::all())
}

/// Encode a delta snapshot: `tick` always; every other field only when it
/// differs from `last` (all fields when `last` is `None` — the keyframe).
/// Omitted tables are absent from the buffer, never empty: the isolate
/// keeps its last JS values for them. Packed `banks` are carried on the
/// keyframe and when `force_banks` even if the stand list is unchanged
/// (a `NavWorld` identity change). Returns the encoded buffer and the
/// fingerprint of what was just posted — the caller stores it as the new
/// `last` (per-slot, reset on Start).
pub fn encode_snapshot_delta(
    last: Option<&SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
    force_banks: bool,
) -> (Vec<u8>, SnapshotFingerprint) {
    let fp = SnapshotFingerprint::from_input(input);
    let mask = match last {
        None => DeltaMask::all(),
        Some(prev) => DeltaMask::changed(prev, &fp, force_banks),
    };
    (encode_snapshot_masked(input, &mask), fp)
}

/// Encode `input` carrying exactly the masked fields (`tick` is always
/// carried).
fn encode_snapshot_masked(input: &SnapshotInput<'_>, mask: &DeltaMask) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    // Children (strings, sub-tables, vectors) are written before the root
    // table's own start — masked-in fields only.
    let here_off = if mask.here {
        input.here.map(|h| tile_off(&mut b, h))
    } else {
        None
    };
    let inv_off = if mask.inv {
        let offs = input
            .inv
            .iter()
            .map(|(name, count)| row_off(&mut b, *name, *count))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let stats_off = if mask.stats {
        let offs = input
            .stats
            .iter()
            .map(|s| stat_off(&mut b, s))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let booths_off = if mask.booths {
        let offs = input
            .booths
            .iter()
            .map(|t| tile_off(&mut b, *t))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let banks_off = if mask.banks {
        let offs = input
            .banks
            .iter()
            .map(|s| bank_stand_off(&mut b, s))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let bank_off = if mask.bank {
        let offs = input
            .bank
            .iter()
            .map(|(name, count)| row_off(&mut b, *name, *count))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
    } else {
        None
    };
    let bank_side_off = if mask.bank_side {
        let offs = input
            .bank_side
            .iter()
            .map(|(name, count)| row_off(&mut b, *name, *count))
            .collect::<Vec<_>>();
        Some(b.create_vector(&offs))
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
    if mask.stats {
        b.push_slot_always(VT_SNAP_STATS, stats_off.expect("mask checked"));
    }
    if mask.booths {
        b.push_slot_always(VT_SNAP_BOOTHS, booths_off.expect("mask checked"));
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
    let root = b.end_table(tab);
    b.finish(root, None);
    b.finished_data().to_vec()
}

fn tile_off<'b>(b: &mut FlatBufferBuilder<'b>, t: TileInput) -> WIPOffset<TileReader<'b>> {
    let tab = b.start_table();
    b.push_slot_always(VT_TILE_X, t.x);
    b.push_slot_always(VT_TILE_Z, t.z);
    b.push_slot_always(VT_TILE_LEVEL, t.level);
    WIPOffset::new(b.end_table(tab).value())
}

fn row_off<'b>(
    b: &mut FlatBufferBuilder<'b>,
    name: Option<&str>,
    count: i32,
) -> WIPOffset<RowReader<'b>> {
    let name_off = name.map(|n| b.create_string(n));
    let tab = b.start_table();
    if let Some(off) = name_off {
        b.push_slot_always(VT_ROW_NAME, off);
    }
    b.push_slot_always(VT_ROW_COUNT, count);
    WIPOffset::new(b.end_table(tab).value())
}

fn stat_off<'b>(b: &mut FlatBufferBuilder<'b>, s: &StatInput<'_>) -> WIPOffset<StatReader<'b>> {
    let name_off = b.create_string(s.name);
    let tab = b.start_table();
    b.push_slot_always(VT_STAT_INDEX, s.index);
    b.push_slot_always(VT_STAT_NAME, name_off);
    b.push_slot_always(VT_STAT_XP, s.xp);
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
}

/// A batch of shim interact requests as decoded.
pub struct InteractBatchReader<'a> {
    tab: Table<'a>,
}

impl InteractBatchReader<'_> {
    /// Interpret `buf` as a root-`InteractBatch` FlatBuffer (produced by
    /// our own encoder; only the root offset is bounds-checked).
    pub fn from_bytes(buf: &[u8]) -> Result<InteractBatchReader<'_>, String> {
        if buf.len() < 4 {
            return Err(format!("interact buffer too short: {} bytes", buf.len()));
        }
        let loc = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if loc + 4 > buf.len() {
            return Err(format!("interact root out of range: {loc} > {}", buf.len()));
        }
        Ok(InteractBatchReader {
            // Safety: `loc` was bounds-checked against `buf`.
            tab: unsafe { Table::new(buf, loc) },
        })
    }

    pub fn reqs(&self) -> Vec<InteractReader<'_>> {
        rows::<InteractReader>(&self.tab, VT_REQS)
    }
}

/// Encode the tick's shim interact queue as a root-`InteractBatch`
/// FlatBuffer.
pub fn encode_interact_batch(reqs: &[crate::shim::InteractReq]) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let offs = reqs
        .iter()
        .map(|req| interact_off(&mut b, req))
        .collect::<Vec<_>>();
    let reqs_off = b.create_vector(&offs);
    let tab = b.start_table();
    b.push_slot_always(VT_REQS, reqs_off);
    let root = b.end_table(tab);
    b.finish(root, None);
    b.finished_data().to_vec()
}

/// Decode a root-`InteractBatch` into the shim's request type. A row with
/// a missing/unknown `op` (or a request missing a required field) fails
/// the whole batch — the host logs it and drops the batch, never fatal,
/// exactly like the old JSON parse.
pub fn decode_interact_batch(buf: &[u8]) -> Result<Vec<crate::shim::InteractReq>, String> {
    let batch = InteractBatchReader::from_bytes(buf)?;
    let mut out = Vec::with_capacity(batch.reqs().len());
    for row in batch.reqs() {
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
            "close" => out.push(crate::shim::InteractReq::Close),
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
    // All strings are created before the table starts (create_string is
    // illegal while a table is open).
    let op_off = b.create_string(match req {
        InteractReq::OpenBooth { .. } => "open-booth",
        InteractReq::OpenStand { .. } => "open-stand",
        InteractReq::Walk { .. } => "walk",
        InteractReq::Deposit { .. } => "deposit",
        InteractReq::Withdraw { .. } => "withdraw",
        InteractReq::Close => "close",
    });
    let kind_off = match req {
        InteractReq::OpenStand { kind, .. } => Some(b.create_string(kind)),
        _ => None,
    };
    let name_off = match req {
        InteractReq::OpenStand { name, .. } => name.as_deref().map(|n| b.create_string(n)),
        InteractReq::Deposit { name } | InteractReq::Withdraw { name, .. } => {
            Some(b.create_string(name))
        }
        _ => None,
    };
    let choose_off = match req {
        InteractReq::OpenStand { choose, .. } => choose.as_deref().map(|c| b.create_string(c)),
        _ => None,
    };
    let action_off = match req {
        InteractReq::Withdraw { action, .. } => Some(b.create_string(action)),
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
        InteractReq::Walk { x, z, level } => {
            b.push_slot_always(VT_IN_X, *x);
            b.push_slot_always(VT_IN_Z, *z);
            b.push_slot_always(VT_IN_LEVEL, *level);
        }
        InteractReq::Deposit { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
        }
        InteractReq::Withdraw { .. } => {
            b.push_slot_always(VT_IN_NAME, name_off.unwrap());
            b.push_slot_always(VT_IN_ACTION, action_off.unwrap());
        }
        InteractReq::Close => {}
    }
    WIPOffset::new(b.end_table(tab).value())
}

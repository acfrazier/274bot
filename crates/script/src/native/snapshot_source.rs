use crate::native::seat_rows::SeatPageInput;
use crate::native::WalkCancelReason;
use api::snapshot::ProjectileView;

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
    /// Authoritative local actor movement. `None` means there is no local
    /// player observation; old buffers likewise decode as unknown.
    pub local_player_moving: Option<bool>,
    /// The host's live gather session. `None` means do not write this delta
    /// page; `Some` has a status only after the card publishes one.
    pub api_gather: Option<&'a crate::api_gather::GatherPage>,
    /// The retained terminal result. Absence means retain any prior outcome.
    pub api_gather_outcome: Option<&'a crate::api_gather::GatherEnd>,
    /// The host's live combat session (gather page shape). `None` clears a
    /// previously posted page, exactly like `api_gather`.
    pub api_combat: Option<&'a crate::api_combat::CombatPage>,
    /// The retained combat terminal. Absence means retain any prior outcome.
    pub api_combat_outcome: Option<&'a crate::api_combat::CombatEnd>,
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
    /// `modals().side` (`-1` none). With `bank_open` it is the posted-side
    /// fact for `bank_side`. `None` omits the slot; the isolate keeps its
    /// last root.
    pub side_modal_id: Option<i32>,
    /// The complete forest/result status page. `None` omits it; `Some` with
    /// empty slices explicitly clears both wire vectors.
    pub seat_page: Option<SeatPageInput<'a>>,
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

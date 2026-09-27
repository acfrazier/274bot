use super::*;
/// A world tile: absolute `x`/`z` plus the plane (`level`). The key type
/// loc/ground-item/player families are positioned by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct WorldTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// A tile relative to the scene origin (`level` is implicit in the world
/// build). The loc/ground-item families' scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalTile {
    pub lx: i32,
    pub lz: i32,
}

/// A family of world state, mirroring the `ClientGens` counters.
/// `Loc`/`GroundItem` have no counters of their own: loc and ground-item
/// changes bump `gens.scene`, so both track it with a dedicated slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Npc,
    Player,
    Inv,
    Varp,
    Stat,
    Chat,
    Scene,
    Iface,
    Camera,
    MapFlag,
    World,
    Loc,
    GroundItem,
    /// Task 4 iface-derived families. They re-read the materialized
    /// `client.ifaces` (and the inv slot data) instead of deep-copying
    /// the world; each tracks its own gen gate.
    Inventory,
    Equipment,
    Bank,
    BankSide,
    Trade,
    Shop,
    Widgets,
    SideTabs,
    ChatOptions,
    MakeProducts,
    /// Main-modal TYPE_INV rows whose component ops start with Make
    /// (the anvil skill-multi panel). Distinct from chat `MakeProducts`.
    MainMake,
    QuestStatuses,
    Modals,
    Controls,
    Menu,
}

/// The kind of entity an actor is facing (`ActorTargetView::kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ActorKind {
    Npc,
    Player,
}

/// A resolved `face_entity`: the entity kind and slot index an actor is
/// facing, or `None` when not facing anyone. Slots are the client's own
/// (`npc`/`players` table indexes), decoded like `entity_face` in
/// `client.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActorTargetView {
    pub kind: ActorKind,
    pub index: usize,
}

/// The shared actor view every entity-family view embeds (the m8aq
/// `ActorSnapshot`): name/actions plus the entity's live pose state.
#[derive(Debug, Clone, Serialize)]
pub struct ActorView {
    pub name: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub animation: i32,
    pub pose_animation: i32,
    pub orientation: i32,
    pub target_orientation: i32,
    pub overhead_text: Option<String>,
    pub spot_animation: i32,
    pub health: i32,
    pub total_health: i32,
    pub face_entity: i32,
    pub target: Option<ActorTargetView>,
    pub moving: bool,
    pub running: bool,
    pub in_combat: bool,
}

/// Owned view of one live NPC slot, keyed by its slot index in
/// `Client.npc`. The reader's copy is independent of later in-place walk
/// mutations, so identity (the slot index) is stable across rebuilds.
#[derive(Debug, Clone, Serialize)]
pub struct NpcView {
    pub index: usize,
    pub r#type: Option<usize>,
    pub name: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub animation: i32,
    pub pose_animation: i32,
    pub orientation: i32,
    pub target_orientation: i32,
    pub overhead_text: Option<String>,
    pub spot_animation: i32,
    pub health: i32,
    pub total_health: i32,
    pub face_entity: i32,
    pub target: Option<ActorTargetView>,
    pub moving: bool,
    pub running: bool,
    pub in_combat: bool,
    pub level: i32,
    pub size: i32,
    /// Path-head network SW: `map_build_base + route[0]`, same level as `tile`.
    /// Packet-time at `rebuild_npcs`; not an alias of rendered `tile`.
    pub network: WorldTile,
    /// Legacy position aliases (`query::npcs_at` and older tests read them).
    /// These are the raw entity pixel coords, not the world `tile` above.
    pub x: i32,
    pub z: i32,
    pub yaw: i32,
}

/// A remote player, keyed by its `players` table slot index.
#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    pub index: usize,
    pub actor: ActorView,
    pub combat_level: i32,
    pub skill_level: i32,
}

/// The local player: a `PlayerView` at `self_slot` plus the run/weight
/// stats. `distance` is always 0.
#[derive(Debug, Clone, Serialize)]
pub struct LocalPlayerView {
    pub player: PlayerView,
    pub energy: i32,
    pub weight: i32,
}

/// One skill slot of the client's 25-entry table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatView {
    pub index: i32,
    pub name: String,
    pub effective: i32,
    pub base: i32,
    pub xp: i32,
    pub used: bool,
}

/// Whether the client's skill table uses stat slot `index`
/// (`Skill::used`). Unused slots always post base 0.
pub fn stat_used(index: usize) -> bool {
    Skill::used.get(index).copied().unwrap_or(false)
}

/// One varp's value from the client's `var` table (the m8aq
/// `VarpSnapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VarpView {
    pub index: i32,
    pub value: i32,
}

/// A contained item: the obj definition plus the container, the slot
/// position and the interface ops (the m8aq `ItemSnapshot`). `def.id` is
/// the real obj id — iface `link_obj_type` stores `obj_id + 1` (0 empty),
/// so the stored value decodes with a `- 1` (the client's own draw
/// convention; m8aq `readInvComponent`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemView {
    pub def: ItemDefView,
    pub container: ItemContainer,
    pub action_family: ItemActionFamily,
    pub slot: i32,
    pub count: i32,
    pub actions: Vec<Option<String>>,
    pub component_id: i32,
}

/// Which surface the item sits on (the m8aq `ItemContainer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ItemContainer {
    Inventory,
    Equipment,
    Bank,
    BankSide,
    TradeMyOffer,
    TradeTheirOffer,
    TradeSidePack,
    ShopStock,
    /// The shop's own view of the local player's pack (`shop_template_side:inv`,
    /// 3823): the container Sell 1/5/10 read and act on.
    ShopPlayer,
    /// Main-modal skill-multi TYPE_INV (anvil Make-N). Distinct from the
    /// backpack and from chat make-products.
    MainMake,
    Widget,
}

/// Where the item's menu ops come from: held items read the obj def's
/// `iop`, component items read the TYPE_INV iface's own `iop`, and the
/// trade partner's offer exposes no ops (the m8aq `ItemActionFamily`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ItemActionFamily {
    Held,
    Component,
    None,
}

/// One varp-bound component script: an opcode-5 (`IF_VARP`) script with
/// the varp index as its first operand, decoded like the client's own
/// toggle/select arms (and m8aq `widgetVarpBindings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WidgetVarpBindingView {
    pub script_index: i32,
    pub varp: i32,
    pub value: Option<i32>,
    pub comparator: Option<i32>,
}

/// The widget tag (which open root's tree the widget lives in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WidgetRoot {
    Main,
    Side,
    Chat,
    Tutorial,
}

/// The discriminated-union tag of a widget view (only `Widget` today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WidgetKind {
    Widget,
}

/// One interface component reachable from an open root, with its derived
/// walk context (accumulated position, parent, root tag).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WidgetView {
    pub kind: WidgetKind,
    pub component_id: i32,
    pub layer_id: i32,
    pub parent_id: i32,
    pub root_component_id: i32,
    pub root: WidgetRoot,
    pub type_: i32,
    pub button_type: i32,
    pub client_code: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scroll_height: i32,
    pub scroll_position: i32,
    pub hidden: bool,
    pub text: Option<String>,
    pub alternate_text: Option<String>,
    pub button_text: Option<String>,
    pub target_verb: Option<String>,
    pub target_base: Option<String>,
    pub target_mask: i32,
    pub model_type: i32,
    pub model_id: i32,
    pub alternate_model_type: i32,
    pub alternate_model_id: i32,
    pub scripts: Option<Vec<Option<Vec<i32>>>>,
    pub script_comparators: Option<Vec<i32>>,
    pub script_operands: Option<Vec<i32>>,
    pub varp_bindings: Vec<WidgetVarpBindingView>,
    pub colour: i32,
    pub actions: Vec<Option<String>>,
    pub items: Vec<ItemView>,
}

/// One side-tab slot: the interface drawn on it and the tab state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SideTabView {
    pub index: i32,
    pub root_component_id: i32,
    pub available: bool,
    pub active: bool,
    pub visible: bool,
    pub widgets: Vec<WidgetView>,
}

/// The open puzzle board observation: the identified piece container (the
/// first depth-first TYPE_INV with `obj_ops` under the main modal), its
/// `link_obj_type` slot count and its rows. `component_id` is `-1` for a
/// closed board (`size` 0, no rows) — a present fact the snapshot posts, not
/// an omitted slot. `items` borrows the identified widget's own rows: the
/// board never copies the world onto the isolate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PuzzleBoardView<'a> {
    pub component_id: i32,
    pub size: i32,
    pub generation: u64,
    pub items: &'a [ItemView],
}

/// The trade ifaces' state and the four trade containers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TradeView {
    pub offer_open: bool,
    pub confirm_open: bool,
    pub my_offer: Vec<ItemView>,
    pub their_offer: Vec<ItemView>,
    pub side_pack: Vec<ItemView>,
    pub partner: Option<String>,
    /// Accept button on the open trade root (-1 when absent).
    pub accept_component_id: i32,
    /// Decline button on the open trade root (-1 when absent).
    pub decline_component_id: i32,
}

impl Default for TradeView {
    fn default() -> Self {
        Self {
            offer_open: false,
            confirm_open: false,
            my_offer: Vec::new(),
            their_offer: Vec::new(),
            side_pack: Vec::new(),
            partner: None,
            accept_component_id: -1,
            decline_component_id: -1,
        }
    }
}

/// The shop main modal's stock container and the shop side interface's
/// player pack (empty while the shop is down).
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct ShopView {
    pub open: bool,
    pub stock: Vec<ItemView>,
    /// The `shop_template_side:inv` (3823) rows: `Sell` acts here, never on
    /// the stock rows and never on the plain backpack container.
    pub player: Vec<ItemView>,
    /// Whether that side container was decoded at all this rebuild. Distinguishes
    /// an unposted/missing player pack (fail closed) from an empty one.
    pub player_available: bool,
}

/// One chat history line. The ring's index 0 is the newest line;
/// `sequence` is the client's per-message counter (one bump per
/// `add_chat`), so `since(last)` queries see only genuinely new lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatLineView {
    pub type_: i32,
    pub username: Option<String>,
    pub text: String,
    pub sequence: i32,
}

/// One BUTTON_OK choice of the chat modal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatOptionView {
    pub component_id: i32,
    pub text: String,
}

/// One make/smelt button of a make product (`quantity` -1 = "Make X").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MakeButtonView {
    pub quantity: i32,
    pub component_id: i32,
}

/// One make-X product: the obj-model component plus its four buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MakeProductView {
    pub object_id: i32,
    pub name: String,
    pub buttons: Vec<MakeButtonView>,
}

/// One quest-journal entry (a TYPE_TEXT row of the quest tab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestStatusView {
    pub component_id: i32,
    pub name: String,
    pub colour: i32,
}

/// Coarse quest state represented by the quest-list display colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestListStatus {
    NotStarted,
    InProgress,
    Complete,
    Unknown,
}

impl QuestListStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "notStarted",
            Self::InProgress => "inProgress",
            Self::Complete => "complete",
            Self::Unknown => "unknown",
        }
    }
}

impl QuestStatusView {
    /// Resolve only the three display colours documented by the script API.
    pub const fn status(&self) -> QuestListStatus {
        match self.colour {
            0xF80000 => QuestListStatus::NotStarted,
            0xF8F800 => QuestListStatus::InProgress,
            0x00F800 => QuestListStatus::Complete,
            _ => QuestListStatus::Unknown,
        }
    }
}

/// The on/off toggle pair of the player-controls overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ToggleControlsView {
    pub on_component_id: i32,
    pub off_component_id: i32,
}

/// The four open modal/overlay roots (-1 = none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct ModalView {
    pub main: i32,
    pub side: i32,
    pub chat: i32,
    pub tutorial: i32,
}

/// The sim-world layer a placed loc occupies (the m8aq `LocSnapshot.layer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LocLayer {
    Wall,
    WallDecoration,
    Ground,
    GroundDecoration,
}

/// One placed loc: the packed typecodes plus the decoded shape/angle and
/// the resolved definition (name/actions/footprint/block flags) from the
/// loc table.
#[derive(Debug, Clone, Serialize)]
pub struct LocView {
    pub typecode: i32,
    pub info: i32,
    pub id: i32,
    pub name: Option<String>,
    pub description: Option<String>,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
    pub layer: LocLayer,
    pub shape: i32,
    pub angle: i32,
    pub width: i32,
    pub length: i32,
    pub footprint_width: i32,
    pub footprint_length: i32,
    pub block_walk: bool,
    pub block_range: bool,
    pub active: bool,
    pub animation: i32,
    pub map_function: i32,
    pub map_scene: i32,
    pub force_approach: i32,
}

/// One object stack on the ground: the obj definition plus the stack count
/// and the ground menu ops.
#[derive(Debug, Clone, Serialize)]
pub struct GroundItemView {
    pub def: ItemDefView,
    pub count: i32,
    pub actions: Vec<Option<String>>,
    pub tile: WorldTile,
    pub distance: i32,
}

/// The built scene: the collision grid the nav/query surface reads, as a
/// flat row-major `x * width + z` flag list.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SceneView {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub collision_flags: Vec<i32>,
}

/// The client's world scalars (the m8aq `WorldStateSnapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct WorldStateView {
    pub map_base_x: i32,
    pub map_base_z: i32,
    pub level: i32,
    pub members: bool,
    pub multi_combat: bool,
    pub player_count: i32,
    pub npc_count: i32,
    pub cycle: i32,
}

/// The camera state: the follow-camera eye plus the orbit target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct CameraView {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub pitch: i32,
    pub yaw: i32,
    pub orbit_pitch: i32,
    pub orbit_yaw: i32,
    pub cinematic: bool,
}

/// The minimap destination flag, in scene-local tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MapFlagView {
    pub lx: i32,
    pub lz: i32,
}

/// The native coordinate hint target. The client normalizes wire hint
/// types 2–6 to type 2 before the snapshot reads these world coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HintTileView {
    pub x: i32,
    pub z: i32,
}

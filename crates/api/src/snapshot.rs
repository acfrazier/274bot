//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use crate::obj_names::ItemDefView;
use client::client::{Client, ClientGens, ClientNpc, Skill};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeView};
use client::config::{Cache, ObjType};
use client::dash3d::client_entity::ClientEntity;
use serde::Serialize;

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
    Widgets,
    SideTabs,
    ChatOptions,
    MakeProducts,
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
    /// Legacy position aliases (`query::npcs_at` and older tests read them).
    /// These are the raw entity pixel coords, not the world `tile` above.
    pub x: i32,
    pub z: i32,
    pub yaw: i32,
}

impl NpcView {
    fn from_slot(
        index: usize,
        npc: &ClientNpc,
        base: (i32, i32),
        level: i32,
        distance: i32,
        cache: &Cache,
    ) -> Self {
        let entity = &npc.entity;
        let (name, actions, npc_level, size) = match npc.r#type.and_then(|t| cache.npcs.get(t)) {
            Some(t) => (
                Some(t.name.clone()),
                t.op.clone(),
                t.vislevel.max(0),
                t.size,
            ),
            None => (None, Vec::new(), 0, entity.size),
        };
        NpcView {
            index,
            r#type: npc.r#type,
            name,
            actions,
            tile: entity_world_tile(entity, base, level),
            distance,
            animation: entity.primary_anim,
            pose_animation: entity.secondary_anim,
            orientation: entity.yaw,
            target_orientation: entity.dst_yaw,
            overhead_text: entity.chat_message.clone(),
            spot_animation: entity.spotanim_id,
            health: entity.health,
            total_health: entity.total_health,
            face_entity: entity.face_entity,
            target: decode_target(entity.face_entity),
            moving: entity.route_length > 0,
            running: entity.primary_anim == entity.runanim,
            in_combat: entity.combat_cycle > 0,
            level: npc_level,
            size,
            x: entity.x,
            z: entity.z,
            yaw: entity.yaw,
        }
    }
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

/// The trade ifaces' state and the four trade containers.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TradeView {
    pub offer_open: bool,
    pub confirm_open: bool,
    pub my_offer: Vec<ItemView>,
    pub their_offer: Vec<ItemView>,
    pub side_pack: Vec<ItemView>,
    pub partner: Option<String>,
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

/// Generation-stamped read model. `rebuild_family` copies only the family
/// whose gen moved; `npcs()` returns the last rebuild without allocating.
/// Serializes to the whole-window shot sidecar JSON (the terminal state).
#[derive(Serialize)]
pub struct GameSnapshot {
    /// World generations the snapshot has been rebuilt up to.
    #[serde(skip)]
    gens: ClientGens,
    npc: Vec<NpcView>,
    player: Option<LocalPlayerView>,
    players: Vec<PlayerView>,
    stats: Vec<StatView>,
    runenergy: i32,
    /// The scene origin (`map_build_base_x/z`); `None` before a world
    /// build. The mainland-seed gate reads it (the tutorial island build
    /// origin stays below 3000).
    base: Option<(i32, i32)>,
    /// The local player's world tile `(x, z, level)`; `None` before the
    /// first `PLAYER_INFO`. Tile level is not decoded on the body yet
    /// (gaps.md), so this is always level 0.
    tile: Option<(i32, i32, i32)>,
    /// The local player's slot (`Client.self_slot`, -1 before `UPDATE_PID`).
    self_slot: i32,
    /// The host game-tick count this snapshot reflects: one bump per
    /// `PLAYER_INFO` (the same tick edge `should_emit_tick` reads).
    tick: u32,
    /// Inventory `(obj id, count)` from the TYPE_INV iface, rebuilt when
    /// the inv gen moves (the server's `UPDATE_INV_FULL` fills it each
    /// frame). Empty before the inv iface loads.
    inv: Vec<(i32, i32)>,
    /// The most recent chat line (`chat_text[0]` is the ring head).
    chat: Option<String>,
    ingame: bool,
    scene_state: i32,
    /// Socket state (`Client.stream`): whether the slot is attached to a
    /// server connection (the m8aq `attached`).
    attached: bool,
    /// Placed locs from the last loc rebuild (sweeps the sim world's four
    /// layers at `minusedlevel`).
    loc: Vec<LocView>,
    /// Ground-object stacks from the last ground-item rebuild.
    ground_item: Vec<GroundItemView>,
    /// The built scene's collision grid from the last scene rebuild.
    scene: SceneView,
    /// The client's world scalars from the last world rebuild.
    world: WorldStateView,
    /// The camera state from the last camera rebuild.
    camera: CameraView,
    /// The minimap flag from the last map-flag rebuild; `None` while no
    /// flag is set.
    map_flag: Option<MapFlagView>,
    /// Scene gen the loc/ground-item views were rebuilt up to. Loc and
    /// ground-item changes bump `gens.scene`, so both track it here
    /// (separately from the scene family's own counter).
    #[serde(skip)]
    loc_gen: u64,
    #[serde(skip)]
    ground_item_gen: u64,

    /// Task 4 iface-derived views, rebuilt when their gen moved. The
    /// item-bearing families track the iface and inv gens (component ids
    /// on the iface gen, slot data on the inv gen); the pure-tree and
    /// scalar families track the iface gen. Each family keeps its own
    /// gate so a movement rebuilds only that family.
    inventory: Vec<ItemView>,
    equipment: Vec<ItemView>,
    bank: Vec<ItemView>,
    bank_side: Vec<ItemView>,
    /// The inv tab component's slot count (the m8aq `inventorySize`);
    /// 0 until the inv tab loads.
    inventory_size: i32,
    /// The open main modal's withdraw component (the m8aq
    /// `bankComponentId`); -1 while no bank is open.
    bank_component_id: i32,
    trade: TradeView,
    widgets: Vec<WidgetView>,
    side_tabs: Vec<SideTabView>,
    chat_lines: Vec<ChatLineView>,
    chat_options: Vec<ChatOptionView>,
    chat_continue_component_id: i32,
    make_products: Vec<MakeProductView>,
    quest_statuses: Vec<QuestStatusView>,
    run_controls: Option<ToggleControlsView>,
    retaliate_controls: Option<ToggleControlsView>,
    modals: ModalView,
    menu_entries: Vec<String>,
    main_modal_texts: Vec<String>,
    chat_modal_texts: Vec<String>,
    login_message: String,
    count_dialog_open: bool,
    active_side_tab: i32,
    /// The client's varp table (one view per definition, from
    /// `Client.var`).
    varps: Vec<VarpView>,
    #[serde(skip)]
    inventory_gate: InvIfaceGate,
    #[serde(skip)]
    equipment_gate: InvIfaceGate,
    #[serde(skip)]
    bank_gate: InvIfaceGate,
    #[serde(skip)]
    bank_side_gate: InvIfaceGate,
    #[serde(skip)]
    trade_gate: InvIfaceGate,
    #[serde(skip)]
    widgets_gate: InvIfaceGate,
    #[serde(skip)]
    side_tabs_gate: InvIfaceGate,
    #[serde(skip)]
    chat_options_gate: u64,
    #[serde(skip)]
    make_products_gate: u64,
    #[serde(skip)]
    quest_statuses_gate: u64,
    #[serde(skip)]
    modals_gate: u64,
    #[serde(skip)]
    controls_gate: u64,
    #[serde(skip)]
    menu_gate: u64,
}

impl Default for GameSnapshot {
    /// An empty snapshot: no gens moved, no views. The "no such component
    /// or slot" sentinels default to -1 (never component 0) and the modal
    /// roots to "none".
    fn default() -> Self {
        GameSnapshot {
            gens: ClientGens::default(),
            npc: Vec::new(),
            player: None,
            players: Vec::new(),
            stats: Vec::new(),
            runenergy: 0,
            base: None,
            tile: None,
            self_slot: -1,
            tick: 0,
            inv: Vec::new(),
            chat: None,
            ingame: false,
            scene_state: 0,
            attached: false,
            loc: Vec::new(),
            ground_item: Vec::new(),
            scene: SceneView::default(),
            world: WorldStateView::default(),
            camera: CameraView::default(),
            map_flag: None,
            loc_gen: 0,
            ground_item_gen: 0,
            inventory: Vec::new(),
            equipment: Vec::new(),
            bank: Vec::new(),
            bank_side: Vec::new(),
            inventory_size: 0,
            bank_component_id: -1,
            trade: TradeView::default(),
            widgets: Vec::new(),
            side_tabs: Vec::new(),
            chat_lines: Vec::new(),
            chat_options: Vec::new(),
            chat_continue_component_id: -1,
            make_products: Vec::new(),
            quest_statuses: Vec::new(),
            run_controls: None,
            retaliate_controls: None,
            modals: ModalView {
                main: -1,
                side: -1,
                chat: -1,
                tutorial: -1,
            },
            menu_entries: Vec::new(),
            main_modal_texts: Vec::new(),
            chat_modal_texts: Vec::new(),
            login_message: String::new(),
            count_dialog_open: false,
            active_side_tab: 0,
            varps: Vec::new(),
            inventory_gate: InvIfaceGate::default(),
            equipment_gate: InvIfaceGate::default(),
            bank_gate: InvIfaceGate::default(),
            bank_side_gate: InvIfaceGate::default(),
            trade_gate: InvIfaceGate::default(),
            widgets_gate: InvIfaceGate::default(),
            side_tabs_gate: InvIfaceGate::default(),
            chat_options_gate: 0,
            make_products_gate: 0,
            quest_statuses_gate: 0,
            modals_gate: 0,
            controls_gate: 0,
            menu_gate: 0,
        }
    }
}

impl GameSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// The generation counters this snapshot reflects.
    pub fn gens(&self) -> ClientGens {
        self.gens
    }

    /// Rebuild `family` from `client` iff its gen moved since the last
    /// rebuild of that family. Returns true iff the gen moved. The npc/
    /// player/stat families rebuild their view caches; the rest track
    /// their counter so a later view can detect movement. `client` is
    /// borrowed immutably (the ground-item lists iterate through the
    /// `LinkList`'s shared iterator).
    pub fn rebuild_family(&mut self, client: &Client, family: Family) -> bool {
        match family {
            Family::Npc => self.rebuild_npcs(client),
            Family::Player => self.rebuild_player(client),
            Family::Inv => self.rebuild_inv(client),
            Family::Varp => self.rebuild_varps(client),
            Family::Stat => self.rebuild_stat(client),
            Family::Chat => self.rebuild_chat(client),
            Family::Scene => self.rebuild_scene(client),
            Family::Loc => self.rebuild_loc(client),
            Family::GroundItem => self.rebuild_ground_items(client),
            Family::Iface => track(client.gens.iface, &mut self.gens.iface),
            Family::Camera => self.rebuild_camera(client),
            Family::MapFlag => self.rebuild_map_flag(client),
            Family::World => self.rebuild_world(client),
            Family::Inventory => self.rebuild_inventory(client),
            Family::Equipment => self.rebuild_equipment(client),
            Family::Bank => self.rebuild_bank(client),
            Family::BankSide => self.rebuild_bank_side(client),
            Family::Trade => self.rebuild_trade(client),
            Family::Widgets => self.rebuild_widgets(client),
            Family::SideTabs => self.rebuild_side_tabs(client),
            Family::ChatOptions => self.rebuild_chat_options(client),
            Family::MakeProducts => self.rebuild_make_products(client),
            Family::QuestStatuses => self.rebuild_quest_statuses(client),
            Family::Modals => self.rebuild_modals(client),
            Family::Controls => self.rebuild_controls(client),
            Family::Menu => self.rebuild_menu(client),
        }
    }

    /// Rebuild every family whose gen moved (the harness "one snapshot
    /// per tick" read). Returns true iff any family gen moved.
    pub fn rebuild(&mut self, client: &Client) -> bool {
        let mut dirty = false;
        dirty |= self.rebuild_family(client, Family::Npc);
        dirty |= self.rebuild_family(client, Family::Player);
        dirty |= self.rebuild_family(client, Family::Inv);
        dirty |= self.rebuild_family(client, Family::Varp);
        dirty |= self.rebuild_family(client, Family::Stat);
        dirty |= self.rebuild_family(client, Family::Chat);
        dirty |= self.rebuild_family(client, Family::Scene);
        dirty |= self.rebuild_family(client, Family::Loc);
        dirty |= self.rebuild_family(client, Family::GroundItem);
        dirty |= self.rebuild_family(client, Family::Iface);
        dirty |= self.rebuild_family(client, Family::Camera);
        dirty |= self.rebuild_family(client, Family::MapFlag);
        dirty |= self.rebuild_family(client, Family::World);
        dirty |= self.rebuild_family(client, Family::Inventory);
        dirty |= self.rebuild_family(client, Family::Equipment);
        dirty |= self.rebuild_family(client, Family::Bank);
        dirty |= self.rebuild_family(client, Family::BankSide);
        dirty |= self.rebuild_family(client, Family::Trade);
        dirty |= self.rebuild_family(client, Family::Widgets);
        dirty |= self.rebuild_family(client, Family::SideTabs);
        dirty |= self.rebuild_family(client, Family::ChatOptions);
        dirty |= self.rebuild_family(client, Family::MakeProducts);
        dirty |= self.rebuild_family(client, Family::QuestStatuses);
        dirty |= self.rebuild_family(client, Family::Modals);
        dirty |= self.rebuild_family(client, Family::Controls);
        dirty |= self.rebuild_family(client, Family::Menu);
        dirty
    }

    /// NPC views from the last npc rebuild, in `npc_ids` order (not sorted
    /// slots).
    pub fn npcs(&self) -> &[NpcView] {
        &self.npc
    }

    /// The local player view from the last player rebuild; `None` before
    /// the first `PLAYER_INFO` decodes one.
    pub fn local_player(&self) -> Option<&LocalPlayerView> {
        self.player.as_ref()
    }

    /// Remote player views from the last player rebuild, in `player_ids`
    /// order (the local player lives on `local_player()`).
    pub fn players(&self) -> &[PlayerView] {
        &self.players
    }

    /// All 25 skill slots from the last stat rebuild.
    pub fn stats(&self) -> &[StatView] {
        &self.stats
    }

    /// The client's varp table from the last varp rebuild, one view per
    /// definition (unset values read 0).
    pub fn varps(&self) -> &[VarpView] {
        &self.varps
    }

    /// Last rebuilt run energy (stat family). `0` until a stat rebuild.
    pub fn runenergy(&self) -> i32 {
        self.runenergy
    }

    /// The world build origin `(x, z)` from the last player-family
    /// rebuild; `None` before any world built.
    pub fn base(&self) -> Option<(i32, i32)> {
        self.base
    }

    /// The local player's world tile `(x, z, level)` from the last
    /// player-family rebuild.
    pub fn tile(&self) -> Option<(i32, i32, i32)> {
        self.tile
    }

    /// The local player's slot (`Client.self_slot`) from the last
    /// player-family rebuild; -1 before the first `PLAYER_INFO`.
    pub fn self_slot(&self) -> i32 {
        self.self_slot
    }

    /// The game-tick count this snapshot reflects: one bump per
    /// `PLAYER_INFO` (the host's `should_emit_tick` edge).
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// The inventory view `(obj id, count)` from the last inv rebuild.
    pub fn inv(&self) -> &[(i32, i32)] {
        &self.inv
    }

    /// The stacked count of `id` across inventory slots (0 when absent).
    pub fn inv_count(&self, id: i32) -> i32 {
        self.inv
            .iter()
            .filter(|(oid, _)| *oid == id)
            .map(|(_, n)| *n)
            .sum()
    }

    /// The most recent chat line from the last chat rebuild.
    pub fn chat(&self) -> Option<&str> {
        self.chat.as_deref()
    }

    /// `Client.ingame` from the last scene rebuild.
    pub fn ingame(&self) -> bool {
        self.ingame
    }

    /// `Client.scene_state` from the last scene rebuild.
    pub fn scene_state(&self) -> i32 {
        self.scene_state
    }

    /// Socket state from the last scene rebuild: whether the slot is
    /// attached to a server connection.
    pub fn attached(&self) -> bool {
        self.attached
    }

    /// Placed locs from the last loc rebuild, in scene sweep order (per
    /// tile: wall, ground, ground decoration, wall decoration).
    pub fn locs(&self) -> &[LocView] {
        &self.loc
    }

    /// Ground-object stacks from the last ground-item rebuild.
    pub fn ground_items(&self) -> &[GroundItemView] {
        &self.ground_item
    }

    /// The built scene (base, level, collision flags) from the last scene
    /// rebuild; the default is "no scene available".
    pub fn scene(&self) -> &SceneView {
        &self.scene
    }

    /// The client's world scalars from the last world rebuild.
    pub fn world(&self) -> &WorldStateView {
        &self.world
    }

    /// The camera state from the last camera rebuild.
    pub fn camera(&self) -> &CameraView {
        &self.camera
    }

    /// The minimap flag from the last map-flag rebuild; `None` while no
    /// flag is set.
    pub fn map_flag(&self) -> Option<&MapFlagView> {
        self.map_flag.as_ref()
    }

    /// Inventory item views from the last inventory rebuild, in slot
    /// order (the inv tab's TYPE_INV component).
    pub fn inventory(&self) -> &[ItemView] {
        &self.inventory
    }

    /// The inv tab component's slot count from the last inventory rebuild
    /// (the m8aq `inventorySize`); 0 until the inv tab loads.
    pub fn inventory_size(&self) -> i32 {
        self.inventory_size
    }

    /// Worn-items views from the last equipment rebuild, in slot order.
    pub fn equipment(&self) -> &[ItemView] {
        &self.equipment
    }

    /// Bank item views from the last bank rebuild (the open main modal's
    /// withdraw component).
    pub fn bank(&self) -> &[ItemView] {
        &self.bank
    }

    /// The open main modal's withdraw component from the last bank
    /// rebuild; -1 while no bank is open.
    pub fn bank_component_id(&self) -> i32 {
        self.bank_component_id
    }

    /// Bank-side (deposit) item views from the last bank-side rebuild.
    pub fn bank_side(&self) -> &[ItemView] {
        &self.bank_side
    }

    /// The trade state (offer/confirm open, the four containers, partner).
    pub fn trade(&self) -> &TradeView {
        &self.trade
    }

    /// Widget views from the last widgets rebuild, one per component
    /// reachable from an open root.
    pub fn widgets(&self) -> &[WidgetView] {
        &self.widgets
    }

    /// Side-tab views (all 14 slots) from the last side-tabs rebuild.
    pub fn side_tabs(&self) -> &[SideTabView] {
        &self.side_tabs
    }

    /// Chat history from the last chat rebuild, newest first (ring order).
    pub fn chat_lines(&self) -> &[ChatLineView] {
        &self.chat_lines
    }

    /// The chat modal's BUTTON_OK choices from the last chat-options
    /// rebuild, in walk order.
    pub fn chat_options(&self) -> &[ChatOptionView] {
        &self.chat_options
    }

    /// The chat modal's BUTTON_CONTINUE component (-1 while the pause
    /// button is latched or no chat modal is open).
    pub fn chat_continue_component_id(&self) -> i32 {
        self.chat_continue_component_id
    }

    /// Make-X products from the last make-products rebuild.
    pub fn make_products(&self) -> &[MakeProductView] {
        &self.make_products
    }

    /// Quest-journal entries from the last quest-statuses rebuild.
    pub fn quest_statuses(&self) -> &[QuestStatusView] {
        &self.quest_statuses
    }

    /// The run-toggle pair from the last controls rebuild.
    pub fn run_controls(&self) -> Option<&ToggleControlsView> {
        self.run_controls.as_ref()
    }

    /// The auto-retaliate toggle pair from the last controls rebuild.
    pub fn retaliate_controls(&self) -> Option<&ToggleControlsView> {
        self.retaliate_controls.as_ref()
    }

    /// The four open modal roots from the last modals rebuild.
    pub fn modals(&self) -> &ModalView {
        &self.modals
    }

    /// The minimenu entries from the last menu rebuild.
    pub fn menu_entries(&self) -> &[String] {
        &self.menu_entries
    }

    /// The main modal's TYPE_TEXT lines from the last modals rebuild.
    pub fn main_modal_texts(&self) -> &[String] {
        &self.main_modal_texts
    }

    /// The chat modal's TYPE_TEXT lines from the last modals rebuild.
    pub fn chat_modal_texts(&self) -> &[String] {
        &self.chat_modal_texts
    }

    /// The login screen message (`login_mes1` + `login_mes2`) from the
    /// last menu rebuild.
    pub fn login_message(&self) -> &str {
        &self.login_message
    }

    /// Whether the enter-name/amount dialog is up from the last modals
    /// rebuild.
    pub fn count_dialog_open(&self) -> bool {
        self.count_dialog_open
    }

    /// The selected side tab from the last modals rebuild.
    pub fn active_side_tab(&self) -> i32 {
        self.active_side_tab
    }

    fn rebuild_stat(&mut self, client: &Client) -> bool {
        if !track(client.gens.stat, &mut self.gens.stat) {
            return false;
        }
        self.runenergy = client.runenergy;
        self.stats = (0..Skill::count)
            .map(|i| StatView {
                index: i as i32,
                name: Skill::names[i].to_string(),
                effective: client.stat_effective_level[i],
                base: client.stat_base_level[i],
                xp: client.stat_xp[i],
                used: Skill::used[i],
            })
            .collect();
        true
    }

    /// Varp-family rebuild: the whole `Client.var` table, one view per
    /// definition (unset entries read 0), gated on the varp gen.
    fn rebuild_varps(&mut self, client: &Client) -> bool {
        if !track(client.gens.varp, &mut self.gens.varp) {
            return false;
        }
        self.varps = (0..client.cache.varps.len())
            .map(|i| VarpView {
                index: i as i32,
                value: client.var.get(i).copied().unwrap_or(0),
            })
            .collect();
        true
    }

    /// Player-family rebuild: the scene origin, the local player's world
    /// tile (base + route head), the `LocalPlayerView`, and the remote
    /// `players` list. `REBUILD_NORMAL` bumps every gen, so a new world
    /// origin re-arms this too.
    fn rebuild_player(&mut self, client: &Client) -> bool {
        if !track(client.gens.player, &mut self.gens.player) {
            return false;
        }
        // One `PLAYER_INFO` per game tick: the snapshot's tick count.
        self.tick = self.tick.wrapping_add(1);
        self.self_slot = client.self_slot;
        let base = (client.map_build_base_x, client.map_build_base_z);
        self.base = Some(base);
        self.tile = client.local_player.as_ref().map(|lp| {
            (
                base.0 + lp.route_x[0],
                base.1 + lp.route_z[0],
                // The scene level (`minusedlevel`), like every actor view.
                client.minusedlevel,
            )
        });
        let level = client.minusedlevel;
        self.player = client.local_player.as_ref().map(|lp| LocalPlayerView {
            player: PlayerView {
                index: client.self_slot.max(0) as usize,
                actor: actor_view(
                    &lp.entity,
                    base,
                    level,
                    0, // the local player's distance to itself
                    lp.name.clone(),
                    client.player_op.to_vec(),
                ),
                combat_level: lp.combat_level,
                skill_level: lp.skill_level,
            },
            energy: client.runenergy,
            weight: client.runweight,
        });
        self.players.clear();
        self.players.reserve(client.player_count as usize);
        let local_tile = local_world_tile(client);
        for i in 0..client.player_count as usize {
            let index = client.player_ids[i] as usize;
            if let Some(player) = client.players.get(index).and_then(|p| p.as_ref()) {
                let tile = entity_world_tile(&player.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.players.push(PlayerView {
                    index,
                    actor: actor_view(
                        &player.entity,
                        base,
                        level,
                        distance,
                        player.name.clone(),
                        client.player_op.to_vec(),
                    ),
                    combat_level: player.combat_level,
                    skill_level: player.skill_level,
                });
            }
        }
        true
    }

    /// Inv-family rebuild: zip the TYPE_INV iface's obj ids/counts. The
    /// iface stores `obj_id + 1` (0 = empty), so the view carries the
    /// real obj ids — the same convention as `ItemView.def.id`, the
    /// `ObjNames` table and the evidence/`Proof::Item` consumers.
    fn rebuild_inv(&mut self, client: &Client) -> bool {
        if !track(client.gens.inv, &mut self.gens.inv) {
            return false;
        }
        self.inv.clear();
        // The live inv is the side-tab-3 TYPE_INV container (the same the
        // `inventory()` family reads); a naive first-TYPE_INV scan can
        // pick an unrelated empty container (a shop/trade modal) and fail
        // the nav `WorldState` gate closed. Tests/stub clients without a
        // side tab fall back to the first TYPE_INV that actually holds
        // decoded slots, else the first TYPE_INV in the table.
        let inv = tab_inv_component(client, 3)
            .and_then(|id| client.if_(id as usize))
            .or_else(|| {
                let mut first = None;
                for com in client
                    .ifaces_merged()
                    .filter(|f| f.r#type == ComponentType::TYPE_INV)
                {
                    if com
                        .link_obj_type
                        .as_ref()
                        .is_some_and(|ids| ids.iter().any(|id| *id > 0))
                    {
                        return Some(com);
                    }
                    if first.is_none() {
                        first = Some(com);
                    }
                }
                first
            });
        if let Some(inv) = inv {
            if let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) {
                self.inv = ids
                    .iter()
                    .zip(counts)
                    .filter(|(id, _)| **id > 0)
                    .map(|(id, n)| (*id - 1, *n))
                    .collect();
            }
        }
        true
    }

    /// Chat-family rebuild: the ring head (`chat_text[0]`) is the most
    /// recent message, and the full ring becomes the `chat_lines` view
    /// (index 0 = newest). Each line's `sequence` is the client's own
    /// per-message counter (`chat_seq` bumps once per `add_chat`), so a
    /// burst in one gen bump still gets distinct sequences that only move
    /// forward.
    fn rebuild_chat(&mut self, client: &Client) -> bool {
        if !track(client.gens.chat, &mut self.gens.chat) {
            return false;
        }
        let latest = client.chat_text[0].clone();
        self.chat = (!latest.is_empty()).then_some(latest);
        let head_seq = client.chat_seq as i32;
        self.chat_lines.clear();
        for i in 0..100 {
            let text = client.chat_text[i].clone();
            if text.is_empty() {
                break; // the ring is dense from the head (m8aq stops at the first hole)
            }
            self.chat_lines.push(ChatLineView {
                type_: client.chat_type[i],
                username: (!client.chat_username[i].is_empty())
                    .then(|| client.chat_username[i].clone()),
                text,
                sequence: head_seq - i as i32,
            });
        }
        true
    }

    /// Inventory rebuild: the inv tab's (side tab 3) TYPE_INV component,
    /// with held ops from the obj defs. Gated on the iface + inv gens.
    fn rebuild_inventory(&mut self, client: &Client) -> bool {
        if !self.inventory_gate.moved(client) {
            return false;
        }
        self.inventory.clear();
        let Some(inv_id) = tab_inv_component(client, 3) else {
            self.inventory_size = 0;
            return true;
        };
        let Some(inv) = client.if_(inv_id as usize) else {
            self.inventory_size = 0;
            return true;
        };
        self.inventory_size = inv
            .link_obj_type
            .as_ref()
            .map(|ids| ids.len() as i32)
            .unwrap_or(0);
        let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) else {
            return true;
        };
        for (slot, stored) in ids
            .iter()
            .copied()
            .enumerate()
            .take(ids.len().min(counts.len()))
        {
            if stored <= 0 {
                continue;
            }
            let id = stored - 1;
            if let Some(view) = item_view(
                &client.cache,
                &inv,
                slot,
                ItemContainer::Inventory,
                ItemActionFamily::Held,
                cache_held_ops(&client.cache, id),
            ) {
                self.inventory.push(view);
            }
        }
        true
    }

    /// Equipment rebuild: the worn-items tab's (side tab 4) TYPE_INV
    /// component with its own interface ops.
    fn rebuild_equipment(&mut self, client: &Client) -> bool {
        if !self.equipment_gate.moved(client) {
            return false;
        }
        self.equipment = tab_inv_component(client, 4)
            .and_then(|com_id| inv_items(client, com_id, ItemContainer::Equipment))
            .unwrap_or_default();
        true
    }

    /// Bank rebuild: the open main modal's withdraw component (m8aq
    /// `bankItems`).
    fn rebuild_bank(&mut self, client: &Client) -> bool {
        if !self.bank_gate.moved(client) {
            return false;
        }
        self.bank_component_id = if client.main_modal_id == -1 {
            -1
        } else {
            let root = client.main_modal_id;
            find_inv_component(client, root, |com| {
                com.iop[0]
                    .as_deref()
                    .is_some_and(|s| s.to_ascii_lowercase().contains("withdraw"))
            })
            .unwrap_or(-1)
        };
        self.bank = if self.bank_component_id == -1 {
            Vec::new()
        } else {
            inv_items(client, self.bank_component_id, ItemContainer::Bank).unwrap_or_default()
        };
        true
    }

    /// Bank-side rebuild: the open side modal's deposit component (m8aq
    /// `bankSideItems`).
    fn rebuild_bank_side(&mut self, client: &Client) -> bool {
        if !self.bank_side_gate.moved(client) {
            return false;
        }
        self.bank_side = if client.side_modal_id == -1 {
            Vec::new()
        } else {
            let root = client.side_modal_id;
            find_inv_component(client, root, |com| {
                com.iop[0]
                    .as_deref()
                    .is_some_and(|s| s.to_ascii_lowercase().contains("deposit"))
            })
            .and_then(|com_id| inv_items(client, com_id, ItemContainer::BankSide))
            .unwrap_or_default()
        };
        true
    }

    /// Trade rebuild: the 274 trade ifaces' state and containers. The
    /// component ids are baked by the packed interface table, so the
    /// reads work whether or not a trade is open (m8aq reads the same
    /// hardcoded ids).
    fn rebuild_trade(&mut self, client: &Client) -> bool {
        if !self.trade_gate.moved(client) {
            return false;
        }
        let my_offer =
            inv_items(client, TRADEMAIN_INV, ItemContainer::TradeMyOffer).unwrap_or_default();
        let their_offer = inv_items(client, TRADEMAIN_OTHER_INV, ItemContainer::TradeTheirOffer)
            .map(|items| {
                items
                    .into_iter()
                    .map(|mut item| {
                        item.action_family = ItemActionFamily::None;
                        item
                    })
                    .collect()
            })
            .unwrap_or_default();
        let side_pack =
            inv_items(client, TRADESIDE_INV, ItemContainer::TradeSidePack).unwrap_or_default();
        self.trade = TradeView {
            offer_open: client.main_modal_id == TRADEMAIN,
            confirm_open: client.main_modal_id == TRADECONFIRM,
            my_offer,
            their_offer,
            side_pack,
            partner: trade_partner(client),
        };
        true
    }

    /// Widgets rebuild: walk every open root's tree into `WidgetView`s.
    /// Gated on the iface gen (tree/component state) and the inv gen
    /// (TYPE_INV slot contents).
    fn rebuild_widgets(&mut self, client: &Client) -> bool {
        if !self.widgets_gate.moved(client) {
            return false;
        }
        self.widgets.clear();
        let roots = widget_roots(client);
        let mut visited = vec![false; client.ifaces_len()];
        for (root_id, root) in roots {
            walk_widget_tree(client, root_id, root, &mut visited, &mut self.widgets);
        }
        true
    }

    /// Side-tabs rebuild: all 14 slots with the tab state and each
    /// available tab's widget tree.
    fn rebuild_side_tabs(&mut self, client: &Client) -> bool {
        if !self.side_tabs_gate.moved(client) {
            return false;
        }
        self.side_tabs.clear();
        let mut visited = vec![false; client.ifaces_len()];
        for index in 0..client.side_icon.len() {
            let root_component_id = client.side_icon[index];
            let available = root_component_id != -1;
            let active = client.active_icon == index as i32;
            let mut widgets = Vec::new();
            if available {
                walk_widget_tree(
                    client,
                    root_component_id,
                    WidgetRoot::Side,
                    &mut visited,
                    &mut widgets,
                );
            }
            self.side_tabs.push(SideTabView {
                index: index as i32,
                root_component_id,
                available,
                active,
                visible: active && client.side_modal_id == -1 && available,
                widgets,
            });
        }
        true
    }

    /// Chat-options rebuild: the chat modal's BUTTON_OK choices and its
    /// BUTTON_CONTINUE component (the m8aq `chatOptions`/continue).
    fn rebuild_chat_options(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.chat_options_gate) {
            return false;
        }
        self.chat_options.clear();
        self.chat_continue_component_id = -1;
        if client.chat_modal_id == -1 {
            return true;
        }
        let root = client.chat_modal_id;
        // The continue button is a direct child of the chat modal.
        if let Some(children) = client.if_(root as usize)
            .and_then(|m| m.children.clone())
        {
            for child_id in children {
                if client.if_(child_id as usize)
                    .is_some_and(|c| c.button_type == ButtonType::BUTTON_CONTINUE)
                {
                    self.chat_continue_component_id = child_id;
                    break;
                }
            }
        }
        if client.resumed_pause_button {
            self.chat_continue_component_id = -1;
        }
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.button_type == ButtonType::BUTTON_OK {
                let label = if !com.text.is_empty() {
                    Some(com.text.to_string())
                } else {
                    non_empty(&com.button_text)
                };
                if let Some(text) = label {
                    self.chat_options.push(ChatOptionView {
                        component_id: id,
                        text,
                    });
                }
            }
            queue.extend(children_of(&com));
        }
        true
    }

    /// Make-products rebuild: the chat (or main) modal's obj-model
    /// components as products with their make/smelt buttons grouped four
    /// per product (m8aq `makeProducts`).
    fn rebuild_make_products(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.make_products_gate) {
            return false;
        }
        self.make_products.clear();
        let root = if client.chat_modal_id != -1 {
            client.chat_modal_id
        } else {
            client.main_modal_id
        };
        if root == -1 {
            return true;
        }
        let mut objs: Vec<i32> = Vec::new();
        let mut buttons: Vec<MakeButtonView> = Vec::new();
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.model1_type == 4 && com.model1_id > 0 {
                objs.push(com.model1_id);
            }
            if com.button_type == ButtonType::BUTTON_OK {
                if let Some(quantity) = make_quantity(&com.button_text) {
                    buttons.push(MakeButtonView {
                        quantity,
                        component_id: id,
                    });
                }
            }
            queue.extend(children_of(&com));
        }
        // Make-X groups four quantity buttons per obj-model. A modal can
        // carry TYPE_MODEL objs with no Make/Smelt buttons (the
        // mysterious-cube random event) — do not invent products, and never
        // slice `buttons[i*4..]` past the end.
        if buttons.is_empty() {
            return true;
        }
        for (i, obj) in objs.iter().enumerate() {
            let name = client
                .cache
                .objs
                .get(*obj as usize)
                .map(|o| o.name.clone())
                .unwrap_or_default();
            let start = i * 4;
            let chunk = if start >= buttons.len() {
                Vec::new()
            } else {
                let end = (start + 4).min(buttons.len());
                buttons[start..end].to_vec()
            };
            self.make_products.push(MakeProductView {
                object_id: *obj,
                name,
                buttons: chunk,
            });
        }
        true
    }

    /// Quest-statuses rebuild: the quest tab's (side tab 2) TYPE_TEXT
    /// rows with their colours (m8aq `questStatuses`).
    fn rebuild_quest_statuses(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.quest_statuses_gate) {
            return false;
        }
        self.quest_statuses.clear();
        let Some(root) = client.side_icon.get(2).copied() else {
            return true;
        };
        if root == -1 {
            return true;
        }
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.r#type == ComponentType::TYPE_TEXT && !com.text.is_empty() {
                self.quest_statuses.push(QuestStatusView {
                    component_id: id,
                    name: com.text.to_string(),
                    colour: com.colour,
                });
            }
            queue.extend(children_of(&com));
        }
        true
    }

    /// Controls rebuild: the run/retaliate toggle pairs from the
    /// player-controls overlay (a table scan — the overlay is a side tab,
    /// so its root is not among the widget roots).
    fn rebuild_controls(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.controls_gate) {
            return false;
        }
        self.run_controls = controls_pair(client, 5, 5, 4);
        self.retaliate_controls = controls_pair(client, 3, 2, 3);
        true
    }

    /// Modals rebuild: the four open roots plus the modal scalars. The
    /// scalar fields copy every rebuild (the count dialog and active tab
    /// flip locally with no packet), while the return value tracks the
    /// iface gen for the harness.
    fn rebuild_modals(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.iface, &mut self.modals_gate);
        self.modals = ModalView {
            main: client.main_modal_id,
            side: client.side_modal_id,
            chat: client.chat_modal_id,
            tutorial: client.tut_com_id,
        };
        self.count_dialog_open = client.dialog_input_open;
        self.active_side_tab = client.active_icon;
        self.main_modal_texts = modal_texts(client, client.main_modal_id);
        self.chat_modal_texts = modal_texts(client, client.chat_modal_id);
        moved
    }

    /// Menu rebuild: the minimenu entries and the login message. The menu
    /// is rebuilt locally every frame with no packet, so the entries copy
    /// every rebuild; the return value tracks the iface gen.
    fn rebuild_menu(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.iface, &mut self.menu_gate);
        let n = client.menu_num_entries.max(0) as usize;
        self.menu_entries = client.menu_option.iter().take(n).cloned().collect();
        let mut login = String::new();
        if !client.login_mes1.is_empty() {
            login.push_str(&client.login_mes1);
        }
        if !client.login_mes2.is_empty() {
            if !login.is_empty() {
                login.push('\n');
            }
            login.push_str(&client.login_mes2);
        }
        self.login_message = login;
        moved
    }

    /// Scene-family rebuild: `ingame` + `scene_state`, always fresh —
    /// these flip locally (`check_scene` sets `scene_state = 2` on the SIM
    /// loop with no gen bump), so a gen-gated copy would pin the snapshot
    /// in a stale "loading" state. The `SceneView` (build base, level and
    /// the collision grid) only changes on a world build, so it rebuilds
    /// when the scene gen moves. The return value still tracks the gen for
    /// the harness's dirty/tick semantics.
    fn rebuild_scene(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.scene, &mut self.gens.scene);
        self.ingame = client.ingame;
        self.scene_state = client.scene_state;
        self.attached = client.stream.is_some();
        if moved {
            let level = client.minusedlevel;
            match client.collision.get(level as usize) {
                Some(cmap) => {
                    self.scene = SceneView {
                        available: true,
                        base_x: client.map_build_base_x,
                        base_z: client.map_build_base_z,
                        level,
                        width: cmap.size_x,
                        height: cmap.size_z,
                        collision_flags: cmap.flags.iter().flatten().copied().collect(),
                    };
                }
                None => {
                    self.scene = SceneView {
                        available: false,
                        base_x: client.map_build_base_x,
                        base_z: client.map_build_base_z,
                        level,
                        ..SceneView::default()
                    };
                }
            }
        }
        moved
    }

    /// Loc-family rebuild: sweep the sim world's four layers at
    /// `minusedlevel` (locs sit on scene tiles, so the world tile is
    /// `base + scene` with no pixel conversion). The dirty flag still
    /// tracks the scene gen (loc packets bump it), but the sweep always
    /// runs — typecodes can change on the world after the observer already
    /// consumed that gen (map restamp after `REBUILD_NORMAL`, a door
    /// multiloc applied in the same drain). A gen-gated copy leaves nav
    /// reading the previous build's door. Same pattern as
    /// [`GameSnapshot::rebuild_scene`]'s always-fresh `scene_state`.
    fn rebuild_loc(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.scene, &mut self.loc_gen);
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.loc.clear();
        for sx in 0..104 {
            for sz in 0..104 {
                // Per-tile layer order matches the m8aq sweep: wall,
                // ground, ground decoration, wall decoration.
                if let Some(wall) = client.world.get_wall(level, sx, sz) {
                    self.loc.push(loc_view(
                        wall.typecode,
                        wall.typecode2 & 0xff,
                        LocLayer::Wall,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(sprite) = client.world.get_scene(level, sx, sz) {
                    self.loc.push(loc_view(
                        sprite.typecode,
                        sprite.typecode2 & 0xff,
                        LocLayer::Ground,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(gd) = client.world.get_gd(level, sx, sz) {
                    self.loc.push(loc_view(
                        gd.typecode,
                        gd.typecode2 & 0xff,
                        LocLayer::GroundDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
                if let Some(decor) = client.world.get_decor(level, sx, sz) {
                    self.loc.push(loc_view(
                        decor.typecode,
                        decor.typecode2 & 0xff,
                        LocLayer::WallDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        &client.cache,
                    ));
                }
            }
        }
        moved
    }

    /// Ground-item rebuild: iterate each `ground_obj` list at
    /// `minusedlevel` into a `GroundItemView` (obj definition, stack
    /// count, ground ops). The `LinkList`'s shared `for_each` iterator
    /// needs no mutable cursor. Gated on the scene gen — object packets
    /// bump it.
    fn rebuild_ground_items(&mut self, client: &Client) -> bool {
        if !track(client.gens.scene, &mut self.ground_item_gen) {
            return false;
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.ground_item.clear();
        for x in 0..104 {
            for z in 0..104 {
                let Some(list) = &client.ground_obj[level as usize][x as usize][z as usize] else {
                    continue;
                };
                let tile = WorldTile {
                    x: base.0 + x,
                    z: base.1 + z,
                    level,
                };
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                list.for_each(|obj| {
                    self.ground_item.push(GroundItemView {
                        def: item_def_view(&client.cache, obj.id),
                        count: obj.count,
                        actions: ground_ops(&client.cache, obj.id),
                        tile,
                        distance,
                    });
                });
            }
        }
        true
    }

    /// World-state rebuild: the client's world scalars. Cheap reads copy
    /// every rebuild (like the scene status), so counts stay fresh between
    /// world-gen bumps; the return value tracks the gen for the harness.
    fn rebuild_world(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.world, &mut self.gens.world);
        self.world = WorldStateView {
            map_base_x: client.map_build_base_x,
            map_base_z: client.map_build_base_z,
            level: client.minusedlevel,
            members: client.members_account != 0,
            multi_combat: client.in_multizone != 0,
            player_count: client.player_count,
            npc_count: client.npc_count,
            cycle: client.loop_cycle,
        };
        moved
    }

    /// Camera rebuild: the follow-camera eye, the orbit target and the
    /// cinematic flag. The follow camera eases every frame with no packet,
    /// so the cheap fields copy every rebuild; the return value tracks the
    /// gen.
    fn rebuild_camera(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.camera, &mut self.gens.camera);
        self.camera = CameraView {
            x: client.cam_x,
            y: client.cam_y,
            z: client.cam_z,
            pitch: client.cam_pitch,
            yaw: client.cam_yaw,
            orbit_pitch: client.orbit_camera_pitch,
            orbit_yaw: client.orbit_camera_yaw,
            cinematic: client.cinema_cam,
        };
        moved
    }

    /// Map-flag rebuild: the minimap destination flag, `Some` only while
    /// it is set. The flag flips on minimap clicks with no packet, so the
    /// view copies every rebuild; the return value tracks the gen.
    fn rebuild_map_flag(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.map_flag, &mut self.gens.map_flag);
        self.map_flag = (client.minimap_flag_x != 0).then_some(MapFlagView {
            lx: client.minimap_flag_x,
            lz: client.minimap_flag_z,
        });
        moved
    }

    fn rebuild_npcs(&mut self, client: &Client) -> bool {
        if client.gens.npc == self.gens.npc {
            return false;
        }
        self.gens.npc = client.gens.npc;
        self.npc.clear();
        self.npc.reserve(client.npc_count as usize);
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        for i in 0..client.npc_count as usize {
            let index = client.npc_ids[i] as usize;
            if let Some(npc) = client.npc.get(index).and_then(|n| n.as_ref()) {
                let tile = entity_world_tile(&npc.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.npc.push(NpcView::from_slot(
                    index,
                    npc,
                    base,
                    level,
                    distance,
                    &client.cache,
                ));
            }
        }
        true
    }
}

/// The borrowing read surface over one `GameSnapshot` (the m8aq
/// `ReadContext`/`ReadApi`): every accessor returns the last rebuild's
/// view without allocating. The query DSL (`api::query`) builds on these
/// slices; `component`/`varp`/`world_tile` do a cheap scan/derivation.
/// Copy so settle outcomes can hand the same context back out of a watch.
#[derive(Clone, Copy)]
pub struct ReadContext<'a>(&'a GameSnapshot);

impl<'a> ReadContext<'a> {
    pub fn new(snapshot: &'a GameSnapshot) -> Self {
        ReadContext(snapshot)
    }

    /// The game-tick count the snapshot reflects.
    pub fn tick(&self) -> u32 {
        self.0.tick()
    }

    /// Whether the slot is attached to a server connection.
    pub fn attached(&self) -> bool {
        self.0.attached()
    }

    /// `Client.ingame`.
    pub fn ingame(&self) -> bool {
        self.0.ingame()
    }

    /// `Client.scene_state`.
    pub fn scene_state(&self) -> i32 {
        self.0.scene_state()
    }

    /// The local player view.
    pub fn local_player(&self) -> Option<&LocalPlayerView> {
        self.0.local_player()
    }

    /// The local player's slot index.
    pub fn self_slot(&self) -> i32 {
        self.0.self_slot()
    }

    /// All 25 skill slots.
    pub fn stats(&self) -> &[StatView] {
        self.0.stats()
    }

    /// Live NPC views (in `npc_ids` order).
    pub fn npcs(&self) -> &[NpcView] {
        self.0.npcs()
    }

    /// Remote player views (the local player lives on `local_player`).
    pub fn players(&self) -> &[PlayerView] {
        self.0.players()
    }

    /// Placed locs from the last loc rebuild.
    pub fn locs(&self) -> &[LocView] {
        self.0.locs()
    }

    /// Ground-item stacks from the last ground-item rebuild.
    pub fn ground_items(&self) -> &[GroundItemView] {
        self.0.ground_items()
    }

    /// Inventory item views.
    pub fn inventory(&self) -> &[ItemView] {
        self.0.inventory()
    }

    /// Worn-items views.
    pub fn equipment(&self) -> &[ItemView] {
        self.0.equipment()
    }

    /// The inv tab's slot count.
    pub fn inventory_capacity(&self) -> i32 {
        self.0.inventory_size()
    }

    /// Bank (withdraw) item views.
    pub fn bank(&self) -> &[ItemView] {
        self.0.bank()
    }

    /// Bank-side (deposit) item views.
    pub fn bank_side_items(&self) -> &[ItemView] {
        self.0.bank_side()
    }

    /// The open main modal's withdraw component, -1 while no bank is open.
    pub fn bank_component_id(&self) -> i32 {
        self.0.bank_component_id()
    }

    /// The full chat ring, newest first (the snapshot's `chat()` head
    /// accessor stays the single most recent line).
    pub fn chat(&self) -> &[ChatLineView] {
        self.0.chat_lines()
    }

    /// The chat modal's BUTTON_OK choices.
    pub fn chat_options(&self) -> &[ChatOptionView] {
        self.0.chat_options()
    }

    /// The chat modal's BUTTON_CONTINUE component, -1 while latched or
    /// no chat modal is open.
    pub fn chat_continue_component_id(&self) -> i32 {
        self.0.chat_continue_component_id()
    }

    /// Make/smelt products.
    pub fn make_products(&self) -> &[MakeProductView] {
        self.0.make_products()
    }

    /// Quest-journal entries.
    pub fn quest_statuses(&self) -> &[QuestStatusView] {
        self.0.quest_statuses()
    }

    /// Widgets of the open roots' trees.
    pub fn widgets(&self) -> &[WidgetView] {
        self.0.widgets()
    }

    /// All 14 side-tab slots.
    pub fn side_tabs(&self) -> &[SideTabView] {
        self.0.side_tabs()
    }

    /// The widget with `component_id` among the open roots' trees and the
    /// side tabs; `None` when it is not part of an open widget.
    pub fn component(&self, component_id: i32) -> Option<&WidgetView> {
        self.0
            .widgets()
            .iter()
            .chain(self.0.side_tabs().iter().flat_map(|tab| tab.widgets.iter()))
            .find(|w| w.component_id == component_id)
    }

    /// The client's varp table.
    pub fn varps(&self) -> &[VarpView] {
        self.0.varps()
    }

    /// The client's world scalars.
    pub fn world(&self) -> &WorldStateView {
        self.0.world()
    }

    /// The built scene (collision grid).
    pub fn scene(&self) -> &SceneView {
        self.0.scene()
    }

    /// The camera state.
    pub fn camera(&self) -> &CameraView {
        self.0.camera()
    }

    /// The minimap flag from the last map-flag rebuild; `None` while no
    /// flag is set.
    pub fn map_flag(&self) -> Option<&MapFlagView> {
        self.0.map_flag()
    }

    /// The trade offer screen's own items.
    pub fn trade_my_offer(&self) -> &[ItemView] {
        &self.0.trade().my_offer
    }

    /// The trade partner's offered items.
    pub fn trade_their_offer(&self) -> &[ItemView] {
        &self.0.trade().their_offer
    }

    /// The side pack of tradeables.
    pub fn trade_side_pack(&self) -> &[ItemView] {
        &self.0.trade().side_pack
    }

    /// The four open modal roots.
    pub fn modals(&self) -> &ModalView {
        self.0.modals()
    }

    /// Whether the enter-name/amount dialog is up.
    pub fn count_dialog_open(&self) -> bool {
        self.0.count_dialog_open()
    }

    /// The selected side tab.
    pub fn active_side_tab(&self) -> i32 {
        self.0.active_side_tab()
    }

    /// The login screen message.
    pub fn login_message(&self) -> &str {
        self.0.login_message()
    }

    /// The minimenu entries.
    pub fn menu_entries(&self) -> &[String] {
        self.0.menu_entries()
    }

    /// The main modal's TYPE_TEXT lines.
    pub fn main_modal_texts(&self) -> &[String] {
        self.0.main_modal_texts()
    }

    /// The chat modal's TYPE_TEXT lines.
    pub fn chat_modal_texts(&self) -> &[String] {
        self.0.chat_modal_texts()
    }

    /// The run toggle pair.
    pub fn run_controls(&self) -> Option<&ToggleControlsView> {
        self.0.run_controls()
    }

    /// The auto-retaliate toggle pair.
    pub fn retaliate_controls(&self) -> Option<&ToggleControlsView> {
        self.0.retaliate_controls()
    }

    /// The local player's world tile — the canonical route-based tile
    /// (`base + route head`) with the scene level; `None` before the
    /// first `PLAYER_INFO`.
    pub fn world_tile(&self) -> Option<WorldTile> {
        self.0.tile().map(|(x, z, level)| WorldTile { x, z, level })
    }

    /// The value of varp `index` (0 when unset).
    pub fn varp(&self, index: i32) -> i32 {
        self.0
            .varps()
            .iter()
            .find(|v| v.index == index)
            .map(|v| v.value)
            .unwrap_or(0)
    }

    /// The TYPE_INV slot views of `component_id`, empty when the
    /// component is not an open widget (or holds no items).
    pub fn component_items(&self, component_id: i32) -> &[ItemView] {
        self.component(component_id)
            .map(|w| w.items.as_slice())
            .unwrap_or(&[])
    }

    /// The widget's text, `None` when the component is not an open widget
    /// or has no text.
    pub fn component_text(&self, component_id: i32) -> Option<&str> {
        self.component(component_id).and_then(|w| w.text.as_deref())
    }

    /// The obj-model id of `component_id` (model type 4), `None`
    /// otherwise.
    pub fn component_model_obj_id(&self, component_id: i32) -> Option<i32> {
        self.component(component_id)
            .filter(|w| w.model_type == 4)
            .map(|w| w.model_id)
    }

    /// The root component drawn on side tab `tab`, -1 when unbound.
    pub fn side_tab_interface(&self, tab: i32) -> i32 {
        self.0
            .side_tabs()
            .iter()
            .find(|t| t.index == tab)
            .map(|t| t.root_component_id)
            .unwrap_or(-1)
    }
}

/// The local player's world tile `(x, z)` from the live client (build
/// base + route head); `None` before the first `PLAYER_INFO`.
fn local_world_tile(client: &Client) -> Option<(i32, i32)> {
    client.local_player.as_ref().map(|lp| {
        (
            client.map_build_base_x + lp.route_x[0],
            client.map_build_base_z + lp.route_z[0],
        )
    })
}

fn chebyshev(ax: i32, az: i32, bx: i32, bz: i32) -> i32 {
    (ax - bx).abs().max((az - bz).abs())
}

/// The absolute world tile of an entity: its scene-local pixel coords
/// (`route * 128 + size * 64`) un-scaled by 128 and offset by the build
/// base, so every actor view is in the same world-tile space as the local
/// player's tile.
fn entity_world_tile(entity: &ClientEntity, base: (i32, i32), level: i32) -> WorldTile {
    WorldTile {
        x: base.0 + (entity.x - entity.size * 64) / 128,
        z: base.1 + (entity.z - entity.size * 64) / 128,
        level,
    }
}

/// Resolve `face_entity` with the client's own scheme (`entity_face` in
/// `client.rs`): slots below 32768 are NPC table indexes, at or above are
/// player slots offset by 32768. The player slot stays the raw server
/// slot (the client only rewrites `self_slot` → 2047 for its internal
/// turn lookup), so `targetingPlayer(self_slot)` matches the local player.
fn decode_target(face_entity: i32) -> Option<ActorTargetView> {
    if face_entity == -1 {
        return None;
    }
    if face_entity < 32768 {
        Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: face_entity as usize,
        })
    } else {
        Some(ActorTargetView {
            kind: ActorKind::Player,
            index: (face_entity - 32768) as usize,
        })
    }
}

/// The shared actor fields from one entity, as the m8aq `ActorSnapshot`.
fn actor_view(
    entity: &ClientEntity,
    base: (i32, i32),
    level: i32,
    distance: i32,
    name: Option<String>,
    actions: Vec<Option<String>>,
) -> ActorView {
    ActorView {
        name,
        actions,
        tile: entity_world_tile(entity, base, level),
        distance,
        animation: entity.primary_anim,
        pose_animation: entity.secondary_anim,
        orientation: entity.yaw,
        target_orientation: entity.dst_yaw,
        overhead_text: entity.chat_message.clone(),
        spot_animation: entity.spotanim_id,
        health: entity.health,
        total_health: entity.total_health,
        face_entity: entity.face_entity,
        target: decode_target(entity.face_entity),
        moving: entity.route_length > 0,
        running: entity.primary_anim == entity.runanim,
        in_combat: entity.combat_cycle > 0,
    }
}

/// One `LocView` from a placed layer: decode the loc id and the
/// shape/angle info byte, then resolve the definition from the loc table.
/// An unloaded loc id reads the `LocType` defaults (the m8aq `LocType.list`
/// dummy type).
#[allow(clippy::too_many_arguments)]
fn loc_view(
    typecode: i32,
    info: i32,
    layer: LocLayer,
    base: (i32, i32),
    level: i32,
    sx: i32,
    sz: i32,
    local_tile: Option<(i32, i32)>,
    cache: &Cache,
) -> LocView {
    let id = (typecode >> 14) & 0x7fff;
    let shape = info & 0x1f;
    let angle = (info >> 6) & 0x3;
    let x = base.0 + sx;
    let z = base.1 + sz;
    let (
        name,
        description,
        actions,
        width,
        length,
        block_walk,
        block_range,
        active,
        animation,
        map_function,
        map_scene,
        force_approach,
    ) = match cache.locs.get(id as usize) {
        Some(loc) => (
            (!loc.name.is_empty()).then(|| loc.name.clone()),
            (!loc.desc.is_empty()).then(|| loc.desc.clone()),
            loc.op.clone(),
            loc.width,
            loc.length,
            loc.blockwalk,
            loc.blockrange,
            loc.active,
            loc.anim,
            loc.mapfunction,
            loc.mapscene,
            loc.forceapproach,
        ),
        None => (
            None,
            None,
            Vec::new(),
            1,
            1,
            true,
            true,
            false,
            -1,
            -1,
            -1,
            0,
        ),
    };
    LocView {
        typecode,
        info,
        id,
        name,
        description,
        actions,
        tile: WorldTile { x, z, level },
        distance: local_tile
            .map(|(lx, lz)| chebyshev(x, z, lx, lz))
            .unwrap_or(0),
        layer,
        shape,
        angle,
        width,
        length,
        // A 90°/270° rotation swaps the footprint axes.
        footprint_width: if angle == 1 || angle == 3 {
            length
        } else {
            width
        },
        footprint_length: if angle == 1 || angle == 3 {
            width
        } else {
            length
        },
        block_walk,
        block_range,
        active,
        animation,
        map_function,
        map_scene,
        force_approach,
    }
}

/// The obj's definition view via Task 1's mapping. An unloaded obj id
/// reads the `ObjType` defaults with the requested id (the m8aq
/// `ObjType.list` dummy type).
fn item_def_view(cache: &Cache, id: i32) -> ItemDefView {
    let mut def = match cache.objs.get(id as usize) {
        Some(o) => ItemDefView::from_obj(o),
        None => ItemDefView::from_obj(&ObjType::default()),
    };
    def.id = id;
    def
}

/// The ground menu ops for an obj: the type's `op` table padded to five
/// slots with a `Take` default filling an empty third (m8aq `groundOps`).
fn ground_ops(cache: &Cache, id: i32) -> Vec<Option<String>> {
    let mut ops = cache
        .objs
        .get(id as usize)
        .map(|o| o.op.to_vec())
        .unwrap_or_else(|| vec![None; 5]);
    if ops.len() < 3 {
        ops.resize(3, None);
    }
    if ops[2].is_none() {
        ops[2] = Some("Take".into());
    }
    ops
}

fn track(world: u64, tracked: &mut u64) -> bool {
    if *tracked == world {
        return false;
    }
    *tracked = world;
    true
}

/// The two-counter gate of the item-bearing iface families: rebuild when
/// either the iface or the inv gen moved (component ids on the iface gen,
/// TYPE_INV slot data on the inv gen).
#[derive(Default, Clone, Copy)]
struct InvIfaceGate {
    iface: u64,
    inv: u64,
}

impl InvIfaceGate {
    fn moved(&mut self, client: &Client) -> bool {
        let moved = client.gens.iface != self.iface || client.gens.inv != self.inv;
        self.iface = client.gens.iface;
        self.inv = client.gens.inv;
        moved
    }
}

/// `Some(s)` for a non-empty string (iface strings are empty when unset).
fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// A component's children list, empty when it has none.
fn children_of(com: &IfType) -> &[i32] {
    com.children.as_deref().unwrap_or_default()
}

/// The open widget roots: the main modal and overlay (both draw above the
/// game view — the main modal carries the trade/bank/dialog tree), the
/// side modal (else the active tab's interface), the chat modal and the
/// tutorial overlay. Roots that are not cleanly resolvable (id not in the
/// table) are skipped by the walk; every emitted widget keeps the root id
/// and tag it was walked under.
fn widget_roots(client: &Client) -> Vec<(i32, WidgetRoot)> {
    let mut roots = Vec::new();
    if client.main_modal_id != -1 {
        roots.push((client.main_modal_id, WidgetRoot::Main));
    }
    if client.main_overlay_id != -1 {
        roots.push((client.main_overlay_id, WidgetRoot::Main));
    }
    let side_root = if client.side_modal_id != -1 {
        client.side_modal_id
    } else {
        client
            .side_icon
            .get(client.active_icon.max(0) as usize)
            .copied()
            .unwrap_or(-1)
    };
    if side_root != -1 {
        roots.push((side_root, WidgetRoot::Side));
    }
    if client.chat_modal_id != -1 {
        roots.push((client.chat_modal_id, WidgetRoot::Chat));
    }
    if client.tut_com_id != -1 {
        roots.push((client.tut_com_id, WidgetRoot::Tutorial));
    }
    roots
}

/// Walk one widget root's tree into `out`, tagging every component with
/// `root`/`root_component_id`. `visited` is shared across roots so a
/// component reachable from two open roots belongs to the first that
/// walks it (its ancestor chain reaches that root first). Positions
/// accumulate `child_x`/`child_y` from the root (m8aq
/// `walkPositionedComponents`); the root itself has parent -1.
fn walk_widget_tree(
    client: &Client,
    root_id: i32,
    root: WidgetRoot,
    visited: &mut [bool],
    out: &mut Vec<WidgetView>,
) {
    let mut queue: Vec<(i32, i32, i32, i32)> = vec![(root_id, -1, 0, 0)];
    let mut head = 0;
    while head < queue.len() {
        let (id, parent_id, x, y) = queue[head];
        head += 1;
        if id < 0 || (id as usize) >= visited.len() || visited[id as usize] {
            continue;
        }
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        visited[id as usize] = true;
        out.push(widget_view(client, &com, id, parent_id, root_id, root, x, y));
        if let Some(children) = &com.children {
            for (i, child) in children.iter().enumerate() {
                let cx = com
                    .child_x
                    .as_ref()
                    .and_then(|xs| xs.get(i))
                    .copied()
                    .unwrap_or(0);
                let cy = com
                    .child_y
                    .as_ref()
                    .and_then(|ys| ys.get(i))
                    .copied()
                    .unwrap_or(0);
                queue.push((*child, id, x + cx, y + cy));
            }
        }
    }
}

/// One `WidgetView` from a component plus its walk context. `component_id`
/// is the table id the walk found the component under (matches `com.id`
/// for well-formed ifaces).
#[allow(clippy::too_many_arguments)]
fn widget_view(
    client: &Client,
    com: &IfTypeView,
    component_id: i32,
    parent_id: i32,
    root_component_id: i32,
    root: WidgetRoot,
    x: i32,
    y: i32,
) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: com.layer_id,
        parent_id,
        root_component_id,
        root,
        type_: com.r#type,
        button_type: com.button_type,
        client_code: com.client_code,
        x,
        y,
        width: com.width,
        height: com.height,
        scroll_height: com.scroll_height,
        scroll_position: com.scroll_pos,
        hidden: com.hide,
        text: non_empty(&com.text),
        alternate_text: non_empty(&com.text2),
        button_text: non_empty(&com.button_text),
        target_verb: non_empty(&com.target_verb),
        target_base: non_empty(&com.target_base),
        target_mask: com.target_mask,
        model_type: com.model1_type,
        model_id: com.model1_id,
        alternate_model_type: com.model2_type,
        alternate_model_id: com.model2_id,
        scripts: com
            .scripts
            .clone()
            .map(|scripts| scripts.into_iter().map(Some).collect()),
        script_comparators: com.script_comparator.clone(),
        script_operands: com.script_operand.clone(),
        varp_bindings: varp_bindings(com),
        colour: com.colour,
        actions: com.iop.to_vec(),
        items: if com.r#type == ComponentType::TYPE_INV {
            read_inv_component(&client.cache, com, ItemContainer::Widget)
        } else {
            Vec::new()
        },
    }
}

/// The varp-bound scripts of a component: opcode-5 (`IF_VARP`) scripts
/// whose first operand is the varp, with the per-script comparator and
/// operand (m8aq `widgetVarpBindings`; the client's own toggle/select
/// arms decode `scripts[0][0] === 5` the same way).
fn varp_bindings(com: &IfType) -> Vec<WidgetVarpBindingView> {
    let mut out = Vec::new();
    let Some(scripts) = &com.scripts else {
        return out;
    };
    for (i, script) in scripts.iter().enumerate() {
        if script.len() >= 2 && script[0] == 5 {
            out.push(WidgetVarpBindingView {
                script_index: i as i32,
                varp: script[1],
                value: com.script_operand.as_ref().and_then(|o| o.get(i)).copied(),
                comparator: com
                    .script_comparator
                    .as_ref()
                    .and_then(|c| c.get(i))
                    .copied(),
            });
        }
    }
    out
}

/// One `ItemView` from a stored slot (`link_obj_type` holds `obj_id + 1`,
/// 0 = empty), with the given ops.
fn item_view(
    cache: &Cache,
    com: &IfTypeView,
    slot: usize,
    container: ItemContainer,
    action_family: ItemActionFamily,
    actions: Vec<Option<String>>,
) -> Option<ItemView> {
    let stored = com.link_obj_type.as_ref()?.get(slot).copied()?;
    if stored <= 0 {
        return None;
    }
    let id = stored - 1;
    Some(ItemView {
        def: item_def_view(cache, id),
        container,
        action_family,
        slot: slot as i32,
        count: com
            .link_obj_number
            .as_ref()
            .and_then(|n| n.get(slot))
            .copied()
            .unwrap_or(0),
        actions,
        component_id: com.id,
    })
}

/// The `ItemView`s of a TYPE_INV component's slots (m8aq
/// `readInvComponent`), with ops from the component's own `iop`.
fn read_inv_component(cache: &Cache, com: &IfTypeView, container: ItemContainer) -> Vec<ItemView> {
    let mut out = Vec::new();
    let Some(ids) = &com.link_obj_type else {
        return out;
    };
    let n = com.link_obj_number.as_ref().map(|n| n.len()).unwrap_or(0);
    for (slot, stored) in ids.iter().copied().enumerate().take(ids.len().min(n)) {
        if stored <= 0 {
            continue;
        }
        if let Some(view) = item_view(
            cache,
            com,
            slot,
            container,
            ItemActionFamily::Component,
            com.iop.to_vec(),
        ) {
            out.push(view);
        }
    }
    out
}

/// The items of the iface-table component `com_id` (component ops).
fn inv_items(client: &Client, com_id: i32, container: ItemContainer) -> Option<Vec<ItemView>> {
    if com_id < 0 {
        return None;
    }
    let com = client.if_(com_id as usize)?;
    Some(read_inv_component(&client.cache, &com, container))
}

/// The held-item ops for obj `id`: the type's `iop` padded to five slots
/// with a `Drop` default in the fifth (m8aq `heldOps`).
fn cache_held_ops(cache: &Cache, id: i32) -> Vec<Option<String>> {
    let mut ops = cache
        .objs
        .get(id as usize)
        .map(|o| o.iop.to_vec())
        .unwrap_or_else(|| vec![None; 5]);
    if ops.len() < 5 {
        ops.resize(5, None);
    }
    if ops[4].is_none() {
        ops[4] = Some("Drop".into());
    }
    ops
}

/// The TYPE_INV component of side tab `tab` (m8aq `findTabInvComponent`):
/// tab 4 (worn items) accepts any TYPE_INV, the other tabs need `obj_ops`.
fn tab_inv_component(client: &Client, tab: usize) -> Option<i32> {
    let root = client.side_icon.get(tab).copied().unwrap_or(-1);
    if root == -1 {
        return None;
    }
    find_inv_component(client, root, |com| com.obj_ops || tab == 4)
}

/// Depth-first search for a TYPE_INV component satisfying `accept` under
/// `root_id` (m8aq `findInvComponentIn`).
fn find_inv_component<F>(client: &Client, root_id: i32, accept: F) -> Option<i32>
where
    F: Fn(&IfType) -> bool,
{
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        if com.r#type == ComponentType::TYPE_INV && accept(&com) {
            return Some(id);
        }
        queue.extend(children_of(&com));
    }
    None
}

/// The 274 trade iface ids (the packed `interface.order` allocation):
/// trademain 3323 (offer screen), tradeconfirm 3443, trademain:inv 3415,
/// trademain:otherinv 3416, trademain:otherplayer 3417, tradeside:inv
/// 3322. The m8aq adapter reads the same hardcoded ids.
const TRADEMAIN: i32 = 3323;
const TRADECONFIRM: i32 = 3443;
const TRADEMAIN_INV: i32 = 3415;
const TRADEMAIN_OTHER_INV: i32 = 3416;
const TRADEMAIN_OTHER_PLAYER: i32 = 3417;
const TRADESIDE_INV: i32 = 3322;

/// The trade partner's name: the `otherplayer` label ("Trading With: X")
/// with the prefix stripped and whitespace trimmed (m8aq
/// `normalizeTradePartner`); `None` for an empty label.
fn trade_partner(client: &Client) -> Option<String> {
    let text = client.if_(TRADEMAIN_OTHER_PLAYER as usize)
        .map(|c| c.text.to_string())
        .unwrap_or_default();
    let name = match text.find(':') {
        Some(colon) => text[colon + 1..].trim(),
        None => text.trim(),
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The TYPE_TEXT contents of a modal tree, in walk order (m8aq
/// `mainModalTexts`/`chatModalTexts`).
fn modal_texts(client: &Client, root: i32) -> Vec<String> {
    let mut out = Vec::new();
    if root == -1 {
        return out;
    }
    let mut queue = vec![root];
    let mut head = 0;
    while head < queue.len() {
        let id = queue[head];
        head += 1;
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        if com.r#type == ComponentType::TYPE_TEXT && !com.text.is_empty() {
            out.push(com.text.to_string());
        }
        queue.extend(children_of(&com));
    }
    out
}

/// The toggle pair of the player-controls overlay: the root with an
/// "Auto retaliate" label among its children, reading `on_index`/
/// `off_index` from its children list (m8aq `runControls` reads 5/4 for
/// run, `readRetaliateControls` 2/3 for retaliate — the 274 `controls.if`
/// com_2/com_3 and com_4/com_5 buttons).
fn controls_pair(
    client: &Client,
    min_children: usize,
    on_index: usize,
    off_index: usize,
) -> Option<ToggleControlsView> {
    for com in client.ifaces_merged() {
        let Some(children) = &com.children else {
            continue;
        };
        let has_retaliate = children.iter().any(|id| {
            client.if_(*id as usize)
                .is_some_and(|c| c.text == "Auto retaliate")
        });
        if !has_retaliate || children.len() <= min_children {
            continue;
        }
        let on = children.get(on_index).copied().unwrap_or(-1);
        let off = children.get(off_index).copied().unwrap_or(-1);
        if on < 0
            || off < 0
            || client.if_(on as usize).is_none()
            || client.if_(off as usize).is_none()
        {
            return None;
        }
        return Some(ToggleControlsView {
            on_component_id: on,
            off_component_id: off,
        });
    }
    None
}

/// The make/smelt button quantity: `"Make X"`/`"Smelt X"` reads -1,
/// `"Make 10"` reads 10 (m8aq's button regex).
fn make_quantity(button_text: &str) -> Option<i32> {
    let lower = button_text.to_ascii_lowercase();
    let start = lower.find("make ").or_else(|| lower.find("smelt "))?;
    let rest = lower[start..].trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if token == "x" {
        Some(-1)
    } else {
        token.parse::<i32>().ok()
    }
}

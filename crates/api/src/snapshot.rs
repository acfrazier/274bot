//! Generation-stamped snapshot families. `GameSnapshot` owns per-family
//! views rebuilt only when that family's gen moved; reads borrow the last
//! rebuild instead of deep-copying the world on every read.

use crate::obj_names::ItemDefView;
use client::client::{Client, ClientGens, ClientNpc, Skill};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeView};
use client::config::{Cache, ObjType};
use client::dash3d::client_entity::ClientEntity;
use serde::Serialize;
mod views;
pub use views::*;
mod decode;
pub use decode::{attacked_by_player, cache_held_ops, tab_inv_component, PLAYER_FACE_BASE};
use decode::{empty_loc_model_stamp, loc_dirty_bits, track, BankInvSession, InvIfaceGate};
mod context;
pub use context::ReadContext;



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
    /// The local actor's native overhead text. This is refreshed on every
    /// snapshot read because local send/expiry changes do not bump a family.
    #[serde(skip)]
    local_overhead_text: Option<String>,
    /// Whether the local player currently shows an active positive combat
    /// hitmark (value > 0, type 1, cycle still ahead of `loop_cycle`). Refreshed
    /// every snapshot read so splat expiry stays current without a player-gen
    /// advance; fail-closed when logged out or no local player.
    #[serde(skip)]
    taking_damage: bool,
    // Native observation history; deliberately absent from serialized snapshots.
    #[serde(skip)]
    thieving_stun_tick: Option<u32>,
    #[serde(skip)]
    thieving_stun_stamp: Option<i32>,
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
    /// The client's current coordinate hint. NPC/player/no hint kinds are
    /// absent; this is refreshed on every snapshot read because HINT_ARROW
    /// has no dedicated generation family.
    hint_tile: Option<HintTileView>,
    /// Scene gen the loc/ground-item views were rebuilt up to. Loc and
    /// ground-item changes bump `gens.scene`, so both track it here
    /// (separately from the scene family's own counter).
    #[serde(skip)]
    loc_gen: u64,
    /// Aggregated tile `model_stamp` from the last loc sweep. Locs can
    /// change without a scene gen bump (door multiloc, map restamp), so
    /// the cheap stamp gates the 104×104×4 sweep between gen moves.
    #[serde(skip)]
    loc_model_stamp: u64,
    /// World static-scenery mutation generation from the last loc sweep.
    /// Ground loc add/del does not bump tile stamps, and LOC_DEL's scene
    /// gen can be consumed before `loc_change_do_queue` applies.
    #[serde(skip)]
    loc_static_gen: u64,
    /// One complete-rebuild fold of `loc_dirty_bits`. Standalone Scene/Loc
    /// compute the pair themselves; this is never a cross-tick cache.
    #[serde(skip)]
    loc_bits_prelude: Option<(u64, u64)>,
    /// Local-player tile the cached `LocView.distance` scalars were last
    /// written for. Player ticks refresh those integers in place when this
    /// origin moves, without cloning loc names or re-sweeping the world.
    #[serde(skip)]
    loc_distance_tile: Option<(i32, i32)>,
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
    /// The open puzzle board's identified TYPE_INV component (the first
    /// depth-first component with `obj_ops` under the main modal); -1 while
    /// no board is open.
    puzzle_board_component_id: i32,
    /// That component's `link_obj_type` slot count, as observed (never the
    /// row count).
    puzzle_board_size: i32,
    /// Puzzle-session identity: bumps when the modal session opens, closes,
    /// or the board component changes — never on a piece move.
    puzzle_session_generation: u64,
    /// Snapshot-local identity for the current bank session. This advances
    /// whenever the selected withdraw component opens, closes, or changes.
    bank_session_generation: u64,
    #[serde(skip)]
    bank_modal_generation_seen: u64,
    /// True when the open withdraw component is transmitting a full inventory
    /// snapshot whose last-full generation is at least the inv generation
    /// recorded at the previous close (0 if this component was never closed).
    bank_loaded: bool,
    #[serde(skip)]
    bank_inventory_session: Option<BankInvSession>,
    #[serde(skip)]
    bank_prev_inv_com: i32,
    #[serde(skip)]
    bank_prev_inv_generation: u64,
    #[serde(skip)]
    bank_full_generation: u64,
    #[serde(skip)]
    bank_full_com: i32,
    #[serde(skip)]
    bank_last_inv_com: i32,
    #[serde(skip)]
    bank_last_inv_generation: u64,
    #[serde(skip)]
    bank_last_full_observation: u64,
    trade: TradeView,
    shop: ShopView,
    widgets: Vec<WidgetView>,
    side_tabs: Vec<SideTabView>,
    chat_lines: Vec<ChatLineView>,
    chat_options: Vec<ChatOptionView>,
    chat_continue_component_id: i32,
    make_products: Vec<MakeProductView>,
    /// Main-modal TYPE_INV rows whose component ops start with Make.
    main_make: Vec<ItemView>,
    quest_statuses: Vec<QuestStatusView>,
    quest_statuses_available: bool,
    run_controls: Option<ToggleControlsView>,
    retaliate_controls: Option<ToggleControlsView>,
    /// The Note/Item toggle pair on the open bank main modal; None while
    /// no bank is open or the buttons are absent.
    bank_note_controls: Option<ToggleControlsView>,
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
    shop_gate: InvIfaceGate,
    #[serde(skip)]
    widgets_gate: InvIfaceGate,
    #[serde(skip)]
    side_tabs_gate: InvIfaceGate,
    #[serde(skip)]
    chat_options_gate: u64,
    #[serde(skip)]
    make_products_gate: u64,
    #[serde(skip)]
    main_make_gate: InvIfaceGate,
    /// Whether the main modal was open at the last board refresh (the
    /// session edge the generation advances on).
    #[serde(skip)]
    puzzle_board_open: bool,
    #[serde(skip)]
    quest_statuses_gate: u64,
    #[serde(skip)]
    modals_gate: u64,
    #[serde(skip)]
    controls_gate: u64,
    #[serde(skip)]
    menu_gate: u64,
    /// False only after a session reset, until that session publishes an
    /// inventory packet. It prevents later unrelated iface updates from
    /// reviving retained item tables from the previous session.
    #[serde(skip)]
    inv_session_current: bool,
    #[serde(skip)]
    session_start: Option<ClientGens>,
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
            local_overhead_text: None,
            taking_damage: false,
            thieving_stun_tick: None,
            thieving_stun_stamp: None,
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
            hint_tile: None,
            loc_gen: 0,
            loc_model_stamp: empty_loc_model_stamp(),
            loc_static_gen: 0,
            loc_bits_prelude: None,
            loc_distance_tile: None,
            ground_item_gen: 0,
            inventory: Vec::new(),
            equipment: Vec::new(),
            bank: Vec::new(),
            bank_side: Vec::new(),
            inventory_size: 0,
            bank_component_id: -1,
            puzzle_board_component_id: -1,
            puzzle_board_size: 0,
            puzzle_session_generation: 0,
            bank_session_generation: 0,
            bank_modal_generation_seen: 0,
            bank_loaded: false,
            bank_inventory_session: None,
            bank_prev_inv_com: -1,
            bank_prev_inv_generation: 0,
            bank_full_generation: 0,
            bank_full_com: -1,
            bank_last_inv_com: -1,
            bank_last_inv_generation: 0,
            bank_last_full_observation: 0,
            trade: TradeView::default(),
            shop: ShopView::default(),
            widgets: Vec::new(),
            side_tabs: Vec::new(),
            chat_lines: Vec::new(),
            chat_options: Vec::new(),
            chat_continue_component_id: -1,
            make_products: Vec::new(),
            main_make: Vec::new(),
            quest_statuses: Vec::new(),
            quest_statuses_available: false,
            run_controls: None,
            retaliate_controls: None,
            bank_note_controls: None,
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
            shop_gate: InvIfaceGate::default(),
            widgets_gate: InvIfaceGate::default(),
            side_tabs_gate: InvIfaceGate::default(),
            chat_options_gate: 0,
            make_products_gate: 0,
            main_make_gate: InvIfaceGate::default(),
            puzzle_board_open: false,
            quest_statuses_gate: 0,
            modals_gate: 0,
            controls_gate: 0,
            menu_gate: 0,
            inv_session_current: true,
            session_start: None,
        }
    }
}

impl GameSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every session-derived view while retaining the client-owned
    /// generation watermark. The client intentionally keeps some decoded
    /// actor/interface tables across `logout()`; the host must not publish
    /// those as current, and must not reinterpret the logout invalidation as
    /// fresh data. A later packet moves its family beyond this watermark and
    /// republishes only that family.
    pub fn reset_session(&mut self, gens: ClientGens) {
        let inv_iface_gate = InvIfaceGate {
            iface: gens.iface,
            inv: gens.inv,
        };
        *self = Self {
            gens,
            loc_gen: gens.scene,
            ground_item_gen: gens.scene,
            inventory_gate: inv_iface_gate,
            equipment_gate: inv_iface_gate,
            bank_gate: inv_iface_gate,
            bank_side_gate: inv_iface_gate,
            trade_gate: inv_iface_gate,
            shop_gate: inv_iface_gate,
            widgets_gate: inv_iface_gate,
            side_tabs_gate: inv_iface_gate,
            chat_options_gate: gens.iface,
            make_products_gate: gens.iface,
            main_make_gate: inv_iface_gate,
            quest_statuses_gate: gens.iface,
            modals_gate: gens.iface,
            controls_gate: gens.iface,
            menu_gate: gens.iface,
            inv_session_current: false,
            session_start: Some(gens),
            ..Self::default()
        };
    }

    /// The generation counters this snapshot reflects.
    pub fn gens(&self) -> ClientGens {
        self.gens
    }

    /// Scene-family generation this snapshot last rebuilt the collision grid at.
    pub fn scene_generation(&self) -> u64 {
        self.gens.scene
    }

    /// Loc/static-scenery generation that last recopied collision flags.
    pub fn loc_static_generation(&self) -> u64 {
        self.loc_static_gen
    }

    /// Loc model stamp mixed into collision identity, stored separately from
    /// [`Self::loc_static_generation`] so the two cannot XOR-alias.
    pub fn loc_model_stamp(&self) -> u64 {
        self.loc_model_stamp
    }

    /// Rebuild `family` from `client` iff its gen moved since the last
    /// rebuild of that family. Returns true iff the gen moved. The npc/
    /// player/stat families rebuild their view caches; the rest track
    /// their counter so a later view can detect movement. `client` is
    /// borrowed immutably (the ground-item lists iterate through the
    /// `LinkList`'s shared iterator).
    pub fn rebuild_family(&mut self, client: &Client, family: Family) -> bool {
        if !self.family_observed(client, family) {
            return false;
        }
        match family {
            Family::Npc => self.rebuild_npcs(client),
            Family::Player => self.rebuild_player(client, true),
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
            Family::Shop => self.rebuild_shop(client),
            Family::Widgets => self.rebuild_widgets(client),
            Family::SideTabs => self.rebuild_side_tabs(client),
            Family::ChatOptions => self.rebuild_chat_options(client),
            Family::MakeProducts => self.rebuild_make_products(client),
            Family::MainMake => self.rebuild_main_make(client),
            Family::QuestStatuses => self.rebuild_quest_statuses(client),
            Family::Modals => self.rebuild_modals(client),
            Family::Controls => self.rebuild_controls(client),
            Family::Menu => self.rebuild_menu(client),
        }
    }

    /// A scene invalidation refreshes current views but cannot make retained
    /// packet data from an earlier connection current. Subtract the client's
    /// explicit invalidation count, rather than guessing from equal deltas.
    fn family_observed(&self, client: &Client, family: Family) -> bool {
        let Some(start) = self.session_start else {
            return true;
        };
        let current = client.gens;
        let (now, before) = match family {
            Family::Player => return current.player_info != start.player_info,
            Family::Npc => (current.npc, start.npc),
            Family::Inv
            | Family::Inventory
            | Family::Equipment
            | Family::Bank
            | Family::BankSide
            | Family::Trade
            | Family::Shop
            | Family::MainMake => (current.inv, start.inv),
            Family::Varp => (current.varp, start.varp),
            Family::Stat => (current.stat, start.stat),
            Family::Chat => (current.chat, start.chat),
            Family::Iface
            | Family::Widgets
            | Family::SideTabs
            | Family::ChatOptions
            | Family::MakeProducts
            | Family::QuestStatuses
            | Family::Modals
            | Family::Controls
            | Family::Menu => (current.iface, start.iface),
            Family::Camera => (current.camera, start.camera),
            Family::MapFlag => (current.map_flag, start.map_flag),
            Family::World => (current.world, start.world),
            Family::Scene | Family::Loc | Family::GroundItem => return true,
        };
        now.wrapping_sub(before) != current.invalidations.wrapping_sub(start.invalidations)
    }

    /// Rebuild every family whose gen moved (the harness "one snapshot
    /// per tick" read). Returns true iff any family gen moved.
    pub fn rebuild(&mut self, client: &Client) -> bool {
        self.rebuild_from_drain(client, true)
    }

    /// Refresh the complete host view, retaining each family's generation
    /// gate and the cheap scalar reads that can change without a packet.
    /// Only a real PLAYER_INFO observation advances the host tick.
    pub fn rebuild_from_drain(&mut self, client: &Client, player_info: bool) -> bool {
        self.refresh_native_facts(client);
        self.loc_bits_prelude = Some(loc_dirty_bits(client));
        let mut dirty = false;
        dirty |= self.rebuild_family(client, Family::Npc);
        dirty |= self.rebuild_player(client, player_info);
        dirty |= self.rebuild_family(client, Family::Inv);
        dirty |= self.rebuild_family(client, Family::Varp);
        dirty |= self.rebuild_family(client, Family::Stat);
        dirty |= self.rebuild_family(client, Family::Chat);
        dirty |= self.rebuild_family(client, Family::Scene);
        dirty |= self.rebuild_family(client, Family::Loc);
        self.loc_bits_prelude = None;
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
        dirty |= self.rebuild_family(client, Family::Shop);
        dirty |= self.rebuild_family(client, Family::Widgets);
        dirty |= self.rebuild_family(client, Family::SideTabs);
        dirty |= self.rebuild_family(client, Family::ChatOptions);
        dirty |= self.rebuild_family(client, Family::MakeProducts);
        dirty |= self.rebuild_family(client, Family::MainMake);
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

    /// Game tick when the latest thieving stun spot-animation packet was
    /// observed. This is an onset hint, not an authoritative lock duration.
    pub fn thieving_stun_tick(&self) -> Option<u32> {
        (self.ingame && self.player.is_some())
            .then_some(self.thieving_stun_tick)
            .flatten()
    }

    /// The local player view from the last player rebuild; `None` before
    /// the first `PLAYER_INFO` decodes one.
    pub fn local_player(&self) -> Option<&LocalPlayerView> {
        self.player.as_ref()
    }

    /// The local actor's current native overhead text. This deliberately
    /// does not fall back to the game-chat ring or another player's bubble.
    pub fn local_overhead_text(&self) -> Option<&str> {
        self.ingame
            .then_some(self.local_overhead_text.as_deref())
            .flatten()
    }

    /// Whether the local player currently has an active positive type-1
    /// combat hit. Fail-closed outside an active session or without a local
    /// player; expires with the hitmark cycle even when player gen is quiet.
    pub fn taking_damage(&self) -> bool {
        self.taking_damage
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

    /// The nearest scene loc whose actions include `Use-quickly` on the
    /// player's plane. None when off-scene or no booth on the plane.
    pub fn nearest_use_quickly_booth(&self) -> Option<&LocView> {
        let (px, pz, level) = self.tile()?;
        self.locs()
            .iter()
            .filter(|l| {
                l.tile.level == level
                    && l.actions.iter().any(|a| {
                        a.as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case("Use-quickly"))
                    })
            })
            .min_by_key(|l| (l.tile.x - px).abs().max((l.tile.z - pz).abs()))
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

    /// The current normalized coordinate hint, absent for other hint kinds
    /// and outside an active game session.
    pub fn hint_tile(&self) -> Option<&HintTileView> {
        self.ingame.then_some(self.hint_tile.as_ref()).flatten()
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

    /// The open puzzle board: the identified TYPE_INV component (the first
    /// depth-first component with `obj_ops` under the main modal), its
    /// `link_obj_type` slot count, its session generation and the rows the
    /// identified widget already holds. Always present — `component_id` is
    /// `-1` for a closed board, which is a postable observation, not an
    /// omitted fact. The rows borrow that widget's own view: no copy.
    pub fn puzzle_board(&self) -> PuzzleBoardView<'_> {
        PuzzleBoardView {
            component_id: self.puzzle_board_component_id,
            size: self.puzzle_board_size,
            generation: self.puzzle_session_generation,
            items: self
                .widgets
                .iter()
                .find(|w| w.component_id == self.puzzle_board_component_id)
                .map_or(&[], |w| w.items.as_slice()),
        }
    }

    /// Identity of the current bank open/close session.
    pub fn bank_session_generation(&self) -> u64 {
        self.bank_session_generation
    }

    /// Whether the current bank component has a transmitting full snapshot
    /// that is current for this open session.
    pub fn bank_loaded(&self) -> bool {
        self.bank_loaded
    }

    /// Bank-side (deposit) item views from the last bank-side rebuild.
    pub fn bank_side(&self) -> &[ItemView] {
        &self.bank_side
    }

    /// The trade state (offer/confirm open, the four containers, partner).
    pub fn trade(&self) -> &TradeView {
        &self.trade
    }

    /// The shop modal's stock (empty while the shop root is not the main
    /// modal).
    pub fn shop(&self) -> &ShopView {
        &self.shop
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

    /// Main-modal skill-multi rows from the last main-make rebuild. Empty
    /// while no main modal is open or the open modal has no Make TYPE_INV.
    pub fn main_make(&self) -> &[ItemView] {
        &self.main_make
    }

    /// Quest-journal entries from the last quest-statuses rebuild.
    pub fn quest_statuses(&self) -> &[QuestStatusView] {
        &self.quest_statuses
    }

    /// Whether the native quest-tab root was loaded for the last rebuild.
    pub fn quest_statuses_available(&self) -> bool {
        self.quest_statuses_available
    }

    /// The run-toggle pair from the last controls rebuild.
    pub fn run_controls(&self) -> Option<&ToggleControlsView> {
        self.run_controls.as_ref()
    }

    /// The auto-retaliate toggle pair from the last controls rebuild.
    pub fn retaliate_controls(&self) -> Option<&ToggleControlsView> {
        self.retaliate_controls.as_ref()
    }

    /// The open bank's Note (on) / Item (off) toggle pair; None when the
    /// bank is shut or the buttons are not on the main modal tree.
    pub fn bank_note_controls(&self) -> Option<&ToggleControlsView> {
        self.bank_note_controls.as_ref()
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

}



use super::*;
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

    pub fn local_overhead_text(&self) -> Option<&str> {
        self.0.local_overhead_text()
    }

    /// Active positive type-1 combat hit on the local player.
    pub fn taking_damage(&self) -> bool {
        self.0.taking_damage()
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

    /// Identity of the current bank open/close session.
    pub fn bank_session_generation(&self) -> u64 {
        self.0.bank_session_generation()
    }

    /// The open puzzle board (identity, slot count, session generation and
    /// the identified widget's rows). `component_id` is -1 when closed.
    pub fn puzzle_board(&self) -> PuzzleBoardView<'_> {
        self.0.puzzle_board()
    }

    /// Whether the current bank component has fresh, transmitting full data.
    pub fn bank_loaded(&self) -> bool {
        self.0.bank_loaded()
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

    /// Main-modal skill-multi rows (anvil Make-N).
    pub fn main_make(&self) -> &[ItemView] {
        self.0.main_make()
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

    pub fn hint_tile(&self) -> Option<&HintTileView> {
        self.0.hint_tile()
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

    /// The open shop modal's stock (empty while the shop is down).
    pub fn shop(&self) -> &ShopView {
        self.0.shop()
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

    pub fn bank_note_controls(&self) -> Option<&ToggleControlsView> {
        self.0.bank_note_controls()
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

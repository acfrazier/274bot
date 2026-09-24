//! One decoded scene per isolate thread.
//!
//! The isolate decodes each posted `Snapshot` FlatBuffer once and applies it
//! here before any module hook runs. Step machines read these owned rows at
//! call time instead of keeping their own copy of the NPC, inventory and loc
//! tables.
//!
//! The merge rule is the snapshot's delta rule: `tick` is always carried; a
//! page the post did not carry keeps its last value, and a page never posted
//! stays absent. Nothing here invents a zero, `-1` or empty table for state
//! the host did not post. Each module applies its own default at read time.
//!
//! Every page remembers the post that carried it. That lets a module keep
//! its logout rule without a private copy: most machines treat a post that
//! carries `ingame == false` as "forget the session", which is
//! [`Scene::since_login`]. [`Scene::latest`] is the plain delta merge, the
//! same page the JS materializer holds.

use crate::isolate_fb::{RowReader, SceneEntityReader, SnapshotReader};
use api::line_of_sight::CollisionQuery;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static SCENE: RefCell<Scene> = RefCell::new(Scene::default());
}

/// Apply one decoded post. Called once per `IsolateCmd::Snapshot`, before
/// the module hooks.
pub fn apply(snap: &SnapshotReader<'_>) {
    SCENE.with(|scene| scene.borrow_mut().apply(snap));
}

/// Read the scene. Must not re-enter [`apply`], [`on_reset`] or [`post`].
pub fn with<R>(f: impl FnOnce(&Scene) -> R) -> R {
    SCENE.with(|scene| f(&scene.borrow()))
}

/// `ResetSession`: the next post is a keyframe for a new session.
pub fn on_reset() {
    SCENE.with(|scene| *scene.borrow_mut() = Scene::default());
}

/// Write one synthetic post. For module test seams that stand in for a
/// decoded snapshot; production writes only through [`apply`].
pub(crate) fn post(tick: u64, f: impl FnOnce(&mut Post<'_>)) {
    SCENE.with(|scene| {
        let mut scene = scene.borrow_mut();
        let mut post = scene.begin_post(tick);
        f(&mut post);
    });
}

/// Replace the whole scene: a fresh session, one post carrying `ingame`,
/// then one post carrying the rest. For the public `set_observation` test
/// seams, whose observation types read back through their own lenses.
pub(crate) fn replace(tick: u64, ingame: bool, f: impl FnOnce(&mut Post<'_>)) {
    on_reset();
    post(tick, |post| {
        post.session(ingame);
    });
    post(tick, f);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// One inv/bank/equipment/shop/trade/panel row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemRow {
    pub id: i32,
    pub count: i32,
    pub name: Option<String>,
    pub ops: Vec<String>,
    pub noted: bool,
    pub cert: i32,
    /// `None` when the row omitted the slot (not `-1`).
    pub component_id: Option<i32>,
    /// `None` when the row omitted the slot (not `-1`).
    pub slot: Option<i32>,
}

impl ItemRow {
    fn read(row: &RowReader<'_>) -> Self {
        Self {
            id: row.id(),
            count: row.count(),
            name: row.name().map(str::to_string),
            ops: row.ops().into_iter().map(str::to_string).collect(),
            noted: row.noted(),
            cert: row.cert(),
            component_id: row.has_component_id().then(|| row.component_id()),
            slot: row.has_slot().then(|| row.slot()),
        }
    }

    /// The name, or `""` when the row posted none.
    pub fn name_or_empty(&self) -> &str {
        self.name.as_deref().unwrap_or_default()
    }

    /// The posted slot, or the reader's `-1` default.
    pub fn slot_or_unset(&self) -> i32 {
        self.slot.unwrap_or(-1)
    }

    /// The posted component id, or the reader's `-1` default.
    pub fn component_or_unset(&self) -> i32 {
        self.component_id.unwrap_or(-1)
    }
}

/// One npc/loc/player/ground row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRow {
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
}

impl Default for EntityRow {
    /// The reader's per-field defaults for an omitted slot.
    fn default() -> Self {
        Self {
            index: 0,
            id: 0,
            name: None,
            x: 0,
            z: 0,
            level: 0,
            distance: 0,
            health: -1,
            max_health: -1,
            in_combat: false,
            animating: false,
            actions: Vec::new(),
            reachable: false,
            reachable_adj: false,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 0,
            nx: 0,
            nz: 0,
        }
    }
}

impl EntityRow {
    fn read(row: &SceneEntityReader<'_>) -> Self {
        Self {
            index: row.index(),
            id: row.id(),
            name: row.name().map(str::to_string),
            x: row.x(),
            z: row.z(),
            level: row.level(),
            distance: row.distance(),
            health: row.health(),
            max_health: row.max_health(),
            in_combat: row.in_combat(),
            animating: row.animating(),
            actions: row.actions().into_iter().map(str::to_string).collect(),
            reachable: row.reachable(),
            reachable_adj: row.reachable_adj(),
            combat_level: row.combat_level(),
            target_kind: row.target_kind(),
            target_index: row.target_index(),
            size: row.size(),
            nx: row.nx(),
            nz: row.nz(),
        }
    }

    pub fn name_or_empty(&self) -> &str {
        self.name.as_deref().unwrap_or_default()
    }

    pub fn tile(&self) -> Tile {
        Tile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatRow {
    pub index: i32,
    pub name: String,
    pub xp: i32,
    pub base: i32,
    pub effective: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VarpRow {
    pub index: i32,
    pub value: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideTabIface {
    pub index: i32,
    pub id: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatLine {
    pub seq: i32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MakeButton {
    pub qty: i32,
    pub com_id: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MakeProduct {
    pub name: String,
    pub buttons: Vec<MakeButton>,
}

/// One packed bank stand. `kind` is `"booth"` or `"npc"`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BankStand {
    pub name: String,
    pub tile: Tile,
    pub kind: String,
    pub op: i32,
    pub choose: Option<String>,
}

/// The Rust-picked nearest Use-quickly booth.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NearestBooth {
    pub tile: Tile,
    pub id: i32,
    pub name: String,
    pub op: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BankApproach {
    pub loc_id: i32,
    pub tile: Tile,
    pub can_operate: bool,
    /// `None` when the host posted no usable destination.
    pub dest: Option<Tile>,
}

/// The main modal's paired text walk. `root == -1` is the closed pair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModalTexts {
    pub root: i32,
    pub texts: Vec<String>,
}

/// The reach bit views the fire planner reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub walkable: Vec<u32>,
    pub step: Vec<u8>,
    pub canlight: Vec<u32>,
}

/// The host's last published walk outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkOutcome {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub failed: bool,
    pub tile: Tile,
    pub radius: i32,
    pub allow_teleports: bool,
}

/// One posted page and the post that carried it.
#[derive(Clone, Debug)]
struct Page<T> {
    seq: u64,
    value: T,
}

type Slot<T> = Option<Page<T>>;

macro_rules! scene_pages {
    (
        copy { $( $cname:ident : $cty:ty ),* $(,)? }
        rows { $( $(#[$doc:meta])* $rname:ident : $rty:ty ),* $(,)? }
    ) => {
        /// The decoded scene. Fields are pages; read them through a [`Lens`].
        #[derive(Default)]
        pub struct Scene {
            /// Posts applied since the last reset (the current post's id).
            seq: u64,
            /// The post that last carried `ingame == false` (0 = none).
            logout_seq: u64,
            tick: Option<u64>,
            $( $cname: Slot<$cty>, )*
            $( $rname: Slot<$rty>, )*
        }

        impl<'a> Lens<'a> {
            $(
                pub fn $cname(&self) -> Option<$cty> {
                    self.get(&self.scene.$cname).copied()
                }
            )*
            $(
                $(#[$doc])*
                pub fn $rname(&self) -> Option<&'a $rty> {
                    self.get(&self.scene.$rname)
                }
            )*
        }

        impl Post<'_> {
            $(
                pub(crate) fn $cname(&mut self, value: $cty) -> &mut Self {
                    self.scene.$cname = Some(Page { seq: self.seq, value });
                    self
                }
            )*
            $(
                pub(crate) fn $rname(&mut self, value: $rty) -> &mut Self {
                    self.scene.$rname = Some(Page { seq: self.seq, value });
                    self
                }
            )*
        }
    };
}

scene_pages! {
    copy {
        ingame: bool,
        here: Tile,
        hold: bool,
        ours: bool,
        scene_state: i32,
        animating: bool,
        in_combat: bool,
        self_slot: i32,
        self_target_kind: i32,
        self_target_index: i32,
        side_tab: i32,
        main_modal_id: i32,
        chat_modal_id: i32,
        chat_open: bool,
        chat_continue: bool,
        count_dialog_open: bool,
        inv_size: i32,
        bank_open: bool,
        bank_loaded: bool,
        bank_generation: u64,
        bank_op_result_seq: u64,
        bank_op_result: bool,
        withdraw_x_result_seq: u64,
        withdraw_x_result: bool,
        trade_offer_open: bool,
        trade_confirm_open: bool,
        trade_accept_id: i32,
        trade_decline_id: i32,
        shop_open: bool,
        walk_outcome: WalkOutcome,
    }
    rows {
        trade_partner: String,
        inv: Vec<ItemRow>,
        equipment: Vec<ItemRow>,
        bank: Vec<ItemRow>,
        bank_side: Vec<ItemRow>,
        trade_mine: Vec<ItemRow>,
        trade_side: Vec<ItemRow>,
        shop_stock: Vec<ItemRow>,
        /// `None` inside the page: the host posted the shop player pack as
        /// not decoded this rebuild.
        shop_player: Option<Vec<ItemRow>>,
        /// `None` inside the page: the skill-multi panel was not decoded.
        main_make: Option<Vec<ItemRow>>,
        npcs: Vec<EntityRow>,
        locs: Vec<EntityRow>,
        ground: Vec<EntityRow>,
        players: Vec<EntityRow>,
        stats: Vec<StatRow>,
        varps: Vec<VarpRow>,
        side_tab_ifaces: Vec<SideTabIface>,
        chat_options: Vec<String>,
        chat_lines: Vec<ChatLine>,
        make_products: Vec<MakeProduct>,
        banks: Vec<BankStand>,
        nearest_booth: NearestBooth,
        bank_approaches: Vec<BankApproach>,
        main_modal_texts: ModalTexts,
        collision: CollisionQuery,
        reach: Reach,
    }
}

/// A read view over the scene: the pages posted after `after`.
#[derive(Clone, Copy)]
pub struct Lens<'a> {
    scene: &'a Scene,
    after: u64,
}

impl<'a> Lens<'a> {
    fn get<T>(&self, slot: &'a Slot<T>) -> Option<&'a T> {
        slot.as_ref()
            .filter(|page| page.seq > self.after)
            .map(|page| &page.value)
    }

    /// The first posted stat row whose name matches, case-insensitively.
    pub fn stat(&self, name: &str) -> Option<&'a StatRow> {
        self.stats()?
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(name))
    }
}

/// One post being written: every page set through it carries this post.
pub(crate) struct Post<'a> {
    scene: &'a mut Scene,
    seq: u64,
}

impl Post<'_> {
    /// `ingame`, with the logout mark a `false` post sets.
    pub(crate) fn session(&mut self, ingame: bool) -> &mut Self {
        if !ingame {
            self.scene.logout_seq = self.seq;
        }
        self.ingame(ingame)
    }
}

impl Scene {
    /// Every page as last posted: the same merge the JS page holds.
    pub fn latest(&self) -> Lens<'_> {
        Lens {
            scene: self,
            after: 0,
        }
    }

    /// Pages posted after the last `ingame == false` post. The logout post
    /// itself is excluded: a machine that forgets its session on logout
    /// ignores the rest of that post too.
    pub fn since_login(&self) -> Lens<'_> {
        Lens {
            scene: self,
            after: self.logout_seq,
        }
    }

    /// Pages posted in or after the last `ingame == false` post.
    pub fn since_logout(&self) -> Lens<'_> {
        Lens {
            scene: self,
            after: self.logout_seq.saturating_sub(1),
        }
    }

    /// The last posted tick. `None` before the first post of a session.
    pub fn tick(&self) -> Option<u64> {
        self.tick
    }

    /// The last posted tick, unless the last post was the logout post.
    pub fn session_tick(&self) -> Option<u64> {
        if self.seq != 0 && self.seq == self.logout_seq {
            None
        } else {
            self.tick
        }
    }

    /// Whether this session has seen a post yet.
    pub fn applied(&self) -> bool {
        self.seq != 0
    }

    fn begin_post(&mut self, tick: u64) -> Post<'_> {
        self.seq += 1;
        self.tick = Some(tick);
        let seq = self.seq;
        Post { scene: self, seq }
    }

    fn apply(&mut self, snap: &SnapshotReader<'_>) {
        let mut post = self.begin_post(snap.tick());
        let p = &mut post;
        if snap.has_ingame() {
            p.session(snap.ingame());
        }
        if let Some(t) = snap.here() {
            p.here(Tile {
                x: t.x(),
                z: t.z(),
                level: t.level(),
            });
        }
        if snap.has_hold() {
            p.hold(snap.hold());
        }
        if snap.has_ours() {
            p.ours(snap.ours());
        }
        if snap.has_scene_state() {
            p.scene_state(snap.scene_state());
        }
        if snap.has_animating() {
            p.animating(snap.animating());
        }
        if snap.has_in_combat() {
            p.in_combat(snap.in_combat());
        }
        if snap.has_self_slot() {
            p.self_slot(snap.self_slot());
        }
        if snap.has_self_target_kind() {
            p.self_target_kind(snap.self_target_kind());
        }
        if snap.has_self_target_index() {
            p.self_target_index(snap.self_target_index());
        }
        if snap.has_side_tab() {
            p.side_tab(snap.side_tab());
        }
        if snap.has_main_modal_id() {
            p.main_modal_id(snap.main_modal_id());
        }
        if snap.has_chat_modal_id() {
            p.chat_modal_id(snap.chat_modal_id());
        }
        if snap.has_chat_open() {
            p.chat_open(snap.chat_open());
        }
        if snap.has_chat_continue() {
            p.chat_continue(snap.chat_continue());
        }
        if snap.has_count_dialog_open() {
            p.count_dialog_open(snap.count_dialog_open());
        }
        if snap.has_inv_size() {
            p.inv_size(snap.inv_size());
        }
        if snap.has_bank_open() {
            p.bank_open(snap.bank_open());
        }
        if snap.has_bank_loaded() {
            p.bank_loaded(snap.bank_loaded());
        }
        if snap.has_bank_generation() {
            p.bank_generation(snap.bank_generation());
        }
        if snap.has_bank_op_result_seq() {
            p.bank_op_result_seq(snap.bank_op_result_seq());
        }
        if snap.has_bank_op_result() {
            p.bank_op_result(snap.bank_op_result());
        }
        if snap.has_withdraw_x_result_seq() {
            p.withdraw_x_result_seq(snap.withdraw_x_result_seq());
        }
        if snap.has_withdraw_x_result() {
            p.withdraw_x_result(snap.withdraw_x_result());
        }
        if snap.has_trade_offer_open() {
            p.trade_offer_open(snap.trade_offer_open());
        }
        if snap.has_trade_confirm_open() {
            p.trade_confirm_open(snap.trade_confirm_open());
        }
        if let Some(partner) = snap.trade_partner() {
            p.trade_partner(partner.to_string());
        }
        if snap.has_trade_accept_id() {
            p.trade_accept_id(snap.trade_accept_id());
        }
        if snap.has_trade_decline_id() {
            p.trade_decline_id(snap.trade_decline_id());
        }
        if snap.has_shop_open() {
            p.shop_open(snap.shop_open());
        }
        if snap.has_inv() {
            p.inv(items(snap.inv()));
        }
        if snap.has_equipment() {
            p.equipment(items(snap.equipment()));
        }
        if snap.has_bank() {
            p.bank(items(snap.bank()));
        }
        if snap.has_bank_side() {
            p.bank_side(items(snap.bank_side()));
        }
        if snap.has_trade_mine() {
            p.trade_mine(items(snap.trade_mine()));
        }
        if snap.has_trade_side() {
            p.trade_side(items(snap.trade_side()));
        }
        if snap.has_shop_stock() {
            p.shop_stock(items(snap.shop_stock()));
        }
        if snap.has_shop_player_available() {
            p.shop_player(
                snap.shop_player_available()
                    .then(|| items(snap.shop_player())),
            );
        }
        if snap.has_main_make_available() {
            p.main_make(snap.main_make_available().then(|| items(snap.main_make())));
        }
        if snap.has_npcs() {
            p.npcs(entities(snap.npcs()));
        }
        if snap.has_locs() {
            p.locs(entities(snap.locs()));
        }
        if snap.has_ground() {
            p.ground(entities(snap.ground()));
        }
        if snap.has_players() {
            p.players(entities(snap.players()));
        }
        if snap.has_stats() {
            p.stats(
                snap.stats()
                    .iter()
                    .map(|row| StatRow {
                        index: row.index(),
                        name: row.name().to_string(),
                        xp: row.xp(),
                        base: row.base(),
                        effective: row.effective(),
                    })
                    .collect(),
            );
        }
        if snap.has_varps() {
            p.varps(
                snap.varps()
                    .iter()
                    .map(|row| VarpRow {
                        index: row.index(),
                        value: row.value(),
                    })
                    .collect(),
            );
        }
        if snap.has_side_tab_ifaces() {
            p.side_tab_ifaces(
                snap.side_tab_ifaces()
                    .iter()
                    .map(|row| SideTabIface {
                        index: row.index(),
                        id: row.id(),
                    })
                    .collect(),
            );
        }
        if snap.has_chat_options() {
            // Empty texts stay: the 1-based answer index is the posted slot.
            p.chat_options(
                snap.chat_options()
                    .iter()
                    .map(|row| row.text().to_string())
                    .collect(),
            );
        }
        if snap.has_chat_lines() {
            p.chat_lines(
                snap.chat_lines()
                    .iter()
                    .map(|line| ChatLine {
                        seq: line.seq(),
                        text: line.text().to_string(),
                    })
                    .collect(),
            );
        }
        if snap.has_make_products() {
            p.make_products(
                snap.make_products()
                    .iter()
                    .map(|product| MakeProduct {
                        name: product.name().to_string(),
                        buttons: product
                            .buttons()
                            .iter()
                            .map(|button| MakeButton {
                                qty: button.qty(),
                                com_id: button.com_id(),
                            })
                            .collect(),
                    })
                    .collect(),
            );
        }
        if snap.has_banks() {
            p.banks(
                snap.banks()
                    .iter()
                    .map(|stand| BankStand {
                        name: stand.name().to_string(),
                        tile: Tile {
                            x: stand.x(),
                            z: stand.z(),
                            level: stand.level(),
                        },
                        kind: stand.kind().to_string(),
                        op: stand.op(),
                        choose: stand.choose().map(str::to_string),
                    })
                    .collect(),
            );
        }
        if let Some(booth) = snap.nearest_booth() {
            p.nearest_booth(NearestBooth {
                tile: Tile {
                    x: booth.x(),
                    z: booth.z(),
                    level: booth.level(),
                },
                id: booth.id(),
                name: booth.name().to_string(),
                op: booth.op().to_string(),
            });
        }
        if snap.has_bank_approaches() {
            p.bank_approaches(
                snap.bank_approaches()
                    .iter()
                    .map(|row| BankApproach {
                        loc_id: row.loc_id(),
                        tile: Tile {
                            x: row.x(),
                            z: row.z(),
                            level: row.level(),
                        },
                        can_operate: row.can_operate(),
                        dest: row.dest_ok().then(|| Tile {
                            x: row.dest_x(),
                            z: row.dest_z(),
                            level: row.dest_level(),
                        }),
                    })
                    .collect(),
            );
        }
        if let Some(pair) = snap.main_modal_texts() {
            p.main_modal_texts(ModalTexts {
                root: pair.root(),
                texts: pair.texts().into_iter().map(str::to_string).collect(),
            });
        }
        if let Some(c) = snap.collision() {
            p.collision(CollisionQuery {
                available: c.available(),
                base_x: c.base_x(),
                base_z: c.base_z(),
                level: c.level(),
                width: c.width(),
                height: c.height(),
                flags: Arc::from(c.flags()),
            });
        }
        if let Some(r) = snap.reach() {
            p.reach(Reach {
                available: r.available(),
                base_x: r.base_x(),
                base_z: r.base_z(),
                level: r.level(),
                width: r.width(),
                height: r.height(),
                walkable: r.walkable(),
                step: r.step(),
                canlight: r.canlight(),
            });
        }
        if snap.has_walk_outcome_seq() {
            p.walk_outcome(WalkOutcome {
                seq: snap.walk_outcome_seq(),
                generation: snap.walk_outcome_generation(),
                request_id: snap.walk_outcome_request_id(),
                failed: snap.walk_outcome_failed(),
                tile: Tile {
                    x: snap.walk_outcome_x(),
                    z: snap.walk_outcome_z(),
                    level: snap.walk_outcome_level(),
                },
                radius: snap.walk_outcome_radius(),
                allow_teleports: snap.walk_outcome_allow_teleports(),
            });
        }
    }
}

fn items(rows: Vec<RowReader<'_>>) -> Vec<ItemRow> {
    rows.iter().map(ItemRow::read).collect()
}

fn entities(rows: Vec<SceneEntityReader<'_>>) -> Vec<EntityRow> {
    rows.iter().map(EntityRow::read).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_delta, ItemRowInput, SceneEntityInput, SnapshotInput,
        TileInput,
    };

    fn empty(tick: u64) -> SnapshotInput<'static> {
        crate::isolate_fb::tests::empty_input(tick)
    }

    fn apply_bytes(bytes: &[u8]) {
        apply(&SnapshotReader::from_bytes(bytes).expect("snapshot"));
    }

    fn goblin(index: i32) -> SceneEntityInput<'static> {
        SceneEntityInput {
            index,
            id: 100,
            name: Some("Goblin"),
            x: 10,
            z: 20,
            level: 0,
            distance: 3,
            health: -1,
            max_health: -1,
            in_combat: false,
            animating: false,
            actions: &[],
            reachable: false,
            reachable_adj: false,
            combat_level: 2,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: 10,
            nz: 20,
        }
    }

    #[test]
    fn a_page_never_posted_stays_absent() {
        on_reset();
        with(|scene| {
            assert!(!scene.applied());
            assert_eq!(scene.tick(), None);
            assert!(scene.latest().here().is_none());
            assert!(scene.latest().npcs().is_none());
        });
    }

    #[test]
    fn a_delta_that_omits_a_page_keeps_its_last_rows() {
        on_reset();
        let npcs = [goblin(7)];
        let inv = [ItemRowInput::nc(Some("Shark"), 3)];
        let mut snap = empty(1);
        snap.here = Some(TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        });
        snap.npcs = &npcs;
        snap.inv = &inv;
        let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
        apply_bytes(&keyframe);
        snap.tick = 2;
        let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        let reader = SnapshotReader::from_bytes(&delta).unwrap();
        assert!(
            !reader.has_npcs(),
            "the delta must omit the unchanged table"
        );
        apply(&reader);
        with(|scene| {
            assert_eq!(scene.tick(), Some(2));
            let rows = scene.latest().npcs().expect("kept npcs");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].index, 7);
            assert_eq!(rows[0].name.as_deref(), Some("Goblin"));
            let inv = scene.latest().inv().expect("kept inv");
            assert_eq!(inv[0].name.as_deref(), Some("Shark"));
            assert_eq!(inv[0].count, 3);
            assert_eq!(
                scene.latest().here(),
                Some(Tile {
                    x: 3200,
                    z: 3200,
                    level: 0
                })
            );
        });
    }

    #[test]
    fn a_later_post_replaces_the_rows_it_carries() {
        on_reset();
        let first = [goblin(1)];
        let mut snap = empty(1);
        snap.npcs = &first;
        apply_bytes(&encode_snapshot(&snap));
        let second = [goblin(2), goblin(3)];
        snap.tick = 2;
        snap.npcs = &second;
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| {
            let rows = scene.latest().npcs().unwrap();
            assert_eq!(
                rows.iter().map(|row| row.index).collect::<Vec<_>>(),
                vec![2, 3]
            );
        });
        // A present empty table is an observation, not an omission.
        snap.tick = 3;
        snap.npcs = &[];
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| assert_eq!(scene.latest().npcs().map(Vec::len), Some(0)));
    }

    #[test]
    fn logout_hides_the_session_from_since_login_only() {
        on_reset();
        let npcs = [goblin(1)];
        let mut snap = empty(1);
        snap.ingame = true;
        snap.npcs = &npcs;
        let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
        apply_bytes(&keyframe);
        snap.tick = 2;
        snap.ingame = false;
        let (logout, fp) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&logout);
        with(|scene| {
            assert_eq!(scene.latest().ingame(), Some(false));
            assert!(scene.latest().npcs().is_some(), "delta merge keeps rows");
            assert!(scene.since_login().npcs().is_none());
            assert!(scene.since_login().ingame().is_none());
            assert_eq!(scene.since_logout().ingame(), Some(false));
            assert_eq!(scene.session_tick(), None);
        });
        snap.tick = 3;
        snap.ingame = true;
        let (login, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&login);
        with(|scene| {
            assert_eq!(scene.since_login().ingame(), Some(true));
            assert!(
                scene.since_login().npcs().is_none(),
                "rows not re-posted since login stay unobserved"
            );
            assert_eq!(scene.session_tick(), Some(3));
        });
    }

    #[test]
    fn reset_session_clears_every_page() {
        let mut snap = empty(4);
        snap.here = Some(TileInput {
            x: 1,
            z: 2,
            level: 0,
        });
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| assert!(scene.latest().here().is_some()));
        on_reset();
        with(|scene| {
            assert!(!scene.applied());
            assert_eq!(scene.tick(), None);
            assert!(scene.latest().here().is_none());
            assert!(scene.latest().ingame().is_none());
        });
    }
}

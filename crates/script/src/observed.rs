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
//! [`Scene::since_login`]. [`Scene::latest`] is the plain delta merge over
//! posted pages. Unlike the JS materializer it fills no default for a page
//! the first post of a session omitted: that page stays absent.
//!
//! The scene keeps only what its readers use. Names and op lists are
//! interned per isolate ([`Text`], [`Ops`]), so a repeated loc, npc or item
//! name is allocated once and shared, not copied per row per post. The
//! interner is bounded ([`INTERN_CAP`]) and resets when full.
//! Loc, ground and player rows are the slim [`SceneRow`]; only npcs carry the
//! combat fields ([`EntityRow`]). Stats keep the four skills machines read
//! ([`Skills`]), and the side-tab and bank-stand tables keep the one fact
//! read from each.

use crate::api_gather::{GatherCounts, GatherEnd, GatherFailure};
use crate::api_progress::{ProgressPage, QuestProgressRow as ApiQuestProgressRow};
use crate::isolate_fb::{
    ApiGather, ApiGatherOutcome, ApiProgress, CombatStyle, QuestProgressRow, QuestStatus, Row,
    SceneEntity, Snapshot, Stat,
};
use api::line_of_sight::CollisionQuery;
use api::quest_progress::{EvidenceStamp, ProgressFlag};
use api::selected::{FactKey, Gap, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use flatbuffers::{ForwardsUOffset, Vector};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

type TableVector<'a, T> = Option<Vector<'a, ForwardsUOffset<T>>>;
type StringVector<'a> = Option<Vector<'a, ForwardsUOffset<&'a str>>>;

thread_local! {
    static SCENE: RefCell<Scene> = RefCell::new(Scene::fresh());
}

/// Side tab 0: the combat tab whose root [`Lens::combat_tab_root`] keeps.
const COMBAT_TAB: i32 = 0;

/// Interned text: repeated strings share one allocation.
pub type Text = Rc<str>;
/// An interned op/action list in posted slot order.
pub type Ops = Rc<[Text]>;

/// Upper bound on interned strings plus lists. Past it the interner starts
/// over; rows already built keep their `Rc`s, so nothing they hold changes.
const INTERN_CAP: usize = 1024;

/// Distinct scenes (isolate start or `ResetSession`), for [`Stamp`].
static EPOCH: AtomicU64 = AtomicU64::new(1);

/// Apply one decoded post. Called once per `IsolateCmd::Snapshot`, before
/// the module hooks.
pub fn apply(snap: &Snapshot<'_>) {
    SCENE.with(|scene| scene.borrow_mut().apply(snap));
}

/// Read the scene. Must not re-enter [`apply`], [`on_reset`] or [`post`].
pub fn with<R>(f: impl FnOnce(&Scene) -> R) -> R {
    SCENE.with(|scene| f(&scene.borrow()))
}

/// `ResetSession`: the next post is a keyframe for a new session.
pub fn on_reset() {
    SCENE.with(|scene| *scene.borrow_mut() = Scene::fresh());
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
    pub name: Option<Text>,
    pub ops: Ops,
    pub noted: bool,
    pub cert: i32,
    /// `None` when the row omitted the slot (not `-1`).
    pub component_id: Option<i32>,
    /// `None` when the row omitted the slot (not `-1`).
    pub slot: Option<i32>,
}

impl ItemRow {
    fn read(row: &Row<'_>, strings: &mut Interner) -> Self {
        Self {
            id: row.id(),
            count: row.count(),
            name: row.name().map(|name| strings.text(name)),
            ops: strings.ops(row.ops()),
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

/// One npc row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRow {
    pub index: i32,
    pub id: i32,
    pub name: Option<Text>,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
    pub health: i32,
    pub max_health: i32,
    pub in_combat: bool,
    pub animating: bool,
    pub actions: Ops,
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
            actions: Ops::default(),
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
    fn read(row: &SceneEntity<'_>, strings: &mut Interner) -> Self {
        Self {
            index: row.index(),
            id: row.id(),
            name: row.name().map(|name| strings.text(name)),
            x: row.x(),
            z: row.z(),
            level: row.level(),
            distance: row.distance(),
            health: row.health(),
            max_health: row.max_health(),
            in_combat: row.in_combat(),
            animating: row.animating(),
            actions: strings.ops(row.actions()),
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

/// One loc, ground stack or other player: identity, place and ops.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneRow {
    pub id: i32,
    pub name: Option<Text>,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub distance: i32,
    pub actions: Ops,
    pub in_combat: bool,
    /// Placed loc wall shape; zero for rows that are not locs.
    pub shape: u8,
    /// Placed loc wall angle; zero for rows that are not locs.
    pub angle: u8,
}

impl SceneRow {
    fn read(row: &SceneEntity<'_>, strings: &mut Interner) -> Self {
        Self {
            id: row.id(),
            name: row.name().map(|name| strings.text(name)),
            x: row.x(),
            z: row.z(),
            level: row.level(),
            distance: row.distance(),
            actions: strings.ops(row.actions()),
            in_combat: row.in_combat(),
            shape: u8::try_from(row.shape()).unwrap_or_default(),
            angle: u8::try_from(row.angle()).unwrap_or_default(),
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

/// One posted UI button row. The combat tab's style buttons and the magic
/// tab's target buttons arrive with the same shape: a varp mode, the button's
/// own label, and its component id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonRow {
    pub mode: i32,
    pub label: Text,
    pub component_id: i32,
}

impl ButtonRow {
    fn read(row: &CombatStyle<'_>, strings: &mut Interner) -> Self {
        Self {
            mode: row.mode(),
            label: strings.text(row.label().unwrap_or_default()),
            component_id: row.component_id(),
        }
    }
}

/// One posted skill row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skill {
    pub xp: i32,
    pub base: i32,
    pub effective: i32,
}

/// The skills machines read from a posted stats page. A skill the page did
/// not carry is `None`. `hitpoints` matches the posted name exactly; the
/// others match it case-insensitively. The first matching row wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skills {
    pub hitpoints: Option<Skill>,
    pub prayer: Option<Skill>,
    pub magic: Option<Skill>,
    pub firemaking: Option<Skill>,
    pub fishing: Option<Skill>,
    /// Every skill the client uses (`Skill::used`, by stat index) has a
    /// posted base level above 0. The rs2b0t `activeStatsReady` rule: a
    /// freshly logged-in client posts 0 until the stat packets arrive.
    pub ready: bool,
}

impl Skills {
    fn read<'a>(rows: TableVector<'a, Stat<'a>>) -> Self {
        let mut skills = Self::default();
        // Bit `i`: stat slot `i` posted a base level above 0.
        let mut loaded = 0u64;
        for row in rows.into_iter().flat_map(|rows| rows.iter()) {
            if row.base() > 0 {
                if let Ok(i) = u32::try_from(row.index()) {
                    loaded |= 1u64.checked_shl(i).unwrap_or(0);
                }
            }
            let name = row.name().unwrap_or_default();
            let slot = if name == "hitpoints" {
                &mut skills.hitpoints
            } else if name.eq_ignore_ascii_case("prayer") {
                &mut skills.prayer
            } else if name.eq_ignore_ascii_case("magic") {
                &mut skills.magic
            } else if name.eq_ignore_ascii_case("firemaking") {
                &mut skills.firemaking
            } else if name.eq_ignore_ascii_case("fishing") {
                &mut skills.fishing
            } else {
                continue;
            };
            slot.get_or_insert(Skill {
                xp: row.xp(),
                base: row.base(),
                effective: row.effective(),
            });
        }
        skills.ready = (0..u64::BITS as usize)
            .filter(|&i| api::snapshot::stat_used(i))
            .all(|i| loaded & (1 << i) != 0);
        skills
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VarpRow {
    pub index: i32,
    pub value: i32,
}
/// One projectile's immutable classification facts, retained from the
/// verified isolate page for borrowed Hunt protection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectileRow {
    pub spotanim: i32,
    pub target_player_index: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatLine {
    pub seq: i32,
    /// Shared, not interned: readers clone the `Rc`, never the text.
    pub text: Text,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MakeButton {
    pub qty: i32,
    pub com_id: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MakeProduct {
    pub object_id: i32,
    pub name: String,
    pub buttons: Vec<MakeButton>,
}

/// The Rust-picked nearest Use-quickly booth.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NearestBooth {
    pub tile: Tile,
    pub id: i32,
    pub name: Option<Text>,
    pub op: Option<Text>,
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
/// One selected widget row. Inventory components carry their observed row
/// count, including zero; other widgets carry `-1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidgetRow {
    pub component_id: i32,
    pub text: Text,
    pub item_count: i32,
}

/// One posted quest-tab row. `component_id` is omitted when the host did not
/// walk a click target; a present `0` is a real id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestStatusRow {
    pub name: Text,
    pub status: Text,
    pub component_id: Option<i32>,
}

impl QuestStatusRow {
    fn read(row: &QuestStatus<'_>, strings: &mut Interner) -> Self {
        Self {
            name: strings.text(row.name().unwrap_or_default()),
            status: strings.text(row.status().unwrap_or("unknown")),
            component_id: row.component_id(),
        }
    }
}

/// Posted quest journal tab. `Unbound` is the host's null tab; `Bound` is a
/// present table, possibly empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestTab {
    Unbound,
    Bound(Vec<QuestStatusRow>),
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

/// Posted puzzle board plus the generation that rides a click.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PuzzlePage {
    pub component_id: i32,
    pub size: i32,
    pub items: Vec<PuzzlePiece>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PuzzlePiece {
    pub slot: i32,
    pub id: i32,
}

/// One navigator-named Carry short from a walk outcome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarryRow {
    pub id: i32,
    pub count: i32,
    pub name: Option<Text>,
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
    /// The settled route end is frozen `'blocked'`.
    pub blocked: bool,
}

/// The latest posted live Gatherer page, retained only for request identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatherObservation {
    pub request_id: u64,
}
/// The retained terminal result of the latest Gatherer session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatherOutcomeObservation {
    pub request_id: u64,
    pub end: GatherEnd,
}

/// Which post carried a page, in which scene. Equal stamps are the same
/// posted table; a new table or a `ResetSession` changes the stamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    epoch: u64,
    post: u64,
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
        copy { $( $(#[$cdoc:meta])* $cname:ident : $cty:ty ),* $(,)? }
        rows { $( $(#[$doc:meta])* $rname:ident : $rty:ty ),* $(,)? }
    ) => {
        /// The decoded scene. Fields are pages; read them through a [`Lens`].
        #[derive(Default)]
        pub struct Scene {
            /// This scene's [`Stamp`] epoch.
            epoch: u64,
            /// Interned names and op lists for the rows below.
            strings: Interner,
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
                $(#[$cdoc])*
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
        /// The local player's primary animation id (`-1` idle).
        self_anim: i32,
        /// Host-observed local motion; 0/omitted is unknown, not stationary.
        local_player_motion: u8,
        in_combat: bool,
        self_slot: i32,
        self_target_kind: i32,
        self_target_index: i32,
        user_move_intent_seq: u64,
        /// Host-published fingerprint of the current chat-modal page.
        chat_page_fingerprint: u64,
        side_tab: i32,
        main_modal_id: i32,
        chat_modal_id: i32,
        /// `modals().side` (`-1` none): with `bank_open`, whether
        /// `bank_side` is a posted pack ([`crate::bank::ops::side_observation`]).
        side_modal_id: i32,
        chat_open: bool,
        chat_continue: bool,
        count_dialog_open: bool,
        inv_size: i32,
        bank_open: bool,
        bank_loaded: bool,
        bank_generation: u64,
        bank_snapshot_generation: i64,
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
        bank_selection: crate::isolate_fb::BankSelectionInput,
        walk_outcome_cancel_reason: crate::isolate_fb::WalkCancelReason,
        /// The posted root of side tab 0, or `-1` when a posted side-tab
        /// table has no row for it.
        combat_tab_root: i32,
        /// Whether a posted bank-stand table has any `booth` stand.
        has_booth_stands: bool,
        stats: Skills,
    }
    rows {
        combat_styles: Vec<ButtonRow>,
        spell_buttons: Vec<ButtonRow>,
        nearest_booth: NearestBooth,
        trade_partner: String,
        /// The local player's name as posted (frozen `localPlayerName()`).
        my_name: String,
        inv: Vec<ItemRow>,
        equipment: Vec<ItemRow>,
        bank: Vec<ItemRow>,
        bank_side: Vec<ItemRow>,
        trade_mine: Vec<ItemRow>,
        trade_theirs: Vec<ItemRow>,
        trade_side: Vec<ItemRow>,
        shop_stock: Vec<ItemRow>,
        /// `None` inside the page: the host posted the shop player pack as
        /// not decoded this rebuild.
        shop_player: Option<Vec<ItemRow>>,
        /// `None` inside the page: the skill-multi panel was not decoded.
        main_make: Option<Vec<ItemRow>>,
        npcs: Vec<EntityRow>,
        locs: Vec<SceneRow>,
        ground: Vec<SceneRow>,
        players: Vec<SceneRow>,
        /// Combat-classification inputs retained from the existing snapshot post.
        projectiles: Vec<ProjectileRow>,
        varps: Vec<VarpRow>,
        chat_text: Text,
        chat_options: Vec<String>,
        chat_option_ids: Vec<i32>,
        chat_lines: Vec<ChatLine>,
        make_products: Vec<MakeProduct>,
        bank_approaches: Vec<BankApproach>,
        /// Null tab vs a present (possibly empty) journal list.
        quest_statuses: QuestTab,
        main_modal_texts: ModalTexts,
        widgets: Vec<WidgetRow>,
        collision: CollisionQuery,
        reach: Reach,
        puzzle_board: PuzzlePage,
        walk_missing_carry: Vec<CarryRow>,
        api_gather: GatherObservation,
        api_gather_outcome: GatherOutcomeObservation,
        api_progress: ProgressPage,
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

    fn stamp<T>(&self, slot: &'a Slot<T>) -> Option<Stamp> {
        slot.as_ref()
            .filter(|page| page.seq > self.after)
            .map(|page| Stamp {
                epoch: self.scene.epoch,
                post: page.seq,
            })
    }

    /// The stamp of the posted reach table.
    pub fn reach_stamp(&self) -> Option<Stamp> {
        self.stamp(&self.scene.reach)
    }

    /// The stamp of the posted collision table.
    pub fn collision_stamp(&self) -> Option<Stamp> {
        self.stamp(&self.scene.collision)
    }
    /// The latest host walk outcome's ordering sequence.
    pub fn walk_outcome_seq(&self) -> Option<u64> {
        self.walk_outcome().map(|outcome| outcome.seq)
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

fn read_items<'a>(rows: TableVector<'a, Row<'a>>, strings: &mut Interner) -> Vec<ItemRow> {
    rows.into_iter()
        .flat_map(|rows| rows.iter())
        .map(|row| ItemRow::read(&row, strings))
        .collect()
}

fn read_places<'a>(
    rows: TableVector<'a, SceneEntity<'a>>,
    strings: &mut Interner,
) -> Vec<SceneRow> {
    rows.into_iter()
        .flat_map(|rows| rows.iter())
        .map(|row| SceneRow::read(&row, strings))
        .collect()
}

fn read_buttons<'a>(
    rows: TableVector<'a, CombatStyle<'a>>,
    strings: &mut Interner,
) -> Vec<ButtonRow> {
    rows.into_iter()
        .flat_map(|rows| rows.iter())
        .map(|row| ButtonRow::read(&row, strings))
        .collect()
}
fn read_gather_page(page: ApiGather<'_>) -> GatherObservation {
    GatherObservation {
        request_id: page.request_id(),
    }
}

fn read_gather_outcome(page: ApiGatherOutcome<'_>) -> Option<GatherOutcomeObservation> {
    use crate::api_gather::GatherEnd;
    let request_id = page.request_id();
    if request_id == 0 {
        return None;
    }
    let counts = GatherCounts {
        yielded: page.yielded(),
        dropped: page.dropped(),
        deposited: page.deposited(),
        trips: page.trips(),
        xp: page.xp(),
    };
    let message = page.message().unwrap_or_default();
    let end = match page.end() {
        1 => GatherEnd::Stopped {
            token: request_id,
            counts,
        },
        2 => GatherEnd::Blocked {
            token: request_id,
            failure: GatherFailure {
                code: page.code().unwrap_or_default().into(),
                message: message.into(),
            },
            counts,
        },
        3 => GatherEnd::Refused {
            token: request_id,
            reason: message.into(),
        },
        4 => GatherEnd::Failed {
            token: request_id,
            reason: message.into(),
            counts,
        },
        _ => return None,
    };
    Some(GatherOutcomeObservation { request_id, end })
}
fn read_api_progress(page: ApiProgress<'_>) -> Option<ProgressPage> {
    let token = page.request_id();
    if token == 0 {
        return None;
    }
    match page.kind() {
        1 => Some(ProgressPage::Reading { token }),
        2 => page
            .row()
            .and_then(read_progress_row)
            .map(|row| ProgressPage::Done {
                token,
                row: Arc::new(row),
            }),
        3 => page.reason().map(|reason| ProgressPage::Refused {
            token,
            reason: Arc::from(reason),
        }),
        _ => None,
    }
}

fn read_progress_row(row: QuestProgressRow<'_>) -> Option<ApiQuestProgressRow> {
    let colour = match row.colour() {
        1 => QuestListStatus::NotStarted,
        2 => QuestListStatus::InProgress,
        3 => QuestListStatus::Complete,
        4 => QuestListStatus::Unknown,
        _ => return None,
    };
    let complete = progress_truth(row.complete())?;
    let stage = progress_knowledge(row.stage()?, row.stage_gap()?);
    let rule = progress_knowledge(row.rule()?, row.rule_gap()?);
    let flags = row.flags()?;
    let flags = flags
        .iter()
        .map(|flag| {
            let name = flag.flag()?;
            let truth = progress_truth(flag.truth())?;
            let count = match flag.count() {
                -1 => None,
                count
                    if (0..=crate::isolate_fb::MAX_PROGRESS_FLAG_COUNT as i32).contains(&count) =>
                {
                    Some(count as u32)
                }
                _ => return None,
            };
            Some(ProgressFlag {
                flag: FactKey::new(name),
                truth,
                count,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ApiQuestProgressRow {
        quest: Arc::from(row.quest()?),
        display: Arc::from(row.display()?),
        colour,
        stage,
        rule,
        complete,
        flags: flags.into(),
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 0,
                run: row.evidence_run(),
                session: row.evidence_session(),
            },
            tick: row.evidence_tick(),
            sequence: row.evidence_sequence(),
        },
        journal_read: row.journal_read(),
        binding: Arc::from(row.binding()?),
        role: row.role().map(Arc::from),
    })
}

fn progress_knowledge(value: &str, gap: &str) -> Knowledge<Arc<str>> {
    if gap.is_empty() {
        Knowledge::Known(Arc::from(value))
    } else {
        let gap = Gap {
            code: Arc::from(gap),
            sources: Arc::from([]),
        };
        if value.is_empty() {
            Knowledge::Unknown(gap)
        } else {
            Knowledge::Partial {
                known: Arc::from(value),
                gaps: Arc::from([gap]),
            }
        }
    }
}

fn progress_truth(code: u8) -> Option<Truth> {
    match code {
        1 => Some(Truth::True),
        2 => Some(Truth::False),
        3 => Some(Truth::Unknown),
        _ => None,
    }
}

impl Scene {
    fn fresh() -> Self {
        Self {
            epoch: EPOCH.fetch_add(1, Ordering::Relaxed),
            ..Self::default()
        }
    }
    /// The session epoch changes whenever this scene is reset.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Every page as last posted (the delta merge; absent stays absent).
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

    /// The post that last logged out (0 = none): a family that captured a
    /// different mark has seen its login session end.
    pub fn login_mark(&self) -> u64 {
        self.logout_seq
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

    fn apply(&mut self, snap: &Snapshot<'_>) {
        let mut strings = std::mem::take(&mut self.strings);
        self.apply_rows(snap, &mut strings);
        self.strings = strings;
    }

    fn apply_rows(&mut self, snap: &Snapshot<'_>, strings: &mut Interner) {
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
        if snap.has_self_anim() {
            p.self_anim(snap.self_anim());
        }
        if snap.has_local_player_motion() {
            p.local_player_motion(snap.local_player_motion());
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
        if snap.has_user_move_intent_seq() {
            p.user_move_intent_seq(snap.user_move_intent_seq());
        }
        if snap.has_chat_page_fingerprint() {
            p.chat_page_fingerprint(snap.chat_page_fingerprint());
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
        if snap.has_side_modal_id() {
            p.side_modal_id(snap.side_modal_id());
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
        if snap.has_bank_snapshot_generation() {
            p.bank_snapshot_generation(snap.bank_snapshot_generation());
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
        if let Some(name) = snap.my_name() {
            p.my_name(name.to_string());
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
            p.inv(read_items(snap.inv(), strings));
        }
        if snap.has_equipment() {
            p.equipment(read_items(snap.equipment(), strings));
        }
        if snap.has_bank() {
            p.bank(read_items(snap.bank(), strings));
        }
        if snap.has_bank_side() {
            p.bank_side(read_items(snap.bank_side(), strings));
        }
        if snap.has_trade_mine() {
            p.trade_mine(read_items(snap.trade_mine(), strings));
        }
        if snap.has_trade_theirs() {
            p.trade_theirs(read_items(snap.trade_theirs(), strings));
        }
        if snap.has_trade_side() {
            p.trade_side(read_items(snap.trade_side(), strings));
        }
        if snap.has_shop_stock() {
            p.shop_stock(read_items(snap.shop_stock(), strings));
        }
        if snap.has_shop_player_available() {
            p.shop_player(
                snap.shop_player_available()
                    .then(|| read_items(snap.shop_player(), strings)),
            );
        }
        if snap.has_main_make_available() {
            p.main_make(
                snap.main_make_available()
                    .then(|| read_items(snap.main_make(), strings)),
            );
        }
        if snap.has_npcs() {
            p.npcs(
                snap.npcs()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| EntityRow::read(&row, strings))
                    .collect(),
            );
        }
        if snap.has_locs() {
            p.locs(read_places(snap.locs(), strings));
        }
        if snap.has_ground() {
            p.ground(read_places(snap.ground(), strings));
        }
        if snap.has_players() {
            p.players(read_places(snap.players(), strings));
        }
        if let Some(projectiles) = snap.projectiles() {
            p.projectiles(
                projectiles
                    .iter()
                    .map(|row| ProjectileRow {
                        spotanim: row.spotanim(),
                        target_player_index: row.target_player_index(),
                    })
                    .collect(),
            );
        }
        if snap.has_stats() {
            p.stats(Skills::read(snap.stats()));
        }
        if snap.has_combat_styles() {
            p.combat_styles(read_buttons(snap.combat_styles(), strings));
        }
        if snap.has_spell_buttons() {
            p.spell_buttons(read_buttons(snap.spell_buttons(), strings));
        }
        if snap.has_varps() {
            p.varps(
                snap.varps()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| VarpRow {
                        index: row.index(),
                        value: row.value(),
                    })
                    .collect(),
            );
        }
        if snap.has_side_tab_ifaces() {
            p.combat_tab_root(
                snap.side_tab_ifaces()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .find(|row| row.index() == COMBAT_TAB)
                    .map_or(-1, |row| row.id()),
            );
        }
        if snap.has_chat_text() {
            p.chat_text(strings.text(snap.chat_text().unwrap_or_default()));
        }
        if snap.has_chat_options() {
            // Empty texts stay: the 1-based answer index is the posted slot.
            p.chat_options(
                snap.chat_options()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| row.text().unwrap_or_default().to_string())
                    .collect(),
            );
            p.chat_option_ids(
                snap.chat_options()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| row.com_id())
                    .collect(),
            );
        }
        if snap.has_chat_lines() {
            p.chat_lines(
                snap.chat_lines()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|line| ChatLine {
                        seq: line.seq(),
                        text: Text::from(line.text().unwrap_or_default()),
                    })
                    .collect(),
            );
        }
        if snap.has_make_products() {
            p.make_products(
                snap.make_products()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|product| MakeProduct {
                        object_id: product.object_id(),
                        name: product.name().unwrap_or_default().to_string(),
                        buttons: product
                            .buttons()
                            .into_iter()
                            .flat_map(|buttons| buttons.iter())
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
            p.has_booth_stands(
                snap.banks()
                    .into_iter()
                    .flat_map(|stands| stands.iter())
                    .any(|stand| stand.kind() == Some("booth")),
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
                name: booth
                    .name()
                    .filter(|name| !name.is_empty())
                    .map(|name| strings.text(name)),
                op: booth
                    .op()
                    .filter(|op| !op.is_empty())
                    .map(|op| strings.text(op)),
            });
        }
        if snap.has_bank_approaches() {
            p.bank_approaches(
                snap.bank_approaches()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
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
        if let Some(result) = snap.bank_selection() {
            p.bank_selection(result);
        }
        if let Some(pair) = snap.main_modal_texts() {
            p.main_modal_texts(ModalTexts {
                root: pair.root(),
                texts: pair
                    .texts()
                    .into_iter()
                    .flat_map(|texts| texts.iter())
                    .map(str::to_string)
                    .collect(),
            });
        }
        if snap.has_widgets() {
            p.widgets(
                snap.widgets()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| WidgetRow {
                        component_id: row.component_id(),
                        text: strings.text(row.text().unwrap_or_default()),
                        item_count: row.item_count(),
                    })
                    .collect(),
            );
        }
        if snap.has_quest_statuses_available() {
            if snap.quest_statuses_available() {
                p.quest_statuses(QuestTab::Bound(
                    snap.quest_statuses()
                        .into_iter()
                        .flat_map(|rows| rows.iter())
                        .map(|row| QuestStatusRow::read(&row, strings))
                        .collect(),
                ));
            } else {
                p.quest_statuses(QuestTab::Unbound);
            }
        } else if snap.has_quest_statuses() {
            p.quest_statuses(QuestTab::Bound(
                snap.quest_statuses()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| QuestStatusRow::read(&row, strings))
                    .collect(),
            ));
        }
        if let Some(c) = snap.collision() {
            p.collision(CollisionQuery {
                available: c.available(),
                base_x: c.base_x(),
                base_z: c.base_z(),
                level: c.level(),
                width: c.width(),
                height: c.height(),
                flags: c
                    .flags()
                    .into_iter()
                    .flat_map(|flags| flags.iter())
                    .collect(),
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
                walkable: r
                    .walkable()
                    .into_iter()
                    .flat_map(|values| values.iter())
                    .collect(),
                step: r
                    .step()
                    .into_iter()
                    .flat_map(|values| values.iter())
                    .collect(),
                canlight: r
                    .canlight()
                    .into_iter()
                    .flat_map(|values| values.iter())
                    .collect(),
            });
        }
        if snap.has_puzzle_board() {
            let board = snap.puzzle_board();
            p.puzzle_board(PuzzlePage {
                component_id: board.as_ref().map(|b| b.component_id()).unwrap_or(-1),
                size: board.as_ref().map(|b| b.size()).unwrap_or(0),
                items: board
                    .and_then(|board| board.items())
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| PuzzlePiece {
                        slot: row.slot(),
                        id: row.id(),
                    })
                    .collect(),
                generation: snap.puzzle_board_generation(),
            });
        }
        if snap.has_walk_missing_carry() {
            p.walk_missing_carry(
                snap.walk_missing_carry()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .map(|row| CarryRow {
                        id: row.id(),
                        count: row.count(),
                        name: row.name().map(|name| strings.text(name)),
                    })
                    .collect(),
            );
        }
        if let Some(page) = snap.api_gather() {
            p.api_gather(read_gather_page(page));
        }
        if let Some(page) = snap.api_gather_outcome() {
            if let Some(outcome) = read_gather_outcome(page) {
                p.api_gather_outcome(outcome);
            }
        }
        if let Some(page) = snap.api_progress().and_then(read_api_progress) {
            p.api_progress(page);
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
                blocked: snap.walk_outcome_blocked(),
            });
            p.walk_outcome_cancel_reason(snap.walk_outcome_cancel_reason());
        }
    }
}

/// An owned copy of an op list, for observation types that keep `String`s.
pub fn strings(ops: &[Text]) -> Vec<String> {
    ops.iter().map(|op| op.to_string()).collect()
}

/// An op list from owned strings (test seams that write the scene).
pub fn ops_of(items: &[String]) -> Ops {
    items.iter().map(|item| Text::from(item.as_str())).collect()
}

/// Per-scene string interner for row names and op lists.
#[derive(Default)]
struct Interner {
    texts: HashSet<Text>,
    lists: HashMap<u64, Vec<Ops>>,
    lists_len: usize,
    empty: Ops,
    hasher: RandomState,
}

impl Interner {
    fn text(&mut self, s: &str) -> Text {
        if let Some(text) = self.texts.get(s) {
            return Rc::clone(text);
        }
        self.make_room();
        let text: Text = Rc::from(s);
        self.texts.insert(Rc::clone(&text));
        text
    }

    fn ops<'a>(&mut self, items: StringVector<'a>) -> Ops {
        let Some(items) = items else {
            return Rc::clone(&self.empty);
        };
        if items.is_empty() {
            return Rc::clone(&self.empty);
        }
        let mut hasher = self.hasher.build_hasher();
        items.len().hash(&mut hasher);
        for item in items.iter() {
            item.hash(&mut hasher);
        }
        let key = hasher.finish();
        let same = |list: &&Ops| {
            list.len() == items.len()
                && list
                    .iter()
                    .zip(items.iter())
                    .all(|(interned, item)| &**interned == item)
        };
        if let Some(list) = self
            .lists
            .get(&key)
            .and_then(|bucket| bucket.iter().find(same))
        {
            return Rc::clone(list);
        }
        let list: Ops = items.iter().map(|item| self.text(item)).collect();
        self.make_room();
        self.lists.entry(key).or_default().push(Rc::clone(&list));
        self.lists_len += 1;
        list
    }

    fn make_room(&mut self) {
        if self.texts.len() + self.lists_len >= INTERN_CAP {
            self.texts.clear();
            self.lists.clear();
            self.lists_len = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_delta, encode_snapshot_delta_with_native,
        encode_snapshot_with_native, BankStandInput, ItemRowInput, NativeFactsInput,
        QuestStatusInput, ReachViewInput, SceneEntityInput, SideTabIfaceInput, SnapshotInput,
        StatInput, TileInput,
    };

    fn empty(tick: u64) -> SnapshotInput<'static> {
        crate::isolate_fb::tests::empty_input(tick)
    }

    fn apply_bytes(bytes: &[u8]) {
        apply(&Snapshot::from_bytes(bytes).expect("snapshot"));
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
            shape: 0,
            angle: 0,
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
    fn user_move_intent_is_visible_latest_and_since_login_without_new_outcome() {
        on_reset();
        let mut initial = empty(1);
        initial.ingame = true;
        let (keyframe, fingerprint) = encode_snapshot_delta(None, &initial, false);
        apply_bytes(&keyframe);

        let mut movement = empty(2);
        movement.ingame = true;
        movement.user_move_intent_seq = 3;
        movement.walk_outcome_cancel_reason = crate::isolate_fb::WalkCancelReason::UserInput;
        let native = NativeFactsInput {
            walk_outcome_seq: 10,
            ..NativeFactsInput::default()
        };
        let (movement_delta, fingerprint) =
            encode_snapshot_delta_with_native(Some(&fingerprint), &movement, native, false);
        apply_bytes(&movement_delta);
        with(|scene| {
            assert_eq!(scene.latest().user_move_intent_seq(), Some(3));
            assert_eq!(scene.since_login().user_move_intent_seq(), Some(3));
            assert_eq!(scene.latest().walk_outcome_seq(), Some(10));
            assert_eq!(
                scene.latest().walk_outcome_cancel_reason(),
                Some(crate::isolate_fb::WalkCancelReason::UserInput)
            );
        });

        let mut later_movement = empty(3);
        later_movement.ingame = true;
        later_movement.user_move_intent_seq = 4;
        later_movement.walk_outcome_cancel_reason = crate::isolate_fb::WalkCancelReason::UserInput;
        let native = NativeFactsInput {
            walk_outcome_seq: 10,
            ..NativeFactsInput::default()
        };
        let (intent_only_delta, _) =
            encode_snapshot_delta_with_native(Some(&fingerprint), &later_movement, native, false);
        let intent_only = Snapshot::from_bytes(&intent_only_delta).expect("intent delta");
        assert!(!intent_only.has_walk_outcome_seq());
        apply(&intent_only);
        with(|scene| {
            assert_eq!(scene.latest().user_move_intent_seq(), Some(4));
            assert_eq!(scene.since_login().user_move_intent_seq(), Some(4));
            assert_eq!(scene.latest().walk_outcome_seq(), Some(10));
            assert_eq!(
                scene.latest().walk_outcome_cancel_reason(),
                Some(crate::isolate_fb::WalkCancelReason::UserInput)
            );
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
        let reader = Snapshot::from_bytes(&delta).unwrap();
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

    #[test]
    fn a_logout_post_that_carries_pages_is_seen_only_since_logout() {
        on_reset();
        let npcs = [goblin(1)];
        let mut snap = empty(1);
        snap.ingame = true;
        snap.here = Some(TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        });
        let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
        apply_bytes(&keyframe);
        // The logout post itself carries a new tile and an npc table.
        snap.tick = 2;
        snap.ingame = false;
        snap.here = Some(TileInput {
            x: 3210,
            z: 3200,
            level: 0,
        });
        snap.npcs = &npcs;
        let (logout, fp) = encode_snapshot_delta(Some(&fp), &snap, false);
        let reader = Snapshot::from_bytes(&logout).unwrap();
        assert!(reader.has_here() && reader.has_npcs() && reader.has_ingame());
        apply(&reader);
        let moved = Tile {
            x: 3210,
            z: 3200,
            level: 0,
        };
        with(|scene| {
            // Machines that forget the session ignore the whole logout post.
            assert_eq!(scene.since_login().here(), None);
            assert!(scene.since_login().npcs().is_none());
            // The hunt `here` rule: a tile posted with the logout counts.
            assert_eq!(scene.since_logout().here(), Some(moved));
            assert_eq!(scene.since_logout().npcs().map(Vec::len), Some(1));
            assert_eq!(scene.latest().here(), Some(moved));
        });
        // A later post with no tile: since_logout still sees the logout's.
        snap.tick = 3;
        let (quiet, fp) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&quiet);
        with(|scene| {
            assert_eq!(scene.since_logout().here(), Some(moved));
            assert_eq!(scene.since_login().here(), None);
        });
        // A second logout moves the mark: the first logout's pages drop out
        // of since_logout too.
        snap.tick = 4;
        snap.ingame = true;
        let (login, fp) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&login);
        snap.tick = 5;
        snap.ingame = false;
        let (again, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&again);
        with(|scene| {
            assert_eq!(scene.since_logout().here(), None);
            assert_eq!(scene.since_logout().ingame(), Some(false));
        });
    }

    #[test]
    fn skills_keep_the_first_matching_row_and_absent_skills_stay_absent() {
        on_reset();
        let stats = [
            StatInput {
                index: 3,
                name: "Hitpoints",
                xp: 1,
                base: 1,
                effective: 1,
            },
            StatInput {
                index: 3,
                name: "hitpoints",
                xp: 1154,
                base: 10,
                effective: 7,
            },
            StatInput {
                index: 6,
                name: "Magic",
                xp: 83,
                base: 2,
                effective: 3,
            },
            StatInput {
                index: 6,
                name: "magic",
                xp: 0,
                base: 1,
                effective: 1,
            },
        ];
        let mut snap = empty(1);
        snap.stats = &stats;
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| {
            let skills = scene.latest().stats().expect("posted stats");
            // `hitpoints` is an exact match; the others ignore case.
            assert_eq!(skills.hitpoints.map(|row| row.effective), Some(7));
            assert_eq!(
                skills.magic,
                Some(Skill {
                    xp: 83,
                    base: 2,
                    effective: 3
                })
            );
            assert_eq!(skills.prayer, None);
            assert_eq!(skills.firemaking, None);
        });
    }

    #[test]
    fn side_tab_and_bank_stand_tables_keep_only_the_read_facts() {
        on_reset();
        let tabs = [
            SideTabIfaceInput { index: 1, id: 3917 },
            SideTabIfaceInput { index: 0, id: 328 },
        ];
        let banks = [BankStandInput {
            name: "Banker",
            x: 1,
            z: 2,
            level: 0,
            kind: "npc",
            op: 1,
            choose: None,
        }];
        let mut snap = empty(1);
        snap.side_tab_ifaces = &tabs;
        snap.banks = &banks;
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| {
            assert_eq!(scene.latest().combat_tab_root(), Some(328));
            assert_eq!(scene.latest().has_booth_stands(), Some(false));
        });
        let no_combat = [SideTabIfaceInput { index: 1, id: 3917 }];
        snap.tick = 2;
        snap.side_tab_ifaces = &no_combat;
        snap.banks = &[];
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| {
            assert_eq!(scene.latest().combat_tab_root(), Some(-1));
            assert_eq!(scene.latest().has_booth_stands(), Some(false));
        });
    }

    #[test]
    fn repeated_names_and_op_lists_share_one_allocation_across_posts() {
        on_reset();
        let ops = ["Talk-to".to_string(), "Attack".to_string()];
        let mut npc = goblin(1);
        npc.actions = &ops;
        let first = [npc, goblin(2)];
        let mut snap = empty(1);
        snap.npcs = &first;
        snap.locs = &first;
        apply_bytes(&encode_snapshot(&snap));
        let (name, actions) = with(|scene| {
            let npcs = scene.latest().npcs().unwrap();
            let locs = scene.latest().locs().unwrap();
            // One name for both npc rows and the loc rows of the same name.
            assert!(Rc::ptr_eq(
                npcs[0].name.as_ref().unwrap(),
                npcs[1].name.as_ref().unwrap()
            ));
            assert!(Rc::ptr_eq(
                npcs[0].name.as_ref().unwrap(),
                locs[0].name.as_ref().unwrap()
            ));
            assert!(Rc::ptr_eq(&npcs[0].actions, &locs[0].actions));
            assert_eq!(&*npcs[0].actions[0], "Talk-to");
            (npcs[0].name.clone().unwrap(), Rc::clone(&npcs[0].actions))
        });
        // A later post of the same rows reuses them instead of copying.
        snap.tick = 2;
        apply_bytes(&encode_snapshot(&snap));
        with(|scene| {
            let npcs = scene.latest().npcs().unwrap();
            assert!(Rc::ptr_eq(npcs[0].name.as_ref().unwrap(), &name));
            assert!(Rc::ptr_eq(&npcs[0].actions, &actions));
        });
    }

    #[test]
    fn reach_stamp_moves_only_with_a_new_table_or_a_reset() {
        on_reset();
        let bits = [1u32];
        let steps = [0xffu8];
        let mut snap = empty(1);
        snap.reach = ReachViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 1,
            height: 1,
            walkable: &bits,
            reachable: &bits,
            reachable_adj: &bits,
            exact_rank: &[],
            adjacent_rank: &[],
            step: &steps,
            canlight: &bits,
            stamp: 0,
        };
        let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
        apply_bytes(&keyframe);
        let first = with(|scene| scene.latest().reach_stamp()).expect("posted reach");
        snap.tick = 2;
        let (same, fp) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&same);
        assert_eq!(with(|scene| scene.latest().reach_stamp()), Some(first));
        snap.tick = 3;
        snap.reach.base_x = 3201;
        let (moved, _) = encode_snapshot_delta(Some(&fp), &snap, false);
        apply_bytes(&moved);
        let second = with(|scene| scene.latest().reach_stamp()).unwrap();
        assert_ne!(second, first);
        on_reset();
        assert_eq!(with(|scene| scene.latest().reach_stamp()), None);
        apply_bytes(&keyframe);
        let after_reset = with(|scene| scene.latest().reach_stamp()).unwrap();
        assert_ne!(after_reset, first, "a new session never repeats a stamp");
    }

    #[test]
    fn quest_tab_unbound_and_bound_rows_are_distinct_pages() {
        on_reset();
        let snap = empty(1);
        apply_bytes(&encode_snapshot_with_native(
            &snap,
            NativeFactsInput {
                quest_statuses: None,
                ..Default::default()
            },
        ));
        with(|scene| {
            assert!(matches!(
                scene.latest().quest_statuses(),
                Some(QuestTab::Unbound)
            ));
        });
        let rows = [QuestStatusInput {
            name: "Death Plateau",
            status: "inProgress",
            component_id: Some(42),
        }];
        apply_bytes(&encode_snapshot_with_native(
            &snap,
            NativeFactsInput {
                quest_statuses: Some(&rows),
                ..Default::default()
            },
        ));
        with(|scene| match scene.latest().quest_statuses() {
            Some(QuestTab::Bound(posted)) => {
                assert_eq!(&*posted[0].name, "Death Plateau");
                assert_eq!(&*posted[0].status, "inProgress");
                assert_eq!(posted[0].component_id, Some(42));
            }
            other => panic!("expected bound tab, got {other:?}"),
        });
    }
}

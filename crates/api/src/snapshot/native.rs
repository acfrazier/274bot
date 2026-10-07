//! Borrowed native observations with readiness attached to their evidence stamp.
use super::{
    ActorTargetView, ChatLineView, ChatOptionView, GameSnapshot, GroundItemView, HitmarksView,
    ItemView, LocView, LocalPlayerView, NpcView, PlayerView, ProjectileView, QuestStatusView,
    SideTabView, StatView, VarpView, WorldStateView,
};
use crate::bank_memory::BankMemory;
use crate::quest_progress::EvidenceStamp;
use crate::selected::Truth;
use crate::stock::Stock;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, Copy)]
pub struct Observed<T> {
    pub value: T,
    pub stamp: EvidenceStamp,
}

#[derive(Debug, Clone, Copy)]
pub struct JournalModalView<'a> {
    pub root: i32,
    pub texts: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankSessionView {
    pub open: bool,
    pub generation: u64,
    /// `modals().side` (`-1` none): a close is acknowledged once it is
    /// released or changed.
    pub side: i32,
}

/// A journal page borrowed from the currently open main root. The caller
/// supplies the selected catalog's root and title component; widget order is
/// never used to infer which text is the title.
#[derive(Clone, Copy)]
pub struct JournalWidgetView<'a> {
    pub title: &'a str,
    root: i32,
    title_component: i32,
    widgets: &'a [super::WidgetView],
}

impl<'a> JournalWidgetView<'a> {
    /// Body text in the host's observed widget order, without the title.
    pub fn lines(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.widgets.iter().filter_map(|widget| {
            (widget.root == super::WidgetRoot::Main
                && widget.root_component_id == self.root
                && widget.component_id != self.title_component
                && !widget.hidden)
                .then_some(widget.text.as_deref())
                .flatten()
        })
    }
}

/// A host frame borrow; readiness comes from the snapshot, never caller flags.
#[derive(Clone, Copy)]
pub struct SnapshotView<'a> {
    snapshot: Option<&'a GameSnapshot>,
    stamp: EvidenceStamp,
    reach: Option<&'a crate::query::ReachQueryView>,
    bank_memory: Option<&'a BankMemory>,
    world_members: Truth,
}

impl<'a> SnapshotView<'a> {
    /// Attach the host's run/session stamp to its current observation. `None`
    /// represents a frame without a snapshot, not an observed empty world.
    pub fn new(snapshot: Option<&'a GameSnapshot>, stamp: EvidenceStamp) -> Self {
        Self {
            snapshot,
            stamp,
            reach: None,
            bank_memory: None,
            world_members: Truth::Unknown,
        }
    }

    /// Attach the bound server profile's world type. The host derives it from
    /// the profile, not from this snapshot, so a view built without it reports
    /// [`Truth::Unknown`].
    pub fn with_world_members(mut self, world_members: Truth) -> Self {
        self.world_members = world_members;
        self
    }

    /// Whether the connected world is a members world, as declared by the
    /// bound server profile. [`Truth::Unknown`] only when no profile fact was
    /// attached (for example a Direct connection). This is the world, not the
    /// account: [`WorldStateView::members`] is the account flag the server
    /// sent at login and can be true on a free-to-play world.
    pub fn world_members(&self) -> Truth {
        self.world_members
    }

    pub fn here(&self) -> Option<Observed<super::WorldTile>> {
        let snapshot = self.snapshot?;
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return None;
        }
        Some(Observed {
            value: snapshot.local_player()?.player.actor.tile,
            stamp: self.stamp,
        })
    }
    /// Anchor-radius arrival for a plain tile walk, regardless of scene locs.
    pub fn walk_arrived(&self, from: super::WorldTile, to: super::WorldTile, radius: i32) -> bool {
        let Some(snapshot) = self.scene_ready() else {
            return false;
        };
        if from == to {
            return radius >= 0;
        }
        let reach = self.reach();
        let query = crate::query::SceneQuery::new(snapshot.scene(), None);
        if !query.contains(to) && !reach.is_some_and(|observed| observed.value.probeable(to)) {
            return false;
        }
        let unavailable = crate::query::ReachQueryView::unavailable();
        crate::query::is_arrived(from, to, radius, || {
            reach.map_or(&unavailable, |observed| observed.value)
        })
    }

    /// Arrival for a walk whose caller explicitly supplied loc identity.
    /// An observed gone/replaced loc settles under the plain tile predicate.
    pub fn walk_loc_arrived(
        &self,
        from: super::WorldTile,
        to: super::WorldTile,
        radius: i32,
        loc_id: i32,
    ) -> bool {
        self.scene_ready().is_some_and(|snapshot| {
            match crate::query::loc_approach::arrived_at(snapshot, from, to, radius, loc_id) {
                Some(arrived) => arrived,
                None => {
                    crate::query::loc_approach::target_gone(snapshot, to, loc_id)
                        && self.walk_arrived(from, to, radius)
                }
            }
        })
    }

    /// The local player once the scene and player family are ready.
    /// Disconnect, a loading scene, or a pre-player frame is not a posted row.
    pub fn local_player(&self) -> Option<Observed<&'a LocalPlayerView>> {
        let snapshot = self.scene_ready()?;
        if !snapshot.players_available {
            return None;
        }
        Some(Observed {
            value: snapshot.local_player()?,
            stamp: self.stamp,
        })
    }

    /// Remote player rows once the scene and player family are ready.
    /// A posted empty slice means no remote players; pre-player frames remain
    /// unavailable.
    pub fn players(&self) -> Option<Observed<&'a [PlayerView]>> {
        let snapshot = self.scene_ready()?;
        snapshot.players_available.then_some(Observed {
            value: snapshot.players(),
            stamp: self.stamp,
        })
    }

    /// World scalars once a build origin has been observed. The origin is the
    /// readiness marker because the rectangle is meaningless before a build.
    pub fn world(&self) -> Option<Observed<&'a WorldStateView>> {
        let snapshot = self.ingame()?;
        snapshot.base()?;
        Some(Observed {
            value: snapshot.world(),
            stamp: self.stamp,
        })
    }

    /// Inventory capacity after the inventory component has posted a
    /// positive slot count. Empty slots are omitted from `inventory()`.
    pub fn inventory_capacity(&self) -> Option<Observed<u8>> {
        let snapshot = self.ingame()?;
        let capacity = snapshot.inventory_size();
        (capacity > 0).then_some(Observed {
            value: capacity as u8,
            stamp: self.stamp,
        })
    }

    /// Attach the cached host reach planes belonging to this frame.
    pub fn with_reach(mut self, reach: Option<&'a crate::query::ReachQueryView>) -> Self {
        self.reach = reach;
        self
    }

    /// Attach the account's bank memory belonging to this frame.
    pub fn with_bank_memory(mut self, bank_memory: Option<&'a BankMemory>) -> Self {
        self.bank_memory = bank_memory;
        self
    }

    /// The account's last-seen bank, whatever the frame's state: the memory
    /// outlives a login.
    pub fn bank_memory(&self) -> Option<&'a BankMemory> {
        self.bank_memory
    }

    /// The frame's item facts for planning: the pack and worn pages under
    /// the same gates as `inventory()` / `equipment()`, plus the bank memory.
    pub fn stock(&self) -> Stock<'a> {
        let mut stock = self.ingame().map(Stock::pages).unwrap_or_default();
        stock.bank = self.bank_memory;
        stock
    }

    pub fn reach(&self) -> Option<Observed<&crate::query::ReachQueryView>> {
        let snapshot = self.snapshot?;
        let reach = self.reach?;
        (snapshot.ingame() && snapshot.scene_state() == 2 && reach.available).then_some(Observed {
            value: reach,
            stamp: self.stamp,
        })
    }

    pub fn inventory(&self) -> Option<Observed<&'a [ItemView]>> {
        let snapshot = self.snapshot?;
        (snapshot.ingame() && snapshot.inventory_size() > 0).then(|| Observed {
            value: snapshot.inventory(),
            stamp: self.stamp,
        })
    }

    pub fn bank(&self) -> Option<Observed<&[ItemView]>> {
        let snapshot = self.snapshot?;
        (snapshot.ingame() && snapshot.bank_loaded()).then(|| Observed {
            value: snapshot.bank(),
            stamp: self.stamp,
        })
    }

    /// Whether the bank modal is open, with the current session generation.
    /// `bank()` remains the separate readiness gate for its item table.
    pub fn bank_session(&self) -> Option<Observed<BankSessionView>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: BankSessionView {
                open: snapshot.bank_component_id() >= 0,
                generation: snapshot.bank_session_generation(),
                side: snapshot.modals().side,
            },
            stamp: self.stamp,
        })
    }

    /// The bank-side backpack once an open bank has raised its side root.
    /// `None` while the side root is down, even if a leftover list is empty;
    /// `Some(&[])` is a posted empty pack. Never inferred from list length.
    pub fn bank_side(&self) -> Option<Observed<&[ItemView]>> {
        let snapshot = self.ingame()?;
        (snapshot.bank_component_id() >= 0 && snapshot.modals().side >= 0).then(|| Observed {
            value: snapshot.bank_side(),
            stamp: self.stamp,
        })
    }

    /// Current shop modal and its posted stock/player containers.
    pub fn shop(&self) -> Option<Observed<&super::ShopView>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.shop(),
            stamp: self.stamp,
        })
    }

    /// Current trade roots, counterpart and posted offer/confirmation containers.
    pub fn trade(&self) -> Option<Observed<&'a super::TradeView>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.trade(),
            stamp: self.stamp,
        })
    }

    /// Products and quantity buttons posted by the chat make menu.
    pub fn make_products(&self) -> Option<Observed<&[super::MakeProductView]>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.make_products(),
            stamp: self.stamp,
        })
    }

    /// Whether the numeric count-entry dialog is currently open.
    pub fn count_dialog_open(&self) -> Option<Observed<bool>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.count_dialog_open(),
            stamp: self.stamp,
        })
    }

    pub fn quest_statuses(&self) -> Option<Observed<&[QuestStatusView]>> {
        let snapshot = self.snapshot?;
        (snapshot.ingame() && snapshot.quest_statuses_available()).then(|| Observed {
            value: snapshot.quest_statuses(),
            stamp: self.stamp,
        })
    }

    pub fn widgets(&self) -> Option<Observed<&'a [super::WidgetView]>> {
        let snapshot = self.snapshot?;
        snapshot.ingame().then(|| Observed {
            value: snapshot.widgets(),
            stamp: self.stamp,
        })
    }

    pub fn journal_widgets(
        &self,
        root: i32,
        title_component: i32,
    ) -> Option<Observed<JournalWidgetView<'a>>> {
        let snapshot = self.snapshot?;
        if !snapshot.ingame() || root < 0 || snapshot.modals().main != root {
            return None;
        }
        let widgets = snapshot.widgets();
        let title = widgets
            .iter()
            .find(|widget| {
                widget.root == super::WidgetRoot::Main
                    && widget.root_component_id == root
                    && widget.component_id == title_component
                    && !widget.hidden
            })?
            .text
            .as_deref()?;
        Some(Observed {
            value: JournalWidgetView {
                title,
                root,
                title_component,
                widgets,
            },
            stamp: self.stamp,
        })
    }

    pub fn main_modal(&self) -> Option<Observed<JournalModalView<'_>>> {
        let snapshot = self.snapshot?;
        snapshot.ingame().then(|| Observed {
            value: JournalModalView {
                root: snapshot.modals().main,
                texts: snapshot.main_modal_texts(),
            },
            stamp: self.stamp,
        })
    }

    fn ingame(&self) -> Option<&'a GameSnapshot> {
        let snapshot = self.snapshot?;
        snapshot.ingame().then_some(snapshot)
    }

    fn scene_ready(&self) -> Option<&'a GameSnapshot> {
        let snapshot = self.ingame()?;
        (snapshot.scene_state() == 2).then_some(snapshot)
    }

    /// Bounded projectile rows: local-target threats first, then targeted
    /// launches from the local tile. An empty list is observed once the scene
    /// is ready; before then it is unavailable.
    pub fn projectiles(&self) -> Option<Observed<&'a [ProjectileView]>> {
        let snapshot = self.scene_ready()?;
        Some(Observed {
            value: snapshot.projectiles(),
            stamp: self.stamp,
        })
    }

    /// The four raw local hitmarks once the scene and player family are ready.
    pub fn hitmarks(&self) -> Option<Observed<HitmarksView>> {
        let snapshot = self.scene_ready()?;
        if !snapshot.players_available {
            return None;
        }
        snapshot.local_player()?;
        Some(Observed {
            value: *snapshot.hitmarks()?,
            stamp: self.stamp,
        })
    }

    /// Side-tab rows after their interface family has posted.
    pub fn side_tabs(&self) -> Option<Observed<&'a [SideTabView]>> {
        let snapshot = self.ingame()?;
        snapshot.side_tabs_available.then_some(Observed {
            value: snapshot.side_tabs(),
            stamp: self.stamp,
        })
    }

    /// The selected side-tab index after the side-tab family has posted.
    pub fn active_side_tab(&self) -> Option<Observed<i32>> {
        let snapshot = self.ingame()?;
        snapshot.side_tabs_available.then_some(Observed {
            value: snapshot.active_side_tab(),
            stamp: self.stamp,
        })
    }

    /// Nearby NPCs once the scene is built. Disconnect or a rebuild is not
    /// an empty crowd.
    pub fn npcs(&self) -> Option<Observed<&'a [NpcView]>> {
        let snapshot = self.scene_ready()?;
        Some(Observed {
            value: snapshot.npcs(),
            stamp: self.stamp,
        })
    }

    /// Placed locs once the scene is built.
    pub fn locs(&self) -> Option<Observed<&'a [LocView]>> {
        let snapshot = self.scene_ready()?;
        Some(Observed {
            value: snapshot.locs(),
            stamp: self.stamp,
        })
    }

    /// Ground-item stacks once the scene is built.
    pub fn ground_items(&self) -> Option<Observed<&'a [GroundItemView]>> {
        let snapshot = self.scene_ready()?;
        Some(Observed {
            value: snapshot.ground_items(),
            stamp: self.stamp,
        })
    }

    /// Worn items. An in-game frame posts the worn table; disconnect is not
    /// an empty worn set.
    pub fn equipment(&self) -> Option<Observed<&'a [ItemView]>> {
        let snapshot = self.ingame()?;
        if !snapshot.equipment_available {
            return None;
        }
        Some(Observed {
            value: snapshot.equipment(),
            stamp: self.stamp,
        })
    }

    /// All 25 skill slots from the last stat rebuild.
    pub fn stats(&self) -> Option<Observed<&'a [StatView]>> {
        let snapshot = self.ingame()?;
        if snapshot.stats().is_empty()
            || snapshot
                .stats()
                .iter()
                .any(|stat| stat.used && stat.base <= 0)
        {
            return None;
        }
        Some(Observed {
            value: snapshot.stats(),
            stamp: self.stamp,
        })
    }

    /// Chat history newer than `since` (the last observed `sequence`).
    /// Newest-first ring order is preserved; no allocation.
    pub fn chat_lines(&self, since: i32) -> Option<Observed<ChatLines<'a>>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: ChatLines {
                lines: snapshot.chat_lines(),
                since,
            },
            stamp: self.stamp,
        })
    }

    /// Chat modal root, body texts, BUTTON_OK options, and continue id.
    /// `root == -1` is an observed closed chat, not unreadiness.
    pub fn chat_modal(&self) -> Option<Observed<ChatModalView<'a>>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: ChatModalView {
                root: snapshot.modals().chat,
                texts: snapshot.chat_modal_texts(),
                options: snapshot.chat_options(),
                continue_component_id: snapshot.chat_continue_component_id(),
            },
            stamp: self.stamp,
        })
    }

    /// BUTTON_OK choices of the open chat modal.
    pub fn chat_options(&self) -> Option<Observed<&'a [ChatOptionView]>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.chat_options(),
            stamp: self.stamp,
        })
    }

    /// Continue-button component; `-1` while latched or closed.
    pub fn chat_continue(&self) -> Option<Observed<i32>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.chat_continue_component_id(),
            stamp: self.stamp,
        })
    }

    /// Main-modal TYPE_TEXT lines. Distinct from [`Self::main_modal`], which
    /// also carries the root id.
    pub fn main_modal_texts(&self) -> Option<Observed<&'a [String]>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.main_modal_texts(),
            stamp: self.stamp,
        })
    }

    /// Client varp table, one view per definition. Presence of a row is the
    /// live table; callers decide transmission from selected configs.
    pub fn varps(&self) -> Option<Observed<&'a [VarpView]>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.varps(),
            stamp: self.stamp,
        })
    }

    /// Last rebuilt run energy. `0` is a real posted value once in-game.
    pub fn run_energy(&self) -> Option<Observed<i32>> {
        let snapshot = self.ingame()?;
        Some(Observed {
            value: snapshot.runenergy(),
            stamp: self.stamp,
        })
    }

    /// Local combat flag and current target. Unready without a local player.
    pub fn in_combat(&self) -> Option<Observed<CombatView>> {
        let snapshot = self.ingame()?;
        let player = snapshot.local_player()?;
        Some(Observed {
            value: CombatView {
                in_combat: player.player.actor.in_combat,
                target: player.player.actor.target,
            },
            stamp: self.stamp,
        })
    }

    /// Overlay prayer bits from varps 83–97. Missing rows stay off; disconnect
    /// is unreadiness, not an all-off overlay.
    pub fn prayers_active(&self) -> Option<Observed<[bool; 15]>> {
        let snapshot = self.ingame()?;
        let mut bits = [false; 15];
        for varp in snapshot.varps() {
            let Some(slot) = varp
                .index
                .checked_sub(83)
                .and_then(|delta| usize::try_from(delta).ok())
            else {
                continue;
            };
            if slot < 15 {
                bits[slot] = varp.value == 1;
            }
        }
        Some(Observed {
            value: bits,
            stamp: self.stamp,
        })
    }
}

/// Newest-first chat lines with sequence strictly after `since`.
#[derive(Debug, Clone, Copy)]
pub struct ChatLines<'a> {
    lines: &'a [ChatLineView],
    since: i32,
}

impl<'a> ChatLines<'a> {
    pub fn iter(&self) -> impl Iterator<Item = &'a ChatLineView> + '_ {
        self.lines.iter().filter(|line| line.sequence > self.since)
    }
}

/// Chat-modal observation: closed is `root == -1`, not a missing field.
#[derive(Debug, Clone, Copy)]
pub struct ChatModalView<'a> {
    pub root: i32,
    pub texts: &'a [String],
    pub options: &'a [ChatOptionView],
    pub continue_component_id: i32,
}

/// Fingerprint the native dialogue page, not the unrelated chat-history ring.
/// Roots and Continue visibility are acknowledged separately by the driver.
/// Borrowed content is hashed in modal walk order without copying the page.
pub fn chat_page_fingerprint<'a, T: Hash>(
    texts: &[T],
    options: impl IntoIterator<Item = (i32, &'a str)>,
) -> u64 {
    let mut hash = DefaultHasher::new();
    texts.hash(&mut hash);
    for (component_id, text) in options {
        component_id.hash(&mut hash);
        text.hash(&mut hash);
    }
    hash.finish()
}

/// Local combat observation from the player actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatView {
    pub in_combat: bool,
    pub target: Option<ActorTargetView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::RunKey;
    use crate::snapshot::HitmarkView;

    #[test]
    fn trade_view_is_borrowed_and_unready_outside_a_session() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 2,
                run: 3,
                session: 4,
            },
            tick: 7,
            sequence: 8,
        };
        let mut snapshot = GameSnapshot::new();
        assert!(SnapshotView::new(Some(&snapshot), stamp).trade().is_none());
        snapshot.seed_ingame(2);
        snapshot.seed_trade(super::super::TradeView {
            offer_open: true,
            partner: Some("configured peer".into()),
            accept_component_id: 3420,
            ..Default::default()
        });
        let observed = SnapshotView::new(Some(&snapshot), stamp).trade().unwrap();
        assert!(std::ptr::eq(observed.value, snapshot.trade()));
        assert_eq!(observed.stamp, stamp);
        assert_eq!(observed.value.partner.as_deref(), Some("configured peer"));
    }

    #[test]
    fn dialogue_page_fingerprint_tracks_all_texts_and_option_component_text_pairs() {
        let texts = ["First line", "Second line"];
        let options = [(4883, "Yes"), (4884, "No")];
        let first = chat_page_fingerprint(&texts, options);
        assert_eq!(
            first,
            chat_page_fingerprint(&texts.map(String::from), options),
            "owned and borrowed views of the same native page hash identically"
        );
        assert_ne!(
            first,
            chat_page_fingerprint(&["First line", "Changed"], options)
        );
        assert_ne!(
            first,
            chat_page_fingerprint(&["Second line", "First line"], options)
        );
        assert_ne!(
            first,
            chat_page_fingerprint(&texts, [(4885, "Yes"), (4884, "No")])
        );
        assert_ne!(
            first,
            chat_page_fingerprint(&texts, [(4883, "Maybe"), (4884, "No")])
        );
        assert_ne!(
            first,
            chat_page_fingerprint(&texts, [(4884, "No"), (4883, "Yes")])
        );
    }

    #[test]
    fn unavailable_fields_never_look_like_observed_empty_fields() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 2,
                run: 3,
                session: 4,
            },
            tick: 7,
            sequence: 8,
        };
        let mut snapshot = GameSnapshot::default();
        for view in [
            SnapshotView::new(None, stamp),
            SnapshotView::new(Some(&snapshot), stamp),
        ] {
            assert!(view.local_player().is_none());
            assert!(view.players().is_none());
            assert!(view.projectiles().is_none());
            assert!(view.hitmarks().is_none());
            assert!(view.side_tabs().is_none());
            assert!(view.active_side_tab().is_none());
            assert!(view.world().is_none());
            assert!(view.inventory_capacity().is_none());
            assert!(view.inventory().is_none());
            assert!(view.bank().is_none());
            assert!(view.quest_statuses().is_none());
            assert!(view.main_modal().is_none());
            assert!(view.npcs().is_none());
            assert!(view.locs().is_none());
            assert!(view.ground_items().is_none());
            assert!(view.equipment().is_none());
            assert!(view.stats().is_none());
            assert!(view.chat_lines(0).is_none());
            assert!(view.chat_modal().is_none());
            assert!(view.chat_options().is_none());
            assert!(view.chat_continue().is_none());
            assert!(view.main_modal_texts().is_none());
            assert!(view.varps().is_none());
            assert!(view.run_energy().is_none());
            assert!(view.in_combat().is_none());
            assert!(view.prayers_active().is_none());
        }
        snapshot.ingame = true;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.local_player().is_none());
        assert!(view.players().is_none());
        assert!(view.projectiles().is_none());
        assert!(view.hitmarks().is_none());
        assert!(view.side_tabs().is_none());
        assert!(view.active_side_tab().is_none());
        assert!(view.world().is_none());
        assert!(view.inventory_capacity().is_none());
        assert!(view.inventory().is_none());
        assert!(view.bank().is_none());
        assert!(view.quest_statuses().is_none());
        assert_eq!(view.main_modal().unwrap().value.root, -1);
        assert!(view.npcs().is_none());
        assert!(view.locs().is_none());
        assert!(view.ground_items().is_none());
        assert!(
            view.equipment().is_none(),
            "unposted worn inventory is not empty"
        );
        assert!(
            view.stats().is_none(),
            "unposted stats are not zero-level skills"
        );
        assert_eq!(view.chat_lines(0).unwrap().value.iter().count(), 0);
        assert_eq!(view.chat_modal().unwrap().value.root, -1);
        assert!(view.chat_options().unwrap().value.is_empty());
        assert_eq!(view.chat_continue().unwrap().value, -1);
        assert!(view.main_modal_texts().unwrap().value.is_empty());
        assert!(view.varps().unwrap().value.is_empty());
        assert_eq!(view.run_energy().unwrap().value, 0);
        assert!(view.in_combat().is_none());
        assert_eq!(view.prayers_active().unwrap().value, [false; 15]);
        snapshot.inventory_size = 28;
        snapshot.bank_loaded = true;
        snapshot.quest_statuses_available = true;
        snapshot.scene_state = 2;
        snapshot.runenergy = 100;
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "Hitpoints".into(),
            effective: 10,
            base: 10,
            xp: 1154,
            used: true,
        }]);
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.local_player().is_none());
        assert!(
            view.players().is_none(),
            "a scene-ready but unposted player family is not an empty list"
        );
        assert!(view.projectiles().unwrap().value.is_empty());
        assert!(view.hitmarks().is_none());
        assert!(view.side_tabs().is_none());
        assert!(view.active_side_tab().is_none());
        assert_eq!(view.inventory_capacity().unwrap().value, 28);
        assert!(view.inventory().unwrap().value.is_empty());
        assert!(view.bank().unwrap().value.is_empty());
        assert!(view.quest_statuses().unwrap().value.is_empty());
        assert_eq!(view.inventory().unwrap().stamp, stamp);
        assert!(view.npcs().unwrap().value.is_empty());
        assert!(view.locs().unwrap().value.is_empty());
        assert!(view.ground_items().unwrap().value.is_empty());
        assert_eq!(view.run_energy().unwrap().value, 100);
        assert!(
            view.equipment().unwrap().value.is_empty(),
            "posted empty equipment is observed"
        );
        assert_eq!(view.stats().unwrap().value[0].base, 10);
        // A disconnected frame cannot lend stale cached observations.
        snapshot.ingame = false;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.local_player().is_none());
        assert!(view.players().is_none());
        assert!(view.projectiles().is_none());
        assert!(view.hitmarks().is_none());
        assert!(view.side_tabs().is_none());
        assert!(view.active_side_tab().is_none());
        assert!(view.world().is_none());
        assert!(view.inventory_capacity().is_none());
        assert!(view.inventory().is_none());
        assert!(view.bank().is_none());
        assert!(view.quest_statuses().is_none());
        assert!(view.main_modal().is_none());
        assert!(view.npcs().is_none());
        assert!(view.equipment().is_none());
        assert!(view.chat_lines(0).is_none());
        assert!(view.varps().is_none());
        assert!(view.run_energy().is_none());
        assert!(view.prayers_active().is_none());
    }

    #[test]
    fn local_world_and_capacity_borrow_posted_facts() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 2,
                session: 3,
            },
            tick: 4,
            sequence: 5,
        };
        let mut snapshot = GameSnapshot {
            ingame: true,
            scene_state: 2,
            base: Some((3200, 3400)),
            world: super::super::WorldStateView {
                map_base_x: 3200,
                map_base_z: 3400,
                level: 1,
                members: true,
                multi_combat: false,
                player_count: 1,
                npc_count: 0,
                cycle: 99,
            },
            ..GameSnapshot::default()
        };
        snapshot.seed_local_player(super::super::LocalPlayerView {
            player: super::super::PlayerView {
                index: 7,
                network: super::super::WorldTile {
                    x: 3205,
                    z: 3405,
                    level: 1,
                },
                actor: super::super::ActorView {
                    name: Some("alice".into()),
                    actions: Vec::new(),
                    tile: super::super::WorldTile {
                        x: 3205,
                        z: 3405,
                        level: 1,
                    },
                    distance: 0,
                    animation: -1,
                    animation_frame: 0,
                    pose_animation: -1,
                    orientation: 0,
                    target_orientation: 0,
                    overhead_text: None,
                    spot_animation: -1,
                    spot_animation_stamp: 0,
                    health: 10,
                    total_health: 10,
                    face_entity: -1,
                    target: None,
                    moving: false,
                    running: false,
                    in_combat: false,
                },
                combat_level: 3,
                skill_level: 3,
                headicons: 0,
                weapon: Some(415),
            },
            energy: 77,
            weight: 0,
        });
        snapshot.side_tabs_available = true;
        snapshot.active_side_tab = 3;
        snapshot.hitmarks = Some(HitmarksView {
            marks: [
                HitmarkView {
                    value: -7,
                    kind: 99,
                    cycle: 111,
                },
                HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                },
                HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                },
                HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                },
            ],
            loop_cycle: 104,
        });
        snapshot.inventory_size = 28;

        let view = SnapshotView::new(Some(&snapshot), stamp);
        let local = view.local_player().expect("posted local player");
        assert!(std::ptr::eq(
            local.value,
            snapshot.local_player().expect("seeded local player")
        ));
        assert_eq!(local.value.energy, 77);

        assert_eq!(view.local_player().unwrap().value.player.weapon, Some(415));
        let players = view.players().expect("posted empty remote-player list");
        assert!(players.value.is_empty());
        assert_eq!(players.stamp, stamp);
        assert!(view.projectiles().unwrap().value.is_empty());
        let hitmarks = view.hitmarks().expect("posted local hitmarks");
        assert_eq!(hitmarks.value.loop_cycle, 104);
        assert_eq!(hitmarks.value.marks[0].value, -7);
        assert_eq!(hitmarks.value.marks[0].kind, 99);
        assert_eq!(hitmarks.value.marks[0].cycle, 111);
        assert_eq!(hitmarks.stamp, stamp);
        assert!(view.side_tabs().unwrap().value.is_empty());
        assert_eq!(view.active_side_tab().unwrap().value, 3);
        let world = view.world().expect("posted world build");
        assert_eq!(world.value.map_base_x, 3200);
        assert_eq!(world.value.map_base_z, 3400);
        assert_eq!(world.value.level, 1);
        assert!(world.value.members);
        assert_eq!(world.stamp, stamp);
        assert_eq!(view.inventory_capacity().unwrap().value, 28);

        snapshot.base = None;
        snapshot.inventory_size = 0;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.world().is_none());
        assert!(view.inventory_capacity().is_none());
        let gens = snapshot.gens();
        snapshot.reset_session(gens);
        assert!(!snapshot.players_available);
        assert!(!snapshot.side_tabs_available);
        assert!(snapshot.hitmarks.is_none());
        assert!(snapshot.projectiles().is_empty());
        let reset_view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(reset_view.local_player().is_none());
        assert!(reset_view.players().is_none());
        assert!(reset_view.projectiles().is_none());
        assert!(reset_view.hitmarks().is_none());
        assert!(reset_view.side_tabs().is_none());
        assert!(reset_view.active_side_tab().is_none());
    }

    #[test]
    fn chat_lines_since_skips_old_sequences_without_copying() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let snapshot = GameSnapshot {
            ingame: true,
            chat_lines: vec![
                super::super::ChatLineView {
                    type_: 0,
                    username: None,
                    text: "new".into(),
                    sequence: 4,
                },
                super::super::ChatLineView {
                    type_: 0,
                    username: None,
                    text: "old".into(),
                    sequence: 2,
                },
            ],
            ..GameSnapshot::default()
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        let newer: Vec<_> = view
            .chat_lines(2)
            .unwrap()
            .value
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(newer, ["new"]);
        assert_eq!(view.chat_lines(4).unwrap().value.iter().count(), 0);
    }

    #[test]
    fn reach_planes_are_unavailable_during_scene_rebuild_or_disconnect() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let mut snapshot = GameSnapshot {
            ingame: true,
            ..GameSnapshot::default()
        };
        let mut reach = crate::query::ReachQueryView::unavailable();
        reach.available = true;
        assert!(SnapshotView::new(Some(&snapshot), stamp)
            .with_reach(Some(&reach))
            .reach()
            .is_none());
        snapshot.scene_state = 2;
        let view = SnapshotView::new(Some(&snapshot), stamp).with_reach(Some(&reach));
        let observed = view.reach().unwrap();
        assert!(std::ptr::eq(observed.value, &reach));
        assert_eq!(observed.stamp, stamp);
        snapshot.ingame = false;
        assert!(SnapshotView::new(Some(&snapshot), stamp)
            .with_reach(Some(&reach))
            .reach()
            .is_none());
    }

    #[test]
    fn journal_title_comes_from_bound_component_not_widget_position() {
        use super::super::{WidgetKind, WidgetRoot, WidgetView};
        let widget = |component_id, text: &str| WidgetView {
            kind: WidgetKind::Widget,
            component_id,
            layer_id: 0,
            parent_id: 0,
            root_component_id: 10,
            root: WidgetRoot::Main,
            type_: 4,
            button_type: 0,
            client_code: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scroll_height: 0,
            scroll_position: 0,
            hidden: false,
            text: Some(text.into()),
            alternate_text: None,
            button_text: None,
            target_verb: None,
            target_base: None,
            target_mask: 0,
            model_type: 0,
            model_id: 0,
            alternate_model_type: 0,
            alternate_model_id: 0,
            scripts: None,
            script_comparators: None,
            script_operands: None,
            varp_bindings: Vec::new(),
            colour: 0,
            actions: Vec::new(),
            items: Vec::new(),
        };
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 2,
        };
        let mut snapshot = GameSnapshot {
            ingame: true,
            ..GameSnapshot::default()
        };
        snapshot.modals.main = 10;
        let mut other_root = widget(21, "unrelated");
        other_root.root_component_id = 20;
        let mut hidden = widget(14, "hidden");
        hidden.hidden = true;
        snapshot.widgets = vec![
            widget(12, "First body line"),
            other_root,
            widget(11, "Selected title"),
            hidden,
            widget(13, "Second body line"),
        ];
        let view = SnapshotView::new(Some(&snapshot), stamp);
        let page = view.journal_widgets(10, 11).unwrap();
        assert_eq!(page.stamp, stamp);
        assert_eq!(page.value.title, "Selected title");
        assert_eq!(
            page.value.lines().collect::<Vec<_>>(),
            ["First body line", "Second body line"],
        );
        assert!(view.journal_widgets(10, 99).is_none());
        assert!(view.journal_widgets(20, 21).is_none());
        snapshot.widgets[2].hidden = true;
        assert!(SnapshotView::new(Some(&snapshot), stamp)
            .journal_widgets(10, 11)
            .is_none());
        snapshot.widgets[2].hidden = false;
        snapshot.ingame = false;
        assert!(SnapshotView::new(Some(&snapshot), stamp)
            .journal_widgets(10, 11)
            .is_none());
    }

    #[test]
    fn posted_player_names_use_the_native_account_identity() {
        let id = crate::snapshot::player_account_id;
        assert_eq!(id("livetest_0"), id("Livetest 0"));
        assert_eq!(id(" Alice_B "), id("Alice B"));
        assert_ne!(id("livetest_0"), id("Livetest 1"));
        assert_eq!(id(""), None);
        assert_eq!(id("___"), None);
    }

    #[test]
    fn world_members_is_the_host_profile_fact_not_the_account_flag() {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        // The account flag the server sent at login is true throughout.
        snapshot.seed_world(WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            members: true,
            ..Default::default()
        });
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.world().unwrap().value.members);
        assert_eq!(view.world_members(), Truth::Unknown);
        assert_eq!(
            view.with_world_members(Truth::False).world_members(),
            Truth::False
        );
        assert_eq!(
            view.with_world_members(Truth::True).world_members(),
            Truth::True
        );
        // No frame at all still reports the attached profile fact.
        let empty = SnapshotView::new(None, stamp);
        assert_eq!(empty.world_members(), Truth::Unknown);
        assert_eq!(
            empty.with_world_members(Truth::True).world_members(),
            Truth::True
        );
    }
}

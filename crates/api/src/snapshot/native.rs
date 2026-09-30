//! Borrowed native observations with readiness attached to their evidence stamp.
use super::{
    ActorTargetView, ChatLineView, ChatOptionView, GameSnapshot, GroundItemView, ItemView, LocView,
    NpcView, QuestStatusView, StatView, VarpView,
};
use crate::quest_progress::EvidenceStamp;

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
}

impl<'a> SnapshotView<'a> {
    /// Attach the host's run/session stamp to its current observation. `None`
    /// represents a frame without a snapshot, not an observed empty world.
    pub fn new(snapshot: Option<&'a GameSnapshot>, stamp: EvidenceStamp) -> Self {
        Self {
            snapshot,
            stamp,
            reach: None,
        }
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

    /// Attach the cached host reach planes belonging to this frame.
    pub fn with_reach(mut self, reach: Option<&'a crate::query::ReachQueryView>) -> Self {
        self.reach = reach;
        self
    }

    pub fn reach(&self) -> Option<Observed<&crate::query::ReachQueryView>> {
        let snapshot = self.snapshot?;
        let reach = self.reach?;
        (snapshot.ingame() && snapshot.scene_state() == 2 && reach.available).then_some(Observed {
            value: reach,
            stamp: self.stamp,
        })
    }

    pub fn inventory(&self) -> Option<Observed<&[ItemView]>> {
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

    pub fn quest_statuses(&self) -> Option<Observed<&[QuestStatusView]>> {
        let snapshot = self.snapshot?;
        (snapshot.ingame() && snapshot.quest_statuses_available()).then(|| Observed {
            value: snapshot.quest_statuses(),
            stamp: self.stamp,
        })
    }

    pub fn widgets(&self) -> Option<Observed<&[super::WidgetView]>> {
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
}

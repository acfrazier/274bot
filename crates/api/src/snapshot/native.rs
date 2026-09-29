//! Borrowed native observations with readiness attached to their evidence stamp.
use super::{GameSnapshot, ItemView, QuestStatusView};
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
        }
        snapshot.ingame = true;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.inventory().is_none());
        assert!(view.bank().is_none());
        assert!(view.quest_statuses().is_none());
        assert_eq!(view.main_modal().unwrap().value.root, -1);
        snapshot.inventory_size = 28;
        snapshot.bank_loaded = true;
        snapshot.quest_statuses_available = true;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.inventory().unwrap().value.is_empty());
        assert!(view.bank().unwrap().value.is_empty());
        assert!(view.quest_statuses().unwrap().value.is_empty());
        assert_eq!(view.inventory().unwrap().stamp, stamp);
        // A disconnected frame cannot lend stale cached observations.
        snapshot.ingame = false;
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.inventory().is_none());
        assert!(view.bank().is_none());
        assert!(view.quest_statuses().is_none());
        assert!(view.main_modal().is_none());
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

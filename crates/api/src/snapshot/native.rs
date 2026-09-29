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

/// A host frame borrow; readiness comes from the snapshot, never caller flags.
#[derive(Clone, Copy)]
pub struct SnapshotView<'a> {
    snapshot: Option<&'a GameSnapshot>,
    stamp: EvidenceStamp,
}

impl<'a> SnapshotView<'a> {
    /// Attach the host's run/session stamp to its current observation. `None`
    /// represents a frame without a snapshot, not an observed empty world.
    pub fn new(snapshot: Option<&'a GameSnapshot>, stamp: EvidenceStamp) -> Self {
        Self { snapshot, stamp }
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
}

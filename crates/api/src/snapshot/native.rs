//! Borrowed native field views. M-297 supplies session readiness and construction.
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

/// A host-created frame borrow; no public constructor can invent readiness.
#[derive(Clone, Copy)]
pub struct SnapshotView<'a> {
    _snapshot: &'a GameSnapshot,
}

impl SnapshotView<'_> {
    /// Body owned by M-297; no native session observation has been attached.
    pub fn inventory(&self) -> Option<Observed<&[ItemView]>> {
        None
    }
    /// Body owned by M-297; open is not loaded in this session.
    pub fn bank(&self) -> Option<Observed<&[ItemView]>> {
        None
    }
    /// Body owned by M-297; no native session observation has been attached.
    pub fn quest_statuses(&self) -> Option<Observed<&[QuestStatusView]>> {
        None
    }
    /// Body owned by M-297; absence is not an observed closed modal.
    pub fn main_modal(&self) -> Option<Observed<JournalModalView<'_>>> {
        None
    }
}

//! Typed quest family contract. Catalog construction and journal programs belong
//! to M-296; the legacy query remains authoritative until its atomic cutover.

use crate::quest_progress::{EvidenceStamp, JournalRead, ProgressError, QuestProgress};
use crate::selected::{EntityId, FactKey, Knowledge, Requirement, SignalRange, SourceSpan};
use crate::selected::{FactError, QuestGate, Truth};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Immutable prepared family. Private indexes/programs and inherent queries are
/// installed by M-296 with the selected family loader, never by a tick caller.
pub struct QuestCatalog {
    _private: (),
}

impl QuestCatalog {
    /// Body owned by M-296; typed family assets are not installed yet.
    pub fn quest(&self, _id: &str) -> Result<&QuestFacts, FactError> {
        Err(FactError::FamilyUnavailable(
            crate::selected::QUESTS_FAMILY.clone(),
        ))
    }

    /// Body owned by M-296; no transmission is inferred from an absent program.
    pub fn transmission(&self, _signal: &FactKey) -> Transmission {
        Transmission::Unknown
    }

    /// Body owned by M-296; an absent resolution program cannot match a journal.
    pub fn resolve(
        &self,
        _read: &JournalRead,
        _role: Option<&FactKey>,
    ) -> Result<QuestProgress, ProgressError> {
        Err(ProgressError::UnknownTemplate)
    }

    /// Body owned by M-296; missing catalog validation never authorizes a gate.
    pub fn test_gate(
        &self,
        _gate: &QuestGate,
        _progress: &QuestProgress,
        _required_after: EvidenceStamp,
    ) -> Truth {
        Truth::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestKind {
    Quest,
    Miniquest,
    Stub,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transmission {
    PostedByContent,
    NotTransmitted,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartLocation {
    pub target: EntityId,
    pub tile: WorldTile,
    pub op: u8,
    pub source: SourceSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageFacts {
    pub id: FactKey,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub role: Option<FactKey>,
    pub terminal: bool,
    pub signals: Arc<[SignalRange]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestFacts {
    pub id: FactKey,
    pub display: Arc<str>,
    pub kind: QuestKind,
    pub component: Knowledge<Option<i32>>,
    pub starts: Knowledge<Arc<[StartLocation]>>,
    pub requirements: Knowledge<Arc<[Requirement]>>,
    pub progress_binding: Knowledge<FactKey>,
    pub stages: Knowledge<Arc<[StageFacts]>>,
}

//! Identity catalog: roster rows only. Stage resolution belongs to the Path.

use crate::game_data::{QuestIdentityFacts, QuestIdentityRow};
use crate::quest_progress::{EvidenceStamp, JournalRead, ProgressError, QuestProgress};
use crate::selected::{EntityId, FactKey, Knowledge, Requirement, SignalRange, SourceSpan};
use crate::selected::{FactError, QuestGate, Truth};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Immutable prepared family. Identity rows come from selected `quest_identity`.
pub struct QuestCatalog {
    rows: Arc<[QuestFacts]>,
}

impl QuestCatalog {
    pub fn from_identity(facts: Option<&QuestIdentityFacts>) -> Result<Self, FactError> {
        let Some(facts) = facts else {
            return Err(FactError::FamilyUnavailable(
                crate::selected::QUESTS_FAMILY.clone(),
            ));
        };
        let rows = facts
            .rows
            .iter()
            .map(facts_from_row)
            .collect::<Vec<_>>()
            .into();
        Ok(Self { rows })
    }

    pub fn empty() -> Self {
        Self {
            rows: Arc::from([]),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn quest(&self, id: &str) -> Result<&QuestFacts, FactError> {
        self.rows
            .iter()
            .find(|row| row.id.0.as_ref() == id)
            .ok_or_else(|| FactError::UnknownKey(FactKey::new(id)))
    }

    pub fn by_display(&self, name: &str) -> Option<&QuestFacts> {
        self.rows.iter().find(|row| row.display.as_ref() == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &QuestFacts> {
        self.rows.iter()
    }

    /// Transmission is not inferred from identity. Live varp table membership
    /// is the Window authority (design §14 N1).
    pub fn transmission(&self, _signal: &FactKey) -> Transmission {
        Transmission::Unknown
    }

    pub fn resolve(
        &self,
        _read: &JournalRead,
        _role: Option<&FactKey>,
    ) -> Result<QuestProgress, ProgressError> {
        Err(ProgressError::UnknownTemplate)
    }

    pub fn test_gate(
        &self,
        _gate: &QuestGate,
        _progress: &QuestProgress,
        _required_after: EvidenceStamp,
    ) -> Truth {
        Truth::Unknown
    }
}

fn facts_from_row(row: &QuestIdentityRow) -> QuestFacts {
    let kind = match row.kind.as_str() {
        "miniquest" => QuestKind::Miniquest,
        "stub" => QuestKind::Stub,
        _ => QuestKind::Quest,
    };
    QuestFacts {
        id: FactKey::new(&row.id),
        display: Arc::from(row.display.as_str()),
        kind,
        component: Knowledge::Known(Some(0)),
        component_name: Arc::from(row.component.as_str()),
        members: row.members,
        quest_points: row.quest_points,
        journal_title: row.journal_title.as_deref().map(Arc::from),
        journal_script: row.journal_script.as_deref().map(Arc::from),
        starts: Knowledge::Unknown(crate::selected::Gap {
            code: Arc::from("identity-only"),
            sources: Arc::from([]),
        }),
        requirements: Knowledge::Unknown(crate::selected::Gap {
            code: Arc::from("identity-only"),
            sources: Arc::from([]),
        }),
        progress_binding: Knowledge::Known(FactKey::new(&format!("journal:{}", row.id))),
        stages: Knowledge::Unknown(crate::selected::Gap {
            code: Arc::from("identity-only"),
            sources: Arc::from([]),
        }),
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
    #[serde(default)]
    pub component_name: Arc<str>,
    #[serde(default)]
    pub members: bool,
    #[serde(default)]
    pub quest_points: i32,
    #[serde(default)]
    pub journal_title: Option<Arc<str>>,
    #[serde(default)]
    pub journal_script: Option<Arc<str>>,
    pub starts: Knowledge<Arc<[StartLocation]>>,
    pub requirements: Knowledge<Arc<[Requirement]>>,
    pub progress_binding: Knowledge<FactKey>,
    pub stages: Knowledge<Arc<[StageFacts]>>,
}

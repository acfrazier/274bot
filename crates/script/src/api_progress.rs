//! Shared host-side wire and output types for API quest progress.

use api::quest_progress::{EvidenceStamp, JournalRead, ProgressFlag};
use api::selected::{Knowledge, Truth};
use api::snapshot::QuestListStatus;
use serde::Serialize;
use std::sync::Arc;

/// One progress page published by the host for an API request.
///
/// `Reading` acknowledges admission. `Done`, `Refused` and `JournalDone`
/// settle the request and replace the previous page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressPage {
    Reading {
        token: u64,
    },
    Done {
        token: u64,
        row: Arc<QuestProgressRow>,
    },
    Refused {
        token: u64,
        reason: Arc<str>,
    },
    /// A journal-only read. `None` is a missing quest or an unclickable row.
    JournalDone {
        token: u64,
        result: Option<Arc<JournalEvidence>>,
    },
}

impl ProgressPage {
    pub const fn token(&self) -> u64 {
        match self {
            Self::Reading { token }
            | Self::Done { token, .. }
            | Self::Refused { token, .. }
            | Self::JournalDone { token, .. } => *token,
        }
    }

    /// FlatBuffer page kind: reading, done, refused, journal done.
    pub const fn kind(&self) -> u8 {
        match self {
            Self::Reading { .. } => 1,
            Self::Done { .. } => 2,
            Self::Refused { .. } => 3,
            Self::JournalDone { .. } => 4,
        }
    }
}

/// The resolved API projection of one quest's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestProgressRow {
    pub quest: Arc<str>,
    pub display: Arc<str>,
    pub colour: QuestListStatus,
    pub stage: Knowledge<Arc<str>>,
    pub rule: Knowledge<Arc<str>>,
    pub complete: Truth,
    pub flags: Arc<[ProgressFlag]>,
    pub evidence: EvidenceStamp,
    pub journal_read: bool,
    pub binding: Arc<str>,
    pub role: Option<Arc<str>>,
}
/// Identity and receipt fields retained for one compatibility journal read.
///
/// The public `Quests.journal` result only exposes `lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEvidence {
    pub quest: Arc<str>,
    pub root: i32,
    pub lines: Arc<[Arc<str>]>,
    pub colour: Option<i32>,
    pub acquired: EvidenceStamp,
    pub closed: EvidenceStamp,
}

impl JournalEvidence {
    pub fn from_read(read: &JournalRead) -> Self {
        Self {
            quest: Arc::clone(&read.quest.0),
            root: read.root,
            lines: Arc::clone(&read.lines),
            colour: read.colour,
            acquired: read.acquired,
            closed: read.closed,
        }
    }
}

fn evidence_value(evidence: EvidenceStamp) -> serde_json::Value {
    serde_json::json!({
        "slot": evidence.run.slot,
        "run": evidence.run.run,
        "session": evidence.run.session,
        "tick": evidence.tick,
        "sequence": evidence.sequence,
    })
}

fn journal_value(journal: &JournalEvidence) -> serde_json::Value {
    let lines = journal
        .lines
        .iter()
        .map(|line| line.as_ref())
        .collect::<Vec<_>>();
    serde_json::json!({
        "quest": journal.quest.as_ref(),
        "root": journal.root,
        "lines": lines,
        "colour": journal.colour,
        "acquired": evidence_value(journal.acquired),
        "closed": evidence_value(journal.closed),
    })
}

/// One released quest Path exposed to API v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestPathRow {
    pub id: Arc<str>,
    pub display: Arc<str>,
    pub journal: bool,
    pub draft: bool,
    pub stages: Arc<[Arc<str>]>,
}

impl From<ProgressPage> for serde_json::Value {
    fn from(page: ProgressPage) -> Self {
        match page {
            // Reading is only an internal host acknowledgment, never a public
            // terminal result. Null prevents it from masquerading as one.
            ProgressPage::Reading { .. } => Self::Null,
            ProgressPage::Done { token, row } => serde_json::json!({
                "end": "done",
                "token": token,
                "row": progress_row_value(&row),
            }),
            ProgressPage::Refused { token, reason } => serde_json::json!({
                "end": "refused",
                "token": token,
                "reason": reason,
            }),
            ProgressPage::JournalDone { token, result } => serde_json::json!({
                "end": "done",
                "token": token,
                "journal": result.as_deref().map(journal_value),
            }),
        }
    }
}

fn progress_row_value(row: &QuestProgressRow) -> serde_json::Value {
    let flags = row
        .flags
        .iter()
        .map(|flag| {
            serde_json::json!({
                "flag": flag.flag.0.as_ref(),
                "truth": truth_name(flag.truth),
                "count": flag.count,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "quest": row.quest.as_ref(),
        "display": row.display.as_ref(),
        "colour": row.colour.as_str(),
        "stage": knowledge_value(&row.stage),
        "complete": truth_name(row.complete),
        "rule": knowledge_value(&row.rule),
        "flags": flags,
        "evidence": {
            "run": row.evidence.run.run,
            "session": row.evidence.run.session,
            "tick": row.evidence.tick,
            "sequence": row.evidence.sequence,
        },
        "journal_read": row.journal_read,
        "binding": row.binding.as_ref(),
        "role": row.role.as_deref(),
    })
}

fn knowledge_value(knowledge: &Knowledge<Arc<str>>) -> serde_json::Value {
    match knowledge {
        Knowledge::Known(value) => serde_json::json!({
            "state": "known",
            "value": value.as_ref(),
        }),
        Knowledge::Partial { known, gaps } if gaps.is_empty() => serde_json::json!({
            "state": "known",
            "value": known.as_ref(),
        }),
        Knowledge::Partial { gaps, .. } => serde_json::json!({
            "state": "unknown",
            "gap": gaps[0].code.as_ref(),
        }),
        Knowledge::Unknown(gap) => serde_json::json!({
            "state": "unknown",
            "gap": gap.code.as_ref(),
        }),
    }
}

const fn truth_name(truth: Truth) -> &'static str {
    match truth {
        Truth::True => "true",
        Truth::False => "false",
        Truth::Unknown => "unknown",
    }
}

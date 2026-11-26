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
        "stage": crate::quest_journal::dragon_stage::journal_stage(&journal.quest, &journal.lines),
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

/// Legal coarse quest-tab status strings shared by JS v2 and Luau.
pub const QUEST_STATUS_VALUES: [&str; 4] = ["notStarted", "inProgress", "complete", "unknown"];

/// One resolved quest-tab row match from [`resolve_quest_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestStatusHit {
    pub status: String,
    pub as_of_sequence: u64,
}

/// Closed refusal vocabulary for quest-tab status reads (ADR 0023 item 24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestStatusError {
    InvalidArgs,
    SnapshotUnavailable,
    QuestTabUnbound,
    NotOnTab,
}

impl QuestStatusError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid-args",
            Self::SnapshotUnavailable => "snapshot-unavailable",
            Self::QuestTabUnbound => "quest-tab-unbound",
            Self::NotOnTab => "not-on-tab",
        }
    }
}

/// The caller's observed quest-tab state for [`resolve_quest_status`].
pub enum QuestTabState<I> {
    NoSnapshot,
    Unbound,
    Bound { rows: I, sequence: Option<u64> },
}

/// A-Z fold only. Not Unicode lowercasing.
pub fn fold_ascii(text: &str) -> String {
    text.chars()
        .map(|c| {
            let n = c as u32;
            if (97..=122).contains(&n) {
                char::from_u32(n - 32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// ECMAScript `String.prototype.trim`: WhiteSpace + LineTerminator, including
/// U+FEFF, excluding U+0085 (NEL).
pub fn js_trim(text: &str) -> &str {
    fn is_es_trim(c: char) -> bool {
        matches!(
            c,
            '\u{0009}'
                | '\u{000A}'
                | '\u{000B}'
                | '\u{000C}'
                | '\u{000D}'
                | '\u{0020}'
                | '\u{00A0}'
                | '\u{1680}'
                | '\u{2000}'
                ..='\u{200A}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}'
                    | '\u{FEFF}'
        )
    }
    text.trim_matches(is_es_trim)
}

/// Whether one quest-tab row has a legal status and matches `wanted_folded`.
pub fn quest_row_matches(row_name: &str, row_status: &str, wanted_folded: &str) -> bool {
    QUEST_STATUS_VALUES.contains(&row_status)
        && js_trim(row_name).eq_ignore_ascii_case(wanted_folded)
}

/// Shared quest-tab status match for both JS v2 and Luau (ADR 0023 item 24).
pub fn resolve_quest_status<'a, I>(
    name: &str,
    tab: QuestTabState<I>,
) -> Result<QuestStatusHit, QuestStatusError>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let wanted = fold_ascii(js_trim(name));
    if wanted.is_empty() {
        return Err(QuestStatusError::InvalidArgs);
    }
    match tab {
        QuestTabState::NoSnapshot => Err(QuestStatusError::SnapshotUnavailable),
        QuestTabState::Unbound => Err(QuestStatusError::QuestTabUnbound),
        QuestTabState::Bound { rows, sequence } => {
            let Some(as_of_sequence) = sequence else {
                return Err(QuestStatusError::SnapshotUnavailable);
            };
            match rows
                .into_iter()
                .find(|(row_name, row_status)| quest_row_matches(row_name, row_status, &wanted))
            {
                Some((_, status)) => Ok(QuestStatusHit {
                    status: status.to_owned(),
                    as_of_sequence,
                }),
                None => Err(QuestStatusError::NotOnTab),
            }
        }
    }
}

/// Resolve one quest-tab status directly from lent snapshot rows.
pub fn resolve_quest_status_inputs(
    name: &str,
    rows: Option<&[crate::native::snapshot_source::QuestStatusInput<'_>]>,
    tick: u64,
) -> Result<QuestStatusHit, QuestStatusError> {
    match rows {
        None => resolve_quest_status(
            name,
            QuestTabState::<std::iter::Empty<(&str, &str)>>::Unbound,
        ),
        Some(rows) => resolve_quest_status(
            name,
            QuestTabState::Bound {
                rows: rows.iter().map(|row| (row.name, row.status)),
                sequence: Some(tick),
            },
        ),
    }
}

#[cfg(test)]
mod quest_status_tests {
    use super::*;
    use crate::native::snapshot_source::QuestStatusInput;

    #[test]
    fn resolve_quest_status_covers_every_tab_state_and_error() {
        let empty = std::iter::empty::<(&str, &str)>();
        assert_eq!(
            resolve_quest_status(
                "   \u{FEFF} ",
                QuestTabState::NoSnapshot::<std::iter::Empty<_>>
            ),
            Err(QuestStatusError::InvalidArgs)
        );
        assert_eq!(
            resolve_quest_status(
                "Cook's Assistant",
                QuestTabState::NoSnapshot::<std::iter::Empty<_>>
            ),
            Err(QuestStatusError::SnapshotUnavailable)
        );
        assert_eq!(
            resolve_quest_status(
                "Cook's Assistant",
                QuestTabState::Unbound::<std::iter::Empty<_>>
            ),
            Err(QuestStatusError::QuestTabUnbound)
        );
        assert_eq!(
            resolve_quest_status(
                "Cook's Assistant",
                QuestTabState::Bound {
                    rows: empty,
                    sequence: None,
                }
            ),
            Err(QuestStatusError::SnapshotUnavailable)
        );
        let rows = [
            ("Cook's Assistant", "bogus"),
            ("  cook's assistant  ", "inProgress"),
            ("Cook's Assistant", "complete"),
        ];
        assert_eq!(
            resolve_quest_status(
                "Cook's Assistant",
                QuestTabState::Bound {
                    rows,
                    sequence: Some(42),
                }
            ),
            Ok(QuestStatusHit {
                status: "inProgress".to_owned(),
                as_of_sequence: 42,
            })
        );
        assert_eq!(
            resolve_quest_status(
                "Missing Quest",
                QuestTabState::Bound {
                    rows,
                    sequence: Some(42),
                }
            ),
            Err(QuestStatusError::NotOnTab)
        );
    }

    #[test]
    fn resolve_quest_status_inputs_matches_lent_frame_contract() {
        assert_eq!(
            resolve_quest_status_inputs("Cook's Assistant", None, 7),
            Err(QuestStatusError::QuestTabUnbound)
        );
        assert_eq!(
            resolve_quest_status_inputs("Cook's Assistant", Some(&[]), 7),
            Err(QuestStatusError::NotOnTab)
        );
        let rows = [QuestStatusInput {
            name: "Cook's Assistant",
            status: "complete",
            component_id: Some(7333),
        }];
        assert_eq!(
            resolve_quest_status_inputs("  cook's assistant ", Some(&rows), 19),
            Ok(QuestStatusHit {
                status: "complete".to_owned(),
                as_of_sequence: 19,
            })
        );
    }
}

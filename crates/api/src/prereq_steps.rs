//! Authored prerequisite-step facts for the navigation runner.
//!
//! The catalog contains authored semantic rows. A row proves a gate only when
//! every value established by the row lies inside that gate's stage window.

use crate::selected::{FactKey, InclusiveRange, QuestGate, StageWindow, Truth};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
mod cache;
mod wire;

pub(crate) use cache::{cached, prepare};

use std::sync::Arc;

/// API-owned transport identity, deliberately independent of nav policy types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossingKind {
    Door,
    Ladder,
    Stairs,
    Boat,
    Teleport,
    AgilityShortcut,
    Glider,
    SpiritTree,
    Npc,
    EssenceExit,
}

/// Stable identity of one directed content crossing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossingId {
    pub loc_id: i32,
    pub option: u8,
    /// Loc interaction anchor, not the tile on which the player stands.
    pub at: WorldTile,
    /// Exact content-proven interaction stand for this direction.
    pub takeoff: WorldTile,
    pub to: WorldTile,
    pub kind: CrossingKind,
}

/// A gate diagnosis in route order, including the exact edge that refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionableGate {
    pub gate: QuestGate,
    pub truth: Truth,
    pub leg: u16,
    pub crossing: CrossingId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrereqCrossing {
    pub crossing: CrossingId,
    pub pass: PassEvidence,
    pub delay: CrossingDelay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassEvidence {
    pub message: Arc<str>,
    pub landing: WorldTile,
    pub proves: InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossingDelay {
    pub ticks: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AfterRule {
    pub proves: InclusiveRange,
    pub after: InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub refuse: Arc<[RefusalPage]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalPage {
    pub page: Arc<str>,
    pub then: RefusalThen,
    pub proves: InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalThen {
    Talk,
    Precondition(QuestGate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrereqTarget {
    pub npc: i32,
    pub op: u8,
    pub anchor: WorldTile,
    pub leash: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrereqPrecondition {
    Gate { gate: QuestGate },
    OwnedAnywhere { item: FactKey, otherwise: OwnedElse },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedElse {
    Reclaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchOutcome {
    Advice,
    Start,
    Reclaimed,
    Satisfied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchExit {
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenerRequirement {
    FreeSlots { count: u8, otherwise: BranchExit },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkBranch {
    pub opener: Arc<str>,
    pub outcome: BranchOutcome,
    #[serde(default)]
    pub effect_page: Option<Arc<str>>,
    #[serde(default)]
    pub terminal: Option<Arc<str>>,
    pub proves: InclusiveRange,
    #[serde(default)]
    pub requires_at_opener: Option<OpenerRequirement>,
    #[serde(default)]
    pub exit: Option<BranchExit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatAfter {
    #[serde(default)]
    pub reclaimed: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Talk {
    pub strict: bool,
    pub prefer: Arc<[Arc<str>]>,
    pub line_rules: Arc<[LineRule]>,
    pub branches: Arc<[TalkBranch]>,
    #[serde(default)]
    pub repeat_after: Option<RepeatAfter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineRule {
    pub page: Arc<str>,
    pub choice: Arc<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMemory {
    Durable,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentSwitch {
    PrereqDialogue,
    QuestStart,
}

/// One authored (gate, step) row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrereqStep {
    pub id: Arc<str>,
    pub crossings: Arc<[PrereqCrossing]>,
    pub establishes: StageWindow,
    pub after: Arc<[AfterRule]>,
    pub probe: Probe,
    pub target: PrereqTarget,
    pub preconditions: Arc<[PrereqPrecondition]>,
    pub talk: Talk,
    pub memory: StepMemory,
    pub consent: ConsentSwitch,
}

impl PrereqStep {
    /// Return the unique matching entry and its stable index within this row.
    pub fn crossing(&self, crossing: &CrossingId) -> Option<(u8, &PrereqCrossing)> {
        let mut matches = self
            .crossings
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.crossing == *crossing);
        let (index, entry) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        u8::try_from(index).ok().map(|index| (index, entry))
    }

    /// Authored range that holds after a page establishes `possible`.
    pub fn after_for(&self, possible: InclusiveRange) -> Option<InclusiveRange> {
        let mut matches = self.after.iter().filter(|rule| rule.proves == possible);
        let after = matches.next()?.after;
        matches.next().is_none().then_some(after)
    }
}

/// A journal page that establishes one account signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrereqJournal {
    pub id: Arc<str>,
    /// Canonical quest whose page the native journal owner reads.
    pub quest: FactKey,
    pub establishes: StageWindow,
    pub after: Arc<[AfterRule]>,
    pub rules: Arc<[JournalRule]>,
}

/// Every normalized substring must occur in the same acquired journal page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRule {
    pub all: Arc<[Arc<str>]>,
    pub proves: InclusiveRange,
}

/// A prerequisite table violates a structural catalog invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrereqCatalogError {
    /// Row ordinals cannot represent this many rows.
    #[error("too many prerequisite rows")]
    TooManyRows,
    /// A row has no identity or has an unsupported crossing count.
    #[error("invalid prerequisite row identity or crossing count")]
    InvalidRowIdentity,
    /// Two entries in one row identify the same directed crossing.
    #[error("duplicate crossing identity in prerequisite row")]
    DuplicateCrossing,
    /// A crossing's proved range has reversed bounds.
    #[error("invalid prerequisite crossing facts")]
    InvalidCrossingFacts,
    /// An establishment or preservation range has reversed bounds.
    #[error("invalid prerequisite stage range")]
    InvalidStageRange,
    /// A journal row lacks a nonempty rule or an exact preservation rule.
    #[error("invalid prerequisite journal facts")]
    InvalidJournalFacts,
}

/// Immutable, verified table indexed by content row ordinal.
#[derive(Debug, Clone)]
pub struct PrereqCatalog {
    entries: Arc<PrereqEntries>,
}

#[derive(Debug)]
struct PrereqEntries {
    rows: Box<[PrereqStep]>,
    journals: Box<[PrereqJournal]>,
    establishable_windows: Arc<[StageWindow]>,
}

impl PrereqCatalog {
    /// Validate the row identities, directed crossings, and stage ranges.
    pub fn from_rows(rows: Vec<PrereqStep>) -> Result<Self, PrereqCatalogError> {
        Self::from_entries(rows, Vec::new())
    }

    /// Validate dialogue and journal evidence in one ordinal space.
    pub fn from_entries(
        rows: Vec<PrereqStep>,
        journals: Vec<PrereqJournal>,
    ) -> Result<Self, PrereqCatalogError> {
        if rows.len().saturating_add(journals.len()) > usize::from(u16::MAX) {
            return Err(PrereqCatalogError::TooManyRows);
        }
        for row in &rows {
            if row.id.is_empty()
                || row.crossings.is_empty()
                || row.crossings.len() > usize::from(u8::MAX) + 1
            {
                return Err(PrereqCatalogError::InvalidRowIdentity);
            }
            for (index, entry) in row.crossings.iter().enumerate() {
                if row.crossings[..index]
                    .iter()
                    .any(|previous| previous.crossing == entry.crossing)
                {
                    return Err(PrereqCatalogError::DuplicateCrossing);
                }
                if !valid_range(&entry.pass.proves) {
                    return Err(PrereqCatalogError::InvalidCrossingFacts);
                }
            }
            if !valid_range(&row.establishes.values)
                || row
                    .after
                    .iter()
                    .any(|rule| !valid_range(&rule.proves) || !valid_range(&rule.after))
            {
                return Err(PrereqCatalogError::InvalidStageRange);
            }
        }
        for journal in &journals {
            if journal.id.is_empty()
                || journal.quest.0.is_empty()
                || journal.rules.is_empty()
                || journal.rules.len() > usize::from(u16::MAX)
                || !valid_range(&journal.establishes.values)
                || journal.rules.iter().any(|rule| {
                    rule.all.is_empty()
                        || rule.all.iter().any(|text| text.trim().is_empty())
                        || !valid_range(&rule.proves)
                        || journal
                            .after
                            .iter()
                            .filter(|after| after.proves == rule.proves)
                            .count()
                            != 1
                })
                || journal
                    .after
                    .iter()
                    .any(|rule| !valid_range(&rule.proves) || !valid_range(&rule.after))
            {
                return Err(PrereqCatalogError::InvalidJournalFacts);
            }
        }
        let establishable_windows = rows
            .iter()
            .map(|row| row.establishes.clone())
            .chain(journals.iter().map(|journal| journal.establishes.clone()))
            .collect::<Vec<_>>()
            .into();
        Ok(Self {
            entries: Arc::new(PrereqEntries {
                rows: rows.into_boxed_slice(),
                journals: journals.into_boxed_slice(),
                establishable_windows,
            }),
        })
    }

    /// Match both the established window and the exact directed crossing.
    pub fn for_gate(&self, gate: &QuestGate, crossing: &CrossingId) -> Option<(u16, &PrereqStep)> {
        self.entries
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| {
                if proves(&row.establishes, gate) && row.crossing(crossing).is_some() {
                    Some((
                        u16::try_from(index).expect("catalog row count was checked"),
                        row,
                    ))
                } else {
                    None
                }
            })
    }

    /// Find a page that can establish this signal without a world interaction.
    pub fn for_journal(&self, gate: &QuestGate) -> Option<(u16, &PrereqJournal)> {
        self.entries
            .journals
            .iter()
            .enumerate()
            .find_map(|(index, journal)| {
                proves(&journal.establishes, gate).then(|| {
                    (
                        u16::try_from(self.entries.rows.len() + index)
                            .expect("catalog row count was checked"),
                        journal,
                    )
                })
            })
    }

    /// Iterate rows with their stable ordinal for refusal and page facts.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &PrereqStep)> {
        self.entries.rows.iter().enumerate().map(|(index, row)| {
            (
                u16::try_from(index).expect("catalog row count was checked"),
                row,
            )
        })
    }

    pub fn len(&self) -> usize {
        self.entries.rows.len() + self.entries.journals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.rows.is_empty() && self.entries.journals.is_empty()
    }

    /// Allocation-free clone of the precomputed establishment set.
    pub fn establishable_windows(&self) -> Arc<[StageWindow]> {
        Arc::clone(&self.entries.establishable_windows)
    }

    pub fn reference(&self, row: u16, entry: u8) -> Option<(&PrereqStep, &PrereqCrossing)> {
        let step = self.entries.rows.get(usize::from(row))?;
        let crossing = step.crossings.get(usize::from(entry))?;
        Some((step, crossing))
    }
}

fn valid_range(range: &InclusiveRange) -> bool {
    !matches!((range.min, range.max), (Some(min), Some(max)) if min > max)
}

/// True only when every stage established by the row satisfies this gate.
pub fn proves(established: &StageWindow, gate: &QuestGate) -> bool {
    let QuestGate::Window(required) = gate else {
        return false;
    };
    established.quest == required.quest
        && established.signal == required.signal
        && required.values.test(&established.values) == Truth::True
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::FactKey;

    fn window(quest: &str, signal: &str, min: i32, max: i32) -> StageWindow {
        StageWindow {
            quest: FactKey::new(quest),
            signal: FactKey::new(signal),
            values: InclusiveRange {
                min: Some(min),
                max: Some(max),
            },
        }
    }

    #[test]
    fn proof_requires_all_values_and_matching_signal() {
        let established = window("druidspirit", "druidspirit", 5, 5);
        assert!(proves(
            &established,
            &QuestGate::Window(window("druidspirit", "druidspirit", 1, i32::MAX))
        ));
        assert!(proves(
            &established,
            &QuestGate::Window(window("druidspirit", "druidspirit", 5, 5))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Window(window("druidspirit", "druidspirit", 6, i32::MAX))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Window(window("druidspirit", "druidspirit", 10, i32::MAX))
        ));
        assert!(proves(
            &window("priestperil", "priestperil", 61, 61),
            &QuestGate::Window(window("priestperil", "priestperil", 60, i32::MAX))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Window(window("other", "druidspirit", 1, i32::MAX))
        ));
        assert!(!proves(
            &window("druidspirit", "druidspirit", 1, 5),
            &QuestGate::Window(window("druidspirit", "druidspirit", 5, 5))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Window(window("druidspirit", "other", 1, i32::MAX))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Complete(FactKey::new("druidspirit"))
        ));
    }

    #[test]
    fn priest_journal_range_opens_lower_ladders_but_not_the_holy_barrier() {
        let established = window("priestperil", "priestperil", 9, 61);
        for minimum in [5, 8] {
            assert!(proves(
                &established,
                &QuestGate::Window(window("priestperil", "priestperil", minimum, 61))
            ));
        }
        assert!(!proves(
            &established,
            &QuestGate::Window(window("priestperil", "priestperil", 61, 61))
        ));
        assert!(!proves(
            &established,
            &QuestGate::Window(window("druidspirit", "priestperil", 5, 61))
        ));
    }

    #[test]
    fn crossing_identity_includes_direction_destination_and_kind() {
        let crossing = CrossingId {
            loc_id: 3507,
            option: 1,
            at: WorldTile {
                x: 3443,
                z: 3458,
                level: 0,
            },
            takeoff: WorldTile {
                x: 3443,
                z: 3458,
                level: 0,
            },
            to: WorldTile {
                x: 3443,
                z: 3457,
                level: 0,
            },
            kind: CrossingKind::Door,
        };
        let reversed = CrossingId {
            takeoff: crossing.to,
            to: crossing.at,
            ..crossing
        };
        assert_ne!(crossing, reversed);
        let other_kind = CrossingId {
            kind: CrossingKind::Stairs,
            ..crossing
        };
        assert_ne!(crossing, other_kind);
        let other_stand = CrossingId {
            takeoff: WorldTile {
                z: crossing.takeoff.z + 1,
                ..crossing.takeoff
            },
            ..crossing
        };
        assert_ne!(crossing, other_stand);
        assert_eq!(crossing.at, reversed.at);
    }
}

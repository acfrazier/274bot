//! Authored prerequisite-step facts for the navigation runner.
//!
//! The catalog contains only source-pinned rows. A row proves a gate only when
//! every value established by the row lies inside that gate's stage window.

use crate::selected::{FactKey, InclusiveRange, QuestGate, StageWindow, Truth};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
use std::fmt;
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

/// Content citation serialized as the authored `file:first-last` string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrereqSource(pub Arc<str>);

impl fmt::Display for PrereqSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrereqCrossing {
    pub crossing: CrossingId,
    pub pass: PassEvidence,
    pub delay: CrossingDelay,
    pub source: PrereqSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassEvidence {
    pub message: Arc<str>,
    pub landing: WorldTile,
    pub proves: InclusiveRange,
    pub source: PrereqSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossingDelay {
    pub ticks: u16,
    #[serde(default)]
    pub source: Option<PrereqSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AfterRule {
    pub proves: InclusiveRange,
    pub after: InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedWrite {
    pub write: PrereqSource,
    pub sets: i32,
    pub guard: PrereqSource,
    pub guard_text: Arc<str>,
    pub called_from: Arc<str>,
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
    pub source: PrereqSource,
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
    pub source: PrereqSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrereqPrecondition {
    Gate {
        gate: QuestGate,
        source: PrereqSource,
    },
    OwnedAnywhere {
        item: FactKey,
        otherwise: OwnedElse,
        source: PrereqSource,
    },
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
    pub source: PrereqSource,
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
    pub source: PrereqSource,
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
    pub reviewed_writes: Arc<[ReviewedWrite]>,
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

    /// Range that the preservation proof says holds after a page establishes `possible`.
    pub fn after_for(&self, possible: InclusiveRange) -> Option<InclusiveRange> {
        let mut matches = self.after.iter().filter(|rule| rule.proves == possible);
        let after = matches.next()?.after;
        matches.next().is_none().then_some(after)
    }

    /// Content identity source for runner diagnostics is the matched crossing's citation.
    pub fn source(&self, crossing: &CrossingId) -> Option<&PrereqSource> {
        self.crossing(crossing).map(|(_, entry)| &entry.source)
    }
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
}

/// Immutable, verified table indexed by content row ordinal.
#[derive(Debug, Clone)]
pub struct PrereqCatalog {
    rows: Arc<[PrereqStep]>,
    establishable_windows: Arc<[StageWindow]>,
}

impl PrereqCatalog {
    /// Validate the row identities, directed crossings, and stage ranges.
    pub fn from_rows(rows: Vec<PrereqStep>) -> Result<Self, PrereqCatalogError> {
        if rows.len() > usize::from(u16::MAX) {
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
        let establishable_windows = rows
            .iter()
            .map(|row| row.establishes.clone())
            .collect::<Vec<_>>()
            .into();
        Ok(Self {
            rows: rows.into(),
            establishable_windows,
        })
    }

    /// Match both the established window and the exact directed crossing.
    pub fn for_gate(&self, gate: &QuestGate, crossing: &CrossingId) -> Option<(u16, &PrereqStep)> {
        self.rows.iter().enumerate().find_map(|(index, row)| {
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

    /// Iterate rows with their stable ordinal for refusal and page facts.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &PrereqStep)> {
        self.rows.iter().enumerate().map(|(index, row)| {
            (
                u16::try_from(index).expect("catalog row count was checked"),
                row,
            )
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Allocation-free clone of the precomputed establishment set.
    pub fn establishable_windows(&self) -> Arc<[StageWindow]> {
        Arc::clone(&self.establishable_windows)
    }

    pub fn reference(&self, row: u16, entry: u8) -> Option<(&PrereqStep, &PrereqCrossing)> {
        let step = self.rows.get(usize::from(row))?;
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

//! Quest-stage gates on transport edges: typed [`QuestGate`]s (a completed
//! quest or a closed [`StageWindow`](api::selected::StageWindow) over a
//! selected progress signal) bound to the selected quest family a pack was
//! baked against, and the resolved progress evidence a search tests them with.
//!
//! Nav never decides a gate from snapshot varps, a hidden stage number or a
//! journal it reads itself: only an [`EvidenceProvider`] prepared from the
//! same quest family answers it, under the caller's causal freshness floor.
//! `True` alone authorizes an edge. `False` (every possible value lies
//! outside the window) and `Unknown` (no evidence, stale evidence, evidence
//! from another family, values that straddle a bound) never do.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{QuestGate, Truth};

use crate::transport::TransportGraph;

/// The selected quest family a pack's gates were baked against: the family
/// artifact digest (`manifest.families.quests.sha256`) and its extractor
/// schema. Never the final manifest hash, which is computed after the bake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QuestFamilyId {
    pub quest_facts_sha256: [u8; 32],
    pub quest_extractor_schema: u16,
}

impl fmt::Display for QuestFamilyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (extractor schema {})",
            crate::pack::sha256_hex(&self.quest_facts_sha256),
            self.quest_extractor_schema
        )
    }
}

/// A quest family other than the one a pack's gates were baked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestFamilyMismatch {
    /// The family the pack records; `None` when its bake consumed none.
    pub pack: Option<QuestFamilyId>,
    /// The family the caller's manifest or evidence names.
    pub expected: QuestFamilyId,
}

impl fmt::Display for QuestFamilyMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pack {
            Some(pack) => write!(
                f,
                "navigation pack quest gates were baked against quest family {pack}, not {}; rebake the pack",
                self.expected
            ),
            None => write!(
                f,
                "navigation pack was baked without a quest family, not against {}; rebake the pack",
                self.expected
            ),
        }
    }
}

impl std::error::Error for QuestFamilyMismatch {}

impl TransportGraph {
    /// Runtime admission: the pack must have been baked against exactly this
    /// quest family digest and extractor schema. A pack that consumed no
    /// family is refused too; nothing is assumed about gates it never saw.
    pub fn admit_quest_family(&self, expected: &QuestFamilyId) -> Result<(), QuestFamilyMismatch> {
        if self.quest_family.as_ref() == Some(expected) {
            Ok(())
        } else {
            Err(QuestFamilyMismatch {
                pack: self.quest_family,
                expected: *expected,
            })
        }
    }
}

/// Why a gate list cannot gate an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestGateError {
    /// No gates: an ungated edge carries `None`, never an empty set.
    Empty,
    /// A quest or signal key is empty.
    EmptyKey,
    /// The window's lower bound exceeds its upper bound: no value is inside.
    EmptyWindow,
    /// The window bounds neither side, so it gates nothing.
    Unbounded,
}

impl fmt::Display for QuestGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "quest gate list is empty",
            Self::EmptyKey => "quest gate names an empty quest or signal key",
            Self::EmptyWindow => "stage window lower bound exceeds its upper bound",
            Self::Unbounded => "stage window bounds neither side",
        })
    }
}

impl std::error::Error for QuestGateError {}

/// An edge's quest-stage gates: a non-empty, canonical (sorted, duplicate
/// free) list bound to the quest family its keys come from. The edge is
/// usable only while every gate is `True`. Cloning shares one allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestGates(Arc<GateSet>);

#[derive(Debug, PartialEq, Eq)]
struct GateSet {
    family: QuestFamilyId,
    gates: Box<[QuestGate]>,
}

impl QuestGates {
    /// Validate and canonicalize `gates` for `family`.
    pub fn new(
        family: QuestFamilyId,
        gates: impl IntoIterator<Item = QuestGate>,
    ) -> Result<Self, QuestGateError> {
        let mut gates: Vec<QuestGate> = gates.into_iter().collect();
        for gate in &gates {
            validate(gate)?;
        }
        gates.sort_by(|a, b| gate_key(a).cmp(&gate_key(b)));
        gates.dedup();
        if gates.is_empty() {
            return Err(QuestGateError::Empty);
        }
        Ok(Self(Arc::new(GateSet {
            family,
            gates: gates.into_boxed_slice(),
        })))
    }

    /// The quest family whose keys the gates name.
    pub fn family(&self) -> &QuestFamilyId {
        &self.0.family
    }

    /// The gates, in canonical order.
    pub fn gates(&self) -> &[QuestGate] {
        &self.0.gates
    }

    /// `True` only when evidence prepared from the same quest family proves
    /// every gate; `False` when it disproves one; otherwise `Unknown`. No
    /// evidence, or evidence from another family, is `Unknown`.
    pub fn test(&self, evidence: Option<&QuestEvidence>) -> Truth {
        match evidence {
            Some(evidence) if evidence.family == self.0.family => {
                Truth::all(self.0.gates.iter().map(|gate| evidence.test(gate)))
            }
            _ => Truth::Unknown,
        }
    }
}

impl PartialOrd for QuestGates {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Canonical pack order: family, then the gates key by key.
impl Ord for QuestGates {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.family.cmp(&other.0.family).then_with(|| {
            self.0
                .gates
                .iter()
                .map(gate_key)
                .cmp(other.0.gates.iter().map(gate_key))
        })
    }
}

/// A gate's total-order key; equal keys are equal gates.
fn gate_key(gate: &QuestGate) -> (u8, &str, &str, Option<i32>, Option<i32>) {
    match gate {
        QuestGate::Complete(quest) => (0, &*quest.0, "", None, None),
        QuestGate::Window(window) => (
            1,
            &*window.quest.0,
            &*window.signal.0,
            window.values.min,
            window.values.max,
        ),
    }
}

fn validate(gate: &QuestGate) -> Result<(), QuestGateError> {
    match gate {
        QuestGate::Complete(quest) if quest.0.is_empty() => Err(QuestGateError::EmptyKey),
        QuestGate::Complete(_) => Ok(()),
        QuestGate::Window(window) => {
            if window.quest.0.is_empty() || window.signal.0.is_empty() {
                return Err(QuestGateError::EmptyKey);
            }
            match (window.values.min, window.values.max) {
                (None, None) => Err(QuestGateError::Unbounded),
                (Some(min), Some(max)) if min > max => Err(QuestGateError::EmptyWindow),
                _ => Ok(()),
            }
        }
    }
}

/// Resolved quest progress a search tests gates with: the caller's immutable
/// provider snapshot, the quest family it was prepared from and the causal
/// freshness floor every answer must meet. Nav only asks the provider; it
/// never reads a journal or a varp to answer a gate itself.
#[derive(Clone)]
pub struct QuestEvidence {
    provider: Arc<dyn EvidenceProvider>,
    family: QuestFamilyId,
    required_after: EvidenceStamp,
}

impl QuestEvidence {
    pub fn new(
        provider: Arc<dyn EvidenceProvider>,
        family: QuestFamilyId,
        required_after: EvidenceStamp,
    ) -> Self {
        Self {
            provider,
            family,
            required_after,
        }
    }

    /// The quest family the provider was prepared from.
    pub fn family(&self) -> &QuestFamilyId {
        &self.family
    }

    /// The causal floor every answer must meet.
    pub fn required_after(&self) -> EvidenceStamp {
        self.required_after
    }

    /// One gate under this evidence's freshness floor.
    pub fn test(&self, gate: &QuestGate) -> Truth {
        self.provider.test_gate(gate, self.required_after)
    }
}

/// Same provider snapshot, family and floor: a refreshed provider is new
/// evidence even when it would answer alike.
impl PartialEq for QuestEvidence {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.provider, &other.provider)
            && self.family == other.family
            && self.required_after == other.required_after
    }
}

impl Eq for QuestEvidence {}

impl fmt::Debug for QuestEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuestEvidence")
            .field("family", &self.family)
            .field("required_after", &self.required_after)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "quest_gates_tests.rs"]
pub(crate) mod tests;

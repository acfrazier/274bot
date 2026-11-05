//! Quest-stage gates on transport edges: typed [`QuestGate`]s (a completed
//! quest or a closed [`StageWindow`](api::selected::StageWindow) over a
//! selected progress signal) and the resolved progress evidence a search tests them with.
//!
//! Nav never decides a gate from snapshot varps, a hidden stage number or a
//! journal it reads itself: only an [`EvidenceProvider`] prepared from the
//! caller's selected content answers it, under the caller's causal freshness floor.
//! `True` alone authorizes an edge. `False` (every possible value lies
//! outside the window) and `Unknown` (no evidence, stale evidence, values
//! that straddle a bound) never do.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{QuestGate, Truth};

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

#[derive(Debug, PartialEq, Eq)]
struct GateSet {
    gates: Arc<[QuestGate]>,
}

/// An edge's non-empty, canonical (sorted, duplicate-free) quest-stage gates.
/// The edge is usable only while every gate is `True`; clones share the gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestGates(Arc<GateSet>);

impl QuestGates {
    /// Validate and canonicalize the gates.
    pub fn new(gates: impl IntoIterator<Item = QuestGate>) -> Result<Self, QuestGateError> {
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
            gates: Arc::from(gates),
        })))
    }

    /// The gates, in canonical order.
    pub fn gates(&self) -> &[QuestGate] {
        &self.0.gates
    }

    /// Share the canonical gate list without copying its members.
    pub fn shared_gates(&self) -> Arc<[QuestGate]> {
        Arc::clone(&self.0.gates)
    }

    /// `True` only when evidence proves every gate; `False` when it disproves
    /// one; otherwise `Unknown`.
    pub fn test(&self, evidence: Option<&QuestEvidence>) -> Truth {
        match evidence {
            Some(evidence) => Truth::all(self.0.gates.iter().map(|gate| evidence.test(gate))),
            None => Truth::Unknown,
        }
    }
}

impl PartialOrd for QuestGates {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Canonical pack order: gates key by key.
impl Ord for QuestGates {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .gates
            .iter()
            .map(gate_key)
            .cmp(other.0.gates.iter().map(gate_key))
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
/// provider snapshot and the causal freshness floor every answer must meet.
/// Nav only asks the provider; it never reads a journal or varp itself.
#[derive(Clone)]
pub struct QuestEvidence {
    provider: Arc<dyn EvidenceProvider>,
    required_after: EvidenceStamp,
    owned_provider_bytes: usize,
}

impl QuestEvidence {
    pub fn new(provider: Arc<dyn EvidenceProvider>, required_after: EvidenceStamp) -> Self {
        Self {
            provider,
            required_after,
            owned_provider_bytes: 0,
        }
    }

    /// A provider allocated by this request, rather than retained by another
    /// owner. The byte bound includes its Arc backing and all owned buffers.
    pub fn with_owned_provider_bytes(mut self, bytes: usize) -> Self {
        self.owned_provider_bytes = bytes;
        self
    }

    pub fn owned_provider_bytes(&self) -> usize {
        self.owned_provider_bytes
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

/// The same provider snapshot and floor; refreshed evidence is new evidence
/// even when it would answer alike.
impl PartialEq for QuestEvidence {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.provider, &other.provider) && self.required_after == other.required_after
    }
}

impl Eq for QuestEvidence {}

impl fmt::Debug for QuestEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuestEvidence")
            .field("required_after", &self.required_after)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "quest_gates_tests.rs"]
pub(crate) mod tests;

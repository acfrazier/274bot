//! Shared selected-fact values. Loading is off-pump; gates never guess missing facts.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

pub use client::io::ClientRevision;
use serde::{Deserialize, Deserializer, Serialize};

/// Case-sensitive content identity, serialized as a string (never a display name).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct FactKey(pub Arc<str>);

impl FactKey {
    /// Intern at preparation/deserialization, not in a tick. Weak entries do not
    /// keep a released catalog's strings alive.
    pub fn new(text: &str) -> Self {
        type InternPool = Mutex<HashMap<u64, Vec<Weak<str>>>>;
        static POOL: LazyLock<InternPool> = LazyLock::new(Mutex::default);
        let mut hash = DefaultHasher::new();
        text.hash(&mut hash);
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let entries = pool.entry(hash.finish()).or_default();
        let mut found = None;
        entries.retain(|entry| {
            if let Some(value) = entry.upgrade() {
                if value.as_ref() == text {
                    found = Some(value);
                }
                true
            } else {
                false
            }
        });
        if let Some(value) = found {
            return Self(value);
        }
        let value: Arc<str> = text.into();
        entries.push(Arc::downgrade(&value));
        Self(value)
    }
}

impl<'de> Deserialize<'de> for FactKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        Ok(Self::new(&text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub file: Arc<str>,
    pub first: u32,
    pub last: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub code: Arc<str>,
    pub sources: Arc<[SourceSpan]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Knowledge<T> {
    Known(T),
    Partial { known: T, gaps: Arc<[Gap]> },
    Unknown(Gap),
}

impl<T> Knowledge<T> {
    /// Missing coverage cannot authorize a gate, even if the known subset passes.
    pub fn test(&self, predicate: impl FnOnce(&T) -> Truth) -> Truth {
        match self {
            Self::Known(value) => predicate(value),
            Self::Partial { .. } | Self::Unknown(_) => Truth::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Truth {
    True,
    False,
    Unknown,
}

impl std::ops::Not for Truth {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl Truth {
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }

    pub fn all(values: impl IntoIterator<Item = Self>) -> Self {
        values
            .into_iter()
            .try_fold(Self::True, |acc, value| {
                let next = acc.and(value);
                if next == Self::False {
                    Err(next)
                } else {
                    Ok(next)
                }
            })
            .unwrap_or(Self::False)
    }

    pub fn any(values: impl IntoIterator<Item = Self>) -> Self {
        values
            .into_iter()
            .try_fold(Self::False, |acc, value| {
                let next = acc.or(value);
                if next == Self::True {
                    Err(next)
                } else {
                    Ok(next)
                }
            })
            .unwrap_or(Self::True)
    }
}

/// Runtime attachment: the final manifest digest is never embedded in its own bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPin {
    pub revision: ClientRevision,
    pub engine_commit: Arc<str>,
    pub content_commit: Arc<str>,
    pub cache_id: Arc<str>,
    pub content_id: Arc<str>,
    pub nav_sha256: [u8; 32],
    pub flags_sha256: [u8; 32],
    pub schema: u16,
    pub manifest_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactError {
    FamilyUnavailable(FactKey),
    UnknownKey(FactKey),
    Schema {
        expected: u16,
        actual: u16,
    },
    PinMismatch,
    Invalid {
        source: SourceSpan,
        reason: Arc<str>,
    },
}

/// Process-local identity; intentionally not deserializable or persistable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub slot: u64,
    pub run: u64,
    pub session: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationExhausted;

impl RunKey {
    /// Allocate a new worker incarnation, never a username/hash. Renames keep it;
    /// worker replacement and remove/re-add must allocate again.
    pub fn allocate() -> Result<Self, GenerationExhausted> {
        static NEXT_SLOT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            slot: allocate_slot(&NEXT_SLOT)?,
            run: 1,
            session: 1,
        })
    }

    pub fn next_run(self) -> Result<Self, GenerationExhausted> {
        Ok(Self {
            run: self.run.checked_add(1).ok_or(GenerationExhausted)?,
            ..self
        })
    }

    pub fn next_session(self) -> Result<Self, GenerationExhausted> {
        Ok(Self {
            session: self.session.checked_add(1).ok_or(GenerationExhausted)?,
            ..self
        })
    }
}

fn allocate_slot(next: &AtomicU64) -> Result<u64, GenerationExhausted> {
    next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .map_err(|_| GenerationExhausted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InclusiveRange {
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub min: Option<i32>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub max: Option<i32>,
}

impl InclusiveRange {
    /// Evaluate every possible value, not merely a lower bound. Invalid ranges
    /// remain unknown rather than authorizing by vacuous truth.
    pub fn test(&self, possible: &Self) -> Truth {
        let (min, max) = (self.min.unwrap_or(i32::MIN), self.max.unwrap_or(i32::MAX));
        let (lo, hi) = (
            possible.min.unwrap_or(i32::MIN),
            possible.max.unwrap_or(i32::MAX),
        );
        if min > max || lo > hi {
            Truth::Unknown
        } else if lo >= min && hi <= max {
            Truth::True
        } else if hi < min || lo > max {
            Truth::False
        } else {
            Truth::Unknown
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalRange {
    pub signal: FactKey,
    pub values: InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageWindow {
    pub quest: FactKey,
    pub signal: FactKey,
    pub values: InclusiveRange,
}

impl StageWindow {
    /// Numeric gate only. The catalog additionally checks quest, pin, binding,
    /// role and freshness before using this value calculation.
    pub fn evaluate(&self, signals: &[SignalRange]) -> Truth {
        let mut result = None;
        for signal in signals.iter().filter(|signal| signal.signal == self.signal) {
            let next = self.values.test(&signal.values);
            result = Some(match result {
                None => next,
                Some(previous) if previous == next => previous,
                Some(_) => Truth::Unknown,
            });
        }
        result.unwrap_or(Truth::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestGate {
    Complete(FactKey),
    Window(StageWindow),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityId {
    Npc(i32),
    Loc(i32),
    Obj(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemAmount {
    pub item: i32,
    pub count: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillMinimum {
    pub skill: u8,
    pub level: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequirementAt {
    Start,
    Stage(FactKey),
    Action(FactKey),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequirementKind {
    Skill(SkillMinimum),
    Item(ItemAmount),
    Quest(QuestGate),
    QuestPoints(u16),
    MembersWorld,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: FactKey,
    pub at: RequirementAt,
    pub kind: RequirementKind,
    pub source: SourceSpan,
}

/// Preparation-worker capability; no public construction or tick access.
/// Its worker constructor and family-cache mechanism belong to the family owners.
pub struct FamilyPreparation {
    _private: (),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_never_authorizes_even_when_negated_or_partial() {
        assert_eq!(!Truth::Unknown, Truth::Unknown);
        assert_eq!(Truth::all([]), Truth::True);
        assert_eq!(Truth::any([]), Truth::False);
        assert_eq!(Truth::Unknown.and(Truth::False), Truth::False);
        assert_eq!(Truth::Unknown.or(Truth::True), Truth::True);
        assert_eq!(Truth::Unknown.and(Truth::True), Truth::Unknown);
        assert_eq!(Truth::Unknown.or(Truth::False), Truth::Unknown);
        let partial = Knowledge::Partial {
            known: Vec::<i32>::new(),
            gaps: Arc::from([]),
        };
        assert_eq!(partial.test(|_| Truth::True), Truth::Unknown);
        assert_eq!(
            Knowledge::Known(Vec::<i32>::new()).test(|v| if v.is_empty() {
                Truth::True
            } else {
                Truth::False
            }),
            Truth::True
        );
    }

    #[test]
    fn windows_require_all_possibilities_and_include_endpoints() {
        let range = |min, max| InclusiveRange { min, max };
        let gate = StageWindow {
            quest: FactKey::new("cook"),
            signal: FactKey::new("cook"),
            values: range(Some(2), Some(4)),
        };
        let test = |possible| {
            gate.evaluate(&[SignalRange {
                signal: gate.signal.clone(),
                values: possible,
            }])
        };
        assert_eq!(test(range(Some(2), Some(4))), Truth::True);
        assert_eq!(test(range(Some(4), Some(4))), Truth::True);
        assert_eq!(test(range(Some(5), None)), Truth::False);
        assert_eq!(test(range(None, Some(1))), Truth::False);
        assert_eq!(test(range(Some(1), Some(2))), Truth::Unknown);
        assert_eq!(test(range(None, Some(3))), Truth::Unknown);
        assert_eq!(test(range(Some(4), Some(2))), Truth::Unknown);
        assert_eq!(
            gate.evaluate(&[SignalRange {
                signal: FactKey::new("other"),
                values: range(Some(2), Some(4))
            }]),
            Truth::Unknown
        );
        assert_eq!(
            gate.evaluate(&[
                SignalRange {
                    signal: gate.signal.clone(),
                    values: range(Some(2), Some(2))
                },
                SignalRange {
                    signal: gate.signal.clone(),
                    values: range(Some(5), Some(5))
                },
            ]),
            Truth::Unknown
        );
        assert_eq!(
            range(Some(4), Some(2)).test(&range(Some(3), Some(3))),
            Truth::Unknown
        );
        assert_eq!(gate.evaluate(&[]), Truth::Unknown);
        assert_eq!(
            range(None, Some(4)).test(&range(None, Some(4))),
            Truth::True
        );
        assert_eq!(
            range(Some(4), Some(4)).test(&range(Some(3), Some(4))),
            Truth::Unknown
        );
        assert_eq!(
            range(None, None).test(&range(Some(i32::MIN), Some(i32::MAX))),
            Truth::True
        );
    }

    #[test]
    fn incarnations_and_generations_never_reuse_or_wrap() {
        let original = RunKey::allocate().unwrap();
        let replacement = RunKey::allocate().unwrap();
        assert_ne!(original.slot, replacement.slot);
        let next = original.next_run().unwrap().next_session().unwrap();
        assert_eq!(next.slot, original.slot);
        assert_eq!(
            (next.run, next.session),
            (original.run + 1, original.session + 1)
        );
        let exhausted = RunKey {
            run: u64::MAX,
            session: u64::MAX,
            ..original
        };
        assert_eq!(exhausted.next_run(), Err(GenerationExhausted));
        assert_eq!(exhausted.next_session(), Err(GenerationExhausted));
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_slot(&counter), Ok(u64::MAX - 1));
        assert_eq!(allocate_slot(&counter), Err(GenerationExhausted));
        assert_eq!(allocate_slot(&counter), Err(GenerationExhausted));
    }

    #[test]
    fn required_facts_cannot_decode_as_empty_or_zero() {
        assert!(serde_json::from_str::<ItemAmount>(r#"{"item":1}"#).is_err());
        assert!(serde_json::from_str::<Gap>(r#"{"code":"missing"}"#).is_err());
        assert!(serde_json::from_str::<InclusiveRange>(r#"{"min":2}"#).is_err());
        assert!(serde_json::from_str::<InclusiveRange>(r#"{"max":2}"#).is_err());
        assert_eq!(
            serde_json::from_str::<InclusiveRange>(r#"{"min":null,"max":2}"#).unwrap(),
            InclusiveRange {
                min: None,
                max: Some(2)
            }
        );
        assert!(serde_json::from_str::<Knowledge<Vec<i32>>>(r#"{"Partial":{"gaps":[]}}"#).is_err());
        assert!(
            serde_json::from_str::<SourceSpan>(r#"{"file":"scripts/quest.rs2","first":1}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<SkillMinimum>(r#"{"skill":1,"level":2,"typo":3}"#).is_err());
    }
}

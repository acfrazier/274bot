//! Shared selected-fact values. Loading is off-pump; gates never guess missing facts.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

pub use client::io::ClientRevision;
use serde::{Deserialize, Deserializer, Serialize};

/// Case-sensitive content identity, serialized as a string (never a display name).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct FactKey(pub Arc<str>);

impl FactKey {
    /// Allocate an owned key off-pump. Family decoders share repeated text through
    /// a scoped [`FactStrings`], never a process-lifetime intern table.
    pub fn new(text: &str) -> Self {
        Self(Arc::from(text))
    }
}

impl<'de> Deserialize<'de> for FactKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        Ok(Self::new(&text))
    }
}

#[cfg(feature = "path-schema")]
impl schemars::JsonSchema for FactKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FactKey".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "minLength": 1 })
    }
}

/// Decode-local string interning for keys, source paths and gap codes.
/// Drop after preparation: only the returned Arcs then retain their strings.
#[derive(Default)]
pub struct FactStrings {
    strings: HashSet<Arc<str>>,
}

impl FactStrings {
    pub fn intern(&mut self, text: &str) -> Arc<str> {
        if let Some(value) = self.strings.get(text) {
            return Arc::clone(value);
        }
        let value: Arc<str> = Arc::from(text);
        self.strings.insert(Arc::clone(&value));
        value
    }
}

// Refusal paths clone a fixed key, without allocating or taking an intern lock.
pub(crate) static QUESTS_FAMILY: LazyLock<FactKey> = LazyLock::new(|| FactKey::new("quests"));
pub(crate) static GATHERING_FAMILY: LazyLock<FactKey> = LazyLock::new(|| FactKey::new("gathering"));
pub(crate) static PIN_FAMILY: LazyLock<FactKey> = LazyLock::new(|| FactKey::new("selected-pin"));

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

/// Process-local identity; `slot` comes from Play's worker lifetime owner only.
/// Intentionally not deserializable or persistable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub slot: u64,
    pub run: u64,
    pub session: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationExhausted;

impl RunKey {
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
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
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

/// Preparation-worker capability. Only [`Self::run`] constructs one, on its
/// worker thread; pump/tick callers never receive it. It cannot leave that
/// thread or be retained by a prepared card.
pub struct FamilyPreparation {
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl FamilyPreparation {
    /// Run preparation off-pump. The caller owns joining/settling the worker
    /// and must fence its result against Stop, replacement and profile removal.
    pub fn run<R: Send + 'static>(
        work: impl FnOnce(&mut Self) -> R + Send + 'static,
    ) -> std::io::Result<std::thread::JoinHandle<R>> {
        std::thread::Builder::new()
            .name("card-prepare".into())
            .spawn(move || {
                work(&mut Self {
                    _thread_bound: std::marker::PhantomData,
                })
            })
    }
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
        let gate = range(Some(2), Some(4));
        let test = |possible| gate.test(&possible);
        assert_eq!(test(range(Some(2), Some(4))), Truth::True);
        assert_eq!(test(range(Some(4), Some(4))), Truth::True);
        assert_eq!(test(range(Some(5), None)), Truth::False);
        assert_eq!(test(range(None, Some(1))), Truth::False);
        assert_eq!(test(range(Some(1), Some(2))), Truth::Unknown);
        assert_eq!(test(range(None, Some(3))), Truth::Unknown);
        assert_eq!(test(range(Some(4), Some(2))), Truth::Unknown);
        assert_eq!(
            range(Some(4), Some(2)).test(&range(Some(3), Some(3))),
            Truth::Unknown
        );
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
    fn generations_never_wrap_or_change_the_worker_identity() {
        let original = RunKey {
            slot: 7,
            run: 1,
            session: 1,
        };
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
        use crate::gather_methods::ToolUse;
        use crate::quest_facts::StageFacts;
        use crate::WorldTile;
        assert!(serde_json::from_str::<ToolUse>(r#"{"item":1265,"use_gate":null}"#).is_err());
        assert!(serde_json::from_str::<ToolUse>(r#"{"item":1265,"wield_gate":null}"#).is_err());
        let tool: ToolUse =
            serde_json::from_str(r#"{"item":1265,"use_gate":null,"wield_gate":null}"#).unwrap();
        assert!(tool.use_gate.is_none() && tool.wield_gate.is_none());
        assert!(serde_json::from_str::<StageFacts>(
            r#"{"id":"start","terminal":false,"signals":[]}"#
        )
        .is_err());
        let stage: StageFacts =
            serde_json::from_str(r#"{"id":"start","role":null,"terminal":false,"signals":[]}"#)
                .unwrap();
        assert!(stage.role.is_none());
        assert!(
            serde_json::from_str::<WorldTile>(r#"{"x":3200,"z":3200,"level":0,"plane":2}"#)
                .is_err()
        );
        assert_eq!(
            serde_json::from_str::<WorldTile>(r#"{"x":3200,"z":3200,"level":2}"#).unwrap(),
            WorldTile {
                x: 3200,
                z: 3200,
                level: 2
            }
        );
    }

    #[test]
    fn scoped_strings_share_storage_without_retaining_released_families() {
        let mut strings = FactStrings::default();
        let key = FactKey(strings.intern("cook"));
        let duplicate = FactKey(strings.intern("cook"));
        assert!(Arc::ptr_eq(&key.0, &duplicate.0));
        let weak = Arc::downgrade(&key.0);
        drop(strings);
        drop(duplicate);
        assert_eq!(key.0.as_ref(), "cook");
        drop(key);
        assert!(weak.upgrade().is_none());
    }
}

//! Shared prerequisite outcomes and logical-walk handover values.
use super::{WalkEnd, WalkOptions};
use api::prereq_knowledge::RefusalObservation;
use api::prereq_steps::ActionableGate;
use api::selected::QuestGate;
use api::WorldTile;
use std::sync::Arc;

/// Effective grants on the original walk, not raw settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrereqAllow {
    pub dialogue: bool,
    pub quest_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrereqStepIdentity {
    pub id: Arc<str>,
    pub source: Arc<str>,
}
impl std::fmt::Display for PrereqStepIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.id, self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrereqSwitch {
    Dialogue,
    QuestStart,
}
impl std::fmt::Display for PrereqSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dialogue => "prereq_dialogue",
            Self::QuestStart => "quest_start",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrereqMissing {
    Quest(QuestGate),
    FreeSlots { required: u16 },
    OwnedAnywhere { obj: i32 },
}

/// W1 remains the reused traveller's limit, not a runner-owned wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrereqWaitName {
    W2,
    W3,
    W4,
    W5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrereqFailure {
    ProbeUnacknowledged,
    ProbeDirection {
        expected: Arc<str>,
        seen: Arc<str>,
    },
    ReclaimLoop,
    DialogueFailed,
    CombatInterrupted,
    Action(Arc<str>),
    WaitSpent {
        wait: PrereqWaitName,
        since: u64,
        at: u64,
        bound: u16,
    },
    /// Every attempt of the conversation's reach budget failed. The engine
    /// cancelled the first Talk: it sent "I can't reach that!" and cleared
    /// the interaction (`Player.ts:1292-1295`), so waiting longer cannot
    /// recover it. Each later attempt re-approached the NPC and either sent a
    /// Talk that the engine also cancelled, or failed its approach. `cause`,
    /// `since`, `at` and the tiles describe the last failure: the player's
    /// and the NPC's tiles when it ended.
    TalkUnreachable {
        attempts: u8,
        since: u64,
        at: u64,
        from: Option<WorldTile>,
        npc: Option<WorldTile>,
        cause: TalkReachCause,
    },
    /// Used only when the reused machine identifies the expiry cause.
    MachineEnded {
        machine: &'static str,
        reason: Arc<str>,
    },
}

/// Why one attempt of a prerequisite Talk did not reach the NPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TalkReachCause {
    /// The engine cancelled the Talk with "I can't reach that!".
    Cancelled,
    /// The re-approach ended while the NPC was not ready to talk: its walk
    /// ended with `walk` (for example `Arrived` at the NPC's earlier tile),
    /// or the approach deadline passed before a walk receipt (`None`).
    Approach { walk: Option<WalkEnd> },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrereqRefusal {
    #[error("no prerequisite row serves {gate:?}")]
    NoRow { gate: QuestGate },
    #[error("{step}: {switch} is not allowed")]
    NotAllowed {
        step: PrereqStepIdentity,
        switch: PrereqSwitch,
    },
    #[error("{step}: missing {missing:?}")]
    PreconditionMissing {
        step: PrereqStepIdentity,
        missing: PrereqMissing,
    },
    #[error("{step}: approach ended {end:?}")]
    NoRouteToStep {
        step: PrereqStepIdentity,
        end: WalkEnd,
    },
    #[error("{step}: prerequisite failed: {why:?}")]
    StepFailed {
        step: PrereqStepIdentity,
        why: PrereqFailure,
    },
    #[error("{step}: prerequisite outcome was not witnessed")]
    StepUnwitnessed { step: PrereqStepIdentity },
    #[error("prerequisite repeated for {gate:?}")]
    StepRepeated { gate: QuestGate },
    #[error("{step}: crossing refused after prerequisite resolution")]
    RefusedAfterStep {
        step: PrereqStepIdentity,
        observation: Arc<RefusalObservation>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterIdentity {
    Manual { intent_seq: u64 },
    Compat { request_id: u64 },
}

/// Original caller policy retained while native inner requests get their own IDs.
#[derive(Clone)]
pub struct OuterWalk {
    pub identity: OuterIdentity,
    pub target: WorldTile,
    pub radius: u16,
    pub arrival: nav_planner::contract::ArrivalKind,
    pub avoid: Box<[Arc<str>]>,
    pub cross: Box<[Arc<str>]>,
    pub options: WalkOptions,
    pub allow_bank_fetch: bool,
    pub prereq_allow: PrereqAllow,
}

#[derive(Debug, Clone)]
pub enum HandoverEntry {
    NeedsStep(Arc<[ActionableGate]>),
    GateRefused(Arc<RefusalObservation>),
}

pub struct WalkHandover {
    pub outer: OuterWalk,
    pub entry: HandoverEntry,
    pub here: WorldTile,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{RetainedMemory, RunKey, StartError};

    fn handover() -> WalkHandover {
        let here = WorldTile {
            x: 3408,
            z: 3489,
            level: 0,
        };
        WalkHandover {
            outer: OuterWalk {
                identity: OuterIdentity::Manual { intent_seq: 9 },
                target: here,
                radius: 0,
                arrival: nav_planner::contract::ArrivalKind::Reach,
                avoid: Box::default(),
                cross: Box::default(),
                options: WalkOptions::default(),
                allow_bank_fetch: false,
                prereq_allow: PrereqAllow::default(),
            },
            entry: HandoverEntry::NeedsStep(Arc::from([])),
            here,
        }
    }

    #[test]
    fn retained_factory_handover_is_run_fenced_and_consumed_once() {
        let mut memory = RetainedMemory::default();
        let run = RunKey {
            slot: 2,
            run: 3,
            session: 4,
        };
        assert!(memory.take_walk_handover(run).is_none());
        let handover = handover();
        let measured = allocation_counter::measure(|| {
            memory
                .set_walk_handover(run, handover)
                .expect("vacant input");
        });
        assert_eq!(
            measured.count_total, 1,
            "only a pending handover allocates its box"
        );
        assert_eq!(
            std::mem::size_of::<Option<Box<crate::native::RetainedWalkHandover>>>(),
            std::mem::size_of::<usize>(),
            "ordinary retained memory pays one pointer, not an inline walk",
        );
        for stale in [
            RunKey { slot: 1, ..run },
            RunKey { run: 2, ..run },
            RunKey { session: 3, ..run },
        ] {
            assert!(memory.take_walk_handover(stale).is_none());
        }
        assert!(matches!(
            memory.set_walk_handover(run, self::handover()),
            Err(StartError::Busy)
        ));
        let taken = memory
            .take_walk_handover(run)
            .expect("matching factory consumes input");
        assert_eq!(
            taken.outer.identity,
            OuterIdentity::Manual { intent_seq: 9 }
        );
        assert!(memory.take_walk_handover(run).is_none());
        memory
            .set_walk_handover(run, self::handover())
            .expect("vacant after consumption");
        assert!(
            memory.take_walk_handover(run).is_some(),
            "cancel can take the same input"
        );
    }

    #[test]
    fn path_grants_are_additive_and_only_request_forbid_narrows() {
        use crate::native::{PrereqOptions, WalkBit};
        assert!(std::mem::size_of::<PrereqOptions>() <= 2);
        for header in [false, true] {
            for request in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                let mut options = WalkOptions {
                    allow_prereq_dialogue: request,
                    allow_quest_start: request,
                    ..WalkOptions::default()
                };
                PrereqOptions {
                    dialogue: header,
                    quest_start: header,
                }
                .apply(&mut options);
                let expected = if request == WalkBit::Forbid {
                    WalkBit::Forbid
                } else if header || request == WalkBit::Allow {
                    WalkBit::Allow
                } else {
                    WalkBit::Inherit
                };
                assert_eq!(options.allow_prereq_dialogue, expected);
                assert_eq!(options.allow_quest_start, expected);
            }
        }
    }
}

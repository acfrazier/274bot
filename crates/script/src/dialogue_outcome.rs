//! Shared dialogue completion policy, independent of either driver.
//! Observation adapters and page scheduling remain owned by each driver.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogueOutcome {
    Completed,
    Failed,
}

impl DialogueOutcome {
    /// A closed chat alone does not prove combat interruption: the local
    /// health-bar window may be raised by self-damage. The snapshot does not
    /// expose hit source, so require separate attacker evidence as well.
    pub fn combat_interrupted(chat_open: bool, in_combat: bool, attacker_evidence: bool) -> bool {
        !chat_open && in_combat && attacker_evidence
    }
}

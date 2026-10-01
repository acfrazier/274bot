//! Shared dialogue completion policy, independent of either driver.
//! Observation adapters and page scheduling remain owned by each driver.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogueOutcome {
    Completed,
    Failed,
    CombatInterrupted,
}

impl DialogueOutcome {
    /// A closed chat observed during combat is not a successful quiet end.
    /// Combat may close an interface without changing the player's hitpoints.
    pub(crate) fn combat_interruption(chat_open: bool, in_combat: bool) -> Option<Self> {
        (!chat_open && in_combat).then_some(Self::CombatInterrupted)
    }
}

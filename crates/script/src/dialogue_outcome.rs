//! Shared dialogue completion policy, independent of either driver.
//! Observation adapters and page scheduling remain owned by each driver.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogueOutcome {
    Completed,
    Failed,
}

impl DialogueOutcome {
    /// A closed chat observed during combat is not a successful quiet end.
    /// Combat may close an interface without changing the player's hitpoints.
    ///
    /// The `in_combat` fact is the local player's health-bar window
    /// (`combat_cycle > loop_cycle`, `crates/api/src/snapshot/decode.rs`).
    /// The 289 client sets `combat_cycle = cycle + 300` for every `Hit`
    /// mask without looking at the damage value (`actor_289.rs`), so a
    /// 0-damage miss raises it exactly like a damaging hit — unlike the
    /// `taking_damage` predicate, which requires a positive type-1 hitmark
    /// and is blind to misses. The NPC combat scripts call `if_close`
    /// before the damage roll lands (`npc_combat_melee.rs2`,
    /// `npc_combat_ranged.rs2`, `npc_combat_magic.rs2`), so the close and
    /// the hit that raises `in_combat` arrive together.
    pub fn combat_interrupted(chat_open: bool, in_combat: bool) -> bool {
        !chat_open && in_combat
    }
}

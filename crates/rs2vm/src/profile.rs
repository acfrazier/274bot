//! The interpreter's single revision-selection boundary.

/// The engine's accepted loc durations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocDuration {
    /// Every scripted duration must be positive.
    Positive,
    /// The engine also accepts zero inside an instance.
    ZeroInInstance {
        /// First instance x coordinate.
        min_x: i32,
    },
}
/// Engine semantics that differ between supported source trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineProfile {
    /// Validation of a loc change's life.
    pub loc_duration: LocDuration,
    /// Global NPC slot count, not a bound on a mutating population.
    pub npc_capacity: u32,
}
impl EngineProfile {
    /// Historical engine semantics (LocOps.ts:26,63; util/Environment.ts:7).
    pub const R274: Self = Self {
        loc_duration: LocDuration::Positive,
        npc_capacity: 16_383,
    };
    /// Production engine semantics (LocOps.ts:26,63; util/Environment.ts:7).
    pub const R289: Self = Self {
        loc_duration: LocDuration::Positive,
        npc_capacity: 16_383,
    };
    /// Instance-aware semantics (LocOps.ts:27,65; CoordGrid.ts:24-27; World.ts:118).
    pub const R377: Self = Self {
        loc_duration: LocDuration::ZeroInInstance { min_x: 6464 },
        npc_capacity: 16_383,
    };
}

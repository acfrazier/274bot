#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! Release navigation generation. Runtime planning stays in `nav-planner`.

pub mod bake;
pub mod canlight;
pub mod collision;
pub mod pack;
pub mod reachability;
pub mod services;
pub mod transport;
mod zones;

#[cfg(test)]
mod test_support;

/// Failure at a navigation-generation boundary.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    /// A required source file or local fact input is invalid.
    #[error(transparent)]
    SourceInput(#[from] SourceInputError),
    /// Canlight input or generation failed.
    #[error(transparent)]
    Canlight(#[from] canlight::CanlightError),
    /// Multiway table input or generation failed.
    #[error(transparent)]
    Multiway(#[from] pack::MultiwayError),
    /// A selected release revision is not supported.
    #[error("unsupported revision {revision}; use 274 or 289")]
    UnsupportedRevision { revision: u16 },
    /// A revision-bound cache path lacks the expected parent.
    #[error("revision 274 cache path has no data/pack parent")]
    CachePathMissingParent { cache_dir: std::path::PathBuf },
    /// A selected revision's local game data could not be decoded.
    #[error("game data revision {revision}: {detail}")]
    SelectedGameDataUnavailable { revision: u16, detail: String },
    /// Local quest facts could not be decoded.
    #[error("quest facts revision {revision}: {detail}")]
    QuestFactsUnavailable { revision: u16, detail: String },
    /// The hunter source inputs could not be collected or interpreted.
    #[error("{detail}")]
    HunterInputInvalid { detail: String },
    /// A hunter definition has an invalid combat level.
    #[error("hunter NPC {npc_id} has invalid combat level {level}")]
    HunterCombatLevelInvalid { npc_id: String, level: i32 },
    /// The packed zone-kind table exceeds its limit.
    #[error("zone kind count exceeds the packed limit")]
    ZoneKindCountOverflow,
    /// A map spawn refers to a hunter without a config.
    #[error("map spawn references hunter id {npc_id} without a config")]
    HunterSpawnMissingConfig { npc_id: i32 },
    /// A hunter's strength-check mode is unsupported.
    #[error("hunter NPC {npc_id} has unsupported check_nottoostrong={value}")]
    HunterCheckModeUnsupported { npc_id: String, value: String },
    /// A hunter's combat cap cannot be represented.
    #[error("hunter NPC {npc_id} has invalid combat cap")]
    HunterCombatCapInvalid { npc_id: String },
    /// A hunter's acquisition mode is unsupported.
    #[error("hunter NPC {npc_id} has unsupported find_newmode={value}")]
    HunterFindModeUnsupported { npc_id: String, value: String },
    /// A hunter kind row could not be found.
    #[error("hunter NPC {npc_id} has no kind row")]
    HunterKindMissing { npc_id: String },
    /// A hunter has an invalid size.
    #[error("hunter NPC {npc_id} has invalid size {size}")]
    HunterSizeInvalid { npc_id: String, size: i32 },
    /// A stationary melee hunter exceeds the supported footprint limit.
    #[error("stationary melee hunter {npc_id} exceeds the 6x6 footprint limit")]
    StationaryMeleeHunterTooLarge { npc_id: String },
    /// A stationary ranged hunter's bounds are invalid.
    #[error("hunter NPC {npc_id}: {detail}")]
    StationaryHunterBoundsInvalid { npc_id: String, detail: String },
    /// A stationary melee hunter's attack shape could not be derived.
    #[error("hunter NPC {npc_id}: {source}")]
    StationaryMeleeShapeInvalid {
        npc_id: String,
        #[source]
        source: Box<GenerationError>,
    },
    /// The hunter acquisition radius overflowed.
    #[error("hunter NPC {npc_id} acquisition radius overflows")]
    HunterAcquisitionRadiusOverflow { npc_id: String },
    /// The hunter tether range overflowed.
    #[error("hunter NPC {npc_id} tether range overflows")]
    HunterTetherRangeOverflow { npc_id: String },
    /// The hunter radius cannot be represented by the packed format.
    #[error("hunter NPC {npc_id} radius {radius} exceeds u8")]
    HunterRadiusTooLarge { npc_id: String, radius: i32 },
    /// The hunter chase radius overflowed.
    #[error("hunter NPC {npc_id} chase radius overflows")]
    HunterChaseRadiusOverflow { npc_id: String },
    /// A moving hunter has no supported step rule.
    #[error("hunter NPC {npc_id} walks but has no step rule")]
    HunterStepRuleMissing { npc_id: String },
    /// A curated zone group overlaps another curated group.
    #[error("zone {kind_id}@{x},{z},{level} belongs to more than one curated group")]
    DuplicateCuratedZoneMembership {
        kind_id: std::sync::Arc<str>,
        x: i32,
        z: i32,
        level: u8,
    },
    /// A curated zone group matched no hunter spawn.
    #[error("curated zone group {group_id} matched no hunter spawn")]
    EmptyCuratedZoneGroup { group_id: String },
    /// The zone shape table exceeds its packed limit.
    #[error("zone shape count exceeds the packed limit")]
    ZoneShapeCountOverflow,
    /// The zone group table exceeds its packed limit.
    #[error("zone group count exceeds the packed limit")]
    ZoneGroupCountOverflow,
    /// The zone table exceeds its packed limit.
    #[error("zone count exceeds the packed limit")]
    ZoneCountOverflow,
    /// A zone grid dimension exceeds its packed limit.
    #[error("zone grid {axis} exceeds u32")]
    ZoneGridDimensionOverflow { axis: &'static str },
    /// The derived zone table violates its invariants.
    #[error("invalid derived zone table: {source}")]
    DerivedZoneTableInvalid {
        #[source]
        source: nav_planner::zones::ZoneTableError,
    },
    /// A zone carving index exceeds its packed limit.
    #[error("zone carve index {index} exceeds u16")]
    ZoneCarveIndexOverflow { index: usize },
    /// Ranged visibility carving needs the raw collision flags.
    #[error("ranged visibility carving requires raw baked collision flags")]
    RangedVisibilityNeedsRawFlags,
    /// Mobile reach carving needs the raw collision flags.
    #[error("mobile hunter reach carving requires raw baked collision flags")]
    MobileReachNeedsRawFlags,
    /// A stationary melee footprint exceeds its mask dimensions.
    #[error("stationary melee hunter footprint exceeds the 8x8 shape limit")]
    StationaryMeleeFootprintTooLarge,
    /// A stationary melee footprint coordinate overflowed.
    #[error("stationary melee footprint overflows {axis}")]
    StationaryMeleeFootprintOverflow { axis: &'static str },
    /// A stationary melee shape bit is outside the packed mask.
    #[error("stationary melee mask bit is out of range")]
    StationaryMeleeMaskBitOutOfRange,
    /// An openable door's adjacent face coordinate overflowed.
    #[error("openable door face overflows {axis}")]
    OpenableDoorFaceOverflow { axis: &'static str },
    /// No door configuration file could be read.
    #[error("no door configs parsed (need {needed} in {doors_dir} plus {gates})")]
    NoDoorConfigs {
        needed: String,
        doors_dir: std::path::PathBuf,
        gates: std::path::PathBuf,
    },
    /// A transport-content validation failed.
    #[error("{detail}")]
    TransportValidation {
        validator: TransportValidator,
        detail: String,
    },
    /// Whole-world collision generation failed.
    #[error("collision bake failed: {source}")]
    CollisionBake {
        #[source]
        source: Box<GenerationError>,
    },
    /// Navpois source data could not be analyzed.
    #[error("{detail}")]
    PoisSourceInvalid { detail: String },
    #[error("navpois encode: {source}")]
    PoisEncodeFailed {
        #[source]
        source: nav_planner::map::MapError,
    },
    /// Pack encoding failed.
    #[error("pack encode failed: {source}")]
    PackEncodeFailed {
        #[source]
        source: nav_planner::pack::PackError,
    },
    /// A file or other artifact could not be read or written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Generated pack data violates the runtime wire contract.
    #[error(transparent)]
    Pack(#[from] nav_planner::pack::PackError),
}

/// Context for a file that belongs to a bake's required source closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceInputContext {
    /// One door configuration.
    DoorConfig,
    /// The gates configuration.
    GatesConfig,
    /// A local core game-data file.
    GameData,
    /// Local quest facts.
    QuestFacts,
    /// The client cache's loc definitions.
    LocDefinitions,
    /// The bank-zone source.
    BankZones,
}

impl std::fmt::Display for SourceInputContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DoorConfig => "door config",
            Self::GatesConfig => "gates config",
            Self::GameData => "game data",
            Self::QuestFacts => "quest facts",
            Self::LocDefinitions => "cannot load loc defs from",
            Self::BankZones => "canlight bank_zones",
        })
    }
}

/// Typed failures while reading or validating the required source closure.
#[derive(Debug, thiserror::Error)]
pub enum SourceInputError {
    /// A required source file or directory could not be read.
    #[error("{context} {path}: {source}")]
    Io {
        context: SourceInputContext,
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A source file is not valid UTF-8.
    #[error("{context} {path}: {source}")]
    InvalidUtf8 {
        context: SourceInputContext,
        path: std::path::PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },
}

/// Which content validation produced a transport-bake error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportValidator {
    /// Wilderness teleport legality.
    WildernessTeleportLegality,
    /// Members-only source/guard consistency.
    MembersGuards,
}

impl From<GenerationError> for String {
    fn from(error: GenerationError) -> Self {
        error.to_string()
    }
}

/// Verbose generator diagnostics use the shared host logging setting.
pub fn debug_enabled() -> bool {
    api::hostlog::debug_enabled()
}

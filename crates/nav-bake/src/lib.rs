#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! Release navigation generation. Runtime planning stays in `nav-planner`.

pub mod bake;
pub mod bundle;
pub mod canlight;
pub mod collision;
pub mod manifest;
pub mod pack;
pub mod services;
pub mod transport;
mod zones;

/// Failure at a navigation-generation boundary.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    /// A source file, its identity, or the required source inventory is invalid.
    #[error(transparent)]
    SourceInput(#[from] SourceInputError),
    /// A staged artifact no longer matches its bake stamp.
    #[error(transparent)]
    StaleStamp(#[from] StaleStampError),
    /// Canlight input or generation failed.
    #[error(transparent)]
    Canlight(#[from] canlight::CanlightError),
    /// A selected release revision is not supported.
    #[error("unsupported revision {revision}; use 274 or 289")]
    UnsupportedRevision { revision: u16 },
    /// A selected revision's embedded game data could not be loaded.
    #[error("{detail}")]
    SelectedGameDataUnavailable { revision: u16, detail: String },
    /// A revision-bound cache path lacks the expected parent.
    #[error("revision 274 cache path has no data/pack parent")]
    CachePathMissingParent { cache_dir: std::path::PathBuf },
    /// A cache manifest could not be read.
    #[error("cache manifest {path}: {source}")]
    CacheManifestRead {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A cache manifest could not be parsed.
    #[error("cache manifest {path}: {source}")]
    CacheManifestParse {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// Capturing or verifying a cache manifest failed.
    #[error("{detail}")]
    CacheManifestCapture { detail: String },
    /// The cache manifest and selected revision differ.
    #[error(
        "cache manifest revision {manifest_revision} does not match selected revision {selected_revision}"
    )]
    CacheManifestRevisionMismatch {
        manifest_revision: u16,
        selected_revision: u16,
    },
    /// The cache manifest does not match the cache files.
    #[error("cache manifest does not match cache bytes at {cache_dir}")]
    CacheManifestMismatch { cache_dir: std::path::PathBuf },
    /// The cache manifest has no config archive identity.
    #[error("cache manifest has no config archive")]
    CacheManifestConfigMissing,
    /// The selected config archive does not belong to the verified cache.
    #[error("selected config {path} does not belong to the verified revision {revision} cache")]
    ConfigCacheMismatch {
        revision: u16,
        path: std::path::PathBuf,
    },
    /// The decoded cache identity could not be computed.
    #[error(transparent)]
    DecodedIdentityFailed(#[from] client::content_identity::ContentIdentityError),
    /// The versionlist could not be read during offline identity preparation.
    #[error("{source}")]
    VersionlistRead {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Cache files changed during offline identity preparation.
    #[error("cache changed during offline identity preparation")]
    CacheChangedDuringIdentityPreparation,
    /// Captured cache inputs no longer match during offline identity preparation.
    #[error("cache inputs changed before offline identity preparation")]
    CacheInputsChangedDuringIdentityPreparation,
    /// The versionlist changed during offline identity preparation.
    #[error("cache versionlist changed during offline identity preparation")]
    VersionlistChangedDuringIdentityPreparation,
    /// The configured resource directory is empty.
    #[error("BOT_NAV_RESOURCE_DIR is empty")]
    EmptyResourceDirectory,
    /// The build-script output path does not have the expected Cargo layout.
    #[error("unexpected build script output layout {out_dir}; set BOT_NAV_RESOURCE_DIR")]
    UnexpectedBuildOutputLayout { out_dir: std::path::PathBuf },
    /// A relative resource directory escapes its root.
    #[error("resource directory {path} must not escape its root")]
    ResourceDirectoryEscapesRoot { path: std::path::PathBuf },
    /// The current working directory could not be read.
    #[error("working directory: {source}")]
    WorkingDirectoryRead {
        #[source]
        source: std::io::Error,
    },
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
    /// The selected quest family identity could not be built.
    #[error("quest family: {error:?}")]
    QuestFamilyIdentityFailed { error: api::selected::FactError },
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
    /// The generated zone count cannot be recorded in a manifest.
    #[error("zone count exceeds the manifest range")]
    ManifestZoneCountOverflow,
    /// The generated NPC zone count cannot be recorded in a manifest.
    #[error("NPC zone count exceeds the manifest range")]
    ManifestNpcZoneCountOverflow,
    /// The decoded content identity is not a SHA-256 value.
    #[error("decoded content identity is not SHA-256 hex")]
    InvalidDecodedContentIdentity,
    /// The baker source digest is not a SHA-256 value.
    #[error("source digest is not SHA-256 hex")]
    InvalidSourceDigest,
    /// The navpois generator identity is not a SHA-256 value.
    #[error("navpois generator identity is not SHA-256 hex")]
    InvalidPoisGeneratorIdentity,
    /// Baker inputs changed during bake preparation.
    #[error("baker inputs changed during preparation")]
    BakerInputsChangedDuringPreparation,
    /// A bound bake was requested without both its revision and cache.
    #[error("a bound bake needs both a revision and its cache manifest")]
    BoundBakeNeedsRevisionAndCache,
    /// Navpois generator source root could not be found.
    #[error("navpois generator workspace root is unavailable")]
    PoisGeneratorWorkspaceUnavailable,
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
    /// A regular bake source file.
    BakeInput,
    /// A recursive bake source directory.
    BakeInputDirectory,
    /// One door configuration.
    DoorConfig,
    /// The gates configuration.
    GatesConfig,
    /// The client cache's loc definitions.
    LocDefinitions,
    /// The bank-zone source.
    BankZones,
    /// A navpois generator source file.
    PoisGeneratorSource,
}

impl std::fmt::Display for SourceInputContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BakeInput => "bake input",
            Self::BakeInputDirectory => "bake input directory",
            Self::DoorConfig => "door config",
            Self::GatesConfig => "gates config",
            Self::LocDefinitions => "cannot load loc defs from",
            Self::BankZones => "canlight bank_zones",
            Self::PoisGeneratorSource => "navpois generator source",
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
    /// A source file could not be hashed.
    #[error("bake input {path}: {detail}")]
    HashFailed {
        path: std::path::PathBuf,
        detail: String,
    },
    /// An explicit input path does not identify a regular file.
    #[error("bake input {path} is not a file")]
    NotRegularFile { path: std::path::PathBuf },
    /// A requested source path is absent from the captured fingerprint set.
    #[error("source input {path} was not fingerprinted")]
    NotFingerprinted { path: std::path::PathBuf },
    /// A required-input inventory line names an unknown kind.
    #[error("required-content-{revision} line {line}: unknown kind {kind:?}")]
    UnknownInventoryKind {
        revision: u16,
        line: usize,
        kind: String,
    },
    /// A required-input inventory line has no path.
    #[error("required-content-{revision} line {line}: no path")]
    MissingInventoryPath { revision: u16, line: usize },
    /// A required-input inventory path is not content-relative.
    #[error("required-content-{revision} line {line}: {path} is not content-relative")]
    InvalidInventoryPath {
        revision: u16,
        line: usize,
        path: String,
    },
}

/// Kind of a staged artifact checked by a bake stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedArtifact {
    /// The navigation pack.
    Pack,
    /// The collision-flags sidecar.
    Flags,
    /// The reachability sidecar.
    Reach,
    /// The canlight sidecar.
    Canlight,
    /// The navigation manifest.
    Manifest,
    /// The point-of-interest sidecar.
    NavPois,
}

impl std::fmt::Display for StagedArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pack => "pack",
            Self::Flags => "flags",
            Self::Reach => "reach",
            Self::Canlight => "canlight",
            Self::Manifest => "manifest",
            Self::NavPois => "navpois",
        })
    }
}

/// The exact stale-stamp cause returned by [`bundle::BakeStamp::covers`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StaleStampError {
    /// The stamp's selected revision differs.
    #[error("staged navigation is revision {stamped}, build selects {selected}")]
    RevisionMismatch { stamped: u16, selected: u16 },
    /// The stamp's selected format differs.
    #[error("staged navigation format {stamped} is not {selected}")]
    FormatMismatch { stamped: String, selected: String },
    /// The baker or its source identity changed.
    #[error("bake generator changed (nav bake/format sources or generator id)")]
    GeneratorChanged,
    /// The selected cache identity changed.
    #[error("selected cache identity changed")]
    CacheIdentityChanged,
    /// A staged artifact is absent.
    #[error("staged {artifact} {path} is missing")]
    StagedArtifactMissing {
        artifact: StagedArtifact,
        path: String,
    },
    /// A staged artifact's byte count differs from the stamp.
    #[error("staged {artifact} {path} is {actual_bytes} bytes, stamped {stamped_bytes}")]
    StagedArtifactSizeMismatch {
        artifact: StagedArtifact,
        path: String,
        actual_bytes: u64,
        stamped_bytes: u64,
    },
    /// The navpois byte count differs from the stamp.
    #[error("staged navpois is {actual_bytes} bytes, stamped {stamped_bytes}")]
    NavPoisSizeMismatch {
        actual_bytes: u64,
        stamped_bytes: u64,
    },
    /// The stamp has no navpois byte count.
    #[error("staged navpois byte count is missing from the stamp")]
    NavPoisBytesMissingFromStamp,
    /// The staged manifest byte count differs from the stamp.
    #[error("staged navigation manifest is {actual_bytes} bytes, stamped {stamped_bytes}")]
    ManifestSizeMismatch {
        actual_bytes: u64,
        stamped_bytes: u64,
    },
    /// A staged digest is absent.
    #[error("staged {artifact} is missing")]
    StagedDigestMissing { artifact: StagedArtifact },
    /// A staged digest differs from the stamp.
    #[error("staged {artifact} digest does not match the stamp")]
    StagedDigestMismatch { artifact: StagedArtifact },
    /// The stamp has no manifest byte count.
    #[error("staged navigation manifest is missing")]
    ManifestBytesMissingFromStamp,
    /// The manifest is missing from the staged directory.
    #[error("staged navigation manifest is missing")]
    ManifestBytesMissingFromDisk,
    /// The stamp has no relative manifest path.
    #[error("staged navigation manifest path is missing from the stamp")]
    ManifestPathMissingFromStamp,
    /// The stamp has no manifest digest.
    #[error("staged navigation manifest digest is missing from the stamp")]
    ManifestDigestMissingFromStamp,
    /// The stamp has no navpois digest.
    #[error("staged navpois is missing")]
    NavPoisDigestMissingFromStamp,
    /// The navpois generator identity changed.
    #[error("navpois generator changed")]
    NavPoisGeneratorChanged {
        stamped: Option<String>,
        expected: String,
    },
    /// The canonical fingerprint set has a different number of entries.
    #[error("canonical inputs changed ({stamped} stamped, {present} present)")]
    CanonicalInputCountMismatch { stamped: usize, present: usize },
    /// One canonical fingerprint differs.
    #[error("canonical input {path} changed")]
    CanonicalInputChanged { path: String },
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

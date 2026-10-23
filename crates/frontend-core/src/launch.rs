//! What a play session launches from: the connection settings, the shared
//! client template a prepared server profile yields, and its one-use final
//! validation.

use std::path::Path;
use std::sync::Arc;

use api::snapshot::WorldTile;
use host_play::{PlayOptions, SharedClientTemplate, ValidatedTemplate};

/// Connection and client settings for a play session.
pub struct PlayConfig(Box<PlayOptions>);

impl PlayConfig {
    /// Settings for a direct play session over TCP to `host`:`port`, with
    /// the client cache in `cache_dir` (a headless driver's play).
    pub fn direct(host: &str, port: u16, cache_dir: &str, lowmem: bool) -> Self {
        Self(Box::new(PlayOptions {
            host: host.to_string(),
            transport: host_play::Transport::Tcp,
            port,
            cache_dir: cache_dir.to_string(),
            lowmem,
            mainland: false,
        }))
    }

    /// The same settings, copied.
    pub fn duplicate(&self) -> Self {
        Self(Box::new((*self.0).clone()))
    }

    /// The game host name.
    pub fn host(&self) -> &str {
        &self.0.host
    }

    /// The game port.
    pub fn port(&self) -> u16 {
        self.0.port
    }

    /// The client cache directory.
    pub fn cache_dir(&self) -> &str {
        &self.0.cache_dir
    }

    /// Whether slots start in low-memory mode.
    pub fn lowmem(&self) -> bool {
        self.0.lowmem
    }

    /// Whether a fresh slot hops to the mainland once in a loaded scene.
    pub fn mainland(&self) -> bool {
        self.0.mainland
    }

    /// Set whether a fresh slot hops to the mainland.
    pub fn set_mainland(&mut self, mainland: bool) {
        self.0.mainland = mainland;
    }
}

/// The shared client template of a prepared server profile: cache,
/// interfaces and navigation decoded once for every slot.
pub struct ProfileTemplate(Arc<SharedClientTemplate>);

impl ProfileTemplate {
    /// A template over an explicit host-play template (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_host(template: Arc<SharedClientTemplate>) -> Self {
        Self(template)
    }

    pub(crate) fn host_template(&self) -> &Arc<SharedClientTemplate> {
        &self.0
    }

    /// Another handle on the same template.
    pub fn share(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    /// Whether this profile's map treats the world as members.
    pub fn map_members(&self) -> bool {
        self.0.profile().map_members()
    }

    /// The scatter seed tile this profile gives the login `uid`.
    pub fn scatter_tile_for(&self, uid: i32) -> WorldTile {
        self.0.scatter_tile_for(uid)
    }
}

/// A template that passed the final resource validation, consumed by the
/// play session it launches.
pub struct ValidatedProfile(ValidatedTemplate);

impl ValidatedProfile {
    /// Whether this validation is for `template`.
    pub fn is_for(&self, template: &ProfileTemplate) -> bool {
        Arc::ptr_eq(self.0.template(), template.host_template())
    }
}

/// How [`crate::OperatorSession::start_play`] launches its play session.
pub struct PlayLaunch {
    source: LaunchSource,
    mainland: bool,
    auto_world: Option<u16>,
    keep_world_state: bool,
}

enum LaunchSource {
    Validated(ValidatedTemplate),
    Template(Arc<SharedClientTemplate>),
    Direct(Box<PlayOptions>),
}

impl PlayLaunch {
    /// Launch from a validated profile template.
    pub fn validated(validated: ValidatedProfile, mainland: bool) -> Self {
        Self::new(LaunchSource::Validated(validated.0), mainland)
    }

    /// Launch from a prepared profile template (validated at launch).
    pub fn template(template: &ProfileTemplate, mainland: bool) -> Self {
        Self::new(LaunchSource::Template(Arc::clone(&template.0)), mainland)
    }

    /// Launch straight from connection settings (the legacy local 274 path).
    pub fn direct(config: &PlayConfig) -> Self {
        Self::new(LaunchSource::Direct(config.0.clone()), config.0.mainland)
    }

    fn new(source: LaunchSource, mainland: bool) -> Self {
        Self {
            source,
            mainland,
            auto_world: None,
            keep_world_state: false,
        }
    }

    /// Log every slot into world `number` automatically.
    pub fn with_auto_world(mut self, number: u16) -> Self {
        self.auto_world = Some(number);
        self
    }

    /// Keep a navigation world state beside each slot's published snapshot
    /// (read through [`crate::OperatorSession::with_slot_world_state`]).
    pub fn keeping_world_state(mut self) -> Self {
        self.keep_world_state = true;
        self
    }

    pub(crate) fn into_parts(self) -> (LaunchParts, Option<u16>, bool) {
        let parts = match self.source {
            LaunchSource::Validated(validated) => LaunchParts::Validated(validated, self.mainland),
            LaunchSource::Template(template) => LaunchParts::Template(template, self.mainland),
            LaunchSource::Direct(options) => LaunchParts::Direct(*options),
        };
        (parts, self.auto_world, self.keep_world_state)
    }
}

pub(crate) enum LaunchParts {
    Validated(ValidatedTemplate, bool),
    Template(Arc<SharedClientTemplate>, bool),
    Direct(PlayOptions),
}

impl LaunchParts {
    pub(crate) fn map_members(&self) -> bool {
        match self {
            Self::Validated(validated, _) => validated.template().profile().map_members(),
            Self::Template(template, _) => template.profile().map_members(),
            Self::Direct(_) => false,
        }
    }
}

/// The scatter seed tile the legacy local profile gives the login `uid`.
pub fn default_scatter_tile_for(uid: i32) -> WorldTile {
    host_play::scatter_tile_for(uid)
}

/// Production server-profile inputs from the command line.
#[derive(Debug)]
pub struct ProfileArgs(host_play::ProfileOptions);

impl ProfileArgs {
    /// Profile inputs from explicit host-play options (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_host(options: host_play::ProfileOptions) -> Self {
        Self(options)
    }

    /// The same inputs, copied.
    pub fn duplicate(&self) -> Self {
        Self(self.0.clone())
    }

    /// Parsed numeric client revision for offline map-bundle tooling.
    pub fn revision_number(&self) -> Result<Option<u16>, ProfileError> {
        self.0
            .revision
            .as_deref()
            .map(host_play::parse_revision)
            .transpose()
            .map(|revision| revision.map(|revision| revision.as_i32() as u16))
            .map_err(|error| ProfileError(ProfileFault::Arguments(error)))
    }

    /// Cache directory selected for offline map-bundle tooling.
    pub fn cache_dir(&self) -> Option<&Path> {
        self.0.cache_dir.as_deref()
    }

    /// Unpacked cache root selected for offline map-bundle tooling.
    pub fn unpack_dir(&self) -> Option<&Path> {
        self.0.unpack_dir.as_deref()
    }

    /// Whether a revision was named.
    pub fn has_revision(&self) -> bool {
        self.0.revision.is_some()
    }

    /// Whether a cache directory was named.
    pub fn has_cache_dir(&self) -> bool {
        self.0.cache_dir.is_some()
    }

    /// Whether an unpack directory was named.
    pub fn has_unpack_dir(&self) -> bool {
        self.0.unpack_dir.is_some()
    }

    /// The host-play options (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn as_host(&self) -> &host_play::ProfileOptions {
        &self.0
    }
}

/// Split the server-profile flags out of `args`; the rest are returned for
/// the front end's own parser.
pub fn parse_profile_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(ProfileArgs, Vec<String>), ProfileError> {
    host_play::parse_profile_args(args)
        .map(|(options, rest)| (ProfileArgs(options), rest))
        .map_err(|error| ProfileError(ProfileFault::Arguments(error)))
}

/// The ambient inputs (environment, home) a profile resolves against,
/// captured once.
pub struct ProfileEnv(host_play::profile::ProfileEnvironment);

impl ProfileEnv {
    /// Capture the process environment now.
    pub fn capture() -> Self {
        Self(host_play::profile::ProfileEnvironment::capture())
    }

    /// An explicit environment (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_host(environment: host_play::profile::ProfileEnvironment) -> Self {
        Self(environment)
    }
}

/// Whether a profile targets a local development server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileClass {
    /// A local server: debug commands and teleports are allowed.
    Local,
    /// A remote server.
    Remote,
}

impl ProfileClass {
    pub(crate) fn from_host(class: host_play::ProfileClass) -> Self {
        match class {
            host_play::ProfileClass::Local => Self::Local,
            host_play::ProfileClass::Remote => Self::Remote,
        }
    }

    pub(crate) fn to_host(self) -> host_play::ProfileClass {
        match self {
            Self::Local => host_play::ProfileClass::Local,
            Self::Remote => host_play::ProfileClass::Remote,
        }
    }
}

/// The server profile a session is bound to, shared by everything that
/// reads it.
pub struct BoundProfile(Arc<host_play::ServerProfile>);

impl BoundProfile {
    pub(crate) fn host(&self) -> &Arc<host_play::ServerProfile> {
        &self.0
    }

    /// Another handle on the same profile.
    pub fn share(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    /// Whether `other` is this same binding.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// A stable identity for this binding while it lives.
    pub fn key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    /// The operator-facing label (`name · revision N`).
    pub fn label(&self) -> String {
        self.0.label()
    }

    /// The client revision number.
    pub fn revision(&self) -> i32 {
        self.0.revision().as_i32()
    }

    /// Local or remote.
    pub fn class(&self) -> ProfileClass {
        ProfileClass::from_host(self.0.profile_class())
    }

    /// Refuse bot operation on a profile that forbids it.
    pub fn require_bot_operation(&self) -> Result<(), ProfileError> {
        self.0
            .require_bot_operation()
            .map_err(|error| ProfileError(ProfileFault::BotOperationRefused(error)))
    }

    /// The vault this profile keeps its accounts in.
    pub fn vault_path(&self) -> &std::path::Path {
        self.0.vault_path()
    }

    /// The rs2b0t catalogue root this profile names, if any.
    pub fn catalog_root(&self) -> Option<&std::path::Path> {
        self.0.catalog_root()
    }

    /// The navigation world, once loaded.
    pub fn world(&self) -> Option<Arc<nav_planner::world::NavWorld>> {
        self.0.world()
    }

    /// Navigation pack identity for map bindings, if the profile published one.
    pub fn nav_identity(&self) -> Option<nav_planner::map::identity::Digest> {
        self.0.nav_identity().and_then(|identity| {
            nav_planner::map::identity::Digest::from_hex(&identity.nav_sha256).ok()
        })
    }

    /// Path to the profile's bound map-flags sidecar.
    pub fn nav_flags_path(&self) -> &std::path::Path {
        self.0.nav_flags()
    }

    /// Expected digest for the bound map-flags sidecar, if available.
    pub fn nav_flags_sha256(&self) -> Option<&str> {
        self.0
            .nav_identity()
            .and_then(|identity| identity.flags_sha256.as_deref())
    }

    /// Whether the sidecar is the build-stamped bundled resource.
    pub fn nav_flags_trusted_bundled(&self) -> bool {
        self.0.nav_flags_origin().is_bundled()
    }

    /// Lazy verified reach source for the profile.
    pub fn walk_reach_source(&self) -> Option<Arc<crate::walk_map::WalkReachSource>> {
        self.0
            .reach_source()
            .map(crate::walk_map::WalkReachSource::from_host)
            .map(Arc::new)
    }

    /// Whether this profile bound a verified nav identity, even if it has no
    /// reach sidecar. This prevents substituting a locally baked reach grid.
    pub fn has_verified_nav_identity(&self) -> bool {
        self.0.nav_identity().is_some()
    }

    /// The selected game data, once loaded.
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.0.game_data()
    }

    /// The opt-in debug command catalogue of this profile.
    #[cfg(feature = "debug-catalog")]
    pub fn debug_catalog(&self) -> Result<Arc<api::debug_commands::DebugCatalog>, ProfileError> {
        self.0
            .debug_catalog()
            .map_err(|error| ProfileError(ProfileFault::DebugCatalog(error)))
    }

    /// The world numbers of the profile's public world list, for profiles
    /// that publish one.
    pub fn public_world_numbers(&self) -> Option<impl ExactSizeIterator<Item = u16> + '_> {
        self.0
            .public_worlds()
            .map(|worlds| worlds.worlds.iter().map(|world| world.number))
    }
}

/// One stage of profile preparation, as the loading banner names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressStage {
    SelectingServerProfile,
    CheckingGameFiles,
    CheckingNavigationFiles,
    CheckingCache,
    PreparingCache,
    ReadingCacheArchives,
    LoadingGameData,
    PreparingNavigation,
    FinalChecks,
}

impl ProgressStage {
    fn from_host(stage: host_play::progress::ProfileProgressStage) -> Self {
        use host_play::progress::ProfileProgressStage as S;
        match stage {
            S::SelectingServerProfile => Self::SelectingServerProfile,
            S::CheckingGameFiles => Self::CheckingGameFiles,
            S::CheckingNavigationFiles => Self::CheckingNavigationFiles,
            S::CheckingCache => Self::CheckingCache,
            S::PreparingCache => Self::PreparingCache,
            S::ReadingCacheArchives => Self::ReadingCacheArchives,
            S::LoadingGameData => Self::LoadingGameData,
            S::PreparingNavigation => Self::PreparingNavigation,
            S::FinalChecks => Self::FinalChecks,
        }
    }

    /// The banner text for this stage.
    pub fn description(self) -> &'static str {
        match self {
            Self::SelectingServerProfile => "Selecting server profile",
            Self::CheckingGameFiles => "Checking game files",
            Self::CheckingNavigationFiles => "Verifying custom navigation",
            Self::CheckingCache => "Checking prepared cache",
            Self::PreparingCache => "Preparing cache snapshot",
            Self::ReadingCacheArchives => "Reading cache archives",
            Self::LoadingGameData => "Loading game data",
            Self::PreparingNavigation => "Loading navigation",
            Self::FinalChecks => "Final checks",
        }
    }
}

/// What a progress count counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressUnit {
    Bytes,
    Files,
    Steps,
}

/// The latest progress a preparation reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressView {
    stage: ProgressStage,
    completed: u64,
    total: u64,
    unit: ProgressUnit,
}

impl ProgressView {
    fn from_host(progress: host_play::progress::ProfileProgress) -> Self {
        use host_play::progress::ProfileProgressUnit as U;
        Self {
            stage: ProgressStage::from_host(progress.stage),
            completed: progress.completed,
            total: progress.total,
            unit: match progress.unit {
                U::Bytes => ProgressUnit::Bytes,
                U::Files => ProgressUnit::Files,
                U::Steps => ProgressUnit::Steps,
            },
        }
    }

    /// A step count in `stage`.
    pub fn steps(stage: ProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProgressUnit::Steps,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn to_host(self) -> host_play::progress::ProfileProgress {
        use host_play::progress::{ProfileProgress, ProfileProgressStage as S};
        let stage = match self.stage {
            ProgressStage::SelectingServerProfile => S::SelectingServerProfile,
            ProgressStage::CheckingGameFiles => S::CheckingGameFiles,
            ProgressStage::CheckingNavigationFiles => S::CheckingNavigationFiles,
            ProgressStage::CheckingCache => S::CheckingCache,
            ProgressStage::PreparingCache => S::PreparingCache,
            ProgressStage::ReadingCacheArchives => S::ReadingCacheArchives,
            ProgressStage::LoadingGameData => S::LoadingGameData,
            ProgressStage::PreparingNavigation => S::PreparingNavigation,
            ProgressStage::FinalChecks => S::FinalChecks,
        };
        match self.unit {
            ProgressUnit::Bytes => ProfileProgress::bytes(stage, self.completed, self.total),
            ProgressUnit::Files => ProfileProgress::files(stage, self.completed, self.total),
            ProgressUnit::Steps => ProfileProgress::steps(stage, self.completed, self.total),
        }
    }

    /// A byte count in `stage`.
    pub fn bytes(stage: ProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProgressUnit::Bytes,
        }
    }

    /// A file count in `stage`.
    pub fn files(stage: ProgressStage, completed: u64, total: u64) -> Self {
        Self {
            stage,
            completed,
            total,
            unit: ProgressUnit::Files,
        }
    }

    pub fn stage(&self) -> ProgressStage {
        self.stage
    }

    pub fn completed(&self) -> u64 {
        self.completed
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn unit(&self) -> ProgressUnit {
        self.unit
    }
}

type LatestProgress = Arc<std::sync::Mutex<Option<host_play::progress::ProfileProgress>>>;

fn progress_channel(
    initial: host_play::progress::ProfileProgress,
) -> (host_play::progress::ProfileProgressObserver, LatestProgress) {
    let latest = Arc::new(std::sync::Mutex::new(Some(initial)));
    let worker_latest = Arc::clone(&latest);
    let observer = host_play::progress::ProfileProgressObserver::new(move |progress| {
        if let Ok(mut latest) = worker_latest.try_lock() {
            *latest = Some(progress);
        }
    });
    (observer, latest)
}

fn latest_view(latest: &LatestProgress) -> Option<ProgressView> {
    latest
        .lock()
        .ok()?
        .as_ref()
        .copied()
        .map(ProgressView::from_host)
}

/// Why a preparation job was not submitted.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SubmitError {
    full: bool,
    message: String,
}

impl SubmitError {
    fn full() -> Self {
        Self {
            full: true,
            message: "background work is full".into(),
        }
    }

    fn refused(message: String) -> Self {
        Self {
            full: false,
            message,
        }
    }

    /// The shared worker is at capacity; submit again on a later frame.
    /// Otherwise the job was refused for good.
    pub fn is_full(&self) -> bool {
        self.full
    }
}

/// Why a server-profile operation failed: a fixed sentence, or the
/// operator-facing sentence the lower layer (`host-play`) reports. Callers
/// show it; the cause stays private.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(transparent)]
pub struct ProfileError(ProfileFault);

/// The cause of a [`ProfileError`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum ProfileFault {
    /// A profile is already bound; the revision cannot change.
    #[error("server profile is already bound; restart to change revision")]
    AlreadyBound,
    /// No production server-profile inputs were configured.
    #[error("no production server profile was configured")]
    NotConfigured,
    /// No profile environment was configured.
    #[error("no profile environment was configured")]
    NoEnvironment,
    /// No template has been prepared yet.
    #[error("server profile is not prepared")]
    NotPrepared,
    /// The session is not bound to a server profile.
    #[error("no bound server profile")]
    NotBound,
    /// The validated template belongs to another profile.
    #[error("validated template does not match the active server profile")]
    ValidationMismatch,
    /// The command-line profile inputs (flags, revision) are invalid.
    #[error("{0}")]
    Arguments(String),
    /// The configured inputs do not resolve to a server profile.
    #[error("{0}")]
    Resolve(String),
    /// Preparing or validating the client template failed.
    #[error("{0}")]
    Prepare(String),
    /// The shared preparation worker failed or lost the job.
    #[error("{0}")]
    Worker(String),
    /// The profile forbids bot operation.
    #[error("{0}")]
    BotOperationRefused(String),
    /// The profile's debug command catalogue is unavailable.
    #[cfg(feature = "debug-catalog")]
    #[error("{0}")]
    DebugCatalog(String),
}

impl ProfileError {
    /// The session is not bound to a server profile.
    pub fn not_bound() -> Self {
        Self(ProfileFault::NotBound)
    }

    /// A preparation that failed with `sentence` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare_failed_for_test(sentence: String) -> Self {
        Self(ProfileFault::Prepare(sentence))
    }

    /// Heap bytes the carried message owns.
    #[cfg(any(test, feature = "test-support"))]
    fn owned_bytes(&self) -> usize {
        match &self.0 {
            ProfileFault::Arguments(message)
            | ProfileFault::Resolve(message)
            | ProfileFault::Prepare(message)
            | ProfileFault::Worker(message)
            | ProfileFault::BotOperationRefused(message) => message.capacity(),
            #[cfg(feature = "debug-catalog")]
            ProfileFault::DebugCatalog(message) => message.capacity(),
            ProfileFault::AlreadyBound
            | ProfileFault::NotConfigured
            | ProfileFault::NoEnvironment
            | ProfileFault::NotPrepared
            | ProfileFault::NotBound
            | ProfileFault::ValidationMismatch => 0,
        }
    }
}

/// One capped process-profile reservation for captured inputs, decode
/// scratch, and the prepared shared-client template. Inline quest preparation
/// shares the profile's 32 MiB scratch and 32 MiB result caps. Its former
/// 8 MiB scratch / 2 MiB result charge is not reserved separately.
/// The combined peak is unmeasured.
const PROFILE_PREPARATION_INPUT_BYTES: usize = 64 * 1024 * 1024;
const PROFILE_PREPARATION_SCRATCH_BYTES: usize = 32 * 1024 * 1024;
const PROFILE_PREPARATION_RESULT_BYTES: usize = 32 * 1024 * 1024;

fn submit_error(error: script::preparation::PreparationError) -> SubmitError {
    match error {
        script::preparation::PreparationError::Refused(api::work::SubmissionRefusal::Full) => {
            SubmitError::full()
        }
        error => SubmitError::refused(error.to_string()),
    }
}

/// Owned inputs captured on the UI thread for one profile preparation.
struct PreparationInputs {
    options: host_play::ProfileOptions,
    environment: host_play::profile::ProfileEnvironment,
    saved_revision: u16,
}

/// A profile preparation waiting for the worker: the server profile is
/// resolved and its client template prepared, cooperatively, on the
/// WorkAdmission worker.
pub struct ProfilePreparation {
    generation: u64,
    inputs: Arc<PreparationInputs>,
    observer: Box<host_play::progress::ProfileProgressObserver>,
    latest: LatestProgress,
    /// Start at the real Load stage with embedded game data, without local assets.
    #[cfg(test)]
    load_fixture: Option<host_play::ProfileSelection>,
}

impl ProfilePreparation {
    /// The launcher generation these inputs were captured at.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The latest reported progress.
    pub fn progress(&self) -> Option<ProgressView> {
        latest_view(&self.latest)
    }

    /// Submit the preparation to the shared worker.
    pub fn submit(&self) -> Result<PreparationJob, SubmitError> {
        let input = script::preparation::PreparationInput::new(
            self.generation,
            PROFILE_PREPARATION_INPUT_BYTES,
            PROFILE_PREPARATION_SCRATCH_BYTES,
            PROFILE_PREPARATION_RESULT_BYTES,
        )
        .map_err(|error| SubmitError::refused(error.to_string()))?;
        let mut steps = PreparationSteps::new(Arc::clone(&self.inputs), (*self.observer).clone());
        #[cfg(test)]
        if let Some(selection) = &self.load_fixture {
            steps.template = Some(
                selection
                    .begin_template_load_for_test()
                    .map_err(SubmitError::refused)?,
            );
        }
        let task = script::preparation::submit_cooperative(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            move |worker| steps.step(worker),
        )
        .map_err(submit_error)?;
        Ok(PreparationJob {
            generation: self.generation,
            task,
            latest: Arc::clone(&self.latest),
        })
    }

    /// Submit the whole preparation and wait outside the worker (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn run_blocking(&self) -> Result<ProfileTemplate, ProfileError> {
        self.submit()
            .map_err(|error| ProfileError(ProfileFault::Worker(error.to_string())))?
            .join()
    }
}

/// A submitted profile preparation.
pub struct PreparationJob {
    generation: u64,
    task: script::preparation::PreparedTask<Result<Arc<SharedClientTemplate>, ProfileError>>,
    latest: LatestProgress,
}

impl PreparationJob {
    /// A job whose worker returns `result` at once (tests), reporting
    /// `progress` as its latest progress.
    #[cfg(any(test, feature = "test-support"))]
    pub fn settled_for_test(
        generation: u64,
        result: Result<ProfileTemplate, ProfileError>,
        progress: ProgressView,
    ) -> Self {
        let result = result.map(|template| template.0);
        let result_bytes = std::mem::size_of_val(&result)
            + match &result {
                Ok(_) => 0,
                Err(error) => error.owned_bytes(),
            };
        let input = script::preparation::PreparationInput::new(generation, 0, 0, result_bytes)
            .expect("a tiny preparation input");
        let task = script::preparation::submit(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            move |_| result,
        )
        .expect("the test worker admits one job");
        let (_, latest) = progress_channel(progress.to_host());
        Self {
            generation,
            task,
            latest,
        }
    }

    /// The launcher generation the job was captured at.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The latest reported progress.
    pub fn progress(&self) -> Option<ProgressView> {
        latest_view(&self.latest)
    }

    /// Whether the worker finished.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// The prepared template, or why preparation failed.
    pub fn join(self) -> Result<ProfileTemplate, ProfileError> {
        self.task
            .join()
            .map_err(|error| ProfileError(ProfileFault::Worker(error.to_string())))
            .and_then(|result| result)
            .map(ProfileTemplate)
    }
}

/// A final resource validation waiting for the worker.
pub struct ValidationPending {
    generation: u64,
    template: ProfileTemplate,
    observer: Box<host_play::progress::ProfileProgressObserver>,
    latest: LatestProgress,
}

impl ValidationPending {
    /// Validate `template` before play, captured at launcher `generation`.
    pub fn new(generation: u64, template: ProfileTemplate) -> Self {
        let (observer, latest) = progress_channel(host_play::progress::ProfileProgress::steps(
            host_play::progress::ProfileProgressStage::FinalChecks,
            0,
            1,
        ));
        Self {
            generation,
            template,
            observer: Box::new(observer),
            latest,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The latest reported progress.
    pub fn progress(&self) -> Option<ProgressView> {
        latest_view(&self.latest)
    }

    /// Submit the validation to the shared worker.
    pub fn submit(&self) -> Result<ValidationJob, SubmitError> {
        let input = script::preparation::PreparationInput::new(
            self.generation,
            std::mem::size_of::<Arc<SharedClientTemplate>>(),
            0,
            std::mem::size_of::<ValidatedTemplate>(),
        )
        .map_err(|error| SubmitError::refused(error.to_string()))?;
        let template = Arc::clone(&self.template.0);
        let observer = (*self.observer).clone();
        let task = script::preparation::submit(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            move |_| {
                template
                    .validate_for_play_with_progress(&observer)
                    .map_err(|error| ProfileError(ProfileFault::Prepare(error)))
            },
        )
        .map_err(submit_error)?;
        Ok(ValidationJob {
            generation: self.generation,
            task,
            latest: Arc::clone(&self.latest),
        })
    }
}

/// A submitted final validation.
pub struct ValidationJob {
    generation: u64,
    task: script::preparation::PreparedTask<Result<ValidatedTemplate, ProfileError>>,
    latest: LatestProgress,
}

impl ValidationJob {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The latest reported progress.
    pub fn progress(&self) -> Option<ProgressView> {
        latest_view(&self.latest)
    }

    /// Whether the worker finished.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// The validated template, or why validation failed.
    pub fn join(self) -> Result<ValidatedProfile, ProfileError> {
        self.task
            .join()
            .map_err(|error| ProfileError(ProfileFault::Worker(error.to_string())))
            .and_then(|result| result)
            .map(ValidatedProfile)
    }
}

/// One cooperative profile preparation on the WorkAdmission worker: the
/// first step resolves the server profile, later steps drive the template
/// preparation. No step waits on the network or the cache lock past the
/// template step wait, so a slow first-launch download never holds one
/// step long enough to look like a lost worker.
struct PreparationSteps {
    inputs: Arc<PreparationInputs>,
    observer: host_play::progress::ProfileProgressObserver,
    template: Option<host_play::TemplatePreparation>,
}

impl PreparationSteps {
    fn new(
        inputs: Arc<PreparationInputs>,
        observer: host_play::progress::ProfileProgressObserver,
    ) -> Self {
        Self {
            inputs,
            observer,
            template: None,
        }
    }

    fn step(
        &mut self,
        worker: &mut api::selected::FamilyPreparation,
    ) -> script::preparation::CooperativePreparationStep<
        Result<Arc<SharedClientTemplate>, ProfileError>,
    > {
        use host_play::progress::{ProfileProgress, ProfileProgressStage};
        use script::preparation::CooperativePreparationStep as Step;
        let Some(template) = self.template.as_mut() else {
            self.observer.report(ProfileProgress::steps(
                ProfileProgressStage::SelectingServerProfile,
                0,
                1,
            ));
            let inputs = &self.inputs;
            return match inputs
                .options
                .resolve_with_env(Some(inputs.saved_revision), &inputs.environment)
            {
                Ok(selection) => {
                    self.observer.report(ProfileProgress::steps(
                        ProfileProgressStage::SelectingServerProfile,
                        1,
                        1,
                    ));
                    self.template = Some(selection.begin_template_preparation());
                    Step::Continue {
                        completed: 0,
                        total: None,
                    }
                }
                Err(error) => Step::Complete {
                    output: Err(ProfileError(ProfileFault::Resolve(error))),
                    completed: 0,
                    total: None,
                },
            };
        };
        match template.step_on_worker(
            std::time::Instant::now() + host_play::TEMPLATE_STEP_WAIT,
            &self.observer,
            worker,
        ) {
            host_play::TemplateStep::Continue { completed, total } => {
                Step::Continue { completed, total }
            }
            host_play::TemplateStep::Complete(output) => Step::Complete {
                output: output.map_err(|error| ProfileError(ProfileFault::Prepare(error))),
                completed: 1,
                total: Some(1),
            },
        }
    }
}

/// How a finished preparation landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationCompletion {
    /// The template is installed.
    Installed,
    /// The inputs changed while it ran; the result was dropped.
    Stale,
    /// Preparation failed (the error is returned beside this).
    Failed,
}

/// A session's server-profile binding: the launch inputs, the prepared
/// template and its validation, and the connection settings derived from
/// the bound profile.
pub struct ProfileLauncher {
    play: PlayConfig,
    bound: Option<BoundProfile>,
    template: Option<ProfileTemplate>,
    args: Option<ProfileArgs>,
    environment: Option<ProfileEnv>,
    /// Moves on every change of the inputs; an older worker completion is
    /// stale.
    generation: u64,
    preparing: bool,
    validated: Option<ValidatedProfile>,
}

impl ProfileLauncher {
    /// A launcher for the legacy local 274 server (no profile inputs).
    pub fn legacy_local() -> Self {
        Self {
            play: PlayConfig(Box::new(PlayOptions {
                host: "127.0.0.1".into(),
                transport: host_play::Transport::Tcp,
                port: 43594,
                cache_dir: String::new(),
                lowmem: true,
                mainland: false,
            })),
            bound: None,
            template: None,
            args: None,
            environment: None,
            generation: 0,
            preparing: false,
            validated: None,
        }
    }

    /// Install production launch inputs. Resolution waits for the first
    /// preparation.
    pub fn configure(
        &mut self,
        args: ProfileArgs,
        environment: ProfileEnv,
    ) -> Result<(), ProfileError> {
        if self.bound.is_some() {
            return Err(ProfileError(ProfileFault::AlreadyBound));
        }
        self.args = Some(args);
        self.environment = Some(environment);
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Whether production inputs are configured.
    pub fn configured(&self) -> bool {
        self.args.is_some()
    }

    /// The input generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Invalidate in-flight preparation (the saved revision changed).
    pub fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Capture a preparation of the configured inputs at `saved_revision`.
    pub fn preparation(&self, saved_revision: u16) -> Result<ProfilePreparation, ProfileError> {
        let (args, environment) = self.inputs()?;
        let options = args.0.clone();
        let environment = environment.0.clone();
        let (observer, latest) = progress_channel(host_play::progress::ProfileProgress::steps(
            host_play::progress::ProfileProgressStage::SelectingServerProfile,
            0,
            1,
        ));
        Ok(ProfilePreparation {
            generation: self.generation,
            inputs: Arc::new(PreparationInputs {
                options,
                environment,
                saved_revision,
            }),
            observer: Box::new(observer),
            latest,
            #[cfg(test)]
            load_fixture: None,
        })
    }

    /// Land a finished preparation. `Installed` binds the profile and
    /// derives the connection settings from it.
    pub fn finish_preparation(
        &mut self,
        generation: u64,
        result: Result<ProfileTemplate, ProfileError>,
    ) -> (PreparationCompletion, Option<ProfileError>) {
        self.preparing = false;
        if generation != self.generation {
            return (PreparationCompletion::Stale, None);
        }
        match result {
            Ok(template) => {
                self.install(template);
                (PreparationCompletion::Installed, None)
            }
            Err(error) => (PreparationCompletion::Failed, Some(error)),
        }
    }

    /// Bind `template`'s profile directly (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_template(&mut self, template: ProfileTemplate) {
        self.install(template);
    }

    fn install(&mut self, template: ProfileTemplate) {
        let profile = Arc::clone(template.0.profile());
        self.play = PlayConfig(Box::new(PlayOptions {
            host: profile.client().game_host().to_string(),
            transport: profile.transport(),
            port: profile.client().game_port(),
            cache_dir: profile.client().cache_dir().display().to_string(),
            lowmem: true,
            mainland: false,
        }));
        self.bound = Some(BoundProfile(profile));
        self.template = Some(template);
    }

    /// Resolve and prepare the profile on this thread at `saved_revision`,
    /// then bind it.
    pub fn bind(&mut self, saved_revision: u16) -> Result<(), ProfileError> {
        self.bind_with(Some(saved_revision))
    }

    /// Bind as [`Self::bind`] does, with no saved revision to fall back on
    /// (the profile's own or default revision applies).
    pub fn bind_with(&mut self, saved_revision: Option<u16>) -> Result<(), ProfileError> {
        if self.bound.is_some() {
            return Err(ProfileError(ProfileFault::AlreadyBound));
        }
        let (options, environment) = self.inputs()?;
        let template = options
            .0
            .resolve_with_env(saved_revision, &environment.0)
            .map_err(|error| ProfileError(ProfileFault::Resolve(error)))?
            .prepare_template()
            .map_err(|error| ProfileError(ProfileFault::Prepare(error)))?;
        self.install(ProfileTemplate(template));
        Ok(())
    }

    /// The prepared template, for final validation.
    pub fn template_for_validation(&self) -> Result<ProfileTemplate, ProfileError> {
        self.template
            .as_ref()
            .map(ProfileTemplate::share)
            .ok_or(ProfileError(ProfileFault::NotPrepared))
    }

    /// Keep a validation of the active template for the next launch.
    pub fn install_validated(&mut self, validated: ValidatedProfile) -> Result<(), ProfileError> {
        let Some(template) = self.template.as_ref() else {
            return Err(ProfileError(ProfileFault::NotPrepared));
        };
        if !validated.is_for(template) {
            return Err(ProfileError(ProfileFault::ValidationMismatch));
        }
        self.validated = Some(validated);
        Ok(())
    }

    pub fn set_preparing(&mut self, preparing: bool) {
        self.preparing = preparing;
    }

    pub fn preparing(&self) -> bool {
        self.preparing
    }

    /// The bound profile.
    pub fn bound(&self) -> Option<&BoundProfile> {
        self.bound.as_ref()
    }

    /// The prepared template.
    pub fn template(&self) -> Option<&ProfileTemplate> {
        self.template.as_ref()
    }

    /// The connection settings play launches with.
    pub fn play_config(&self) -> &PlayConfig {
        &self.play
    }

    /// Point the legacy connection at `host` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_play_host(&mut self, host: &str) {
        self.play.0.host = host.to_string();
    }

    /// The configured production inputs, or why they are missing.
    fn inputs(&self) -> Result<(&ProfileArgs, &ProfileEnv), ProfileError> {
        let args = self
            .args
            .as_ref()
            .ok_or(ProfileError(ProfileFault::NotConfigured))?;
        let environment = self
            .environment
            .as_ref()
            .ok_or(ProfileError(ProfileFault::NoEnvironment))?;
        Ok((args, environment))
    }

    /// Resolve the configured inputs at `saved_revision` without binding.
    fn resolve(&self, saved_revision: u16) -> Result<host_play::ProfileSelection, ProfileError> {
        let (options, environment) = self.inputs()?;
        options
            .0
            .resolve_with_env(Some(saved_revision), &environment.0)
            .map_err(|error| ProfileError(ProfileFault::Resolve(error)))
    }

    /// The server label: the bound profile's, else the configured one at
    /// `saved_revision`, else the legacy local label.
    pub fn server_label(&self, saved_revision: u16) -> String {
        if let Some(profile) = &self.bound {
            return profile.label();
        }
        if self.args.is_none() {
            return "legacy local-274 · revision 274".into();
        }
        self.resolve(saved_revision).map_or_else(
            |error| format!("invalid profile: {error}"),
            |profile| profile.label(),
        )
    }

    /// The effective client revision label.
    pub fn revision_label(&self, saved_revision: u16) -> Result<String, ProfileError> {
        if let Some(profile) = &self.bound {
            return Ok(profile.revision().to_string());
        }
        match self.args.as_ref() {
            Some(_) => self
                .resolve(saved_revision)
                .map(|selection| selection.revision().as_i32().to_string()),
            None => Ok(saved_revision.to_string()),
        }
    }

    /// Local or remote: the bound profile's class, else the legacy play
    /// connection's when `playing`, else remote.
    pub fn class(&self, playing: bool) -> ProfileClass {
        if let Some(profile) = &self.bound {
            profile.class()
        } else if playing {
            ProfileClass::from_host(host_play::profile::profile_class(
                self.play.0.transport,
                [self.play.0.host.as_str()],
            ))
        } else {
            ProfileClass::Remote
        }
    }

    /// The class of the profile the configured inputs resolve to at
    /// `saved_revision` (before it is bound).
    pub fn configured_class(&self, saved_revision: u16) -> Result<ProfileClass, ProfileError> {
        self.resolve(saved_revision)
            .map(|selection| ProfileClass::from_host(selection.profile_class()))
    }

    /// The vault path: the bound profile's, else the configured one at
    /// `saved_revision`, else `legacy`.
    pub fn vault_path(
        &self,
        saved_revision: u16,
        legacy: std::path::PathBuf,
    ) -> Result<std::path::PathBuf, ProfileError> {
        if let Some(profile) = &self.bound {
            return Ok(profile.vault_path().to_path_buf());
        }
        match self.args.as_ref() {
            Some(_) => self
                .resolve(saved_revision)
                .map(|selection| selection.vault_path().to_path_buf()),
            None => Ok(legacy),
        }
    }

    /// The vault path the configured profile will bind with, resolved as
    /// [`Self::bind_with`] resolves it (`saved_revision` as given), without
    /// binding: no engine key is read and no cache is prepared.
    pub fn unbound_vault_path(
        &self,
        saved_revision: Option<u16>,
    ) -> Result<std::path::PathBuf, ProfileError> {
        if let Some(profile) = &self.bound {
            return Ok(profile.vault_path().to_path_buf());
        }
        let (options, environment) = self.inputs()?;
        options
            .0
            .resolve_with_env(saved_revision, &environment.0)
            .map(|selection| selection.vault_path().to_path_buf())
            .map_err(|error| ProfileError(ProfileFault::Resolve(error)))
    }

    /// The catalogue root: the bound profile's, else the configured one at
    /// `saved_revision`.
    pub fn catalog_root(
        &self,
        saved_revision: u16,
    ) -> Result<Option<std::path::PathBuf>, ProfileError> {
        if let Some(profile) = &self.bound {
            return Ok(profile.catalog_root().map(std::path::Path::to_path_buf));
        }
        self.resolve(saved_revision)
            .map(|selection| selection.catalog_root().map(std::path::Path::to_path_buf))
    }

    /// The launch for the next play session: the validated template when
    /// one is kept (consumed), else the prepared template, else the legacy
    /// connection.
    pub fn take_launch(&mut self) -> PlayLaunch {
        let mainland = self.play.0.mainland;
        match self.validated.take() {
            Some(validated) => PlayLaunch::validated(validated, mainland),
            None => match self.template.as_ref() {
                Some(template) => PlayLaunch::template(template, mainland),
                None => PlayLaunch::direct(&self.play),
            },
        }
    }
}

/// Whether WalkTo teleports are authorized for `class` against `host`
/// before any play session runs.
pub fn debug_teleport_authorized(class: ProfileClass, host: &str) -> bool {
    host_play::walk_map::debug_teleport_authorized(class.to_host(), host)
}

/// A vault passphrase as typed: the buffer is zeroed on drop and by
/// [`Passphrase::clear`].
#[derive(Default)]
pub struct Passphrase(vault::Secret);

impl Passphrase {
    /// An empty passphrase with room for `capacity` bytes, so typing into it
    /// does not reallocate (leaving an unzeroed copy behind).
    pub fn with_capacity(capacity: usize) -> Self {
        Self(vault::Secret::with_capacity(capacity))
    }

    /// The passphrase text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Zero and empty the buffer.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn from_secret(secret: vault::Secret) -> Self {
        Self(secret)
    }
}

impl From<&str> for Passphrase {
    fn from(value: &str) -> Self {
        Self(vault::Secret::from(value))
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(..)")
    }
}

impl std::ops::Deref for Passphrase {
    type Target = String;

    fn deref(&self) -> &String {
        &self.0
    }
}

impl std::ops::DerefMut for Passphrase {
    fn deref_mut(&mut self) -> &mut String {
        &mut self.0
    }
}

/// Which step of an unlock failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockStage {
    /// Opening (or first-run creating) the vault.
    Vault,
    /// Starting the play session.
    Launch,
}

/// Why an unlock failed.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct UnlockError {
    stage: UnlockStage,
    message: String,
}

impl UnlockError {
    pub(crate) fn new(stage: UnlockStage, message: String) -> Self {
        Self { stage, message }
    }

    /// The step that failed.
    pub fn stage(&self) -> UnlockStage {
        self.stage
    }
}

/// Open the vault at `path` with `typed` exactly as typed, creating it on
/// first run. A vault the 0.1.9.1 panel made from input with surrounding
/// spaces only opens with the trimmed text, so a wrong-passphrase failure
/// retries once with the trimmed text.
pub(crate) fn open_vault_typed(
    path: &std::path::Path,
    typed: &str,
) -> Result<vault::Vault, vault::VaultError> {
    match host_play::open_vault(path, typed) {
        Err(vault::VaultError::WrongPassphrase) if typed.trim() != typed => {
            host_play::open_vault(path, typed.trim())
        }
        other => other,
    }
}

/// Delete the vault file at `path` (forgotten-passphrase recovery). A
/// missing file is not an error.
pub(crate) fn reset_vault_file(path: &std::path::Path) -> Result<(), vault::VaultError> {
    vault::Vault::reset_file(path)
}

/// Whether `arg` is a removed passphrase flag.
pub fn is_removed_passphrase_flag(arg: &str) -> bool {
    host_play::passphrase::is_removed_flag(arg)
}

/// The error a removed passphrase flag reports for `program`.
pub fn removed_passphrase_flag_error(program: &str) -> String {
    host_play::passphrase::removed_flag_error(program)
}

/// Why reading the vault passphrase failed: it could not be asked for or
/// read (no terminal, closed or unreadable input, a refused new
/// passphrase). The sentence is the one `host-play` reports.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PassphraseError(String);

/// Read the vault passphrase for `program`: prompted on a terminal, else
/// read from stdin when `from_stdin`. `vault_exists` chooses the unlock or
/// the create prompt.
pub fn obtain_passphrase(
    program: &str,
    from_stdin: bool,
    vault_exists: bool,
) -> Result<Passphrase, PassphraseError> {
    host_play::passphrase::obtain(
        program,
        from_stdin,
        host_play::passphrase::Purpose::for_vault(vault_exists),
    )
    .map(Passphrase::from_secret)
    .map_err(PassphraseError)
}

/// Warn once on stderr when the legacy passphrase environment variable is
/// set (it is ignored).
pub fn warn_legacy_passphrase_env(program: &str) {
    host_play::passphrase::warn_legacy_env(program);
}

/// Which front end holds the single-instance lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    Panel,
    Tui,
}

impl InstanceKind {
    fn to_host(self) -> host_play::InstanceKind {
        match self {
            Self::Panel => host_play::InstanceKind::Panel,
            Self::Tui => host_play::InstanceKind::Tui,
        }
    }
}

/// Permission to open the operator's preferences and vault: the held
/// single-instance lock, or an explicit decision to run without it.
pub struct InstancePermit(host_play::InstancePermit);

impl InstancePermit {
    /// Run without the lock (harness boots, a confirmed second instance).
    pub fn skip() -> Self {
        Self(host_play::InstancePermit::skip())
    }

    pub(crate) fn into_host(self) -> host_play::InstancePermit {
        self.0
    }
}

/// Another running 274bot holds the instance lock.
pub struct InstanceConflict(host_play::InstanceHolder);

impl InstanceConflict {
    /// The operator-facing explanation (who holds the lock and where).
    pub fn message(&self) -> String {
        host_play::instance_conflict_message(&self.0)
    }
}

/// Take the single-instance lock for `kind` (or skip it when `skip`).
/// `Ok(Err(conflict))` means another instance holds it and the operator
/// must decide; `Err` is an I/O failure taking the lock.
pub fn acquire_instance(
    kind: InstanceKind,
    skip: bool,
) -> std::io::Result<Result<InstancePermit, InstanceConflict>> {
    Ok(
        match host_play::resolve_instance_permit(kind.to_host(), skip)? {
            host_play::InstancePermitOutcome::Ready(permit) => Ok(InstancePermit(permit)),
            host_play::InstancePermitOutcome::NeedsConfirm(holder) => Err(InstanceConflict(holder)),
        },
    )
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;

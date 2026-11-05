use super::*;
use api::host_log;
use api::hostlog::{Category, Level};
use std::io::{BufRead, BufReader};

fn open_nav_file(path: &Path) -> std::io::Result<(std::fs::File, usize)> {
    let file = std::fs::File::open(path)?;
    let length = usize::try_from(file.metadata()?.len())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok((file, length))
}

// Login protocol CRC slots, in the client's archive order.
const JAGS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

struct LoadedNav {
    availability: NavAvailability,
    world: Option<Arc<NavWorld>>,
    reach: Option<Arc<DeferredReach>>,
    canlight: Option<Arc<[u64]>>,
    counters: NavLoadCounters,
}

#[cfg(feature = "debug-catalog")]
impl ServerProfile {
    /// Decode on Debug-tab demand. Closing the tab releases its heap.
    pub fn debug_catalog(&self) -> Result<Arc<api::debug_commands::DebugCatalog>, String> {
        if self.profile_class() != ProfileClass::Local {
            return Err("Debug commands require a Local profile".into());
        }
        let data = self.game_data().ok_or_else(|| {
            format!(
                "Debug catalog unavailable: {}",
                self.game_data_status().detail()
            )
        })?;
        let catalog = api::debug_commands::DebugCatalog::load(self.revision(), data)?;
        Ok(Arc::new(catalog))
    }
}

impl ProfileSelection {
    pub fn selection(&self) -> &LaunchProfile {
        &self.profile
    }
    pub fn name(&self) -> &str {
        &self.profile.name
    }
    pub fn revision(&self) -> ClientRevision {
        parse_revision(&self.profile.revision.to_string()).expect("validated profile revision")
    }
    pub fn transport(&self) -> Transport {
        self.profile.client_transport()
    }
    pub fn profile_class(&self) -> ProfileClass {
        self.class
    }
    pub fn game_host(&self) -> &str {
        &self.game_host
    }
    pub fn game_port(&self) -> u16 {
        self.game_port
    }
    pub fn asset_host(&self) -> &str {
        &self.asset_host
    }
    pub fn asset_port(&self) -> u16 {
        self.asset_port
    }
    pub fn public_worlds(&self) -> Option<&Arc<PublicWorlds>> {
        self.public_worlds.as_ref()
    }
    /// Engine install selected by the read-only native profile resolver.
    pub fn engine_dir(&self) -> &Path {
        &self.engine_dir
    }
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
    pub fn unpack_dir(&self) -> &Path {
        &self.unpack_dir
    }
    pub fn nav_pack(&self) -> &Path {
        &self.nav_pack
    }
    pub fn nav_flags(&self) -> &Path {
        &self.nav_flags
    }
    pub fn content_dir(&self) -> &Path {
        &self.content_dir
    }
    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }
    pub fn catalog_root(&self) -> Option<&Path> {
        self.catalog_root.as_deref()
    }
    pub fn world_members(&self) -> &WorldMembersFact {
        &self.world_members
    }
    pub fn map_members(&self) -> bool {
        self.world_members.map_members()
    }
    /// Bind may attach generated facts only when this is true; cache identity
    /// still has to match, and local worlds also require source-file hashes.
    pub fn supported_server(&self) -> bool {
        self.supported_server
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {}:{} · revision {}",
            self.profile.name,
            self.game_host,
            self.game_port,
            self.revision().as_i32()
        )
    }
    pub fn require_bot_operation(&self) -> Result<(), String> {
        require_bot_operation(self.revision())
    }

    /// Validate and freeze identities without network traffic or filesystem writes.
    pub fn bind(&self) -> Result<Arc<ServerProfile>, String> {
        self.bind_with_progress(&ProfileProgressObserver::default())
    }

    pub fn bind_with_progress(
        &self,
        observer: &ProfileProgressObserver,
    ) -> Result<Arc<ServerProfile>, String> {
        self.bind_with_resource_root(
            observer,
            std::env::current_exe()
                .ok()
                .map(|exe| install_resource_root(&exe))
                .as_deref(),
        )
    }

    /// Bind with an explicit install resource root.
    pub fn bind_with_resource_root(
        &self,
        observer: &ProfileProgressObserver,
        resource_root: Option<&Path>,
    ) -> Result<Arc<ServerProfile>, String> {
        self.bind_inner(observer, resource_root, false, &mut None)
    }

    /// Negotiate the selected endpoint before freezing a complete owned cache.
    /// Unlike `bind`, this is the application preparation path and may use the
    /// selected update server. All bots must share the resulting profile.
    pub fn bind_runtime(&self) -> Result<Arc<ServerProfile>, String> {
        self.bind_runtime_with_progress(&ProfileProgressObserver::default())
    }

    pub fn bind_runtime_with_progress(
        &self,
        observer: &ProfileProgressObserver,
    ) -> Result<Arc<ServerProfile>, String> {
        let mut fallback = WorldFallback::new(self);
        let mut selected = self.clone();
        loop {
            let mut fill_error = None;
            let error = match selected.bind_inner(
                observer,
                std::env::current_exe()
                    .ok()
                    .map(|exe| install_resource_root(&exe))
                    .as_deref(),
                true,
                &mut fill_error,
            ) {
                Ok(profile) => return Ok(profile),
                Err(error) => error,
            };
            if !fill_error.is_some_and(|fill_error| fallback.next(&mut selected, &fill_error)) {
                return Err(error);
            }
        }
    }

    fn bind_inner(
        &self,
        observer: &ProfileProgressObserver,
        resource_root: Option<&Path>,
        runtime: bool,
        fill_error: &mut Option<client::unpack::RuntimeCacheError>,
    ) -> Result<Arc<ServerProfile>, String> {
        let runtime_cache = if runtime {
            observer.report(ProfileProgress::steps(
                ProfileProgressStage::PreparingCache,
                0,
                1,
            ));
            let prepared = client::unpack::prepare_runtime_cache(&self.runtime_cache_request())
                .map_err(|error| {
                    let message = error.to_string();
                    *fill_error = Some(error);
                    message
                })?;
            observer.report(ProfileProgress::steps(
                ProfileProgressStage::PreparingCache,
                1,
                1,
            ));
            Some(prepared)
        } else {
            None
        };
        self.bind_with_cache(observer, resource_root, runtime, runtime_cache)
    }

    fn runtime_cache_request(&self) -> client::unpack::RuntimeCacheRequest<'_> {
        client::unpack::RuntimeCacheRequest {
            revision: self.revision(),
            transport: self.transport(),
            jag_source: &self.cache_dir,
            snapshot_root: &self.unpack_dir,
            asset_host: &self.asset_host,
            asset_port: self.asset_port,
            game_host: &self.game_host,
            game_port: self.game_port,
        }
    }

    /// Read archives and configure login, navigation, facts and the frozen client binding.
    fn bind_with_cache(
        &self,
        observer: &ProfileProgressObserver,
        resource_root: Option<&Path>,
        runtime: bool,
        runtime_cache: Option<Arc<client::unpack::PreparedRuntimeCache>>,
    ) -> Result<Arc<ServerProfile>, String> {
        let cache_dir = runtime_cache
            .as_ref()
            .map_or(self.cache_dir.as_path(), |p| p.jag_dir.as_path());
        let unpack_dir = runtime_cache
            .as_ref()
            .map_or(self.unpack_dir.as_path(), |p| p.unpack_root());
        let cache_preparation = if runtime_cache.is_none() {
            Some(crate::cache::prepare(
                cache_dir,
                unpack_dir,
                self.transport(),
                &self.asset_host,
                self.asset_port,
                observer,
            ))
        } else {
            None
        };
        let mut cache_version = None;
        let mut crcs = [0; 9];
        let archive_total = JAGS.len() as u64;
        observer.report(ProfileProgress::files(
            ProfileProgressStage::CheckingGameFiles,
            0,
            archive_total,
        ));
        observer.report(ProfileProgress::files(
            ProfileProgressStage::ReadingCacheArchives,
            0,
            archive_total,
        ));
        for (index, name) in JAGS.iter().enumerate() {
            let bytes = std::fs::read(cache_dir.join(name)).map_err(|error| {
                let archive_error = format!("cache {name}: {error}");
                match cache_preparation.as_ref().map(|prep| &prep.availability) {
                    Some(crate::cache::CacheAvailability::Degraded(reason)) => {
                        format!("{archive_error}; cache preparation degraded: {reason}")
                    }
                    _ => archive_error,
                }
            })?;
            if runtime_cache.is_none() && *name == "versionlist" {
                cache_version = Some(client::unpack::version_hash(&bytes));
            }
            crcs[index + 1] = Packet::getcrc(&bytes, 0, bytes.len());
            let completed = index as u64 + 1;
            observer.report(ProfileProgress::files(
                ProfileProgressStage::CheckingGameFiles,
                completed,
                archive_total,
            ));
            observer.report(ProfileProgress::files(
                ProfileProgressStage::ReadingCacheArchives,
                completed,
                archive_total,
            ));
        }
        // Runtime map capabilities use the client's prepared resource key.
        // Offline bindings use its existing versionlist namespace instead.
        let cache_id = runtime_cache.as_ref().map_or_else(
            || cache_version.expect("versionlist is a required archive"),
            |prepared| prepared.identity.content_id_hex(),
        );
        let (rsa_modulus, rsa_exponent) = if let Some(modulus) = &self.rsa_modulus {
            (
                modulus.clone(),
                self.rsa_exponent
                    .clone()
                    .unwrap_or_else(|| client::JAVA_LOGIN_RSAE.into()),
            )
        } else if self.rsa_exponent.is_some() {
            return Err("LOGIN_RSAE requires LOGIN_RSAN on an explicit server profile".into());
        } else {
            match &self.profile.login_key {
                LoginKey::Named(_) => (
                    client::PROD_LOGIN_RSAN.into(),
                    client::PROD_LOGIN_RSAE.into(),
                ),
                LoginKey::Inline { modulus, exponent } => (modulus.clone(), exponent.clone()),
                LoginKey::EngineDir { .. } => {
                    let pem = self.engine_dir.join("data/config/private.pem");
                    client::login_rsa::rsa_from_pkcs1_pem_file(&pem).map_err(|error| {
                        format!(
                            "profile RSA {}: {error}; configure the selected profile or LOGIN_RSAN/E",
                            pem.display()
                        )
                    })?
                }
            }
        };
        let nav_pack = if self.nav_pack_overridden {
            self.nav_pack.clone()
        } else {
            resource_root
                .map(|root| {
                    root.join(artifact_layout(self.revision().as_i32() as u16).relative_pack)
                })
                .filter(|path| path.is_file())
                .unwrap_or_else(|| self.nav_pack.clone())
        };
        let nav_flags = if self.nav_flags_overridden {
            self.nav_flags.clone()
        } else {
            nav_pack.with_extension("navflags")
        };
        let loaded = self.load_nav(&nav_pack, observer)?;
        if runtime && loaded.world.is_some() {
            eprintln!("host-play: navigation source: {}", nav_pack.display());
        }
        let availability = if let Some(p) = &runtime_cache {
            CacheAvailability::Ready {
                version: p.version.clone(),
                published: true,
                source: if p.source == "update-server" {
                    "update-server"
                } else {
                    "local-store"
                },
            }
        } else {
            cache_preparation
                .expect("non-runtime cache preparation")
                .availability
        };
        let binding = Arc::new(ClientSessionProfile::new(ClientSessionConfig {
            revision: self.revision(),
            transport: self.transport(),
            game_host: self.game_host.clone(),
            game_port: self.game_port,
            asset_host: self.asset_host.clone(),
            asset_port: self.asset_port,
            cache_dir: cache_dir.to_path_buf(),
            unpack_dir: unpack_dir.to_path_buf(),
            rsa_modulus,
            rsa_exponent,
            expected_crc: Some(crcs),
            content_id: cache_id.clone(),
            file_store_dir: runtime_cache.as_ref().and_then(|p| p.store_dir.clone()),
            ondemand_persist_dir: runtime_cache.as_ref().map(|p| p.persist_dir.clone()),
            map_archive: runtime_cache.as_ref().map(|p| Arc::clone(&p.map_archive)),
        })?);
        let (game_data, game_data_status) = self.attach_game_data(runtime)?;
        if runtime && !game_data_status.is_attached() {
            host_log!(
                stderr;
                Category::Lifecycle,
                Level::Warn,
                "generated game data unavailable ({}): {}",
                game_data_status.reason(),
                game_data_status.detail()
            );
        }
        Ok(Arc::new(ServerProfile {
            profile: self.profile.clone(),
            class: self.class,
            client: binding,
            cache: availability,
            runtime_cache,
            game_data,
            game_data_status,
            nav_pack,
            nav_flags,
            nav_load: loaded.counters,
            world: SharedWorld(loaded.world),
            reach: loaded.reach,
            canlight: loaded.canlight,
            nav: loaded.availability,
            content_dir: self.content_dir.clone(),
            vault_path: self.vault_path.clone(),
            state_root: self.state_root.clone(),
            catalog_root: self.catalog_root.clone(),
            world_members: self.world_members.clone(),
            public_worlds: self.public_worlds.clone(),
        }))
    }

    /// Attach the selected revision's facts for a supported server.
    fn attach_game_data(
        &self,
        runtime: bool,
    ) -> Result<
        (
            Option<Arc<api::game_data::SelectedGameData>>,
            GameDataStatus,
        ),
        String,
    > {
        let public = self.transport() == Transport::Wss;
        if !(self.supported_server || (runtime && public)) {
            return Ok((
                None,
                GameDataStatus::Unsupported {
                    reason: self.unsupported_reason(),
                },
            ));
        }
        Ok((
            Some(api::game_data::for_revision(self.revision())?),
            GameDataStatus::Attached,
        ))
    }

    /// Which supported-server condition this selection failed. Only called
    /// when `supported_server` is false.
    fn unsupported_reason(&self) -> String {
        let endpoints = format!(
            "{}:{} / {}:{}",
            self.game_host, self.game_port, self.asset_host, self.asset_port
        );
        if self.transport() == Transport::Wss {
            return format!("public endpoint {endpoints} is not a bundled rs2b2t world");
        }
        if !(crate::is_loopback_host(&self.game_host) && crate::is_loopback_host(&self.asset_host))
        {
            return format!("endpoint {endpoints} is not loopback");
        }
        let world_json = self.engine_dir.join("data/config/world.json");
        match &self.world_members {
            WorldMembersFact::Known {
                source: WorldMembersSource::LocalWorldJson { .. },
                ..
            } => format!("local endpoint {endpoints} did not qualify"),
            WorldMembersFact::Known { source, .. } => format!(
                "explicit endpoint flags need the guarded local world.json ({}), but world membership came from {source:?}",
                world_json.display()
            ),
            WorldMembersFact::Unknown => format!(
                "explicit endpoint flags need a guarded local world.json (revision {}, boolean node.members) at {}; none usable",
                self.revision().as_i32(),
                world_json.display()
            ),
        }
    }

    /// Bind the immutable profile and decode the shared process resources.
    pub fn prepare_template(&self) -> Result<Arc<crate::SharedClientTemplate>, String> {
        self.prepare_template_with_progress(&ProfileProgressObserver::default())
    }

    pub fn prepare_template_with_progress(
        &self,
        observer: &ProfileProgressObserver,
    ) -> Result<Arc<crate::SharedClientTemplate>, String> {
        crate::SharedClientTemplate::load_with_progress(
            self.bind_runtime_with_progress(observer)?,
            observer,
        )
    }

    /// [`Self::prepare_template_with_progress`] in bounded steps for the
    /// WorkAdmission worker. See [`TemplatePreparation`].
    pub fn begin_template_preparation(&self) -> TemplatePreparation {
        TemplatePreparation {
            fallback: WorldFallback::new(self),
            selected: self.clone(),
            last_total: None,
            stage: TemplateStage::Start,
        }
    }

    /// Start at Load with this revision's embedded game data (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn begin_template_load_for_test(&self) -> Result<TemplatePreparation, String> {
        let mut profile = self.bind()?;
        Arc::get_mut(&mut profile)
            .expect("the fixture profile is not shared yet")
            .game_data = Some(api::game_data::for_revision(self.revision())?);
        let mut preparation = self.begin_template_preparation();
        preparation.stage = TemplateStage::Load(profile);
        Ok(preparation)
    }

    fn load_nav(
        &self,
        pack_path: &Path,
        observer: &ProfileProgressObserver,
    ) -> Result<LoadedNav, String> {
        let mut counters = NavLoadCounters::default();
        if !pack_path.exists() {
            return Ok(LoadedNav {
                availability: NavAvailability::Unavailable(format!(
                    "navigation unavailable: {} has not been prepared",
                    pack_path.display()
                )),
                world: None,
                reach: None,
                canlight: None,
                counters,
            });
        }
        let (file, length) = open_nav_file(pack_path)
            .map_err(|error| format!("navigation {}: {error}", pack_path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        counters.pack_reads = 1;
        let world = decode_nav_world(&mut reader, length, pack_path, observer, &mut counters)?;
        let reach = if pack_path.with_extension("navreach").is_file() {
            counters.reach_reads = 1;
            Some(Arc::new(DeferredReach::prepare(pack_path, &world)?))
        } else {
            None
        };
        let canlight = if pack_path.with_extension("navcanlight").is_file() {
            counters.canlight_reads = 1;
            Some(load_canlight(pack_path, &world)?)
        } else {
            None
        };
        Ok(LoadedNav {
            availability: NavAvailability::Bound,
            world: Some(world),
            reach,
            canlight,
            counters,
        })
    }
}

/// The world fallback of a runtime cache fill. After a connection failure,
/// the fill moves on to the next roster world as a whole: its game endpoint
/// and its asset endpoint together. From the first failed lookup of a name
/// (`RuntimeCacheError::failed_lookup`), it passes over each later world
/// whose game host or asset host is that name. The stepped fill and the
/// one-call fill both report a failed lookup of either host.
///
/// Limit: the one-call fill (`prepare_runtime_cache`) looks up the asset
/// host again for each JAG download, and that error does not name the host.
/// So a name that looked up for `/crc` and then fails at the JAG download
/// can be looked up again for the next world that shares it. The cost is
/// one more resolver wait for each such world. Only a public (wss) roster
/// has more than one world.
struct WorldFallback {
    count: usize,
    /// The roster index of the selected world.
    initial: usize,
    /// The number of roster worlds after `initial` that the chain passed.
    passed: usize,
    failed_lookups: Vec<String>,
}

impl WorldFallback {
    fn new(selection: &ProfileSelection) -> Self {
        let worlds = &selection.profile.worlds;
        let count = selection.public_worlds.as_ref().map_or(1, |_| worlds.len());
        let initial = worlds
            .iter()
            .position(|world| {
                world.host == selection.game_host
                    && world.port == selection.game_port
                    && world.asset_host == selection.asset_host
                    && world.asset_port == selection.asset_port
            })
            .unwrap_or(0);
        Self {
            count,
            initial,
            passed: 0,
            failed_lookups: Vec::new(),
        }
    }

    /// After the fill of `selection` failed with `error`, select the next
    /// world in `selection`. False if `error` is not a connection failure
    /// or no world is left to try.
    fn next(
        &mut self,
        selection: &mut ProfileSelection,
        error: &client::unpack::RuntimeCacheError,
    ) -> bool {
        if !error.is_connection() {
            return false;
        }
        if let Some(host) = error.failed_lookup() {
            self.failed_lookups.push(host.to_owned());
        }
        let worlds = &selection.profile.worlds;
        while self.passed + 1 < self.count {
            self.passed += 1;
            let world = &worlds[(self.initial + self.passed) % worlds.len()];
            if self
                .failed_lookups
                .iter()
                .any(|name| *name == world.host || *name == world.asset_host)
            {
                continue;
            }
            selection.game_host.clone_from(&world.host);
            selection.game_port = world.port;
            selection.asset_host.clone_from(&world.asset_host);
            selection.asset_port = world.asset_port;
            return true;
        }
        false
    }
}

/// Log a cache-fill name lookup step that ended after its deadline, the
/// one exception to the step deadline ([`client::unpack::SlowLookup`]).
fn log_slow_lookup(lookup: &client::unpack::SlowLookup<'_>) {
    host_log!(
        stderr;
        Category::Lifecycle,
        Level::Warn,
        "cache preparation: name lookup of the {} {} took {:?}, {:?} past the step deadline \
         (the system resolver cannot be interrupted)",
        lookup.role,
        lookup.host,
        lookup.took,
        lookup.past_deadline
    );
}

/// Longest wait for the network or the preparation lock inside one
/// [`TemplatePreparation::step`]. It stays below WorkAdmission's ordinary
/// slice, so a cache-fill step returns before the slice ends.
pub const TEMPLATE_STEP_WAIT: std::time::Duration = std::time::Duration::from_millis(8);
const _: () =
    assert!(TEMPLATE_STEP_WAIT.as_nanos() < script::work_admission::WORK_SLICE.as_nanos());

/// A runtime template preparation in bounded steps
/// ([`ProfileSelection::begin_template_preparation`]). The cache fill is a
/// [`client::unpack::RuntimeCacheFill`]; each step reports
/// `PreparingCache n/total` once the entry count is known. When the cache is
/// ready, one step binds the profile and the next loads the template. A
/// connection failure to the asset host or the game host moves on to the
/// next world as a whole ([`WorldFallback`]), as
/// [`ProfileSelection::bind_runtime_with_progress`] does. A name lookup
/// step that ends after its deadline is logged through `host_log!`.
/// Dropping the value cancels the fill and releases everything it holds.
pub struct TemplatePreparation {
    selected: ProfileSelection,
    fallback: WorldFallback,
    last_total: Option<u32>,
    stage: TemplateStage,
}

enum TemplateStage {
    Start,
    Fill {
        /// Boxed: the fill is far larger than the other stages.
        fill: Box<client::unpack::RuntimeCacheFill>,
    },
    Bind {
        prepared: Arc<client::unpack::PreparedRuntimeCache>,
    },
    Load(Arc<ServerProfile>),
    Done,
}

/// Outcome of one [`TemplatePreparation::step`].
pub enum TemplateStep {
    /// More steps follow; `completed`/`total` count cache entries written.
    Continue {
        completed: u32,
        total: Option<u32>,
    },
    Complete(Result<Arc<crate::SharedClientTemplate>, String>),
}

impl TemplatePreparation {
    /// Do the next bounded unit outside the preparation worker.
    /// Network and lock waits end at `until`.
    pub fn step(
        &mut self,
        until: std::time::Instant,
        observer: &ProfileProgressObserver,
    ) -> TemplateStep {
        self.step_inner(until, observer, None)
    }

    /// Do the next unit using the enclosing WorkAdmission preparation.
    /// Quest facts use this capability without submitting or joining work.
    pub fn step_on_worker(
        &mut self,
        until: std::time::Instant,
        observer: &ProfileProgressObserver,
        worker: &mut api::selected::FamilyPreparation,
    ) -> TemplateStep {
        self.step_inner(until, observer, Some(worker))
    }

    fn step_inner(
        &mut self,
        until: std::time::Instant,
        observer: &ProfileProgressObserver,
        worker: Option<&mut api::selected::FamilyPreparation>,
    ) -> TemplateStep {
        let waiting = TemplateStep::Continue {
            completed: 0,
            total: None,
        };
        match std::mem::replace(&mut self.stage, TemplateStage::Done) {
            TemplateStage::Start => {
                observer.report(ProfileProgress::steps(
                    ProfileProgressStage::PreparingCache,
                    0,
                    1,
                ));
                let mut fill = Box::new(client::unpack::RuntimeCacheFill::begin(
                    &self.selected.runtime_cache_request(),
                ));
                fill.observe_slow_lookups(log_slow_lookup);
                self.stage = TemplateStage::Fill { fill };
                waiting
            }
            TemplateStage::Fill { mut fill } => match fill.step(Some(until)) {
                Ok(client::unpack::RuntimeCacheStep::Continue { written, total }) => {
                    if let Some(total) = total {
                        self.last_total = Some(total);
                        observer.report(ProfileProgress::steps(
                            ProfileProgressStage::PreparingCache,
                            u64::from(written),
                            u64::from(total),
                        ));
                    }
                    self.stage = TemplateStage::Fill { fill };
                    TemplateStep::Continue {
                        completed: written,
                        total,
                    }
                }
                Ok(client::unpack::RuntimeCacheStep::Ready(prepared)) => {
                    let total = u64::from(self.last_total.unwrap_or(1));
                    observer.report(ProfileProgress::steps(
                        ProfileProgressStage::PreparingCache,
                        total,
                        total,
                    ));
                    self.stage = TemplateStage::Bind { prepared };
                    self.progress()
                }
                Err(error) if self.fallback.next(&mut self.selected, &error) => {
                    self.last_total = None;
                    self.stage = TemplateStage::Start;
                    waiting
                }
                Err(error) => TemplateStep::Complete(Err(error.to_string())),
            },
            TemplateStage::Bind { prepared } => {
                let bound = self.selected.bind_with_cache(
                    observer,
                    std::env::current_exe()
                        .ok()
                        .map(|exe| install_resource_root(&exe))
                        .as_deref(),
                    true,
                    Some(prepared),
                );
                match bound {
                    Ok(profile) => {
                        self.stage = TemplateStage::Load(profile);
                        self.progress()
                    }
                    Err(error) => TemplateStep::Complete(Err(error)),
                }
            }
            TemplateStage::Load(profile) => TemplateStep::Complete(match worker {
                Some(worker) => {
                    crate::SharedClientTemplate::load_on_worker(profile, observer, worker)
                }
                None => crate::SharedClientTemplate::load_with_progress(profile, observer),
            }),
            TemplateStage::Done => {
                TemplateStep::Complete(Err("profile preparation already finished".into()))
            }
        }
    }

    fn progress(&self) -> TemplateStep {
        TemplateStep::Continue {
            completed: self.last_total.unwrap_or(1),
            total: Some(self.last_total.unwrap_or(1)),
        }
    }
}

/// Paint-only data, decoded once when a panel or TUI consumer requests it.
/// Routing never requests it.
#[derive(Debug)]
pub struct DeferredReach {
    path: PathBuf,
    header: nav_planner::pack::ReachSidecarHeader,
    bits: std::sync::OnceLock<Result<Arc<[u64]>, String>>,
    reads: std::sync::atomic::AtomicU32,
}

impl DeferredReach {
    fn prepare(pack_path: &Path, world: &NavWorld) -> Result<Self, String> {
        let path = pack_path.with_extension("navreach");
        let (file, length) = open_nav_file(&path)
            .map_err(|error| format!("navigation {}: {error}", path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let header = nav_planner::pack::read_reach_sidecar_header(&mut reader, length)
            .map_err(|e| format!("navigation {}: {e}", path.display()))?;
        let c = &world.collision;
        if header.origin != c.origin
            || header.width != c.width
            || header.height != c.height
            || header.word_count != c.walk.len().div_ceil(64)
        {
            return Err(format!(
                "navigation {} geometry does not match pack",
                path.display()
            ));
        }
        Ok(Self {
            path,
            header,
            bits: std::sync::OnceLock::new(),
            reads: std::sync::atomic::AtomicU32::new(0),
        })
    }

    fn load(&self) -> Result<Arc<[u64]>, String> {
        let (file, length) = open_nav_file(&self.path)
            .map_err(|error| format!("navigation {}: {error}", self.path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let side = nav_planner::pack::read_reach_sidecar(&mut reader, length)
            .map_err(|e| format!("navigation {}: {e}", self.path.display()))?;
        if side.origin != self.header.origin
            || side.width != self.header.width
            || side.height != self.header.height
            || side.word_count != self.header.word_count
        {
            return Err(format!(
                "navigation {} geometry does not match pack",
                self.path.display()
            ));
        }
        Ok(side.bits)
    }

    /// Decode once on paint demand, caching success or failure.
    pub fn get(&self) -> Option<Arc<[u64]>> {
        self.bits
            .get_or_init(|| {
                self.load()
                    .inspect_err(|error| eprintln!("host-play: {error}"))
            })
            .as_ref()
            .ok()
            .cloned()
    }

    pub(super) fn load_count(&self) -> u32 {
        self.reads.load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn load_canlight(pack_path: &Path, world: &NavWorld) -> Result<Arc<[u64]>, String> {
    let path = pack_path.with_extension("navcanlight");
    let (file, length) =
        open_nav_file(&path).map_err(|error| format!("navigation {}: {error}", path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let side = nav_planner::pack::read_canlight_sidecar(&mut reader, length)
        .map_err(|e| format!("navigation {}: {e}", path.display()))?;
    let c = &world.collision;
    if side.origin != c.origin
        || side.width != c.width
        || side.height != c.height
        || side.word_count != c.walk.len().div_ceil(64)
    {
        return Err(format!(
            "navigation {} geometry does not match pack",
            path.display()
        ));
    }
    Ok(side.bits)
}

fn decode_nav_world(
    reader: &mut impl BufRead,
    length: usize,
    pack_path: &Path,
    observer: &ProfileProgressObserver,
    counters: &mut NavLoadCounters,
) -> Result<Arc<NavWorld>, String> {
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingNavigation,
        0,
        1,
    ));
    let world = NavWorld::from_reader(reader, length)
        .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
    counters.pack_decodes = 1;
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingNavigation,
        1,
        1,
    ));
    Ok(Arc::new(world))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "274bot-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn template_load_with_game_data_completes_on_the_preparation_worker() {
        if api::test_support::local_asset_path(
            "",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../api/data/game-data/289.json"
            ),
            "revision 289 game data",
        )
        .is_none()
        {
            return;
        }
        use script::preparation::{
            submit_cooperative, CooperativePreparationStep as Step, PreparationInput,
            PreparationKind,
        };
        use std::time::{Duration, Instant};

        let root = scratch_dir("template-preparation-worker");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
        let selection = super::ProfileOptions {
            revision: Some("289".into()),
            cache_dir: Some(fixtures.clone()),
            unpack_dir: Some(root.join("unpack")),
            nav_pack: Some(root.join("missing.navpack")),
            ..Default::default()
        }
        .resolve_with_env(
            None,
            &super::ProfileEnvironment {
                home: Some(root.clone()),
                engine_dir: Some(root.join("fixture-engine")),
                rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
                rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut preparation = selection.begin_template_load_for_test().unwrap();
        let observer = super::ProfileProgressObserver::default();
        let task = submit_cooperative(
            PreparationKind::Profile,
            api::identity::Scope::Process,
            PreparationInput::new(1, 64 << 20, 32 << 20, 32 << 20).unwrap(),
            move |worker| match preparation.step_on_worker(
                Instant::now() + super::TEMPLATE_STEP_WAIT,
                &observer,
                worker,
            ) {
                super::TemplateStep::Continue { completed, total } => {
                    Step::Continue { completed, total }
                }
                super::TemplateStep::Complete(output) => Step::Complete {
                    output,
                    completed: 1,
                    total: Some(1),
                },
            },
        )
        .unwrap();
        let started = Instant::now();
        while !task.is_finished() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "template load blocked its own preparation worker"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let template = task.join().unwrap().unwrap();
        let data = template.game_data().expect("the profile has game data");
        assert!(
            data.try_quests().is_some(),
            "quest facts are ready before play"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paint_sidecar_is_lazy_shared_and_rejects_malformed_payload() {
        let dir = scratch_dir("lazy-paint");
        let pack = dir.join("world.navpack");
        let world = nav_planner::world::NavWorld::from_grid(
            &nav_planner::grid::StepGrid::fixture_open_3x3(),
        );
        let c = &world.collision;
        let bits = vec![0x1234u64; c.walk.len().div_ceil(64)];
        let bytes = nav_bake::pack::encode_reach_sidecar(c.origin, c.width, c.height, &bits);
        let path = pack.with_extension("navreach");
        std::fs::write(&path, &bytes).unwrap();
        let prepared = super::DeferredReach::prepare(&pack, &world).unwrap();
        assert!(
            prepared.bits.get().is_none(),
            "binding must retain no paint payload"
        );
        let panel = prepared.get().unwrap();
        let tui = prepared.get().unwrap();
        assert_eq!(&*panel, &bits);
        assert!(
            std::sync::Arc::ptr_eq(&panel, &tui),
            "frontends share the first decode"
        );

        let changed = super::DeferredReach::prepare(&pack, &world).unwrap();
        let mut edited = bytes.clone();
        edited[0] ^= 1;
        std::fs::write(&path, &edited).unwrap();
        assert!(
            changed.get().is_none(),
            "a malformed sidecar must not publish paint words"
        );
        std::fs::write(&path, &bytes).unwrap();
        assert!(
            changed.get().is_none(),
            "a failed first load remains failed"
        );
        assert_eq!(
            &*prepared.get().unwrap(),
            &bits,
            "published words remain immutable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

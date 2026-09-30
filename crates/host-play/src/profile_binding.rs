use super::*;
use api::host_log;
use api::hostlog::{Category, Level};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};

// Hash the very bytes the decoder reads, without reopening the resource or
// retaining a full-file staging allocation. Bundled identities need no hash.
struct NavReader<'a> {
    file: std::fs::File,
    digest: Option<Sha256>,
    completed: u64,
    total: u64,
    observer: &'a ProfileProgressObserver,
}

impl Read for NavReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.file.read(buffer)?;
        if let Some(digest) = &mut self.digest {
            digest.update(&buffer[..count]);
            self.completed += count as u64;
            if self.completed < self.total {
                self.observer.report(ProfileProgress::bytes(
                    ProfileProgressStage::CheckingNavigationFiles,
                    self.completed,
                    self.total,
                ));
            }
        }
        Ok(count)
    }
}

fn open_nav_file(path: &Path) -> std::io::Result<(std::fs::File, usize)> {
    let file = std::fs::File::open(path)?;
    let length = usize::try_from(file.metadata()?.len())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok((file, length))
}

fn finish_nav_hash(mut reader: BufReader<NavReader<'_>>) -> std::io::Result<String> {
    // Byte decoders historically accept trailing bytes. They still contribute
    // to the external identity, including bytes already buffered by BufReader.
    std::io::copy(&mut reader, &mut std::io::sink())?;
    let mut reader = reader.into_inner();
    let hash = format!(
        "{:x}",
        reader.digest.take().expect("external nav hash").finalize()
    );
    reader.observer.report(ProfileProgress::bytes(
        ProfileProgressStage::CheckingNavigationFiles,
        reader.completed,
        reader.completed,
    ));
    Ok(hash)
}

const JAGS: [&str; 8] = CacheManifest::ARCHIVES;
struct LoadedNav {
    availability: NavAvailability,
    identity: Option<NavManifest>,
    world: Option<Arc<NavWorld>>,
    reach: Option<Arc<[u64]>>,
    canlight: Option<Arc<[u64]>>,
    counters: NavLoadCounters,
}
/// One pinned generator source file: length plus SHA-256 is the identity.
/// Length alone is not: same-size different bytes must not verify.
fn verify_game_data_source(
    path: &Path,
    expected: &api::game_data::SourceInput,
) -> Result<(), GameDataSourceFault> {
    let actual_len = std::fs::metadata(path)
        .map_err(|error| GameDataSourceFault::Unreadable {
            error: error.to_string(),
        })?
        .len();
    if actual_len != expected.bytes {
        return Err(GameDataSourceFault::Length {
            expected: expected.bytes,
            actual: actual_len,
        });
    }
    let actual = nav::manifest::hash_file(path)
        .map_err(|error| GameDataSourceFault::Unreadable { error })?;
    if actual != expected.sha256 {
        return Err(GameDataSourceFault::Content {
            expected: expected.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

/// Every pinned generator source must verify: engine and decoder inputs
/// under the engine dir, content inputs under the content dir. Any mismatch
/// (missing file, length, or bytes) keeps generated facts closed. The whole
/// list is always checked so a rejection reports how many inputs failed (a
/// wrong source root fails all of them; one edited file fails one). Returns
/// the number of verified inputs.
fn verify_game_data_sources(
    data: &api::game_data::SelectedGameData,
    engine_dir: &Path,
    content_dir: &Path,
) -> Result<usize, GameDataSourceRejection> {
    verify_source_inputs(data.source_inputs(), engine_dir, content_dir)
}

fn verify_source_inputs<'a>(
    inputs: impl Iterator<Item = (bool, &'a api::game_data::SourceInput)>,
    engine_dir: &Path,
    content_dir: &Path,
) -> Result<usize, GameDataSourceRejection> {
    let mut total = 0;
    let mut failures = Vec::new();
    for (is_content, input) in inputs {
        total += 1;
        let (root, base) = if is_content {
            (GameDataSourceRoot::Content, content_dir)
        } else {
            (GameDataSourceRoot::Engine, engine_dir)
        };
        let path = base.join(&input.path);
        if let Err(fault) = verify_game_data_source(&path, input) {
            failures.push(GameDataSourceFailure {
                root,
                input: input.path.clone(),
                path,
                fault,
            });
        }
    }
    if failures.is_empty() {
        Ok(total)
    } else {
        Err(GameDataSourceRejection { total, failures })
    }
}

#[cfg(feature = "debug-catalog")]
impl ServerProfile {
    /// Independently verify and decode on Debug-tab demand, never on bind.
    /// The caller owns the catalog; closing the tab releases its heap.
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
        verify_source_inputs(catalog.source_inputs(), &self.debug_engine_dir, &self.content_dir)
            .map_err(|rejection| format!(
                "Debug catalog withheld: {} of {} debug inputs failed verification; other generated facts remain available",
                rejection.failures.len(), rejection.total
            ))?;
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
        self.bind_with_nav_identities(
            observer,
            bundled_nav_identities(),
            std::env::current_exe()
                .ok()
                .map(|exe| install_resource_root(&exe))
                .as_deref(),
        )
    }

    /// Bind using an explicit identity table and install root. Production
    /// uses the compile-time table (empty in git). Tests cover the nonempty
    /// bundled path without claiming a released app.
    pub fn bind_with_nav_identities(
        &self,
        observer: &ProfileProgressObserver,
        table: &[BundledNavIdentity],
        resource_root: Option<&Path>,
    ) -> Result<Arc<ServerProfile>, String> {
        self.bind_inner(observer, table, resource_root, false, &mut false)
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
        let worlds = self.public_worlds.as_deref();
        let mut selected = self.clone();
        let count = worlds.map_or(1, |w| w.worlds.len());
        let initial = worlds
            .and_then(|w| {
                w.worlds.iter().position(|world| {
                    world.host == self.asset_host && world.port == self.asset_port
                })
            })
            .unwrap_or(0);
        for attempt in 0..count {
            let mut asset_connection_failed = false;
            match selected.bind_inner(
                observer,
                bundled_nav_identities(),
                std::env::current_exe()
                    .ok()
                    .map(|exe| install_resource_root(&exe))
                    .as_deref(),
                true,
                &mut asset_connection_failed,
            ) {
                Ok(profile) => return Ok(profile),
                Err(_) if attempt + 1 < count && asset_connection_failed => {
                    let world = &worlds.expect("public worlds for fallback").worlds
                        [(initial + attempt + 1) % count];
                    selected.asset_host = world.host.clone();
                    selected.asset_port = world.port;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("at least one world or one local endpoint")
    }

    fn bind_inner(
        &self,
        observer: &ProfileProgressObserver,
        table: &[BundledNavIdentity],
        resource_root: Option<&Path>,
        runtime: bool,
        asset_connection_failed: &mut bool,
    ) -> Result<Arc<ServerProfile>, String> {
        // The declared identity is read before any preparation: a manifest for
        // another revision must be rejected without touching the update server
        // for the selected cache directory.
        let supplied = match &self.cache_manifest {
            Some(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| format!("cache manifest {}: {e}", path.display()))?;
                let manifest = serde_json::from_slice::<CacheManifest>(&bytes)
                    .map_err(|e| format!("cache manifest {}: {e}", path.display()))?;
                let revision = self.revision().as_i32() as u16;
                if manifest.revision != revision {
                    return Err(format!(
                        "cache/profile mismatch for revision {revision} at {}: \
                         declared manifest revision {}",
                        self.cache_dir.display(),
                        manifest.revision
                    ));
                }
                Some(manifest)
            }
            None => None,
        };
        // A runtime `--cache-manifest` authenticates the operator-selected
        // source cache. Negotiation may replace the prepared copies.
        if runtime {
            if let Some(manifest) = &supplied {
                manifest.verify(self.revision().as_i32() as u16, &self.cache_dir)?;
            }
        }

        let runtime_cache = if runtime {
            observer.report(ProfileProgress::steps(
                ProfileProgressStage::PreparingCache,
                0,
                1,
            ));
            let prepared =
                client::unpack::prepare_runtime_cache(&client::unpack::RuntimeCacheRequest {
                    revision: self.revision(),
                    transport: self.transport(),
                    jag_source: &self.cache_dir,
                    snapshot_root: &self.unpack_dir,
                    asset_host: &self.asset_host,
                    asset_port: self.asset_port,
                    game_host: &self.game_host,
                    game_port: self.game_port,
                })
                .map_err(|error| {
                    *asset_connection_failed = error.is_connection();
                    error.to_string()
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
        let cache_dir = runtime_cache
            .as_ref()
            .map_or(self.cache_dir.as_path(), |p| p.jag_dir.as_path());
        let unpack_dir = runtime_cache
            .as_ref()
            .map_or(self.unpack_dir.as_path(), |p| p.unpack_root());
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
            crate::cache::prepare(
                cache_dir,
                unpack_dir,
                self.transport(),
                &self.asset_host,
                self.asset_port,
                observer,
            )
            .availability
        };
        let mut archives = std::collections::BTreeMap::new();
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
            let bytes =
                std::fs::read(cache_dir.join(name)).map_err(|e| format!("cache {name}: {e}"))?;
            archives.insert((*name).into(), hash_bytes_with_progress(&bytes, |_, _| {}));
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
        let actual = CacheManifest {
            revision: self.revision().as_i32() as u16,
            archives,
        };
        // Runtime manifests attest the selected source revision, not the
        // endpoint's future packed representation. Negotiation may replace it.
        if runtime && supplied.is_none() {
            let data = api::game_data::for_revision(self.revision())?;
            let decoded = runtime_cache
                .as_ref()
                .expect("runtime preparation")
                .identity
                .content_id_hex();
            if data.content_id() != Some(decoded.as_str()) {
                return Err("unknown decoded cache content; supply an explicit cache manifest for a custom server (generated metadata may be unavailable)".into());
            }
        }
        let declared = match supplied {
            _ if runtime => actual.clone(),
            Some(manifest) => manifest,
            None => {
                let known: Vec<CacheManifest> =
                    serde_json::from_str(include_str!("known-cache-identities.json"))
                        .expect("bundled cache identities");
                known.into_iter().find(|m| m.archives == actual.archives)
                    .ok_or_else(|| "cache revision is unverified; supply --cache-manifest for the prepared server/cache pairing".to_string())?
            }
        };
        if declared != actual {
            return Err(format!(
                "cache/profile mismatch for revision {} at {}",
                self.revision().as_i32(),
                self.cache_dir.display()
            ));
        }
        let cache_id = runtime_cache
            .as_ref()
            .map_or_else(|| actual.identity(), |p| p.identity.content_id_hex());
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
        let origin = select_nav_origin(
            table,
            resource_root,
            self.revision().as_i32() as u16,
            &cache_id,
            self.nav_pack_overridden,
            &self.nav_pack,
        )?;
        let nav_pack = origin.path().to_path_buf();
        // Flags provenance is captured explicitly at bind time. A bundled
        // pack does not imply bundled flags: --nav-flags / NAV_FLAGS keeps
        // external validation even when the path equals the bundle sibling.
        let (nav_flags, nav_flags_origin) = if origin.is_bundled() && !self.nav_flags_overridden {
            (nav_pack.with_extension("navflags"), NavFlagsOrigin::Bundled)
        } else {
            (self.nav_flags.clone(), NavFlagsOrigin::External)
        };
        let loaded = self.load_nav(
            &origin,
            &actual,
            &nav_pack,
            runtime_cache
                .as_ref()
                .map(|p| p.identity.content_id_hex())
                .as_deref(),
            observer,
        )?;
        if runtime && loaded.world.is_some() {
            if origin.is_bundled() {
                eprintln!(
                    "host-play: navigation source: packaged {} bundle: {}",
                    nav::pack::FORMAT_ID,
                    nav_pack.display()
                );
            } else {
                eprintln!(
                    "host-play: navigation source: external pack: {}",
                    nav_pack.display()
                );
            }
        }
        if runtime {
            if let Some(identity) = &loaded.identity {
                // Public installations need no server source checkout: only a
                // compiled build/packager row can attest the supported world.
                if self.transport() == Transport::Wss {
                    let trusted = table.iter().any(|row| {
                        row.revision == identity.revision
                            && row.content_id == identity.content_id
                            && row.source_sha256 == identity.source_sha256
                            && row.nav_sha256 == identity.nav_sha256
                    });
                    if !trusted {
                        return Err("navigation source provenance is not a trusted supported-public build; rebuild/package the resources".into());
                    }
                }
            }
        }
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
        })?);
        let (game_data, game_data_status) = self.attach_game_data(runtime, &cache_id)?;
        if runtime && !game_data_status.is_attached() {
            host_log!(
                stderr;
                Category::Lifecycle,
                Level::Warn,
                "generated game data withheld ({}): {}",
                game_data_status.reason(),
                game_data_status.detail()
            );
        }
        Ok(Arc::new(ServerProfile {
            profile: self.profile.clone(),
            class: self.class,
            client: binding,
            cache: availability,
            cache_manifest: actual,
            runtime_cache,
            game_data,
            game_data_status,
            nav_pack,
            nav_flags,
            nav_origin: origin,
            nav_flags_origin,
            nav_identity: loaded.identity,
            nav_load: loaded.counters,
            world: SharedWorld(loaded.world),
            reach: loaded.reach,
            canlight: loaded.canlight,
            nav: loaded.availability,
            content_dir: self.content_dir.clone(),
            #[cfg(feature = "debug-catalog")]
            debug_engine_dir: self.engine_dir.clone(),
            vault_path: self.vault_path.clone(),
            catalog_root: self.catalog_root.clone(),
            world_members: self.world_members.clone(),
            public_worlds: self.public_worlds.clone(),
        }))
    }

    /// The fact-trust decision, with its cause. Facts attach only for a
    /// supported server whose cache the generated asset describes and, for a
    /// local world, whose pinned generator sources all verify. Recording the
    /// cause never widens what attaches.
    fn attach_game_data(
        &self,
        runtime: bool,
        cache_id: &str,
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
        let Some(data) = api::game_data::for_optional_profile(self.revision(), cache_id)? else {
            let generated = api::game_data::for_revision(self.revision())?;
            return Ok((
                None,
                GameDataStatus::CacheIdentity {
                    profile_cache_id: cache_id.to_owned(),
                    generated_cache_id: generated.cache_id().to_owned(),
                    generated_content_id: generated.content_id().map(str::to_owned),
                },
            ));
        };
        if public {
            return Ok((
                Some(data),
                GameDataStatus::Attached(GameDataAttestation::PublicBuild),
            ));
        }
        match verify_game_data_sources(&data, &self.engine_dir, &self.content_dir) {
            Ok(inputs) => Ok((
                Some(data),
                GameDataStatus::Attached(GameDataAttestation::SourcesVerified { inputs }),
            )),
            Err(rejection) => Ok((None, GameDataStatus::SourceRejected(rejection))),
        }
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

    /// Bind the immutable profile, check the captured cache identity once, and
    /// decode the shared process resources.
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

    fn load_nav(
        &self,
        origin: &NavOrigin,
        cache: &CacheManifest,
        pack_path: &Path,
        content_id: Option<&str>,
        observer: &ProfileProgressObserver,
    ) -> Result<LoadedNav, String> {
        let revision = self.revision().as_i32() as u16;
        let mut counters = NavLoadCounters::default();
        if !pack_path.exists() {
            if origin.is_bundled() {
                return Err(format!(
                    "bundled navigation {} is missing",
                    pack_path.display()
                ));
            }
            return Ok(LoadedNav {
                availability: NavAvailability::Unavailable(format!(
                    "navigation unavailable: {} has not been prepared",
                    pack_path.display()
                )),
                identity: None,
                world: None,
                reach: None,
                canlight: None,
                counters,
            });
        }
        let (file, length) = open_nav_file(pack_path)
            .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
        let mut reader = BufReader::with_capacity(
            64 * 1024,
            NavReader {
                file,
                digest: (!origin.is_bundled()).then(Sha256::new),
                completed: 0,
                total: length as u64,
                observer,
            },
        );
        counters.pack_reads = 1;
        let prefix = reader
            .fill_buf()
            .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
        if !origin.is_bundled()
            && prefix.starts_with(b"274V")
            && prefix
                .get(4)
                .is_some_and(|version| *version < nav::pack::VERSION)
        {
            let message = format!(
                "navigation unavailable: navigation pack was built by an older 274bot; rebuild it with nav-pack: {}",
                pack_path.display()
            );
            eprintln!("host-play: {message}");
            return Ok(LoadedNav {
                availability: NavAvailability::Unavailable(message),
                identity: None,
                world: None,
                reach: None,
                canlight: None,
                counters,
            });
        }
        let world_result =
            decode_nav_world(&mut reader, length, pack_path, observer, &mut counters);
        let identity = match origin {
            NavOrigin::Bundled { identity, .. } => NavManifest {
                revision: identity.revision,
                cache_id: identity.cache_id.clone(),
                nav_sha256: identity.nav_sha256.clone(),
                flags_sha256: identity.flags_sha256.clone(),
                reach_sha256: identity.reach_sha256.clone(),
                canlight_sha256: identity.canlight_sha256.clone(),
                pois_sha256: identity.pois_sha256.clone(),
                content_id: identity.content_id.clone(),
                source_sha256: identity.source_sha256.clone(),
            },
            NavOrigin::External { .. } => {
                let nav_hash = finish_nav_hash(reader)
                    .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
                counters.pack_hashes = 1;
                let manifest_path = nav_manifest_path(pack_path);
                if !manifest_path.exists() {
                    if self.revision() == ClientRevision::R274 && content_id.is_none() {
                        let world = world_result?;
                        let identity = NavManifest {
                            revision,
                            cache_id: cache.identity(),
                            nav_sha256: nav_hash,
                            flags_sha256: None,
                            reach_sha256: None,
                            canlight_sha256: None,
                            pois_sha256: None,
                            content_id: None,
                            source_sha256: None,
                        };
                        return Ok(LoadedNav {
                            availability: NavAvailability::Legacy274,
                            identity: Some(identity),
                            world: Some(world),
                            reach: None,
                            canlight: None,
                            counters,
                        });
                    }
                    return Err(format!(
                        "navigation/profile mismatch: revision 289 requires {} (prepare in step 5)",
                        manifest_path.display()
                    ));
                }
                let manifest_bytes = std::fs::read(&manifest_path)
                    .map_err(|e| format!("nav manifest {}: {e}", manifest_path.display()))?;
                let manifest: NavManifest = serde_json::from_slice(&manifest_bytes)
                    .map_err(|e| format!("nav manifest: {e}"))?;
                manifest.verify_pack(revision, cache, &nav_hash, content_id)?;
                manifest
            }
        };
        let world = world_result?;
        let reach = if origin.is_bundled() {
            if identity.reach_sha256.is_none() {
                return Err("bundled navigation reach identity is missing".into());
            }
            Some(load_bundled_reach(
                pack_path,
                &world,
                &identity.nav_sha256,
                &mut counters,
            )?)
        } else {
            None
        };
        let canlight = if origin.is_bundled() {
            let Some(policy_hex) = origin
                .bundled_identity()
                .and_then(|row| row.canlight_identity.as_deref())
            else {
                return Err("bundled navigation canlight identity is missing".into());
            };
            if identity.canlight_sha256.is_none() {
                return Err("bundled navigation canlight identity is missing".into());
            }
            Some(load_bundled_canlight(
                pack_path,
                &world,
                &identity.nav_sha256,
                policy_hex,
                &mut counters,
            )?)
        } else {
            None
        };
        Ok(LoadedNav {
            availability: NavAvailability::Bound,
            identity: Some(identity),
            world: Some(world),
            reach,
            canlight,
            counters,
        })
    }
}
fn load_bundled_reach(
    pack_path: &Path,
    world: &NavWorld,
    nav_sha256: &str,
    counters: &mut NavLoadCounters,
) -> Result<Arc<[u64]>, String> {
    let reach_path = pack_path.with_extension("navreach");
    if !reach_path.exists() {
        return Err(format!(
            "bundled navigation {} is missing",
            reach_path.display()
        ));
    }
    let (file, length) = open_nav_file(&reach_path)
        .map_err(|e| format!("bundled navigation {}: {e}", reach_path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    counters.reach_reads = 1;
    let side = nav::pack::read_reach_sidecar(&mut reader, length)
        .map_err(|e| format!("bundled navigation {}: {e}", reach_path.display()))?;
    if sha256_hex(&side.binding) != nav_sha256 {
        return Err(format!(
            "bundled navigation {} binding does not match pack identity",
            reach_path.display()
        ));
    }
    let c = &world.collision;
    let words = c.walk.len().div_ceil(64);
    if side.origin != c.origin
        || side.width != c.width
        || side.height != c.height
        || side.word_count != words
        || side.bits.len() != words
    {
        return Err(format!(
            "bundled navigation {} geometry does not match pack",
            reach_path.display()
        ));
    }
    Ok(side.bits)
}

fn load_bundled_canlight(
    pack_path: &Path,
    world: &NavWorld,
    nav_sha256: &str,
    canlight_identity: &str,
    counters: &mut NavLoadCounters,
) -> Result<Arc<[u64]>, String> {
    let canlight_path = pack_path.with_extension("navcanlight");
    if !canlight_path.exists() {
        return Err(format!(
            "bundled navigation {} is missing",
            canlight_path.display()
        ));
    }
    let (file, length) = open_nav_file(&canlight_path)
        .map_err(|e| format!("bundled navigation {}: {e}", canlight_path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    counters.canlight_reads = 1;
    let side = nav::pack::read_canlight_sidecar(&mut reader, length).map_err(|e| match e {
        nav::pack::PackError::BadMagic => {
            format!("bundled navigation {}: bad magic", canlight_path.display())
        }
        nav::pack::PackError::BadVersion(v) => format!(
            "bundled navigation {}: unsupported version {v}",
            canlight_path.display()
        ),
        nav::pack::PackError::Truncated => {
            format!("bundled navigation {}: truncated", canlight_path.display())
        }
        other => format!("bundled navigation {}: {other}", canlight_path.display()),
    })?;
    let expected = canlight::expected_header_binding(nav_sha256, canlight_identity)
        .map_err(|e| format!("bundled navigation {} binding {e}", canlight_path.display()))?;
    if side.binding != expected {
        return Err(format!(
            "bundled navigation {} binding does not match pack and policy identity",
            canlight_path.display()
        ));
    }
    let c = &world.collision;
    let words = c.walk.len().div_ceil(64);
    if side.origin != c.origin
        || side.width != c.width
        || side.height != c.height
        || side.word_count != words
        || side.bits.len() != words
    {
        return Err(format!(
            "bundled navigation {} geometry does not match pack",
            canlight_path.display()
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
    use super::{
        verify_game_data_source, verify_source_inputs, GameDataSourceFault, GameDataSourceRoot,
        GameDataStatus,
    };
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

    /// Pin `path` by its own current length and SHA-256, the same
    /// `nav::manifest::hash_file` the binding uses (the real pinned digests
    /// have no synthetic preimage).
    fn pin(path: &Path, relative: &str) -> api::game_data::SourceInput {
        api::game_data::SourceInput {
            path: relative.to_string(),
            bytes: std::fs::metadata(path).unwrap().len(),
            sha256: nav::manifest::hash_file(path).unwrap(),
        }
    }

    /// Length-plus-SHA-256 source binding, with no real engine or content.
    ///
    /// `bind` attaches generated facts only when every pinned generator source
    /// verifies by exact bytes, through this predicate. The pinned SHA-256
    /// values have no synthetic preimage, so the expected identity comes from
    /// the probe's own bytes via the same `nav::manifest::hash_file` the
    /// binding uses; one flipped byte at the same length must be refused.
    #[test]
    fn equal_size_different_bytes_sources_do_not_attach_game_data() {
        let dir = scratch_dir("verify-sources");

        // Exact bytes verify through the predicate `bind` uses.
        let probe = dir.join("probe.bin");
        let bytes: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
        std::fs::write(&probe, &bytes).unwrap();
        let expected = pin(&probe, "probe.bin");
        verify_game_data_source(&probe, &expected).expect("exact source bytes must verify");

        // One flipped byte keeps the length: the size-only shortcut this guards
        // against would accept this file, so it must be refused for content.
        let mut mutated = bytes.clone();
        mutated[0] ^= 0xff;
        std::fs::write(&probe, &mutated).unwrap();
        assert_eq!(
            std::fs::metadata(&probe).unwrap().len(),
            expected.bytes,
            "the mutation must keep the length so only bytes differ"
        );
        let fault = verify_game_data_source(&probe, &expected)
            .expect_err("equal-size different-bytes sources must keep facts closed");
        assert!(
            matches!(fault, GameDataSourceFault::Content { .. }),
            "refusal must be a content mismatch, got {fault:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rejection keeps facts closed but names every failed input, its root
    /// and how it failed, so a wrong source root, a missing generated file and
    /// an edited file are told apart without a debugger.
    #[test]
    fn rejection_tells_missing_truncated_and_edited_inputs_apart() {
        let dir = scratch_dir("reject-sources");
        let engine = dir.join("engine");
        let content = dir.join("content");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::create_dir_all(&content).unwrap();
        for (root, name, bytes) in [
            (&engine, "ok.bin", vec![1_u8; 64]),
            (&engine, "short.bin", vec![2_u8; 64]),
            (&content, "edited.bin", vec![3_u8; 64]),
            (&content, "gone.bin", vec![4_u8; 64]),
        ] {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        let inputs = [
            (false, pin(&engine.join("ok.bin"), "ok.bin")),
            (false, pin(&engine.join("short.bin"), "short.bin")),
            (true, pin(&content.join("edited.bin"), "edited.bin")),
            (true, pin(&content.join("gone.bin"), "gone.bin")),
        ];
        let refs = || inputs.iter().map(|(content, input)| (*content, input));
        assert_eq!(
            verify_source_inputs(refs(), &engine, &content),
            Ok(4),
            "pristine inputs verify and report how many"
        );

        std::fs::write(engine.join("short.bin"), vec![2_u8; 10]).unwrap();
        let mut edited = vec![3_u8; 64];
        edited[63] ^= 0xff;
        std::fs::write(content.join("edited.bin"), edited).unwrap();
        std::fs::remove_file(content.join("gone.bin")).unwrap();

        let rejection = verify_source_inputs(refs(), &engine, &content)
            .expect_err("three broken inputs must reject the whole set");
        assert_eq!((rejection.total, rejection.verified()), (4, 1));
        let failures = rejection
            .failures
            .iter()
            .map(|failure| (failure.root, failure.input.as_str(), failure.fault.kind()))
            .collect::<Vec<_>>();
        assert_eq!(
            failures,
            [
                (GameDataSourceRoot::Engine, "short.bin", "length-mismatch"),
                (
                    GameDataSourceRoot::Content,
                    "edited.bin",
                    "content-mismatch"
                ),
                (GameDataSourceRoot::Content, "gone.bin", "unreadable"),
            ],
            "failures keep the generator's order and name their root"
        );
        assert_eq!(
            rejection.failures[2].path,
            content.join("gone.bin"),
            "the failure names the resolved path that was read"
        );

        let json = GameDataStatus::SourceRejected(rejection).to_json();
        assert_eq!(json["attached"], false);
        assert_eq!(json["reason"], "source-rejected");
        assert_eq!(json["sources"]["total"], 4);
        assert_eq!(json["sources"]["verified"], 1);
        assert_eq!(json["sources"]["failed"], 3);
        assert_eq!(json["sources"]["failures"][2]["root"], "content");
        assert_eq!(json["sources"]["failures"][2]["kind"], "unreadable");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A wrong source root fails every pinned input; the record still counts
    /// them all but stays small.
    #[test]
    fn wrong_source_root_record_counts_every_input_but_lists_a_bounded_few() {
        let dir = scratch_dir("wrong-root");
        let pinned = (0..12)
            .map(|i| api::game_data::SourceInput {
                path: format!("data/pack/input-{i}.bin"),
                bytes: 8,
                sha256: "00".repeat(32),
            })
            .collect::<Vec<_>>();
        let rejection = verify_source_inputs(pinned.iter().map(|input| (true, input)), &dir, &dir)
            .expect_err("nothing exists under the wrong root");
        assert_eq!((rejection.total, rejection.verified()), (12, 0));
        let json = GameDataStatus::SourceRejected(rejection).to_json();
        assert_eq!(json["sources"]["failed"], 12);
        assert_eq!(json["sources"]["failures"].as_array().unwrap().len(), 8);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

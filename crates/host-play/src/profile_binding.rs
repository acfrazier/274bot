use super::*;
const JAGS: [&str; 8] = CacheManifest::ARCHIVES;
struct LoadedNav {
    availability: NavAvailability,
    identity: Option<NavManifest>,
    world: Option<Arc<NavWorld>>,
    reach: Option<Arc<[u64]>>,
    canlight: Option<Arc<[u64]>>,
    counters: NavLoadCounters,
}
impl ProfileSelection {
    pub fn selection(&self) -> ServerSelection {
        self.selection
    }
    pub fn revision(&self) -> ClientRevision {
        self.selection.revision()
    }
    pub fn target(&self) -> BotTarget {
        self.selection.target()
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
            self.selection.name(),
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
                    target: self.target(),
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
                self.target(),
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
        let (rsa_modulus, rsa_exponent) = if self.target() == BotTarget::Prod {
            (
                client::PROD_LOGIN_RSAN.into(),
                client::PROD_LOGIN_RSAE.into(),
            )
        } else if let Some(n) = &self.rsa_modulus {
            (
                n.clone(),
                self.rsa_exponent
                    .clone()
                    .unwrap_or_else(|| client::JAVA_LOGIN_RSAE.into()),
            )
        } else if self.rsa_exponent.is_some() {
            return Err("LOGIN_RSAE requires LOGIN_RSAN on an explicit server profile".into());
        } else {
            let pem = self.engine_dir.join("data/config/private.pem");
            client::login_rsa::rsa_from_pkcs1_pem_file(&pem).map_err(|e| {
                format!(
                    "profile RSA {}: {e}; configure the selected local engine or LOGIN_RSAN/E",
                    pem.display()
                )
            })?
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
        if runtime {
            if let Some(identity) = &loaded.identity {
                // Public installations need no server source checkout: only a
                // compiled build/packager row can attest the supported world.
                if self.target() == BotTarget::Prod {
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
            target: self.target(),
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
        let game_data = if self.supported_server || (runtime && self.target() == BotTarget::Prod) {
            api::game_data::for_optional_profile(self.revision(), &cache_id)?.filter(|data| {
                self.target() == BotTarget::Prod
                    || data.source_inputs().all(|(content, input)| {
                        let base = if content {
                            &self.content_dir
                        } else {
                            &self.engine_dir
                        };
                        let path = base.join(&input.path);
                        std::fs::metadata(&path).is_ok_and(|m| m.len() == input.bytes)
                            && nav::manifest::hash_file(&path)
                                .is_ok_and(|hash| hash == input.sha256)
                    })
            })
        } else {
            None
        };
        Ok(Arc::new(ServerProfile {
            selection: self.selection,
            client: binding,
            cache: availability,
            cache_manifest: actual,
            runtime_cache,
            game_data,
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
            vault_path: self.vault_path.clone(),
            catalog_root: self.catalog_root.clone(),
            world_members: self.world_members.clone(),
            public_worlds: self.public_worlds.clone(),
        }))
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
        let bytes = std::fs::read(pack_path)
            .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
        counters.pack_reads = 1;
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
                let nav_hash = hash_bytes_with_progress(&bytes, |completed, total| {
                    observer.report(ProfileProgress::bytes(
                        ProfileProgressStage::CheckingNavigationFiles,
                        completed,
                        total,
                    ));
                });
                counters.pack_hashes = 1;
                let manifest_path = nav_manifest_path(pack_path);
                if !manifest_path.exists() {
                    if self.revision() == ClientRevision::R274 && content_id.is_none() {
                        let world = decode_nav_world(&bytes, pack_path, observer, &mut counters)?;
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
        let world = decode_nav_world(&bytes, pack_path, observer, &mut counters)?;
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
    let bytes = std::fs::read(&reach_path)
        .map_err(|e| format!("bundled navigation {}: {e}", reach_path.display()))?;
    counters.reach_reads = 1;
    let side = decode_reach_sidecar(&bytes)
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
    Ok(Arc::from(side.bits))
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
    let bytes = std::fs::read(&canlight_path)
        .map_err(|e| format!("bundled navigation {}: {e}", canlight_path.display()))?;
    counters.canlight_reads = 1;
    let side = decode_canlight_sidecar(&bytes).map_err(|e| match e {
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
    Ok(Arc::from(side.bits))
}

fn decode_nav_world(
    bytes: &[u8],
    pack_path: &Path,
    observer: &ProfileProgressObserver,
    counters: &mut NavLoadCounters,
) -> Result<Arc<NavWorld>, String> {
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingNavigation,
        0,
        1,
    ));
    let world = NavWorld::from_bytes(bytes)
        .map_err(|e| format!("navigation {}: {e}", pack_path.display()))?;
    counters.pack_decodes = 1;
    observer.report(ProfileProgress::steps(
        ProfileProgressStage::PreparingNavigation,
        1,
        1,
    ));
    Ok(Arc::new(world))
}

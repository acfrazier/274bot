//! Resolve launch inputs once, before shared assets, vault mutation or sockets.
//!
//! The host profile owns world/vault identity and default script paths.
//! The client receives only its immutable connection and resource binding. Legacy `PlayOptions` remains
//! available for old callers; the frontends use this checked path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use client::client::ClientConfig;
use client::io::{ClientRevision, Packet};
use client::session::{ClientSessionConfig, ClientSessionProfile};
use client::BotTarget;
use nav::manifest::hash_file;
pub use nav::manifest::{nav_manifest_path, CacheManifest, NavManifest};

const JAGS: [&str; 8] = CacheManifest::ARCHIVES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSelection {
    Local274,
    Local289,
    Public289,
}

impl ServerSelection {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "local-274" => Ok(Self::Local274),
            "local-289" => Ok(Self::Local289),
            "public-289" => Ok(Self::Public289),
            "public-274" => Err("public revision 274 is unavailable; use public-289".into()),
            _ => Err(format!(
                "unsupported server profile {name:?}; use local-274, local-289 or public-289"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Local274 => "local-274",
            Self::Local289 => "local-289",
            Self::Public289 => "public-289",
        }
    }

    pub fn revision(self) -> ClientRevision {
        match self {
            Self::Local274 => ClientRevision::R274,
            Self::Local289 | Self::Public289 => ClientRevision::R289,
        }
    }

    pub fn target(self) -> BotTarget {
        match self {
            Self::Local274 | Self::Local289 => BotTarget::Local,
            Self::Public289 => BotTarget::Prod,
        }
    }
}

pub fn parse_revision(value: &str) -> Result<ClientRevision, String> {
    match value {
        "274" => Ok(ClientRevision::R274),
        "289" => Ok(ClientRevision::R289),
        _ => Err(format!("unsupported revision {value:?}; use 274 or 289")),
    }
}

/// Explicit launch overrides. No process state changes occur while parsing.
#[derive(Debug, Clone, Default)]
pub struct ProfileOptions {
    pub profile: Option<String>,
    pub revision: Option<String>,
    pub prod: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub asset_host: Option<String>,
    pub http_port: Option<u16>,
    pub engine_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
    pub nav_pack: Option<PathBuf>,
    pub nav_flags: Option<PathBuf>,
    pub content_dir: Option<PathBuf>,
    pub vault_path: Option<PathBuf>,
    pub catalog_root: Option<PathBuf>,
    pub cache_manifest: Option<PathBuf>,
}

/// Consume shared connection/resource flags; return frontend-specific args.
/// This makes flag order independent and gives both frontends one parser.
pub fn parse_profile_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(ProfileOptions, Vec<String>), String> {
    let mut options = ProfileOptions::default();
    let mut rest = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let flag = arg.as_ref();
        if flag == "--prod" {
            options.prod = true;
            continue;
        }
        if !matches!(
            flag,
            "--profile"
                | "--revision"
                | "--host"
                | "--port"
                | "--asset-host"
                | "--http-port"
                | "--engine"
                | "--cache"
                | "--unpack"
                | "--nav-pack"
                | "--nav-flags"
                | "--content"
                | "--vault"
                | "--catalog"
                | "--cache-manifest"
        ) {
            rest.push(flag.to_string());
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let value = value.as_ref();
        if value.is_empty() || value.starts_with("--") {
            return Err(format!("{flag} needs a value"));
        }
        let port = || {
            value
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| format!("{flag} needs a port from 1 to 65535"))
        };
        match flag {
            "--profile" => options.profile = Some(value.into()),
            "--revision" => {
                parse_revision(value)?;
                options.revision = Some(value.into());
            }
            "--host" => options.host = Some(value.into()),
            "--port" => options.port = Some(port()?),
            "--asset-host" => options.asset_host = Some(value.into()),
            "--http-port" => options.http_port = Some(port()?),
            "--engine" => options.engine_dir = Some(value.into()),
            "--cache" => options.cache_dir = Some(value.into()),
            "--unpack" => options.unpack_dir = Some(value.into()),
            "--nav-pack" => options.nav_pack = Some(value.into()),
            "--nav-flags" => options.nav_flags = Some(value.into()),
            "--content" => options.content_dir = Some(value.into()),
            "--vault" => options.vault_path = Some(value.into()),
            "--catalog" => options.catalog_root = Some(value.into()),
            "--cache-manifest" => options.cache_manifest = Some(value.into()),
            _ => unreachable!(),
        }
    }
    Ok((options, rest))
}

/// Captured environment, injectable without mutating process settings in tests.
#[derive(Debug, Clone, Default)]
pub struct ProfileEnvironment {
    pub home: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub profile: Option<String>,
    pub revision: Option<String>,
    pub target: Option<String>,
    pub engine_dir: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
    pub nav_pack: Option<PathBuf>,
    pub nav_flags: Option<PathBuf>,
    pub catalog_root: Option<PathBuf>,
    pub cache_manifest: Option<PathBuf>,
    pub rsa_modulus: Option<String>,
    pub rsa_exponent: Option<String>,
}

impl ProfileEnvironment {
    pub fn capture() -> Self {
        fn value(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|s| !s.is_empty())
        }
        Self {
            home: client::operator_home().ok().map(PathBuf::from),
            working_dir: std::env::current_dir().ok(),
            profile: value("BOT_SERVER_PROFILE"),
            revision: value("BOT_REVISION"),
            target: value("BOT_TARGET"),
            engine_dir: value("ENGINE_DIR").map(PathBuf::from),
            unpack_dir: value("CLIENT_UNPACK_DIR").map(PathBuf::from),
            nav_pack: value("NAV_PACK").map(PathBuf::from),
            nav_flags: value("NAV_FLAGS").map(PathBuf::from),
            catalog_root: script::rs2b0t_root(),
            cache_manifest: value("BOT_CACHE_MANIFEST").map(PathBuf::from),
            rsa_modulus: value("LOGIN_RSAN"),
            rsa_exponent: value("LOGIN_RSAE"),
        }
    }
}

/// Launch selection, still changeable before a session is bound. It contains
/// paths and captured key inputs, but loads no cache and opens no socket.
#[derive(Debug, Clone)]
pub struct ProfileSelection {
    selection: ServerSelection,
    game_host: String,
    game_port: u16,
    asset_host: String,
    asset_port: u16,
    engine_dir: PathBuf,
    cache_dir: PathBuf,
    unpack_dir: PathBuf,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    content_dir: PathBuf,
    vault_path: PathBuf,
    catalog_root: Option<PathBuf>,
    cache_manifest: Option<PathBuf>,
    rsa_modulus: Option<String>,
    rsa_exponent: Option<String>,
}

impl ProfileOptions {
    pub fn resolve(&self, saved_revision: Option<u16>) -> Result<ProfileSelection, String> {
        self.resolve_with_env(saved_revision, &ProfileEnvironment::capture())
    }

    pub fn resolve_with_env(
        &self,
        saved_revision: Option<u16>,
        env: &ProfileEnvironment,
    ) -> Result<ProfileSelection, String> {
        let cli_revision = self.revision.as_deref().map(parse_revision).transpose()?;
        let named = self
            .profile
            .as_deref()
            .map(ServerSelection::parse)
            .transpose()?;
        let selection = if let Some(named) = named {
            if self.prod && named.target() != BotTarget::Prod {
                return Err("--prod conflicts with the selected local --profile".into());
            }
            if cli_revision.is_some_and(|rev| rev != named.revision()) {
                return Err("--revision conflicts with the selected --profile".into());
            }
            named
        } else {
            let env_named = env
                .profile
                .as_deref()
                .map(ServerSelection::parse)
                .transpose()?;
            let target = if self.prod {
                BotTarget::Prod
            } else {
                env_named.map(ServerSelection::target).unwrap_or_else(|| {
                    client::bot_target::bot_target_from_env(env.target.as_deref())
                })
            };
            let revision = if let Some(revision) = cli_revision {
                revision
            } else if let Some(named) = env_named {
                named.revision()
            } else if let Some(revision) = env.revision.as_deref() {
                parse_revision(revision)?
            } else if target == BotTarget::Prod {
                ClientRevision::R289
            } else if let Some(revision) = saved_revision {
                parse_revision(&revision.to_string())?
            } else {
                ClientRevision::R274
            };
            match (target, revision) {
                (BotTarget::Local, ClientRevision::R274) => ServerSelection::Local274,
                (BotTarget::Local, ClientRevision::R289) => ServerSelection::Local289,
                (BotTarget::Prod, ClientRevision::R274) => {
                    return Err("public revision 274 is unavailable; use revision 289".into())
                }
                (BotTarget::Prod, ClientRevision::R289) => ServerSelection::Public289,
            }
        };
        let home = env.home.clone().unwrap_or_default();
        let bot_dir = home.join(".274bot");
        let is_289 = selection.revision() == ClientRevision::R289;
        let engine_dir = self
            .engine_dir
            .clone()
            .or_else(|| env.engine_dir.clone())
            .unwrap_or_else(|| {
                home.join(if is_289 {
                    "experiments/lostcity-289/engine"
                } else {
                    "experiments/Server/engine"
                })
            });
        let unpack_dir = self
            .unpack_dir
            .clone()
            .or_else(|| env.unpack_dir.clone())
            .unwrap_or_else(|| bot_dir.join(if is_289 { "unpack-289" } else { "unpack" }));
        let cache_dir = self.cache_dir.clone().unwrap_or_else(|| {
            if selection.target() == BotTarget::Prod {
                unpack_dir.clone()
            } else {
                engine_dir.join("data/pack/client")
            }
        });
        let game_host = self
            .host
            .clone()
            .unwrap_or_else(|| client::world_host_for(selection.target()).into());
        let game_port = self.port.unwrap_or(match selection {
            ServerSelection::Local274 => 43594,
            ServerSelection::Local289 => 44594,
            ServerSelection::Public289 => 443,
        });
        let asset_host = self.asset_host.clone().unwrap_or_else(|| game_host.clone());
        let asset_port = self.http_port.unwrap_or(match selection {
            ServerSelection::Local274 => 80,
            ServerSelection::Local289 => 1080,
            ServerSelection::Public289 => 443,
        });
        if selection.target() == BotTarget::Local {
            crate::validate_play_host(&game_host, BotTarget::Local).map_err(str::to_string)?;
            crate::validate_play_host(&asset_host, BotTarget::Local).map_err(str::to_string)?;
        } else if game_host != "w1.rs2b2t.com"
            || asset_host != "w1.rs2b2t.com"
            || game_port != 443
            || asset_port != 443
        {
            return Err(
                "public-289 requires the known w1.rs2b2t.com:443 game/asset pairing".into(),
            );
        }
        if game_port == 0 || asset_port == 0 {
            return Err("profile ports must be nonzero".into());
        }
        let nav_pack = self
            .nav_pack
            .clone()
            .or_else(|| env.nav_pack.clone())
            .unwrap_or_else(|| {
                bot_dir.join(if is_289 {
                    "289/274bot.navpack"
                } else {
                    "274bot.navpack"
                })
            });
        let nav_flags = self
            .nav_flags
            .clone()
            .or_else(|| env.nav_flags.clone())
            .unwrap_or_else(|| nav_pack.with_extension("navflags"));
        let content_dir = self.content_dir.clone().unwrap_or_else(|| {
            engine_dir
                .parent()
                .unwrap_or(Path::new("."))
                .join("content")
        });
        let vault_path = self.vault_path.clone().unwrap_or_else(|| {
            bot_dir.join(match selection {
                ServerSelection::Local274 => "vault",
                ServerSelection::Local289 => "vault-289",
                ServerSelection::Public289 => "vault-prod",
            })
        });
        let working_dir = env
            .working_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(|e| format!("profile working directory: {e}"))?;
        let absolute = |path: PathBuf| {
            if path.is_absolute() {
                path
            } else {
                working_dir.join(path)
            }
        };
        Ok(ProfileSelection {
            selection,
            game_host,
            game_port,
            asset_host,
            asset_port,
            engine_dir: absolute(engine_dir),
            cache_dir: absolute(cache_dir),
            unpack_dir: absolute(unpack_dir),
            nav_pack: absolute(nav_pack),
            nav_flags: absolute(nav_flags),
            content_dir: absolute(content_dir),
            vault_path: absolute(vault_path),
            catalog_root: self
                .catalog_root
                .clone()
                .or_else(|| env.catalog_root.clone())
                .map(absolute),
            cache_manifest: self
                .cache_manifest
                .clone()
                .or_else(|| env.cache_manifest.clone())
                .map(absolute),
            rsa_modulus: env.rsa_modulus.clone(),
            rsa_exponent: env.rsa_exponent.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavAvailability {
    Unavailable(String),
    Legacy274,
    Bound,
}

/// Frozen session inputs. Getters expose no mutable connection/resource fields.
#[derive(Debug)]
pub struct ServerProfile {
    selection: ServerSelection,
    client: Arc<ClientSessionProfile>,
    cache_manifest: CacheManifest,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    nav_hash: Option<String>,
    flags_hash: Option<String>,
    nav: NavAvailability,
    content_dir: PathBuf,
    vault_path: PathBuf,
    catalog_root: Option<PathBuf>,
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
        let actual = CacheManifest::capture(self.revision().as_i32() as u16, &self.cache_dir)?;
        let declared = if let Some(path) = &self.cache_manifest {
            let bytes = std::fs::read(path)
                .map_err(|e| format!("cache manifest {}: {e}", path.display()))?;
            serde_json::from_slice::<CacheManifest>(&bytes)
                .map_err(|e| format!("cache manifest {}: {e}", path.display()))?
        } else {
            let known: Vec<CacheManifest> =
                serde_json::from_str(include_str!("known-cache-identities.json"))
                    .expect("bundled cache identities");
            known.into_iter().find(|m| m.archives == actual.archives)
                .ok_or_else(|| "cache revision is unverified; supply --cache-manifest for the prepared server/cache pairing".to_string())?
        };
        if declared != actual {
            return Err(format!(
                "cache/profile mismatch for revision {} at {}",
                self.revision().as_i32(),
                self.cache_dir.display()
            ));
        }
        let cache_id = actual.identity();
        let (nav, nav_hash, flags_hash) = self.validate_nav(&cache_id)?;
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
        let mut crcs = [0; 9];
        for (i, name) in JAGS.iter().enumerate() {
            let bytes = std::fs::read(self.cache_dir.join(name))
                .map_err(|e| format!("cache {name}: {e}"))?;
            crcs[i + 1] = Packet::getcrc(&bytes, 0, bytes.len());
        }
        let binding = Arc::new(ClientSessionProfile::new(ClientSessionConfig {
            revision: self.revision(),
            target: self.target(),
            game_host: self.game_host.clone(),
            game_port: self.game_port,
            asset_host: self.asset_host.clone(),
            asset_port: self.asset_port,
            cache_dir: self.cache_dir.clone(),
            unpack_dir: self.unpack_dir.clone(),
            rsa_modulus,
            rsa_exponent,
            expected_crc: Some(crcs),
            content_id: cache_id,
        })?);
        Ok(Arc::new(ServerProfile {
            selection: self.selection,
            client: binding,
            cache_manifest: actual,
            nav_pack: self.nav_pack.clone(),
            nav_flags: self.nav_flags.clone(),
            nav_hash,
            flags_hash,
            nav,
            content_dir: self.content_dir.clone(),
            vault_path: self.vault_path.clone(),
            catalog_root: self.catalog_root.clone(),
        }))
    }

    /// Bind the immutable profile, preserve the template loader's second
    /// resource validation, and decode the shared process resources once.
    pub fn prepare_template(&self) -> Result<Arc<crate::SharedClientTemplate>, String> {
        crate::SharedClientTemplate::load(self.bind()?)
    }

    fn validate_nav(
        &self,
        cache_id: &str,
    ) -> Result<(NavAvailability, Option<String>, Option<String>), String> {
        if !self.nav_pack.exists() {
            return Ok((
                NavAvailability::Unavailable(format!(
                    "navigation unavailable: {} has not been prepared",
                    self.nav_pack.display()
                )),
                None,
                None,
            ));
        }
        let nav_hash = hash_file(&self.nav_pack)?;
        let flags_hash = self
            .nav_flags
            .is_file()
            .then(|| hash_file(&self.nav_flags))
            .transpose()?;
        let manifest_path = nav_manifest_path(&self.nav_pack);
        if !manifest_path.exists() {
            if self.revision() == ClientRevision::R274 {
                return Ok((NavAvailability::Legacy274, Some(nav_hash), flags_hash));
            }
            return Err(format!(
                "navigation/profile mismatch: revision 289 requires {} (prepare in step 5)",
                manifest_path.display()
            ));
        }
        let bytes = std::fs::read(&manifest_path)
            .map_err(|e| format!("nav manifest {}: {e}", manifest_path.display()))?;
        let manifest: NavManifest =
            serde_json::from_slice(&bytes).map_err(|e| format!("nav manifest: {e}"))?;
        if i32::from(manifest.revision) != self.revision().as_i32()
            || manifest.cache_id != cache_id
            || manifest.nav_sha256 != nav_hash
            || manifest.flags_sha256 != flags_hash
        {
            return Err("navigation/profile mismatch: revision, cache identity or pack/flags content differs".into());
        }
        Ok((NavAvailability::Bound, Some(nav_hash), flags_hash))
    }
}

impl ServerProfile {
    pub fn selection(&self) -> ServerSelection {
        self.selection
    }
    pub fn revision(&self) -> ClientRevision {
        self.selection.revision()
    }
    pub fn target(&self) -> BotTarget {
        self.selection.target()
    }
    pub fn client(&self) -> &Arc<ClientSessionProfile> {
        &self.client
    }
    pub fn cache_id(&self) -> &str {
        self.client.content_id()
    }
    pub fn nav_pack(&self) -> &Path {
        &self.nav_pack
    }
    pub fn nav_flags(&self) -> &Path {
        &self.nav_flags
    }
    pub fn nav_availability(&self) -> &NavAvailability {
        &self.nav
    }
    pub fn content_dir(&self) -> &Path {
        &self.content_dir
    }
    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }
    /// Suggested source directory only; scripts are not revision-bound resources.
    pub fn catalog_root(&self) -> Option<&Path> {
        self.catalog_root.as_deref()
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {}:{} · revision {}",
            self.selection.name(),
            self.client.game_host(),
            self.client.game_port(),
            self.revision().as_i32()
        )
    }
    pub fn client_config(&self, members: bool, lowmem: bool) -> ClientConfig {
        self.client.client_config(members, lowmem)
    }
    pub fn require_bot_operation(&self) -> Result<(), String> {
        require_bot_operation(self.revision())
    }

    pub fn validate_resources(&self) -> Result<(), String> {
        if CacheManifest::capture(self.revision().as_i32() as u16, self.client.cache_dir())?
            != self.cache_manifest
        {
            return Err(
                "cache changed after profile binding; restart with the prepared profile".into(),
            );
        }
        if let Some(hash) = &self.nav_hash {
            if hash_file(&self.nav_pack)? != *hash {
                return Err("navigation changed after profile binding; restart required".into());
            }
        }
        if let Some(hash) = &self.flags_hash {
            if hash_file(&self.nav_flags)? != *hash {
                return Err(
                    "navigation flags changed after profile binding; restart required".into(),
                );
            }
        }
        Ok(())
    }
}

fn require_bot_operation(revision: ClientRevision) -> Result<(), String> {
    match revision {
        // Both revisions have controlled host action, selected-world navigation,
        // bank-return and Guardian proof. Retain exhaustive revision dispatch:
        // a future client revision must make an explicit qualification choice.
        ClientRevision::R274 | ClientRevision::R289 => Ok(()),
    }
}

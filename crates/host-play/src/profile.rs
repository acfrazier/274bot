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
use nav::canlight;
use nav::manifest::hash_bytes_with_progress;
pub use nav::manifest::{nav_manifest_path, CacheManifest, NavManifest};
use nav::pack::{decode_canlight_sidecar, decode_reach_sidecar, sha256_hex};
use nav::world::NavWorld;
use sha2::{Digest, Sha256};

use crate::cache::CacheAvailability;
use crate::nav_identity::{
    bundled_nav_identities, install_resource_root, select_nav_origin, BundledNavIdentity,
    NavFlagsOrigin, NavLoadCounters, NavOrigin,
};
use crate::progress::{ProfileProgress, ProfileProgressObserver, ProfileProgressStage};
use crate::public_worlds::PublicWorlds;

#[path = "profile_options.rs"]
mod options;
pub use options::{
    parse_profile_args, parse_revision, ProfileEnvironment, ProfileOptions, ServerSelection,
    WorldMembersFact, WorldMembersSource,
};
#[path = "profile_binding.rs"]
mod binding;
#[path = "profile_selection.rs"]
mod selection;

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
    nav_pack_overridden: bool,
    nav_flags_overridden: bool,
    world_members: WorldMembersFact,
    public_worlds: Option<Arc<PublicWorlds>>,
    supported_server: bool,
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
    cache: CacheAvailability,
    cache_manifest: CacheManifest,
    // Keeps owned packs/snapshot alive across every clone and bot.
    runtime_cache: Option<Arc<client::unpack::PreparedRuntimeCache>>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    nav_origin: NavOrigin,
    nav_flags_origin: NavFlagsOrigin,
    nav_identity: Option<NavManifest>,
    nav_load: NavLoadCounters,
    world: SharedWorld,
    reach: Option<Arc<[u64]>>,
    canlight: Option<Arc<[u64]>>,
    nav: NavAvailability,
    content_dir: PathBuf,
    vault_path: PathBuf,
    catalog_root: Option<PathBuf>,
    world_members: WorldMembersFact,
    public_worlds: Option<Arc<PublicWorlds>>,
}

#[derive(Clone)]
struct SharedWorld(Option<Arc<NavWorld>>);

impl std::fmt::Debug for SharedWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedWorld")
            .field(&self.0.is_some())
            .finish()
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
    pub fn public_worlds(&self) -> Option<&Arc<PublicWorlds>> {
        self.public_worlds.as_ref()
    }
    pub fn cache_id(&self) -> &str {
        self.client.content_id()
    }
    /// Facts additionally qualified by the supported server/source boundary.
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.game_data.clone()
    }
    pub fn prepared_cache(&self) -> Option<&Arc<client::unpack::PreparedRuntimeCache>> {
        self.runtime_cache.as_ref()
    }
    /// Prepared-snapshot availability for the selected cache version: cold
    /// (published by this preparation), warm (already complete), or degraded
    /// with the failing check named.
    pub fn cache_availability(&self) -> &CacheAvailability {
        &self.cache
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
    pub fn nav_origin(&self) -> &NavOrigin {
        &self.nav_origin
    }
    /// Explicit flags provenance captured at bind. Do not infer this from
    /// pack origin or path equality: an override stays external.
    pub fn nav_flags_origin(&self) -> NavFlagsOrigin {
        self.nav_flags_origin
    }
    pub fn nav_identity(&self) -> Option<&NavManifest> {
        self.nav_identity.as_ref()
    }
    pub fn nav_load_counters(&self) -> NavLoadCounters {
        self.nav_load
    }
    pub fn world(&self) -> Option<Arc<NavWorld>> {
        self.world.0.clone()
    }
    /// Bundled paint-reach bitset decoded at bind. `None` on the external
    /// path, which keeps its one-time `bake_reach`.
    pub fn reach(&self) -> Option<Arc<[u64]>> {
        self.reach.clone()
    }
    /// Bundled static canlight bitset decoded at bind. `None` on the external
    /// / legacy path (Fire fail-closed; no runtime bake).
    pub fn canlight(&self) -> Option<Arc<[u64]>> {
        self.canlight.clone()
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
    pub fn world_members(&self) -> &WorldMembersFact {
        &self.world_members
    }
    pub fn map_members(&self) -> bool {
        self.world_members.map_members()
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
        self.validate_resources_with_progress(&ProfileProgressObserver::default())
    }

    pub fn validate_resources_with_progress(
        &self,
        observer: &ProfileProgressObserver,
    ) -> Result<(), String> {
        let actual = CacheManifest::capture_with_progress(
            self.revision().as_i32() as u16,
            self.client.cache_dir(),
            |completed, total| {
                observer.report(ProfileProgress::files(
                    ProfileProgressStage::CheckingGameFiles,
                    completed,
                    total,
                ));
            },
        )?;
        if actual != self.cache_manifest {
            return Err(
                "cache changed after profile binding; restart with the prepared profile".into(),
            );
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

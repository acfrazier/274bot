//! Resolve launch inputs once, before shared assets, vault mutation or sockets.
//!
//! The host profile owns endpoints, vault paths and shared resources.
//! The client receives only its immutable connection and resource binding.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use client::client::ClientConfig;
use client::io::{ClientRevision, Packet};
use client::session::{ClientSessionConfig, ClientSessionProfile};
use client::Transport;
use nav_planner::world::NavWorld;

use crate::cache::CacheAvailability;
use crate::nav_assets::{artifact_layout, install_resource_root};
use crate::progress::{ProfileProgress, ProfileProgressObserver, ProfileProgressStage};
use crate::public_worlds::PublicWorlds;
use crate::servers::{LaunchProfile, LoginKey};

#[path = "profile_options.rs"]
mod options;
pub use options::{
    parse_profile_args, parse_revision, profile_class, ProfileClass, ProfileEnvironment,
    ProfileOptions, WorldMembersFact, WorldMembersSource,
};
#[path = "profile_binding.rs"]
mod binding;
pub use binding::{DeferredReach, TemplatePreparation, TemplateStep, TEMPLATE_STEP_WAIT};
#[path = "profile_selection.rs"]
mod selection;

/// Launch selection, still changeable before a session is bound. It contains
/// paths and captured key inputs, but loads no cache and opens no socket.
#[derive(Debug, Clone)]
pub struct ProfileSelection {
    profile: LaunchProfile,
    class: ProfileClass,
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
    /// `<home>/.274bot`, resolved once here: the root the process
    /// local-state service derives every state path from.
    state_root: PathBuf,
    catalog_root: Option<PathBuf>,
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
    Bound,
}

/// Generated facts attached for this profile's supported revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameDataStatus {
    Attached,
    /// The selection is not a supported server, so no facts were requested.
    Unsupported {
        reason: String,
    },
}

impl GameDataStatus {
    pub fn is_attached(&self) -> bool {
        matches!(self, Self::Attached)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::Attached => "attached",
            Self::Unsupported { .. } => "unsupported-server",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Attached => "generated game data loaded".into(),
            Self::Unsupported { reason } => reason.clone(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "attached": self.is_attached(),
            "reason": self.reason(),
            "detail": self.detail(),
        })
    }
}

/// Actual navigation payload reads and decodes for this shared profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NavLoadCounters {
    pub pack_reads: u32,
    pub pack_decodes: u32,
    pub reach_reads: u32,
    pub canlight_reads: u32,
}

/// Frozen session inputs. Getters expose no mutable connection/resource fields.
#[derive(Debug)]
pub struct ServerProfile {
    profile: LaunchProfile,
    class: ProfileClass,
    client: Arc<ClientSessionProfile>,
    cache: CacheAvailability,
    // Keeps owned packs/snapshot alive across every clone and bot.
    runtime_cache: Option<Arc<client::unpack::PreparedRuntimeCache>>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    game_data_status: GameDataStatus,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    nav_load: NavLoadCounters,
    world: SharedWorld,
    reach: Option<Arc<binding::DeferredReach>>,
    canlight: Option<Arc<[u64]>>,
    nav: NavAvailability,
    content_dir: PathBuf,
    vault_path: PathBuf,
    state_root: PathBuf,
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
    pub fn client(&self) -> &Arc<ClientSessionProfile> {
        &self.client
    }
    pub fn public_worlds(&self) -> Option<&Arc<PublicWorlds>> {
        self.public_worlds.as_ref()
    }
    pub fn cache_id(&self) -> &str {
        self.client.content_id()
    }
    /// Facts loaded for the selected supported revision.
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.game_data.clone()
    }
    /// Why [`Self::game_data`] is present or absent. Facts are attached only
    /// when this is [`GameDataStatus::Attached`].
    pub fn game_data_status(&self) -> &GameDataStatus {
        &self.game_data_status
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
    pub fn nav_load_counters(&self) -> NavLoadCounters {
        let mut counters = self.nav_load;
        if let Some(reach) = &self.reach {
            counters.reach_reads += reach.load_count();
        }
        counters
    }
    pub fn world(&self) -> Option<Arc<NavWorld>> {
        self.world.0.clone()
    }
    /// Process-lifetime ownership key for this loaded navigation world.
    pub fn nav_pack_id(&self) -> Option<nav_planner::contract::PackId> {
        self.world.0.as_deref().and_then(|world| {
            nav_planner::producer::identity(world)
                .ok()
                .map(|identity| identity.pack())
        })
    }
    /// Paint-reach words, decoded once on the first paint request.
    /// Routing never requests these words. Invalid payloads return `None`.
    pub fn reach(&self) -> Option<Arc<[u64]>> {
        self.reach.as_ref().and_then(|reach| reach.get())
    }
    /// Lazy reach handle. Binding a paint consumer does not decode it.
    pub fn reach_source(&self) -> Option<Arc<DeferredReach>> {
        self.reach.clone()
    }
    /// Static canlight plane decoded for the loaded world's geometry.
    pub fn canlight(&self) -> Option<Arc<[u64]>> {
        self.canlight.clone()
    }
    pub fn content_dir(&self) -> &Path {
        &self.content_dir
    }
    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }
    /// The explicit state root (`<home>/.274bot`) the process local-state
    /// service is opened at. Resolved once at selection; nothing below the
    /// composition root reads `HOME` for state.
    pub fn state_root(&self) -> &Path {
        &self.state_root
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
    /// The bound profile's world type for scripts; see
    /// [`WorldMembersFact::world_truth`].
    pub fn world_members_truth(&self) -> api::selected::Truth {
        self.world_members.world_truth()
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {}:{} · revision {}",
            self.profile.name,
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
}

fn require_bot_operation(revision: ClientRevision) -> Result<(), String> {
    match revision {
        // Both revisions have controlled host action, selected-world navigation,
        // bank-return and Guardian proof. Retain exhaustive revision dispatch:
        // a future client revision must make an explicit qualification choice.
        ClientRevision::R274 | ClientRevision::R289 => Ok(()),
    }
}

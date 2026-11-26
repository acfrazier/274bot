use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use client::client::Client;
use scenario::{
    file_scenario::{file_scenario, FileScenario},
    ScenarioSettings,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::profile::{ProfileClass, ProfileSelection};

const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Default)]
pub struct FileOptions {
    pub path: PathBuf,
    pub case: Option<String>,
    pub evidence: Option<PathBuf>,
    pub expect_engine: Option<String>,
    pub expect_content: Option<String>,
    pub expect_crc: Option<String>,
    pub seed_pack: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub(super) struct Provenance {
    profile: String,
    revision: u16,
    #[serde(skip)]
    transport: client::Transport,
    engine_mirror_head: String,
    content_mirror_head: String,
    game_endpoint: (String, u16),
    asset_endpoint: (String, u16),
    served_crc_sha256: String,
    source_sha256: String,
}

#[derive(Clone, Copy)]
pub(super) enum ClosureRoute {
    AdmittedCheckMatched,
    NoEffectWindow,
}

#[derive(Clone, Copy)]
pub(super) struct WalkContract;

impl WalkContract {
    pub(super) fn require(self, route: ClosureRoute) -> Result<(), &'static str> {
        match route {
            ClosureRoute::AdmittedCheckMatched => Ok(()),
            ClosureRoute::NoEffectWindow => {
                Err("no selected source contract certifies a silent action window")
            }
        }
    }

    pub(super) fn admits(
        self,
        snapshot: &api::snapshot::GameSnapshot,
        target: api::snapshot::WorldTile,
        radius: u16,
    ) -> bool {
        snapshot
            .tile()
            .is_some_and(|(x, z, level)| in_walk_ground(api::snapshot::WorldTile { x, z, level }))
            && in_walk_ground(target)
            && radius <= 1
    }
}

// The supported source-backed family is the open Lumbridge courtyard only.
// Elsewhere the content may install a persistent player walktrigger.
pub(crate) fn in_walk_ground(tile: api::snapshot::WorldTile) -> bool {
    (3220..=3224).contains(&tile.x) && (3216..=3220).contains(&tile.z) && tile.level == 0
}
impl Provenance {
    pub(super) fn matches(&self, bound: &crate::ServerProfile) -> bool {
        let client = bound.client();
        bound.profile_class() == ProfileClass::Local
            && bound.name() == self.profile
            && bound.selection().revision == self.revision
            && bound.transport() == self.transport
            && (client.game_host(), client.game_port())
                == (self.game_endpoint.0.as_str(), self.game_endpoint.1)
            && (client.asset_host(), client.asset_port())
                == (self.asset_endpoint.0.as_str(), self.asset_endpoint.1)
            && client
                .expected_crc()
                .is_some_and(|crc| crc_sha256(crc) == self.served_crc_sha256)
    }

    pub(super) fn walk_contract(&self) -> Option<WalkContract> {
        // MoveClickHandler.ts:26–54 uses an ordered ground-click path and
        // PathingEntity.ts:215–238 consumes the final waypoint. The contract
        // excludes unproved transport, interactions and delayed effects.
        (self.revision == 289
            && self.engine_mirror_head == "574f10f493cf39ffcc70a9b2f9a7b12b11d914e8"
            && self.content_mirror_head == "a60809082bc9f8fbdaa00c7b1a1d3fd6134c18fe")
            .then_some(WalkContract)
    }
}

/// Validated File admission. The shell passes ownership without interpreting
/// the host's compiled plan or copying its proof as a value.
pub struct FilePlan(pub(super) Box<Plan>);

pub(super) struct Plan {
    pub(super) scenario: FileScenario,
    pub(super) source: script_luau::ScenarioBody,
    pub(super) evidence: Option<PathBuf>,
    pub(super) provenance: Provenance,
    pub(super) headed: bool,
    engine_dir: PathBuf,
    content_dir: PathBuf,
}

impl FilePlan {
    pub fn load(
        options: FileOptions,
        profile: &ProfileSelection,
        headed: bool,
    ) -> Result<Self, String> {
        if profile.profile_class() != ProfileClass::Local || !profile.supported_server() {
            return Err("Luau File scenarios require a supported Local profile".into());
        }
        let revision = profile.selection().revision;
        let path = if options.path.is_relative() {
            profile
                .catalog_root()
                .map_or_else(|| options.path.clone(), |root| root.join(&options.path))
        } else {
            options.path.clone()
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("luau") {
            return Err("scenario source must have a .luau extension".into());
        }
        let original = read_source(&path)?;
        let source_sha256 = format!("{:x}", Sha256::digest(original.as_bytes()));
        let source = script_luau::scenario_source(original).map_err(|err| err.to_string())?;
        let header = source
            .header
            .into_json()
            .map_err(|error| format!("scenario_header: {error:?}"))?;
        let scenario = file_scenario(header, options.case.as_deref(), revision, headed)?;
        if !headed && (scenario.settings.capture || scenario.terminal_shot.is_some()) {
            return Err("scenario_header: this output requires a headed caller".into());
        }
        if options.seed_pack.is_some() {
            return Err(
                "extra seed packs are not admitted: use the selected engine/cache definitions"
                    .into(),
            );
        }
        let engine = mirror_head(profile.engine_dir())?;
        expected("engine mirror", &engine, options.expect_engine.as_deref())?;
        let content = mirror_head(profile.content_dir())?;
        expected(
            "content mirror",
            &content,
            options.expect_content.as_deref(),
        )?;
        let game_endpoint = (profile.game_host().to_owned(), profile.game_port());
        let asset_endpoint = (profile.asset_host().to_owned(), profile.asset_port());
        let crc =
            Client::get_jag_checksums_for(profile.transport(), &asset_endpoint.0, asset_endpoint.1)
                .map_err(|error| format!("selected profile /crc: {error}"))?;
        let served_crc_sha256 = crc_sha256(crc);
        expected(
            "served CRC SHA-256",
            &served_crc_sha256,
            options.expect_crc.as_deref(),
        )?;
        Ok(Self(Box::new(Plan {
            scenario,
            headed,
            source: source.erased,
            evidence: options.evidence,
            provenance: Provenance {
                profile: profile.name().into(),
                revision,
                transport: profile.transport(),
                engine_mirror_head: engine,
                content_mirror_head: content,
                game_endpoint,
                asset_endpoint,
                served_crc_sha256,
                source_sha256,
            },
            engine_dir: profile.engine_dir().to_path_buf(),
            content_dir: profile.content_dir().to_path_buf(),
        })))
    }

    pub fn name(&self) -> &str {
        &self.0.scenario.name
    }
    pub fn role_count(&self) -> usize {
        self.0.scenario.roles.len()
    }
    pub fn mint_accounts(&self) -> Vec<String> {
        self.0
            .scenario
            .roles
            .values()
            .map(|role| {
                let mut names = role.fresh.as_deref().map_or_else(
                    || crate::mint_live_names(1),
                    |prefix| crate::play_bootstrap::mint_live_names_with(prefix, 1),
                );
                names.pop().expect("requested one minted account")
            })
            .collect()
    }
    pub fn deadline(&self) -> Duration {
        self.0.scenario.deadline
    }
    pub fn settings(&self) -> &ScenarioSettings {
        &self.0.scenario.settings
    }
}

impl Plan {
    pub(super) fn load_fixtures(
        &self,
        profile: &crate::ServerProfile,
    ) -> Result<super::fixtures::FixtureDefinitions, String> {
        let prepared = profile
            .prepared_cache()
            .ok_or("File scenario requires a prepared runtime cache")?;
        super::fixtures::FixtureDefinitions::load(
            i32::from(self.provenance.revision),
            &self.engine_dir,
            &self.content_dir,
            &prepared.jag_dir,
            profile.profile_class(),
        )
    }
}

fn read_source(path: &Path) -> Result<Arc<str>, String> {
    let file =
        File::open(path).map_err(|error| format!("scenario source {}: {error}", path.display()))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("scenario source must be a regular file".into());
    }
    let mut source = String::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut source)
        .map_err(|error| format!("scenario source: {error}"))?;
    if source.len() as u64 > MAX_SOURCE_BYTES {
        return Err("scenario source exceeds 1 MiB".into());
    }
    Ok(source.into())
}

fn mirror_head(dir: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map_err(|error| format!("mirror provenance for {}: {error}", dir.display()))?;
    let utf8 = std::str::from_utf8(&output.stdout).map_err(|error| error.to_string())?;
    let head = utf8.trim();
    if !output.status.success()
        || !matches!(head.len(), 40 | 64)
        || !head.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("cannot resolve Git head for {}", dir.display()));
    }
    Ok(head.into())
}

fn expected(source: &str, actual: &str, expected: Option<&str>) -> Result<(), String> {
    if let Some(expected) = expected {
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!("{source}: expected {expected}, observed {actual}"));
        }
    }
    Ok(())
}

fn crc_sha256(crc: [i32; 9]) -> String {
    let mut checksum = Sha256::new();
    for word in crc {
        checksum.update(word.to_be_bytes());
    }
    format!("{:x}", checksum.finalize())
}

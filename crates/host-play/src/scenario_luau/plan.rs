#[path = "seed_pack.rs"]
pub(in crate::scenario_luau) mod seed_pack;

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

/// Where a File scenario's bytes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSource {
    /// A caller-selected `.luau` path (`--live-file`, `SCENARIO_FILE`).
    Path(PathBuf),
    /// A registry id implemented as a File scenario (`--live script_<id>`,
    /// `SCENARIO_ID`). Its bytes were embedded at build time.
    Registered(scenario::FileEntry),
}

impl FileSource {
    /// The registry File source for `id`, `None` when `id` is unknown or
    /// names a Rust table scenario.
    pub fn registered(id: &str) -> Option<Self> {
        scenario::file_entry(id).map(Self::Registered)
    }
}

#[derive(Clone, Debug)]
pub struct FileOptions {
    pub source: FileSource,
    pub case: Option<String>,
    pub evidence: Option<PathBuf>,
    pub expect_engine: Option<String>,
    pub expect_content: Option<String>,
    pub expect_crc: Option<String>,
    pub seed_pack: Option<PathBuf>,
    pub seed_pack_grant: Option<String>,
}

impl FileOptions {
    /// Options with only a source; every other input absent.
    pub fn new(source: FileSource) -> Self {
        Self {
            source,
            case: None,
            evidence: None,
            expect_engine: None,
            expect_content: None,
            expect_crc: None,
            seed_pack: None,
            seed_pack_grant: None,
        }
    }
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
    /// `registry:<id>/<file>` for an embedded entry, else the read path.
    source: String,
    source_sha256: String,
    seed_pack_sha256: Option<String>,
    seed_pack_grant: Option<String>,
}

/// The R289 walk family closes only by an admitted fresh arrival on a later
/// PLAYER_INFO than the arrival's own (LD2 §5.1 route 1); it has no silent
/// no-effect window.
#[derive(Clone, Copy)]
pub(super) struct WalkContract;

/// Revision-289 level-0 map squares `(x >> 6, z >> 6)` whose only movement
/// trigger at the pinned content head is the music `[mapzone]` entry (no
/// `[zone]`, `[zoneexit]` or `[mapzoneexit]` script) and whose map file
/// spawns no chaos druid (`pack/npc.pack:182,194`, the one npc that installs
/// a walktrigger by attacking). Each entry names its music mapzone line.
const R289_WALK_SQUARES: &[((i32, i32), &str)] = &[
    ((41, 50), "content scripts/music/scripts/move.rs2:128"),
    ((41, 51), "content scripts/music/scripts/move.rs2:129"),
    ((49, 50), "content scripts/music/scripts/move.rs2:482"),
    ((49, 51), "content scripts/music/scripts/move.rs2:483"),
    ((50, 49), "content scripts/music/scripts/move.rs2:554"),
    ((50, 50), "content scripts/music/scripts/move.rs2:555"),
    ((50, 51), "content scripts/music/scripts/move.rs2:556"),
    ((50, 52), "content scripts/music/scripts/move.rs2:557"),
    ((50, 53), "content scripts/music/scripts/move.rs2:558"),
];

/// Closed single doors a File walk may open on its route: loc type and tile.
/// East Ardougne `loc_1530` (wall_diagonal) at (2669,3316).
const R289_WALK_DOORS: &[(i32, (i32, i32, i32))] = &[(1530, (2669, 3316, 0))];

/// Citations at the pinned R289 heads.
pub(super) const WALK_CITATIONS: &[&str] = &[
    "engine MoveClickHandler.ts:12-15,30-50: a delayed player is refused; else the pending action clears, the client path is queued and any walktrigger runs",
    "engine PathingEntity.ts:206-239: each step consumes waypoints; the last waypoint ends the route",
    "engine NetworkPlayer.ts:238-281, World.ts:1101-1110: map-square and zone changes enqueue triggers in updateMap, before that cycle's PLAYER_INFO",
    "engine Player.ts:580-615,660-670, World.ts:699-717: those ENGINE queue entries run in the next cycle, before movement",
    "content scripts/music/scripts/music.rs2:83-109: music_playbyregion only sets a music varp and plays a song; no delay, queue or timer",
    "content walktrigger installers (player/scripts/movecheck.rs2:1-6, skill_thieving/scripts/thieving.rs2:51, npc/scripts/chaos_druid.rs2:47-55, skill_combat/scripts/npc/npc_combat_magic.rs2:7-12,47-50): no File action contract installs one, no admitted square spawns a chaos druid, and no npc_cast_spell site selects a freeze spell (magic_combat_spells.dbrow:319,336,354)",
    "content scripts/doors/configs/doors.loc:45-57, scripts/doors/scripts/doors.rs2:1,6-20: oploc1 on loc_1530 runs ~open_door(500); the player effect is at most p_teleport and p_delay(1); the 500-tick revert changes only the loc",
    "engine OpLocHandler.ts:14-18,46-56: a delayed player or an invalid op is refused before the door script",
];

impl WalkContract {
    pub(super) fn admits(
        self,
        snapshot: &api::snapshot::GameSnapshot,
        target: api::snapshot::WorldTile,
        radius: u16,
    ) -> Result<(), &'static str> {
        let here = snapshot
            .tile()
            .map(|(x, z, level)| api::snapshot::WorldTile { x, z, level })
            .ok_or("walk source tile is unknown")?;
        if !in_walk_ground(here) || !in_walk_ground(target) {
            return Err("walk source or target lies outside the audited revision-289 ground");
        }
        if radius > 1 {
            return Err("walk radius above 1 has no arrival contract");
        }
        // A stun or freeze walktrigger comes only with combat or a thieving
        // failure; an actor already in combat has no closed walk family.
        if snapshot
            .local_player()
            .is_none_or(|player| player.player.actor.in_combat)
        {
            return Err("walk source actor is unknown or in combat");
        }
        Ok(())
    }
}

/// Whether `tile` lies in the audited revision-289 walk ground.
pub(crate) fn in_walk_ground(tile: api::snapshot::WorldTile) -> bool {
    tile.level == 0
        && R289_WALK_SQUARES
            .iter()
            .any(|(square, _)| *square == (tile.x >> 6, tile.z >> 6))
}

/// Whether a route may open this door loc at this tile (option 1, Open).
pub(crate) fn walk_door(loc: i32, tile: api::snapshot::WorldTile, option: i32) -> bool {
    option == 1
        && R289_WALK_DOORS
            .iter()
            .any(|(id, at)| *id == loc && *at == (tile.x, tile.z, tile.level))
}

/// The music mapzone line that admits `tile`'s map square, for the receipt.
pub(super) fn walk_ground_citation(tile: api::snapshot::WorldTile) -> Option<&'static str> {
    R289_WALK_SQUARES
        .iter()
        .find(|(square, _)| tile.level == 0 && *square == (tile.x >> 6, tile.z >> 6))
        .map(|(_, citation)| *citation)
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

    /// The R289 walk family exists only at the pinned lab heads; elsewhere
    /// no citation in [`WALK_CITATIONS`] was read.
    pub(super) fn walk_contract(&self) -> Option<WalkContract> {
        (self.revision == 289
            && self.engine_mirror_head == super::contracts::R289_ENGINE
            && self.content_mirror_head == super::contracts::R289_CONTENT)
            .then_some(WalkContract)
    }

    pub(super) fn action_contract(
        &self,
        action: super::contracts::FileAction,
        snapshot: &api::snapshot::GameSnapshot,
    ) -> Result<super::contracts::Contract, String> {
        super::contracts::select(
            super::contracts::Heads {
                revision: self.revision,
                engine: &self.engine_mirror_head,
                content: &self.content_mirror_head,
            },
            action,
            snapshot,
        )
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
    pub(super) seed_pack: Option<seed_pack::SeedPack>,
    pub(super) seed_pack_grant: Option<String>,
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
        let (original, source_identity) = match &options.source {
            FileSource::Path(path) => {
                let path = if path.is_relative() {
                    profile
                        .catalog_root()
                        .map_or_else(|| path.clone(), |root| root.join(path))
                } else {
                    path.clone()
                };
                if path.extension().and_then(|ext| ext.to_str()) != Some("luau") {
                    return Err("scenario source must have a .luau extension".into());
                }
                (read_source(&path)?, path.display().to_string())
            }
            FileSource::Registered(entry) => {
                if entry.source.len() as u64 > MAX_SOURCE_BYTES {
                    return Err("registered scenario source exceeds 1 MiB".into());
                }
                (
                    Arc::<str>::from(entry.source),
                    format!("registry:{}/{}", entry.name, entry.file_name),
                )
            }
        };
        let source_sha256 = format!("{:x}", Sha256::digest(original.as_bytes()));
        let source = script_luau::scenario_source(original).map_err(|err| err.to_string())?;
        let header = source
            .header
            .into_json()
            .map_err(|error| format!("scenario_header: {error:?}"))?;
        let scenario = file_scenario(header, options.case.as_deref(), revision, headed)?;
        let seed_pack = options
            .seed_pack
            .as_deref()
            .map(seed_pack::SeedPack::load)
            .transpose()?;
        if seed_pack.is_none() && options.seed_pack_grant.is_some() {
            return Err("a seed pack grant requires --seed-pack".into());
        }
        if let Some(pack) = &seed_pack {
            pack.validate_admission(scenario.confirmed_only, options.seed_pack_grant.as_deref())?;
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
                source: source_identity,
                source_sha256,
                seed_pack_sha256: seed_pack.as_ref().map(|pack| pack.sha256().to_owned()),
                seed_pack_grant: options.seed_pack_grant.clone(),
            },
            engine_dir: profile.engine_dir().to_path_buf(),
            content_dir: profile.content_dir().to_path_buf(),
            seed_pack,
            seed_pack_grant: options.seed_pack_grant,
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
    pub fn seed_pack_sha256(&self) -> Option<&str> {
        self.0.seed_pack.as_ref().map(seed_pack::SeedPack::sha256)
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

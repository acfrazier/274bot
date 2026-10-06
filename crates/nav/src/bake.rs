//! Shared world bake: door ids, loc defs, the whole-world collision, the
//! transport graph, the bank stand table, the v16 pack bytes, the raw flags
//! sidecar, the paint-reach sidecar, the static canlight sidecar and the bound
//! manifest. Both frontends of this logic call
//! [`bake_world`] — the `nav-pack` developer CLI and the application build
//! (`host-play`'s build script) — so the derivation exists once.
//!
//! Revision-bound bakes verify the selected cache before baking (see
//! [`verify_cache_manifest`]); the cache is a bake input, never a shipped
//! resource.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::Cache;
use client::dash3d::{CollisionFlag, LocAngle, LocShape};
use client::io::JagFile;
use sha2::{Digest, Sha256};

use crate::canlight::{self, BANK_ZONES_REL};
use crate::collision::{bake_from_maps, WorldCollision};
use crate::manifest::{CacheManifest, NavManifest};
use crate::pack::{
    derive_banks, encode, encode_canlight_sidecar, encode_flags_sidecar, encode_reach_sidecar,
    sha256_hex, FORMAT_ID,
};
use crate::paint::bake_reach;
use crate::transport::{
    assert_transmitted_varp_reqs, derive_transports_for_bake, require_members_guards,
    require_wilderness_teleport_legality,
};
use crate::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable, NO_GROUP, NO_SHAPE};

/// Door loc configs under `content/scripts/doors/configs`.
pub const DOOR_CONFIGS: [&str; 3] = ["doors.loc", "doubledoors.loc", "opened_doors.loc"];

/// Manual generator identity. Bump when the bake's semantics change in a way
/// [`FORMAT_ID`] does not already capture (a pack format bump changes the
/// format identity instead).
pub const GENERATOR_ID: &str = "nav-bake-2";
pub use crate::map::services::{pois_generator_identity, POIS_GENERATOR_SOURCES};

/// Baker sources whose bytes join the generator identity: a generated
/// artifact is stale after any change to one of them. Paths are relative to
/// the `nav` crate root; keep this list to the code that decides artifact
/// bytes. Pack/flags come from bake/collision/pack/transport and zone
/// derivation; reach bits also depend on `paint.rs` (`bake_reach`) and
/// `router.rs` (`step_ok`). Traveller and grid-search changes do not decide
/// those bytes.
pub const GENERATOR_SOURCES: [&str; 48] = [
    "src/bake.rs",
    "../api/src/line_of_sight.rs",
    "src/canlight.rs",
    "src/collision.rs",
    "src/map/services.rs",
    "src/pack/zones.rs",
    "src/pack.rs",
    "src/paint.rs",
    "src/quest_gates.rs",
    "src/router.rs",
    "src/transport.rs",
    "src/transport/condparse.rs",
    "src/transport/index.rs",
    "src/transport/script_text.rs",
    "src/pack/config_parse.rs",
    "src/pack/mapsquare.rs",
    "src/pack/banks.rs",
    "src/pack/sidecars.rs",
    "src/transport/gates.rs",
    "src/transport/quest_doors.rs",
    "src/transport/doors.rs",
    "src/transport/door_members.rs",
    "src/transport/brass_key.rs",
    "src/transport/membergate.rs",
    "src/transport/members_guard.rs",
    "src/transport/webs.rs",
    "src/transport/vertical.rs",
    "src/transport/shortcuts.rs",
    "src/transport/static_routes.rs",
    "src/transport/npc_hops.rs",
    "src/transport/observable.rs",
    "src/transport/gliders.rs",
    "src/transport/scripted_doors.rs",
    "src/transport/spirit_trees.rs",
    "src/transport/levers.rs",
    "src/transport/toll.rs",
    "src/transport/magic_guild.rs",
    "src/transport/ranging_guild.rs",
    "src/transport/zanaris.rs",
    "src/transport/teleports.rs",
    "src/transport/wilderness.rs",
    "src/transport/rs2_syntax.rs",
    "src/transport/stage_doors.rs",
    "src/transport/engine_door_procs.rs2",
    "src/transport/stage_doors_forced.rs",
    "src/transport/engine_forced_procs.rs2",
    "src/zones.rs",
    "src/zones/curated.rs",
];

/// Digest of the bake generator: the manual id, the pack format identity and
/// the bytes of [`GENERATOR_SOURCES`] as read by the caller (the build
/// script), so this stays a pure function of its inputs.
pub fn generator_identity(sources: &[(&str, &str)]) -> String {
    let mut digest = Sha256::new();
    digest.update(GENERATOR_ID.as_bytes());
    digest.update([0]);
    digest.update(FORMAT_ID.as_bytes());
    for (label, text) in sources {
        digest.update([0]);
        digest.update(label.as_bytes());
        digest.update([0]);
        digest.update(text.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// `content/` inputs of a revision-bound bake. `gates.loc` lives under the
/// content tree, sibling of `maps/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentInputs {
    pub maps_dir: PathBuf,
    pub doors_dir: PathBuf,
    pub gates: PathBuf,
}

/// Resolve the three content inputs of a bake from a `content/` root.
pub fn content_inputs(content_dir: &Path) -> ContentInputs {
    ContentInputs {
        maps_dir: content_dir.join("maps"),
        doors_dir: content_dir.join("scripts/doors/configs"),
        gates: content_dir.join("scripts/general_use/configs/gates.loc"),
    }
}

/// The loc-definition jag of a revision-bound cache directory: revision 289
/// keeps `config` inside the cache directory, revision 274 reads the server
/// `data/pack/config` beside the `client/` cache directory.
pub fn config_jag_for(revision: u16, cache_dir: &Path) -> Result<PathBuf, String> {
    if revision == 289 {
        Ok(cache_dir.join("config"))
    } else {
        cache_dir
            .parent()
            .map(|parent| parent.join("config"))
            .ok_or_else(|| "revision 274 cache path has no data/pack parent".to_string())
    }
}

/// Load and verify a bound cache manifest: manifest revision, every cache
/// archive's bytes, and the selected loc-definition jag belonging to that
/// verified cache.
pub fn verify_cache_manifest(
    revision: u16,
    cache_dir: &Path,
    manifest_path: &Path,
    config_jag: &Path,
) -> Result<CacheManifest, String> {
    let bytes = std::fs::read(manifest_path)
        .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
    let manifest: CacheManifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
    manifest.verify(revision, cache_dir)?;
    let expected_config = manifest
        .archives
        .get("config")
        .ok_or_else(|| "cache manifest has no config archive".to_string())?;
    let actual_config = crate::manifest::hash_file(config_jag)?;
    if &actual_config != expected_config {
        return Err(format!(
            "selected config {} does not belong to the verified revision {revision} cache",
            config_jag.display()
        ));
    }
    Ok(manifest)
}

/// Verify a bound cache manifest using hashes already captured for a caller's
/// complete input snapshot.
pub fn verify_cache_manifest_from_fingerprints(
    revision: u16,
    cache_dir: &Path,
    manifest_path: &Path,
    config_jag: &Path,
    inputs: &[crate::bundle::InputFingerprint],
) -> Result<CacheManifest, String> {
    let bytes = std::fs::read(manifest_path)
        .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
    let manifest: CacheManifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
    if manifest.revision != revision {
        return Err(format!(
            "cache manifest revision {} does not match selected revision {revision}",
            manifest.revision
        ));
    }
    let captured = crate::bundle::cache_manifest_from_fingerprints(revision, cache_dir, inputs)?;
    if manifest != captured {
        return Err(format!(
            "cache manifest does not match cache bytes at {}",
            cache_dir.display()
        ));
    }
    let expected_config = manifest
        .archives
        .get("config")
        .ok_or_else(|| "cache manifest has no config archive".to_string())?;
    let actual_config = &crate::bundle::fingerprint_for_path(inputs, config_jag)?.sha256;
    if actual_config != expected_config {
        return Err(format!(
            "selected config {} does not belong to the verified revision {revision} cache",
            config_jag.display()
        ));
    }
    Ok(manifest)
}

/// Strict read-only decoded identity for offline packaging. The snapshot
/// must match the selected versionlist; no endpoint is contacted.
pub fn decoded_identity(
    revision: u16,
    cache_dir: &Path,
    snapshot_root: &Path,
) -> Result<String, String> {
    let versionlist = std::fs::read(cache_dir.join("versionlist")).map_err(|e| e.to_string())?;
    let version = client::unpack::version_hash(&versionlist);
    let before = CacheManifest::capture(revision, cache_dir)?;
    let result = client::content_identity::compute_decoded_content_identity(
        revision,
        cache_dir,
        snapshot_root.join(version),
    )
    .map_err(|e| e.to_string())?
    .content_id_hex();
    if before != CacheManifest::capture(revision, cache_dir)? {
        return Err("cache changed during offline identity preparation".into());
    }
    Ok(result)
}
/// Compute a decoded identity after verifying the cache against a captured
/// input-fingerprint set. The decoded snapshot is not in that set, so callers
/// must recompute and compare this identity before publishing.
pub fn decoded_identity_from_snapshot(
    revision: u16,
    cache_dir: &Path,
    snapshot_root: &Path,
    inputs: &[crate::bundle::InputFingerprint],
    expected_manifest: &CacheManifest,
) -> Result<String, String> {
    let manifest = crate::bundle::cache_manifest_from_fingerprints(revision, cache_dir, inputs)?;
    if &manifest != expected_manifest {
        return Err("cache inputs changed before offline identity preparation".into());
    }
    let versionlist_path = cache_dir.join("versionlist");
    let versionlist = std::fs::read(&versionlist_path).map_err(|e| e.to_string())?;
    let versionlist_fingerprint = crate::bundle::fingerprint_for_path(inputs, &versionlist_path)?;
    if crate::manifest::hash_bytes(&versionlist) != versionlist_fingerprint.sha256 {
        return Err("cache versionlist changed during offline identity preparation".into());
    }
    let version = client::unpack::version_hash(&versionlist);
    Ok(client::content_identity::compute_decoded_content_identity(
        revision,
        cache_dir,
        snapshot_root.join(version),
    )
    .map_err(|e| e.to_string())?
    .content_id_hex())
}

/// One bake request: canonical inputs for a single revision.
pub struct BakeRequest<'a> {
    /// `Some(revision)` binds the pack to a verified cache identity;
    /// `None` is the legacy unmanifested 274 bake.
    pub revision: Option<u16>,
    pub maps_dir: &'a Path,
    pub doors_dir: &'a Path,
    pub gates: &'a Path,
    pub config_jag: &'a Path,
    /// The verified cache manifest that goes into the sidecar manifest.
    pub cache: Option<&'a CacheManifest>,
    /// A missing required config would bake a world that silently disagrees
    /// with the server. Only explicitly diagnostic callers should skip them.
    pub require_all_door_configs: bool,
    /// Caller-owned complete input snapshot, when the caller also owns the
    /// final stability check (the application build path).
    pub input_fingerprints: Option<&'a [crate::bundle::InputFingerprint]>,
    /// Decoded cache identity bound into navpois; required for a sidecar.
    pub content_id: Option<&'a str>,
}

/// What one bake produced, in the order the CLI summarises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BakeSummary {
    pub mapsquares: usize,
    pub width: usize,
    pub height: usize,
    pub walkable: usize,
    pub edges: usize,
    pub banks: usize,
}

/// The bytes and identity of one bake.
pub struct BakedNav {
    pub pack: Vec<u8>,
    pub flags: Vec<u8>,
    pub reach: Vec<u8>,
    pub canlight: Vec<u8>,
    pub pois: Option<Vec<u8>>,
    /// Hex of the canlight policy digest (algorithm + revision + bank_zones).
    pub canlight_identity: String,
    pub manifest: Option<NavManifest>,
    pub summary: BakeSummary,
    /// Non-fatal notes the caller reports (skipped door configs).
    pub notes: Vec<String>,
}

struct DerivedZones {
    table: ZoneTable,
    npc_count: usize,
    npc_kind_count: usize,
    always_count: usize,
    level_rule_count: usize,
    shaped_count: usize,
    total_npc_spawns: usize,
}

struct PendingZone {
    zone: Zone,
    npc_id: i32,
    shape_bits: Option<u64>,
    carve_visibility: bool,
    mobile_reach: Option<MobileReach>,
}

/// Engine facts that bound where a moving hunter can acquire a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MobileReach {
    /// NPC footprint side (`npc.size`).
    size: i32,
    /// Chebyshev bound on the NPC's south-west tile from its spawn: the larger
    /// of its wander box and the farthest a valid chase can carry it.
    move_radius: i32,
    /// `huntrange`, measured from the NPC's south-west tile.
    hunt_range: i32,
    /// The hunt mode requires `check_vis=lineofsight`.
    line_of_sight: bool,
    /// The collision rule the engine steps this NPC under.
    strategy: StepStrategy,
}

/// Engine `CollisionType` of a walking NPC (`PathingEntity.getCollisionStrategy`),
/// tested per tile by `CollisionStrategy.canMove`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepStrategy {
    /// `normal` and `passthru`, and the stand-in for `indoors`/`outdoors`:
    /// those add a roof test to the same mask test, and the bake carries no
    /// roof flag, so NORMAL reaches a superset of their tiles.
    Normal,
    /// `blocked`: only onto blocked ground (`FLOOR`), still stopped by walls,
    /// scenery and ground decor.
    Blocked,
    /// `blocked+normal` (`CollisionType.LINE_OF_SIGHT`): only projectile-
    /// blocking walls and scenery stop it; ground and ordinary scenery don't.
    LineOfSight,
}

impl StepStrategy {
    fn of(restrict: crate::map::services::MoveRestrict) -> Option<Self> {
        use crate::map::services::MoveRestrict;
        match restrict {
            MoveRestrict::Normal
            | MoveRestrict::Passthru
            | MoveRestrict::Indoors
            | MoveRestrict::Outdoors => Some(Self::Normal),
            MoveRestrict::Blocked => Some(Self::Blocked),
            MoveRestrict::BlockedNormal => Some(Self::LineOfSight),
            MoveRestrict::Nomove => None,
        }
    }

    /// `CollisionStrategy.canMove(collision, tileFlag, blockFlag)`.
    fn can_move(self, tile: u32, block: u32) -> bool {
        const FLOOR: u32 = 0x20_0000;
        // Walls (`0xff`) and scenery (`0x100`); `<< 9` turns each into its
        // projectile-blocker bit.
        const LINE_OF_SIGHT_MOVEMENT: u32 = 0x1ff;
        match self {
            Self::Normal => tile & block == 0,
            Self::Blocked => tile & block & !FLOOR == 0 && tile & FLOOR != 0,
            Self::LineOfSight => tile & ((block & LINE_OF_SIGHT_MOVEMENT) << 9) == 0,
        }
    }
}

fn derive_zone_table(
    content_root: &Path,
    collision: &WorldCollision,
    graph: &crate::transport::TransportGraph,
    npc_types: &[client::config::NpcType],
    door_ids: &HashSet<i32>,
    opened_door_ids: &HashSet<i32>,
) -> Result<DerivedZones, String> {
    let inputs = crate::map::services::collect_hunter_inputs(
        content_root,
        npc_types,
        door_ids,
        opened_door_ids,
    )?;
    let definitions: HashMap<_, _> = inputs
        .definitions
        .iter()
        .map(|definition| (definition.npc_id, definition))
        .collect();

    let mut used_ids = HashSet::new();
    for spawn in &inputs.spawns {
        if definitions
            .get(&spawn.npc_id)
            .is_some_and(|definition| definition.huntrange > 0)
        {
            used_ids.insert(spawn.npc_id);
        }
    }
    let mut kinds = Vec::with_capacity(used_ids.len() + crate::zones::curated::HAZARDS.len());
    let mut kind_indices = HashMap::with_capacity(used_ids.len());
    for definition in &inputs.definitions {
        if !used_ids.contains(&definition.npc_id) {
            continue;
        }
        let vislevel = u16::try_from(definition.vislevel)
            .map_err(|_| format!("hunter NPC {} has invalid combat level", definition.id))?;
        let kind_index = u16::try_from(kinds.len())
            .map_err(|_| "zone kind count exceeds the packed limit".to_string())?;
        kind_indices.insert(definition.npc_id, kind_index);
        kinds.push(ZoneKind::new(
            definition.id.as_str(),
            format!("{} (L{})", definition.display_name, definition.vislevel),
            definition.npc_id,
            vislevel,
            definition
                .find_newmode
                .to_ascii_lowercase()
                .starts_with("applayer"),
            definition.vis_off,
        ));
    }

    let open_door_faces = openable_door_faces(&inputs.openable_doors)?;
    let mut pending = Vec::with_capacity(used_ids.len());
    for spawn in &inputs.spawns {
        let Some(definition) = definitions.get(&spawn.npc_id).copied() else {
            return Err(format!(
                "map spawn references hunter id {} without a config",
                spawn.npc_id
            ));
        };
        if definition.huntrange < 1 {
            continue;
        }
        // NPCs hunt on their raw spawn plane even on link-below tiles.
        // The engine remaps loc/land collision, not GameMap::loadNpcs.
        let class = match definition.check_nottoostrong.to_ascii_lowercase().as_str() {
            "off" => ZoneClass::Always,
            "outside_wilderness" => ZoneClass::LevelRule,
            other => {
                return Err(format!(
                    "hunter NPC {} has unsupported check_nottoostrong={other}",
                    definition.id
                ))
            }
        };
        let cap = if class == ZoneClass::Always {
            u16::MAX
        } else {
            u16::try_from(definition.vislevel)
                .ok()
                .and_then(|level| level.checked_mul(2))
                .ok_or_else(|| format!("hunter NPC {} has invalid combat cap", definition.id))?
        };
        let trigger = definition.find_newmode.to_ascii_lowercase();
        let ap = if trigger.starts_with("applayer") {
            true
        } else if trigger.starts_with("opplayer") {
            false
        } else {
            return Err(format!(
                "hunter NPC {} has unsupported find_newmode={trigger}",
                definition.id
            ));
        };
        let kind = *kind_indices
            .get(&spawn.npc_id)
            .ok_or_else(|| format!("hunter NPC {} has no kind row", definition.id))?;
        let tile = WorldTile {
            x: spawn.x,
            z: spawn.z,
            level: i32::from(spawn.level),
        };
        let size = definition.size;
        if size < 1 {
            return Err(format!(
                "hunter NPC {} has invalid size {size}",
                definition.id
            ));
        }
        let (zone, shape_bits, mobile_reach) = if definition.stationary() && !ap {
            let width = u8::try_from(size)
                .ok()
                .filter(|size| *size <= 6)
                .ok_or_else(|| {
                    format!(
                        "stationary melee hunter {} exceeds the 6x6 footprint limit",
                        definition.id
                    )
                })?;
            let bits = stationary_melee_shape(collision, tile, width, &open_door_faces)
                .map_err(|error| format!("hunter NPC {}: {error}", definition.id))?;
            (
                Zone::shaped_npc(tile, width, width, class, cap, kind, NO_SHAPE),
                Some(bits),
                None,
            )
        } else if definition.stationary() && ap {
            let (min_x, min_z, max_x, max_z) =
                stationary_ranged_bounds(tile, size, definition.huntrange, definition.attackrange)
                    .map_err(|error| format!("hunter NPC {}: {error}", definition.id))?;
            let mut zone = Zone::npc(tile, 0, class, cap, kind);
            zone.min_x = min_x;
            zone.min_z = min_z;
            zone.max_x = max_x;
            zone.max_z = max_z;
            (zone, None, None)
        } else {
            let wander = if definition.never_wanders {
                0
            } else {
                definition.wanderrange
            };
            let acquisition = wander.checked_add(definition.huntrange).ok_or_else(|| {
                format!("hunter NPC {} acquisition radius overflows", definition.id)
            })?;
            let tether_range = if ap { definition.attackrange } else { 1 };
            let tether = definition
                .maxrange
                .checked_add(tether_range)
                .ok_or_else(|| format!("hunter NPC {} tether range overflows", definition.id))?;
            let radius = u8::try_from(acquisition.min(tether))
                .map_err(|_| format!("hunter NPC {} radius exceeds u8", definition.id))?;
            // A chase holds the NPC's footprint next to a target that is still
            // inside the tether, and a non-wandering NPC stays wherever its last
            // chase ended, so it can hunt from anywhere in that envelope.
            let move_radius = tether
                .checked_add(size)
                .ok_or_else(|| format!("hunter NPC {} chase radius overflows", definition.id))?
                .max(wander);
            let strategy = StepStrategy::of(definition.move_restrict).ok_or_else(|| {
                format!("hunter NPC {} walks but has no step rule", definition.id)
            })?;
            let reach = MobileReach {
                size,
                move_radius,
                hunt_range: definition.huntrange,
                line_of_sight: definition.check_lineofsight,
                strategy,
            };
            (Zone::npc(tile, radius, class, cap, kind), None, Some(reach))
        };
        // The engine checks LOS from the NPC's current tile. Keep positive-
        // wander hunters conservative; carve only for nomove or zero-wander
        // hunters around their spawn tile.
        let carve_visibility = ap
            && definition.check_lineofsight
            && (definition.stationary() || definition.wanderrange == 0);
        pending.push(PendingZone {
            zone,
            npc_id: spawn.npc_id,
            shape_bits,
            carve_visibility,
            // Spawn-centred visibility carving already narrows zero-wander
            // ranged hunters; every other moving hunter gets its reach flood.
            mobile_reach: mobile_reach.filter(|_| !carve_visibility),
        });
    }
    pending.sort_unstable_by_key(|row| {
        (
            row.zone.level,
            row.zone.spawn_z,
            row.zone.spawn_x,
            row.npc_id,
        )
    });

    let npc_count = pending.len();
    let always_count = pending
        .iter()
        .filter(|row| row.zone.class == ZoneClass::Always)
        .count();
    let level_rule_count = npc_count - always_count;
    let mut zones = Vec::with_capacity(npc_count + crate::zones::curated::HAZARDS.len());
    let mut shapes = Vec::new();
    let mut carves = Vec::new();
    for mut row in pending {
        if let Some(bits) = row.shape_bits {
            row.zone.shape = u16::try_from(shapes.len())
                .map_err(|_| "zone shape count exceeds the packed limit".to_string())?;
            shapes.push(bits);
        }
        let level = i32::from(row.zone.level);
        let (min_x, min_z, max_x, max_z) = (
            row.zone.min_x,
            row.zone.min_z,
            row.zone.max_x,
            row.zone.max_z,
        );
        // Closed openable doors are not permanent cover or walls. Keep the
        // conservative rectangle wherever such a door could affect LOS or
        // the hunter's movement.
        if row.carve_visibility
            && !openable_door_within(&inputs.openable_doors, level, min_x, min_z, max_x, max_z)
        {
            append_ranged_visibility_carves(collision, &row.zone, zones.len(), &mut carves)?;
        }
        if let Some(reach) = row.mobile_reach {
            // The hunter can stand anywhere within its move radius, so a door
            // that could open or close a passage for it anywhere there counts.
            let envelope = reach.move_radius + reach.size;
            let (min_x, min_z, max_x, max_z) = (
                min_x.min(row.zone.spawn_x - envelope),
                min_z.min(row.zone.spawn_z - envelope),
                max_x.max(row.zone.spawn_x + envelope),
                max_z.max(row.zone.spawn_z + envelope),
            );
            if !openable_door_within(&inputs.openable_doors, level, min_x, min_z, max_x, max_z)
                && !openable_door_within(&inputs.opened_doors, level, min_x, min_z, max_x, max_z)
            {
                append_mobile_reach_carves(collision, &row.zone, reach, zones.len(), &mut carves)?;
            }
        }
        zones.push(row.zone);
    }

    let mut groups = Vec::with_capacity(crate::zones::curated::GROUPS.len());
    for spec in crate::zones::curated::GROUPS {
        let group_index = u16::try_from(groups.len())
            .map_err(|_| "zone group count exceeds the packed limit".to_string())?;
        let mut members = Vec::new();
        for (index, zone) in zones.iter_mut().enumerate() {
            let npc_id = kinds[usize::from(zone.kind)].npc_id;
            if spec.npc_ids.contains(&npc_id)
                && spec.rect.contains(WorldTile {
                    x: zone.spawn_x,
                    z: zone.spawn_z,
                    level: i32::from(zone.level),
                })
            {
                if zone.group != NO_GROUP {
                    return Err(format!(
                        "zone {}@{},{},{} belongs to more than one curated group",
                        kinds[usize::from(zone.kind)].id,
                        zone.spawn_x,
                        zone.spawn_z,
                        zone.level
                    ));
                }
                zone.group = group_index;
                members.push(
                    u16::try_from(index)
                        .map_err(|_| "zone count exceeds the packed limit".to_string())?,
                );
            }
        }
        if members.is_empty() {
            return Err(format!(
                "curated zone group {} matched no hunter spawn",
                spec.id
            ));
        }
        groups.push(ZoneGroup::new(
            spec.id,
            spec.label,
            spec.rect,
            members.into_boxed_slice(),
        ));
    }

    let npc_kind_count = kinds.len();
    for hazard in crate::zones::curated::HAZARDS {
        let kind_index = u16::try_from(kinds.len())
            .map_err(|_| "zone kind count exceeds the packed limit".to_string())?;
        kinds.push(ZoneKind::new(hazard.id, hazard.label, -1, 0, false, false));
        zones.push(Zone::hazard(hazard.rect, hazard.level, kind_index));
    }
    let table = ZoneTable::from_parts(
        zones,
        kinds,
        groups,
        carves,
        shapes,
        collision.origin,
        u32::try_from(collision.width).map_err(|_| "zone grid width exceeds u32".to_string())?,
        u32::try_from(collision.height).map_err(|_| "zone grid height exceeds u32".to_string())?,
        &graph.wilderness,
    )
    .map_err(|error| format!("invalid derived zone table: {error}"))?;
    Ok(DerivedZones {
        npc_count,
        npc_kind_count,
        always_count,
        level_rule_count,
        shaped_count: table.shapes().len(),
        total_npc_spawns: inputs.total_npc_spawns,
        table,
    })
}

fn stationary_ranged_bounds(
    spawn: WorldTile,
    size: i32,
    hunt_range: i32,
    attack_range: i32,
) -> Result<(i32, i32, i32, i32), String> {
    let footprint_max_x = spawn
        .x
        .checked_add(size - 1)
        .ok_or_else(|| "stationary hunter footprint overflows x".to_string())?;
    let footprint_max_z = spawn
        .z
        .checked_add(size - 1)
        .ok_or_else(|| "stationary hunter footprint overflows z".to_string())?;
    let min_x = spawn
        .x
        .checked_sub(hunt_range)
        .ok_or_else(|| "stationary hunter acquisition bounds overflow x".to_string())?
        .max(
            spawn
                .x
                .checked_sub(attack_range)
                .ok_or_else(|| "stationary hunter attack bounds overflow x".to_string())?,
        );
    let min_z = spawn
        .z
        .checked_sub(hunt_range)
        .ok_or_else(|| "stationary hunter acquisition bounds overflow z".to_string())?
        .max(
            spawn
                .z
                .checked_sub(attack_range)
                .ok_or_else(|| "stationary hunter attack bounds overflow z".to_string())?,
        );
    let max_x = spawn
        .x
        .checked_add(hunt_range)
        .ok_or_else(|| "stationary hunter acquisition bounds overflow x".to_string())?
        .min(
            footprint_max_x
                .checked_add(attack_range)
                .ok_or_else(|| "stationary hunter attack bounds overflow x".to_string())?,
        );
    let max_z = spawn
        .z
        .checked_add(hunt_range)
        .ok_or_else(|| "stationary hunter acquisition bounds overflow z".to_string())?
        .min(
            footprint_max_z
                .checked_add(attack_range)
                .ok_or_else(|| "stationary hunter attack bounds overflow z".to_string())?,
        );
    if min_x > max_x || min_z > max_z {
        return Err("stationary hunter range intersection is empty".into());
    }
    Ok((min_x, min_z, max_x, max_z))
}

/// Keep the range-derived index bounds, subtracting only cells hidden from the
/// NPC's spawn tile. The engine's HuntIterator casts from the player to the
/// NPC's current tile; the caller limits this spawn-centred approximation to
/// `nomove` and zero-wander NPCs.
/// Horizontal runs keep the existing packed carve representation compact.
fn append_ranged_visibility_carves(
    collision: &WorldCollision,
    zone: &Zone,
    index: usize,
    carves: &mut Vec<(u16, crate::router::AvoidRect)>,
) -> Result<(), String> {
    use api::line_of_sight::{has_line_of_sight_local, Footprint};

    if collision.flags.is_none() {
        return Err("ranged visibility carving requires raw baked collision flags".into());
    }
    let index = u16::try_from(index).map_err(|_| "zone carve index exceeds u16")?;
    let level = i32::from(zone.level);
    let npc = Footprint {
        lx: zone.spawn_x,
        lz: zone.spawn_z,
        size: 1,
    };
    let visible = |x, z| {
        has_line_of_sight_local(
            &|x, z| Some(collision.flag(x, z, level) as i32),
            Footprint {
                lx: x,
                lz: z,
                size: 1,
            },
            npc,
        )
    };
    for z in zone.min_z..=zone.max_z {
        let mut x = zone.min_x;
        while x <= zone.max_x {
            if visible(x, z) {
                x += 1;
                continue;
            }
            let min_x = x;
            while x < zone.max_x && !visible(x + 1, z) {
                x += 1;
            }
            carves.push((
                index,
                crate::router::AvoidRect {
                    min_x,
                    max_x: x,
                    min_z: z,
                    max_z: z,
                    level: Some(level),
                },
            ));
            x += 1;
        }
    }
    Ok(())
}

/// Whether an openable (closed) door stands within the inclusive box grown by
/// one tile on `level`.
fn openable_door_within(
    doors: &[crate::map::services::OpenableDoor],
    level: i32,
    min_x: i32,
    min_z: i32,
    max_x: i32,
    max_z: i32,
) -> bool {
    doors.iter().any(|door| {
        door.level == level
            && door.x >= min_x - 1
            && door.x <= max_x + 1
            && door.z >= min_z - 1
            && door.z <= max_z + 1
    })
}

/// Engine `routefinder/flags.ts` movement masks. The baked raw flags share the
/// engine's wall (`0xff`), loc (`0x100`), projectile-blocker (`0x3fe00`),
/// ground-decor (`0x40000`) and ground (`0x200000`) bits; dynamic NPC/player
/// occupancy is never baked, so it is treated as open.
mod npc_block {
    pub const SOUTH: u32 = 0x24_0102;
    pub const NORTH: u32 = 0x24_0120;
    pub const WEST: u32 = 0x24_0108;
    pub const EAST: u32 = 0x24_0180;
    pub const SOUTH_WEST: u32 = 0x24_010e;
    pub const SOUTH_EAST: u32 = 0x24_0183;
    pub const NORTH_WEST: u32 = 0x24_0138;
    pub const NORTH_EAST: u32 = 0x24_01e0;
    pub const NORTH_AND_SOUTH_EAST: u32 = 0x24_013e;
    pub const NORTH_AND_SOUTH_WEST: u32 = 0x24_01e3;
    pub const NORTH_EAST_AND_WEST: u32 = 0x24_018f;
    pub const SOUTH_EAST_AND_WEST: u32 = 0x24_01f8;
}

/// Whether the engine moves a `size`×`size` NPC whose south-west tile is
/// `(x, z)` one step by `(dx, dz)`: `PathingEntity.takeStep` takes a diagonal
/// only for width-1 NPCs and otherwise tries the E/W then the N/S component,
/// each through `StepValidator.canTravel` under the NPC's own collision
/// `strategy` with no dynamic extra flag.
fn npc_step_ok(
    flag: &impl Fn(i32, i32) -> u32,
    x: i32,
    z: i32,
    (dx, dz): (i32, i32),
    size: i32,
    strategy: StepStrategy,
) -> bool {
    use npc_block::*;
    let open = |x: i32, z: i32, mask: u32| strategy.can_move(flag(x, z), mask);
    let s = size;
    match (dx, dz) {
        (0, -1) if s == 1 => open(x, z - 1, SOUTH),
        (0, 1) if s == 1 => open(x, z + 1, NORTH),
        (-1, 0) if s == 1 => open(x - 1, z, WEST),
        (1, 0) if s == 1 => open(x + 1, z, EAST),
        (-1, -1) if s == 1 => {
            open(x - 1, z - 1, SOUTH_WEST) && open(x - 1, z, WEST) && open(x, z - 1, SOUTH)
        }
        (-1, 1) if s == 1 => {
            open(x - 1, z + 1, NORTH_WEST) && open(x - 1, z, WEST) && open(x, z + 1, NORTH)
        }
        (1, -1) if s == 1 => {
            open(x + 1, z - 1, SOUTH_EAST) && open(x + 1, z, EAST) && open(x, z - 1, SOUTH)
        }
        (1, 1) if s == 1 => {
            open(x + 1, z + 1, NORTH_EAST) && open(x + 1, z, EAST) && open(x, z + 1, NORTH)
        }
        // Sizes 2 and up: the engine's size-2 cases equal this general form.
        (0, -1) => {
            open(x, z - 1, SOUTH_WEST)
                && open(x + s - 1, z - 1, SOUTH_EAST)
                && (x + 1..x + s - 1).all(|mid| open(mid, z - 1, NORTH_EAST_AND_WEST))
        }
        (0, 1) => {
            open(x, z + s, NORTH_WEST)
                && open(x + s - 1, z + s, NORTH_EAST)
                && (x + 1..x + s - 1).all(|mid| open(mid, z + s, SOUTH_EAST_AND_WEST))
        }
        (-1, 0) => {
            open(x - 1, z, SOUTH_WEST)
                && open(x - 1, z + s - 1, NORTH_WEST)
                && (z + 1..z + s - 1).all(|mid| open(x - 1, mid, NORTH_AND_SOUTH_EAST))
        }
        (1, 0) => {
            open(x + s, z, SOUTH_EAST)
                && open(x + s, z + s - 1, NORTH_EAST)
                && (z + 1..z + s - 1).all(|mid| open(x + s, mid, NORTH_AND_SOUTH_WEST))
        }
        _ => false,
    }
}

/// Tiles of a moving hunter's zone rectangle from which the engine could
/// acquire a player: the NPC's south-west tile reaches a cell by engine steps
/// (under its own collision strategy) from its spawn without leaving
/// `move_radius`, and the player stands within `huntrange` of it (plus, for
/// `lineofsight` hunt modes, the player-to-NPC ray the engine's `HuntIterator`
/// casts). Row-major over the zone bounds.
fn mobile_acquisition(collision: &WorldCollision, zone: &Zone, reach: MobileReach) -> Vec<bool> {
    use api::line_of_sight::{has_line_of_sight_local, Footprint};

    let level = i32::from(zone.level);
    let flag = |x: i32, z: i32| collision.flag(x, z, level);
    let radius = reach.move_radius.max(0);
    let side = (2 * radius + 1) as usize;
    let (spawn_x, spawn_z) = (zone.spawn_x, zone.spawn_z);
    let cell = |x: i32, z: i32| -> Option<usize> {
        let (dx, dz) = (x - spawn_x + radius, z - spawn_z + radius);
        (dx >= 0 && dz >= 0 && dx < side as i32 && dz < side as i32)
            .then(|| dz as usize * side + dx as usize)
    };
    let mut occupied = vec![false; side * side];
    let mut queue = std::collections::VecDeque::from([(spawn_x, spawn_z)]);
    occupied[cell(spawn_x, spawn_z).expect("spawn is the flood centre")] = true;
    while let Some((x, z)) = queue.pop_front() {
        for (dx, dz) in [
            (0, -1),
            (0, 1),
            (-1, 0),
            (1, 0),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let Some(next) = cell(x + dx, z + dz) else {
                continue;
            };
            if occupied[next] || !npc_step_ok(&flag, x, z, (dx, dz), reach.size, reach.strategy) {
                continue;
            }
            occupied[next] = true;
            queue.push_back((x + dx, z + dz));
        }
    }

    let width = (zone.max_x - zone.min_x + 1) as usize;
    let height = (zone.max_z - zone.min_z + 1) as usize;
    let hunt = reach.hunt_range;
    let mut member = vec![false; width * height];
    for z in zone.min_z..=zone.max_z {
        for x in zone.min_x..=zone.max_x {
            let acquired = (z - hunt..=z + hunt).any(|npc_z| {
                (x - hunt..=x + hunt).any(|npc_x| {
                    cell(npc_x, npc_z).is_some_and(|index| occupied[index])
                        && (!reach.line_of_sight
                            || has_line_of_sight_local(
                                &|x, z| Some(flag(x, z) as i32),
                                Footprint {
                                    lx: x,
                                    lz: z,
                                    size: 1,
                                },
                                Footprint {
                                    lx: npc_x,
                                    lz: npc_z,
                                    size: 1,
                                },
                            ))
                })
            });
            member[(z - zone.min_z) as usize * width + (x - zone.min_x) as usize] = acquired;
        }
    }
    member
}

/// Subtract every cell of a moving hunter's rectangle that
/// [`mobile_acquisition`] rules out. Identical horizontal runs on consecutive
/// rows merge into one rectangle to keep the packed carve list compact.
fn append_mobile_reach_carves(
    collision: &WorldCollision,
    zone: &Zone,
    reach: MobileReach,
    index: usize,
    carves: &mut Vec<(u16, crate::router::AvoidRect)>,
) -> Result<(), String> {
    if collision.flags.is_none() {
        return Err("mobile hunter reach carving requires raw baked collision flags".into());
    }
    let index = u16::try_from(index).map_err(|_| "zone carve index exceeds u16")?;
    let level = i32::from(zone.level);
    let member = mobile_acquisition(collision, zone, reach);
    let width = (zone.max_x - zone.min_x + 1) as usize;
    // (min_x, max_x, carve position) of the runs that end on the previous row.
    let mut open_runs: Vec<(i32, i32, usize)> = Vec::new();
    for z in zone.min_z..=zone.max_z {
        let row = &member[(z - zone.min_z) as usize * width..][..width];
        let mut next_runs = Vec::new();
        let mut x = zone.min_x;
        while x <= zone.max_x {
            if row[(x - zone.min_x) as usize] {
                x += 1;
                continue;
            }
            let run_min = x;
            while x < zone.max_x && !row[(x + 1 - zone.min_x) as usize] {
                x += 1;
            }
            let position = match open_runs
                .iter()
                .find(|(min_x, max_x, _)| *min_x == run_min && *max_x == x)
            {
                Some(&(_, _, position)) => {
                    carves[position].1.max_z = z;
                    position
                }
                None => {
                    carves.push((
                        index,
                        crate::router::AvoidRect {
                            min_x: run_min,
                            max_x: x,
                            min_z: z,
                            max_z: z,
                            level: Some(level),
                        },
                    ));
                    carves.len() - 1
                }
            };
            next_runs.push((run_min, x, position));
            x += 1;
        }
        open_runs = next_runs;
    }
    Ok(())
}

fn stationary_melee_shape(
    collision: &WorldCollision,
    spawn: WorldTile,
    size: u8,
    openable_door_faces: &HashSet<(i32, i32, i32, u8)>,
) -> Result<u64, String> {
    let side = u32::from(size) + 2;
    if side * side > u64::BITS {
        return Err("stationary melee hunter footprint exceeds the 8x8 shape limit".into());
    }
    let mut mask = 0u64;
    for dz in 0..u32::from(size) {
        for dx in 0..u32::from(size) {
            set_shape_cell(&mut mask, dx + 1, dz + 1, side)?;
        }
    }
    let size_i32 = i32::from(size);
    for offset in 0..size_i32 {
        let west_z = spawn
            .z
            .checked_add(offset)
            .ok_or_else(|| "stationary melee footprint overflows z".to_string())?;
        if face_is_open(
            collision,
            openable_door_faces,
            spawn.x,
            west_z,
            spawn.level,
            CollisionFlag::W_W as u8,
        ) {
            set_shape_cell(&mut mask, 0, u32::try_from(offset + 1).unwrap(), side)?;
        }
        let east_x = spawn
            .x
            .checked_add(size_i32 - 1)
            .ok_or_else(|| "stationary melee footprint overflows x".to_string())?;
        if face_is_open(
            collision,
            openable_door_faces,
            east_x,
            west_z,
            spawn.level,
            CollisionFlag::W_E as u8,
        ) {
            set_shape_cell(
                &mut mask,
                u32::from(size) + 1,
                u32::try_from(offset + 1).unwrap(),
                side,
            )?;
        }
        let south_x = spawn
            .x
            .checked_add(offset)
            .ok_or_else(|| "stationary melee footprint overflows x".to_string())?;
        if face_is_open(
            collision,
            openable_door_faces,
            south_x,
            spawn.z,
            spawn.level,
            CollisionFlag::W_S as u8,
        ) {
            set_shape_cell(&mut mask, u32::try_from(offset + 1).unwrap(), 0, side)?;
        }
        let north_z = spawn
            .z
            .checked_add(size_i32 - 1)
            .ok_or_else(|| "stationary melee footprint overflows z".to_string())?;
        if face_is_open(
            collision,
            openable_door_faces,
            south_x,
            north_z,
            spawn.level,
            CollisionFlag::W_N as u8,
        ) {
            set_shape_cell(
                &mut mask,
                u32::try_from(offset + 1).unwrap(),
                u32::from(size) + 1,
                side,
            )?;
        }
    }
    Ok(mask)
}

fn set_shape_cell(mask: &mut u64, x: u32, z: u32, width: u32) -> Result<(), String> {
    let bit = z
        .checked_mul(width)
        .and_then(|row| row.checked_add(x))
        .filter(|bit| *bit < u64::BITS)
        .ok_or_else(|| "stationary melee mask bit is out of range".to_string())?;
    *mask |= 1u64 << bit;
    Ok(())
}

fn face_is_open(
    collision: &WorldCollision,
    openable_door_faces: &HashSet<(i32, i32, i32, u8)>,
    x: i32,
    z: i32,
    level: i32,
    wall_flag: u8,
) -> bool {
    let is_door = openable_door_faces.contains(&(x, z, level, wall_flag));
    is_door || collision_walk_byte(collision, x, z, level) & wall_flag == 0
}

fn collision_walk_byte(collision: &WorldCollision, x: i32, z: i32, level: i32) -> u8 {
    let Ok(plane) = usize::try_from(level) else {
        return 0;
    };
    if plane >= 4 {
        return 0;
    }
    let Some(local_x) = x.checked_sub(collision.origin.x) else {
        return 0;
    };
    let Some(local_z) = z.checked_sub(collision.origin.z) else {
        return 0;
    };
    let (Ok(local_x), Ok(local_z)) = (usize::try_from(local_x), usize::try_from(local_z)) else {
        return 0;
    };
    if local_x >= collision.width || local_z >= collision.height {
        return 0;
    }
    let Some(index) = plane
        .checked_mul(collision.width.saturating_mul(collision.height))
        .and_then(|base| {
            local_z
                .checked_mul(collision.width)
                .and_then(|row| base.checked_add(row))
        })
        .and_then(|row| row.checked_add(local_x))
    else {
        return 0;
    };
    collision.walk.get(index).copied().unwrap_or(0)
}

fn openable_door_faces(
    doors: &[crate::map::services::OpenableDoor],
) -> Result<HashSet<(i32, i32, i32, u8)>, String> {
    let mut faces = HashSet::with_capacity(doors.len().saturating_mul(2));
    for door in doors {
        if i32::from(door.shape) != LocShape::WALL_STRAIGHT {
            continue;
        }
        let (tile_flag, opposite_flag, dx, dz) = match i32::from(door.rotation) {
            angle if angle == LocAngle::WEST => {
                (CollisionFlag::W_W as u8, CollisionFlag::W_E as u8, -1, 0)
            }
            angle if angle == LocAngle::NORTH => {
                (CollisionFlag::W_N as u8, CollisionFlag::W_S as u8, 0, 1)
            }
            angle if angle == LocAngle::EAST => {
                (CollisionFlag::W_E as u8, CollisionFlag::W_W as u8, 1, 0)
            }
            angle if angle == LocAngle::SOUTH => {
                (CollisionFlag::W_S as u8, CollisionFlag::W_N as u8, 0, -1)
            }
            _ => continue,
        };
        faces.insert((door.x, door.z, door.level, tile_flag));
        let Some(neighbor_x) = door.x.checked_add(dx) else {
            return Err("openable door face overflows x".into());
        };
        let Some(neighbor_z) = door.z.checked_add(dz) else {
            return Err("openable door face overflows z".into());
        };
        faces.insert((neighbor_x, neighbor_z, door.level, opposite_flag));
    }
    Ok(faces)
}

/// Bake the whole world for one request. Every `.jm2` under the maps dir
/// bakes or the call fails; non-`.jm2` files are metadata and skipped.
pub fn bake_world(request: &BakeRequest<'_>) -> Result<BakedNav, String> {
    let content_root = request.maps_dir.parent().unwrap_or(Path::new("."));
    let owned_fingerprints = request
        .input_fingerprints
        .is_none()
        .then(|| crate::bundle::fingerprints(content_root, &[request.config_jag]))
        .transpose()?;
    let source_fingerprints = request
        .input_fingerprints
        .or(owned_fingerprints.as_deref())
        .expect("one bake input snapshot is present");
    let source_before = crate::bundle::source_digest_from_fingerprints(
        content_root,
        &[request.config_jag],
        source_fingerprints,
    )?;
    let mut notes = Vec::new();

    // Openable wall door loc ids from the Server door configs.
    let mut door_ids = HashSet::new();
    // Closed door id -> the id it becomes when opened, from the same configs.
    let mut door_open_ids = HashMap::new();
    let mut parsed_door_configs = Vec::with_capacity(DOOR_CONFIGS.len() + 1);
    let mut config_failed = 0usize;
    for name in DOOR_CONFIGS {
        let path = request.doors_dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                door_ids.extend(crate::pack::parse_door_config(&text));
                door_open_ids.extend(crate::pack::parse_door_open_ids(&text, &HashMap::new()));
                parsed_door_configs.push(path);
            }
            Err(e) => {
                if request.require_all_door_configs {
                    return Err(format!("door config {}: {e}", path.display()));
                }
                notes.push(format!("skipping {name}: {e}"));
                config_failed += 1;
            }
        }
    }
    // Fence gates (`scripts/general_use/configs/gates.loc`) join the door
    // set so their tiles do not stamp blocked in the bake. The configured ids
    // and source paths are passed to `door_edges` to avoid deriving them twice.
    match std::fs::read_to_string(request.gates) {
        Ok(text) => {
            door_ids.extend(crate::pack::parse_door_config(&text));
            door_open_ids.extend(crate::pack::parse_door_open_ids(&text, &HashMap::new()));
            parsed_door_configs.push(request.gates.to_path_buf());
        }
        Err(e) => {
            if request.require_all_door_configs {
                return Err(format!("gates config {}: {e}", request.gates.display()));
            }
            notes.push(format!("skipping gates.loc: {e}"));
            config_failed += 1;
        }
    }
    if config_failed == DOOR_CONFIGS.len() + 1 {
        return Err(format!(
            "no door configs parsed (need {} in {} plus {})",
            DOOR_CONFIGS.join(", "),
            request.doors_dir.display(),
            request.gates.display()
        ));
    }
    // Open states of those doors: a player can close one placed open.
    let opened_door_ids: HashSet<i32> = door_open_ids
        .iter()
        .filter(|(closed, _)| door_ids.contains(*closed))
        .map(|(_, open)| *open)
        .collect();

    // Loc/NPC definitions from the client cache: collision uses loc defs;
    // navpois validates NPC/loc ids and operations against the same tables.
    let (loc_defs, npc_types, loc_types) = match std::fs::read(request.config_jag) {
        Ok(bytes) => {
            let cache = Cache::unpack(&JagFile::new(bytes));
            let loc_defs = LocDefs::from_locs(&cache.locs);
            (loc_defs, cache.npcs, cache.locs)
        }
        Err(e) => {
            return Err(format!(
                "cannot load loc defs from {}: {e}",
                request.config_jag.display()
            ))
        }
    };

    // Whole-world collision bake (the walkability source of truth).
    let mut collision = bake_from_maps(request.maps_dir, &loc_defs, &door_ids)
        .map_err(|e| format!("collision bake failed: {e}"))?;
    let walkable = walkable_tiles(&collision);

    // The transport graph from the Server content tree (maps/scripts/pack
    // all live under the maps dir's parent); door edge from/to snap to the
    // nearest walkable tile on the collision just baked.
    let content_root = request.maps_dir.parent().unwrap_or(Path::new("."));
    let (mut graph, audit) = derive_transports_for_bake(
        content_root,
        &loc_defs,
        &collision,
        &door_ids,
        &parsed_door_configs,
    );
    assert_transmitted_varp_reqs(content_root, &graph);
    require_wilderness_teleport_legality(content_root, &graph)?;
    require_members_guards(content_root, &graph, &audit)?;
    let derived_zones = derive_zone_table(
        content_root,
        &collision,
        &graph,
        &npc_types,
        &door_ids,
        &opened_door_ids,
    )?;
    let zone_count = u32::try_from(derived_zones.table.zones().len())
        .map_err(|_| "zone count exceeds the manifest range".to_string())?;
    let zone_npc_count = u32::try_from(derived_zones.npc_count)
        .map_err(|_| "NPC zone count exceeds the manifest range".to_string())?;
    let group_count = derived_zones.table.groups().len();
    let hazard_count = crate::zones::curated::HAZARDS.len();
    let npc_kind_count = derived_zones.npc_kind_count;
    let npc_count = derived_zones.npc_count;
    let always_count = derived_zones.always_count;
    let level_rule_count = derived_zones.level_rule_count;
    let shaped_count = derived_zones.shaped_count;
    let total_npc_spawns = derived_zones.total_npc_spawns;
    let carve_count = derived_zones.table.carves().len();
    graph.zones = Some(derived_zones.table);
    notes.push(format!(
        "zones: {zone_count} ({npc_count} NPC, {hazard_count} hazard), \
         {npc_kind_count} NPC kinds, {group_count} groups, {shaped_count} stationary melee shapes; \
         {total_npc_spawns} map NPC spawns counted; {always_count} Always / {level_rule_count} LevelRule; \
         {carve_count} carves"
    ));
    if audit.converted != 0 {
        notes.push(format!(
            "converted {} non-transmitted varp requirements to completed journal gates",
            audit.converted
        ));
    }
    if !audit.omitted.is_empty() {
        let mut omitted: Vec<_> = audit.omitted.into_iter().collect();
        omitted.sort_unstable_by_key(|(id, _)| *id);
        notes.push(format!("omitted non-transmitted varp-gated edges without a unique completed journal proof: {omitted:?}"));
    }

    // The bank stand table from the same content tree (every `bankbooth`
    // placement with Use-quickly, plus `category=bank_teller` NPC stands).
    let banks = derive_banks(content_root);

    let bank_zones_path = content_root.join(BANK_ZONES_REL);
    let bank_zones_bytes = std::fs::read(&bank_zones_path)
        .map_err(|e| format!("canlight bank_zones {}: {e}", bank_zones_path.display()))?;
    let bank_zones_text = std::str::from_utf8(&bank_zones_bytes)
        .map_err(|e| format!("canlight bank_zones {}: {e}", bank_zones_path.display()))?;
    let zones = canlight::parse_bank_zones(bank_zones_text)?;
    let flags_ref = collision
        .flags
        .as_ref()
        .expect("bake_from_maps always stamps raw flags");
    let canlight_bits =
        canlight::bake_canlight(&collision, flags_ref, request.maps_dir, &loc_defs, &zones)?;

    // The raw baked flags ride in the sidecar; the v16 pack carries only
    // the packed walk surface (the router's resident form).
    let flags = collision
        .flags
        .take()
        .expect("bake_from_maps always stamps raw flags");
    let flags_bytes =
        encode_flags_sidecar(collision.origin, collision.width, collision.height, &flags);
    let bytes = encode(&collision, &graph, &banks)
        .map_err(|error| format!("pack encode failed: {error}"))?;
    let reach_bits = bake_reach(&collision, &graph);
    let pack_digest: [u8; 32] = Sha256::digest(&bytes).into();
    let reach_bytes = encode_reach_sidecar(
        collision.origin,
        collision.width,
        collision.height,
        &reach_bits,
        &pack_digest,
    );
    let revision_for_policy = request.revision.unwrap_or(274);
    let canlight_identity_digest =
        canlight::identity_digest(revision_for_policy, &bank_zones_bytes, &canlight_bits);
    let canlight_identity = sha256_hex(&canlight_identity_digest);
    let canlight_binding = canlight::header_binding(&pack_digest, &canlight_identity_digest);
    let canlight_bytes = encode_canlight_sidecar(
        collision.origin,
        collision.width,
        collision.height,
        &canlight_bits,
        &canlight_binding,
    );
    if request.input_fingerprints.is_none() {
        let source_after_inputs = crate::bundle::fingerprints(content_root, &[request.config_jag])?;
        let source_after = crate::bundle::source_digest_from_fingerprints(
            content_root,
            &[request.config_jag],
            &source_after_inputs,
        )?;
        if source_before != source_after {
            return Err("baker inputs changed during preparation".into());
        }
    }
    let pois = match (request.revision, request.cache, request.content_id) {
        (Some(revision), Some(_), Some(content_id)) => {
            let content = crate::map::identity::Digest::from_hex(content_id)
                .map_err(|_| "decoded content identity is not SHA-256 hex".to_string())?;
            let source = crate::map::identity::Digest::from_hex(&source_before)
                .map_err(|_| "source digest is not SHA-256 hex".to_string())?;
            let generator = crate::map::identity::Digest::from_hex(
                &crate::map::services::pois_generator_identity_from_crate()?,
            )
            .map_err(|_| "navpois generator identity is not SHA-256 hex".to_string())?;
            Some(
                crate::map::services::produce_navpois(&crate::map::services::ProduceRequest {
                    revision,
                    content_root,
                    npcs: &npc_types,
                    locs: &loc_types,
                    content_id: content,
                    nav_sha256: crate::map::identity::Digest(pack_digest),
                    source_sha256: source,
                    generator_sha256: generator,
                })?
                .bytes,
            )
        }
        _ => None,
    };
    let mut manifest = match (request.revision, request.cache) {
        (Some(revision), Some(cache)) => Some(NavManifest::capture(
            revision,
            cache,
            &bytes,
            Some(&flags_bytes),
            Some(&reach_bytes),
            Some(&canlight_bytes),
            pois.as_deref(),
            zone_count,
            zone_npc_count,
        )?),
        (None, None) => None,
        _ => return Err("a bound bake needs both a revision and its cache manifest".into()),
    };
    if let Some(manifest) = &mut manifest {
        manifest.source_sha256 = Some(source_before);
    };
    Ok(BakedNav {
        pack: bytes,
        flags: flags_bytes,
        reach: reach_bytes,
        canlight: canlight_bytes,
        pois,
        canlight_identity,
        manifest,
        summary: BakeSummary {
            mapsquares: squares_baked(request.maps_dir),
            width: collision.width,
            height: collision.height,
            walkable,
            edges: graph.edges.len(),
            banks: banks.len(),
        },
        notes,
    })
}

/// Count `.jm2` files under `maps_dir` (for the summary line).
pub fn squares_baked(maps_dir: &Path) -> usize {
    std::fs::read_dir(maps_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jm2"))
                .count()
        })
        .unwrap_or(0)
}

/// Count tiles with no walk-blocking flag on the bake's level-0 plane.
fn walkable_tiles(c: &WorldCollision) -> usize {
    (0..c.height)
        .flat_map(|z| (0..c.width).map(move |x| (c.origin.x + x as i32, c.origin.z + z as i32)))
        .filter(|(x, z)| {
            c.walkable(WorldTile {
                x: *x,
                z: *z,
                level: 0,
            })
        })
        .count()
}

#[cfg(test)]
#[path = "bake_tests.rs"]
mod tests;

//! Shared world bake: door ids, loc defs, the whole-world collision, the
//! transport graph, the bank stand table, the v8 pack bytes, the raw flags
//! sidecar, the paint-reach sidecar and the bound manifest. Both frontends of this logic call
//! [`bake_world`] — the `nav-pack` developer CLI and the application build
//! (`host-play`'s build script) — so the derivation exists once.
//!
//! Revision-bound bakes verify the selected cache before baking (see
//! [`verify_cache_manifest`]); the cache is a bake input, never a shipped
//! resource.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::Cache;
use client::io::JagFile;
use sha2::{Digest, Sha256};

use crate::collision::{bake_from_maps, WorldCollision};
use crate::manifest::{CacheManifest, NavManifest};
use crate::pack::{derive_banks, encode, encode_flags_sidecar, encode_reach_sidecar, FORMAT_ID};
use crate::paint::bake_reach;
use crate::transport::derive_transports;

/// Door loc configs under `content/scripts/doors/configs`.
pub const DOOR_CONFIGS: [&str; 3] = ["doors.loc", "doubledoors.loc", "opened_doors.loc"];

/// Manual generator identity. Bump when the bake's semantics change in a way
/// [`FORMAT_ID`] does not already capture (a pack format bump changes the
/// format identity instead).
pub const GENERATOR_ID: &str = "nav-bake-1";

/// Baker sources whose bytes join the generator identity: a generated
/// artifact is stale after any change to one of them. Paths are relative to
/// the `nav` crate root; keep this list to the code that decides artifact
/// bytes. Pack/flags come from bake/collision/pack/transport; reach bits also
/// depend on `paint.rs` (`bake_reach`) and `router.rs` (`step_ok`). Traveller
/// and grid-search changes do not decide those bytes.
pub const GENERATOR_SOURCES: [&str; 6] = [
    "src/bake.rs",
    "src/collision.rs",
    "src/pack.rs",
    "src/paint.rs",
    "src/router.rs",
    "src/transport.rs",
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
    /// Application builds require every door config and `gates.loc`: a
    /// missing one would bake a world that silently disagrees with the
    /// server. The developer CLI keeps skipping unavailable configs.
    pub require_all_door_configs: bool,
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
    pub manifest: Option<NavManifest>,
    pub summary: BakeSummary,
    /// Non-fatal notes the caller reports (skipped door configs).
    pub notes: Vec<String>,
}

/// Bake the whole world for one request. Every `.jm2` under the maps dir
/// bakes or the call fails; non-`.jm2` files are metadata and skipped.
pub fn bake_world(request: &BakeRequest<'_>) -> Result<BakedNav, String> {
    let mut notes = Vec::new();

    // Openable wall door loc ids from the Server door configs.
    let mut door_ids = HashSet::new();
    let mut config_failed = 0usize;
    for name in DOOR_CONFIGS {
        let path = request.doors_dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => door_ids.extend(crate::pack::parse_door_config(&text)),
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
    // set so their tiles do not stamp blocked in the bake; the transport
    // graph derives the same set itself in `door_edges`.
    match std::fs::read_to_string(request.gates) {
        Ok(text) => door_ids.extend(crate::pack::parse_door_config(&text)),
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

    // Loc definitions (blockwalk, width/length, active) from the client
    // cache: the same table the game client builds its collision from.
    let loc_defs = match std::fs::read(request.config_jag) {
        Ok(bytes) => {
            let cache = Cache::unpack(&JagFile::new(bytes));
            LocDefs::from_locs(&cache.locs)
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
    let graph = derive_transports(content_root, &loc_defs, &collision);

    // The bank stand table from the same content tree (every `bankbooth`
    // placement, Use-quickly op).
    let banks = derive_banks(content_root);

    // The raw baked flags ride in the sidecar; the v8 pack carries only
    // the packed walk surface (the router's resident form).
    let flags = collision
        .flags
        .take()
        .expect("bake_from_maps always stamps raw flags");
    let flags_bytes =
        encode_flags_sidecar(collision.origin, collision.width, collision.height, &flags);
    let bytes = encode(&collision, &graph, &banks);
    let reach_bits = bake_reach(&collision, &graph);
    let pack_digest: [u8; 32] = Sha256::digest(&bytes).into();
    let reach_bytes = encode_reach_sidecar(
        collision.origin,
        collision.width,
        collision.height,
        &reach_bits,
        &pack_digest,
    );
    let manifest = match (request.revision, request.cache) {
        (Some(revision), Some(cache)) => Some(NavManifest::capture(
            revision,
            cache,
            &bytes,
            Some(&flags_bytes),
            Some(&reach_bytes),
        )?),
        (None, None) => None,
        _ => return Err("a bound bake needs both a revision and its cache manifest".into()),
    };
    Ok(BakedNav {
        pack: bytes,
        flags: flags_bytes,
        reach: reach_bytes,
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
mod tests {
    use super::*;

    #[test]
    fn content_inputs_follow_the_content_root() {
        let inputs = content_inputs(Path::new("/content"));
        assert_eq!(inputs.maps_dir, PathBuf::from("/content/maps"));
        assert_eq!(
            inputs.doors_dir,
            PathBuf::from("/content/scripts/doors/configs")
        );
        assert_eq!(
            inputs.gates,
            PathBuf::from("/content/scripts/general_use/configs/gates.loc")
        );
    }

    #[test]
    fn the_289_config_jag_is_inside_the_cache_and_274_beside_it() {
        assert_eq!(
            config_jag_for(289, Path::new("/289/engine/data/pack/client")).unwrap(),
            PathBuf::from("/289/engine/data/pack/client/config")
        );
        assert_eq!(
            config_jag_for(274, Path::new("/274/engine/data/pack/client")).unwrap(),
            PathBuf::from("/274/engine/data/pack/config")
        );
    }

    #[test]
    fn generator_identity_tracks_the_manual_id_format_and_sources() {
        let base = generator_identity(&[("src/pack.rs", "fn a() {}")]);
        assert_eq!(base.len(), 64);
        assert!(base.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(base, generator_identity(&[("src/pack.rs", "fn b() {}")]));
        assert_ne!(
            base,
            generator_identity(&[("src/collision.rs", "fn a() {}")])
        );
        // Order and labels are part of the identity.
        assert_ne!(
            generator_identity(&[("a.rs", "x"), ("b.rs", "y")]),
            generator_identity(&[("b.rs", "x"), ("a.rs", "y")])
        );
    }

    #[test]
    fn router_source_bytes_invalidate_a_warm_reach_stamp() {
        // bake_reach floods with router::step_ok; a movement change must not
        // keep a warm-stamped 274R sidecar while pack/flags stamps still match.
        assert!(
            GENERATOR_SOURCES.contains(&"src/router.rs"),
            "GENERATOR_SOURCES must include the step_ok-owning source"
        );
        assert!(GENERATOR_SOURCES.contains(&"src/paint.rs"));

        let baseline: Vec<(&str, &str)> = GENERATOR_SOURCES
            .iter()
            .map(|path| (*path, "fn a() {}"))
            .collect();
        let mut router_only = baseline.clone();
        for (label, text) in &mut router_only {
            if *label == "src/router.rs" {
                *text = "fn step_ok_changed() {}";
            }
        }
        assert_eq!(
            router_only
                .iter()
                .find(|(path, _)| *path == "src/paint.rs")
                .map(|(_, text)| *text),
            Some("fn a() {}"),
            "paint.rs bytes stay the same; only router.rs changes"
        );

        let warm = generator_identity(&baseline);
        let after_router = generator_identity(&router_only);
        assert_ne!(
            warm, after_router,
            "router.rs bytes join generator_identity"
        );

        let inputs = [crate::bundle::InputFingerprint {
            path: "/content/maps/m1.jm2".into(),
            bytes: 10,
            modified_nanos: 5,
        }];
        let baked = crate::bundle::BakeStamp {
            generator: warm,
            format: "274V8".into(),
            revision: 289,
            cache_id: "cache-1".into(),
            cache_manifest: None,
            nav_sha256: "ab".repeat(32),
            flags_sha256: "cd".repeat(32),
            reach_sha256: "ef".repeat(32),
            pack_bytes: 11,
            flags_bytes: 7,
            reach_bytes: 9,
            relative_pack: "nav/289/274bot.navpack".into(),
            relative_flags: "nav/289/274bot.navflags".into(),
            relative_reach: "nav/289/274bot.navreach".into(),
            inputs: inputs.to_vec(),
        };
        let expected = crate::bundle::StampExpectation {
            revision: 289,
            format: "274V8",
            generator: &after_router,
            cache_id: "cache-1",
            inputs: &inputs,
            staged_pack_bytes: Some(11),
            staged_flags_bytes: Some(7),
            staged_reach_bytes: Some(9),
        };
        let error = baked
            .covers(&expected)
            .expect_err("router source change must fail covers and force reach rebake");
        assert!(error.contains("generator"), "{error}");
    }

    #[test]
    fn a_missing_canonical_input_fails_the_bake() {
        let request = BakeRequest {
            revision: None,
            maps_dir: Path::new("/nonexistent/content/maps"),
            doors_dir: Path::new("/nonexistent/content/scripts/doors/configs"),
            gates: Path::new("/nonexistent/content/scripts/general_use/configs/gates.loc"),
            config_jag: Path::new("/nonexistent/engine/data/pack/config"),
            cache: None,
            require_all_door_configs: true,
        };
        let error = bake_world(&request).err().expect("a bake without inputs");
        assert!(error.contains("doors.loc"), "{error}");
    }
}

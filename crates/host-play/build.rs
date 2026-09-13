//! Ordinary application builds own the navigation artifact.
//!
//! A default build bakes (or reuses) the selected release revision's nav pack
//! and flags with the shared baker ([`nav::bake`]), stages them under the
//! install resource root the running binary resolves — the cargo profile
//! directory for a plain executable, `Contents/Resources` for a macOS bundle,
//! `BOT_NAV_RESOURCE_DIR` when a packager stages elsewhere — and publishes
//! their identities to the compile-time table `nav_identity` reads. No manual
//! `nav-pack` step is needed for a normal WalkTo; the CLI stays for deliberate
//! developer/custom-input bakes.
//!
//! The cache is a bake input, never a shipped resource. Expensive identities
//! (cache identity, pack/flags digests, input fingerprints) are computed here,
//! once per artifact set: a warm build with unchanged canonical inputs reuses
//! the staged artifacts and only re-publishes their stamped identities.
//!
//! Knobs (see docs/api/nav.md):
//! - `BOT_NAV_BUILD=require|skip` (default `require`)
//! - `BOT_NAV_REVISION=289|274` (default `289`)
//! - `BOT_NAV_ENGINE_DIR`, else `ENGINE_DIR`, else the revision's canonical
//!   local engine (`$HOME/experiments/lostcity-289/engine` /
//!   `$HOME/experiments/Server/engine`)
//! - `BOT_NAV_CONTENT_DIR` (default the engine's sibling `content/`)
//! - `BOT_CACHE_MANIFEST` (verified cache manifest; otherwise the captured
//!   cache identity must be one of the checked-in known cache identities)
//! - `BOT_NAV_RESOURCE_DIR` (staging root; default the cargo profile dir)

use std::path::{Path, PathBuf};

use nav::bake::{
    config_jag_for, content_inputs, generator_identity, verify_cache_manifest, BakeRequest,
    GENERATOR_SOURCES,
};
use nav::bundle::{
    artifact_layout, fingerprints, merge_identity_rows, resource_root_for_build, BakeStamp,
    NavIdentityRow, StampExpectation,
};
use nav::manifest::CacheManifest;
use nav::pack::FORMAT_ID;

/// Environment inputs, all re-checked by cargo before this script is skipped.
const ENV_KEYS: [&str; 7] = [
    "BOT_NAV_BUILD",
    "BOT_NAV_REVISION",
    "BOT_NAV_ENGINE_DIR",
    "ENGINE_DIR",
    "BOT_NAV_CONTENT_DIR",
    "BOT_CACHE_MANIFEST",
    "BOT_NAV_RESOURCE_DIR",
];

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    for key in ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }

    // The bake generator identity is part of the artifact stamp: any change to
    // the baker or the pack format invalidates staged artifacts.
    let nav_dir = manifest_dir.join("../nav");
    let mut sources = Vec::new();
    for relative in GENERATOR_SOURCES {
        let path = nav_dir.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => fail(&format!("nav bake source {}: {e}", path.display())),
        };
        sources.push((relative, text));
    }
    let source_refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(label, text)| (*label, text.as_str()))
        .collect();
    let generator = generator_identity(&source_refs);

    // Checked-in rows describe prebuilt resource bundles a packager shipped.
    let checked_in_path = manifest_dir.join("src/bundled-nav-identities.json");
    println!("cargo:rerun-if-changed={}", checked_in_path.display());
    let checked_in = read_rows(&checked_in_path);

    if std::env::var("BOT_NAV_BUILD").as_deref() == Ok("skip") {
        let rows = publish(&out_dir, checked_in, Vec::new());
        println!("cargo:rustc-env=BOT_NAV_BUNDLED=none");
        println!(
            "cargo:warning=nav bundle: BOT_NAV_BUILD=skip; {} checked-in identity row(s) published, no artifact baked or staged",
            rows
        );
        return;
    }

    let revision = selected_revision();
    let engine_dir = engine_dir(revision);
    let content_dir = std::env::var_os("BOT_NAV_CONTENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            engine_dir
                .parent()
                .unwrap_or(Path::new("."))
                .join("content")
        });
    let cache_dir = engine_dir.join("data/pack/client");
    let inputs = content_inputs(&content_dir);
    let config_jag = match config_jag_for(revision, &cache_dir) {
        Ok(path) => path,
        Err(e) => fail(&e),
    };

    // Canonical inputs are required: a build that cannot produce the artifact
    // fails here instead of shipping an apparently ready app without nav.
    require_dir(&inputs.maps_dir, revision);
    require_dir(&inputs.doors_dir, revision);
    require_file(&inputs.gates, revision);
    require_file(&config_jag, revision);
    let archives: Vec<PathBuf> = CacheManifest::ARCHIVES
        .iter()
        .map(|name| cache_dir.join(name))
        .collect();
    for archive in &archives {
        require_file(archive, revision);
    }
    println!("cargo:rerun-if-changed={}", content_dir.display());
    println!("cargo:rerun-if-changed={}", config_jag.display());
    for archive in &archives {
        println!("cargo:rerun-if-changed={}", archive.display());
    }

    // The expensive cache identity: verified once, stamped with the artifacts.
    let supplied_manifest = std::env::var_os("BOT_CACHE_MANIFEST").map(PathBuf::from);
    let (manifest_source, manifest) = match &supplied_manifest {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", path.display());
            let manifest = match verify_cache_manifest(revision, &cache_dir, path, &config_jag) {
                Ok(manifest) => manifest,
                Err(e) => fail(&format!("navigation cache manifest: {e}")),
            };
            (Some(path.clone()), manifest)
        }
        None => {
            let captured = match CacheManifest::capture(revision, &cache_dir) {
                Ok(captured) => captured,
                Err(e) => fail(&format!("navigation cache identity: {e}")),
            };
            let known =
                read_known_identities(&manifest_dir.join("src/known-cache-identities.json"));
            if !known.contains(&captured) {
                fail(&format!(
                    "cache revision is unverified at {}; supply BOT_CACHE_MANIFEST for the \
                     prepared server/cache pairing (the runtime applies the same rule)",
                    cache_dir.display()
                ));
            }
            (None, captured)
        }
    };
    let cache_id = manifest.identity();

    let mut explicit: Vec<PathBuf> = vec![config_jag.clone()];
    explicit.extend(archives.iter().cloned());
    let explicit_refs: Vec<&Path> = explicit.iter().map(PathBuf::as_path).collect();
    let input_fingerprints = match fingerprints(&content_dir, &explicit_refs) {
        Ok(rows) => rows,
        Err(e) => fail(&e),
    };

    let resource_override = std::env::var_os("BOT_NAV_RESOURCE_DIR").map(PathBuf::from);
    let resource_root = match resource_root_for_build(&out_dir, resource_override.as_deref()) {
        Ok(root) => root,
        Err(e) => fail(&e),
    };
    let layout = artifact_layout(revision);
    let pack_path = resource_root.join(&layout.relative_pack);
    let flags_path = resource_root.join(&layout.relative_flags);
    let stamp_path = resource_root.join(&layout.relative_stamp);
    // Staged artifacts are watched so that a later build notices one that was
    // deleted or replaced (cargo treats a missing watched path as changed),
    // while a plain warm build stays a no-op.
    for staged in [&pack_path, &flags_path, &stamp_path] {
        println!("cargo:rerun-if-changed={}", staged.display());
    }

    let expectation = StampExpectation {
        revision,
        format: FORMAT_ID,
        generator: &generator,
        cache_id: &cache_id,
        inputs: &input_fingerprints,
        staged_pack_bytes: file_len(&pack_path),
        staged_flags_bytes: file_len(&flags_path),
    };
    let staged = read_stamp(&stamp_path);
    let (row, reused) = match staged
        .as_ref()
        .and_then(|stamp| stamp.covers(&expectation).ok())
    {
        Some(()) => (
            NavIdentityRow {
                revision,
                cache_id: cache_id.clone(),
                format: FORMAT_ID.into(),
                nav_sha256: staged.as_ref().expect("stamp").nav_sha256.clone(),
                flags_sha256: Some(staged.as_ref().expect("stamp").flags_sha256.clone()),
                relative_path: layout.relative_pack.clone(),
            },
            true,
        ),
        None => (
            match bake_and_stage(
                revision,
                &inputs,
                &config_jag,
                &manifest,
                manifest_source.as_deref(),
                &generator,
                &cache_id,
                &input_fingerprints,
                &resource_root,
                &layout,
                staged
                    .as_ref()
                    .and_then(|stamp| stamp.covers(&expectation).err())
                    .as_deref(),
            ) {
                Ok(row) => row,
                Err(e) => fail(&e),
            },
            false,
        ),
    };

    let rows = publish(&out_dir, checked_in, vec![row]);
    println!("cargo:rustc-env=BOT_NAV_BUNDLED={revision}");
    println!(
        "cargo:warning=nav bundle: {} revision {revision} cache {cache_id}, {rows} identity row(s)",
        if reused { "reused" } else { "baked" }
    );
}

/// Bake the selected revision and stage pack, flags, sidecar manifest and the
/// stamp under the resource root. The stamp is written last: it is the commit
/// point warm builds trust.
#[allow(clippy::too_many_arguments)]
fn bake_and_stage(
    revision: u16,
    inputs: &nav::bake::ContentInputs,
    config_jag: &Path,
    cache: &CacheManifest,
    manifest_source: Option<&Path>,
    generator: &str,
    cache_id: &str,
    input_fingerprints: &[nav::bundle::InputFingerprint],
    resource_root: &Path,
    layout: &nav::bundle::ArtifactLayout,
    stale_reason: Option<&str>,
) -> Result<NavIdentityRow, String> {
    if let Some(reason) = stale_reason {
        println!("cargo:warning=nav bundle: rebaking ({reason})");
    }
    let baked = nav::bake::bake_world(&BakeRequest {
        revision: Some(revision),
        maps_dir: &inputs.maps_dir,
        doors_dir: &inputs.doors_dir,
        gates: &inputs.gates,
        config_jag,
        cache: Some(cache),
        require_all_door_configs: true,
    })?;
    for note in &baked.notes {
        println!("cargo:warning=nav bundle: {note}");
    }
    let manifest = baked
        .manifest
        .ok_or_else(|| "a bound bake carries its manifest".to_string())?;
    let summary = baked.summary;

    let pack_path = resource_root.join(&layout.relative_pack);
    let flags_path = resource_root.join(&layout.relative_flags);
    let dir = pack_path
        .parent()
        .ok_or_else(|| format!("resource path {} has no parent", pack_path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("resource dir {}: {e}", dir.display()))?;
    write_atomic(&pack_path, &baked.pack)?;
    write_atomic(&flags_path, &baked.flags)?;
    let nav_manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| format!("navigation manifest: {e}"))?;
    write_atomic(
        &resource_root.join(&layout.relative_manifest),
        &nav_manifest_bytes,
    )?;

    let flags_sha256 = manifest
        .flags_sha256
        .clone()
        .ok_or_else(|| "a bound bake stamps the flags digest".to_string())?;
    let stamp = BakeStamp {
        generator: generator.to_string(),
        format: FORMAT_ID.to_string(),
        revision,
        cache_id: cache_id.to_string(),
        cache_manifest: manifest_source.map(|path| path.to_string_lossy().into_owned()),
        nav_sha256: manifest.nav_sha256.clone(),
        flags_sha256: flags_sha256.clone(),
        pack_bytes: baked.pack.len() as u64,
        flags_bytes: baked.flags.len() as u64,
        relative_pack: layout.relative_pack.clone(),
        relative_flags: layout.relative_flags.clone(),
        inputs: input_fingerprints.to_vec(),
    };
    let stamp_bytes =
        serde_json::to_vec(&stamp).map_err(|e| format!("navigation build stamp: {e}"))?;
    write_atomic(&resource_root.join(&layout.relative_stamp), &stamp_bytes)?;

    println!(
        "cargo:warning=nav bundle: baked {} mapsquares into a {}x{} grid, {} walkable tiles, {} edges, {} banks; pack {} bytes, flags {} bytes -> {}",
        summary.mapsquares,
        summary.width,
        summary.height,
        summary.walkable,
        summary.edges,
        summary.banks,
        baked.pack.len(),
        baked.flags.len(),
        pack_path.display()
    );
    Ok(NavIdentityRow {
        revision,
        cache_id: cache_id.to_string(),
        format: FORMAT_ID.to_string(),
        nav_sha256: manifest.nav_sha256,
        flags_sha256: Some(flags_sha256),
        relative_path: layout.relative_pack.clone(),
    })
}

/// Publish the compile-time identity table: the generated row wins for its
/// `(revision, cache_id)`; checked-in rows fill in identities this build did
/// not generate.
fn publish(
    out_dir: &Path,
    checked_in: Vec<NavIdentityRow>,
    generated: Vec<NavIdentityRow>,
) -> usize {
    let (rows, notes) = merge_identity_rows(generated, checked_in);
    for note in notes {
        println!("cargo:warning=nav bundle: {note}");
    }
    let count = rows.len();
    let bytes = match serde_json::to_vec_pretty(&rows) {
        Ok(bytes) => bytes,
        Err(e) => fail(&format!("bundled nav identities: {e}")),
    };
    let path = out_dir.join("bundled-nav-identities.json");
    if let Err(e) = std::fs::write(&path, bytes) {
        fail(&format!("write {}: {e}", path.display()));
    }
    count
}

fn selected_revision() -> u16 {
    match std::env::var("BOT_NAV_REVISION") {
        Ok(value) if value == "289" => 289,
        Ok(value) if value == "274" => 274,
        Ok(value) => fail(&format!(
            "BOT_NAV_REVISION must be 274 or 289, got {value:?}"
        )),
        Err(_) => 289,
    }
}

fn engine_dir(revision: u16) -> PathBuf {
    if let Some(dir) = std::env::var_os("BOT_NAV_ENGINE_DIR").map(PathBuf::from) {
        return dir;
    }
    if let Some(dir) = std::env::var_os("ENGINE_DIR").map(PathBuf::from) {
        return dir;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(if revision == 289 {
        "experiments/lostcity-289/engine"
    } else {
        "experiments/Server/engine"
    })
}

fn read_rows(path: &Path) -> Vec<NavIdentityRow> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => fail(&format!("navigation identities {}: {e}", path.display())),
    };
    match serde_json::from_slice::<Vec<NavIdentityRow>>(&bytes) {
        Ok(rows) => rows,
        Err(e) => fail(&format!("navigation identities {}: {e}", path.display())),
    }
}

fn read_known_identities(path: &Path) -> Vec<CacheManifest> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => fail(&format!("known cache identities {}: {e}", path.display())),
    };
    match serde_json::from_slice(&bytes) {
        Ok(rows) => rows,
        Err(e) => fail(&format!("known cache identities {}: {e}", path.display())),
    }
}

fn read_stamp(path: &Path) -> Option<BakeStamp> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|e| format!("write {}: {e}", temp.display()))?;
    if std::fs::rename(&temp, path).is_err() {
        // Windows keeps the destination on rename; replace and retry.
        let _ = std::fs::remove_file(path);
        std::fs::rename(&temp, path).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn require_dir(path: &Path, revision: u16) {
    if !path.is_dir() {
        fail(&format!(
            "{} (revision {revision}) is missing; point BOT_NAV_ENGINE_DIR/BOT_NAV_CONTENT_DIR \
             at the canonical tree, or set BOT_NAV_BUILD=skip to build without bundled \
             navigation",
            path.display()
        ));
    }
}

fn require_file(path: &Path, revision: u16) {
    if !path.is_file() {
        fail(&format!(
            "{} (revision {revision}) is missing; point BOT_NAV_ENGINE_DIR/BOT_NAV_CONTENT_DIR \
             at the canonical tree, or set BOT_NAV_BUILD=skip to build without bundled \
             navigation",
            path.display()
        ));
    }
}

fn fail(message: &str) -> ! {
    panic!("navigation build: {message}");
}

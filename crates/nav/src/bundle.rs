//! Build-time navigation bundling: the identities and machine-local stamps
//! that let an ordinary application build bake the selected revision's nav
//! artifacts once, stage them in the install resource layout, and reuse them
//! on every warm build, plus the required canonical content inventory the
//! build checks before it bakes or reuses anything.
//!
//! Nothing here is runtime state: the runtime keeps its cheap format/revision
//! checks and the explicit external overrides (`--nav-pack` / `NAV_PACK`).
//! Cache and world stay shared across clients.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One shipped navpack identity: the compile-time table row the runtime uses
/// to select the bundled fast path. It is written by the build (generated
/// artifacts) or checked in by a packager (prebuilt resource bundles).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavIdentityRow {
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub source_sha256: Option<String>,
    pub revision: u16,
    pub cache_id: String,
    pub format: String,
    pub nav_sha256: String,
    pub flags_sha256: Option<String>,
    #[serde(default)]
    pub reach_sha256: Option<String>,
    #[serde(default)]
    pub canlight_sha256: Option<String>,
    #[serde(default)]
    pub canlight_identity: Option<String>,
    #[serde(default)]
    pub pois_sha256: Option<String>,
    pub relative_path: String,
}

/// One canonical input file's machine-local identity: size plus modification
/// time. Content hashing stays at bake time; warm builds compare these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFingerprint {
    pub path: String,
    pub bytes: u64,
    pub modified_nanos: u64,
}

impl InputFingerprint {
    pub fn of(path: &Path) -> Result<Self, String> {
        let metadata =
            std::fs::metadata(path).map_err(|e| format!("bake input {}: {e}", path.display()))?;
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| age.as_nanos() as u64)
            .unwrap_or(0);
        Ok(Self {
            path: path.to_string_lossy().into_owned(),
            bytes: metadata.len(),
            modified_nanos,
        })
    }
}

/// Fingerprint every regular file under `root` (recursively) plus each
/// explicit file, sorted by path. A missing `root` or explicit file is an
/// error: canonical inputs are required, not optional.
pub fn fingerprints(root: &Path, files: &[&Path]) -> Result<Vec<InputFingerprint>, String> {
    let mut paths = Vec::new();
    collect_files(root, &mut paths)?;
    for file in files {
        let metadata =
            std::fs::metadata(file).map_err(|e| format!("bake input {}: {e}", file.display()))?;
        if !metadata.is_file() {
            return Err(format!("bake input {} is not a file", file.display()));
        }
        paths.push((*file).to_path_buf());
    }
    let mut rows = paths
        .iter()
        .map(|path| InputFingerprint::of(path))
        .collect::<Result<Vec<_>, String>>()?;
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows.dedup_by(|a, b| a.path == b.path);
    Ok(rows)
}

/// Content-address the conservative baker input closure. Paths are relative
/// to the content root; VCS metadata is not a baker input. Explicit inputs
/// are labelled by argument position so provenance is machine-independent.
pub fn source_digest(root: &Path, files: &[&Path]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut paths = Vec::new();
    collect_files(root, &mut paths)?;
    paths.sort();
    let mut digest = Sha256::new();
    digest.update(b"274NAVSOURCE01");
    for path in paths {
        // `/`-joined on every platform: a Windows label must not differ.
        let label = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        digest.update((label.len() as u64).to_be_bytes());
        digest.update(label.as_bytes());
        digest.update(crate::manifest::hash_file(&path)?.as_bytes());
    }
    for (index, path) in files.iter().enumerate() {
        digest.update((index as u64).to_be_bytes());
        digest.update(crate::manifest::hash_file(path)?.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("bake input directory {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("bake input directory {}: {e}", dir.display()))?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let metadata =
            std::fs::metadata(&path).map_err(|e| format!("bake input {}: {e}", path.display()))?;
        if metadata.is_dir() {
            collect_files(&path, out)?;
        } else if metadata.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Required canonical content inputs.
// ---------------------------------------------------------------------------

/// Kind of one required canonical content input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredKind {
    /// A file a derivation reads by name.
    File,
    /// One mapsquare of the revision's canonical map set (the whole-world
    /// collision bake and the jm2 placement reader consume them).
    Mapsquare,
    /// One script of the baker's recursive `scripts/**/*.rs2` scan (door open
    /// scripts, agility shortcuts, jewellery rubs).
    Rs2,
    /// One config of the baker's recursive `scripts/**/*.constant` scan
    /// (script constants, spirit-tree and lever destinations).
    Constant,
    /// One config of the baker's recursive `scripts/**/*.obj` scan
    /// (slash-attack blades).
    Obj,
    /// One door config of the baker's recursive `*.loc` scan
    /// (`scripts/doors/configs`, `scripts/quests`, `scripts/areas`,
    /// `scripts/general_use/configs`).
    Loc,
}

impl RequiredKind {
    /// The inventory's name for this kind.
    fn name(self) -> &'static str {
        match self {
            RequiredKind::File => "file",
            RequiredKind::Mapsquare => "map",
            RequiredKind::Rs2 => "rs2",
            RequiredKind::Constant => "constant",
            RequiredKind::Obj => "obj",
            RequiredKind::Loc => "loc",
        }
    }

    /// The inventory name → kind, `None` for an unknown name.
    fn from_name(name: &str) -> Option<Self> {
        [
            RequiredKind::File,
            RequiredKind::Mapsquare,
            RequiredKind::Rs2,
            RequiredKind::Constant,
            RequiredKind::Obj,
            RequiredKind::Loc,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }

    /// What the bake reads a scanned input of this kind for; the consumer of
    /// a named [`RequiredKind::File`] comes from its inventory row instead,
    /// and a mapsquare's kind already says what it is.
    fn scan_consumer(self) -> &'static str {
        match self {
            RequiredKind::Rs2 => {
                "recursive scripts/**/*.rs2 scan (door open scripts, agility shortcuts, jewellery rubs)"
            }
            RequiredKind::Constant => {
                "recursive scripts/**/*.constant scan (script constants, lever and spirit-tree destinations)"
            }
            RequiredKind::Obj => "recursive scripts/**/*.obj scan (slash-attack blades)",
            RequiredKind::Loc => {
                "recursive *.loc door config scan (doors/configs, quests, areas, general_use/configs)"
            }
            RequiredKind::File | RequiredKind::Mapsquare => "",
        }
    }
}

/// One canonical content input a default (`BOT_NAV_BUILD=require`) build must
/// have. The baker quietly skips what is absent, so the build script requires
/// them before it bakes or accepts a staged artifact set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredInput {
    /// Content-root-relative, `/`-separated path.
    pub path: &'static str,
    pub kind: RequiredKind,
    /// What the bake consumes the input for (the actionable part of a
    /// missing-input error); empty for mapsquares, which the kind describes.
    pub consumer: &'static str,
}

/// The embedded revision inventory. Only the release revisions the build
/// script accepts have one.
fn required_inventory(revision: u16) -> Option<&'static str> {
    match revision {
        289 => Some(include_str!("required-content-289.tsv")),
        274 => Some(include_str!("required-content-274.tsv")),
        _ => None,
    }
}

/// The required canonical content inputs of `revision`, in inventory order:
/// `#` comments and blank lines are skipped, every other line is
/// `kind<TAB>content-relative path[<TAB>consumer]` with kinds `file` (read by
/// name), `map` (one canonical mapsquare) and the recursive scans' `rs2`,
/// `constant`, `obj` and `loc` (an actual file of the scan, listed one row
/// each, so a missing child is reported while its siblings remain). The
/// inventory is data, verified by the crate's tests against the configured
/// canonical trees; a revision without an inventory reports none, so the
/// tolerated path stays for revisions the build script rejects.
pub fn required_content_inputs(revision: u16) -> Result<Vec<RequiredInput>, String> {
    let Some(text) = required_inventory(revision) else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.split('\t');
        let kind_name = columns.next().unwrap_or_default();
        let Some(kind) = RequiredKind::from_name(kind_name) else {
            return Err(format!(
                "required-content-{revision} line {}: unknown kind {kind_name:?}",
                index + 1
            ));
        };
        let Some(path) = columns.next().filter(|path| !path.is_empty()) else {
            return Err(format!(
                "required-content-{revision} line {}: no path",
                index + 1
            ));
        };
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(format!(
                "required-content-{revision} line {}: {path} is not content-relative",
                index + 1
            ));
        }
        rows.push(RequiredInput {
            path,
            kind,
            consumer: columns.next().unwrap_or(""),
        });
    }
    Ok(rows)
}

/// The required inputs of `revision` missing under `content_dir`, one
/// actionable description each; an empty list means the canonical content is
/// complete. Bytes are never judged here — a deliberate edit or addition
/// stays eligible for the ordinary fingerprint/staleness path.
pub fn missing_content_inputs(revision: u16, content_dir: &Path) -> Result<Vec<String>, String> {
    let mut missing = Vec::new();
    for input in required_content_inputs(revision)? {
        if !content_dir.join(input.path).is_file() {
            missing.push(describe_required(revision, &input));
        }
    }
    Ok(missing)
}

/// One missing required input, described for a build error.
fn describe_required(revision: u16, input: &RequiredInput) -> String {
    match input.kind {
        RequiredKind::File => format!("{} ({})", input.path, input.consumer),
        RequiredKind::Mapsquare => {
            format!(
                "{} (canonical mapsquare of revision {revision})",
                input.path
            )
        }
        kind => format!("{} ({})", input.path, kind.scan_consumer()),
    }
}

/// Install-relative layout of one revision's staged navigation artifacts,
/// under the resource root the running binary resolves (macOS
/// `Contents/Resources`, otherwise the executable's parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLayout {
    /// Install-relative directory holding the staged artifacts.
    pub dir: PathBuf,
    pub relative_pack: String,
    pub relative_flags: String,
    pub relative_reach: String,
    pub relative_canlight: String,
    pub relative_pois: String,
    pub relative_manifest: String,
    pub relative_stamp: String,
}

/// `nav/<revision>/…` keeps revisions side by side: building the other
/// revision never overwrites the staged artifacts of this one. The relative
/// strings are install-relative and written into the stamp, so they use `/`
/// on every platform (Windows file APIs accept it).
pub fn artifact_layout(revision: u16) -> ArtifactLayout {
    let rel = |file: &str| format!("nav/{revision}/{file}");
    ArtifactLayout {
        relative_pack: rel("274bot.navpack"),
        relative_flags: rel("274bot.navflags"),
        relative_reach: rel("274bot.navreach"),
        relative_canlight: rel("274bot.navcanlight"),
        relative_pois: rel("274bot.navpois"),
        relative_manifest: rel("274bot.navpack.json"),
        relative_stamp: rel("nav-build.json"),
        dir: PathBuf::from(format!("nav/{revision}")),
    }
}

/// Machine-local stamp of one staged artifact set. It records the expensive
/// identities computed at bake time (cache identity, pack/flags digests) and
/// the input fingerprints that decide staleness on later builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BakeStamp {
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub source_sha256: Option<String>,
    /// Generator identity of the bake, from [`crate::bake::generator_identity`].
    pub generator: String,
    /// Pack format identity (`274V10`); a format bump invalidates.
    pub format: String,
    pub revision: u16,
    pub cache_id: String,
    /// Manifest the verified cache identity came from, when one was supplied.
    pub cache_manifest: Option<String>,
    pub nav_sha256: String,
    pub flags_sha256: String,
    pub reach_sha256: String,
    pub canlight_sha256: String,
    pub canlight_identity: String,
    pub pack_bytes: u64,
    pub flags_bytes: u64,
    pub reach_bytes: u64,
    pub canlight_bytes: u64,
    pub relative_pack: String,
    pub relative_flags: String,
    pub relative_reach: String,
    pub relative_canlight: String,
    #[serde(default)]
    pub pois_sha256: Option<String>,
    #[serde(default)]
    pub pois_bytes: Option<u64>,
    #[serde(default)]
    pub relative_pois: Option<String>,
    #[serde(default)]
    pub pois_generator: Option<String>,
    pub inputs: Vec<InputFingerprint>,
}

/// What a stamp must describe to count as current, plus what the staged files
/// currently look like on disk (`None` when a file is missing).
pub struct StampExpectation<'a> {
    pub revision: u16,
    pub format: &'a str,
    pub generator: &'a str,
    pub cache_id: &'a str,
    pub inputs: &'a [InputFingerprint],
    pub staged_pack_bytes: Option<u64>,
    pub staged_flags_bytes: Option<u64>,
    pub staged_reach_bytes: Option<u64>,
    pub staged_canlight_bytes: Option<u64>,
    pub staged_pois_bytes: Option<u64>,
    pub pois_generator: &'a str,
}

impl BakeStamp {
    /// `Ok(())` when the stamp covers the requested artifact set and both
    /// staged files are still the stamped ones; `Err(reason)` otherwise.
    pub fn covers(&self, expected: &StampExpectation<'_>) -> Result<(), String> {
        if self.revision != expected.revision {
            return Err(format!(
                "staged navigation is revision {}, build selects {}",
                self.revision, expected.revision
            ));
        }
        if self.format != expected.format {
            return Err(format!(
                "staged navigation format {} is not {}",
                self.format, expected.format
            ));
        }
        if self.generator != expected.generator {
            return Err("bake generator changed (nav bake/format sources or generator id)".into());
        }
        if self.cache_id != expected.cache_id {
            return Err("selected cache identity changed".into());
        }
        match expected.staged_pack_bytes {
            None => return Err(format!("staged pack {} is missing", self.relative_pack)),
            Some(bytes) if bytes != self.pack_bytes => {
                return Err(format!(
                    "staged pack {} is {bytes} bytes, stamped {}",
                    self.relative_pack, self.pack_bytes
                ))
            }
            Some(_) => {}
        }
        match expected.staged_flags_bytes {
            None => return Err(format!("staged flags {} are missing", self.relative_flags)),
            Some(bytes) if bytes != self.flags_bytes => {
                return Err(format!(
                    "staged flags {} are {bytes} bytes, stamped {}",
                    self.relative_flags, self.flags_bytes
                ))
            }
            Some(_) => {}
        }
        match expected.staged_reach_bytes {
            None => return Err(format!("staged reach {} is missing", self.relative_reach)),
            Some(bytes) if bytes != self.reach_bytes => {
                return Err(format!(
                    "staged reach {} is {bytes} bytes, stamped {}",
                    self.relative_reach, self.reach_bytes
                ))
            }
            Some(_) => {}
        }
        match expected.staged_canlight_bytes {
            None => {
                return Err(format!(
                    "staged canlight {} is missing",
                    self.relative_canlight
                ))
            }
            Some(bytes) if bytes != self.canlight_bytes => {
                return Err(format!(
                    "staged canlight {} is {bytes} bytes, stamped {}",
                    self.relative_canlight, self.canlight_bytes
                ))
            }
            Some(_) => {}
        }
        if self.pois_sha256.is_none() {
            return Err("staged navpois is missing".into());
        }
        if self.pois_generator.as_deref() != Some(expected.pois_generator) {
            return Err("navpois generator changed".into());
        }
        match expected.staged_pois_bytes {
            None => {
                return Err(format!(
                    "staged navpois {} is missing",
                    self.relative_pois.as_deref().unwrap_or("274bot.navpois")
                ))
            }
            Some(bytes) if self.pois_bytes != Some(bytes) => {
                return Err(format!(
                    "staged navpois is {bytes} bytes, stamped {}",
                    self.pois_bytes.unwrap_or(0)
                ))
            }
            Some(_) => {}
        }
        if self.inputs.len() != expected.inputs.len() {
            return Err(format!(
                "canonical inputs changed ({} stamped, {} present)",
                self.inputs.len(),
                expected.inputs.len()
            ));
        }
        for (stamped, present) in self.inputs.iter().zip(expected.inputs) {
            if stamped != present {
                return Err(format!("canonical input {} changed", present.path));
            }
        }
        Ok(())
    }
}

/// Merge the identity rows a build generated with the checked-in rows a
/// packager published: the generated row wins for its `(revision, cache_id)`
/// because it describes the artifacts this build staged, and a checked-in row
/// that is superseded is reported. Checked-in rows fill in identities this
/// build did not generate (prebuilt resource bundles).
pub fn merge_identity_rows(
    generated: Vec<NavIdentityRow>,
    checked_in: Vec<NavIdentityRow>,
) -> (Vec<NavIdentityRow>, Vec<String>) {
    let mut rows = generated;
    let mut notes = Vec::new();
    for row in checked_in {
        if rows
            .iter()
            .any(|kept| kept.revision == row.revision && kept.cache_id == row.cache_id)
        {
            notes.push(format!(
                "checked-in identity for revision {} cache {} is superseded by this build's artifacts",
                row.revision, row.cache_id
            ));
            continue;
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        (a.revision, &a.cache_id, &a.relative_path).cmp(&(
            b.revision,
            &b.cache_id,
            &b.relative_path,
        ))
    });
    (rows, notes)
}

/// Install resource root for a build: `BOT_NAV_RESOURCE_DIR` when set (a
/// macOS bundle's `Contents/Resources`, an installer staging dir), otherwise
/// the cargo profile directory the built binaries land in — which is exactly
/// the directory [`crate::bundle`] consumers resolve at runtime for an
/// ordinary executable. `out_dir` is the build script's `OUT_DIR`.
pub fn resource_root_for_build(
    out_dir: &Path,
    override_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        if dir.as_os_str().is_empty() {
            return Err("BOT_NAV_RESOURCE_DIR is empty".into());
        }
        return normalize(dir);
    }
    let build_dir = out_dir
        .parent()
        .and_then(Path::parent)
        .filter(|build| build.file_name().is_some_and(|name| name == "build"))
        .ok_or_else(|| {
            format!(
                "unexpected build script output layout {}; set BOT_NAV_RESOURCE_DIR",
                out_dir.display()
            )
        })?;
    let profile_dir = build_dir.parent().ok_or_else(|| {
        format!(
            "unexpected build script output layout {}; set BOT_NAV_RESOURCE_DIR",
            out_dir.display()
        )
    })?;
    Ok(profile_dir.to_path_buf())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| format!("working directory: {e}"))?;
    let joined = cwd.join(path);
    if joined
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "resource directory {} must not escape its root",
            path.display()
        ));
    }
    Ok(joined)
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

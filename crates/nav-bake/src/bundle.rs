//! Build-time navigation bundling: the identities and machine-local stamps
//! that let an ordinary application build bake the selected revision's nav
//! artifacts once, stage them in the install resource layout, and reuse them
//! on every warm build, plus the required canonical content inventory the
//! build checks before it bakes or reuses anything.
//!
//! Nothing here is runtime state: the runtime keeps its cheap format/revision
//! checks and the explicit external overrides (`--nav-pack` / `NAV_PACK`).
//! Cache and world stay shared across clients.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use nav_planner::manifest;

/// One canonical input file's machine-local identity. Content hashes are
/// captured with the metadata so a single snapshot drives source/cache
/// identities and later staleness checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFingerprint {
    pub path: String,
    pub bytes: u64,
    pub modified_nanos: u64,
    #[serde(default)]
    pub sha256: String,
}

impl InputFingerprint {
    pub fn of(path: &Path) -> Result<Self, crate::GenerationError> {
        let metadata = std::fs::metadata(path).map_err(|source| {
            crate::SourceInputError::Io {
                context: crate::SourceInputContext::BakeInput,
                path: path.to_path_buf(),
                source,
            }
        })?;
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| age.as_nanos() as u64)
            .unwrap_or(0);
        let sha256 = nav_planner::manifest::hash_file(path).map_err(|detail| {
            crate::SourceInputError::HashFailed {
                path: path.to_path_buf(),
                detail,
            }
        })?;
        Ok(Self {
            path: path.to_string_lossy().into_owned(),
            bytes: metadata.len(),
            modified_nanos,
            sha256,
        })
    }
}

/// Fingerprint every regular file under `root` (recursively) plus each
/// explicit file, sorted by path. A missing `root` or explicit file is an
/// error: canonical inputs are required, not optional.
pub fn fingerprints(
    root: &Path,
    files: &[&Path],
) -> Result<Vec<InputFingerprint>, crate::GenerationError> {
    let mut paths = Vec::new();
    collect_files(root, &mut paths)?;
    for file in files {
        let metadata = std::fs::metadata(file).map_err(|source| crate::SourceInputError::Io {
            context: crate::SourceInputContext::BakeInput,
            path: file.to_path_buf(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(crate::SourceInputError::NotRegularFile {
                path: file.to_path_buf(),
            }
            .into());
        }
        paths.push((*file).to_path_buf());
    }
    paths.sort();
    paths.dedup();
    let mut rows = paths
        .iter()
        .map(|path| InputFingerprint::of(path))
        .collect::<Result<Vec<_>, crate::GenerationError>>()?;
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows.dedup_by(|a, b| a.path == b.path);
    Ok(rows)
}

/// Content-address the conservative baker input closure using an existing
/// fingerprint snapshot. Paths are relative to the content root; VCS
/// metadata is not a baker input. Explicit inputs are labelled by argument
/// position so provenance is machine-independent.
pub fn source_digest_from_fingerprints(
    root: &Path,
    files: &[&Path],
    inputs: &[InputFingerprint],
) -> Result<String, crate::GenerationError> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"274NAVSOURCE01");
    let mut source_inputs = Vec::new();
    for input in inputs {
        let path = Path::new(&input.path);
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        // Preserve path-component order while hashing a platform-neutral
        // `/`-joined label.
        let mut label = String::new();
        for component in relative.components() {
            if !label.is_empty() {
                label.push('/');
            }
            label.push_str(&component.as_os_str().to_string_lossy());
        }
        source_inputs.push((relative, label, input));
    }
    source_inputs.sort_unstable_by(|(left_path, _, left), (right_path, _, right)| {
        left_path
            .components()
            .cmp(right_path.components())
            .then_with(|| left.sha256.cmp(&right.sha256))
    });
    for (_, label, input) in source_inputs {
        digest.update((label.len() as u64).to_be_bytes());
        digest.update(label.as_bytes());
        digest.update(input.sha256.as_bytes());
    }
    for (index, path) in files.iter().enumerate() {
        let input = fingerprint_for_path(inputs, path)?;
        digest.update((index as u64).to_be_bytes());
        digest.update(input.sha256.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Find one row in the path-sorted snapshot produced by [`fingerprints`].
pub fn fingerprint_for_path<'a>(
    inputs: &'a [InputFingerprint],
    path: &Path,
) -> Result<&'a InputFingerprint, crate::GenerationError> {
    let label = path.to_string_lossy();
    inputs
        .binary_search_by(|input| input.path.as_str().cmp(label.as_ref()))
        .ok()
        .map(|index| &inputs[index])
        .ok_or_else(|| {
            crate::SourceInputError::NotFingerprinted {
                path: path.to_path_buf(),
            }
        })
}

/// Reconstruct the selected cache identity from an already captured input
/// snapshot instead of hashing the same archive set a second time.
pub fn cache_manifest_from_fingerprints(
    revision: u16,
    cache_dir: &Path,
    inputs: &[InputFingerprint],
) -> Result<manifest::CacheManifest, crate::GenerationError> {
    let mut archives = BTreeMap::new();
    for name in nav_planner::manifest::CacheManifest::ARCHIVES {
        let path = cache_dir.join(name);
        let input = fingerprint_for_path(inputs, &path)?;
        archives.insert(name.to_string(), input.sha256.clone());
    }
    Ok(manifest::CacheManifest { revision, archives })
}

/// Content-address the conservative baker input closure. Paths are relative
/// to the content root; VCS metadata is not a baker input. Explicit inputs
/// are labelled by argument position so provenance is machine-independent.
pub fn source_digest(root: &Path, files: &[&Path]) -> Result<String, crate::GenerationError> {
    let inputs = fingerprints(root, files)?;
    source_digest_from_fingerprints(root, files, &inputs)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), crate::GenerationError> {
    let entries = std::fs::read_dir(dir).map_err(|source| crate::SourceInputError::Io {
        context: crate::SourceInputContext::BakeInputDirectory,
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| crate::SourceInputError::Io {
            context: crate::SourceInputContext::BakeInputDirectory,
            path: dir.to_path_buf(),
            source,
        })?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let metadata = std::fs::metadata(&path).map_err(|source| crate::SourceInputError::Io {
            context: crate::SourceInputContext::BakeInput,
            path: path.clone(),
            source,
        })?;
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
        289 => Some(include_str!(
            "../../nav-planner/src/required-content-289.tsv"
        )),
        274 => Some(include_str!(
            "../../nav-planner/src/required-content-274.tsv"
        )),
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
pub fn required_content_inputs(
    revision: u16,
) -> Result<Vec<RequiredInput>, crate::GenerationError> {
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
            return Err(crate::SourceInputError::UnknownInventoryKind {
                revision,
                line: index + 1,
                kind: kind_name.to_owned(),
            }
            .into());
        };
        let Some(path) = columns.next().filter(|path| !path.is_empty()) else {
            return Err(crate::SourceInputError::MissingInventoryPath {
                revision,
                line: index + 1,
            }
            .into());
        };
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(crate::SourceInputError::InvalidInventoryPath {
                revision,
                line: index + 1,
                path: path.to_owned(),
            }
            .into());
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
pub fn missing_content_inputs(
    revision: u16,
    content_dir: &Path,
) -> Result<Vec<String>, crate::GenerationError> {
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
    /// Pack format identity (`274V16`); a format bump invalidates.
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
    #[serde(default)]
    pub manifest_sha256: Option<String>,
    #[serde(default)]
    pub manifest_bytes: Option<u64>,
    pub relative_pack: String,
    pub relative_flags: String,
    pub relative_reach: String,
    pub relative_canlight: String,
    #[serde(default)]
    pub relative_manifest: Option<String>,
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
    pub staged_pack_sha256: Option<&'a str>,
    pub staged_flags_bytes: Option<u64>,
    pub staged_flags_sha256: Option<&'a str>,
    pub staged_reach_bytes: Option<u64>,
    pub staged_reach_sha256: Option<&'a str>,
    pub staged_canlight_bytes: Option<u64>,
    pub staged_canlight_sha256: Option<&'a str>,
    pub staged_manifest_bytes: Option<u64>,
    pub staged_manifest_sha256: Option<&'a str>,
    pub staged_pois_bytes: Option<u64>,
    pub staged_pois_sha256: Option<&'a str>,
    pub pois_generator: &'a str,
}

impl BakeStamp {
    /// `Ok(())` when the stamp covers the requested artifact set and both
    /// staged files are still the stamped ones; `Err(reason)` otherwise.
    pub fn covers(&self, expected: &StampExpectation<'_>) -> Result<(), crate::GenerationError> {
        if self.revision != expected.revision {
            return Err(crate::StaleStampError::RevisionMismatch {
                stamped: self.revision,
                selected: expected.revision,
            }
            .into());
        }
        if self.format != expected.format {
            return Err(crate::StaleStampError::FormatMismatch {
                stamped: self.format.clone(),
                selected: expected.format.to_owned(),
            }
            .into());
        }
        if self.generator != expected.generator {
            return Err(crate::StaleStampError::GeneratorChanged.into());
        }
        if self.cache_id != expected.cache_id {
            return Err(crate::StaleStampError::CacheIdentityChanged.into());
        }
        check_staged_size(
            crate::StagedArtifact::Pack,
            &self.relative_pack,
            self.pack_bytes,
            expected.staged_pack_bytes,
        )?;
        check_staged_size(
            crate::StagedArtifact::Flags,
            &self.relative_flags,
            self.flags_bytes,
            expected.staged_flags_bytes,
        )?;
        check_staged_size(
            crate::StagedArtifact::Reach,
            &self.relative_reach,
            self.reach_bytes,
            expected.staged_reach_bytes,
        )?;
        check_staged_size(
            crate::StagedArtifact::Canlight,
            &self.relative_canlight,
            self.canlight_bytes,
            expected.staged_canlight_bytes,
        )?;
        check_staged_digest(
            crate::StagedArtifact::Pack,
            &self.nav_sha256,
            expected.staged_pack_sha256,
        )?;
        check_staged_digest(
            crate::StagedArtifact::Flags,
            &self.flags_sha256,
            expected.staged_flags_sha256,
        )?;
        check_staged_digest(
            crate::StagedArtifact::Reach,
            &self.reach_sha256,
            expected.staged_reach_sha256,
        )?;
        check_staged_digest(
            crate::StagedArtifact::Canlight,
            &self.canlight_sha256,
            expected.staged_canlight_sha256,
        )?;
        let Some(stamped_manifest_sha256) = self.manifest_sha256.as_deref() else {
            return Err(crate::StaleStampError::ManifestDigestMissingFromStamp.into());
        };
        check_staged_digest(
            crate::StagedArtifact::Manifest,
            stamped_manifest_sha256,
            expected.staged_manifest_sha256,
        )?;
        match (self.manifest_bytes, expected.staged_manifest_bytes) {
            (None, _) => {
                return Err(crate::StaleStampError::ManifestBytesMissingFromStamp.into())
            }
            (_, None) => {
                return Err(crate::StaleStampError::ManifestBytesMissingFromDisk.into())
            }
            (Some(stamped), Some(actual)) if stamped != actual => {
                return Err(crate::StaleStampError::ManifestSizeMismatch {
                    actual_bytes: actual,
                    stamped_bytes: stamped,
                }
                .into())
            }
            (Some(_), Some(_)) => {}
        }
        if self.relative_manifest.is_none() {
            return Err(crate::StaleStampError::ManifestPathMissingFromStamp.into());
        }
        let Some(stamped_pois_sha256) = self.pois_sha256.as_deref() else {
            return Err(crate::StaleStampError::NavPoisDigestMissingFromStamp.into());
        };
        if self.pois_generator.as_deref() != Some(expected.pois_generator) {
            return Err(crate::StaleStampError::NavPoisGeneratorChanged {
                stamped: self.pois_generator.clone(),
                expected: expected.pois_generator.to_owned(),
            }
            .into());
        }
        match expected.staged_pois_bytes {
            None => {
                return Err(crate::StaleStampError::StagedArtifactMissing {
                    artifact: crate::StagedArtifact::NavPois,
                    path: self
                        .relative_pois
                        .as_deref()
                        .unwrap_or("274bot.navpois")
                        .to_owned(),
                }
                .into())
            }
            Some(actual) => match self.pois_bytes {
                None => return Err(crate::StaleStampError::NavPoisBytesMissingFromStamp.into()),
                Some(stamped_bytes) if stamped_bytes != actual => {
                    return Err(crate::StaleStampError::NavPoisSizeMismatch {
                        actual_bytes: actual,
                        stamped_bytes,
                    }
                    .into())
                }
                Some(_) => {}
            },
        }
        let Some(actual_pois_sha256) = expected.staged_pois_sha256 else {
            return Err(crate::StaleStampError::StagedDigestMissing {
                artifact: crate::StagedArtifact::NavPois,
            }
            .into());
        };
        if stamped_pois_sha256 != actual_pois_sha256 {
            return Err(crate::StaleStampError::StagedDigestMismatch {
                artifact: crate::StagedArtifact::NavPois,
            }
            .into());
        }
        if self.inputs.len() != expected.inputs.len() {
            return Err(crate::StaleStampError::CanonicalInputCountMismatch {
                stamped: self.inputs.len(),
                present: expected.inputs.len(),
            }
            .into());
        }
        for (stamped, present) in self.inputs.iter().zip(expected.inputs) {
            if stamped != present {
                return Err(crate::StaleStampError::CanonicalInputChanged {
                    path: present.path.clone(),
                }
                .into());
            }
        }
        Ok(())
    }
}

fn check_staged_size(
    artifact: crate::StagedArtifact,
    path: &str,
    stamped_bytes: u64,
    actual_bytes: Option<u64>,
) -> Result<(), crate::GenerationError> {
    let Some(actual_bytes) = actual_bytes else {
        return Err(crate::StaleStampError::StagedArtifactMissing {
            artifact,
            path: path.to_owned(),
        }
        .into());
    };
    if actual_bytes != stamped_bytes {
        return Err(crate::StaleStampError::StagedArtifactSizeMismatch {
            artifact,
            path: path.to_owned(),
            actual_bytes,
            stamped_bytes,
        }
        .into());
    }
    Ok(())
}

fn check_staged_digest(
    artifact: crate::StagedArtifact,
    stamped: &str,
    actual: Option<&str>,
) -> Result<(), crate::GenerationError> {
    let Some(actual) = actual else {
        return Err(crate::StaleStampError::StagedDigestMissing { artifact }.into());
    };
    if stamped != actual {
        return Err(crate::StaleStampError::StagedDigestMismatch { artifact }.into());
    }
    Ok(())
}

/// Merge the identity rows a build generated with the checked-in rows a
/// packager published: the generated row wins for its `(revision, cache_id)`
/// because it describes the artifacts this build staged, and a checked-in row
/// that is superseded is reported. Checked-in rows fill in identities this
/// build did not generate (prebuilt resource bundles).
pub fn merge_identity_rows(
    generated: Vec<manifest::NavIdentityRow>,
    checked_in: Vec<manifest::NavIdentityRow>,
) -> (Vec<manifest::NavIdentityRow>, Vec<String>) {
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
/// the directory runtime resource consumers resolve for an
/// ordinary executable. `out_dir` is the build script's `OUT_DIR`.
pub fn resource_root_for_build(
    out_dir: &Path,
    override_dir: Option<&Path>,
) -> Result<PathBuf, crate::GenerationError> {
    if let Some(dir) = override_dir {
        if dir.as_os_str().is_empty() {
            return Err(crate::GenerationError::EmptyResourceDirectory);
        }
        return normalize(dir);
    }
    let build_dir = out_dir
        .parent()
        .and_then(Path::parent)
        .filter(|build| build.file_name().is_some_and(|name| name == "build"))
        .ok_or_else(|| crate::GenerationError::UnexpectedBuildOutputLayout {
            out_dir: out_dir.to_path_buf(),
        })?;
    let profile_dir =
        build_dir
            .parent()
            .ok_or_else(|| crate::GenerationError::UnexpectedBuildOutputLayout {
                out_dir: out_dir.to_path_buf(),
            })?;
    Ok(profile_dir.to_path_buf())
}

fn normalize(path: &Path) -> Result<PathBuf, crate::GenerationError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir()
        .map_err(|source| crate::GenerationError::WorkingDirectoryRead { source })?;
    let joined = cwd.join(path);
    if joined
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(crate::GenerationError::ResourceDirectoryEscapesRoot {
            path: path.to_path_buf(),
        });
    }
    Ok(joined)
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

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
        let label = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy();
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
    /// Pack format identity (`274V9`); a format bump invalidates.
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
        return Ok(normalize(dir)?);
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
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn stamp(generator: &str, format: &str, cache_id: &str, pack_bytes: u64) -> BakeStamp {
        BakeStamp {
            content_id: None,
            source_sha256: None,
            generator: generator.into(),
            format: format.into(),
            revision: 289,
            cache_id: cache_id.into(),
            cache_manifest: None,
            nav_sha256: "ab".repeat(32),
            flags_sha256: "cd".repeat(32),
            reach_sha256: "ef".repeat(32),
            canlight_sha256: "12".repeat(32),
            canlight_identity: "34".repeat(32),
            pack_bytes,
            flags_bytes: 7,
            reach_bytes: 9,
            canlight_bytes: 5,
            relative_pack: "nav/289/274bot.navpack".into(),
            relative_flags: "nav/289/274bot.navflags".into(),
            relative_reach: "nav/289/274bot.navreach".into(),
            relative_canlight: "nav/289/274bot.navcanlight".into(),
            inputs: vec![InputFingerprint {
                path: "/content/maps/m1.jm2".into(),
                bytes: 10,
                modified_nanos: 5,
            }],
        }
    }

    fn expectation<'a>(inputs: &'a [InputFingerprint]) -> StampExpectation<'a> {
        StampExpectation {
            revision: 289,
            format: "274V8",
            generator: "gen-1",
            cache_id: "cache-1",
            inputs,
            staged_pack_bytes: Some(11),
            staged_flags_bytes: Some(7),
            staged_reach_bytes: Some(9),
            staged_canlight_bytes: Some(5),
        }
    }

    #[test]
    fn a_matching_stamp_covers_and_every_change_is_stale() {
        let baked = stamp("gen-1", "274V8", "cache-1", 11);
        let inputs = baked.inputs.clone();
        assert!(baked.covers(&expectation(&inputs)).is_ok());

        let mut changed = expectation(&inputs);
        changed.cache_id = "cache-2";
        assert!(baked
            .covers(&changed)
            .unwrap_err()
            .contains("cache identity"));

        let mut changed = expectation(&inputs);
        changed.generator = "gen-2";
        assert!(baked.covers(&changed).unwrap_err().contains("generator"));

        let mut changed = expectation(&inputs);
        changed.format = "274V9";
        assert!(baked.covers(&changed).unwrap_err().contains("format"));

        let mut changed = expectation(&inputs);
        changed.revision = 274;
        assert!(baked.covers(&changed).unwrap_err().contains("revision"));

        let mut changed = expectation(&inputs);
        changed.staged_pack_bytes = None;
        assert!(baked.covers(&changed).unwrap_err().contains("missing"));

        let mut changed = expectation(&inputs);
        changed.staged_flags_bytes = Some(8);
        assert!(baked.covers(&changed).unwrap_err().contains("bytes"));

        let mut changed = expectation(&inputs);
        changed.staged_reach_bytes = None;
        assert!(baked.covers(&changed).unwrap_err().contains("reach"));

        let mut changed = expectation(&inputs);
        changed.staged_canlight_bytes = None;
        assert!(baked.covers(&changed).unwrap_err().contains("canlight"));

        let modified = [InputFingerprint {
            path: "/content/maps/m1.jm2".into(),
            bytes: 10,
            modified_nanos: 6,
        }];
        assert!(baked
            .covers(&expectation(&modified))
            .unwrap_err()
            .contains("changed"));

        let added = [
            modified[0].clone(),
            InputFingerprint {
                path: "/content/maps/m2.jm2".into(),
                bytes: 1,
                modified_nanos: 1,
            },
        ];
        assert!(baked
            .covers(&expectation(&added))
            .unwrap_err()
            .contains("inputs changed"));
    }

    #[test]
    fn content_digest_detects_same_size_replacement() {
        let root = std::env::temp_dir().join(format!("nav-source-digest-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("map.jm2");
        std::fs::write(&file, b"one").unwrap();
        let first = super::source_digest(&root, &[]).unwrap();
        std::fs::write(&file, b"two").unwrap();
        assert_ne!(first, super::source_digest(&root, &[]).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fingerprints_cover_the_tree_and_detect_edits() {
        let root = std::env::temp_dir().join(format!("274bot-bundle-{}", std::process::id()));
        let nested = root.join("maps");
        std::fs::create_dir_all(&nested).unwrap();
        let map = nested.join("m1.jm2");
        std::fs::write(&map, b"one").unwrap();
        let extra = root.join("..").join("outside.loc");
        std::fs::write(&extra, b"loc").unwrap();

        let first = fingerprints(&nested, &[&extra]).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first.iter().any(|row| row.path.ends_with("m1.jm2")));
        assert!(first.iter().any(|row| row.path.ends_with("outside.loc")));

        // Same bytes, newer mtime: still a change.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&map, b"one").unwrap();
        let second = fingerprints(&nested, &[&extra]).unwrap();
        assert_ne!(first, second);

        assert!(fingerprints(&root.join("missing"), &[]).is_err());
        std::fs::remove_file(&extra).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn generated_rows_win_over_superseded_checked_in_rows() {
        let generated = NavIdentityRow {
            revision: 289,
            cache_id: "cache".into(),
            content_id: None,
            source_sha256: None,
            format: "274V8".into(),
            nav_sha256: "aa".repeat(32),
            flags_sha256: None,
            reach_sha256: None,
            canlight_sha256: None,
            canlight_identity: None,
            relative_path: "nav/289/274bot.navpack".into(),
        };
        let superseded = NavIdentityRow {
            nav_sha256: "bb".repeat(32),
            ..generated.clone()
        };
        let kept = NavIdentityRow {
            revision: 274,
            cache_id: "other".into(),
            content_id: None,
            source_sha256: None,
            format: "274V8".into(),
            nav_sha256: "cc".repeat(32),
            flags_sha256: None,
            reach_sha256: None,
            canlight_sha256: None,
            canlight_identity: None,
            relative_path: "prebuilt/274bot.navpack".into(),
        };
        let (rows, notes) =
            merge_identity_rows(vec![generated.clone()], vec![superseded, kept.clone()]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], kept);
        assert_eq!(rows[1].nav_sha256, "aa".repeat(32));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("superseded"), "{}", notes[0]);
        // Checked-in rows alone (a skip-mode or prebuilt-only build) survive.
        let (rows, notes) = merge_identity_rows(Vec::new(), vec![kept.clone()]);
        assert_eq!(rows, vec![kept]);
        assert!(notes.is_empty());
    }

    #[test]
    fn artifact_layout_is_install_relative_and_revision_scoped() {
        let layout = artifact_layout(289);
        assert_eq!(layout.dir, PathBuf::from("nav/289"));
        assert_eq!(layout.relative_pack, "nav/289/274bot.navpack");
        assert_eq!(layout.relative_flags, "nav/289/274bot.navflags");
        assert_eq!(layout.relative_reach, "nav/289/274bot.navreach");
        assert_eq!(layout.relative_canlight, "nav/289/274bot.navcanlight");
        assert_eq!(layout.relative_stamp, "nav/289/nav-build.json");
        assert!(!Path::new(&layout.relative_pack).is_absolute());
        assert_eq!(artifact_layout(274).relative_pack, "nav/274/274bot.navpack");
        assert_ne!(artifact_layout(289).dir, artifact_layout(274).dir);
    }

    #[test]
    fn build_resource_root_is_the_profile_directory_or_the_override() {
        let out = Path::new("/w/target/debug/build/host-play-abc123/out");
        assert_eq!(
            resource_root_for_build(out, None).unwrap(),
            PathBuf::from("/w/target/debug")
        );
        // Cross builds keep the target-triple directory.
        let out = Path::new("/w/target/x86_64-pc-windows-msvc/release/build/host-play-abc/out");
        assert_eq!(
            resource_root_for_build(out, None).unwrap(),
            PathBuf::from("/w/target/x86_64-pc-windows-msvc/release")
        );
        // A macOS bundle (or installer staging dir) stages into its own
        // absolute directory instead; an absolute override passes through.
        let bundle = std::env::temp_dir().join("274bot.app/Contents/Resources");
        assert_eq!(resource_root_for_build(out, Some(&bundle)).unwrap(), bundle);
        assert!(resource_root_for_build(Path::new("/unexpected/out"), None).is_err());
        assert!(resource_root_for_build(out, Some(Path::new(""))).is_err());
    }

    // -----------------------------------------------------------------------
    // Required canonical content inputs.
    // -----------------------------------------------------------------------

    struct RequiredFixture(PathBuf);

    impl RequiredFixture {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "274bot-nav-required-{}-{}-{name}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for RequiredFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// Write one file of an inventory row under `root`, creating parents.
    fn write_required_file(root: &Path, path: &str) {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file, b"fixture").unwrap();
    }

    /// A fixture of exactly the inventory: every required input written.
    fn write_inventory_tree(root: &Path, revision: u16) {
        for input in required_content_inputs(revision).expect("inventory") {
            write_required_file(root, input.path);
        }
    }

    /// Whether a kind names a file of one of the baker's recursive scans.
    fn is_scan_kind(kind: RequiredKind) -> bool {
        matches!(
            kind,
            RequiredKind::Rs2 | RequiredKind::Constant | RequiredKind::Obj | RequiredKind::Loc
        )
    }

    /// The scan classes' roots and extensions, mirroring the baker
    /// (`transport.rs` `visit_rs2`, `script_constants`, `slash_weapon_ids`,
    /// `door_config_names`).
    const SCAN_CLASSES: &[(RequiredKind, &[&str], &str)] = &[
        (RequiredKind::Rs2, &["scripts"], "rs2"),
        (RequiredKind::Constant, &["scripts"], "constant"),
        (RequiredKind::Obj, &["scripts"], "obj"),
        (
            RequiredKind::Loc,
            &[
                "scripts/doors/configs",
                "scripts/quests",
                "scripts/areas",
                "scripts/general_use/configs",
            ],
            "loc",
        ),
    ];

    /// Every file of the scan classes under a canonical content tree, as
    /// `content-relative path → class kind`.
    fn scan_inputs(root: &Path) -> BTreeMap<String, RequiredKind> {
        let mut out = BTreeMap::new();
        for (kind, roots, extension) in SCAN_CLASSES {
            for root_rel in *roots {
                collect_scan_files(root, &root.join(root_rel), extension, *kind, &mut out);
            }
        }
        out
    }

    fn collect_scan_files(
        root: &Path,
        dir: &Path,
        extension: &str,
        kind: RequiredKind,
        out: &mut BTreeMap<String, RequiredKind>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_scan_files(root, &path, extension, kind, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some(extension) {
                let relative = path
                    .strip_prefix(root)
                    .expect("scan file is under the content root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, kind);
            }
        }
    }

    /// Every canonical mapsquare of a content tree, `maps/*.jm2`.
    fn canonical_maps(root: &Path) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if let Ok(entries) = std::fs::read_dir(root.join("maps")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".jm2") {
                    out.insert(format!("maps/{name}"));
                }
            }
        }
        out
    }

    /// The canonical local content root of a revision, the same default the
    /// build script resolves.
    fn canonical_content_root(revision: u16) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        let root = PathBuf::from(home).join(if revision == 289 {
            "experiments/lostcity-289/content"
        } else {
            "experiments/Server/content"
        });
        root.is_dir().then_some(root)
    }

    fn mapsquares(revision: u16) -> usize {
        required_content_inputs(revision)
            .expect("inventory")
            .iter()
            .filter(|row| row.kind == RequiredKind::Mapsquare)
            .count()
    }

    #[test]
    fn the_inventory_names_the_implicit_bake_inputs_of_each_release_revision() {
        let rows = required_content_inputs(289).expect("289 inventory");
        let paths: Vec<&str> = rows.iter().map(|row| row.path).collect();
        for expected in [
            "pack/loc.pack",
            "pack/obj.pack",
            "pack/varp.pack",
            "scripts/interface_bank/configs/bank_booth.loc",
            "scripts/ladders+stairs/scripts/ladders.rs2",
            "scripts/ladders+stairs/scripts/stairs.rs2",
            "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
            "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
            "scripts/areas/area_alkharid/configs/border_gate.loc",
            "scripts/minigames/game_ranging/configs/ranging.loc",
            "scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2",
            "scripts/skill_magic/configs/magic_spells.dbrow",
            "scripts/skill_magic/configs/enchanted_jewelry.obj",
            "scripts/skill_firemaking/configs/bank_zones.dbrow",
        ] {
            assert!(paths.contains(&expected), "{expected} is required");
        }
        // The scans' members are named one row each, not anchored by their
        // directory: the concrete holes of the round-1 guard.
        for scanned in [
            "scripts/skill_agility/scripts/shortcuts.rs2",
            "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
            "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
            "scripts/doors/configs/doubledoors.constant",
        ] {
            assert!(
                rows.iter()
                    .any(|row| row.path == scanned && is_scan_kind(row.kind)),
                "{scanned} is a scan-class input"
            );
        }
        // Every scan-class row is the file its kind promises, under the scan
        // roots, and every class is populated in both revisions.
        for revision in [289u16, 274] {
            let rows = required_content_inputs(revision).expect("inventory");
            for row in &rows {
                let extension = match row.kind {
                    RequiredKind::Rs2 => Some("rs2"),
                    RequiredKind::Constant => Some("constant"),
                    RequiredKind::Obj => Some("obj"),
                    RequiredKind::Loc => Some("loc"),
                    RequiredKind::Mapsquare => Some("jm2"),
                    RequiredKind::File => None,
                };
                if matches!(row.kind, RequiredKind::Mapsquare) {
                    assert!(row.path.starts_with("maps/"), "{}", row.path);
                } else if let Some(extension) = extension {
                    assert!(
                        row.path.ends_with(&format!(".{extension}")),
                        "{} is a {} row",
                        row.path,
                        extension
                    );
                    assert!(row.path.starts_with("scripts/"), "{}", row.path);
                }
            }
            for kind in [
                RequiredKind::Rs2,
                RequiredKind::Constant,
                RequiredKind::Obj,
                RequiredKind::Loc,
            ] {
                assert!(
                    rows.iter().any(|row| row.kind == kind),
                    "revision {revision} has {} rows",
                    kind.name()
                );
            }
        }
        assert_eq!(mapsquares(289), 534, "the canonical 289 map set");
        assert_eq!(mapsquares(274), 483, "the canonical 274 map set");
        let mut unique = paths.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), paths.len(), "no duplicate inventory rows");
        assert!(paths
            .iter()
            .all(|path| !path.starts_with('/') && !path.split('/').any(|part| part == "..")));
        // The two revisions are distinct inventories, and a revision without
        // one reports none (the build script rejects it before this point).
        assert_ne!(rows.len(), required_content_inputs(274).unwrap().len());
        assert!(required_content_inputs(1).expect("no inventory").is_empty());
    }

    #[test]
    fn a_complete_inventory_tree_passes_and_the_reduced_round_one_set_fails() {
        let complete = RequiredFixture::new("complete");
        write_inventory_tree(&complete.0, 289);
        assert!(
            missing_content_inputs(289, &complete.0)
                .expect("guard")
                .is_empty(),
            "a fixture of exactly the inventory is complete"
        );
        // Bytes are never the guard's business: an edit or an addition stays
        // eligible for the ordinary fingerprint/staleness path.
        std::fs::write(complete.0.join("pack/loc.pack"), b"edited bytes").unwrap();
        std::fs::write(
            complete.0.join("scripts/general_use/configs/extra.loc"),
            b"added",
        )
        .unwrap();
        std::fs::write(complete.0.join("maps/m99_99.jm2"), b"added").unwrap();
        assert!(missing_content_inputs(289, &complete.0)
            .expect("guard")
            .is_empty());

        // The round-1 native input set (docs/compat/release-p1b-native.md):
        // maps, the door configs and gates.loc, nothing else.
        let reduced = RequiredFixture::new("reduced");
        for input in required_content_inputs(289).expect("inventory") {
            let kept = match input.kind {
                RequiredKind::Mapsquare => true,
                _ => input.path.contains("doors/configs") || input.path.ends_with("gates.loc"),
            };
            if kept {
                write_required_file(&reduced.0, input.path);
            }
        }
        let missing = missing_content_inputs(289, &reduced.0).expect("guard");
        for expected in [
            "pack/loc.pack",
            "pack/obj.pack",
            "scripts/interface_bank/configs/bank_booth.loc",
            "scripts/ladders+stairs/scripts/ladders.rs2",
            "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
            "scripts/areas/area_alkharid/configs/border_gate.loc",
            "scripts/skill_magic/configs/magic_spells.dbrow",
            "scripts/skill_agility/scripts/shortcuts.rs2",
            "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        ] {
            assert!(
                missing.iter().any(|row| row.starts_with(expected)),
                "{expected} is reported missing: {missing:?}"
            );
        }
        assert!(
            !missing.iter().any(|row| row.starts_with("maps/")),
            "the reduced set keeps every mapsquare: {missing:?}"
        );
        assert!(missing.len() > 10, "the reduced set is far from complete");
    }

    #[test]
    fn a_missing_scan_child_is_reported_while_its_siblings_remain() {
        let root = RequiredFixture::new("scan-child");
        write_inventory_tree(&root.0, 289);
        assert!(missing_content_inputs(289, &root.0)
            .expect("guard")
            .is_empty());

        // Consumed children whose directory anchor stayed non-empty: the
        // concrete holes of the round-1 directory-anchor guard.
        let removed = [
            "scripts/skill_agility/scripts/shortcuts.rs2",
            "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
            "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
        ];
        for path in removed {
            std::fs::remove_file(root.0.join(path)).unwrap();
            let siblings = std::fs::read_dir(root.0.join(path).parent().unwrap())
                .unwrap()
                .count();
            assert!(siblings > 0, "{path}: its directory is not empty");
        }

        let missing = missing_content_inputs(289, &root.0).expect("guard");
        assert_eq!(missing.len(), removed.len(), "{missing:?}");
        for path in removed {
            assert!(
                missing
                    .iter()
                    .any(|row| row.starts_with(&format!("{path} ("))
                        && row.contains("recursive scripts/**/*.rs2 scan")),
                "{path} is reported missing: {missing:?}"
            );
        }
    }

    #[test]
    fn a_single_missing_mapsquare_or_named_input_is_reported() {
        let root = RequiredFixture::new("single-missing");
        write_inventory_tree(&root.0, 274);
        assert!(missing_content_inputs(274, &root.0)
            .expect("guard")
            .is_empty());

        let map = required_content_inputs(274)
            .expect("inventory")
            .into_iter()
            .find(|row| row.kind == RequiredKind::Mapsquare)
            .expect("a required mapsquare")
            .path
            .to_string();
        std::fs::remove_file(root.0.join(&map)).unwrap();
        std::fs::remove_file(root.0.join("scripts/ladders+stairs/scripts/stairs.rs2")).unwrap();

        let missing = missing_content_inputs(274, &root.0).expect("guard");
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(
            missing
                .iter()
                .any(|row| row.starts_with(&map)
                    && row.contains("canonical mapsquare of revision 274")),
            "{missing:?}"
        );
        assert!(
            missing
                .iter()
                .any(|row| row
                    .starts_with("scripts/ladders+stairs/scripts/stairs.rs2 (stair edges)")),
            "{missing:?}"
        );
    }

    #[test]
    fn a_missing_ranging_loc_is_reported_without_duplicating_scan_owned_rs2() {
        const RANGING_LOC: &str = "scripts/minigames/game_ranging/configs/ranging.loc";
        const RANGING_DOOR_RS2: &str =
            "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2";
        for revision in [289u16, 274] {
            let rows = required_content_inputs(revision).expect("inventory");
            let loc_rows: Vec<_> = rows.iter().filter(|row| row.path == RANGING_LOC).collect();
            assert_eq!(
                loc_rows.len(),
                1,
                "revision {revision}: {RANGING_LOC} is one named file row"
            );
            assert_eq!(
                loc_rows[0].kind,
                RequiredKind::File,
                "revision {revision}: ranging.loc is outside the loc scan roots"
            );
            assert!(
                rows.iter()
                    .filter(|row| row.path == RANGING_DOOR_RS2)
                    .all(|row| row.kind == RequiredKind::Rs2),
                "revision {revision}: {RANGING_DOOR_RS2} stays scan-owned only"
            );

            let root = RequiredFixture::new(&format!("ranging-loc-{revision}"));
            write_inventory_tree(&root.0, revision);
            std::fs::remove_file(root.0.join(RANGING_LOC)).unwrap();
            let missing = missing_content_inputs(revision, &root.0).expect("guard");
            assert_eq!(missing.len(), 1, "revision {revision}: {missing:?}");
            assert!(
                missing[0].starts_with(&format!("{RANGING_LOC} (Ranging Guild door edges)")),
                "revision {revision}: {missing:?}"
            );
        }
    }

    #[test]
    #[ignore = "reads the configured canonical 289/274 content trees"]
    fn the_canonical_content_trees_satisfy_their_inventory() {
        for revision in [289u16, 274] {
            let Some(root) = canonical_content_root(revision) else {
                println!("revision {revision}: no canonical content tree here");
                continue;
            };
            let rows = required_content_inputs(revision).expect("inventory");
            let missing = missing_content_inputs(revision, &root).expect("guard");
            assert!(missing.is_empty(), "{}: {missing:?}", root.display());

            // The scan classes are file membership of the canonical tree, not
            // a sample: every file the scans read is a row of its class (or a
            // named `file` row), and no class row names a file the tree does
            // not read. A mismatch here means the inventory needs refreshing.
            let scanned = scan_inputs(&root);
            let mut class_rows = 0;
            for row in &rows {
                if !is_scan_kind(row.kind) {
                    continue;
                }
                class_rows += 1;
                assert_eq!(
                    scanned.get(row.path).copied(),
                    Some(row.kind),
                    "revision {revision}: {} is not a canonical {} scan input",
                    row.path,
                    row.kind.name()
                );
            }
            for (path, kind) in &scanned {
                let row_kind = rows.iter().find(|row| row.path == path).map(|row| row.kind);
                assert!(
                    row_kind == Some(*kind) || row_kind == Some(RequiredKind::File),
                    "revision {revision}: canonical {} scan input {path} is {row_kind:?} \
                     in the inventory",
                    kind.name()
                );
            }

            // The mapsquare rows are the canonical map set, all of it.
            let maps: BTreeSet<String> = rows
                .iter()
                .filter(|row| row.kind == RequiredKind::Mapsquare)
                .map(|row| row.path.to_string())
                .collect();
            assert_eq!(maps, canonical_maps(&root), "the canonical map set");

            println!(
                "revision {revision}: {} required inputs ({class_rows} scan-class rows) present in {}",
                rows.len(),
                root.display()
            );
        }
    }
}

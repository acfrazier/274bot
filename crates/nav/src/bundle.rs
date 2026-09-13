//! Build-time navigation bundling: the identities and machine-local stamps
//! that let an ordinary application build bake the selected revision's nav
//! artifacts once, stage them in the install resource layout, and reuse them
//! on every warm build.
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
    pub revision: u16,
    pub cache_id: String,
    pub format: String,
    pub nav_sha256: String,
    pub flags_sha256: Option<String>,
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

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("bake input directory {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("bake input directory {}: {e}", dir.display()))?;
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

/// Install-relative layout of one revision's staged navigation artifacts,
/// under the resource root the running binary resolves (macOS
/// `Contents/Resources`, otherwise the executable's parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLayout {
    /// Install-relative directory holding the staged artifacts.
    pub dir: PathBuf,
    pub relative_pack: String,
    pub relative_flags: String,
    pub relative_manifest: String,
    pub relative_stamp: String,
}

/// `nav/<revision>/…` keeps revisions side by side: building the other
/// revision never overwrites the staged artifacts of this one.
pub fn artifact_layout(revision: u16) -> ArtifactLayout {
    let dir = PathBuf::from(format!("nav/{revision}"));
    ArtifactLayout {
        relative_pack: dir.join("274bot.navpack").to_string_lossy().into_owned(),
        relative_flags: dir.join("274bot.navflags").to_string_lossy().into_owned(),
        relative_manifest: dir
            .join("274bot.navpack.json")
            .to_string_lossy()
            .into_owned(),
        relative_stamp: dir.join("nav-build.json").to_string_lossy().into_owned(),
        dir,
    }
}

/// Machine-local stamp of one staged artifact set. It records the expensive
/// identities computed at bake time (cache identity, pack/flags digests) and
/// the input fingerprints that decide staleness on later builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BakeStamp {
    /// Generator identity of the bake, from [`crate::bake::generator_identity`].
    pub generator: String,
    /// Pack format identity (`274V8`); a format bump invalidates.
    pub format: String,
    pub revision: u16,
    pub cache_id: String,
    /// Manifest the verified cache identity came from, when one was supplied.
    pub cache_manifest: Option<String>,
    pub nav_sha256: String,
    pub flags_sha256: String,
    pub pack_bytes: u64,
    pub flags_bytes: u64,
    pub relative_pack: String,
    pub relative_flags: String,
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
    use super::*;

    fn stamp(generator: &str, format: &str, cache_id: &str, pack_bytes: u64) -> BakeStamp {
        BakeStamp {
            generator: generator.into(),
            format: format.into(),
            revision: 289,
            cache_id: cache_id.into(),
            cache_manifest: None,
            nav_sha256: "ab".repeat(32),
            flags_sha256: "cd".repeat(32),
            pack_bytes,
            flags_bytes: 7,
            relative_pack: "nav/289/274bot.navpack".into(),
            relative_flags: "nav/289/274bot.navflags".into(),
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
            format: "274V8".into(),
            nav_sha256: "aa".repeat(32),
            flags_sha256: None,
            relative_path: "nav/289/274bot.navpack".into(),
        };
        let superseded = NavIdentityRow {
            nav_sha256: "bb".repeat(32),
            ..generated.clone()
        };
        let kept = NavIdentityRow {
            revision: 274,
            cache_id: "other".into(),
            format: "274V8".into(),
            nav_sha256: "cc".repeat(32),
            flags_sha256: None,
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
        // A macOS bundle stages into Contents/Resources instead.
        assert_eq!(
            resource_root_for_build(
                out,
                Some(Path::new("/Applications/274bot.app/Contents/Resources"))
            )
            .unwrap(),
            PathBuf::from("/Applications/274bot.app/Contents/Resources")
        );
        assert!(resource_root_for_build(Path::new("/unexpected/out"), None).is_err());
        assert!(resource_root_for_build(out, Some(Path::new(""))).is_err());
    }
}

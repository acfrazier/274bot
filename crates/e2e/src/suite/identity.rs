//! Run identities: what a receipt binds, and what a resume is allowed to accept.
//!
//! A run records the manifest/reference identity, the host and client revision *including
//! the working-tree content*, the resolved executable identities (a real file hash, never
//! an unresolved command string), the resolved native profile/input configuration with
//! content digests for the catalog, vault, engine and cache paths, the settings and the
//! ordered selection.
//!
//! Resume accepts only an unchanged identity and selection, and it refuses an identity
//! with unresolved components: a run that could not bind an input cannot prove that input
//! is unchanged. Nothing here is inferred or invented — an unavailable value is recorded
//! as unresolved with its reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::manifest::{ExecTemplate, SuiteManifest};
use super::SuiteResult;

/// Bounds for a content digest of a tree. A tree past them is recorded as *unresolved*
/// (the suite refuses to claim an identity it did not read) instead of hashing unbounded
/// or pretending a path is an identity.
pub const MAX_DIGEST_FILES: usize = 20_000;
pub const MAX_DIGEST_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_DIGEST_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// One git tree's identity. `content_sha256` binds HEAD, the index and the working-tree
/// diff: the same path with different bytes is a different identity, which a bare
/// `dirty` flag cannot express.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    pub available: bool,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
    #[serde(default)]
    pub content_sha256: Option<String>,
    pub note: Option<String>,
}

impl GitIdentity {
    pub fn capture(root: &Path) -> Self {
        let commit = git(root, &["rev-parse", "HEAD"]);
        let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let status = git(root, &["status", "--porcelain"]);
        let index = git(root, &["ls-files", "-s"]);
        let diff = git_bytes(root, &["diff", "HEAD", "--binary"]);
        match (commit, status, index, diff) {
            (Some(commit), Some(status), Some(index), Some(diff)) => {
                let mut hasher = Sha256::new();
                hasher.update(commit.trim().as_bytes());
                hasher.update(b"\0");
                hasher.update(index.as_bytes());
                hasher.update(b"\0");
                hasher.update(&diff);
                hasher.update(b"\0");
                hasher.update(status.trim().as_bytes());
                GitIdentity {
                    available: true,
                    commit: Some(commit.trim().to_string()),
                    branch: branch.map(|b| b.trim().to_string()),
                    dirty: Some(!status.trim().is_empty()),
                    content_sha256: Some(format!("{:x}", hasher.finalize())),
                    note: None,
                }
            }
            _ => GitIdentity::unresolved(&format!(
                "git could not describe {}; the source content is unresolved",
                root.display()
            )),
        }
    }

    /// Whether this identity binds the tree's content, not only its path and claimed ref.
    pub fn resolved(&self) -> bool {
        self.available && self.content_sha256.is_some()
    }

    /// A tree that is not there at all: a *defined* absence, so the identity changes when
    /// the checkout (or submodule) appears later instead of silently passing resume.
    pub fn absent(reason: &str) -> Self {
        GitIdentity {
            available: false,
            commit: None,
            branch: None,
            dirty: None,
            content_sha256: Some(sha256(b"absent")),
            note: Some(reason.to_string()),
        }
    }

    /// A tree that exists but whose content the suite could not read: unresolved.
    pub fn unresolved(reason: &str) -> Self {
        GitIdentity {
            available: false,
            commit: None,
            branch: None,
            dirty: None,
            content_sha256: None,
            note: Some(reason.to_string()),
        }
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_bytes(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// The content identity of one resolved input (a file, a tree, or a declared absence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDigest {
    /// The path this identity describes, as the run resolved it.
    pub target: String,
    /// Content digest. `None` means unresolved (see `note`).
    pub sha256: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub files: usize,
    #[serde(default)]
    pub note: Option<String>,
}

impl InputDigest {
    pub fn resolved(&self) -> bool {
        self.sha256.is_some()
    }

    /// A single file's content identity.
    pub fn file(path: &Path) -> Self {
        let target = path.display().to_string();
        match std::fs::read(path) {
            Ok(bytes) => InputDigest {
                target,
                sha256: Some(sha256(&bytes)),
                bytes: Some(bytes.len() as u64),
                files: 1,
                note: None,
            },
            Err(error) => InputDigest {
                target,
                sha256: None,
                bytes: None,
                files: 0,
                note: Some(format!("cannot read the file: {error}")),
            },
        }
    }

    /// An input the run resolved to nothing: recorded as a defined absence, so a later
    /// resume that finds a file there refuses (the identity changed) rather than passing.
    pub fn absent(path: &Path, reason: &str) -> Self {
        InputDigest {
            target: path.display().to_string(),
            sha256: Some(sha256(b"absent")),
            bytes: Some(0),
            files: 0,
            note: Some(reason.to_string()),
        }
    }

    /// A directory tree's content identity: every regular file below `root`, sorted by
    /// relative path, hashed with its bytes. Past the bounds the digest is unresolved.
    pub fn tree(root: &Path) -> Self {
        let target = root.display().to_string();
        if !root.is_dir() {
            return InputDigest {
                target,
                sha256: None,
                bytes: None,
                files: 0,
                note: Some("not a directory".into()),
            };
        }
        let mut files: Vec<PathBuf> = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) => {
                    return InputDigest {
                        target,
                        sha256: None,
                        bytes: None,
                        files: files.len(),
                        note: Some(format!("cannot read {}: {error}", dir.display())),
                    }
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    files.push(path);
                }
            }
        }
        files.sort();
        if files.len() > MAX_DIGEST_FILES {
            return InputDigest {
                target,
                sha256: None,
                bytes: None,
                files: files.len(),
                note: Some(format!(
                    "{} files exceeds the suite's bounded digest of {MAX_DIGEST_FILES}",
                    files.len()
                )),
            };
        }
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        for path in &files {
            let Ok(bytes) = std::fs::read(path) else {
                return InputDigest {
                    target,
                    sha256: None,
                    bytes: None,
                    files: files.len(),
                    note: Some(format!("cannot read {}", path.display())),
                };
            };
            if bytes.len() as u64 > MAX_DIGEST_FILE_BYTES {
                return InputDigest {
                    target,
                    sha256: None,
                    bytes: None,
                    files: files.len(),
                    note: Some(format!(
                        "{} is {} bytes, past the suite's bounded digest",
                        path.display(),
                        bytes.len()
                    )),
                };
            }
            total += bytes.len() as u64;
            if total > MAX_DIGEST_TOTAL_BYTES {
                return InputDigest {
                    target,
                    sha256: None,
                    bytes: None,
                    files: files.len(),
                    note: Some(format!(
                        "the tree exceeds the suite's bounded digest of {MAX_DIGEST_TOTAL_BYTES} bytes"
                    )),
                };
            }
            let relative = path.strip_prefix(root).unwrap_or(path);
            hasher.update(relative.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            hasher.update(&bytes);
            hasher.update(b"\0");
        }
        InputDigest {
            target,
            sha256: Some(format!("{:x}", hasher.finalize())),
            bytes: Some(total),
            files: files.len(),
            note: None,
        }
    }

    /// The catalog's script sources: the part of the catalog a native run consumes.
    pub fn catalog(root: &Path) -> Self {
        let scripts = root.join("src/bot/scripts");
        let mut digest = Self::tree(&scripts);
        if digest.sha256.is_none() && digest.note.is_none() {
            digest.note = Some("the catalog carries no src/bot/scripts tree".into());
        }
        digest
    }
}

/// The executable a case will be launched from, and its content identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryIdentity {
    pub kind: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub note: Option<String>,
}

impl BinaryIdentity {
    pub fn direct(path: &Path) -> SuiteResult<Self> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("native executable {}: {error}", path.display()))?;
        Ok(BinaryIdentity {
            kind: "direct".into(),
            program: path.display().to_string(),
            args: Vec::new(),
            sha256: Some(sha256(&bytes)),
            size: Some(bytes.len() as u64),
            note: None,
        })
    }

    /// Resolve the manifest's cargo template to the artifact a build produced and hash it.
    ///
    /// `cargo run` at evaluation time may rebuild different bytes under the same command,
    /// so the suite never records a bare command as a runnable identity: it requires the
    /// executable to exist *before* the ledger is written and binds its bytes.
    pub fn resolve_template(
        template: &ExecTemplate,
        repo_root: Option<&Path>,
        flag: &str,
    ) -> SuiteResult<Self> {
        let name = template
            .args
            .iter()
            .enumerate()
            .find_map(|(index, arg)| {
                matches!(arg.as_str(), "--example" | "--bin")
                    .then(|| template.args.get(index + 1).cloned())
                    .flatten()
            })
            .ok_or_else(|| {
                format!(
                    "the manifest cargo template {} has no --bin/--example target; pass {flag} PATH",
                    template.program
                )
            })?;
        let examples = template.args.iter().any(|arg| arg == "--example");
        let root = repo_root.ok_or_else(|| {
            format!(
                "cannot resolve the {name} executable without a workspace root; pass {flag} PATH"
            )
        })?;
        let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => root.join("target"),
        };
        let relative = if examples {
            PathBuf::from("examples").join(&name)
        } else {
            PathBuf::from(&name)
        };
        let mut found: Vec<PathBuf> = ["release", "debug"]
            .into_iter()
            .map(|profile| target_dir.join(profile).join(&relative))
            .filter(|candidate| candidate.is_file())
            .collect();
        found.sort();
        let mut hashed: Vec<(PathBuf, String, u64)> = Vec::new();
        for candidate in &found {
            let bytes = std::fs::read(candidate)
                .map_err(|error| format!("resolved executable {}: {error}", candidate.display()))?;
            hashed.push((candidate.clone(), sha256(&bytes), bytes.len() as u64));
        }
        hashed.dedup_by(|a, b| a.1 == b.1);
        match hashed.len() {
            0 => Err(format!(
                "no built {name} executable under {}; the suite binds the executable it launches, \
                 so build it first (`cargo build -p panel {} {name}`) or pass {flag} PATH",
                target_dir.display(),
                if examples { "--example" } else { "--bin" }
            )),
            1 => {
                let (path, digest, size) = hashed.pop().expect("one candidate");
                Ok(BinaryIdentity {
                    kind: "resolved-cargo".into(),
                    program: path.display().to_string(),
                    args: template.args.clone(),
                    sha256: Some(digest),
                    size: Some(size),
                    note: Some(format!(
                        "resolved from the manifest cargo template ({} {}); build once before the run",
                        template.program,
                        template.args.join(" ")
                    )),
                })
            }
            _ => Err(format!(
                "the {} candidates under {} differ; pass {flag} PATH to name the executable to run",
                hashed.len(),
                target_dir.display()
            )),
        }
    }

    pub fn resolved(&self) -> bool {
        self.sha256.is_some()
    }

    pub fn matches(&self, other: &BinaryIdentity) -> bool {
        self.kind == other.kind
            && self.program == other.program
            && self.args == other.args
            && self.sha256 == other.sha256
    }
}

/// The explicit native profile/input configuration a run was started with, including the
/// content identity of every resolved input path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileIdentity {
    pub profile: String,
    pub revision: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub engine: Option<InputDigest>,
    pub cache: Option<InputDigest>,
    pub catalog: InputDigest,
    pub vault: InputDigest,
    pub lowmem: bool,
    pub mainland: bool,
    pub jobs: u32,
}

/// Suite settings that shape what a run does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsIdentity {
    pub level: Option<String>,
    pub only: Vec<String>,
    pub changed_paths: Vec<String>,
    pub changed_source: String,
    pub child_args: Vec<String>,
    pub child_env_keys: Vec<String>,
}

/// The whole identity block written into the ledger and compared on resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunIdentity {
    pub manifest_sha256: String,
    pub suite_id: String,
    pub reference_commit: String,
    pub reference_tree: String,
    pub reference_archive_sha256: String,
    pub host: GitIdentity,
    pub client: GitIdentity,
    pub binaries: BTreeMap<String, BinaryIdentity>,
    pub profile: ProfileIdentity,
    pub settings: SettingsIdentity,
    pub selection: Vec<String>,
}

impl RunIdentity {
    pub fn value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn hash(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        sha256(canonical.as_bytes())
    }

    /// Identity components the suite could not bind to content. A run refuses to start
    /// with any, and a resume refuses an identity that carries any.
    pub fn unresolved(&self) -> Vec<String> {
        let mut unresolved = Vec::new();
        if !self.host.resolved() {
            unresolved.push("host source content".to_string());
        }
        if !self.client.resolved() {
            unresolved.push("client source content".to_string());
        }
        for (kind, binary) in &self.binaries {
            if !binary.resolved() {
                unresolved.push(format!("{kind} executable content"));
            }
        }
        if !self.profile.catalog.resolved() {
            unresolved.push("catalog script content".to_string());
        }
        for (name, digest) in [
            ("engine", &self.profile.engine),
            ("cache", &self.profile.cache),
        ] {
            if let Some(digest) = digest {
                if !digest.resolved() {
                    unresolved.push(format!("{name} content"));
                }
            }
        }
        if !self.profile.vault.resolved() {
            unresolved.push("vault content".to_string());
        }
        unresolved
    }

    /// Human-readable differences between a stored identity and the current one. Empty
    /// means resume is allowed.
    pub fn differences(&self, other: &RunIdentity) -> Vec<String> {
        let mut differences = Vec::new();
        let mut note = |field: &str| differences.push(field.to_string());
        if self.manifest_sha256 != other.manifest_sha256 {
            note("manifest_sha256");
        }
        if self.suite_id != other.suite_id {
            note("suite_id");
        }
        if self.reference_commit != other.reference_commit
            || self.reference_tree != other.reference_tree
            || self.reference_archive_sha256 != other.reference_archive_sha256
        {
            note("reference");
        }
        if self.host != other.host {
            note("host revision");
        }
        if self.client != other.client {
            note("client revision");
        }
        if self.binaries.len() != other.binaries.len()
            || self.binaries.iter().any(|(kind, binary)| {
                other.binaries.get(kind).map(|o| !binary.matches(o)) != Some(false)
            })
        {
            note("native binary");
        }
        if self.profile != other.profile {
            note("profile/input configuration");
        }
        if self.settings != other.settings {
            note("settings");
        }
        if self.selection != other.selection {
            note("selection");
        }
        differences
    }
}

pub struct IdentityInputs<'a> {
    pub manifest_bytes: &'a [u8],
    pub manifest: &'a SuiteManifest,
    pub repo_root: Option<PathBuf>,
    pub client_root: Option<PathBuf>,
    pub binaries: BTreeMap<String, BinaryIdentity>,
    pub profile: ProfileIdentity,
    pub settings: SettingsIdentity,
    pub selection: &'a [String],
}

pub fn capture(inputs: &IdentityInputs<'_>) -> RunIdentity {
    let host = inputs
        .repo_root
        .as_deref()
        .map(GitIdentity::capture)
        .unwrap_or_else(|| GitIdentity::absent("no repository root"));
    let client = inputs
        .client_root
        .as_deref()
        .map(GitIdentity::capture)
        .unwrap_or_else(|| GitIdentity::absent("client submodule not present"));
    RunIdentity {
        manifest_sha256: sha256(inputs.manifest_bytes),
        suite_id: inputs.manifest.suite_id.clone(),
        reference_commit: inputs.manifest.provenance.reference_commit.clone(),
        reference_tree: inputs.manifest.provenance.reference_tree.clone(),
        reference_archive_sha256: inputs.manifest.provenance.reference_archive_sha256.clone(),
        host,
        client,
        binaries: inputs.binaries.clone(),
        profile: inputs.profile.clone(),
        settings: inputs.settings.clone(),
        selection: inputs.selection.to_vec(),
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Resolve the workspace root by walking up from the current directory, and the client
/// submodule under it. Both are optional; absence is recorded, never guessed.
pub fn repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn client_root(root: &Path) -> Option<PathBuf> {
    let client = root.join("vendor/fr-client-rust");
    client.join(".git").exists().then_some(client)
}

/// Compact summary used in the ledger header (no paths that leak a local campaign
/// layout, no credentials).
pub fn summary_value(identity: &RunIdentity) -> Value {
    json!({
        "suite_id": identity.suite_id,
        "manifest_sha256": identity.manifest_sha256,
        "reference_commit": identity.reference_commit,
        "host_commit": identity.host.commit,
        "host_dirty": identity.host.dirty,
        "host_content_sha256": identity.host.content_sha256,
        "client_commit": identity.client.commit,
        "binaries": identity.binaries.len(),
        "catalog_sha256": identity.profile.catalog.sha256,
        "vault_sha256": identity.profile.vault.sha256,
        "unresolved": identity.unresolved(),
        "selection": identity.selection.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(sha: &str) -> InputDigest {
        InputDigest {
            target: "/catalog".into(),
            sha256: Some(sha.into()),
            bytes: Some(1),
            files: 1,
            note: None,
        }
    }

    fn identity() -> RunIdentity {
        serde_json::from_value(json!({
            "manifest_sha256": "a",
            "suite_id": "s",
            "reference_commit": "c",
            "reference_tree": "t",
            "reference_archive_sha256": "z",
            "host": {"available": true, "commit": "h", "branch": "b", "dirty": false, "content_sha256": "hc", "note": null},
            "client": {"available": true, "commit": "k", "branch": null, "dirty": false, "content_sha256": "kc", "note": null},
            "binaries": {"core": {"kind": "direct", "program": "p", "args": [], "sha256": "d", "size": 1, "note": null}},
            "profile": {
                "profile": "local-274", "revision": null, "host": null, "port": null,
                "engine": null, "cache": null,
                "catalog": {"target": "/catalog", "sha256": "cat", "bytes": 2, "files": 1, "note": null},
                "vault": {"target": "/vault", "sha256": "vv", "bytes": 2, "files": 1, "note": null},
                "lowmem": true, "mainland": false, "jobs": 1
            },
            "settings": {
                "level": "quick", "only": [], "changed_paths": [], "changed_source": "supplied",
                "child_args": [], "child_env_keys": []
            },
            "selection": ["thiever"]
        }))
        .unwrap()
    }

    #[test]
    fn resume_differences_name_every_changed_component() {
        let base = identity();
        assert!(base.differences(&base).is_empty());
        assert!(base.unresolved().is_empty(), "{:?}", base.unresolved());

        let mut changed = base.clone();
        changed.manifest_sha256 = "b".into();
        assert_eq!(changed.differences(&base), vec!["manifest_sha256"]);

        // A same-path, changed-content catalog refuses: the digest is what is compared.
        let mut changed = base.clone();
        changed.profile.catalog = digest("other-catalog");
        assert_eq!(
            changed.differences(&base),
            vec!["profile/input configuration"]
        );

        let mut changed = base.clone();
        changed.profile.vault = digest("other-vault");
        assert_eq!(
            changed.differences(&base),
            vec!["profile/input configuration"]
        );

        let mut changed = base.clone();
        changed.selection = vec!["thiever".into(), "ardy_fighter".into()];
        assert_eq!(changed.differences(&base), vec!["selection"]);

        let mut changed = base.clone();
        changed.host.commit = Some("other".into());
        assert_eq!(changed.differences(&base), vec!["host revision"]);

        // The same commit with a different working tree is a different identity.
        let mut changed = base.clone();
        changed.host.content_sha256 = Some("other-content".into());
        assert_eq!(changed.differences(&base), vec!["host revision"]);

        let mut changed = base.clone();
        changed.binaries.get_mut("core").unwrap().sha256 = Some("other".into());
        assert_eq!(changed.differences(&base), vec!["native binary"]);

        let mut changed = base.clone();
        changed.settings.only = vec!["thiever".into()];
        assert_eq!(changed.differences(&base), vec!["settings"]);

        let mut changed = base.clone();
        changed.settings.child_env_keys = vec!["BOT_DEBUG".into()];
        assert_eq!(changed.differences(&base), vec!["settings"]);
    }

    #[test]
    fn unresolved_components_are_named_not_ignored() {
        let mut identity = identity();
        identity.host.content_sha256 = None;
        identity.profile.catalog.sha256 = None;
        identity.binaries.get_mut("core").unwrap().sha256 = None;
        assert_eq!(
            identity.unresolved(),
            vec![
                "host source content",
                "core executable content",
                "catalog script content"
            ]
        );
    }

    #[test]
    fn hash_is_stable_and_content_addressed() {
        let base = identity();
        assert_eq!(base.hash(), base.hash());
        let mut changed = base.clone();
        changed.selection.push("other".into());
        assert_ne!(base.hash(), changed.hash());
        assert_eq!(sha256(b"274bot").len(), 64);
    }

    #[test]
    fn unresolved_git_and_client_are_recorded_not_invented() {
        let missing = GitIdentity::capture(Path::new("/definitely/not/a/repo"));
        assert!(!missing.available);
        assert!(missing.commit.is_none());
        assert!(!missing.resolved());
        assert!(missing.note.is_some());
    }

    #[test]
    fn tree_digests_bind_content_and_stay_bounded() {
        let root = std::env::temp_dir().join(format!("274bot-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a.ts"), "one").unwrap();
        std::fs::write(root.join("nested/b.ts"), "two").unwrap();

        let first = InputDigest::tree(&root);
        assert!(first.resolved(), "{:?}", first.note);
        assert_eq!(first.files, 2);
        assert_eq!(first.bytes, Some(6));

        // Same path, same size, different bytes: the digest changes.
        std::fs::write(root.join("a.ts"), "eno").unwrap();
        let second = InputDigest::tree(&root);
        assert_ne!(first.sha256, second.sha256);

        let absent = InputDigest::absent(&root.join("missing"), "no vault file");
        assert!(absent.resolved(), "an absence is a defined identity");
        assert_ne!(absent.sha256, second.sha256);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_digest_covers_the_script_sources_only() {
        let root = std::env::temp_dir().join(format!("274bot-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/bot/scripts/Thiever")).unwrap();
        std::fs::write(root.join("src/bot/scripts/index.ts"), "registry").unwrap();
        std::fs::write(root.join("src/bot/scripts/Thiever/index.ts"), "script").unwrap();
        std::fs::write(root.join("package.json"), "{\"name\":\"x\"}").unwrap();
        let digest = InputDigest::catalog(&root);
        assert!(digest.resolved(), "{:?}", digest.note);
        assert_eq!(digest.files, 2, "the package.json is not a script source");

        // A script edit changes the bound identity; an unrelated file does not.
        std::fs::write(root.join("src/bot/scripts/Thiever/index.ts"), "script v2").unwrap();
        let edited = InputDigest::catalog(&root);
        assert_ne!(digest.sha256, edited.sha256);
        std::fs::write(root.join("package.json"), "{\"name\":\"y\"}").unwrap();
        assert_eq!(edited.sha256, InputDigest::catalog(&root).sha256);
        std::fs::remove_dir_all(root).unwrap();

        // A catalog without the script tree is unresolved, never invented.
        let missing = std::env::temp_dir().join("274bot-catalog-absent");
        let _ = std::fs::remove_dir_all(&missing);
        std::fs::create_dir_all(&missing).unwrap();
        let digest = InputDigest::catalog(&missing);
        assert!(!digest.resolved());
        assert!(digest.note.is_some());
        std::fs::remove_dir_all(missing).unwrap();
    }
}

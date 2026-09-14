//! Run identities: what a receipt binds, and what a resume is allowed to accept.
//!
//! A run records the manifest/reference identity, the host and client revision, the
//! launched binary identities, the native profile/input configuration, the settings and
//! the ordered selection. Resume accepts only an unchanged identity and selection; every
//! component is compared, and nothing here is inferred or invented — an unavailable
//! value is recorded as unresolved with its reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::manifest::SuiteManifest;
use super::SuiteResult;

/// One git tree's identity. `available: false` records that git could not answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    pub available: bool,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
    pub note: Option<String>,
}

impl GitIdentity {
    pub fn capture(root: &Path) -> Self {
        let commit = git(root, &["rev-parse", "HEAD"]);
        let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let status = git(root, &["status", "--porcelain"]);
        match (commit, branch, status) {
            (Some(commit), branch, Some(status)) => GitIdentity {
                available: true,
                commit: Some(commit.trim().to_string()),
                branch: branch.map(|b| b.trim().to_string()),
                dirty: Some(!status.trim().is_empty()),
                note: None,
            },
            _ => GitIdentity {
                available: false,
                commit: None,
                branch: None,
                dirty: None,
                note: Some(format!("git could not describe {}", root.display())),
            },
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

/// The executable a case will be launched from, and its content identity when the suite
/// is the one choosing the file.
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

    pub fn cargo(program: &str, args: &[String]) -> Self {
        BinaryIdentity {
            kind: "cargo".into(),
            program: program.to_string(),
            args: args.to_vec(),
            sha256: None,
            size: None,
            note: Some("cargo run resolves and builds the executable; the suite records the command, not a file hash".into()),
        }
    }

    pub fn matches(&self, other: &BinaryIdentity) -> bool {
        self.kind == other.kind
            && self.program == other.program
            && self.args == other.args
            && self.sha256 == other.sha256
    }
}

/// The explicit native profile/input configuration a run was started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileIdentity {
    pub profile: String,
    pub revision: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub engine: Option<String>,
    pub cache: Option<String>,
    pub catalog: String,
    pub vault: Option<String>,
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
        .unwrap_or_else(|| GitIdentity {
            available: false,
            commit: None,
            branch: None,
            dirty: None,
            note: Some("no repository root".into()),
        });
    let client = inputs
        .client_root
        .as_deref()
        .map(GitIdentity::capture)
        .unwrap_or_else(|| GitIdentity {
            available: false,
            commit: None,
            branch: None,
            dirty: None,
            note: Some("client submodule not present".into()),
        });
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
        "client_commit": identity.client.commit,
        "binaries": identity.binaries.len(),
        "selection": identity.selection.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> RunIdentity {
        serde_json::from_value(json!({
            "manifest_sha256": "a",
            "suite_id": "s",
            "reference_commit": "c",
            "reference_tree": "t",
            "reference_archive_sha256": "z",
            "host": {"available": true, "commit": "h", "branch": "b", "dirty": false, "note": null},
            "client": {"available": true, "commit": "k", "branch": null, "dirty": false, "note": null},
            "binaries": {"core": {"kind": "direct", "program": "p", "args": [], "sha256": "d", "size": 1, "note": null}},
            "profile": {
                "profile": "local-274", "revision": null, "host": null, "port": null,
                "engine": null, "cache": null, "catalog": "/catalog", "vault": null,
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

        let mut changed = base.clone();
        changed.manifest_sha256 = "b".into();
        assert_eq!(changed.differences(&base), vec!["manifest_sha256"]);

        let mut changed = base.clone();
        changed.profile.catalog = "/other".into();
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
        assert!(missing.note.is_some());
    }
}

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
use std::io::Read;
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
/// Bound on directories visited. Without it, a tree of directories (or a directory the
/// walker cannot descend) costs work that no file bound would stop.
pub const MAX_DIGEST_DIRS: usize = 20_000;

/// The bounds a bounded tree digest is walked under. Production uses [`Bounds::default`];
/// the tests drive a tiny bound to prove the walk stops early.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    files: usize,
    file_bytes: u64,
    total_bytes: u64,
    dirs: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Bounds {
            files: MAX_DIGEST_FILES,
            file_bytes: MAX_DIGEST_FILE_BYTES,
            total_bytes: MAX_DIGEST_TOTAL_BYTES,
            dirs: MAX_DIGEST_DIRS,
        }
    }
}

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

impl Default for InputDigest {
    fn default() -> Self {
        InputDigest {
            target: String::new(),
            sha256: None,
            bytes: None,
            files: 0,
            note: Some("unspecified".into()),
        }
    }
}

impl InputDigest {
    pub fn resolved(&self) -> bool {
        self.sha256.is_some()
    }

    /// A single file's content identity: streamed under [`MAX_DIGEST_FILE_BYTES`].
    /// `fs::read` is not used; a metadata size check is not itself the cap.
    pub fn file(path: &Path) -> Self {
        match hash_file_capped(path, MAX_DIGEST_FILE_BYTES) {
            Ok((digest, bytes)) => InputDigest {
                target: path.display().to_string(),
                sha256: Some(digest),
                bytes: Some(bytes),
                files: 1,
                note: None,
            },
            Err(note) => InputDigest::unresolved_at(path, &note),
        }
    }

    /// An input that exists but whose content the suite could not bind.
    pub fn unresolved_at(path: &Path, reason: &str) -> Self {
        InputDigest {
            target: path.display().to_string(),
            sha256: None,
            bytes: None,
            files: 0,
            note: Some(reason.to_string()),
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
        Self::tree_bounded(root, Bounds::default())
    }

    /// [`InputDigest::tree`] with explicit bounds, so a test can prove the walk stops early
    /// instead of measuring the tree and rejecting it afterwards.
    ///
    /// The bounds are enforced *while* the tree is walked: a tree past them is unresolved
    /// before its files are read. Symlinks are rejected rather than followed — a link can
    /// leave the tree (or loop inside it), and the digest binds the paths a child resolves.
    /// An entry that cannot be read or stat'ed is unresolved, never silently dropped.
    fn tree_bounded(root: &Path, bounds: Bounds) -> Self {
        let target = root.display().to_string();
        let unresolved = |files: usize, note: String| InputDigest {
            target: target.clone(),
            sha256: None,
            bytes: None,
            files,
            note: Some(note),
        };
        match std::fs::symlink_metadata(root) {
            Ok(meta) if meta.is_dir() => {}
            Ok(meta) if meta.file_type().is_symlink() => {
                return unresolved(
                    0,
                    format!(
                    "{} is a symlink; a bounded digest binds the path, it does not follow links",
                    root.display()
                ),
                )
            }
            Ok(_) => return unresolved(0, "not a directory".into()),
            Err(error) => {
                return unresolved(0, format!("cannot stat {}: {error}", root.display()));
            }
        }
        let mut files: Vec<(PathBuf, u64)> = Vec::new();
        let mut dirs = 0usize;
        let mut total: u64 = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            dirs += 1;
            if dirs > bounds.dirs {
                return unresolved(
                    files.len(),
                    format!(
                        "the tree has more than {} directories, past the suite's bounded digest",
                        bounds.dirs
                    ),
                );
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) => {
                    return unresolved(
                        files.len(),
                        format!("cannot read {}: {error}", dir.display()),
                    );
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        return unresolved(
                            files.len(),
                            format!("cannot read an entry of {}: {error}", dir.display()),
                        );
                    }
                };
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(error) => {
                        return unresolved(
                            files.len(),
                            format!("cannot stat {}: {error}", path.display()),
                        );
                    }
                };
                if file_type.is_symlink() {
                    return unresolved(
                        files.len(),
                        format!(
                            "{} is a symlink; a bounded digest does not follow links",
                            path.display()
                        ),
                    );
                }
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    return unresolved(
                        files.len(),
                        format!("{} is not a regular file", path.display()),
                    );
                }
                let size = match entry.metadata() {
                    Ok(meta) => meta.len(),
                    Err(error) => {
                        return unresolved(
                            files.len(),
                            format!("cannot stat {}: {error}", path.display()),
                        );
                    }
                };
                // Bounded before the bytes are read: an oversized or oversized-in-total
                // file is unresolved instead of being loaded to find that out.
                if size > bounds.file_bytes {
                    return unresolved(
                        files.len(),
                        format!(
                            "{} is {size} bytes, past the suite's bounded digest of {} bytes per file",
                            path.display(),
                            bounds.file_bytes
                        ),
                    );
                }
                total += size;
                if total > bounds.total_bytes {
                    return unresolved(
                        files.len(),
                        format!(
                            "the tree exceeds the suite's bounded digest of {} bytes",
                            bounds.total_bytes
                        ),
                    );
                }
                files.push((path, size));
                if files.len() > bounds.files {
                    return unresolved(
                        files.len(),
                        format!(
                            "more than {} files, past the suite's bounded digest",
                            bounds.files
                        ),
                    );
                }
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hasher = Sha256::new();
        for (path, size) in &files {
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return unresolved(
                        files.len(),
                        format!("cannot read {}: {error}", path.display()),
                    );
                }
            };
            // The file can have changed between the walk and the read.
            if bytes.len() as u64 != *size || bytes.len() as u64 > bounds.file_bytes {
                return unresolved(
                    files.len(),
                    format!(
                        "{} changed while it was being digested ({} bytes read, {size} stat'ed)",
                        path.display(),
                        bytes.len()
                    ),
                );
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

fn is_not_found(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// Stream a file into SHA-256 without holding it, refusing past `cap` bytes.
fn hash_file_capped(path: &Path, cap: u64) -> Result<(String, u64), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let expected = file
        .metadata()
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?
        .len();
    if expected > cap {
        return Err(format!(
            "{} is {expected} bytes, past the suite's bounded digest of {cap} bytes per file",
            path.display()
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > cap {
            return Err(format!(
                "{} exceeded the suite's bounded digest of {cap} bytes per file while reading",
                path.display()
            ));
        }
        hasher.update(&buffer[..read]);
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

enum PathClass {
    Absent,
    File,
    Dir(PathBuf),
    Unresolved(String),
}

/// Classify one identity path. Only [`std::io::ErrorKind::NotFound`] is absence.
/// A symlink is followed once (the child follows a linked vault/cache); a dangling
/// link, an unsupported type, or any other error is unresolved.
fn classify_input(path: &Path) -> PathClass {
    match std::fs::symlink_metadata(path) {
        Err(error) if is_not_found(&error) => PathClass::Absent,
        Err(error) => PathClass::Unresolved(format!("cannot stat {}: {error}", path.display())),
        Ok(meta) if meta.file_type().is_symlink() => match std::fs::metadata(path) {
            Err(error) if is_not_found(&error) => PathClass::Unresolved(format!(
                "{} is a dangling symlink; the suite will not treat a link as a defined absence",
                path.display()
            )),
            Err(error) => {
                PathClass::Unresolved(format!("cannot follow {}: {error}", path.display()))
            }
            Ok(target) if target.is_file() => PathClass::File,
            Ok(target) if target.is_dir() => match std::fs::canonicalize(path) {
                Ok(canonical) => PathClass::Dir(canonical),
                Err(error) => PathClass::Unresolved(format!(
                    "cannot canonicalize {}: {error}",
                    path.display()
                )),
            },
            Ok(_) => PathClass::Unresolved(format!(
                "{} is a symlink to an unsupported file type",
                path.display()
            )),
        },
        Ok(meta) if meta.is_file() => PathClass::File,
        Ok(meta) if meta.is_dir() => PathClass::Dir(path.to_path_buf()),
        Ok(_) => PathClass::Unresolved(format!(
            "{} is not a regular file or directory",
            path.display()
        )),
    }
}

/// Content identity of one resolved input path. Only a missing path is a defined
/// absence; a link is followed once and hashed, and every other case fails closed.
pub fn bind_input(path: &Path) -> InputDigest {
    match classify_input(path) {
        PathClass::Absent => InputDigest::absent(
            path,
            "the resolved path does not exist; the panel creates its vault on first use and fills its cache dir from the engine",
        ),
        PathClass::File => InputDigest::file(path),
        PathClass::Dir(canonical) => InputDigest::tree(&canonical),
        PathClass::Unresolved(note) => InputDigest::unresolved_at(path, &note),
    }
}

/// P1 cache identity: the eight jag archives, not a walk of the pack directory.
pub fn bind_cache(path: &Path, revision: u16) -> InputDigest {
    match classify_input(path) {
        PathClass::Absent => InputDigest::absent(
            path,
            "the resolved cache directory does not exist; the panel fills it from the engine",
        ),
        PathClass::Unresolved(note) => InputDigest::unresolved_at(path, &note),
        PathClass::File => {
            InputDigest::unresolved_at(path, "cache path is a file, not a directory")
        }
        PathClass::Dir(canonical) => {
            match nav::manifest::CacheManifest::capture(revision, &canonical) {
                Ok(manifest) => InputDigest {
                    target: path.display().to_string(),
                    sha256: Some(manifest.identity()),
                    bytes: None,
                    files: nav::manifest::CacheManifest::ARCHIVES.len(),
                    note: Some("cache profile identity (jag archives)".into()),
                },
                Err(error) => InputDigest::unresolved_at(
                    path,
                    &format!("cannot capture cache profile identity: {error}"),
                ),
            }
        }
    }
}

/// Native consumed content: maps, door configs, and gates.loc — not the whole
/// content tree (models, sprites, fonts).
pub fn bind_content(path: &Path) -> InputDigest {
    match classify_input(path) {
        PathClass::Absent => {
            InputDigest::absent(path, "the resolved content directory does not exist")
        }
        PathClass::Unresolved(note) => InputDigest::unresolved_at(path, &note),
        PathClass::File => {
            InputDigest::unresolved_at(path, "content path is a file, not a directory")
        }
        PathClass::Dir(canonical) => {
            let inputs = nav::bake::content_inputs(&canonical);
            let maps = bind_input(&inputs.maps_dir);
            let doors = bind_input(&inputs.doors_dir);
            let gates = bind_input(&inputs.gates);
            for part in [&maps, &doors, &gates] {
                if !part.resolved() {
                    return InputDigest::unresolved_at(
                        path,
                        part.note.as_deref().unwrap_or("unresolved content input"),
                    );
                }
            }
            let mut hasher = Sha256::new();
            for part in [&maps, &doors, &gates] {
                hasher.update(part.target.as_bytes());
                hasher.update(b"\0");
                hasher.update(part.sha256.as_deref().unwrap_or("").as_bytes());
                hasher.update(b"\0");
            }
            InputDigest {
                target: path.display().to_string(),
                sha256: Some(format!("{:x}", hasher.finalize())),
                bytes: Some(
                    maps.bytes.unwrap_or(0) + doors.bytes.unwrap_or(0) + gates.bytes.unwrap_or(0),
                ),
                files: maps.files + doors.files + gates.files,
                note: Some("nav content inputs (maps, doors, gates)".into()),
            }
        }
    }
}

/// The RSA pem the local profile reads from the engine install. Never the engine tree.
pub fn bind_engine_pem(engine_dir: &Path) -> InputDigest {
    bind_input(&engine_dir.join("data/config/private.pem"))
}

/// Unpack is derived runtime of the bound cache. An explicit override must be a
/// pack directory the cache identity can name, otherwise resume is refused.
pub fn bind_unpack(path: &Path, revision: u16, overridden: bool) -> InputDigest {
    if !overridden {
        match classify_input(path) {
            PathClass::Absent => InputDigest::absent(
                path,
                "default unpack is derived runtime of the bound cache; the path is recorded, the cache archives are the content identity",
            ),
            PathClass::Unresolved(note) => InputDigest::unresolved_at(path, &note),
            _ => InputDigest {
                target: path.display().to_string(),
                sha256: Some(sha256(b"unpack-derived-runtime")),
                bytes: Some(0),
                files: 0,
                note: Some(
                    "default unpack is derived runtime; cache archives are the bound content"
                        .into(),
                ),
            },
        }
    } else {
        bind_cache(path, revision)
    }
}

/// Canonicalize a path the suite will both hash and launch. Relative paths are
/// refused: the child would resolve them against a different working directory.
pub fn canonicalize_launch_path(path: &Path, flag: &str) -> SuiteResult<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(format!("{flag} is empty"));
    }
    if path.is_relative() {
        return Err(format!(
            "{flag} {} is relative; the child resolves it against its own working directory, so \
             the suite cannot bind the path it launches. Pass an absolute path",
            path.display()
        ));
    }
    std::fs::canonicalize(path).map_err(|error| format!("{flag} {}: {error}", path.display()))
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
        let canonical = canonicalize_launch_path(path, "native executable")?;
        let (digest, size) = hash_file_capped(&canonical, MAX_DIGEST_FILE_BYTES)
            .map_err(|error| format!("native executable {}: {error}", canonical.display()))?;
        Ok(BinaryIdentity {
            kind: "direct".into(),
            program: canonical.display().to_string(),
            args: Vec::new(),
            sha256: Some(digest),
            size: Some(size),
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
            Some(dir) if !dir.is_empty() => {
                canonicalize_launch_path(Path::new(&dir), "CARGO_TARGET_DIR")?
            }
            _ => root.join("target"),
        };
        let executable_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let relative = if examples {
            PathBuf::from("examples").join(&executable_name)
        } else {
            PathBuf::from(&executable_name)
        };
        let mut found: Vec<PathBuf> = ["release", "debug"]
            .into_iter()
            .map(|profile| target_dir.join(profile).join(&relative))
            .filter(|candidate| candidate.is_file())
            .collect();
        found.sort();
        let mut hashed: Vec<(PathBuf, String, u64)> = Vec::new();
        for candidate in &found {
            let canonical = canonicalize_launch_path(candidate, "resolved executable")?;
            let (digest, size) = hash_file_capped(&canonical, MAX_DIGEST_FILE_BYTES)
                .map_err(|error| format!("resolved executable {}: {error}", canonical.display()))?;
            hashed.push((canonical, digest, size));
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

/// The bound external loader source: the path the child resolves and the content identity of
/// the file at that path.
///
/// The loader smoke is the one case whose input is a raw TypeScript file rather than a catalog
/// tree, so a resume has to bind *that* path and *those* bytes. The producer materializes its
/// own temporary copy for the load/reload legs, so the receipt's `script.path` is never
/// equated with this path — only its `script.sha256` is compared against [`Self::sha256`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSource {
    /// The resolved input path (canonical, absolute), as the child resolves it.
    pub path: String,
    /// SHA-256 of the file at that path.
    pub sha256: String,
    pub bytes: u64,
    /// Whether the child's own compile-time default fixture was bound (no `--external-ts`).
    pub default_fixture: bool,
    /// The producer's frozen fixture digest (`host_play::external_loader::FROZEN_SHA256`).
    pub frozen_sha256: String,
    /// Whether the bound bytes are the frozen fixture. The producer refuses to materialize any
    /// other content, so a mismatch can only fail; recording it keeps that visible instead of
    /// silently claiming the input is the frozen one.
    pub frozen_match: bool,
    /// What the producer's harmless whitespace reload of this input hashes to: the bound bytes
    /// with one trailing newline (`apply_harmless_whitespace` writes exactly that to its owned
    /// copy). A qualified receipt's changed-reload source hash *and* its reloaded raw-source
    /// cache key must both equal it.
    pub harmless_whitespace_sha256: String,
}

impl ExternalSource {
    /// Resolve the external input the child will read: an explicit absolute `--external-ts`
    /// path, or the producer's own tracked default fixture.
    ///
    /// A relative path is refused for the same reason every other launch path is: the child
    /// resolves it against its own working directory, so the suite could not bind the file it
    /// reads. This runs only when the selection actually launches the loader smoke.
    pub fn resolve(explicit: Option<&Path>) -> SuiteResult<Self> {
        let default_fixture = explicit.is_none();
        let path = match explicit {
            Some(path) => canonicalize_launch_path(path, "--external-ts")?,
            None => canonicalize_launch_path(
                &host_play::external_loader::default_frozen_source(),
                "the external loader default fixture",
            )?,
        };
        let size = std::fs::metadata(&path)
            .map_err(|error| format!("external loader source {}: {error}", path.display()))?
            .len();
        if size > MAX_DIGEST_FILE_BYTES {
            return Err(format!(
                "external loader source {} is {size} bytes, past the suite's bounded digest of {} \
                 bytes",
                path.display(),
                MAX_DIGEST_FILE_BYTES
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("external loader source {}: {error}", path.display()))?;
        let digest = sha256(&bytes);
        let mut whitespace = bytes.clone();
        whitespace.push(b'\n');
        Ok(ExternalSource {
            frozen_match: digest == host_play::external_loader::FROZEN_SHA256,
            frozen_sha256: host_play::external_loader::FROZEN_SHA256.to_string(),
            path: path.display().to_string(),
            sha256: digest,
            bytes: size,
            default_fixture,
            harmless_whitespace_sha256: sha256(&whitespace),
        })
    }

    /// One line for the ledger header and case identity: what was bound, and whether it is the
    /// producer's frozen fixture.
    pub fn summary(&self) -> String {
        format!(
            "{} (sha256 {}, {} bytes, {}{})",
            self.path,
            self.sha256,
            self.bytes,
            if self.default_fixture {
                "default fixture"
            } else {
                "--external-ts"
            },
            if self.frozen_match {
                ", frozen fixture"
            } else {
                ", not the frozen fixture"
            }
        )
    }
}

/// The paths the *native* resolver selected for a configuration, recorded as the child
/// itself resolves them.
///
/// The suite does not invent these: it runs the same read-only resolver the panel runs over
/// the effective argv the child receives, with the child's cwd and env. A recorded path is
/// part of the run identity; the content-bound inputs on [`ProfileIdentity`] say which of
/// them were read as well.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedInputs {
    /// The native selection (`local-274`, `local-289`, `public-289`).
    pub selection: String,
    pub cache: String,
    pub vault: String,
    pub nav_pack: String,
    pub nav_flags: String,
    pub content: String,
    pub unpack: String,
}

/// The explicit native profile/input configuration a run was started with, including the
/// content identity of every input the suite binds.
///
/// Catalog scripts, the selected vault, the P1 cache jag identity, nav pack/flags, the
/// nav content inputs (maps/doors/gates), and the engine RSA pem are content-bound.
/// Default unpack is derived runtime of that cache (path recorded; cache archives are
/// the content). An explicit unpack override must be identifiable as a cache pack or the
/// run refuses. The engine *tree* is never hashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileIdentity {
    pub profile: String,
    pub revision: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub resolved: ResolvedInputs,
    pub engine: Option<InputDigest>,
    pub cache: InputDigest,
    pub catalog: InputDigest,
    pub vault: InputDigest,
    #[serde(default)]
    pub nav_pack: InputDigest,
    #[serde(default)]
    pub nav_flags: InputDigest,
    #[serde(default)]
    pub content: InputDigest,
    #[serde(default)]
    pub unpack: InputDigest,
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
    #[serde(default)]
    pub nav_paints: bool,
    pub child_env_keys: Vec<String>,
    /// Canonical working directory the children are launched in.
    #[serde(default)]
    pub cwd: String,
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
    /// The bound external loader source, when the selection launches the loader smoke. `None`
    /// for an ordinary core/pair run: an external resource a selection does not launch is never
    /// resolved, let alone required to resolve.
    #[serde(default)]
    pub external: Option<ExternalSource>,
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
        if !self.profile.cache.resolved() {
            unresolved.push("cache content".to_string());
        }
        if let Some(engine) = &self.profile.engine {
            if !engine.resolved() {
                unresolved.push("engine content".to_string());
            }
        }
        if !self.profile.vault.resolved() {
            unresolved.push("vault content".to_string());
        }
        if !self.profile.nav_pack.resolved() {
            unresolved.push("nav pack content".to_string());
        }
        if !self.profile.nav_flags.resolved() {
            unresolved.push("nav flags content".to_string());
        }
        if !self.profile.content.resolved() {
            unresolved.push("content inputs".to_string());
        }
        if !self.profile.unpack.resolved() {
            unresolved.push("unpack content".to_string());
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
        if self.external != other.external {
            note("external loader source");
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
    pub external: Option<ExternalSource>,
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
        external: inputs.external.clone(),
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
                "selection": "local-274",
                "resolved": {"selection": "local-274", "cache": "/cache", "vault": "/vault", "nav_pack": "/nav", "nav_flags": "/navflags", "content": "/content", "unpack": "/unpack"},
                "engine": null,
                "cache": {"target": "/cache", "sha256": "cc", "bytes": 2, "files": 1, "note": null},
                "catalog": {"target": "/catalog", "sha256": "cat", "bytes": 2, "files": 1, "note": null},
                "vault": {"target": "/vault", "sha256": "vv", "bytes": 2, "files": 1, "note": null},
                "nav_pack": {"target": "/nav", "sha256": "np", "bytes": 1, "files": 1, "note": null},
                "nav_flags": {"target": "/navflags", "sha256": "nf", "bytes": 1, "files": 1, "note": null},
                "content": {"target": "/content", "sha256": "cn", "bytes": 1, "files": 1, "note": null},
                "unpack": {"target": "/unpack", "sha256": "un", "bytes": 1, "files": 1, "note": null},
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

    /// The loader smoke's input identity is its path *and* its bytes, and only a selection
    /// that launches it resolves the file at all.
    #[test]
    fn external_source_binds_path_and_content_and_refuses_relative_inputs() {
        let fixture = host_play::external_loader::default_frozen_source();
        let default = ExternalSource::resolve(None).expect("the tracked fixture resolves");
        assert!(default.default_fixture);
        assert_eq!(default.sha256, host_play::external_loader::FROZEN_SHA256);
        assert!(
            default.frozen_match,
            "the tracked fixture is the frozen one"
        );
        assert_eq!(
            default.frozen_sha256,
            host_play::external_loader::FROZEN_SHA256
        );
        assert_eq!(
            std::fs::canonicalize(&fixture)
                .unwrap()
                .display()
                .to_string(),
            default.path
        );
        assert!(default.bytes > 0);
        // The whitespace transform the producer applies to its owned copy, hashed by the suite:
        // a real digest of the bound bytes plus one newline, not an assumed constant.
        let bytes = std::fs::read(&fixture).unwrap();
        let mut whitespace = bytes.clone();
        whitespace.push(b'\n');
        assert_eq!(
            default.harmless_whitespace_sha256,
            super::sha256(&whitespace)
        );
        assert_ne!(default.harmless_whitespace_sha256, default.sha256);

        let error = ExternalSource::resolve(Some(Path::new("ExampleBot.ts"))).unwrap_err();
        assert!(error.contains("relative"), "{error}");
        let error = ExternalSource::resolve(Some(Path::new("/definitely/missing/ExampleBot.ts")))
            .unwrap_err();
        assert!(error.contains("--external-ts"), "{error}");

        // A copy at another path with the same bytes is a different input identity.
        let dir = std::env::temp_dir().join(format!("274bot-external-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("ExampleBot.ts");
        std::fs::copy(&fixture, &copy).unwrap();
        let explicit = ExternalSource::resolve(Some(&copy)).unwrap();
        assert!(!explicit.default_fixture);
        assert!(explicit.frozen_match);
        assert_eq!(explicit.sha256, default.sha256);
        assert_ne!(explicit.path, default.path);
        assert_ne!(explicit, default, "the bound path is part of the identity");

        // The same path with changed bytes is a different identity, and records that the
        // producer's frozen contract no longer holds.
        std::fs::write(&copy, "// not the bot\n").unwrap();
        let changed = ExternalSource::resolve(Some(&copy)).unwrap();
        assert!(!changed.frozen_match);
        assert_ne!(changed.sha256, default.sha256);
        assert_ne!(changed, explicit);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// An ordinary core/pair identity carries no external component, and a resume refuses when
    /// the bound external source (bytes or path) changes.
    #[test]
    fn an_external_source_difference_refuses_resume_and_is_absent_by_default() {
        let base = identity();
        assert!(
            base.external.is_none(),
            "a core/pair selection binds no external input"
        );

        let mut bound = base.clone();
        bound.external = Some(ExternalSource {
            path: "/tmp/ExampleBot.ts".into(),
            sha256: "sha-one".into(),
            bytes: 10,
            default_fixture: true,
            frozen_sha256: host_play::external_loader::FROZEN_SHA256.into(),
            frozen_match: true,
            harmless_whitespace_sha256: "sha-whitespace".into(),
        });
        assert_eq!(bound.differences(&base), vec!["external loader source"]);
        assert_eq!(base.differences(&bound), vec!["external loader source"]);

        let mut changed = bound.clone();
        changed.external.as_mut().unwrap().sha256 = "sha-two".into();
        assert_eq!(changed.differences(&bound), vec!["external loader source"]);

        let mut path_changed = bound.clone();
        path_changed.external.as_mut().unwrap().path = "/tmp/other.ts".into();
        assert_eq!(
            path_changed.differences(&bound),
            vec!["external loader source"]
        );

        let mut unbound = bound.clone();
        unbound.external = None;
        assert_eq!(unbound.differences(&bound), vec!["external loader source"]);
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

    /// A tree that cannot be bounded is unresolved *before* it is walked unbounded. Each
    /// bound is checked where it applies: a symlink (which can leave the tree or loop) is
    /// rejected rather than followed, an oversized file is rejected before its bytes are
    /// read, and an entry that cannot be read is unresolved instead of silently dropped.
    #[test]
    fn a_tree_that_cannot_be_bounded_is_unresolved_not_walked_unbounded() {
        let root = std::env::temp_dir().join(format!("274bot-tree-bounds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real/a.ts"), "one").unwrap();

        // The file bound is enforced during the walk, not measured after it.
        let started = std::time::Instant::now();
        let bounded = InputDigest::tree_bounded(
            &root,
            Bounds {
                files: 0,
                ..Bounds::default()
            },
        );
        assert!(!bounded.resolved(), "{:?}", bounded.note);
        assert!(
            bounded
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("more than 0 files"),
            "{:?}",
            bounded.note
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));

        // A directory symlink is not followed: a link back to the tree's own parent would
        // otherwise loop without bound.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
            let started = std::time::Instant::now();
            let linked = InputDigest::tree(&root);
            assert!(!linked.resolved(), "{:?}", linked.note);
            assert!(
                linked
                    .note
                    .as_deref()
                    .unwrap_or_default()
                    .contains("symlink"),
                "{:?}",
                linked.note
            );
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "a symlink must be rejected, not followed"
            );
            std::fs::remove_file(root.join("loop")).unwrap();
        }

        // A file past the per-file bound is rejected before its bytes are read.
        let big = root.join("real/big.bin");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_DIGEST_FILE_BYTES + 1).unwrap();
        drop(file);
        let started = std::time::Instant::now();
        let oversized = InputDigest::tree(&root);
        assert!(!oversized.resolved(), "{:?}", oversized.note);
        assert!(
            oversized
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("past the suite's bounded digest"),
            "{:?}",
            oversized.note
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the bound is known from metadata, not by reading the file"
        );
        std::fs::remove_file(&big).unwrap();

        // A file the walker cannot read is unresolved, never silently skipped.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let secret = root.join("real/secret.ts");
            std::fs::write(&secret, "secret").unwrap();
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
            let unreadable = InputDigest::tree(&root);
            assert!(!unreadable.resolved(), "{:?}", unreadable.note);
            assert!(
                unreadable
                    .note
                    .as_deref()
                    .unwrap_or_default()
                    .contains("secret.ts"),
                "{:?}",
                unreadable.note
            );
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::remove_dir_all(&root).unwrap();
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

    #[test]
    fn bind_input_follows_a_linked_file_and_only_not_found_is_absence() {
        let root = std::env::temp_dir().join(format!("274bot-bind-input-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("vault-bytes");
        std::fs::write(&target, "vault v1").unwrap();
        let linked = root.join("vault");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &linked).unwrap();
            let first = bind_input(&linked);
            assert!(first.resolved(), "{:?}", first.note);
            std::fs::write(&target, "vault v2").unwrap();
            let second = bind_input(&linked);
            assert_ne!(first.sha256, second.sha256, "followed content must change");

            let dangling = root.join("dangling");
            std::os::unix::fs::symlink(root.join("missing-target"), &dangling).unwrap();
            let digest = bind_input(&dangling);
            assert!(!digest.resolved(), "{:?}", digest.note);
            assert!(
                digest
                    .note
                    .as_deref()
                    .unwrap_or_default()
                    .contains("dangling"),
                "{:?}",
                digest.note
            );
        }
        let absent = bind_input(&root.join("no-such"));
        assert!(absent.resolved(), "NotFound is a defined absence");
        assert_eq!(
            absent.sha256,
            InputDigest::absent(&root.join("no-such"), "x").sha256
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_past_the_digest_cap_is_unresolved_without_an_unbounded_read() {
        let path = std::env::temp_dir().join(format!("274bot-file-cap-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_DIGEST_FILE_BYTES + 1).unwrap();
        drop(file);
        let started = std::time::Instant::now();
        let digest = InputDigest::file(&path);
        assert!(!digest.resolved(), "{:?}", digest.note);
        assert!(
            digest
                .note
                .as_deref()
                .unwrap_or_default()
                .contains("past the suite's bounded digest"),
            "{:?}",
            digest.note
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::fs::remove_file(&path).unwrap();
    }
}

use std::path::{Component, Path, PathBuf};

/// The registry entry file below a root: `src/bot/scripts/index.ts`.
pub fn registry_index_path(root: &Path) -> PathBuf {
    root.join("src/bot/scripts/index.ts")
}

/// The on-disk file for a card's `./…` import path. rs2b0t imports end in
/// `.js` while the files on disk are `.ts`, so the `.ts` twin wins when
/// the verbatim path is absent. Returns `None` when `rel_path` is absolute,
/// contains `..`, or would resolve outside `root/src/bot/scripts`.
pub fn script_file_path(root: &Path, rel_path: &str) -> Option<PathBuf> {
    let base = root.join("src/bot/scripts");
    let resolved = resolve_under_catalog(&base, rel_path)?;

    let verbatim = base.join(strip_leading_dot_slash(rel_path));
    let candidate = if verbatim.is_file() {
        verbatim
    } else if let Some(stem) = rel_path.strip_suffix(".js") {
        let ts = base.join(format!("{}.ts", strip_leading_dot_slash(stem)));
        if ts.is_file() {
            ts
        } else {
            resolved
        }
    } else {
        resolved
    };

    canonical_under(&base, &candidate)
}

/// Join `rel_path` under `catalog` without touching the filesystem. Rejects
/// absolute paths, `..`, and anything not starting with `./`.
fn resolve_under_catalog(catalog: &Path, rel_path: &str) -> Option<PathBuf> {
    if rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return None;
    }
    let rel = rel_path.strip_prefix("./")?;
    let mut out = catalog.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn strip_leading_dot_slash(rel_path: &str) -> &str {
    rel_path.strip_prefix("./").unwrap_or(rel_path)
}

/// Canonicalize `path` when possible and require it stay under `catalog`.
fn canonical_under(catalog: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(catalog) {
        return None;
    }
    if let Ok(canon_catalog) = catalog.canonicalize() {
        if let Ok(canon_path) = path.canonicalize() {
            if !canon_path.starts_with(&canon_catalog) {
                return None;
            }
            return Some(canon_path);
        }
    }
    Some(path.to_path_buf())
}

const RS2B0T_IMPORT_DEFERRED: &str = "deferred";

/// Default first-run defer flag file (`~/.274bot/rs2b0t-import`).
pub fn default_rs2b0t_import_file() -> PathBuf {
    crate::bot_file("rs2b0t-import")
}

/// Whether the operator chose **Not now** on the first Browse catalog prompt.
pub fn rs2b0t_import_deferred_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.trim() == RS2B0T_IMPORT_DEFERRED)
        .unwrap_or(false)
}

/// [`rs2b0t_import_deferred_at`] against the default import file.
pub fn rs2b0t_import_deferred() -> bool {
    rs2b0t_import_deferred_at(&default_rs2b0t_import_file())
}

/// Record that the operator deferred the rs2b0t catalog import.
pub fn set_rs2b0t_import_deferred_at(path: &Path) -> Result<(), String> {
    vault::write_private_file(path, RS2B0T_IMPORT_DEFERRED.as_bytes())
        .map_err(|e| format!("rs2b0t-import: {e}"))
}

/// Clear the defer flag after a successful import.
pub fn clear_rs2b0t_import_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("rs2b0t-import: {e}")),
    }
}

/// Default persisted rs2b0t root file (`~/.274bot/rs2b0t-path`).
pub fn default_rs2b0t_path_file() -> PathBuf {
    crate::bot_file("rs2b0t-path")
}

/// Longest `rs2b0t-path` file accepted: it holds one path.
const MAX_ROOT_FILE_BYTES: u64 = 8 * 1024;

/// The rs2b0t checkout root: `$RS2B0T` first, else the path persisted by a
/// previous successful parse. A persisted file that is refused (see
/// [`rs2b0t_root_checked_at`]) counts as no root and is reported once on
/// stderr; choosing the catalog again writes a fresh, trusted file.
pub fn rs2b0t_root_at(path_file: &Path) -> Option<PathBuf> {
    match rs2b0t_root_checked_at(path_file) {
        Ok(root) => root,
        Err(refusal) => {
            static REPORTED: std::sync::Once = std::sync::Once::new();
            REPORTED.call_once(|| eprintln!("{refusal}"));
            None
        }
    }
}

/// [`rs2b0t_root_at`] with a refusal returned as an error naming the reason.
///
/// The persisted file decides which checkout's scripts are loaded and run, so
/// it is read with [`vault::read_private_file`] (a regular file, owned by this
/// user, that no one else can write) and its content must be an absolute path
/// without `..`. `$RS2B0T` is the operator's own environment and is taken as
/// given.
pub fn rs2b0t_root_checked_at(path_file: &Path) -> Result<Option<PathBuf>, String> {
    if let Some(root) = crate::rs2b0t_env() {
        if !root.as_os_str().is_empty() {
            return Ok(Some(root));
        }
    }
    let persisted = match vault::read_private_file(path_file, MAX_ROOT_FILE_BYTES) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("rs2b0t-path: {e}")),
    };
    let root = persisted.trim();
    if root.is_empty() {
        return Ok(None);
    }
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(format!(
            "rs2b0t-path: {} holds {}, which is not an absolute path",
            path_file.display(),
            root.display()
        ));
    }
    if root.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "rs2b0t-path: {} holds {}, which contains `..`",
            path_file.display(),
            root.display()
        ));
    }
    Ok(Some(root))
}

/// [`rs2b0t_root_at`] against the default persisted file.
pub fn rs2b0t_root() -> Option<PathBuf> {
    rs2b0t_root_at(&default_rs2b0t_path_file())
}

/// [`rs2b0t_root_checked_at`] against the default persisted file.
pub fn rs2b0t_root_checked() -> Result<Option<PathBuf>, String> {
    rs2b0t_root_checked_at(&default_rs2b0t_path_file())
}

/// `path` made absolute against the current directory and lexically cleaned
/// (`.` dropped, `..` folded), without touching the filesystem or resolving
/// symlinks. Persisted paths go through this so they mean the same file
/// wherever the host is started later, and pass restore's checks.
pub(crate) fn absolute_clean(path: &Path) -> std::io::Result<PathBuf> {
    let mut clean = PathBuf::new();
    for component in std::path::absolute(path)?.components() {
        match component {
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other.as_os_str()),
        }
    }
    Ok(clean)
}

/// Persist `root` to `path_file` (a previous successful parse recorded the
/// checkout path), as an absolute path. Writes only when the file would change.
pub fn persist_rs2b0t_root_at(root: &Path, path_file: &Path) -> Result<(), String> {
    let root = absolute_clean(root).map_err(|e| format!("rs2b0t-path: {}: {e}", root.display()))?;
    // The unchanged-file shortcut reads through the same checked, non-blocking
    // reader as the load path; a refused or unreadable file is rewritten.
    if let Ok(existing) = vault::read_private_file(path_file, MAX_ROOT_FILE_BYTES) {
        if existing.trim() == root.to_string_lossy() {
            return Ok(());
        }
    }
    vault::write_private_file(path_file, root.to_string_lossy().as_bytes())
        .map_err(|e| format!("rs2b0t-path: {e}"))
}

/// [`persist_rs2b0t_root_at`] against the default persisted file.
pub fn persist_rs2b0t_root(root: &Path) -> Result<(), String> {
    persist_rs2b0t_root_at(root, &default_rs2b0t_path_file())
}

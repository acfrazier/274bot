use std::path::{Path, PathBuf};

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

/// The rs2b0t checkout root: `$RS2B0T` first, else the path persisted by a
/// previous successful parse.
pub fn rs2b0t_root_at(path_file: &Path) -> Option<PathBuf> {
    if let Some(root) = crate::rs2b0t_env() {
        if !root.as_os_str().is_empty() {
            return Some(root);
        }
    }
    let persisted = std::fs::read_to_string(path_file).ok()?;
    let root = persisted.trim();
    if root.is_empty() {
        return None;
    }
    Some(PathBuf::from(root))
}

/// [`rs2b0t_root_at`] against the default persisted file.
pub fn rs2b0t_root() -> Option<PathBuf> {
    rs2b0t_root_at(&default_rs2b0t_path_file())
}

/// Persist `root` to `path_file` (a previous successful parse recorded the
/// checkout path). Writes only when the file would change.
pub fn persist_rs2b0t_root_at(root: &Path, path_file: &Path) -> Result<(), String> {
    if let Ok(existing) = std::fs::read_to_string(path_file) {
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

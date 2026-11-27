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

/// `path` made absolute against the current directory and lexically cleaned
/// (`.` dropped, `..` folded), without touching the filesystem or resolving
/// symlinks. Persisted paths go through this so they mean the same file
/// wherever the host is started later, and pass restore's checks.
pub fn absolute_clean(path: &Path) -> std::io::Result<PathBuf> {
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


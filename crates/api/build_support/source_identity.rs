use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Identity {
    commit: String,
    worktree_dirty: bool,
}

pub fn bake(manifest_dir: &Path) -> io::Result<()> {
    for name in [
        "GIT_COMMIT",
        "GITHUB_SHA",
        "GIT_DIRTY",
        "CLIENT_GIT_COMMIT",
        "CLIENT_GIT_DIRTY",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let host_repo = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "API manifest directory must be inside the repository's crates directory",
            )
        })?;
    let client_repo = host_repo.join("vendor/fr-client-rust");

    emit_git_rerun_paths(host_repo);
    emit_git_rerun_paths(&client_repo);

    let host = identity(host_repo, &["GIT_COMMIT", "GITHUB_SHA"], "GIT_DIRTY");
    let client = identity(&client_repo, &["CLIENT_GIT_COMMIT"], "CLIENT_GIT_DIRTY");
    let short: String = host.commit.chars().take(7).collect();

    println!("cargo:rustc-env=API_HOST_COMMIT={}", host.commit);
    println!("cargo:rustc-env=API_HOST_SHORT={short}");
    println!(
        "cargo:rustc-env=API_HOST_DIRTY={}",
        if host.worktree_dirty { "1" } else { "0" }
    );
    println!("cargo:rustc-env=API_CLIENT_COMMIT={}", client.commit);
    println!(
        "cargo:rustc-env=API_CLIENT_DIRTY={}",
        if client.worktree_dirty { "1" } else { "0" }
    );
    Ok(())
}

fn identity(repo: &Path, commit_overrides: &[&str], dirty_override: &str) -> Identity {
    let commit = commit_overrides
        .iter()
        .find_map(|name| env_nonempty(name))
        .or_else(|| git_text(repo, &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    let worktree_dirty = env_nonempty(dirty_override)
        .map(|value| truthy(&value))
        .unwrap_or_else(|| {
            commit != "unknown"
                && git_output(repo, &["status", "--porcelain"])
                    .is_some_and(|status| !status.is_empty())
        });
    Identity {
        commit,
        worktree_dirty,
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn truthy(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn git_output(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn git_text(repo: &Path, args: &[&str]) -> Option<String> {
    let output = String::from_utf8(git_output(repo, args)?).ok()?;
    let value = output.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn emit_git_rerun_paths(repo: &Path) {
    let git_marker = repo.join(".git");
    if git_marker.is_file() {
        emit_rerun_path(&git_marker);
    }

    for name in ["HEAD", "index", "packed-refs"] {
        emit_git_path(repo, name);
    }
    if let Some(branch) = git_text(repo, &["symbolic-ref", "-q", "HEAD"]) {
        emit_git_path(repo, &branch);
    }
    if let Some(tracked_paths) = git_output(repo, &["ls-files", "-z"]) {
        for tracked_path in tracked_paths
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            if let Some(relative_path) = path_from_git(tracked_path) {
                emit_tracked_path(repo, relative_path);
            }
        }
    }
}

fn emit_git_path(repo: &Path, git_path: &str) {
    if let Some(path) = git_text(repo, &["rev-parse", "--git-path", git_path]) {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            repo.join(path)
        };
        emit_rerun_path(&path);
    }
}

fn emit_rerun_path(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn emit_tracked_path(repo: &Path, relative_path: PathBuf) {
    let path = repo.join(relative_path);
    if matches!(
        fs::symlink_metadata(&path),
        Ok(metadata) if metadata.file_type().is_dir()
    ) {
        return;
    }
    emit_rerun_path(&path);
}

#[cfg(unix)]
fn path_from_git(path: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    Some(PathBuf::from(OsStr::from_bytes(path)))
}

#[cfg(not(unix))]
fn path_from_git(path: &[u8]) -> Option<PathBuf> {
    String::from_utf8(path.to_vec()).ok().map(PathBuf::from)
}

//! Bake git identity the way rs2b0t does: `git rev-parse HEAD` + dirty
//! from `git status --porcelain`, overridable with `GIT_COMMIT` /
//! `GITHUB_SHA` and `GIT_DIRTY`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn truthy(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn iso8601_utc() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GIT_DIRTY");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    let commit = env_nonempty("GIT_COMMIT")
        .or_else(|| env_nonempty("GITHUB_SHA"))
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let short: String = if commit == "unknown" {
        "unknown".into()
    } else {
        commit.chars().take(7).collect()
    };
    let dirty = if let Some(v) = env_nonempty("GIT_DIRTY") {
        truthy(&v)
    } else if commit == "unknown" {
        false
    } else {
        git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty())
    };
    let built_at = iso8601_utc();

    println!("cargo:rustc-env=GIT_COMMIT={commit}");
    println!("cargo:rustc-env=GIT_COMMIT_SHORT={short}");
    println!(
        "cargo:rustc-env=GIT_DIRTY={}",
        if dirty { "1" } else { "0" }
    );
    println!("cargo:rustc-env=BUILD_TIME={built_at}");
}

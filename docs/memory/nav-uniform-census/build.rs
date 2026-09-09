use sha2::Digest;
use std::{env, fs, path::Path, process::Command};

const HOST_FROZEN: &str = "c0709aba2f8b45e42193225cf8f4e7325b5ca9bf";
const CLIENT_FROZEN: &str = "3456edc8dabf7b25ada78110ffa56327af9f67a4";

fn git(path: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(["-C", path])
        .args(args)
        .output()
        .map_err(|e| format!("git {path}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {path} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

fn tracked_source(
    repo: &str,
    frozen: &str,
    prefixes: &[&str],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    for prefix in prefixes {
        let status = String::from_utf8(git(
            repo,
            &[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--",
                prefix,
            ],
        )?)
        .map_err(|e| format!("status is not utf8: {e}"))?;
        if status.lines().any(|line| !line.is_empty()) {
            return Err(format!(
                "dependency has dirty relevant source: {}",
                status.trim()
            ));
        }
    }
    let files = String::from_utf8(git(repo, &["ls-tree", "-r", "--name-only", frozen])?)
        .map_err(|e| format!("file list is not utf8: {e}"))?;
    let mut result = Vec::new();
    for file in files.lines().filter(|f| {
        prefixes.iter().any(|p| f.starts_with(p))
            && (f.ends_with(".rs")
                || f.ends_with("Cargo.toml")
                || f.ends_with("Cargo.lock")
                || f.ends_with("build.rs"))
    }) {
        let expected = git(repo, &["show", &format!("{frozen}:{file}")])?;
        let actual =
            fs::read(Path::new(repo).join(file)).map_err(|e| format!("read {file}: {e}"))?;
        if actual != expected {
            return Err(format!("frozen source byte mismatch: {file}"));
        }
        result.push((file.to_owned(), expected));
    }
    Ok(result)
}

fn digest(files: &[(String, Vec<u8>)]) -> String {
    // The manifest is a deterministic source identity, not a cryptographic gate.
    // The tool reports this value alongside the frozen commits; build.rs compares
    // every compiled Rust/Cargo input byte before producing the executable.
    let mut data = Vec::new();
    for (name, bytes) in files {
        data.extend_from_slice(name.as_bytes());
        data.push(0);
        data.extend_from_slice(bytes);
        data.push(0);
    }
    format!("{:x}", sha2::Sha256::digest(data))
}

fn main() {
    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let host = format!("{root}/../../..");
    let client = format!("{host}/vendor/fr-client-rust");
    let h = String::from_utf8(git(&host, &["rev-parse", "HEAD"]).expect("host identity"))
        .expect("host utf8")
        .trim()
        .to_owned();
    let c = String::from_utf8(git(&client, &["rev-parse", "HEAD"]).expect("client identity"))
        .expect("client utf8")
        .trim()
        .to_owned();
    let host_files = tracked_source(
        &host,
        HOST_FROZEN,
        &["Cargo.toml", "Cargo.lock", "crates/nav/", "crates/api/"],
    )
    .expect("host frozen source");
    // `client` is the workspace package at crates/client; the repository root
    // has no compiled `src/` tree.  Verify every source/build input in that
    // package, including untracked-file detection in the package directory.
    let client_files = tracked_source(
        &client,
        CLIENT_FROZEN,
        &["Cargo.toml", "Cargo.lock", "crates/client/"],
    )
    .expect("client frozen source");
    let mut all = host_files;
    all.extend(client_files);
    println!("cargo:rustc-env=NAV_CENSUS_HOST={h}");
    println!("cargo:rustc-env=NAV_CENSUS_CLIENT={c}");
    println!("cargo:rustc-env=NAV_CENSUS_FROZEN_HOST={HOST_FROZEN}");
    println!("cargo:rustc-env=NAV_CENSUS_FROZEN_CLIENT={CLIENT_FROZEN}");
    println!(
        "cargo:rustc-env=NAV_CENSUS_SOURCE_MANIFEST={}",
        digest(&all)
    );
    println!("cargo:rerun-if-changed=../../../Cargo.toml");
    println!("cargo:rerun-if-changed=../../../Cargo.lock");
    println!("cargo:rerun-if-changed=../../../crates/nav");
    println!("cargo:rerun-if-changed=../../../crates/api");
    println!("cargo:rerun-if-changed=../../../vendor/fr-client-rust/Cargo.toml");
    println!("cargo:rerun-if-changed=../../../vendor/fr-client-rust/Cargo.lock");
    println!("cargo:rerun-if-changed=../../../vendor/fr-client-rust/crates/client");
}

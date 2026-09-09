use sha2::{Digest, Sha256};
use std::{env, fs, path::Path, process::Command};

fn git(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git").args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).into_owned()); }
    Ok(out.stdout)
}
fn main() {
    // Build provenance is emitted, but a build never silently qualifies a moving checkout.
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let host = git(&["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned()).unwrap_or_else(|e| format!("unavailable:{e}"));
    let lock = fs::read(root.join("Cargo.lock")).unwrap_or_default();
    let lock_hash = format!("{:x}", Sha256::digest(&lock));
    println!("cargo:rustc-env=STAGE_A_HOST_HEAD={host}");
    println!("cargo:rustc-env=STAGE_A_LOCK_SHA256={lock_hash}");
    println!("cargo:rerun-if-changed=../../../Cargo.lock");
}

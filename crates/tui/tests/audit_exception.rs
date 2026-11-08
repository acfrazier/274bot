//! Guard for the `RUSTSEC-2026-0009` ignore in `.cargo/audit.toml`.
//!
//! `time` enters the host graph only through this crate's `ratatui`
//! dependency (`ratatui-widgets` 0.3.0, the calendar widget). The advisory
//! covers code that only the `time` features `parsing` and `formatting`
//! compile, and `cargo audit` reads the lockfile without seeing features, so
//! the ignore would stay green however a dependent crate is configured. This
//! test reads the resolved feature set from `cargo metadata` and fails when
//! either feature is selected, which keeps a later reachable use from hiding
//! behind the exception.
//!
//! It runs in the workspace test lane (`cargo test --workspace`) and in every
//! `tui` test invocation; `.cargo/audit.toml` names it as the guard.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `time` features that compile the code `RUSTSEC-2026-0009` describes.
const ADVISORY_FEATURES: [&str; 2] = ["parsing", "formatting"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the tui crate sits under the workspace crates directory")
        .to_path_buf()
}

/// The resolved feature set of the locked `time` package.
fn resolved_time_features(root: &Path) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .current_dir(root)
        .args([
            "metadata",
            // Resolve every workspace feature so a feature-gated dependent
            // cannot select an advisory feature and stay invisible.
            "--all-features",
            // The committed lockfile is the reviewed artifact; never rewrite it.
            "--locked",
            "--format-version",
            "1",
        ])
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata writes JSON");

    let time = metadata["packages"]
        .as_array()
        .expect("metadata lists packages")
        .iter()
        .filter(|package| package["name"] == "time")
        .map(|package| package["id"].as_str().expect("each package has an id"))
        .collect::<Vec<_>>();
    assert_eq!(
        time.len(),
        1,
        "expected one locked `time` package, found {time:?}: re-check the \
         RUSTSEC-2026-0009 ignore in .cargo/audit.toml against this graph"
    );
    let node = metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata resolves the graph")
        .iter()
        .find(|node| node["id"].as_str() == Some(time[0]))
        .expect("the resolved graph contains the locked `time` package");
    node["features"]
        .as_array()
        .expect("each resolved node lists features")
        .iter()
        .map(|feature| {
            feature
                .as_str()
                .expect("feature names are strings")
                .to_owned()
        })
        .collect()
}

#[test]
fn time_selects_neither_parsing_nor_formatting() {
    let features = resolved_time_features(&workspace_root());
    let reachable = ADVISORY_FEATURES
        .into_iter()
        .filter(|advisory| features.iter().any(|enabled| enabled == advisory))
        .collect::<Vec<_>>();
    assert!(
        reachable.is_empty(),
        "`time` selected {reachable:?}, whose code RUSTSEC-2026-0009 covers \
         (resolved features: {features:?}). Drop the selection or remove the \
         ignore from .cargo/audit.toml."
    );
}

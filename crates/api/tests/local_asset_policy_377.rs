//! Revision-377 defaults are optional on every host. Explicit inputs are required.

use std::path::Path;
use std::process::{Command, Output};

const PROBE: &str = "API_LOCAL_ASSET_377_POLICY_PROBE";
const ASSET: &str = "API_LOCAL_ASSET_377_POLICY_PATH";
const PRESENT: &str = "API_LOCAL_ASSET_377_POLICY_PRESENT";

#[test]
fn asset_377_probe() {
    let Some(default) = std::env::var_os(PROBE) else {
        return;
    };
    let actual = api::test_support::local_asset_path_377(
        ASSET,
        default.to_str().expect("test path is UTF-8"),
        "377 policy regression asset",
    );
    if std::env::var_os(PRESENT).is_some() {
        assert_eq!(actual.as_deref(), Some(Path::new(&default)));
    } else {
        assert!(actual.is_none());
    }
}

fn probe(default: &Path, configured: Option<&Path>, present: bool, ci: bool) -> Output {
    let mut child = Command::new(std::env::current_exe().expect("test executable"));
    child
        .args(["--exact", "asset_377_probe", "--nocapture"])
        .env(PROBE, default)
        .env_remove(ASSET)
        .env_remove(PRESENT)
        .env_remove("GITHUB_ACTIONS");
    if let Some(path) = configured {
        child.env(ASSET, path);
    }
    if present {
        child.env(PRESENT, "true");
    }
    if ci {
        child.env("GITHUB_ACTIONS", "true");
    }
    child.output().expect("377 asset-policy child starts")
}

#[test]
fn absent_377_default_skips_on_non_ci_and_ci_hosts() {
    let missing = std::env::temp_dir().join(format!("377-default-missing-{}", std::process::id()));
    assert!(!missing.exists());
    for ci in [false, true] {
        let output = probe(&missing, None, false, ci);
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("Skipping 377 policy regression asset"));
    }
}

#[test]
fn explicit_missing_or_invalid_377_path_fails_on_non_ci_and_ci_hosts() {
    let missing = std::env::temp_dir().join(format!("377-explicit-missing-{}", std::process::id()));
    assert!(!missing.exists());
    for ci in [false, true] {
        for configured in [&missing, std::env::temp_dir().as_path(), Path::new("")] {
            let output = probe(&missing, Some(configured), false, ci);
            assert!(!output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("must name an existing file"));
        }
    }
}

#[test]
fn present_377_default_and_explicit_paths_resolve_on_non_ci_and_ci_hosts() {
    let path = std::env::temp_dir().join(format!("377-present-{}", std::process::id()));
    assert!(!path.exists());
    std::fs::write(&path, b"policy fixture").unwrap();
    for ci in [false, true] {
        for configured in [None, Some(path.as_path())] {
            let output = probe(&path, configured, true, ci);
            assert!(output.status.success(), "{output:?}");
        }
    }
    std::fs::remove_file(path).unwrap();
}

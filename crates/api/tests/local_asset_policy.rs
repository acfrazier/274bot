//! Missing local assets skip only in the hosted GitHub Actions environment.

use std::process::Command;

const PROBE: &str = "API_LOCAL_ASSET_POLICY_PROBE";

#[test]
fn missing_asset_probe() {
    let Some(path) = std::env::var_os(PROBE) else {
        return;
    };
    assert!(api::test_support::local_asset_path(
        "",
        path.to_str().expect("test path is UTF-8"),
        "policy regression asset",
    )
    .is_none());
}

#[test]
fn only_explicit_hosted_ci_can_skip_missing_local_assets() {
    let missing = std::env::temp_dir().join(format!(
        "274bot-missing-asset-policy-{}",
        std::process::id()
    ));
    assert!(!missing.exists());
    for github_actions in [None, Some("false"), Some("1"), Some("true")] {
        let mut child = Command::new(std::env::current_exe().expect("test executable"));
        child
            .args(["--exact", "missing_asset_probe", "--nocapture"])
            .env(PROBE, &missing)
            .env_remove("GITHUB_ACTIONS");
        if let Some(value) = github_actions {
            child.env("GITHUB_ACTIONS", value);
        }
        let output = child.output().expect("asset-policy child starts");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if github_actions == Some("true") {
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(stderr.contains("Skipping policy regression asset"));
            assert!(stderr.contains("GITHUB_ACTIONS=true"));
        } else {
            assert!(!output.status.success(), "{stdout}\n{stderr}");
            assert!(stderr.contains("missing policy regression asset"));
            assert!(stderr.contains("python tools/local-data/build.py"));
        }
    }
}

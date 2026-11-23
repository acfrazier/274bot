//! `api::hostlog::stderr_drain` drains the stderr queue on every way out of
//! `main`, and every binary that logs through the host log holds it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CHILD: &str = "T1_STDERR_DRAIN_CHILD";
const LINE: &str = "line logged right before exit";

/// The child of the process tests: log one always-stderr line and leave the
/// way `kind` names, with the guard held for the whole of the function.
fn child(kind: &str) {
    let _drain = api::hostlog::stderr_drain();
    api::host_log!(
        stderr;
        api::hostlog::Category::Lifecycle,
        api::hostlog::Level::Error,
        "{LINE}"
    );
    match kind {
        // An early `return ExitCode::FAILURE` of main.
        "return" => {}
        // A panic that unwinds out of main.
        "panic" => panic!("child panic"),
        other => unreachable!("{other}"),
    }
}

fn run_child(kind: &str, test: &str) -> (bool, String) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, kind)
        .stdin(Stdio::null())
        .output()
        .expect("start the child");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_line_logged_before_an_early_return_reaches_stderr() {
    if let Ok(kind) = std::env::var(CHILD) {
        child(&kind);
        return;
    }
    let (_, stderr) = run_child(
        "return",
        "a_line_logged_before_an_early_return_reaches_stderr",
    );
    assert!(stderr.contains(LINE), "stderr was: {stderr}");
}

#[test]
fn a_line_logged_before_a_panic_reaches_stderr() {
    if let Ok(kind) = std::env::var(CHILD) {
        // The panic unwinds out of `child`, which drops the guard; the test
        // harness then reports the panic as a failed test in the child.
        child(&kind);
        return;
    }
    let (_, stderr) = run_child("panic", "a_line_logged_before_a_panic_reaches_stderr");
    assert!(stderr.contains(LINE), "stderr was: {stderr}");
}

/// Rust sources of every binary target of the workspace: `src/main.rs`,
/// `src/bin/*.rs` and `examples/*.rs` of each crate.
fn binary_sources() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        let dir = entry.path();
        let main = dir.join("src/main.rs");
        if main.is_file() {
            found.push(main);
        }
        for sub in ["src/bin", "examples"] {
            if let Ok(files) = std::fs::read_dir(dir.join(sub)) {
                found.extend(
                    files
                        .flatten()
                        .map(|f| f.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "rs")),
                );
            }
        }
    }
    found
}

#[test]
fn every_binary_that_logs_through_the_host_log_holds_the_drain_guard() {
    let sources = binary_sources();
    for named in ["host-play", "panel", "tui"] {
        assert!(
            sources
                .iter()
                .any(|p| p.ends_with(format!("{named}/src/main.rs"))),
            "{named}/src/main.rs is not among the scanned binaries"
        );
    }
    let mut missing = Vec::new();
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        let logs = text.contains("host_log!") || text.contains("hostlog::");
        if logs && !text.contains("stderr_drain()") {
            missing.push(path.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "binaries that log through hostlog without `let _drain = api::hostlog::stderr_drain();`: {missing:?}"
    );
}

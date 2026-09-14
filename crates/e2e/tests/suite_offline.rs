//! Offline verification of the native suite entrypoint: the real `e2e-suite` binary
//! driving real child processes.
//!
//! No engine, no client, no login and no gameplay: wherever a real run would launch a
//! native panel executable, these tests hand the suite the disposable
//! `e2e-suite-fixture` binary through `--exec-core`. `LIVE` is never read here; the tests
//! run in a plain `cargo test -p e2e`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use e2e::suite::child::{self, ChildSpec};
use e2e::suite::cli::{EXIT_FAILURE, EXIT_OK, EXIT_USAGE};
use e2e::suite::ledger::{AttemptStatus, Ledger};

const SUITE: &str = env!("CARGO_BIN_EXE_e2e-suite");
const FIXTURE: &str = env!("CARGO_BIN_EXE_e2e-suite-fixture");

/// `interrupt_flag` is process-wide, so the two tests that touch it are serialized.
static SIGNAL_GUARD: Mutex<()> = Mutex::new(());

fn fixture_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/native-suite/offline-suite-manifest.json")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-suite-offline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn suite(manifest: &Path, run_dir: &Path, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let tmp = run_dir.parent().unwrap();
    let mut command = Command::new(SUITE);
    command
        .arg("run")
        .arg("--manifest")
        .arg(manifest)
        .arg("--catalog")
        .arg(tmp)
        .arg("--profile")
        .arg("offline-fixture")
        .arg("--exec-core")
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(run_dir)
        .env("E2E_SUITE_FIXTURE_LOG", tmp.join("launches.log"));
    for (key, value) in env {
        command.env(key, value);
    }
    command.args(extra);
    command.output().expect("the suite binary runs")
}

fn launches(tmp: &Path) -> Vec<String> {
    std::fs::read_to_string(tmp.join("launches.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `list`/`dry-run` must print the real command line and leave the run directory alone.
#[test]
fn dry_run_prints_the_real_native_command_and_never_touches_a_run_directory() {
    let tmp = temp_dir("dry-run");
    let run_dir = tmp.join("run");
    let out = Command::new(SUITE)
        .args(["dry-run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--level", "full", "--catalog"])
        .arg(&tmp)
        .args(["--profile", "offline-fixture", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        stdout.contains(&format!(
            "{} --profile offline-fixture --catalog {} --live script_thiever",
            FIXTURE,
            tmp.display()
        )),
        "the printed command must be exactly what a child would receive:\n{stdout}"
    );
    assert!(stdout.contains("fixture_one"), "{stdout}");
    // The reference row stays visible with its reason instead of being substituted.
    assert!(stdout.contains("excluded_script"), "{stdout}");
    assert!(!run_dir.exists(), "dry-run must not create a run directory");
}

/// A child that exits 0 without a terminal receipt is a shared harness failure: the run
/// stops, the remaining cases are not launched, and the ledger says why.
#[test]
fn a_zero_exit_child_without_a_terminal_receipt_stops_the_run() {
    let tmp = temp_dir("no-receipt");
    let run_dir = tmp.join("run");
    let out = suite(
        &fixture_manifest(),
        &run_dir,
        &[
            "--only",
            "fixture_one,fixture_two",
            "--child-arg",
            "--mode",
            "--child-arg",
            "zero-no-receipt",
        ],
        &[],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_FAILURE),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger.attempt("fixture_one").expect("attempt recorded");
    assert_eq!(attempt.status, AttemptStatus::SharedFailure);
    assert!(
        attempt.reason.contains("no terminal scenario receipt"),
        "{}",
        attempt.reason
    );
    assert_eq!(attempt.exit_code, Some(0));
    assert!(
        ledger.attempt("fixture_two").is_none(),
        "a shared failure must stop the run instead of launching the next case"
    );
    assert!(
        ledger.state.stop_reason.is_some(),
        "the stop reason is durable"
    );
    assert_eq!(launches(&tmp).len(), 1, "only the first case was launched");
}

/// A case-local assertion failure is recorded, the run continues while the shared harness
/// is healthy, and the exit code is still nonzero.
#[test]
fn an_isolated_assertion_failure_is_recorded_and_the_run_continues() {
    let tmp = temp_dir("assert-fail");
    let run_dir = tmp.join("run");
    let out = suite(
        &fixture_manifest(),
        &run_dir,
        &[
            "--only",
            "fixture_one,fixture_two",
            "--child-arg",
            "--mode",
            "--child-arg",
            "fail",
        ],
        &[],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_FAILURE),
        "{}",
        text(&out.stdout)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    for id in ["fixture_one", "fixture_two"] {
        let attempt = ledger.attempt(id).expect("both cases were attempted");
        assert_eq!(attempt.status, AttemptStatus::Failed, "{id}");
        assert!(
            attempt
                .reason
                .contains("assertion failed: no action observed"),
            "{id}: {}",
            attempt.reason
        );
    }
    assert!(
        ledger.state.stop_reason.is_none(),
        "a case assertion failure does not stop the shared harness"
    );
    assert_eq!(launches(&tmp).len(), 2);
}

/// A recorded result is durable: resume carries it, never relaunches it, and refuses a
/// changed request (settings or manifest) before any launch.
#[test]
fn a_passing_case_is_retained_and_resume_never_relaunches_it() {
    let tmp = temp_dir("resume");
    let run_dir = tmp.join("run");
    let witness = [("E2E_SUITE_FIXTURE_CORE", "Thiever")];
    let only = ["--only", "fixture_one"];

    let first = suite(&fixture_manifest(), &run_dir, &only, &witness);
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    assert_eq!(
        ledger.attempt("fixture_one").unwrap().status,
        AttemptStatus::Passed
    );
    assert_eq!(launches(&tmp).len(), 1);

    let resumed = suite(
        &fixture_manifest(),
        &run_dir,
        &["--only", "fixture_one", "--resume"],
        &witness,
    );
    assert_eq!(
        resumed.status.code(),
        Some(EXIT_OK),
        "{}",
        text(&resumed.stderr)
    );
    assert_eq!(
        launches(&tmp).len(),
        1,
        "resume must not relaunch a recorded case"
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    assert_eq!(
        ledger.attempt("fixture_one").unwrap().attempt,
        1,
        "no silent retry, no new attempt number"
    );
    let summary = ledger.state.summary.clone().expect("summary recorded");
    assert_eq!(summary.carried_success, 1);
    assert_eq!(summary.attempted, 0);

    // A changed settings request refuses before any launch.
    let changed = suite(
        &fixture_manifest(),
        &run_dir,
        &["--only", "fixture_one", "--resume", "--child-arg", "--mode"],
        &witness,
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("settings"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1, "a refused resume launches nothing");

    // A changed manifest binds different bytes, so it refuses too.
    let other = tmp.join("other-manifest.json");
    std::fs::write(
        &other,
        std::fs::read_to_string(fixture_manifest())
            .unwrap()
            .replace("offline-suite-fixture", "offline-suite-fixture-b"),
    )
    .unwrap();
    let changed = suite(
        &other,
        &run_dir,
        &["--only", "fixture_one", "--resume"],
        &witness,
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("manifest_sha256"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// The suite owns the process tree it launches: a child that ignores the graceful signal
/// is force-killed and reaped inside the bounded cleanup.
#[test]
fn a_child_over_budget_is_killed_with_its_process_tree() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("timeout");
    let spec = ChildSpec {
        label: "fixture-timeout".into(),
        command: vec![
            FIXTURE.to_string(),
            "--mode".into(),
            "hang".into(),
            "--ignore-sigterm".into(),
        ],
        env: Default::default(),
        cwd: None,
    };
    let budget = Duration::from_secs(2);
    let started = Instant::now();
    let run = child::run(&spec, budget, &tmp.join("child.log"), false).unwrap();
    let elapsed = started.elapsed();
    assert!(run.timed_out, "{run:?}");
    assert!(!run.interrupted, "{run:?}");
    assert!(
        run.cleanup.escalated_to_sigkill,
        "a child that ignores SIGTERM must be force-killed: {:?}",
        run.cleanup
    );
    assert!(
        run.cleanup.reaped,
        "cleanup must reap the tree: {:?}",
        run.cleanup
    );
    assert!(elapsed >= budget, "the budget was respected: {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(20),
        "cleanup stays bounded: {elapsed:?}"
    );
    assert_eq!(run.exit_code, None, "{run:?}");
    assert_eq!(run.signal, Some(9), "{run:?}");
}

/// An interrupt reaps the owned tree through the same cleanup path instead of leaking it.
#[test]
fn an_interrupted_child_is_reaped_before_run_returns() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("interrupt");
    let spec = ChildSpec {
        label: "fixture-interrupt".into(),
        command: vec![FIXTURE.to_string(), "--mode".into(), "hang".into()],
        env: Default::default(),
        cwd: None,
    };
    child::interrupt_flag().store(true, Ordering::SeqCst);
    let started = Instant::now();
    let run = child::run(
        &spec,
        Duration::from_secs(60),
        &tmp.join("child.log"),
        false,
    )
    .unwrap();
    child::interrupt_flag().store(false, Ordering::SeqCst);
    assert!(run.interrupted, "{run:?}");
    assert!(!run.timed_out, "{run:?}");
    assert!(run.cleanup.reaped, "{:?}", run.cleanup);
    assert!(run.cleanup.killed_signal.is_some(), "{:?}", run.cleanup);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "an interrupt must not wait for the budget"
    );
}

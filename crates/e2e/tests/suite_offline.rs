//! Offline verification of the native suite entrypoint: the real `e2e-suite` binary
//! driving real child processes.
//!
//! No engine, no client, no login and no gameplay: wherever a real run would launch a
//! native panel executable, these tests hand the suite the disposable
//! `e2e-suite-fixture` binary through `--exec-core`. `LIVE` is never read here; the tests
//! run in a plain `cargo test -p e2e`.
//!
//! The fixture speaks the panel's real line contract — outer `PASS: live script_thiever`,
//! inner evidence `"scenario":"thiever"`, witness `"case":"thiever"` — and writes the
//! terminal shot the scenario declares, so a successful case is retained as
//! `pending_visual_review` until a human reads the capture back.

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

/// A disposable working root with the catalog the suite binds. A real run points
/// `--catalog` at the `$RS2B0T` clone; the suite binds its `src/bot/scripts` content, so a
/// catalog without that tree is refused rather than recorded as an unresolved identity.
fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-suite-offline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("catalog/src/bot/scripts/Thiever")).unwrap();
    std::fs::write(
        dir.join("catalog/src/bot/scripts/index.ts"),
        "// fixture registry\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("catalog/src/bot/scripts/Thiever/index.ts"),
        "// fixture script\n",
    )
    .unwrap();
    dir
}

fn catalog(tmp: &Path) -> PathBuf {
    tmp.join("catalog")
}

fn suite(tmp: &Path, run_dir: &Path, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(SUITE);
    command
        .arg("run")
        .arg("--manifest")
        .arg(fixture_manifest())
        .arg("--catalog")
        .arg(catalog(tmp))
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
        .arg(catalog(&tmp))
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
            catalog(&tmp).display()
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
        &tmp,
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
        &tmp,
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

/// A contracted terminal shot that the case did not write is a shared failure: the panel
/// holds a PASS until the shot lands, so a PASS without it is not a pass. A capture under
/// an unrelated label is not this case's evidence either.
#[test]
fn a_contracted_capture_that_never_arrives_stops_the_run() {
    for (mode, needle) in [
        ("no-shot", "wrote no capture at all"),
        ("wrong-label", "unrelated_shot"),
    ] {
        let tmp = temp_dir(&format!("capture-{mode}"));
        let run_dir = tmp.join("run");
        let out = suite(
            &tmp,
            &run_dir,
            &[
                "--only",
                "fixture_one",
                "--child-arg",
                "--mode",
                "--child-arg",
                mode,
            ],
            &[],
        );
        assert_eq!(
            out.status.code(),
            Some(EXIT_FAILURE),
            "{mode}\n{}",
            text(&out.stdout)
        );
        let ledger = Ledger::resume(&run_dir).unwrap();
        let attempt = ledger.attempt("fixture_one").expect("attempt recorded");
        assert_eq!(attempt.status, AttemptStatus::SharedFailure, "{mode}");
        assert_eq!(
            attempt
                .receipts
                .as_ref()
                .and_then(|r| r.catalog_core.clone()),
            Some(
                "CATALOG_CORE: script_thiever {\"case\":\"thiever\",\"post_start_observations\":2}"
                    .into()
            ),
            "{mode}: the real panel line contract was parsed"
        );
        match mode {
            "no-shot" => assert!(attempt.reason.contains(needle), "{}", attempt.reason),
            _ => assert!(
                attempt.reason.contains("contracted capture")
                    && attempt.reason.contains("thiever paint"),
                "{}",
                attempt.reason
            ),
        }
    }
}

/// A recorded result is durable: resume carries it, never relaunches it, and refuses a
/// changed request — settings, manifest, or the *content* of an input at the same path —
/// before any launch.
#[test]
fn a_pending_visual_result_is_retained_and_resume_refuses_changed_inputs() {
    let tmp = temp_dir("resume");
    let run_dir = tmp.join("run");
    let vault = tmp.join("vault");
    std::fs::write(&vault, "vault content v1").unwrap();
    let vault_arg = vault.display().to_string();
    let base = ["--only", "fixture_one", "--vault", &vault_arg];

    let first = suite(&tmp, &run_dir, &base, &[]);
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger
        .attempt("fixture_one")
        .expect("attempt recorded")
        .clone();
    assert_eq!(
        attempt.status,
        AttemptStatus::PendingVisualReview,
        "a headed functional result stays pending visual review: {}",
        attempt.reason
    );
    assert_eq!(attempt.captures.len(), 1, "{:?}", attempt.captures);
    let capture = &attempt.captures[0];
    assert!(capture.complete(), "{:?}", capture.structural_reason());
    assert_eq!(capture.sidecar_ingame, Some(true));
    assert_eq!(capture.sidecar_scene_state, Some(2));
    assert_eq!(launches(&tmp).len(), 1);

    let resumed = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--vault", &vault_arg, "--resume"],
        &[],
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
    assert_eq!(ledger.attempt("fixture_one").unwrap().attempt, 1);
    let summary = ledger.state.summary.clone().expect("summary recorded");
    assert_eq!(summary.carried_success, 1);
    assert_eq!(summary.attempted, 0);

    // A changed settings request refuses before any launch.
    let changed = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one",
            "--vault",
            &vault_arg,
            "--resume",
            "--child-arg",
            "--mode",
        ],
        &[],
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
    let mut changed = Command::new(SUITE);
    changed
        .arg("run")
        .arg("--manifest")
        .arg(&other)
        .arg("--catalog")
        .arg(catalog(&tmp))
        .args(["--profile", "offline-fixture", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .arg("--only")
        .arg("fixture_one")
        .arg("--vault")
        .arg(&vault)
        .arg("--resume")
        .env("E2E_SUITE_FIXTURE_LOG", tmp.join("launches.log"));
    let changed = changed.output().unwrap();
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("manifest_sha256"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1);

    // The same path with changed *content* refuses: the vault is bound by digest.
    std::fs::write(&vault, "vault content v2").unwrap();
    let changed = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--vault", &vault_arg, "--resume"],
        &[],
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("profile/input configuration"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1, "a refused resume launches nothing");

    // The same path with changed *script source* refuses as well.
    std::fs::write(
        catalog(&tmp).join("src/bot/scripts/Thiever/index.ts"),
        "// fixture script (edited)\n",
    )
    .unwrap();
    let changed = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--vault", &vault_arg, "--resume"],
        &[],
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("profile/input configuration"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// An input the suite cannot bind is refused before the ledger exists: a run that could
/// not read an input could not detect that it changed.
#[test]
fn a_run_refuses_when_it_cannot_bind_its_inputs() {
    // A catalog without the script tree.
    let tmp = temp_dir("unbindable-catalog");
    let empty = tmp.join("empty-catalog");
    std::fs::create_dir_all(&empty).unwrap();
    let run_dir = tmp.join("run");
    let out = Command::new(SUITE)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(&empty)
        .args(["--profile", "offline-fixture", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("refusing to run") && stderr.contains("catalog script content"),
        "{stderr}"
    );
    assert!(!run_dir.exists(), "nothing was created");

    // An executable the suite cannot resolve and hash (the manifest's cargo template with
    // no explicit executable) is refused too.
    let tmp = temp_dir("unbindable-exec");
    let run_dir = tmp.join("run");
    let out = Command::new(SUITE)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "offline-fixture", "--run-dir"])
        .arg(&run_dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("--exec-core"), "{stderr}");
    assert!(!run_dir.exists(), "nothing was created");
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

/// A log that cannot be opened refuses before any spawn: the suite must not launch a child
/// it would then have to abandon.
#[test]
fn a_log_that_cannot_be_opened_refuses_before_any_spawn() {
    let tmp = temp_dir("log-open");
    let blocker = tmp.join("not-a-directory");
    std::fs::write(&blocker, "regular file").unwrap();
    let launches = tmp.join("launches.log");
    let spec = ChildSpec {
        label: "fixture-log".into(),
        command: vec![FIXTURE.to_string(), "--mode".into(), "pass".into()],
        env: [(
            "E2E_SUITE_FIXTURE_LOG".to_string(),
            launches.display().to_string(),
        )]
        .into_iter()
        .collect(),
        cwd: None,
    };
    let error = child::run(
        &spec,
        Duration::from_secs(30),
        &blocker.join("child.log"),
        false,
    )
    .unwrap_err();
    assert!(error.contains("log"), "{error}");
    assert!(
        !launches.exists(),
        "no child may be launched when the log cannot be opened"
    );
}

/// A descendant that inherits the pipes cannot hang the suite: the drain is bounded and
/// the leftover process group is force-killed to close the write ends.
#[test]
fn a_descendant_holding_the_pipes_cannot_hang_the_suite() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("pipe-hold");
    let spec = ChildSpec {
        label: "pipe-holder".into(),
        // The direct child exits 0 immediately; the backgrounded `sleep` inherits stdout
        // and stderr and would hold the pipes open indefinitely.
        command: vec!["/bin/sh".into(), "-c".into(), "sleep 300 & exit 0".into()],
        env: Default::default(),
        cwd: None,
    };
    let started = Instant::now();
    let run = child::run(
        &spec,
        Duration::from_secs(30),
        &tmp.join("child.log"),
        false,
    )
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(run.exit_code, Some(0), "{run:?}");
    assert!(
        elapsed >= child::PIPE_DRAIN,
        "the drain waits for the inherited pipes: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "the drain is bounded, not an unbounded join: {elapsed:?}"
    );
    assert!(
        run.cleanup.note.contains("output pipes were still open"),
        "{:?}",
        run.cleanup
    );
}

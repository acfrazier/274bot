//! Offline verification of the native suite entrypoint: the real `e2e-suite` binary
//! driving real child processes.
//!
//! The tests are portable on purpose. The runner owns a child tree on unix (process groups
//! and signals) and on Windows (job objects), so the real-process tests run on both and
//! only genuinely platform-only actions are gated: the fixture's `escape-pipe` mode (a
//! process can leave a unix process group, while a job grants no breakaway) and a symlinked
//! vault. Assertions that name a signal (or an exit code only unix produces) are cfg-split
//! rather than dropped.
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

/// `interrupt_flag` is process-wide, so every test that drives `child::run` in this process
/// holds the guard: an interrupt flag set by one test would otherwise be seen by another
/// test's wait loop.
static SIGNAL_GUARD: Mutex<()> = Mutex::new(());

fn fixture_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/native-suite/offline-suite-manifest.json")
}

/// A disposable working root with the catalog the suite binds and a disposable `$HOME` for
/// the native profile resolver. A real run points `--catalog` at the `$RS2B0T` clone and
/// resolves its profile against the operator's home; the suite binds the catalog's
/// `src/bot/scripts` content and the *resolved* profile paths, so a catalog without that
/// tree — or a profile the native resolver does not know — is refused rather than recorded
/// as an unresolved identity.
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
    std::fs::create_dir_all(home(&dir)).unwrap();
    dir
}

/// The disposable `$HOME` the suite resolves the native profile against. The profile the
/// offline tests select is a real one (`local-289`); only its resolved paths are disposable.
fn home(tmp: &Path) -> PathBuf {
    tmp.join("home")
}

/// The vault the `local-289` profile resolves by default (`~/.274bot/vault-289`), which the
/// suite must bind — not the process-wide default vault.
fn default_vault(tmp: &Path) -> PathBuf {
    home(tmp).join(".274bot").join("vault-289")
}

fn catalog(tmp: &Path) -> PathBuf {
    tmp.join("catalog")
}

fn suite(tmp: &Path, run_dir: &Path, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = suite_command(tmp);
    command
        .arg("run")
        .arg("--manifest")
        .arg(fixture_manifest())
        .arg("--catalog")
        .arg(catalog(tmp))
        .args(["--profile", "local-289"])
        .arg("--exec-core")
        .arg(FIXTURE)
        // The loader smoke's own runner: the same disposable child stands in for `external_watch`.
        .arg("--exec-external")
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

/// The suite binary with the disposable profile home in the environment: `HOME`, and the
/// `USERPROFILE` fallback the native resolver uses on Windows.
fn suite_command(tmp: impl AsRef<Path>) -> Command {
    let home = home(tmp.as_ref());
    let mut command = Command::new(SUITE);
    command.env("HOME", &home).env("USERPROFILE", &home);
    command
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

fn canonical(path: impl AsRef<Path>) -> String {
    std::fs::canonicalize(path.as_ref())
        .unwrap()
        .display()
        .to_string()
}

/// A child that leaves a descendant behind, plus the file the descendant records its pid in.
/// The mode decides what the descendant does: `descendant` gives it its own stdio,
/// `descendant-pipe` lets it inherit the suite's pipes, `descendant-ignore-stop` also makes
/// it ignore the graceful stop.
fn descendant_spec(tmp: &Path, mode: &str) -> (ChildSpec, PathBuf) {
    let pid_file = tmp.join("descendant.pid");
    let spec = ChildSpec {
        label: format!("fixture-{mode}"),
        command: vec![FIXTURE.to_string(), "--mode".into(), mode.to_string()],
        env: [(
            "E2E_SUITE_DESCENDANT_PID".to_string(),
            pid_file.display().to_string(),
        )]
        .into_iter()
        .collect(),
        cwd: None,
    };
    (spec, pid_file)
}

/// Whether a pid is still running. The probe belongs to the test, and is platform-native:
/// signal 0 on unix, `OpenProcess`/`GetExitCodeProcess` on Windows.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

fn recorded_pid(pid_file: &Path) -> u32 {
    std::fs::read_to_string(pid_file)
        .expect("the descendant recorded its pid")
        .trim()
        .parse()
        .expect("a pid")
}

/// `list`/`dry-run` must print the real command line and leave the run directory alone.
#[test]
fn dry_run_prints_the_real_native_command_and_never_touches_a_run_directory() {
    let tmp = temp_dir("dry-run");
    let run_dir = tmp.join("run");
    let out = suite_command(&tmp)
        .args(["dry-run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--level", "full", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .arg("--exec-external")
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .env("HOME", home(&tmp))
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        stdout.contains(&format!(
            "{} --profile local-289 --catalog {} --lowmem --nav-paints on --live script_thiever",
            canonical(FIXTURE),
            catalog(&tmp).display()
        )),
        "the printed command must be exactly what a child would receive:\n{stdout}"
    );
    assert!(stdout.contains("fixture_one"), "{stdout}");
    // The reference row stays visible with its reason instead of being substituted.
    assert!(stdout.contains("excluded_script"), "{stdout}");
    assert!(!run_dir.exists(), "dry-run must not create a run directory");
}

/// Without `--exec-*` the suite still launches a *resolved, hashed* executable: the
/// artifact a build produced, never the manifest's `cargo run` template — which could
/// rebuild different bytes under the same command after the identity was captured.
///
/// The template resolves the native artifact suffix on each supported platform.
#[test]
fn a_run_without_an_explicit_executable_launches_the_resolved_artifact() {
    let tmp = temp_dir("resolved-exec");
    // A controlled target directory holding exactly one built artifact: the fixture,
    // standing in for `target/{debug,release}/<bin>` a real `cargo build` would produce.
    let target = tmp.join("target");
    let resolved = target
        .join("debug")
        .join(format!("e2e-suite-fixture{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(resolved.parent().unwrap()).unwrap();
    std::fs::copy(FIXTURE, &resolved).unwrap();

    // The manifest's cargo template names the artifact; it is never a launch program.
    let manifest = tmp.join("cargo-template-manifest.json");
    std::fs::write(
        &manifest,
        std::fs::read_to_string(fixture_manifest())
            .unwrap()
            .replace("\"panel-play\"", "\"cargo\"")
            .replace(
                "\"--live\"",
                "\"run\", \"-p\", \"e2e\", \"--bin\", \"e2e-suite-fixture\"",
            ),
    )
    .unwrap();

    let run_dir = tmp.join("run");
    let mut command = suite_command(&tmp);
    let out = command
        .args(["run", "--manifest"])
        .arg(&manifest)
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--run-dir"])
        .arg(&run_dir)
        .env("HOME", home(&tmp))
        .env("CARGO_TARGET_DIR", &target)
        .env("E2E_SUITE_FIXTURE_LOG", tmp.join("launches.log"))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(
        launches(&tmp).len(),
        1,
        "the resolved artifact was actually launched"
    );

    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger.attempt("fixture_one").expect("attempt recorded");
    let resolved_canonical = canonical(&resolved);
    assert_eq!(
        attempt.command.first().map(String::as_str),
        Some(resolved_canonical.as_str()),
        "argv[0] is the resolved artifact: {:?}",
        attempt.command
    );
    assert!(
        !attempt
            .command
            .iter()
            .any(|arg| arg == "cargo" || arg == "run"),
        "the cargo template never reaches argv: {:?}",
        attempt.command
    );
    let binary = &ledger.state.identity["binaries"]["core"];
    assert_eq!(
        binary["program"],
        serde_json::json!(resolved_canonical),
        "the ledger identity binds the executable that was launched"
    );
    assert_eq!(binary["kind"], serde_json::json!("resolved-cargo"));
    assert!(
        binary["sha256"].is_string(),
        "the resolved artifact is content-bound: {binary}"
    );

    // dry-run prints the same line the child receives, not the cargo template.
    let dry = suite_command(&tmp)
        .args(["dry-run", "--manifest"])
        .arg(&manifest)
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289"])
        .env("HOME", home(&tmp))
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    let stdout = text(&dry.stdout);
    assert!(dry.status.success(), "{}", text(&dry.stderr));
    assert!(
        stdout.contains(&format!(
            "{} --profile local-289 --catalog {} --lowmem --nav-paints on --live script_thiever",
            resolved_canonical,
            catalog(&tmp).display()
        )),
        "{stdout}"
    );
    assert!(!stdout.contains("cargo"), "{stdout}");

    // An executable the run cannot resolve and hash refuses before the ledger exists.
    let empty_target = tmp.join("empty-target");
    std::fs::create_dir_all(&empty_target).unwrap();
    let refused_run = tmp.join("refused-run");
    let refused = suite_command(&tmp)
        .arg("run")
        .arg("--manifest")
        .arg(&manifest)
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--run-dir"])
        .arg(&refused_run)
        .env("HOME", home(&tmp))
        .env("CARGO_TARGET_DIR", &empty_target)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(EXIT_USAGE));
    let stderr = text(&refused.stderr);
    assert!(
        stderr.contains("no built e2e-suite-fixture executable") && stderr.contains("--exec-core"),
        "{stderr}"
    );
    assert!(
        !refused_run.exists(),
        "an unresolved executable refuses before the run directory"
    );
    assert_eq!(launches(&tmp).len(), 1, "nothing else was launched");
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
    let mut changed = suite_command(&tmp);
    changed
        .arg("run")
        .arg("--manifest")
        .arg(&other)
        .arg("--catalog")
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .arg("--only")
        .arg("fixture_one")
        .arg("--vault")
        .arg(&vault)
        .arg("--resume")
        .env("HOME", home(&tmp))
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
    let out = suite_command(&tmp)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(&empty)
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .env("HOME", home(&tmp))
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
    let out = suite_command(&tmp)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--run-dir"])
        .arg(&run_dir)
        .env("HOME", home(&tmp))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("--exec-core"), "{stderr}");
    assert!(!run_dir.exists(), "nothing was created");

    // A profile the native resolver does not know is refused as well: the suite binds the
    // inputs the child resolves, so it cannot invent a selection of its own.
    let tmp = temp_dir("unresolvable-profile");
    let run_dir = tmp.join("run");
    let out = suite_command(&tmp)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "offline-fixture", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .env("HOME", home(&tmp))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("unsupported server profile") && stderr.contains("local-289"),
        "{stderr}"
    );
    assert!(!run_dir.exists(), "nothing was created");
}

/// The suite owns the process tree it launches: a child that ignores the graceful stop is
/// force-stopped and reaped inside the bounded cleanup, on unix (`SIGTERM`→`SIGKILL`) and on
/// Windows (console `CTRL_BREAK`→job termination) alike.
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
            "--ignore-stop".into(),
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
        "a child that ignores the graceful stop must be force-stopped: {:?}",
        run.cleanup
    );
    assert!(
        run.cleanup.reaped,
        "cleanup must reap the tree: {:?}",
        run.cleanup
    );
    assert!(elapsed >= budget, "the budget was respected: {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(25),
        "cleanup stays bounded: {elapsed:?}"
    );
    // Unix reports the signal that ended the child; the owned tree is what matters, and on
    // Windows the job termination code is the exit code with no signal to report.
    #[cfg(unix)]
    {
        assert_eq!(run.exit_code, None, "{run:?}");
        assert_eq!(run.signal, Some(9), "{run:?}");
        assert!(run.cleanup.killed_signal.is_some(), "{:?}", run.cleanup);
    }
    #[cfg(windows)]
    {
        assert_eq!(run.signal, None, "{run:?}");
        assert_eq!(run.exit_code, Some(1), "{run:?}");
    }
}

/// A child that exits on its own is still owned: the direct child is waited, the tree it
/// belonged to holds no process, and the run reports a normal exit instead of a
/// termination.
#[test]
fn a_normal_exit_is_reaped_without_a_termination() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("normal-exit");
    let spec = ChildSpec {
        label: "fixture-normal".into(),
        command: vec![FIXTURE.to_string(), "--mode".into(), "pass".into()],
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
    assert!(!run.timed_out, "{run:?}");
    assert!(!run.interrupted, "{run:?}");
    assert_eq!(run.exit_code, Some(0), "{run:?}");
    assert!(run.cleanup.reaped, "{:?}", run.cleanup);
    assert!(!run.cleanup.escalated_to_sigkill, "{:?}", run.cleanup);
    assert!(
        run.cleanup.note.contains("child exited on its own"),
        "{:?}",
        run.cleanup
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "a normal exit must not wait for the budget"
    );
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
    // Unix names the signal it asked the tree to stop with; Windows has no signal number
    // and records the mechanism in the note instead.
    #[cfg(unix)]
    assert!(run.cleanup.killed_signal.is_some(), "{:?}", run.cleanup);
    #[cfg(windows)]
    assert!(!run.cleanup.note.is_empty(), "{:?}", run.cleanup);
    assert!(
        started.elapsed() < Duration::from_secs(25),
        "an interrupt must not wait for the budget"
    );
}

/// Exercise the real Windows console handler in the suite process, not only
/// the interrupt flag used by the shared child-run path above.
#[cfg(windows)]
#[test]
fn a_windows_console_interrupt_stops_the_suite_and_owned_job() {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
    use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

    let tmp = temp_dir("console-interrupt");
    let run_dir = tmp.join("run");
    let mut command = suite_command(&tmp);
    command
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .arg("--catalog")
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .arg("--run-dir")
        .arg(&run_dir)
        .args([
            "--only",
            "fixture_one",
            "--child-arg",
            "--mode",
            "--child-arg",
            "hang",
        ])
        .env("E2E_SUITE_FIXTURE_LOG", tmp.join("launches.log"))
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut process = command.spawn().expect("start the actual suite");
    let result = (|| -> Result<std::process::ExitStatus, String> {
        let ready_deadline = Instant::now() + Duration::from_secs(30);
        while launches(&tmp).is_empty() {
            if process.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err("suite exited before launching its fixture".into());
            }
            if Instant::now() >= ready_deadline {
                return Err("suite did not launch its fixture within the bound".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, process.id()) } == 0 {
            return Err(format!(
                "console interrupt: {}",
                std::io::Error::last_os_error()
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = process.try_wait().map_err(|e| e.to_string())? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err("suite did not finish interrupted job cleanup within the bound".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    if result.is_err() {
        // Closing the suite process closes its kill-on-close job too.
        let _ = process.kill();
        let _ = process.wait();
    }
    assert_eq!(
        result.expect("real console interrupt completes").code(),
        Some(130)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger
        .attempt("fixture_one")
        .expect("interrupted attempt retained");
    assert_eq!(attempt.status, AttemptStatus::Interrupted);
    assert!(
        attempt.cleanup.as_ref().unwrap().reaped,
        "{:?}",
        attempt.cleanup
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// A log that cannot be opened refuses before any spawn: the suite must not launch a child
/// it would then have to abandon.
#[test]
fn a_log_that_cannot_be_opened_refuses_before_any_spawn() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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

/// A descendant that inherits the pipes cannot hang the suite: the drain is bounded and the
/// leftover tree is terminated to close the write ends.
#[test]
fn a_descendant_holding_the_pipes_cannot_hang_the_suite() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("pipe-hold");
    // The direct child exits 0 immediately; the descendant it leaves inherits the suite's
    // stdout/stderr and would hold the pipes open indefinitely.
    let (spec, _) = descendant_spec(&tmp, "descendant-pipe");
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
        elapsed < Duration::from_secs(30),
        "the drain is bounded, not an unbounded join: {elapsed:?}"
    );
    assert!(
        run.cleanup.note.contains("output pipes were still open"),
        "{:?}",
        run.cleanup
    );
    assert!(
        run.cleanup.reaped,
        "the leftover tree is terminated and the pipes close: {:?}",
        run.cleanup
    );
}

/// A descendant the direct child left behind with its *own* stdio cannot be detected from
/// the pipes: nothing looks wrong, the direct child exited, and the drain finishes. It is
/// still the suite's process, so it is terminated and gone when `run` returns.
#[test]
fn a_detached_descendant_is_reaped_not_ignored() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("detached-descendant");
    let (spec, pid_file) = descendant_spec(&tmp, "descendant");
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
        run.cleanup.reaped,
        "a surviving descendant is not a reaped tree: {:?}",
        run.cleanup
    );
    assert!(
        run.cleanup.note.contains("descendant"),
        "the leftover tree is reported: {:?}",
        run.cleanup
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "cleanup stays bounded: {elapsed:?}"
    );
    let descendant = recorded_pid(&pid_file);
    assert!(
        !process_alive(descendant),
        "descendant {descendant} outlived the suite's ownership"
    );
}

/// A descendant that ignores the graceful stop is force-stopped and verified inside the same
/// bound, instead of the suite reporting a clean reap because only the direct child exited.
#[test]
fn a_detached_descendant_that_ignores_the_graceful_stop_is_force_stopped() {
    let _guard = SIGNAL_GUARD
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = temp_dir("detached-forced");
    let (spec, pid_file) = descendant_spec(&tmp, "descendant-ignore-stop");
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
        run.cleanup.escalated_to_sigkill,
        "a descendant that ignores the graceful stop must be force-stopped: {:?}",
        run.cleanup
    );
    assert!(
        run.cleanup.reaped,
        "the tree is reaped after the escalation: {:?}",
        run.cleanup
    );
    assert!(
        elapsed < Duration::from_secs(45),
        "cleanup stays bounded: {elapsed:?}"
    );
    let descendant = recorded_pid(&pid_file);
    assert!(
        !process_alive(descendant),
        "descendant {descendant} survived the forced stop"
    );
}

/// The vault of the *selected profile* is bound, not a process-wide default: with
/// `--profile local-289` and no `--vault`, the suite binds `~/.274bot/vault-289`, so a
/// same-path change of the vault the panel actually reads refuses a resume.
#[test]
fn the_profile_default_vault_is_bound_and_a_same_path_change_refuses_resume() {
    let tmp = temp_dir("default-vault");
    let run_dir = tmp.join("run");
    let vault = default_vault(&tmp);
    std::fs::create_dir_all(vault.parent().unwrap()).unwrap();
    std::fs::write(&vault, "vault-289 content v1").unwrap();

    let first = suite(&tmp, &run_dir, &["--only", "fixture_one"], &[]);
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    let ledger = Ledger::resume(&run_dir).unwrap();
    let identity = &ledger.state.identity;
    assert_eq!(
        identity["profile"]["selection"],
        serde_json::json!("local-289"),
        "the native selection is recorded"
    );
    assert_eq!(
        identity["profile"]["resolved"]["vault"],
        serde_json::json!(vault.display().to_string()),
        "the vault the selected profile resolves is the one bound"
    );
    assert_eq!(
        identity["profile"]["vault"]["target"],
        serde_json::json!(vault.display().to_string())
    );
    assert!(
        identity["profile"]["vault"]["sha256"].is_string(),
        "the resolved vault is content-bound: {identity}"
    );
    assert_eq!(launches(&tmp).len(), 1);

    // The same resolved path with different content refuses resume, before any launch.
    std::fs::write(&vault, "vault-289 content v2").unwrap();
    let changed = suite(&tmp, &run_dir, &["--only", "fixture_one", "--resume"], &[]);
    assert_eq!(
        changed.status.code(),
        Some(EXIT_USAGE),
        "{}",
        text(&changed.stdout)
    );
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("profile/input configuration"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1, "a refused resume launches nothing");
}

/// A relative input path is refused: the child would resolve it against its own working
/// directory, so the suite cannot bind the path the child reads.
#[test]
fn a_relative_profile_input_is_refused() {
    let tmp = temp_dir("relative-input");
    let run_dir = tmp.join("run");
    let out = suite_command(&tmp)
        .args(["run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .args(["--vault", "relative-vault", "--run-dir"])
        .arg(&run_dir)
        .env("HOME", home(&tmp))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("--vault relative-vault is relative"),
        "{stderr}"
    );
    assert!(!run_dir.exists(), "nothing was created");
}

/// dry-run refuses a plan whose executable cannot be resolved rather than printing a line
/// the run would reject (or a `cargo run` template it would never launch).
#[test]
fn dry_run_refuses_an_unresolvable_executable() {
    let tmp = temp_dir("dry-run-unresolved");
    let target = tmp.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let manifest = tmp.join("cargo-template-manifest.json");
    std::fs::write(
        &manifest,
        std::fs::read_to_string(fixture_manifest())
            .unwrap()
            .replace("\"panel-play\"", "\"cargo\"")
            .replace(
                "\"--live\"",
                "\"run\", \"-p\", \"e2e\", \"--bin\", \"e2e-suite-fixture\"",
            ),
    )
    .unwrap();
    let out = suite_command(&tmp)
        .args(["dry-run", "--manifest"])
        .arg(&manifest)
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289"])
        .env("HOME", home(&tmp))
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("refusing to plan a run")
            && stderr.contains("no built e2e-suite-fixture executable"),
        "{stderr}"
    );
}

/// A linked vault whose target bytes change at the same path refuses resume before launch.
/// Unix-only: creating a symlink on Windows needs a privilege the test does not assume.
#[cfg(unix)]
#[test]
fn resume_refuses_a_linked_vault_whose_target_bytes_changed() {
    let tmp = temp_dir("symlink-vault");
    let run_dir = tmp.join("run");
    let target = tmp.join("vault-bytes");
    std::fs::write(&target, "vault v1").unwrap();
    let vault = tmp.join("vault");
    std::os::unix::fs::symlink(&target, &vault).unwrap();
    let vault_arg = vault.display().to_string();

    let first = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--vault", &vault_arg],
        &[],
    );
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);

    std::fs::write(&target, "vault v2").unwrap();
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
}

/// Nav pack bytes at the resolved path are bound; a same-path edit refuses resume.
#[test]
fn resume_refuses_when_nav_pack_bytes_change_at_the_same_path() {
    let tmp = temp_dir("nav-pack");
    let run_dir = tmp.join("run");
    let nav = home(&tmp).join(".274bot/289/274bot.navpack");
    std::fs::create_dir_all(nav.parent().unwrap()).unwrap();
    std::fs::write(&nav, "nav pack v1").unwrap();

    let first = suite(&tmp, &run_dir, &["--only", "fixture_one"], &[]);
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);

    std::fs::write(&nav, "nav pack v2").unwrap();
    let changed = suite(&tmp, &run_dir, &["--only", "fixture_one", "--resume"], &[]);
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("profile/input configuration"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// The launched argv[0] and cwd are the canonical paths recorded in the identity.
#[test]
fn launched_argv_and_cwd_match_the_bound_identity() {
    let tmp = temp_dir("launch-bind");
    let run_dir = tmp.join("run");
    let child_cwd = tmp.join("child-cwd");
    std::fs::create_dir_all(&child_cwd).unwrap();
    let report = tmp.join("report.txt");
    let cwd_arg = child_cwd.display().to_string();
    let report_arg = report.display().to_string();

    let out = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one",
            "--cwd",
            &cwd_arg,
            "--child-arg",
            "--report-args",
            "--child-arg",
            &report_arg,
        ],
        &[],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );

    let ledger = Ledger::resume(&run_dir).unwrap();
    let cwd_canonical = canonical(&child_cwd);
    let exec_canonical = canonical(FIXTURE);
    assert_eq!(
        ledger.state.identity["settings"]["cwd"],
        serde_json::json!(cwd_canonical)
    );
    assert_eq!(
        ledger.state.identity["binaries"]["core"]["program"],
        serde_json::json!(exec_canonical)
    );
    let attempt = ledger.attempt("fixture_one").expect("attempt recorded");
    assert_eq!(
        attempt.command.first().map(String::as_str),
        Some(exec_canonical.as_str())
    );

    let report_text = std::fs::read_to_string(&report).expect("fixture reported launch paths");
    for (prefix, expected) in [("cwd=", &cwd_canonical), ("argv0=", &exec_canonical)] {
        let actual = report_text
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing {prefix} in {report_text}"));
        // Windows current_dir can omit the verbatim-path prefix. Resolve the
        // reported path before comparing it with the bound canonical identity.
        assert_eq!(&canonical(actual), expected, "{report_text}");
    }

    let other_cwd = tmp.join("other-cwd");
    std::fs::create_dir_all(&other_cwd).unwrap();
    let other = other_cwd.display().to_string();
    let changed = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one",
            "--cwd",
            &other,
            "--child-arg",
            "--report-args",
            "--child-arg",
            &report_arg,
            "--resume",
        ],
        &[],
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("settings"),
        "{stderr}"
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// A --child-arg that supplies --vault is bound as the effective argv, not the earlier request.
#[test]
fn extra_args_cannot_evade_profile_identity() {
    let tmp = temp_dir("extra-vault");
    let run_dir = tmp.join("run");
    let vault_a = tmp.join("vault-a");
    let vault_b = tmp.join("vault-b");
    std::fs::write(&vault_a, "a-v1").unwrap();
    std::fs::write(&vault_b, "b-v1").unwrap();
    let a = vault_a.display().to_string();
    let b = vault_b.display().to_string();

    let first = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one",
            "--vault",
            &a,
            "--child-arg",
            "--vault",
            "--child-arg",
            &b,
        ],
        &[],
    );
    assert_eq!(
        first.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&first.stdout),
        text(&first.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);
    let ledger = Ledger::resume(&run_dir).unwrap();
    assert_eq!(
        ledger.state.identity["profile"]["resolved"]["vault"],
        serde_json::json!(b),
        "effective extra --vault is what the identity binds"
    );

    std::fs::write(&vault_b, "b-v2").unwrap();
    let changed = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one",
            "--vault",
            &a,
            "--child-arg",
            "--vault",
            "--child-arg",
            &b,
            "--resume",
        ],
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

/// The actual child argv and retained identity carry the same typed paint choice.
#[test]
fn headed_paints_default_on_and_changed_choice_refuses_resume() {
    let tmp = temp_dir("nav-paints");
    let run_dir = tmp.join("run");
    let report = tmp.join("argv.txt");
    let report_arg = report.display().to_string();
    let base = [
        "--only",
        "fixture_one",
        "--child-arg",
        "--report-args",
        "--child-arg",
        &report_arg,
    ];
    let out = suite(&tmp, &run_dir, &base, &[]);
    assert_eq!(out.status.code(), Some(EXIT_OK), "{}", text(&out.stderr));
    let argv = std::fs::read_to_string(&report).unwrap();
    assert!(argv.contains("--nav-paints\non\n"), "{argv}");
    assert_eq!(
        Ledger::resume(&run_dir).unwrap().state.identity["settings"]["nav_paints"],
        true
    );
    let mut changed = base.to_vec();
    changed.extend(["--resume", "--nav-paints", "off"]);
    let out = suite(&tmp, &run_dir, &changed, &[]);
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    assert!(text(&out.stderr).contains("refusing resume"));
    assert_eq!(launches(&tmp).len(), 1);
    let mut off = base.to_vec();
    off.extend(["--nav-paints", "off"]);
    let out = suite(&tmp, &tmp.join("off-run"), &off, &[]);
    assert_eq!(out.status.code(), Some(EXIT_OK), "{}", text(&out.stderr));
    assert!(std::fs::read_to_string(&report)
        .unwrap()
        .contains("--nav-paints\noff\n"));
}

/// If a pipe outlives even the stopped owned tree, stop with cleanup failure.
/// The escaped fixture is deliberately outside the suite's ownership and cleaned by this
/// test. Unix-only: a process can leave a process group (`process_group(0)`), while a job
/// grants no breakaway, so there is no honest Windows equivalent to gate.
#[cfg(unix)]
#[test]
fn unclosed_output_pipe_is_cleanup_failure_and_stops_the_next_case() {
    let tmp = temp_dir("escaped-pipe");
    let run_dir = tmp.join("run");
    let pid_file = tmp.join("escaped.pid");
    let pid_path = pid_file.display().to_string();
    let started = Instant::now();
    let out = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_one,fixture_two",
            "--child-arg",
            "--mode",
            "--child-arg",
            "escape-pipe",
        ],
        &[("E2E_SUITE_ESCAPED_PID", &pid_path)],
    );
    // Clean the known test process before assertions can panic.
    let escaped_pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    unsafe {
        libc::kill(escaped_pid, libc::SIGKILL);
    }
    assert_eq!(
        out.status.code(),
        Some(EXIT_FAILURE),
        "{}",
        text(&out.stderr)
    );
    assert!(started.elapsed() < Duration::from_secs(15));
    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger.attempt("fixture_one").unwrap();
    assert_eq!(attempt.status, AttemptStatus::CleanupFailed);
    assert!(
        attempt.reason.contains("output pipes did not close"),
        "{}",
        attempt.reason
    );
    assert!(
        ledger.attempt("fixture_two").is_none(),
        "cleanup failure must stop the suite"
    );
}

/// An inherited native deadline control is refused before any launch, and the message names
/// it: the suite hands a child the case budget and the scenario's own inner deadline, so an
/// inherited `BUDGET_S` must not silently move them.
#[test]
fn an_inherited_native_deadline_override_is_refused_before_any_launch_or_resume() {
    let tmp = temp_dir("deadline-env");
    let run_dir = tmp.join("run");

    // A run that inherits the override refuses, names it, and launches nothing.
    let out = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one"],
        &[("BUDGET_S", "900")],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_USAGE),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    let stderr = text(&out.stderr);
    assert!(stderr.contains("BUDGET_S"), "{stderr}");
    assert!(stderr.contains("Unset BUDGET_S"), "{stderr}");
    assert!(
        !run_dir.exists(),
        "a refused deadline override creates no run directory"
    );
    assert_eq!(
        launches(&tmp).len(),
        0,
        "nothing may be launched while an inner deadline override is inherited"
    );

    // The same request without the override is accepted: the refusal is about the
    // environment, not about the configuration.
    let out = suite(&tmp, &run_dir, &["--only", "fixture_one"], &[]);
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);

    // An empty value is not an override (the native reader treats it as unset), so a resume
    // carries the recorded result instead of refusing.
    let out = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--resume"],
        &[("BUDGET_S", "")],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1, "resume must not relaunch");

    // A resume that inherits a nonempty override refuses before any launch as well.
    let out = suite(
        &tmp,
        &run_dir,
        &["--only", "fixture_one", "--resume"],
        &[("BUDGET_S", "900")],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_USAGE),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert!(
        text(&out.stderr).contains("BUDGET_S"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1, "a refused resume launches nothing");
}

/// `dry-run` refuses the same inheritance instead of printing a plan the run would reject.
#[test]
fn dry_run_refuses_an_inherited_native_deadline_override() {
    let tmp = temp_dir("deadline-env-dry");
    let out = suite_command(&tmp)
        .args(["dry-run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_one", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--exec-core"])
        .arg(FIXTURE)
        .env("BUDGET_S", "900")
        .output()
        .expect("the suite binary runs");
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("BUDGET_S"), "{stderr}");
    assert_eq!(launches(&tmp).len(), 0, "dry-run launches nothing");
}

/// The loader smoke's raw source, copied out of the tracked fixture so a test can change the
/// bytes at the same path without touching the producer's file.
fn external_source_copy(tmp: &Path) -> PathBuf {
    let tracked = host_play::external_loader::default_frozen_source();
    let copy = tmp.join("catalog-external/ExampleBot.ts");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&tracked, &copy).unwrap();
    copy
}

/// The external loader smoke is executed through the dedicated runner, and its result is the
/// producer's own receipt plus the terminal capture — never a bare PASS.
#[test]
fn the_external_loader_smoke_is_retained_as_pending_visual_review() {
    let tmp = temp_dir("external-pass");
    let run_dir = tmp.join("run");
    let source = external_source_copy(&tmp);
    let source_path = source.display().to_string();
    let report = tmp.join("argv.txt");
    let report_arg = report.display().to_string();
    let out = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &source_path,
            "--child-arg",
            "--report-args",
            "--child-arg",
            &report_arg,
            "--child-arg",
            "--mode",
            "--child-arg",
            "external-pass",
        ],
        &[("E2E_SUITE_FIXTURE_ACCOUNT", "alice")],
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);

    // The child received the typed source override and the case's own live name.
    let argv = std::fs::read_to_string(&report).unwrap();
    assert!(
        argv.contains(&format!("--external-ts\n{source_path}\n")),
        "{argv}"
    );
    assert!(argv.contains("--live\nscript_external_loader\n"), "{argv}");

    let ledger = Ledger::resume(&run_dir).unwrap();
    let attempt = ledger
        .attempt("fixture_external")
        .expect("attempt recorded");
    assert_eq!(attempt.status, AttemptStatus::PendingVisualReview);
    assert_eq!(attempt.exit_code, Some(0));
    let receipt = attempt
        .receipts
        .as_ref()
        .and_then(|receipts| receipts.external.as_deref())
        .expect("the external witness line is recorded");
    assert!(
        receipt.starts_with("EXTERNAL_LOADER: script_external_loader {"),
        "{receipt}"
    );
    assert!(
        receipt.contains(host_play::external_loader::FROZEN_SHA256),
        "the receipt carries the bound raw source digest: {receipt}"
    );
    assert!(
        attempt
            .completed_operation
            .as_deref()
            .is_some_and(|line| line.contains("+external_loader")),
        "{:?}",
        attempt.completed_operation
    );
    assert_eq!(
        attempt.captures.len(),
        1,
        "the terminal capture is attributed"
    );
    assert!(
        attempt.captures[0]
            .label
            .contains("external_loader_terminal"),
        "{:?}",
        attempt.captures[0]
    );
    assert!(attempt.captures[0].complete(), "{:?}", attempt.captures[0]);

    // The run identity binds the source it hashed, and the case row records the same input.
    let bound = &ledger.state.identity["external"];
    assert_eq!(bound["path"], serde_json::json!(canonical(&source)));
    assert_eq!(
        bound["sha256"],
        serde_json::json!(host_play::external_loader::FROZEN_SHA256)
    );
    assert_eq!(bound["default_fixture"], serde_json::json!(false));
    assert_eq!(bound["frozen_match"], serde_json::json!(true));
    assert_eq!(
        attempt.case_identity["external"]["path"],
        serde_json::json!(canonical(&source))
    );

    // The run is not a qualification: the capture still needs a human readback.
    assert!(!text(&out.stdout).contains("PASS: live script_external_loader"));
}

/// A same-path byte change in the bound raw source refuses a resume before any spawn, and a raw
/// `--child-arg --external-ts` cannot re-bind the typed source at all.
#[test]
fn an_external_source_change_or_a_raw_rebind_refuses_before_any_spawn() {
    let tmp = temp_dir("external-resume");
    let run_dir = tmp.join("run");
    let source = external_source_copy(&tmp);
    let source_path = source.display().to_string();
    let out = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &source_path,
            "--child-arg",
            "--mode",
            "--child-arg",
            "external-pass",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(EXIT_OK), "{}", text(&out.stderr));
    assert_eq!(launches(&tmp).len(), 1);

    // Same path, different bytes: the bound content changed.
    std::fs::write(&source, "// not the bot\n").unwrap();
    let changed = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &source_path,
            "--resume",
        ],
        &[],
    );
    assert_eq!(changed.status.code(), Some(EXIT_USAGE));
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("refusing resume") && stderr.contains("external loader source"),
        "{stderr}"
    );
    assert_eq!(
        launches(&tmp).len(),
        1,
        "nothing was launched after the refusal"
    );

    // The same bytes at a different path are a different bound input.
    let other = tmp.join("elsewhere/ExampleBot.ts");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::copy(host_play::external_loader::default_frozen_source(), &other).unwrap();
    let other_path = other.display().to_string();
    let moved = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &other_path,
            "--resume",
        ],
        &[],
    );
    assert_eq!(moved.status.code(), Some(EXIT_USAGE));
    assert!(
        text(&moved.stderr).contains("refusing resume"),
        "{}",
        text(&moved.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);

    // A raw extra argument may not re-bind the typed source.
    let evasive = suite(
        &tmp,
        &tmp.join("evasive-run"),
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &other_path,
            "--child-arg",
            "--external-ts",
            "--child-arg",
            "/tmp/other.ts",
        ],
        &[],
    );
    assert_eq!(evasive.status.code(), Some(EXIT_USAGE));
    assert!(
        text(&evasive.stderr).contains("--external-ts"),
        "{}",
        text(&evasive.stderr)
    );
    assert_eq!(launches(&tmp).len(), 1);
}

/// A relative `--external-ts` is refused up front, like the panel refuses its own flag.
#[test]
fn a_relative_external_source_is_refused() {
    let tmp = temp_dir("external-relative");
    let out = suite_command(&tmp)
        .args(["dry-run", "--manifest"])
        .arg(fixture_manifest())
        .args(["--only", "fixture_external", "--catalog"])
        .arg(catalog(&tmp))
        .args(["--profile", "local-289", "--external-ts", "ExampleBot.ts"])
        .output()
        .expect("the suite binary runs");
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    assert!(
        text(&out.stderr).contains("relative"),
        "{}",
        text(&out.stderr)
    );
}

/// Every dishonest external proof shape is refused: a missing witness, a contradictory or
/// incomplete record, a prerequisite-only capture, no capture, and a torn terminal capture.
#[test]
fn dishonest_external_proof_shapes_are_refused() {
    for (mode, status, needle) in [
        (
            "external-no-witness",
            AttemptStatus::SharedFailure,
            "no EXTERNAL_LOADER witness",
        ),
        (
            "external-inconsistent",
            AttemptStatus::SharedFailure,
            "disagree",
        ),
        (
            "external-incomplete",
            AttemptStatus::SharedFailure,
            "capture request",
        ),
        (
            "external-prereq-only",
            AttemptStatus::SharedFailure,
            "not written by this case",
        ),
        (
            "external-no-capture",
            AttemptStatus::SharedFailure,
            "was not written by this case",
        ),
        (
            "external-bad-capture",
            AttemptStatus::SharedFailure,
            "not a PNG",
        ),
        (
            "external-fail",
            AttemptStatus::Failed,
            "failed at stage failed",
        ),
    ] {
        let tmp = temp_dir(&format!("external-{mode}"));
        let run_dir = tmp.join("run");
        let source = external_source_copy(&tmp);
        let source_path = source.display().to_string();
        let out = suite(
            &tmp,
            &run_dir,
            &[
                "--only",
                "fixture_external",
                "--external-ts",
                &source_path,
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
            "{mode} must not qualify: {}",
            text(&out.stdout)
        );
        let ledger = Ledger::resume(&run_dir).unwrap();
        let attempt = ledger
            .attempt("fixture_external")
            .unwrap_or_else(|| panic!("{mode}: attempt recorded"));
        assert_eq!(attempt.status, status, "{mode}: {}", attempt.reason);
        assert!(
            attempt.reason.contains(needle),
            "{mode}: expected {needle:?} in {:?}",
            attempt.reason
        );
    }

    // A failed case keeps its own stage/request/completion record instead of a bare "failed".
    let tmp = temp_dir("external-fail-detail");
    let run_dir = tmp.join("run");
    let source = external_source_copy(&tmp);
    let source_path = source.display().to_string();
    let out = suite(
        &tmp,
        &run_dir,
        &[
            "--only",
            "fixture_external",
            "--external-ts",
            &source_path,
            "--child-arg",
            "--mode",
            "--child-arg",
            "external-fail",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(EXIT_FAILURE));
    let ledger = Ledger::resume(&run_dir).unwrap();
    let reason = &ledger
        .attempt("fixture_external")
        .expect("attempt recorded")
        .reason;
    assert!(reason.contains("requested prepare_fixture"), "{reason}");
    assert!(
        reason.contains("fixture did not reach the scene gate"),
        "{reason}"
    );
}

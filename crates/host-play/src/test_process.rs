//! Keep unrelated process spawns away from local-state file locks.
//!
//! State tests hold an exclusive resource lease through root and service
//! teardown. Exact state subprocesses retain the parent's lease until exit.
//! This also bounds filesystem-heavy fixture concurrency. Other tests remain
//! parallel, and each state test keeps its internal thread and process concurrency.
//! Linux flock locks belong to open file descriptions: an unrelated fork can
//! retain a lock until exec, even after the owning test drops its descriptor.

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};

#[path = "../tests/support/test_executable.rs"]
mod test_executable;

static STATE_AND_SPAWN: Mutex<()> = Mutex::new(());

pub(crate) fn executable() -> PathBuf {
    test_executable::executable().expect("the current test executable has a path")
}

/// Hold through a state test, including its service and root teardown.
pub(crate) fn state() -> MutexGuard<'static, ()> {
    STATE_AND_SPAWN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Exclude state tests while a different test creates a child process.
pub(crate) fn spawn_window() -> MutexGuard<'static, ()> {
    state()
}

/// Release the gate as soon as process creation returns, before waiting.
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    let _window = spawn_window();
    command.spawn()
}

/// Capture a child without holding the spawn gate while the child executes.
pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn(command)?.wait_with_output()
}

const ISOLATED_TEST: &str = "HOST_PLAY_ISOLATED_STATE_TEST";

/// Return true after running the selected test in a fresh process. In that
/// process, return false so the caller executes its unchanged test body.
pub(crate) fn isolate(full_name: &str) -> bool {
    let (_, test) = full_name.split_once("::").expect("a crate-qualified test");
    if std::env::var(ISOLATED_TEST).as_deref() == Ok(test) {
        return false;
    }
    run(test, &[]);
    true
}

/// Run one exact state test, including an ignored child, under the resource
/// lease. Preserve output on failure and require exactly one passed test.
pub(crate) fn run(test: &str, envs: &[(&str, &str)]) {
    let _state_lease = state();
    let mut command = Command::new(executable());
    command
        .args(["--include-ignored", "--exact", test, "--nocapture"])
        .env(ISOLATED_TEST, test)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the isolated child {test} failed ({status}):\n{stdout}\n{stderr}",
        status = output.status
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;")),
        "the isolated child {test} did not pass exactly one test:\n{stdout}\n{stderr}"
    );
}

macro_rules! isolate_state_test {
    ($test:ident) => {
        if $crate::test_process::isolate(concat!(module_path!(), "::", stringify!($test))) {
            return;
        }
    };
}

pub(crate) use isolate_state_test;

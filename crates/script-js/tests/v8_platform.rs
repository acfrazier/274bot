use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const CHILD_ARGUMENT: &str = "--v8-platform-ancestry-child";
const CHILD_TIMEOUT: Duration = Duration::from_secs(45);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WORKER_TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new(CHILD_ARGUMENT)) {
        assert!(args.next().is_none(), "unexpected child arguments");
        run_child();
    } else {
        run_parent();
    }
}

fn run_parent() {
    let executable = std::env::current_exe().expect("locate v8_platform executable");
    let mut child = Command::new(executable)
        .arg(CHILD_ARGUMENT)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn v8_platform child");

    let stdout = child.stdout.take().expect("capture child stdout");
    let stderr = child.stderr.take().expect("capture child stderr");
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));

    let (status, timed_out) = wait_for_child(&mut child);
    let stdout = stdout_reader.join().expect("read child stdout");
    let stderr = stderr_reader.join().expect("read child stderr");
    let status_description = describe_status(&status);

    assert!(
        status.success(),
        "V8 ancestry child {status_description}{}\nstdout:\n{}\nstderr:\n{}",
        if timed_out { " (timed out)" } else { "" },
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
}

fn read_all(mut reader: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .expect("read subprocess output");
    output
}

fn wait_for_child(child: &mut Child) -> (ExitStatus, bool) {
    let deadline = Instant::now() + CHILD_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("wait for V8 ancestry child") {
            return (status, false);
        }
        if Instant::now() >= deadline {
            // A timeout must stay confined to this subprocess. `kill` can race
            // with natural exit; `wait` below still collects either outcome.
            let _ = child.kill();
            return (
                child.wait().expect("reap timed-out V8 ancestry child"),
                true,
            );
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn describe_status(status: &ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            #[cfg(target_os = "linux")]
            if signal == 11 {
                return format!("terminated by signal {signal} (SIGSEGV)");
            }
            return format!("terminated by signal {signal}");
        }
    }

    match status.code() {
        Some(code) => format!("exited with code {code}"),
        None => format!("terminated with status {status:?}"),
    }
}

fn run_child() {
    let late = thread::spawn(script_js::bootstrap_v8_platform)
        .join()
        .expect("late worker returned");
    assert!(
        matches!(
            late,
            Err(script_js::LoadIsolateError::PlatformNotMainThread)
        ),
        "worker bootstrap must fail before it can initialize V8: {late:?}"
    );
    // Keep this as the single root bootstrap call: removing this line must
    // leave only the worker's deliberately late, lazy-initializer call below.
    script_js::bootstrap_v8_platform().expect("process-main protected bootstrap");

    let (start_a_tx, start_a_rx) = mpsc::channel();
    let (ready_a_tx, ready_a_rx) = mpsc::channel();
    let (release_a_tx, release_a_rx) = mpsc::channel();
    let worker_a = thread::Builder::new()
        .name("v8-platform-slot-a".into())
        .spawn(move || {
            receive(&start_a_rx, "start slot-a");

            // Harmless after the main-thread bootstrap; this reproduces the
            // original lazy initializer when the root call is mutation-deleted.
            script_js::bootstrap_v8_platform().expect("already bootstrapped");
            let isolate = new_native_tick_isolate("slot-a");
            ready_a_tx.send(()).expect("report slot-a ready");
            receive(&release_a_rx, "release slot-a");
            isolate.join();
            say("slot-a: joined");
        })
        .expect("spawn slot-a worker");

    let (start_b_tx, start_b_rx) = mpsc::channel();
    let (ready_b_tx, ready_b_rx) = mpsc::channel();
    let (restart_b_tx, restart_b_rx) = mpsc::channel();
    let (finished_b_tx, finished_b_rx) = mpsc::channel();
    let worker_b = thread::Builder::new()
        .name("v8-platform-slot-b".into())
        .spawn(move || {
            receive(&start_b_rx, "start slot-b");
            let first = new_native_tick_isolate("slot-b");
            first
                .probe("globalThis.__v8_platform_generation = 'first'; true")
                .expect("mark slot-b runtime");
            assert_eq!(
                first
                    .probe("globalThis.__v8_platform_generation")
                    .expect("read slot-b runtime marker"),
                "first"
            );
            ready_b_tx.send(()).expect("report slot-b ready");

            receive(&restart_b_rx, "restart slot-b isolate");
            first.join();

            // Recreate on this same worker only after the third worker's
            // isolate has completed, proving a fresh V8 runtime is usable.
            let restarted = new_native_tick_isolate("slot-b-restarted");
            assert_eq!(
                restarted
                    .probe("typeof globalThis.__v8_platform_generation")
                    .expect("check restarted runtime state"),
                "undefined",
                "the restarted isolate must not retain the previous runtime's globals"
            );
            restarted.join();
            finished_b_tx
                .send(())
                .expect("report slot-b restart complete");
            say("slot-b: original and restarted isolates joined");
        })
        .expect("spawn slot-b worker");

    let (start_c_tx, start_c_rx) = mpsc::channel();
    let (ready_c_tx, ready_c_rx) = mpsc::channel();
    let worker_c = thread::Builder::new()
        .name("v8-platform-slot-c".into())
        .spawn(move || {
            receive(&start_c_rx, "start slot-c");
            let isolate = new_native_tick_isolate("slot-c");
            isolate.join();
            ready_c_tx.send(()).expect("report slot-c ready and joined");
            say("slot-c: joined");
        })
        .expect("spawn slot-c worker");

    // All three sibling workers now exist. Start each isolate only after its
    // predecessor has completed its probe; no concurrent construction is
    // needed to exercise the process-root PKU ancestry requirement.
    start_a_tx.send(()).expect("start slot-a");
    receive(&ready_a_rx, "wait for slot-a probe");

    start_b_tx.send(()).expect("start slot-b");
    receive(&ready_b_rx, "wait for slot-b probe");

    start_c_tx.send(()).expect("start slot-c");
    receive(&ready_c_rx, "wait for slot-c probe and join");

    restart_b_tx.send(()).expect("restart slot-b isolate");
    receive(&finished_b_rx, "wait for slot-b restart");

    release_a_tx.send(()).expect("release slot-a isolate");
    worker_a.join().expect("join slot-a worker");
    worker_b.join().expect("join slot-b worker");
    worker_c.join().expect("join slot-c worker");

    say("all sibling-owned isolates passed; workers were joined deterministically");
}

fn new_native_tick_isolate(label: &str) -> script_js::LoadIsolate {
    let isolate = script_js::LoadIsolate::spawn(
        "export function tick(api) {}".to_owned(),
        script_js::LoadShape::NativeTick,
        Vec::new(),
    )
    .unwrap_or_else(|error| panic!("{label}: spawn NativeTick isolate: {error}"));
    let result = isolate
        .probe("1 + 1")
        .unwrap_or_else(|error| panic!("{label}: wait for isolate readiness: {error}"));
    assert_eq!(result.as_i64(), Some(2), "{label}: probe evaluated");
    say(&format!("{label}: ready and probe evaluated"));
    isolate
}

fn say(message: &str) {
    println!("{message}");
    let _ = std::io::stdout().flush();
}

fn receive<T>(receiver: &Receiver<T>, stage: &str) -> T {
    receiver
        .recv_timeout(WORKER_TIMEOUT)
        .unwrap_or_else(|error| panic!("{stage}: worker coordination failed: {error}"))
}

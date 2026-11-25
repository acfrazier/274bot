#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsFd;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(unix)]
use std::sync::{mpsc, Arc};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
const CHILD_MODE: &str = "T1_STALLED_STDERR_CHILD";
#[cfg(unix)]
const TEST_NAME: &str = "stalled_pipe_does_not_block_slot_progress_or_shutdown";
#[cfg(unix)]
const FILL_LIMIT: usize = 16 * 1024 * 1024;
#[cfg(unix)]
const FILL_CHUNK: usize = 8 * 1024;

/// The ignored live cell gives this test a real process stderr pipe whose
/// parent deliberately does not read until the child has exited.
#[cfg(unix)]
#[test]
#[ignore = "real unread stderr pipe; run from tools/live-map"]
fn stalled_pipe_does_not_block_slot_progress_or_shutdown() {
    if std::env::var_os(CHILD_MODE).is_some() {
        run_stalled_child();
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_MODE, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not finish while its stderr pipe was unread");
        }
        thread::sleep(Duration::from_millis(10));
    };

    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(status.success(), "child failed: {status}; {stdout}");
    assert!(stdout.contains("pipe_bytes="), "{stdout}");
    assert!(stdout.contains("progress=4096"), "{stdout}");
    assert!(stdout.contains("dropped="), "{stdout}");
    assert!(stdout.contains("shutdown_ms="), "{stdout}");
    assert!(stdout.contains("tick=4095"), "{stdout}");
}

#[cfg(unix)]
fn run_stalled_child() -> ! {
    let written = Arc::new(AtomicUsize::new(0));
    let filler_done = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let filler_written = Arc::clone(&written);
    let filler_done_flag = Arc::clone(&filler_done);
    let filler = thread::spawn(move || {
        // Write through a duplicate of descriptor 2 (`AsFd` borrows it without
        // Rust's stderr lock), so the host logger can acquire that lock and
        // block in the pipe write itself. The `File` owns the duplicate; the
        // process exit closes it, because this thread stays blocked in `write`.
        let Ok(fd) = std::io::stderr().as_fd().try_clone_to_owned() else {
            let _ = started_tx.send(());
            filler_done_flag.store(true, Ordering::Release);
            return;
        };
        let mut stderr = std::fs::File::from(fd);
        let chunk = [b'!'; FILL_CHUNK];
        let _ = started_tx.send(());
        let mut total = 0;
        while total < FILL_LIMIT {
            let len = (FILL_LIMIT - total).min(chunk.len());
            match stderr.write(&chunk[..len]) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    total += count;
                    filler_written.store(total, Ordering::Release);
                }
            }
        }
        filler_done_flag.store(true, Ordering::Release);
    });
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stderr filler did not start");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_written = 0;
    let mut unchanged_since = Instant::now();
    loop {
        if filler_done.load(Ordering::Acquire) {
            child_fail(format!(
                "stderr filler ended before stalling after {} bytes",
                written.load(Ordering::Acquire)
            ));
        }
        let current = written.load(Ordering::Acquire);
        if current != last_written {
            last_written = current;
            unchanged_since = Instant::now();
        } else if current > 0 && unchanged_since.elapsed() >= Duration::from_millis(500) {
            break;
        }
        if Instant::now() >= deadline {
            child_fail("stderr filler did not establish a stalled pipe".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    let stalled_bytes = last_written;

    api::hostlog::start_stderr_writer();
    api::hostlog::set_debug(true);
    api::hostlog::register_secret("stalled-pipe-secret");
    api::hostlog::bind_slot("t1-stalled-pipe");
    let payload = "x".repeat(900);
    for index in 0..4096 {
        api::hostlog::set_tick(index as u32);
        match index % 3 {
            0 => api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "debug {index} {payload}"
            ),
            1 => api::host_log!(
                stderr;
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Info,
                "always-stderr {index} {payload}"
            ),
            _ => api::host_log!(
                stderr;
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Error,
                "error {index} stalled-pipe-secret {payload}"
            ),
        }
    }
    let pipe_bytes = written.load(Ordering::Acquire);
    if pipe_bytes != stalled_bytes {
        child_fail("stderr pipe advanced during slot log submission".into());
    }
    let tick = api::hostlog::slot_tick("t1-stalled-pipe").unwrap_or(u32::MAX);
    if tick != 4095 {
        child_fail(format!("slot tick stopped at {tick}"));
    }
    let dropped = api::hostlog::stderr_dropped();
    let minimum_drops = 4096 - api::hostlog::STDERR_QUEUE_LINES - 1;
    if dropped < minimum_drops as u64 {
        child_fail(format!(
            "stalled stderr dropped {dropped} lines; expected at least {minimum_drops}"
        ));
    }

    let shutdown_start = Instant::now();
    let lost = api::hostlog::shutdown_stderr_writer();
    let shutdown_ms = shutdown_start.elapsed().as_millis();
    if shutdown_ms >= 1_000 {
        child_fail(format!("shutdown waited {shutdown_ms} ms"));
    }
    // The entries still pending behind the stalled sink are given up and counted.
    if lost == 0 || api::hostlog::stderr_dropped() != dropped + lost {
        child_fail(format!(
            "lost {lost} at exit; drop counter {} was {dropped} before",
            api::hostlog::stderr_dropped()
        ));
    }

    println!(
        "pipe_bytes={pipe_bytes} progress=4096 tick={tick} dropped={dropped} lost_at_exit={lost} shutdown_ms={shutdown_ms}",
    );
    std::io::stdout().flush().unwrap();
    // The filler thread is still blocked in `write` on the full pipe and its
    // `File` is never dropped. Dropping the handle only detaches the thread; the
    // process exit closes the duplicated descriptor. Joining would deadlock,
    // because the parent reads stderr only after this process has exited.
    drop(filler);
    std::process::exit(0)
}

#[cfg(unix)]
fn child_fail(message: String) -> ! {
    println!("FAIL {message}");
    let _ = std::io::stdout().flush();
    std::process::exit(1)
}

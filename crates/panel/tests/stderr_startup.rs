use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn bad_flag_prints_error_and_exits_without_hanging() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_panel-play"))
        .arg("--review-no-such-flag")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start panel-play");
    // A start on a loaded host took over five seconds in three full-suite runs,
    // while a warm run takes 10 ms: the deadline separates a slow start from a
    // hang.
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if child.try_wait().expect("poll panel-play").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("panel-play did not exit after rejecting a bad flag within thirty seconds");
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }

    let output = child.wait_with_output().expect("collect panel-play output");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown --review-no-such-flag"),
        "panel-play did not print its bad-flag error to stderr: {stderr}"
    );
}

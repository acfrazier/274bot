//! A session log records the startup open-file limit raise, even though the
//! binaries raise it before any session file exists. Its own test binary:
//! it lowers this process's limit and points HOME at a scratch directory.

#[cfg(unix)]
#[test]
fn session_log_records_the_raised_open_file_limit() {
    let home = std::env::temp_dir().join(format!("nofile-limit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // This test binary has one test and no other threads yet.
    std::env::set_var("HOME", &home);

    // A Finder-launched macOS app starts at 256.
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid, writable rlimit.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) }, 0);
    lim.rlim_cur = 256;
    // SAFETY: lowering the soft limit below the unchanged hard limit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) }, 0);

    // What panel-play and tui-play do first, then the user's session log.
    assert!(host_play::fd_limit::raise_at_startup().is_some());
    let path = frontend_core::log_file::apply_session_log(true).expect("session file");
    frontend_core::log_file::apply_session_log(false);

    // The writer thread creates and fills the file asynchronously; closing
    // the session lets it drain and exit.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let text = loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.contains("open-file limit raised from 256 to")
            || std::time::Instant::now() >= deadline
        {
            break text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        text.contains("open-file limit raised from 256 to"),
        "session log lacks the limit line:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

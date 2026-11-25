//! After a host-log stderr line, other threads must still be able to use stderr.
use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

fn probe(label: &'static str, action: fn()) -> bool {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        action();
        let _ = tx.send(());
    });
    let returned = rx.recv_timeout(Duration::from_secs(5)).is_ok();
    println!("probe {label}: returned_within_5s={returned}");
    returned
}

#[test]
fn stderr_stays_usable_after_a_host_log_line() {
    api::host_log!(
        stderr;
        api::hostlog::Category::Lifecycle,
        api::hostlog::Level::Error,
        "reviewer probe line"
    );
    // Give a lazily started writer time to run.
    std::thread::sleep(Duration::from_millis(500));
    let flush = probe("io::stderr().flush()", || {
        let _ = std::io::stderr().flush();
    });
    let write = probe("writeln!(io::stderr())", || {
        let _ = writeln!(std::io::stderr(), "reviewer probe direct write");
    });
    assert!(
        flush && write,
        "flush={flush} write={write}: another thread holds the std stderr lock"
    );
}

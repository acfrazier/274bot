//! LOGIN-NOTICE-1024 live cell `frontend-core.login-notice-refused`: real
//! slot workers log in to the refused endpoint `127.0.0.1:1` (the headed
//! check's endpoint) through the real `HeadlessDriver`, on the wall clock.
//! Only client asset startup is bypassed; the refusal, the host's Error
//! publication, its retry backoff and the poll are the product's own.
//!
//! 1. bob's refused login raises his login notice; Log out parks him as
//!    logged out, and the notice stays in every poll until 15 s after it
//!    appeared, then clears (the error stays as history).
//! 2. Log in again without logging out: the host retries after its backoff
//!    and each refused retry renews the notice, so it never lapses.

use std::time::{Duration, Instant};

use frontend_core::launch::{InstancePermit, PlayConfig};
use frontend_core::{HeadlessDriver, OperatorCommand, Phase, Targets, ERROR_MESSAGE_TIMEOUT};
use vault::{Profile, ProfileSettings, Vault};

const FRAME: Duration = Duration::from_millis(20);

fn notice(driver: &HeadlessDriver) -> Option<String> {
    driver
        .session()
        .fleet_view()
        .detail()
        .and_then(|detail| detail.row().login_notice().map(str::to_owned))
}

fn phase(driver: &HeadlessDriver) -> Phase {
    driver
        .session()
        .fleet_view()
        .detail()
        .map_or(Phase::Offline, |detail| detail.row().phase())
}

/// The host's raw Error-phase start for bob, while his row is in Error.
fn raw_failure(driver: &HeadlessDriver) -> Option<Instant> {
    let play = driver.session().play_for_test()?;
    let rows = play.statuses.lock().unwrap();
    rows.iter()
        .find(|row| row.username == "bob")
        .filter(|row| row.startup_phase == host_play::StartupPhase::Error && row.error.is_some())
        .map(|row| row.startup_phase_started)
}

/// Poll until bob shows a login notice; returns when it was first seen.
fn await_notice(driver: &mut HeadlessDriver, deadline: Duration) -> Instant {
    let start = Instant::now();
    loop {
        driver.poll();
        if notice(driver).is_some() {
            return Instant::now();
        }
        assert!(
            start.elapsed() < deadline,
            "bob's refused login must raise a login notice (phase {:?})",
            phase(driver)
        );
        std::thread::park_timeout(FRAME);
    }
}

#[test]
#[ignore = "live cell frontend-core.login-notice-refused: real workers against 127.0.0.1:1, about 40 s of wall clock"]
fn a_refused_login_notice_outlives_log_out_and_renews_on_real_retries() {
    let refused = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:1".parse().unwrap(),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::ConnectionRefused);

    let dir = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"))
        .join("login-notice-refused");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "login-notice-test-passphrase").unwrap();
    for (i, name) in ["alice", "bob"].into_iter().enumerate() {
        vault
            .upsert(Profile {
                username: name.into(),
                password: "test-password".into(),
                uid: 274_102_400 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    drop(vault);
    let mut driver = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
    driver.session_mut().set_bypass_asset_startup(true);
    driver.session_mut().set_focus_hint(Some("bob"));
    driver
        .unlock(
            path,
            "login-notice-test-passphrase",
            &PlayConfig::direct("127.0.0.1", 1, "/tmp", true),
        )
        .unwrap();
    driver
        .submit(OperatorCommand::Load, Targets::Everyone)
        .unwrap();
    driver
        .submit(OperatorCommand::Select, Targets::Profile("bob"))
        .unwrap();
    let ready = Instant::now();
    loop {
        driver.poll();
        if matches!(phase(&driver), Phase::Offline | Phase::LoggedOut) {
            break;
        }
        assert!(
            ready.elapsed() < Duration::from_secs(10),
            "bob must finish preparing"
        );
        std::thread::park_timeout(FRAME);
    }

    // 1. Refused, then Log out: the notice stays until its 15 s.
    driver
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .unwrap();
    let seen = await_notice(&mut driver, Duration::from_secs(10));
    let text = notice(&driver).unwrap();
    println!("NOTICE {text:?} phase={:?}", phase(&driver));
    assert!(
        text.starts_with("login Error connecting to server.") && text.contains("refused"),
        "{text}"
    );
    driver
        .submit(OperatorCommand::Logout, Targets::Profile("bob"))
        .unwrap();
    let mut frames = 0;
    let mut logged_out_frames = 0;
    let gone = loop {
        driver.poll();
        let now = Instant::now();
        if notice(&driver).is_none() {
            break now;
        }
        frames += 1;
        if phase(&driver) == Phase::LoggedOut {
            logged_out_frames += 1;
        }
        assert!(
            now.duration_since(seen) < ERROR_MESSAGE_TIMEOUT + Duration::from_secs(1),
            "the notice must expire"
        );
        std::thread::park_timeout(FRAME);
    };
    let stood = gone.duration_since(seen);
    println!(
        "PART1 stood={:.3}s frames={frames} logged_out_frames={logged_out_frames} phase={:?}",
        stood.as_secs_f64(),
        phase(&driver)
    );
    assert!(
        stood >= ERROR_MESSAGE_TIMEOUT - Duration::from_millis(100)
            && stood <= ERROR_MESSAGE_TIMEOUT + Duration::from_millis(500),
        "the notice stood {stood:?}, not the shared 15 s"
    );
    assert!(
        logged_out_frames > 0,
        "the notice stood while bob was logged out"
    );
    assert_eq!(phase(&driver), Phase::LoggedOut);
    let history = driver
        .session()
        .fleet_view()
        .detail()
        .and_then(|detail| detail.row().last_error().map(str::to_owned));
    assert!(
        history
            .as_deref()
            .is_some_and(|error| error.contains("refused")),
        "the error stays as history: {history:?}"
    );

    // 2. Log in and let the host retry: each refusal renews the notice.
    driver
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .unwrap();
    let seen = await_notice(&mut driver, Duration::from_secs(10));
    let mut failures = vec![];
    let mut phases = vec![];
    loop {
        driver.poll();
        if let Some(at) = raw_failure(&driver) {
            if failures.last() != Some(&at) {
                failures.push(at);
            }
        }
        let current = phase(&driver);
        if phases.last() != Some(&current) {
            phases.push(current);
        }
        assert!(
            notice(&driver).is_some(),
            "the notice lapsed between real retries at {:.3}s (phases {phases:?})",
            seen.elapsed().as_secs_f64()
        );
        if seen.elapsed() >= ERROR_MESSAGE_TIMEOUT + Duration::from_secs(2) {
            break;
        }
        std::thread::park_timeout(FRAME);
    }
    println!(
        "PART2 failures={} phases={phases:?} stood={:.3}s",
        failures.len(),
        seen.elapsed().as_secs_f64()
    );
    assert!(
        failures.len() >= 2,
        "the host retried and failed again within 17 s ({} failures)",
        failures.len()
    );

    driver
        .submit(OperatorCommand::Logout, Targets::Profile("bob"))
        .unwrap();
    for name in ["alice", "bob"] {
        if let Some(play) = driver.session_mut().play_mut_for_test() {
            play.stop_slot(name);
        }
    }
}

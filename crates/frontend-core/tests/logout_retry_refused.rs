//! LOGOUT-RETRY-1025 live cell `frontend-core.logout-retry-refused`: a real
//! slot worker whose login is refused keeps retrying with backoff until Log
//! out parks it. The endpoint is a local listener that accepts each
//! connection and closes it at once, so the worker's own login fails and is
//! retried the way a refused port fails it, and every attempt is counted on
//! the wire. The worker, its Error publication, its retry backoff, the poll
//! and the shared Log out projection are the product's own; only client
//! asset startup is bypassed.
//!
//! For each path (the single Log out command that the panel button and the
//! TUI key submit, then the Fleet Log out plan):
//! 1. bob's login is refused and retried: at least two attempts arrive, and
//!    Log out is eligible in every state the retry loop passes through.
//! 2. Log out parks bob logged out. Accepts are counted from the moment Log
//!    out is sent, with no settling period: at most one is allowed (an
//!    attempt that had already passed the host's pre-connect check may still
//!    open one socket inside the client connect, and the client closes it
//!    at once: `vendor/fr-client-rust/crates/client/src/io/client_stream.rs:760-768,810-811`),
//!    and none for the rest of a window longer than the next backoff step.
//!
//! alice never logs in and is never eligible.

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use frontend_core::launch::{InstancePermit, PlayConfig};
use frontend_core::{
    FleetAction, HeadlessDriver, LogoutEligibility, OperatorCommand, Phase, Targets,
};
use vault::{Profile, ProfileSettings, Vault};

const FRAME: Duration = Duration::from_millis(20);
/// The window after Log out is sent. It is longer than the host's second
/// retry backoff (about 9 s), so a worker that was still retrying would
/// attempt again inside it.
const QUIET: Duration = Duration::from_secs(20);
/// Accepts allowed inside that window: the one attempt that was already in
/// flight when Log out was sent.
const IN_FLIGHT: usize = 1;

fn endpoint() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            counted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (address, attempts)
}

fn row(driver: &HeadlessDriver, name: &str) -> (Phase, LogoutEligibility) {
    let row = driver
        .session()
        .fleet_view()
        .row(name)
        .unwrap_or_else(|| panic!("{name} has a row"));
    (row.phase(), row.logout_eligibility())
}

/// Poll until `until` holds; every polled state of bob satisfies `each`.
fn poll_until(
    driver: &mut HeadlessDriver,
    deadline: Duration,
    what: &str,
    mut each: impl FnMut(&HeadlessDriver),
    mut until: impl FnMut(&HeadlessDriver) -> bool,
) {
    let start = Instant::now();
    loop {
        driver.poll();
        each(driver);
        if until(driver) {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "{what} (bob {:?})",
            row(driver, "bob")
        );
        std::thread::park_timeout(FRAME);
    }
}

/// Log bob in against the endpoint, let two attempts arrive, check Log out
/// is eligible in every state on the way, then `log_out`; for [`QUIET`]
/// after it is sent at most [`IN_FLIGHT`] attempt arrives.
fn one_path(
    driver: &mut HeadlessDriver,
    attempts: &AtomicUsize,
    path: &str,
    log_out: impl FnOnce(&mut HeadlessDriver),
) {
    let base = attempts.load(Ordering::SeqCst);
    driver
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .unwrap();
    let mut states = vec![];
    poll_until(
        driver,
        Duration::from_secs(40),
        "two refused attempts reach the endpoint",
        |driver| {
            let (phase, eligibility) = row(driver, "bob");
            if states.last() != Some(&phase) {
                states.push(phase);
            }
            assert!(
                eligibility.eligible(),
                "{path}: a bot whose login is retrying must be eligible in {phase:?}"
            );
        },
        |_| attempts.load(Ordering::SeqCst) >= base + 2,
    );
    // The retry loop alternates between its backoff and a new attempt: the
    // phase seen right now is one of its states, and Log out is offered.
    let (phase, eligibility) = row(driver, "bob");
    println!(
        "{path}: attempts={} phases={states:?} now={phase:?}/{eligibility:?}",
        attempts.load(Ordering::SeqCst) - base
    );
    assert!(eligibility.eligible(), "{path}: {phase:?}");
    assert!(
        states.contains(&Phase::LoginError),
        "{path}: the refused login shows as an error between attempts: {states:?}"
    );
    assert_eq!(
        row(driver, "alice").1,
        LogoutEligibility::NotLoggedIn,
        "{path}: alice never logged in"
    );

    // Count accepts from the moment Log out is sent: the window opens before
    // the command, nothing is discarded.
    let at_logout = attempts.load(Ordering::SeqCst);
    let sent = Instant::now();
    log_out(driver);
    let mut parked = false;
    let mut accepted_at = vec![];
    while sent.elapsed() < QUIET {
        driver.poll();
        let (phase, _) = row(driver, "bob");
        parked |= phase == Phase::LoggedOut;
        if parked {
            assert_eq!(phase, Phase::LoggedOut, "{path}: bob stays parked");
        } else {
            assert!(
                sent.elapsed() < Duration::from_secs(10),
                "{path}: bob is parked logged out ({phase:?})"
            );
        }
        let since = attempts.load(Ordering::SeqCst) - at_logout;
        if since > accepted_at.len() {
            accepted_at.push(sent.elapsed());
        }
        assert!(
            since <= IN_FLIGHT,
            "{path}: {since} accepts after Log out was sent (at {accepted_at:?}); \
             at most {IN_FLIGHT} may be the attempt already in flight"
        );
        std::thread::park_timeout(FRAME);
    }
    assert!(parked, "{path}: bob was never parked");
    assert_eq!(row(driver, "bob").1, LogoutEligibility::NotLoggedIn);
    println!(
        "{path}: accepts_after_logout={} at={accepted_at:?} window={}s",
        attempts.load(Ordering::SeqCst) - at_logout,
        QUIET.as_secs()
    );
}

#[test]
#[ignore = "live cell frontend-core.logout-retry-refused: real workers against a local refusing endpoint, about 80 s of wall clock"]
fn log_out_stops_a_login_that_keeps_retrying_for_the_single_command_and_the_fleet_plan() {
    let (address, attempts) = endpoint();
    let dir = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"))
        .join("logout-retry-refused");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "logout-retry-test-passphrase").unwrap();
    for (i, name) in ["alice", "bob"].into_iter().enumerate() {
        vault
            .upsert(Profile {
                username: name.into(),
                password: "test-password".into(),
                uid: 274_102_500 + i as i32,
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
            "logout-retry-test-passphrase",
            &PlayConfig::direct("127.0.0.1", address.port(), "/tmp", true),
        )
        .unwrap();
    driver
        .submit(OperatorCommand::Load, Targets::Everyone)
        .unwrap();
    driver
        .submit(OperatorCommand::Select, Targets::Profile("bob"))
        .unwrap();
    poll_until(
        &mut driver,
        Duration::from_secs(10),
        "bob must finish preparing",
        |_| {},
        |driver| matches!(row(driver, "bob").0, Phase::Offline | Phase::LoggedOut),
    );
    assert_eq!(row(&driver, "bob").1, LogoutEligibility::NotLoggedIn);
    assert_eq!(attempts.load(Ordering::SeqCst), 0, "nothing logs in alone");

    // 1. The single command the panel button and the TUI key submit.
    one_path(&mut driver, &attempts, "single", |driver| {
        driver
            .submit(OperatorCommand::Logout, Targets::Profile("bob"))
            .unwrap();
    });

    // 2. The Fleet Log out plan, over both bots: alice is skipped.
    one_path(&mut driver, &attempts, "fleet", |driver| {
        let planned = driver
            .submit(
                OperatorCommand::PlanFleetAction(FleetAction::Logout),
                Targets::Application,
            )
            .unwrap();
        assert_eq!(
            driver.session().command_failure(planned.request()),
            None,
            "bob has a login to cancel"
        );
        let plan = driver.session().fleet_plan().expect("the plan waits");
        let prompt = plan.prompt().to_owned();
        assert_eq!(plan.target_names().collect::<Vec<_>>(), ["bob"], "{prompt}");
        assert!(prompt.contains("alice: not logged in"), "{prompt}");
        let command = plan.command();
        driver.submit(command, Targets::Plan).unwrap();
    });

    for name in ["alice", "bob"] {
        if let Some(play) = driver.session_mut().play_mut_for_test() {
            play.stop_slot(name);
        }
    }
}

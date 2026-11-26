//! FL-055 deterministic proof: real slot workers driven through
//! `HeadlessDriver` against an ephemeral loopback TCP server that runs the
//! two-stage Java login handshake (hello + seed + full opcode 16/18 login
//! frame) before returning wire refusal codes `6`, `4`, or `7`.
//!
//! Verified invariants:
//! 1. Wire `6` (alice) and wire `4` (bob) stop after their first completed
//!    handshake: `Phase::LoginError`, `retrying == false`, typed
//!    `LoginRefusal`, no FIFO queue place, not logout-eligible, and persistent
//!    host text past the 15 s `ERROR_MESSAGE_TIMEOUT` notice window.
//! 2. Zero additional socket accepts or login frames arrive for alice or bob
//!    during a quiet window longer than both the old 5 s retry delay and the
//!    15 s login-notice timeout.
//! 3. Sibling carol uses the shared login queue normally and still retries
//!    automatically on retryable wire response `7` (world full).
//! 4. An explicit `OperatorCommand::Login` for a stopped bot clears the hold
//!    and produces exactly one new socket attempt and completed login frame,
//!    then stops again with no automatic follow-up.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::lifecycle::LoginRefusal;
use client::util::JString;
use frontend_core::launch::{InstancePermit, PlayConfig};
use frontend_core::{
    HeadlessDriver, LogoutEligibility, OperatorCommand, Phase, Targets, ERROR_MESSAGE_TIMEOUT,
};
use vault::{Profile, ProfileSettings, Vault};

const FRAME: Duration = Duration::from_millis(20);
/// Longer than both the old 5 s retry schedule and the 15 s login notice
/// timeout, so a still-retrying bot would attempt again and a timed notice
/// would expire inside this window.
const PERSIST_WINDOW: Duration = Duration::from_secs(17);
/// Longer than the old 5 s retry delay after an explicit rearm.
const REARM_QUIET_WINDOW: Duration = Duration::from_secs(7);

struct BotCounters {
    hash: u8,
    response: u8,
    accepts: AtomicUsize,
    frames: AtomicUsize,
}

impl BotCounters {
    fn new(name: &str, response: u8) -> Self {
        Self {
            hash: ((JString::to_userhash(name) >> 16) & 0x1f) as u8,
            response,
            accepts: AtomicUsize::new(0),
            frames: AtomicUsize::new(0),
        }
    }
}

struct MockLoginServer {
    addr: SocketAddr,
    alice: Arc<BotCounters>,
    bob: Arc<BotCounters>,
    carol: Arc<BotCounters>,
    shutdown: Arc<AtomicBool>,
}

impl MockLoginServer {
    fn start() -> Self {
        let alice = Arc::new(BotCounters::new("alice", 6));
        let bob = Arc::new(BotCounters::new("bob", 4));
        let carol = Arc::new(BotCounters::new("carol", 7));
        assert_ne!(alice.hash, bob.hash);
        assert_ne!(alice.hash, carol.hash);
        assert_ne!(bob.hash, carol.hash);

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let shutdown = Arc::new(AtomicBool::new(false));
        let bots = [Arc::clone(&alice), Arc::clone(&bob), Arc::clone(&carol)];
        let stop = Arc::clone(&shutdown);

        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let mut hello = [0u8; 2];
                        if stream.read_exact(&mut hello).is_err() {
                            continue;
                        }
                        assert_eq!(hello[0], 14, "expected Java login hello opcode 14");
                        let Some(bot) = bots.iter().find(|b| b.hash == hello[1]) else {
                            panic!("unexpected login hello name hash {}", hello[1]);
                        };
                        bot.accepts.fetch_add(1, Ordering::SeqCst);

                        // Java hello accepted: 8 ignored bytes + 0 + 8-byte server seed.
                        if stream.write_all(&[0u8; 9]).is_err()
                            || stream.write_all(&[0, 0, 0, 0, 0, 0, 0, 1]).is_err()
                        {
                            continue;
                        }

                        // Read complete opcode 16 (fresh) or 18 (reconnect) login frame.
                        let mut frame_hdr = [0u8; 2];
                        if stream.read_exact(&mut frame_hdr).is_err() {
                            continue;
                        }
                        assert!(
                            matches!(frame_hdr[0], 16 | 18),
                            "expected login opcode 16 or 18, got {}",
                            frame_hdr[0]
                        );
                        let mut payload = vec![0u8; frame_hdr[1] as usize];
                        if stream.read_exact(&mut payload).is_err() {
                            continue;
                        }
                        bot.frames.fetch_add(1, Ordering::SeqCst);
                        let _ = stream.write_all(&[bot.response]);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            alice,
            bob,
            carol,
            shutdown,
        }
    }
}

impl Drop for MockLoginServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn poll_until(
    driver: &mut HeadlessDriver,
    deadline: Duration,
    what: &str,
    mut until: impl FnMut(&HeadlessDriver) -> bool,
) {
    let start = Instant::now();
    loop {
        driver.poll();
        if until(driver) {
            return;
        }
        assert!(start.elapsed() < deadline, "timed out waiting for: {what}");
        std::thread::park_timeout(FRAME);
    }
}

fn assert_stopped(driver: &mut HeadlessDriver, name: &str, expected: LoginRefusal) {
    driver
        .submit(OperatorCommand::Select, Targets::Profile(name))
        .unwrap();
    driver.poll();
    let view = driver.session().fleet_view();
    let row = view
        .row(name)
        .unwrap_or_else(|| panic!("{name} row missing"));
    assert_eq!(row.phase(), Phase::LoginError, "{name} phase");
    assert!(!row.retrying(), "{name} must not retry");
    assert_eq!(row.login_refusal(), Some(expected), "{name} typed refusal");
    assert_eq!(row.queue(), None, "{name} must hold no FIFO place");
    assert_eq!(
        row.logout_eligibility(),
        LogoutEligibility::NotLoggedIn,
        "{name} is already stopped"
    );
    assert_eq!(
        row.brief(),
        format!("Login stopped (code {})", expected.code()),
        "{name} brief"
    );
    let detail = view.detail().expect("selected detail");
    assert_eq!(detail.login_refusal(), Some(expected));
    assert_eq!(detail.login_notice_line(), None);
    let state = detail.state();
    assert!(
        state.contains(&format!("Login stopped (code {})", expected.code())),
        "{name} state missing code: {state}"
    );
    assert!(
        state.contains(expected.explanation()),
        "{name} state missing explanation: {state}"
    );
    assert!(
        !state.starts_with("login Login stopped"),
        "{name} state must not double-prefix login: {state}"
    );
}

#[test]
fn terminal_login_refusals_stop_automatic_attempts_while_retryable_sibling_continues() {
    let server = MockLoginServer::start();
    let dir = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"))
        .join("fl-055-login-refusal-stops");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vault_path = dir.join("vault");

    let mut vault = Vault::create(&vault_path, "fl055-test-passphrase").unwrap();
    for (i, name) in ["alice", "bob", "carol"].into_iter().enumerate() {
        let settings = ProfileSettings {
            auto_login: true,
            ..ProfileSettings::default()
        };
        vault
            .upsert(Profile {
                username: name.into(),
                password: "test-password".into(),
                uid: 274_105_500 + i as i32,
                settings,
            })
            .unwrap();
    }
    drop(vault);

    let mut driver = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip())
        .expect("headless driver opens");
    driver.session_mut().set_bypass_asset_startup(true);
    driver.session_mut().set_focus_hint(Some("alice"));
    driver
        .unlock(
            vault_path,
            "fl055-test-passphrase",
            &PlayConfig::direct("127.0.0.1", server.addr.port(), "/tmp", true),
        )
        .unwrap();
    driver
        .submit(OperatorCommand::Load, Targets::Everyone)
        .unwrap();

    // Wait for all three bots to complete their first full two-stage handshake:
    // alice -> 6 (stop), bob -> 4 (stop), carol -> 7 (retryable).
    poll_until(
        &mut driver,
        Duration::from_secs(20),
        "initial refusal for alice(6), bob(4), and first full-world reply for carol(7)",
        |driver| {
            let view = driver.session().fleet_view();
            let alice_stopped = view.row("alice").is_some_and(|r| {
                r.login_refusal() == Some(LoginRefusal::RevisionCacheOrRsaMismatch)
            });
            let bob_stopped = view
                .row("bob")
                .is_some_and(|r| r.login_refusal() == Some(LoginRefusal::AccountBanned));
            alice_stopped
                && bob_stopped
                && server.alice.frames.load(Ordering::SeqCst) == 1
                && server.bob.frames.load(Ordering::SeqCst) == 1
                && server.carol.frames.load(Ordering::SeqCst) >= 1
        },
    );

    assert_stopped(
        &mut driver,
        "alice",
        LoginRefusal::RevisionCacheOrRsaMismatch,
    );
    assert_stopped(&mut driver, "bob", LoginRefusal::AccountBanned);

    // Hold for > 15 s (`ERROR_MESSAGE_TIMEOUT`):
    // - alice and bob must make ZERO additional attempts and keep persistent
    //   stopped diagnostics past the notice timeout;
    // - carol (wire 7) must automatically retry through the shared queue.
    assert!(PERSIST_WINDOW > ERROR_MESSAGE_TIMEOUT);
    let persist_start = Instant::now();
    let mut carol_retried = false;
    while persist_start.elapsed() < PERSIST_WINDOW {
        driver.poll();
        assert_eq!(
            server.alice.accepts.load(Ordering::SeqCst),
            1,
            "alice (code 6) must not open a second socket automatically"
        );
        assert_eq!(
            server.alice.frames.load(Ordering::SeqCst),
            1,
            "alice (code 6) must not send a second login frame automatically"
        );
        assert_eq!(
            server.bob.accepts.load(Ordering::SeqCst),
            1,
            "bob (code 4) must not open a second socket automatically"
        );
        assert_eq!(
            server.bob.frames.load(Ordering::SeqCst),
            1,
            "bob (code 4) must not send a second login frame automatically"
        );
        if !carol_retried && server.carol.frames.load(Ordering::SeqCst) >= 2 {
            carol_retried = true;
            // Park carol once her automatic retry is proven so she does not
            // compete with alice/bob's explicit rearm check below.
            driver
                .submit(OperatorCommand::Logout, Targets::Profile("carol"))
                .unwrap();
        }
        std::thread::park_timeout(FRAME);
    }
    assert!(
        carol_retried,
        "sibling carol (code 7) must retry automatically through the shared queue (frames={})",
        server.carol.frames.load(Ordering::SeqCst)
    );

    // Past the 15 s notice window: alice and bob still show persistent stopped state.
    assert_stopped(
        &mut driver,
        "alice",
        LoginRefusal::RevisionCacheOrRsaMismatch,
    );
    assert_stopped(&mut driver, "bob", LoginRefusal::AccountBanned);

    // Explicit Login re-arms each stopped bot for exactly one new attempt.
    driver
        .submit(OperatorCommand::Login, Targets::Profile("alice"))
        .unwrap();
    driver
        .submit(OperatorCommand::Login, Targets::Profile("bob"))
        .unwrap();
    poll_until(
        &mut driver,
        Duration::from_secs(15),
        "one explicit rearm attempt each for alice and bob",
        |driver| {
            let view = driver.session().fleet_view();
            let alice_stopped = view.row("alice").is_some_and(|r| {
                r.login_refusal() == Some(LoginRefusal::RevisionCacheOrRsaMismatch)
            });
            let bob_stopped = view
                .row("bob")
                .is_some_and(|r| r.login_refusal() == Some(LoginRefusal::AccountBanned));
            alice_stopped
                && bob_stopped
                && server.alice.frames.load(Ordering::SeqCst) == 2
                && server.bob.frames.load(Ordering::SeqCst) == 2
        },
    );

    // Wait longer than the old 5 s retry delay to prove explicit Login armed
    // only one attempt each, not a retry loop.
    let rearm_quiet = Instant::now();
    while rearm_quiet.elapsed() < REARM_QUIET_WINDOW {
        driver.poll();
        assert_eq!(
            server.alice.accepts.load(Ordering::SeqCst),
            2,
            "explicit Login for alice must produce exactly one new socket"
        );
        assert_eq!(
            server.alice.frames.load(Ordering::SeqCst),
            2,
            "explicit Login for alice must produce exactly one new login frame"
        );
        assert_eq!(
            server.bob.accepts.load(Ordering::SeqCst),
            2,
            "explicit Login for bob must produce exactly one new socket"
        );
        assert_eq!(
            server.bob.frames.load(Ordering::SeqCst),
            2,
            "explicit Login for bob must produce exactly one new login frame"
        );
        std::thread::park_timeout(FRAME);
    }

    assert_stopped(
        &mut driver,
        "alice",
        LoginRefusal::RevisionCacheOrRsaMismatch,
    );
    assert_stopped(&mut driver, "bob", LoginRefusal::AccountBanned);

    for name in ["alice", "bob", "carol"] {
        if let Some(play) = driver.session_mut().play_mut_for_test() {
            play.stop_slot(name);
        }
    }
}

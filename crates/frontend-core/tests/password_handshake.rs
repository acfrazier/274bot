//! A saved password reaches an already-running worker's next login. Its own
//! test binary: it points the client's login RSA at a readable pair through
//! the process environment, so no other test shares that environment.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use frontend_core::{ArmMirror, HeadlessSurface, OperatorSession, Outcome};
use host_play::{InstancePermit, PlayOptions};
use vault::{Profile, ProfileSettings, Vault};

/// Fake login server: records `(accepted at, password)` of each handshake,
/// then rejects it (code 3). With RSA exponent 1 the login block arrives as
/// plaintext.
fn password_recording_server() -> (SocketAddr, Receiver<(Instant, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            let accepted = Instant::now();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut preface = [0u8; 2];
            if socket.read_exact(&mut preface).is_err() || preface[0] != 14 {
                continue;
            }
            // 8 ignored bytes, response 0, 8-byte server seed.
            socket.write_all(&[0; 17]).unwrap();
            let mut head = [0u8; 2];
            if socket.read_exact(&mut head).is_err() {
                continue;
            }
            let mut body = vec![0u8; head[1] as usize];
            if socket.read_exact(&mut body).is_err() {
                continue;
            }
            // 255, revision (2), lowmem (1), 9 CRCs (36), block length (1).
            let block = &body[41..];
            // 10, four seed ints, uid, then newline-terminated user and pass.
            let mut strings = block[21..].split(|b| *b == 10);
            let _user = strings.next();
            let password = String::from_utf8_lossy(strings.next().unwrap()).into_owned();
            socket.write_all(&[3]).unwrap();
            if tx.send((accepted, password)).is_err() {
                return;
            }
        }
    });
    (addr, rx)
}

/// Every handshake accepted after `since`, waiting until at least one.
fn handshakes_after(rx: &Receiver<(Instant, String)>, since: Instant) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((at, password)) if at >= since => seen.push(password),
            Ok(_) => {}
            Err(_) if !seen.is_empty() => break,
            Err(_) => {}
        }
    }
    seen
}

#[test]
fn a_parked_worker_logs_in_with_the_password_saved_after_it_spawned() {
    std::env::set_var("LOGIN_RSAN", format!("1{}", "0".repeat(400)));
    std::env::set_var("LOGIN_RSAE", "1");
    let (addr, handshakes) = password_recording_server();
    let play = host_play::run_with_io(
        &PlayOptions {
            host: addr.ip().to_string(),
            port: addr.port(),
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let dir = std::env::temp_dir().join(format!(
        "274bot-frontend-core-password-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "bot").unwrap();
    vault
        .upsert(Profile {
            username: "alice".into(),
            password: "old-pass".into(),
            uid: 7,
            settings: ProfileSettings::default(),
        })
        .unwrap();
    let mut s: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    s.set_bypass_asset_startup(true);
    s.start(vault, play);
    let mut surface = HeadlessSurface::new();
    s.load("alice", &mut surface);
    let arm = s.play().unwrap().arm("alice").unwrap();
    assert!(!arm.wants_login(), "the worker is parked");
    let mut edited = s.durable_profile("alice").unwrap().clone();
    edited.password = "new-pass".into();

    // A failed save leaves the worker's credential alone.
    let blocker = path.with_extension("tmp");
    std::fs::create_dir_all(&blocker).unwrap();
    let op = s
        .save_profile(edited.clone(), ArmMirror::Remember, "credentials")
        .unwrap();
    s.flush_writes();
    std::fs::remove_dir_all(&blocker).unwrap();
    assert!(matches!(
        s.operation(op).unwrap().outcome("alice"),
        Some(Outcome::Failed(_))
    ));
    let since = Instant::now();
    s.login("alice", &mut surface);
    assert_eq!(handshakes_after(&handshakes, since), ["old-pass"]);
    s.logout("alice");

    // A successful save reaches the same worker's next handshake.
    s.save_profile(edited, ArmMirror::Remember, "credentials")
        .unwrap();
    s.flush_writes();
    let since = Instant::now();
    s.login("alice", &mut surface);
    let seen = handshakes_after(&handshakes, since);
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|p| p == "new-pass"),
        "every handshake after the save uses it: {seen:?}"
    );
    s.logout("alice");
    if let Some(play) = s.play_mut() {
        play.stop_slot("alice");
    }
}

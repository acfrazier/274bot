//! The parked memory notice and real login packet must agree on dirty exits.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use frontend_core::{HeadlessSurface, OperatorSession};
use host_play::{InstancePermit, PlayOptions};
use vault::{Profile, ProfileSettings, Vault};

fn wait_for(session: &mut OperatorSession<()>, condition: impl Fn(&OperatorSession<()>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.poll();
        if condition(session) {
            return;
        }
        assert!(Instant::now() < deadline, "worker transition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn dirty_drop_notice_and_explicit_login_preserve_mode_until_clean_logout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let (login_tx, login_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for attempt in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut preface = [0; 2];
            socket.read_exact(&mut preface).unwrap();
            assert_eq!(preface[0], 14);
            socket.write_all(&[0; 17]).unwrap();
            let mut header = [0; 2];
            socket.read_exact(&mut header).unwrap();
            let mut body = vec![0; usize::from(header[1])];
            socket.read_exact(&mut body).unwrap();
            login_tx.send((header[0], body[3])).unwrap();
            socket
                .write_all(if attempt == 1 { &[15] } else { &[2, 0, 0] })
                .unwrap();
            release_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        }
    });
    let clean_logout = Arc::new(AtomicBool::new(false));
    let requested_logout = Arc::clone(&clean_logout);
    let play = host_play::run_with_io(
        &PlayOptions {
            host: endpoint.ip().to_string(),
            transport: host_play::Transport::Tcp,
            port: endpoint.port(),
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        move |client, _, _| {
            if client.ingame {
                client.scene_state = 2;
                if requested_logout.swap(false, Ordering::AcqRel) {
                    let mut packet = client::io::Packet::new(vec![]);
                    client.psize = 0;
                    client.handle_packet(client::io::ServerProt::LOGOUT, &mut packet);
                }
            }
        },
    );
    let dir = std::env::temp_dir().join(format!("274bot-memory-notice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "test-passphrase-01").unwrap();
    vault
        .upsert(Profile {
            username: "alice".into(),
            password: "pw".into(),
            uid: 7,
            settings: ProfileSettings {
                auto_login: false,
                lowmem: true,
                ..ProfileSettings::default()
            },
        })
        .unwrap();
    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.set_bypass_asset_startup(true);
    session.start(vault, play);
    let mut surface = HeadlessSurface::new();
    session.load("alice", &mut surface);
    session.login("alice", &mut surface);
    let first = login_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    wait_for(&mut session, |s| {
        s.memory_status("alice").is_some_and(|n| n.connected)
    });
    session.set_memory_mode("alice", false).unwrap();
    session.flush_writes();

    release_tx.send(()).unwrap(); // Dirty transport drop, not Logout.
    wait_for(&mut session, |s| {
        s.memory_status("alice").is_some_and(|n| !n.connected)
    });
    let dirty = session.memory_status("alice").unwrap();
    let arm = session.play().unwrap().arm("alice").unwrap();
    assert!(!arm.wants_login(), "the dropped one-shot session is parked");
    session.login("alice", &mut surface);
    let reconnect = login_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    wait_for(&mut session, |s| {
        s.memory_status("alice").is_some_and(|n| n.connected)
    });
    let reconnected = session.memory_status("alice").unwrap();

    session.logout("alice");
    clean_logout.store(true, Ordering::Release);
    session.play().unwrap().wake("alice");
    wait_for(&mut session, |s| {
        s.memory_status("alice")
            .is_some_and(|n| !n.connected && n.login_applies_memory)
    });
    let clean = session.memory_status("alice").unwrap();
    release_tx.send(()).unwrap();
    session.login("alice", &mut surface);
    let relog = login_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    wait_for(&mut session, |s| {
        s.memory_status("alice").is_some_and(|n| !n.login_lowmem)
    });
    arm.stop.store(true, Ordering::Relaxed);
    release_tx.send(()).unwrap();
    session.play_mut().unwrap().stop_slot("alice");
    server.join().unwrap();
    drop(session);
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(first, (16, 1));
    assert_eq!(
        reconnect,
        (18, 1),
        "explicit Log in after a dirty drop keeps the applied mode"
    );
    assert_eq!(
        (reconnected.login_lowmem, reconnected.desired_lowmem),
        (true, false)
    );
    assert_eq!(relog, (18, 0), "clean Logout then Log in applies the queue");
    assert!(dirty.differs());
    assert!(!dirty.can_relog());
    assert!(
        !dirty.notice_text().contains("Applies at the next Log in"),
        "{}",
        dirty.notice_text()
    );
    assert!(dirty
        .notice_text()
        .contains("Log in keeps the current mode"));
    assert!(!frontend_core::MemoryNotice::status_text(true, Some(dirty)).contains("next login"));
    assert_eq!(clean.notice_text(), "Applies at the next Log in.");
}

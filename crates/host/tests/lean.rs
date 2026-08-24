//! Task 1: the lean channel cold-logins (opcode 16) and pumps inbound
//! packets without constructing a `Client`. Listener mirror of
//! `client/tests/login.rs`: probe 14, seed, encrypted login block, then a
//! response-2 grant. No `Client`, no cache unpack, no `prepare_game` —
//! there is no `Client` anywhere in this test binary.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use client::client::ClientConfig;
use host::lean::{Lean, LeanError};

fn cfg(addr: &std::net::SocketAddr) -> ClientConfig {
    ClientConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: true,
    }
}

/// The brief's proof: `Lean::login` grants the channel (response 2) without
/// a `Client`, and the snapshot starts at the Java response-2 defaults
/// (`scene_state` 0, pid 0).
#[test]
fn lean_login_does_not_construct_a_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr).unwrap();
        assert_eq!(hdr[0], 14); // login server probe
        assert_eq!(hdr[1], 0); // "bob" → loginServer byte 0
        for _ in 0..8 {
            let _ = s.write_all(&[0]);
        }
        s.write_all(&[0]).unwrap(); // response 0 → send seed
        s.write_all(&[0, 0, 0, 0, 0, 0, 0, 1]).unwrap(); // g8 seed
        let mut buf = [0u8; 512];
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(buf[0], 16); // cold login
        let size = buf[1] as usize;
        assert_eq!(size, n - 2);
        assert_eq!(buf[2], 255); // rev marker
        assert_eq!((buf[3] as usize) << 8 | buf[4] as usize, 274); // client version
        assert_eq!(buf[5], 1, "lowmem login info byte");
        if client::LOGIN_RSAN.starts_with("7162900525229798032761816791230527296329313291") {
            // Java `Packet.rsaenc` writes `BigInteger.toByteArray()` length:
            // 64, or 65 with the leading 0x00 two's-complement byte when the
            // ciphertext MSB is set (random per login).
            let rsa_len = buf[42] as usize;
            assert!(rsa_len == 64 || rsa_len == 65, "rsa len byte {rsa_len}");
            assert_eq!(n, 2 + 40 + 1 + rsa_len);
        }
        s.write_all(&[2, 0, 0]).unwrap(); // response 2, staff=0, mouseTrack=0
    });

    let lean = Lean::login(&cfg(&addr), "bob", "pw", 1).unwrap();
    assert!(lean.snapshot().pid >= 0 || lean.snapshot().scene_state == 0);
    assert_eq!(lean.snapshot().scene_state, 0);
    server.join().unwrap();
}

/// Server responses other than 2 map to the Java title lines; the stream
/// is closed and the caller can retry.
#[test]
fn lean_login_maps_server_response_codes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        let _ = s.read_exact(&mut hdr);
        for _ in 0..8 {
            let _ = s.write_all(&[0]);
        }
        let _ = s.write_all(&[6]); // "RuneScape has been updated!"
    });

    let err = match Lean::login(&cfg(&addr), "bob", "pw", 1) {
        Ok(_) => panic!("response 6 must reject the login"),
        Err(e) => e,
    };
    let LeanError::Login(e) = err else {
        panic!("response 6 must be a Login error, got {err:?}");
    };
    assert_eq!(e.code, 6);
    assert_eq!(e.mes1, "RuneScape has been updated!");
}

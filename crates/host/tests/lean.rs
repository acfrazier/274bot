//! Task 1: the lean channel cold-logins (opcode 16) and pumps inbound
//! packets without constructing a `Client`. Task 3 fix: a reconnect
//! (`reconnect = true`, the 274bot park after a head socket DC) sends
//! wrapper opcode 18 and accepts a response-15 grant. Listener mirror of
//! `client/tests/login.rs`: probe 14, seed, encrypted login block, then
//! the grant. No `Client`, no cache unpack, no `prepare_game` — there is
//! no `Client` anywhere in this test binary.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use client::client::ClientConfig;
use client::io::{Isaac, ServerProt};
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

    let lean = Lean::login(&cfg(&addr), "bob", "pw", 1, false).unwrap();
    assert!(lean.snapshot().pid >= 0 || lean.snapshot().scene_state == 0);
    assert_eq!(lean.snapshot().scene_state, 0);
    server.join().unwrap();
}

/// 274bot park: the head socket dropped (a DC), so the lean reconnect
/// sends wrapper opcode **18** and accepts a response-**15** grant (the
/// same lost_con grant a fat `Client` reconnects with). The snapshot still
/// starts zeroed — a lean channel has no scene to keep.
#[test]
fn lean_login_reconnect_uses_18_and_accepts_15() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr).unwrap();
        assert_eq!(hdr[0], 14); // login server probe
        for _ in 0..8 {
            let _ = s.write_all(&[0]);
        }
        s.write_all(&[0]).unwrap(); // response 0 → send seed
        s.write_all(&[0, 0, 0, 0, 0, 0, 0, 1]).unwrap(); // g8 seed
        let mut buf = [0u8; 512];
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(buf[0], 18); // reconnect wrapper
        s.write_all(&[15]).unwrap(); // reconnect grant
    });

    let lean = Lean::login(&cfg(&addr), "bob", "pw", 1, true).unwrap();
    assert_eq!(lean.snapshot().scene_state, 0);
    server.join().unwrap();
}

/// Server responses other than 2 map to the Java title lines; the stream
/// is closed and the caller can retry. Task 11: code 6 refreshes the login
/// modulus and retries once, so a persistent code 6 needs two responses
/// before the error surfaces.
#[test]
fn lean_login_maps_server_response_codes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for _ in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            let mut hdr = [0u8; 2];
            let _ = s.read_exact(&mut hdr);
            for _ in 0..8 {
                let _ = s.write_all(&[0]);
            }
            let _ = s.write_all(&[6]); // "RuneScape has been updated!"
        }
    });

    let err = match Lean::login(&cfg(&addr), "bob", "pw", 1, false) {
        Ok(_) => panic!("response 6 must reject the login"),
        Err(e) => e,
    };
    let LeanError::Login(e) = err else {
        panic!("response 6 must be a Login error, got {err:?}");
    };
    assert_eq!(e.code, 6);
    assert_eq!(e.mes1, "RuneScape has been updated!");
}

/// The lean tick edge: each inbound `PLAYER_INFO` frame bumps
/// `LeanSnapshot.tick` by one even though the blob is skip-as-seen (no
/// player-list decode yet); the packets a lean channel does apply leave
/// the tick alone. The grant-on-the-probe path never exchanges a seed, so
/// both sides use the zero inbound Isaac (the channel `Lean::login` builds
/// when `response == 2` without a seed round-trip).
#[test]
fn lean_snapshot_tick_counts_player_info_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr).unwrap();
        assert_eq!(hdr[0], 14); // login server probe
        for _ in 0..8 {
            let _ = s.write_all(&[0]);
        }
        s.write_all(&[2]).unwrap(); // grant on the probe: no seed exchange
        s.write_all(&[0, 0]).unwrap(); // staff level, mouse tracking

        let mut enc = Isaac::new(&[0; 4]);
        // UPDATE_PID (133, size 3): pid 7.
        let frame = vec![ServerProt::UPDATE_PID.wrapping_add(enc.next_int()) as u8, 0, 7, 1];
        s.write_all(&frame).unwrap();
        // REBUILD_NORMAL (231, size 4): zone (48, 49).
        let frame = vec![
            ServerProt::REBUILD_NORMAL.wrapping_add(enc.next_int()) as u8,
            0,
            48,
            0,
            49,
        ];
        s.write_all(&frame).unwrap();
        // PLAYER_INFO (167, size -2): 2-byte length prefix + a 1-byte blob.
        let frame = vec![
            ServerProt::PLAYER_INFO.wrapping_add(enc.next_int()) as u8,
            0,
            1,
            0,
        ];
        s.write_all(&frame).unwrap();
    });

    let mut lean = Lean::login(&cfg(&addr), "bob", "pw", 1, false).unwrap();
    assert_eq!(lean.snapshot().tick, 0, "no frame pumped yet");
    for _ in 0..100 {
        if lean.snapshot().tick == 1 {
            break;
        }
        lean.pump().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(lean.snapshot().tick, 1, "one PLAYER_INFO frame counted");
    assert_eq!(lean.snapshot().pid, 7, "UPDATE_PID still applies");
    assert_eq!(lean.snapshot().scene_state, 1);
    assert_eq!(lean.snapshot().tile_x, 336);
    assert_eq!(lean.snapshot().tile_z, 344);
    server.join().unwrap();
}

//! Task 11: the login-key refresh path the lean channel shares with the
//! fat `Client` — `fetch_login_modulus` against a tiny HTTP/1.0 stub, and
//! the code-6 retry-once in `Lean::login`.

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

/// Read a request line + headers off a stub socket and close our side, so
/// the client's `read_to_end` sees EOF right after the response.
fn read_request(s: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = s.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    buf
}

/// `/loginkey` served as a plain decimal body (`/^\d{250,}$/`).
#[test]
fn login_key_fetch_reads_plain_loginkey() {
    let key = "9".repeat(300);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stub_key = key.clone();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let req = read_request(&mut s);
        assert!(String::from_utf8_lossy(&req).contains("/loginkey"));
        write!(s, "HTTP/1.0 200 OK\r\n\r\n{stub_key}\r\n").unwrap();
    });
    let got = client::login_rsa::fetch_login_modulus(&addr.ip().to_string(), addr.port(), "http")
        .unwrap();
    assert_eq!(got, key);
    server.join().unwrap();
}

/// `/loginkey` answers garbage; the `/client/client.js` scrape extracts the
/// first ≥250-digit run (b0t.sh's `grep -oE '[0-9]+' | awk length>=250`).
#[test]
fn login_key_fetch_scrapes_client_js_when_loginkey_is_not_plain() {
    let key = "7".repeat(300);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stub_key = key.clone();
    let server = thread::spawn(move || {
        for (i, path) in ["/loginkey", "/client/client.js"].iter().enumerate() {
            let (mut s, _) = listener.accept().unwrap();
            let req = read_request(&mut s);
            assert!(String::from_utf8_lossy(&req).contains(path));
            if i == 0 {
                write!(s, "HTTP/1.0 404 Not Found\r\n\r\n<html>no key here</html>").unwrap();
            } else {
                write!(s, "HTTP/1.0 200 OK\r\n\r\nvar KEY=\"{stub_key}\";").unwrap();
            }
            drop(s); // close the side so the client's read_to_end sees EOF
        }
    });
    let got = client::login_rsa::fetch_login_modulus(&addr.ip().to_string(), addr.port(), "http")
        .unwrap();
    assert_eq!(got, key);
    server.join().unwrap();
}

/// Code 6 retries the handshake once on a fresh connection before giving
/// up, even when the web origin has no key to serve (the local engine's
/// code 6, or a live fetch that failed).
#[test]
fn login_key_lean_retries_once_on_code_6() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        // First handshake: code 6. Second (the retry): full grant.
        let (mut s, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr).unwrap();
        assert_eq!(hdr[0], 14);
        for _ in 0..8 {
            let _ = s.write_all(&[0]);
        }
        s.write_all(&[6]).unwrap();

        let (mut s2, _) = listener.accept().unwrap();
        let mut hdr = [0u8; 2];
        s2.read_exact(&mut hdr).unwrap();
        for _ in 0..8 {
            let _ = s2.write_all(&[0]);
        }
        s2.write_all(&[0]).unwrap();
        s2.write_all(&[0, 0, 0, 0, 0, 0, 0, 1]).unwrap();
        let mut buf = [0u8; 512];
        let _ = s2.read(&mut buf).unwrap();
        s2.write_all(&[2, 0, 0]).unwrap(); // response 2, staff=0, mouseTrack=0
    });

    let lean = Lean::login(&cfg(&addr), "bob", "pw", 1, false).unwrap();
    assert_eq!(lean.snapshot().scene_state, 0);
    server.join().unwrap();
}

/// Two consecutive code-6 responses (refresh failed or the new key was
/// rejected too) surface as the Java "RuneScape has been updated!" error.
#[test]
fn login_key_lean_code_6_twice_is_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            let mut hdr = [0u8; 2];
            let _ = s.read_exact(&mut hdr);
            for _ in 0..8 {
                let _ = s.write_all(&[0]);
            }
            let _ = s.write_all(&[6]);
        }
    });

    let err = match Lean::login(&cfg(&addr), "bob", "pw", 1, false) {
        Ok(_) => panic!("double code 6 must reject the login"),
        Err(e) => e,
    };
    let LeanError::Login(e) = err else {
        panic!("response 6 must be a Login error, got {err:?}");
    };
    assert_eq!(e.code, 6);
    assert_eq!(e.mes1, "RuneScape has been updated!");
    server.join().unwrap();
}

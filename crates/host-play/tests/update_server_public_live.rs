//! Read each public update endpoint once without logging in or using a warm cache.

use client::{Client, Transport};

fn public_readers(host: &str) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let root = std::path::PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR is required"),
    )
    .join("public-cache")
    .join(host);
    assert!(!root.exists(), "the public JAG proof requires a cold cache");
    let key = Client::fetch_login_modulus_checked(Transport::Wss, host, 443);
    println!(
        "{host}: login-key digits={:?}",
        key.as_ref().map(String::len)
    );
    let checksums = Client::get_jag_checksums_checked(Transport::Wss, host, 443)
        .expect("the public /crc body must pass its checksum");
    println!("{host}: /crc valid, title CRC={}", checksums[1]);
    let title = Client::get_jag_file_checked(
        Transport::Wss,
        root.to_str().expect("the evidence path is UTF-8"),
        host,
        443,
        "title",
        1,
        &checksums,
    )
    .expect("the public title JAG must pass its CRC");
    assert!(!title.is_empty());
    assert_eq!(
        client::io::Packet::getcrc(&title, 0, title.len()),
        checksums[1]
    );
    assert_eq!(std::fs::read(root.join("title")).unwrap(), title);
    let key = key.expect("the public client.js must provide a complete login key");
    assert_eq!(key.len(), 309);
    assert!(key.bytes().all(|byte| byte.is_ascii_digit()));
    println!(
        "PASS: {host}: login-key digits=309, /crc valid, title bytes={}",
        title.len()
    );
}

#[test]
#[ignore = "contacts the public w1 update server once per endpoint"]
fn public_update_w1() {
    public_readers("w1.rs2b2t.com");
}

#[test]
#[ignore = "contacts the public w2 update server once per endpoint"]
fn public_update_w2() {
    public_readers("w2.rs2b2t.com");
}

#[test]
#[ignore = "contacts the public w3 update server once per endpoint"]
fn public_update_w3() {
    public_readers("w3.rs2b2t.com");
}

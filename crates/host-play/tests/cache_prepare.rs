//! The host's ordered cache preparation on the normal profile path: a
//! genuinely empty cache directory is fetched through the real update-server
//! `/crc` + `getJagFile` machinery *before* anything reads its version, the
//! missing local store is reported as the remaining failure (the bind path has
//! no OnDemand worker yet), and the profile still binds and loads its shared
//! template from the fetched packs — the OnDemand fallback below is untouched.
//!
//! The packs are the synthetic fixture archives (`tests/fixtures/profile`);
//! this test's cache, unpack and home roots are temporary, never the
//! operator's cache.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use client::io::Packet;
use host_play::cache::CacheAvailability;
use host_play::profile::{ProfileEnvironment, ProfileSelection};
use host_play::progress::ProfileProgressObserver;
use host_play::{ProfileOptions, SharedClientTemplate};

/// Update-server checksum slots 1-8 (`title`=1 .. `sounds`=8), the order the
/// `/crc` body uses.
const JAG_SLOTS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// One test's temporary root, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-host-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile")
}

fn fixture_packs() -> Arc<Vec<(String, Vec<u8>)>> {
    let fixture = fixture_dir();
    Arc::new(
        JAG_SLOTS
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    std::fs::read(fixture.join(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
                )
            })
            .collect(),
    )
}

/// A local-test profile for the fixture packs: assets come from the mock
/// update server, the cache/unpack/home roots are all temporary, and the
/// navigation pack is deliberately absent (as in `session_profile`).
fn selection(root: &TempRoot, revision: &str, asset_port: u16) -> ProfileSelection {
    let manifest = fixture_dir().join(format!("manifest-{revision}.json"));
    let options = ProfileOptions {
        revision: Some(revision.to_string()),
        cache_dir: Some(root.join("cache")),
        unpack_dir: Some(root.join("unpack")),
        cache_manifest: Some(manifest),
        nav_pack: Some(root.join("missing.navpack")),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(asset_port),
        ..Default::default()
    };
    let environment = ProfileEnvironment {
        home: Some(root.0.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..Default::default()
    };
    options.resolve_with_env(None, &environment).unwrap()
}

/// The 9×g4 + hash body the client's `/crc` read expects (slot 0 empty).
fn crc_body(packs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (name, bytes) in packs {
        let slot = JAG_SLOTS
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or_else(|| panic!("{name}: not a jag pack file"))
            + 1;
        checksums[slot] = Packet::getcrc(bytes, 0, bytes.len());
    }
    let mut body = Packet::alloc(0);
    for &checksum in &checksums {
        body.p4(checksum);
    }
    let mut hash = 1234i32;
    for &checksum in &checksums {
        hash = hash.wrapping_shl(1).wrapping_add(checksum);
    }
    body.p4(hash);
    body.data()[..body.pos].to_vec()
}

fn read_http_request(sock: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&request).to_string()
}

fn respond(sock: &mut std::net::TcpStream, body: &[u8]) {
    let response = [
        b"HTTP/1.0 200 OK\r\nContent-Length: ".as_slice(),
        body.len().to_string().as_bytes(),
        b"\r\n\r\n",
        body,
    ]
    .concat();
    let _ = sock.write_all(&response);
}

/// Mock update server: `/crc` plus one `GET /{name}{crc}` response per pack
/// file, each on its own connection (the client's HTTP helper opens one per
/// request). Serves until every pack has been fetched once, then stops
/// accepting, so a further fetch attempt would surface as a failed connection.
fn serve_packs(packs: Arc<Vec<(String, Vec<u8>)>>) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut served = Vec::new();
        while Instant::now() < deadline && served.len() < packs.len() {
            let (mut sock, _) = match listener.accept() {
                Ok(conn) => conn,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("mock update server accept: {e}"),
            };
            sock.set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let request = read_http_request(&mut sock);
            let path = request
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            if path == "/crc" {
                respond(&mut sock, &crc_body(&packs));
                continue;
            }
            let name = packs
                .iter()
                .map(|(name, _)| name.clone())
                .find(|name| path.starts_with(&format!("/{name}")))
                .unwrap_or_else(|| panic!("unexpected update-server path {path}"));
            let bytes = packs
                .iter()
                .find(|(pack, _)| *pack == name)
                .map(|(_, bytes)| bytes.clone())
                .unwrap();
            respond(&mut sock, &bytes);
            served.push(name);
        }
        served
    });
    (port, handle)
}

/// An empty cache directory on the normal profile path: every pack (the
/// versionlist included) is fetched from the update server before the cache
/// version is read, the missing local store is the honest remaining failure at
/// bind time, and the profile still binds and decodes its shared template from
/// the fetched packs.
#[test]
fn empty_cache_is_fetched_through_normal_profile_preparation() {
    let root = TempRoot::new("cache-prepare");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    assert!(
        std::fs::read_dir(&cache).unwrap().next().is_none(),
        "the cache directory must start genuinely empty"
    );

    let packs = fixture_packs();
    let (port, server) = serve_packs(Arc::clone(&packs));
    let selection = selection(&root, "274", port);
    let observer = ProfileProgressObserver::default();

    let profile = selection.bind_with_progress(&observer).unwrap();
    let served = server.join().unwrap();

    let mut seen = served.clone();
    seen.sort();
    let mut want: Vec<String> = JAG_SLOTS.iter().map(|name| (*name).to_string()).collect();
    want.sort();
    assert_eq!(
        seen, want,
        "the empty cache is fetched from the update server, once per pack"
    );
    for (name, bytes) in packs.iter() {
        assert_eq!(
            &std::fs::read(cache.join(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
            bytes,
            "{name} fetched into the cache"
        );
    }
    let degraded = match profile.cache_availability() {
        CacheAvailability::Degraded(reason) => reason.clone(),
        other => panic!("no local store and no worker yet cannot be ready at bind: {other:?}"),
    };
    assert!(
        degraded.contains("local store"),
        "the missing local store is the remaining failure, got: {degraded}"
    );
    assert!(
        !degraded.contains("versionlist unreadable"),
        "the packs must be fetched before the cache version is read: {degraded}"
    );

    // Neither the profile's cache identity nor the shared template depends on
    // a prepared snapshot: both still come from the fetched packs.
    let template =
        SharedClientTemplate::load_with_progress(Arc::clone(&profile), &observer).unwrap();
    template.validate_for_play_with_progress(&observer).unwrap();

    // A second binding repeats neither the fetch (nothing is missing, and the
    // mock server has stopped) nor a per-slot report: the same degraded status
    // instead of a fetch failure.
    let again = selection.bind_with_progress(&observer).unwrap();
    assert!(
        matches!(again.cache_availability(), CacheAvailability::Degraded(reason) if *reason == degraded),
        "a second bind must not re-fetch: {:?}",
        again.cache_availability()
    );
}

/// A manifest declared for another revision is rejected before any pack is
/// fetched: no update-server work for the wrong selection, and no resource
/// is created.
#[test]
fn wrong_revision_manifest_is_rejected_before_any_fetch() {
    let root = TempRoot::new("cache-wrong-revision");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    for name in JAG_SLOTS {
        std::fs::copy(fixture_dir().join(name), cache.join(name)).unwrap();
    }
    // The 289 selection with the 274 identity: the fixture declares the same
    // bytes for both revisions, so only the declared revision can reject it.
    let manifest = fixture_dir().join("manifest-274.json");
    let options = ProfileOptions {
        revision: Some("289".into()),
        cache_dir: Some(cache.clone()),
        unpack_dir: Some(root.join("unpack")),
        cache_manifest: Some(manifest),
        nav_pack: Some(root.join("missing.navpack")),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(9),
        ..Default::default()
    };
    let environment = ProfileEnvironment {
        home: Some(root.0.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..Default::default()
    };
    let selection = options.resolve_with_env(None, &environment).unwrap();
    let error = selection
        .bind_with_progress(&ProfileProgressObserver::default())
        .unwrap_err();
    assert!(
        error.contains("cache/profile mismatch") && error.contains("declared manifest revision"),
        "a wrong-revision manifest is a profile mismatch, got: {error}"
    );
    assert!(!selection.unpack_dir().exists(), "no preparation happened");
}

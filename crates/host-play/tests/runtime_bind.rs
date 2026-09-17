//! Runtime profile binding: the normal application path negotiates `/crc`
//! from the selected asset endpoint BEFORE freezing, materializes missing or
//! CRC-mismatched jags into a runtime-owned directory (the operator's source
//! cache stays read-only), freezes the negotiated transfer CRCs and the
//! decoded content identity, and reports a complete snapshot as Ready — not
//! the offline bind's degraded no-worker state.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use client::content_identity::compute_decoded_content_identity;
use client::io::{JagFile, Packet};
use host_play::profile::{ProfileEnvironment, ProfileSelection};
use host_play::{ProfileOptions, SharedClientTemplate};

impl TempRoot {
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

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

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-runtime-bind-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn g3(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
}

/// One synthetic jag pack: two bzip2-packed members (`data`), the same
/// construction the client's identity fixtures use.
fn jag(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut raw = (files.len() as u16).to_be_bytes().to_vec();
    for (name, bytes) in files {
        raw.extend_from_slice(&JagFile::gen_hash(name).to_be_bytes());
        g3(&mut raw, bytes.len());
        g3(&mut raw, bytes.len());
    }
    for (_, bytes) in files {
        raw.extend_from_slice(bytes);
    }
    let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    enc.write_all(&raw).unwrap();
    let compressed = &enc.finish().unwrap()[4..];
    assert_ne!(raw.len(), compressed.len());
    let mut out = Vec::new();
    g3(&mut out, raw.len());
    g3(&mut out, compressed.len());
    out.extend_from_slice(compressed);
    out
}

/// A pack set whose versionlist carries the four version/crc tables. `crc`
/// marks the crc-table members so two pack sets are transfer-distinct while
/// staying decoded-equal (the retained local/public 289 shape).
fn packs(crc: u32) -> Vec<Vec<u8>> {
    let mut members = Vec::new();
    for prefix in ["model", "anim", "midi", "map"] {
        members.push((format!("{prefix}_version"), vec![0, 1]));
        members.push((format!("{prefix}_crc"), crc.to_be_bytes().to_vec()));
        members.push((format!("{prefix}_index"), vec![0]));
    }
    let members: Vec<_> = members
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.clone()))
        .collect();
    JAG_SLOTS
        .iter()
        .map(|name| {
            if *name == "versionlist" {
                jag(&members)
            } else if matches!(*name, "config" | "interface") {
                std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/profile")
                        .join(name),
                )
                .unwrap()
            } else {
                jag(&[("data", b"content".to_vec())])
            }
        })
        .collect()
}

fn write_packs(dir: &Path, packs: &[Vec<u8>]) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, bytes) in JAG_SLOTS.iter().zip(packs) {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

/// A complete retained snapshot for `packs` (jags + one-record bins + the
/// manifest completion marker), as an earlier preparation would have left it.
fn snapshot(root: &Path, packs: &[Vec<u8>]) -> PathBuf {
    let version = client::unpack::version_hash(&packs[4]);
    let dir = root.join(&version);
    write_packs(&dir, packs);
    let mut manifest = format!(
        "version={version}\ndir={}\nsource=update-server\ncomplete=1\n",
        dir.display()
    );
    for (name, bytes) in JAG_SLOTS.iter().zip(packs) {
        manifest += &format!("jag.{name}.bytes={}\n", bytes.len());
    }
    for name in ["models", "anims", "midi", "maps"] {
        let mut bin = 0u32.to_le_bytes().to_vec();
        bin.extend_from_slice(&4u32.to_le_bytes());
        bin.extend_from_slice(b"body");
        std::fs::write(dir.join(format!("{name}.bin")), &bin).unwrap();
        manifest += &format!(
            "{name}.total=1\n{name}.unpacked=1\n{name}.skipped=0\n{name}.bytes={}\n",
            bin.len()
        );
    }
    std::fs::write(dir.join("manifest"), manifest).unwrap();
    dir
}

fn crc_body(packs: &[Vec<u8>]) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (slot, bytes) in packs.iter().enumerate() {
        checksums[slot + 1] = Packet::getcrc(bytes, 0, bytes.len());
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

/// Mock update server: `/crc` once, then one `GET /{name}{crc}` per served
/// pack, each on its own connection.
fn serve_packs(packs: Vec<Vec<u8>>, downloads: usize) -> (u16, thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let body = crc_body(&packs);
        for _ in 0..=downloads {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8];
            while !request.ends_with(b"\r\n\r\n") {
                sock.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_string();
            let body = if path == "/crc" {
                body.clone()
            } else {
                let index = JAG_SLOTS
                    .iter()
                    .position(|name| path.starts_with(&format!("/{name}")))
                    .unwrap();
                packs[index].clone()
            };
            let response = [
                b"HTTP/1.0 200 OK\r\nContent-Length: ".as_slice(),
                body.len().to_string().as_bytes(),
                b"\r\n\r\n",
                &body,
            ]
            .concat();
            sock.write_all(&response).unwrap();
        }
    });
    (port, handle)
}

fn selection(root: &TempRoot, revision: &str, asset_port: u16) -> ProfileSelection {
    let options = ProfileOptions {
        revision: Some(revision.to_string()),
        cache_dir: Some(root.join("cache")),
        unpack_dir: Some(root.join("unpack")),
        nav_pack: Some(root.join("missing.navpack")),
        cache_manifest: Some(root.join("cache-manifest.json")),
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

/// Local packed CRCs differ from the endpoint's; the negotiated packs are
/// refreshed into a runtime-owned directory, the operator cache is untouched,
/// the frozen expected CRC is the server array, and the content id is the
/// decoded identity of the negotiated assets.
#[test]
fn runtime_bind_negotiates_and_freezes_server_identity() {
    let root = TempRoot::new("negotiate");
    let cache = root.join("cache");
    let local = packs(1);
    let public = packs(2);
    write_packs(&cache, &local);
    // Earlier complete snapshot, independently keyed by negotiated transfer.
    let retained = snapshot(&root.join("unpack"), &public);
    let expected_identity = compute_decoded_content_identity(274, &retained, &retained).unwrap();

    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(274, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let (port, server) = serve_packs(public.clone(), 1);
    let selection = selection(&root, "274", port);
    let profile = selection.bind_runtime().unwrap();
    server.join().unwrap();

    for (slot, bytes) in local.iter().enumerate() {
        assert_eq!(
            std::fs::read(cache.join(JAG_SLOTS[slot])).unwrap(),
            *bytes,
            "operator {} must remain byte-identical",
            JAG_SLOTS[slot]
        );
    }
    let owned = profile.client().cache_dir().to_path_buf();
    assert_ne!(
        owned, cache,
        "the runtime cache must be a runtime-owned directory, not the operator cache"
    );
    for (slot, bytes) in public.iter().enumerate() {
        assert_eq!(
            std::fs::read(owned.join(JAG_SLOTS[slot])).unwrap(),
            *bytes,
            "{} negotiated into the owned cache",
            JAG_SLOTS[slot]
        );
    }
    let mut negotiated = [0i32; 9];
    for (slot, bytes) in public.iter().enumerate() {
        negotiated[slot + 1] = Packet::getcrc(bytes, 0, bytes.len());
    }
    assert_eq!(profile.client().expected_crc(), Some(negotiated));
    assert_eq!(profile.cache_id(), expected_identity.content_id_hex());
    assert!(profile.game_data().is_none(), "an endpoint override has no built-in server facts");
    let second_bot_profile = Arc::clone(&profile);
    assert!(Arc::ptr_eq(profile.prepared_cache().unwrap(), second_bot_profile.prepared_cache().unwrap()));
    drop(second_bot_profile);
    assert!(matches!(
        profile.cache_availability(),
        host_play::cache::CacheAvailability::Ready { .. }
    ));

    // The bound profile is usable: resources validate against the frozen
    // transfer identity and the shared template decodes from the owned cache.
    profile.validate_resources().unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    template.validate_for_play().unwrap();
    drop(template);
    drop(profile);
    assert!(!owned.exists(), "the owned runtime cache is cleaned up");
}

#[test]
fn runtime_external_nav_requires_actual_selected_source_provenance() {
    let root = TempRoot::new("nav-source");
    let p = packs(1);
    write_packs(&root.join("cache"), &p);
    let retained = snapshot(&root.join("unpack"), &p);
    let cache = host_play::profile::CacheManifest::capture(289, &root.join("cache")).unwrap();
    std::fs::write(root.join("cache-manifest.json"), serde_json::to_vec(&cache).unwrap()).unwrap();
    let content = root.join("content");
    std::fs::create_dir(&content).unwrap();
    std::fs::write(content.join("world"), b"world-a").unwrap();
    let (walk, blocked) = nav::collision::pack_walk(&[0; 8]);
    let bytes = nav::pack::encode(&nav::collision::WorldCollision {
        origin: api::snapshot::WorldTile { x: 3200, z: 3200, level: 0 },
        width: 2, height: 1, walk, blocked, flags: None,
    }, &nav::transport::TransportGraph::default(), &[]);
    let pack = root.join("world.navpack");
    std::fs::write(&pack, &bytes).unwrap();
    let mut manifest = host_play::profile::NavManifest::capture(289, &cache, &bytes, None, None, None).unwrap();
    manifest.content_id = Some(compute_decoded_content_identity(289, &retained, &retained).unwrap().content_id_hex());
    manifest.source_sha256 = Some(nav::bundle::source_digest(&content, &[&root.join("cache/config")]).unwrap());
    std::fs::write(host_play::profile::nav_manifest_path(&pack), serde_json::to_vec(&manifest).unwrap()).unwrap();
    for changed in [false, true] {
        if changed { std::fs::write(content.join("world"), b"world-b").unwrap(); }
        let (port, server) = serve_packs(p.clone(), 0);
        let options = ProfileOptions {
            revision: Some("289".into()), cache_dir: Some(root.join("cache")),
            cache_manifest: Some(root.join("cache-manifest.json")), unpack_dir: Some(root.join("unpack")),
            nav_pack: Some(pack.clone()), content_dir: Some(content.clone()), http_port: Some(port),
            ..Default::default()
        };
        let env = ProfileEnvironment { home: Some(root.0.clone()), rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()), rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()), ..Default::default() };
        let result = options.resolve_with_env(None, &env).unwrap().bind_runtime();
        server.join().unwrap();
        if changed {
            assert!(result.unwrap_err().contains("source provenance differs"));
        } else {
            assert!(result.unwrap().world().is_some());
        }
    }
}

/// The offline explicit bind keeps its semantics: no `/crc` when every pack is
/// present, and a missing snapshot stays an honest degraded report instead of
/// a fabricated Ready.
#[test]
fn offline_bind_freezes_local_files_without_network() {
    let root = TempRoot::new("offline");
    let cache = root.join("cache");
    let local = packs(1);
    write_packs(&cache, &local);
    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(274, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let selection = selection(&root, "274", 9);
    let profile = selection.bind().unwrap();
    assert!(
        selection.bind_runtime().is_err(),
        "runtime preparation must contact its endpoint"
    );
    assert!(matches!(
        profile.cache_availability(),
        host_play::cache::CacheAvailability::Degraded(_)
    ));
}

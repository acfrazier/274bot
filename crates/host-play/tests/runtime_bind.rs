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
        Self::under(&std::env::temp_dir(), name)
    }

    /// On tmpfs where the host has one, so step timings measure the steps,
    /// not the disk's fsync latency.
    fn in_memory(name: &str) -> Self {
        let shm = Path::new("/dev/shm");
        if shm.is_dir() {
            Self::under(shm, name)
        } else {
            Self::new(name)
        }
    }

    fn under(base: &Path, name: &str) -> Self {
        let path = base.join(format!(
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

/// Seed retained inputs through the real verified publisher, not a size-only
/// marker. These synthetic records isolate profile binding from a game server.
fn snapshot(root: &Path, revision: u16, packs: &[Vec<u8>]) -> PathBuf {
    struct FixtureEntries(Vec<(i32, i32)>);
    impl client::unpack::EntrySource for FixtureEntries {
        fn request_entry(&mut self, archive: i32, file: i32) -> Result<(), String> {
            self.0.push((archive, file));
            Ok(())
        }

        fn take_entries(
            &mut self,
            _until: std::time::Instant,
            out: &mut Vec<client::unpack::FetchedEntry>,
        ) -> Result<(), String> {
            out.extend(
                self.0
                    .drain(..)
                    .map(|(archive, file)| client::unpack::FetchedEntry {
                        archive,
                        file,
                        payload: b"body".to_vec(),
                    }),
            );
            Ok(())
        }
    }
    let transfer = nav_planner::manifest::hash_bytes(&packs[4]);
    let negotiated = nav_planner::manifest::hash_bytes(&crc_body(packs)[..36]);
    let input = root.join("fixture-packs");
    write_packs(&input, packs);
    let out = root
        .join(format!("revision-{revision}"))
        .join(&negotiated)
        .join(&transfer);
    let manifest = client::unpack::fetch_snapshot(
        &input.to_string_lossy(),
        &out.to_string_lossy(),
        &mut FixtureEntries(Vec::new()),
    )
    .unwrap();
    PathBuf::from(manifest.dir)
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
        engine_dir: Some(root.join("engine")),
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
    let retained = snapshot(&root.join("unpack"), 274, &public);
    let expected_identity = compute_decoded_content_identity(274, &retained, &retained).unwrap();

    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(274, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let (port, server) = serve_packs(public.clone(), 0);
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
    assert!(
        profile.game_data().is_none(),
        "an endpoint override has no built-in server facts"
    );
    let second_bot_profile = Arc::clone(&profile);
    assert!(Arc::ptr_eq(
        profile.prepared_cache().unwrap(),
        second_bot_profile.prepared_cache().unwrap()
    ));
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

/// A same-revision `--cache-manifest` must still authenticate the operator's
/// source cache. Negotiation may replace runtime copies; empty/wrong archives
/// must not bind.
#[test]
fn runtime_bind_refuses_wrong_archive_same_revision_manifest() {
    let root = TempRoot::new("wrong-manifest");
    let cache = root.join("cache");
    let local = packs(1);
    write_packs(&cache, &local);
    snapshot(&root.join("unpack"), 274, &local);

    let wrong = host_play::profile::CacheManifest {
        revision: 274,
        archives: Default::default(),
    };
    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&wrong).unwrap(),
    )
    .unwrap();
    let error = selection(&root, "274", 9).bind_runtime().unwrap_err();
    assert!(
        error.contains("cache manifest") || error.contains("cache bytes"),
        "{error}"
    );

    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(274, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let (port, server) = serve_packs(local, 0);
    let profile = selection(&root, "274", port).bind_runtime().unwrap();
    server.join().unwrap();
    assert!(profile.client().cache_dir() != cache);
}

#[test]
fn runtime_external_nav_binds_without_rehashing_selected_content() {
    let root = TempRoot::new("nav-source");
    let p = packs(1);
    write_packs(&root.join("cache"), &p);
    let retained = snapshot(&root.join("unpack"), 289, &p);
    let cache = host_play::profile::CacheManifest::capture(289, &root.join("cache")).unwrap();
    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
    let content = root.join("content");
    std::fs::create_dir(&content).unwrap();
    std::fs::write(content.join("world"), b"world-a").unwrap();
    let (walk, blocked) = nav_planner::collision::pack_walk(&[0; 8]);
    let bytes = nav_bake::pack::encode(
        &nav_planner::collision::WorldCollision {
            origin: api::snapshot::WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        },
        &nav_planner::transport::TransportGraph::default(),
        &[],
    )
    .unwrap();
    let pack = root.join("world.navpack");
    std::fs::write(&pack, &bytes).unwrap();
    let mut manifest =
        nav_bake::manifest::capture_nav_manifest(289, &cache, &bytes, None, None, None, None, 0, 0)
            .unwrap();
    manifest.content_id = Some(
        compute_decoded_content_identity(289, &retained, &retained)
            .unwrap()
            .content_id_hex(),
    );
    manifest.source_sha256 =
        Some(nav_bake::bundle::source_digest(&content, &[&root.join("cache/config")]).unwrap());
    std::fs::write(
        host_play::profile::nav_manifest_path(&pack),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(content.join("world"), b"world-b").unwrap();
    let (port, server) = serve_packs(p.clone(), 0);
    let options = ProfileOptions {
        revision: Some("289".into()),
        cache_dir: Some(root.join("cache")),
        cache_manifest: Some(root.join("cache-manifest.json")),
        unpack_dir: Some(root.join("unpack")),
        nav_pack: Some(pack.clone()),
        content_dir: Some(content.clone()),
        http_port: Some(port),
        ..Default::default()
    };
    let env = ProfileEnvironment {
        home: Some(root.0.clone()),
        engine_dir: Some(root.0.join("engine")),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..Default::default()
    };
    let result = options.resolve_with_env(None, &env).unwrap().bind_runtime();
    server.join().unwrap();
    assert!(result.unwrap().world().is_some());
    let sidecar = host_play::profile::nav_manifest_path(&pack);
    manifest.zone_count = 1;
    std::fs::write(&sidecar, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (port, server) = serve_packs(p.clone(), 0);
    let mismatched = ProfileOptions {
        http_port: Some(port),
        ..options.clone()
    };
    let result = mismatched
        .resolve_with_env(None, &env)
        .unwrap()
        .bind_runtime();
    server.join().unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a mismatched decoded zone count must refuse the sidecar"),
    };
    assert!(
        error.contains("zone counts differ from decoded zone table"),
        "{error}"
    );
    manifest.zone_count = 0;
    manifest.zone_npc_count = 1;
    std::fs::write(&sidecar, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (port, server) = serve_packs(p, 0);
    let mismatched = ProfileOptions {
        http_port: Some(port),
        ..options
    };
    let result = mismatched
        .resolve_with_env(None, &env)
        .unwrap()
        .bind_runtime();
    server.join().unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a mismatched decoded NPC zone count must refuse the sidecar"),
    };
    assert!(
        error.contains("zone counts differ from decoded zone table"),
        "{error}"
    );
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

/// A retained public snapshot behind a local cache whose manifest is
/// captured; returns the public packs and the namespace's preparation lock.
fn stepped_fixture(name: &str) -> (TempRoot, Vec<Vec<u8>>, PathBuf) {
    let root = TempRoot::in_memory(name);
    let cache = root.join("cache");
    write_packs(&cache, &packs(1));
    let public = packs(2);
    snapshot(&root.join("unpack"), 274, &public);
    std::fs::write(
        root.join("cache-manifest.json"),
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(274, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let negotiated = nav_planner::manifest::hash_bytes(&crc_body(&public)[..36]);
    let lock = root
        .join("unpack")
        .join("revision-274")
        .join(format!(".{negotiated}.lock"));
    (root, public, lock)
}

/// Another preparation of the same namespace holds the lock for `held`.
/// `releasing_at` records the instant just before the holder unlocks, so a
/// test can tell the steps that started while the lock was held from the
/// work that runs after it.
struct LockHold {
    releasing_at: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    thread: thread::JoinHandle<()>,
}

impl LockHold {
    /// `None` while the lock is still held.
    fn releasing_at(&self) -> Option<std::time::Instant> {
        *self.releasing_at.lock().unwrap()
    }

    fn join(self) {
        self.thread.join().unwrap();
    }
}

fn hold_lock(path: &Path, held: std::time::Duration) -> LockHold {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.lock().unwrap();
    let releasing_at = Arc::new(std::sync::Mutex::new(None));
    let record = Arc::clone(&releasing_at);
    let thread = thread::spawn(move || {
        thread::sleep(held);
        // Recorded before the unlock: a step that starts after this instant
        // is never mistaken for a step that waited on the lock.
        *record.lock().unwrap() = Some(std::time::Instant::now());
        file.unlock().unwrap();
    });
    LockHold {
        releasing_at,
        thread,
    }
}

/// While another preparation holds the cache lock, every step that waits on
/// the lock returns by its own deadline, never after one long wait, and the
/// preparation binds once the lock is free.
///
/// The explicit `step` deadline bounds the waits only: `step` does "the next
/// bounded unit. Network and lock waits end at `until`" (see
/// `TemplatePreparation::step` and `TEMPLATE_STEP_WAIT`). The bind and load
/// steps that run after the lock is free do no waiting, so the contract gives
/// them no deadline; they are excluded from the wait bound. The root is on
/// tmpfs where the host has one: the bound is the steps' own work, not the
/// disk's fsync latency.
#[test]
fn template_preparation_steps_stay_bounded_while_the_cache_lock_is_held() {
    use host_play::progress::ProfileProgressObserver;
    use std::time::{Duration, Instant};
    // A busy host can delay the wakeup of a step that only waited. This slack
    // is much smaller than the hold below, so a step that blocks for the
    // whole hold still fails.
    const SCHEDULING_SLACK: Duration = Duration::from_millis(250);
    let (root, public, lock) = stepped_fixture("stepped-lock");
    let held = Duration::from_millis(1500);
    // The hold starts when the lock is taken, so the total is measured from
    // there too, not from after the server and preparation setup.
    let started = Instant::now();
    let release = hold_lock(&lock, held);
    let (port, server) = serve_packs(public, 0);
    let mut preparation = selection(&root, "274", port).begin_template_preparation();
    let observer = ProfileProgressObserver::default();
    let (mut steps, mut wait_steps) = (0u32, 0u32);
    let (mut worst_wait, mut worst_wait_step) = (Duration::ZERO, 0u32);
    let (mut longest, mut longest_step, mut longest_waited) = (Duration::ZERO, 0u32, false);
    let mut late: Option<(u32, Duration)> = None;
    let template = loop {
        let step_started = Instant::now();
        let step = preparation.step(step_started + host_play::TEMPLATE_STEP_WAIT, &observer);
        let took = step_started.elapsed();
        steps += 1;
        // A step that started before the holder began to release the lock can
        // only have waited on it (or on the network, bounded by the same
        // deadline). Steps that start afterwards do the real work.
        let waited = match release.releasing_at() {
            None => true,
            Some(releasing) => step_started < releasing,
        };
        if waited {
            wait_steps += 1;
            if took > worst_wait {
                worst_wait = took;
                worst_wait_step = steps;
            }
            let deadline = host_play::TEMPLATE_STEP_WAIT + SCHEDULING_SLACK;
            if took > deadline && late.is_none() {
                late = Some((steps, took));
            }
        }
        if took > longest {
            longest = took;
            longest_step = steps;
            longest_waited = waited;
        }
        if let host_play::TemplateStep::Complete(result) = step {
            break result.unwrap();
        }
    };
    let elapsed = started.elapsed();
    release.join();
    server.join().unwrap();
    eprintln!(
        "stepped lock wait: {steps} steps, {wait_steps} waiting, worst waiting \
         {worst_wait:?} at step {worst_wait_step}, longest {longest:?} at step \
         {longest_step} (waited: {longest_waited}), total {elapsed:?}"
    );
    assert!(elapsed >= held, "the lock was honoured: {elapsed:?}");
    if let Some((step, took)) = late {
        panic!(
            "waiting step {step} took {took:?}, past its {:?} deadline plus \
             {SCHEDULING_SLACK:?} scheduling slack",
            host_play::TEMPLATE_STEP_WAIT
        );
    }
    assert!(wait_steps > 50, "{wait_steps} steps waited on the lock");
    template.validate_for_play().unwrap();
}

/// Cancelling the WorkAdmission job ends a preparation that waits for the
/// cache lock at its next step and releases what it holds; a later
/// preparation binds normally.
#[test]
fn cancelled_template_preparation_ends_promptly_and_the_next_one_binds() {
    use host_play::progress::ProfileProgressObserver;
    use script::preparation::CooperativePreparationStep as Step;
    use std::time::{Duration, Instant};
    let (root, public, lock) = stepped_fixture("stepped-cancel");
    let held = Duration::from_millis(3000);
    let release = hold_lock(&lock, held);
    let (port, server) = serve_packs(public.clone(), 0);
    let mut preparation = selection(&root, "274", port).begin_template_preparation();
    let observer = ProfileProgressObserver::default();
    let mut task = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Profile,
        api::identity::Scope::Process,
        script::preparation::PreparationInput::new(1, 0, 0, 0).unwrap(),
        move |worker| match preparation.step_on_worker(
            Instant::now() + host_play::TEMPLATE_STEP_WAIT,
            &observer,
            worker,
        ) {
            host_play::TemplateStep::Continue { completed, total } => {
                Step::Continue { completed, total }
            }
            host_play::TemplateStep::Complete(output) => Step::Complete {
                output: output.map(|_| ()),
                completed: 1,
                total: Some(1),
            },
        },
    )
    .unwrap();
    // `/crc` is served: the preparation now waits for the held lock.
    server.join().unwrap();
    thread::sleep(Duration::from_millis(100));
    let cancelled_at = Instant::now();
    task.cancel(api::identity::Cancellation::Operator);
    while !task.is_finished() {
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(1),
            "cancel did not end the preparation while the lock was held"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        task.join(),
        Err(script::preparation::PreparationError::Cancelled(_))
    ));
    release.join();
    let (port, server) = serve_packs(public, 0);
    let template = selection(&root, "274", port).prepare_template().unwrap();
    server.join().unwrap();
    template.validate_for_play().unwrap();
}

//! The template preparations' cold cache fill and the update servers'
//! name lookups: the world fallback after a failed game-host or asset-host
//! lookup, and the host log of a slow lookup.
//!
//! This test executable defines its own `getaddrinfo`, which the standard
//! library's name lookup calls in place of libc's. [`FAILED_GAME_HOST`]
//! and [`FAILED_ASSET_HOST`] fail at once. [`SLOW_ASSET_HOST`] resolves to
//! 127.0.0.1 after [`LOOKUP_DELAY`]. Every other name goes to libc's
//! `getaddrinfo` unchanged. No system resolver setting changes, and no
//! external server is contacted. The update servers listen on 127.0.0.1
//! only. The HTTPS servers use the client's throwaway `localhost`
//! certificate, which only this test process trusts (`SSL_CERT_FILE`).
#![cfg(target_os = "linux")]

use std::ffi::{c_char, c_int, CStr};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};

use api::hostlog::{Record, Sink};
use client::io::{JagFile, Packet};
use host_play::profile::{ProfileEnvironment, ProfileSelection};
use host_play::progress::{ProfileProgressObserver, ProfileProgressStage};
use host_play::{ProfileOptions, TemplateStep, TEMPLATE_STEP_WAIT};

/// A game host whose lookup fails at once.
const FAILED_GAME_HOST: &str = "no-game.cold-fill-lookups.invalid";
/// An asset host whose lookup fails at once.
const FAILED_ASSET_HOST: &str = "no-asset.cold-fill-lookups.invalid";
/// An asset host whose lookup takes [`LOOKUP_DELAY`].
const SLOW_ASSET_HOST: &str = "slow-asset.cold-fill-lookups.invalid";
const LOOKUP_DELAY: Duration = Duration::from_millis(350);

static FAILED_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
static FAILED_ASSET_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
static SLOW_LOOKUPS: AtomicUsize = AtomicUsize::new(0);

/// The resolver of this test executable (see the module documentation).
///
/// # Safety
///
/// The arguments are those of libc's `getaddrinfo`, and they go to it.
#[no_mangle]
pub unsafe extern "C" fn getaddrinfo(
    node: *const c_char,
    service: *const c_char,
    hints: *const libc::addrinfo,
    res: *mut *mut libc::addrinfo,
) -> c_int {
    type Getaddrinfo = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const libc::addrinfo,
        *mut *mut libc::addrinfo,
    ) -> c_int;
    // SAFETY: libc's `getaddrinfo` has this signature.
    let libc_getaddrinfo: Getaddrinfo = unsafe {
        let symbol = libc::dlsym(libc::RTLD_NEXT, c"getaddrinfo".as_ptr());
        assert!(!symbol.is_null(), "libc getaddrinfo");
        std::mem::transmute::<*mut libc::c_void, Getaddrinfo>(symbol)
    };
    let mut node = node;
    if !node.is_null() {
        // SAFETY: a non-null `node` is a C string (`getaddrinfo` contract).
        let name = unsafe { CStr::from_ptr(node) }.to_bytes();
        let failed = if name == FAILED_GAME_HOST.as_bytes() {
            Some(&FAILED_LOOKUPS)
        } else if name == FAILED_ASSET_HOST.as_bytes() {
            Some(&FAILED_ASSET_LOOKUPS)
        } else {
            None
        };
        if let Some(failed) = failed {
            failed.fetch_add(1, Ordering::Relaxed);
            // SAFETY: `res` is the caller's result slot (`getaddrinfo`
            // contract).
            unsafe { *res = std::ptr::null_mut() };
            return libc::EAI_NONAME;
        }
        if name == SLOW_ASSET_HOST.as_bytes() {
            SLOW_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            thread::sleep(LOOKUP_DELAY);
            node = c"127.0.0.1".as_ptr();
        }
    }
    // SAFETY: the caller's arguments, with `node` a C string.
    unsafe { libc_getaddrinfo(node, service, hints, res) }
}

const CERT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
);
const KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
);
const CA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
);

/// Trust the test certificate. Every test calls this first, so the
/// environment changes once, before any test thread connects or looks up
/// a name.
fn trust_test_certificate() {
    static TRUST: Once = Once::new();
    TRUST.call_once(|| std::env::set_var("SSL_CERT_FILE", CA));
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-cold-fill-lookups-{name}-{}-{}",
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

fn g3(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
}

/// One bzip2-packed JAG pack with these members.
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
    let mut out = Vec::new();
    g3(&mut out, raw.len());
    g3(&mut out, compressed.len());
    out.extend_from_slice(compressed);
    out
}

/// The synthetic profile packs, with a versionlist of one entry in each
/// table, so that the update-server fill has entries to wait for.
fn packs() -> Vec<Vec<u8>> {
    let mut members = Vec::new();
    for prefix in ["model", "anim", "midi", "map"] {
        members.push((format!("{prefix}_version"), vec![0, 1]));
        members.push((format!("{prefix}_crc"), 1u32.to_be_bytes().to_vec()));
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
            } else {
                std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/profile")
                        .join(name),
                )
                .unwrap()
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

/// The `/crc` response for these packs.
fn crc_body(packs: &[Vec<u8>]) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (slot, bytes) in packs.iter().enumerate() {
        checksums[slot + 1] = Packet::getcrc(bytes, 0, bytes.len());
    }
    let mut body = Vec::new();
    let mut hash = 1234i32;
    for checksum in checksums {
        body.extend_from_slice(&checksum.to_be_bytes());
        hash = hash.wrapping_shl(1).wrapping_add(checksum);
    }
    body.extend_from_slice(&hash.to_be_bytes());
    body
}

/// A local update server on 127.0.0.1, HTTPS or plain TCP. It counts
/// connections, answers `GET /crc` and counts WebSocket upgrade requests,
/// which it never answers. Other connections, such as a plain game
/// handshake, are held until the client closes them.
struct UpdateServer {
    port: u16,
    connections: Arc<AtomicUsize>,
    crc: Arc<AtomicUsize>,
    upgrades: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    accept: Option<thread::JoinHandle<()>>,
}

impl UpdateServer {
    fn start(crc_body: &[u8], tls: bool) -> Self {
        let acceptor = tls.then(|| {
            let identity = native_tls::Identity::from_pkcs8(
                &std::fs::read(CERT).unwrap(),
                &std::fs::read(KEY).unwrap(),
            )
            .unwrap();
            Arc::new(native_tls::TlsAcceptor::new(identity).unwrap())
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let crc = Arc::new(AtomicUsize::new(0));
        let upgrades = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let body = Arc::new(crc_body.to_vec());
        let accept = {
            let (connections, stop) = (connections.clone(), stop.clone());
            let (crc, upgrades) = (crc.clone(), upgrades.clone());
            thread::spawn(move || {
                for socket in listener.incoming() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(socket) = socket else { continue };
                    connections.fetch_add(1, Ordering::Relaxed);
                    let (acceptor, body) = (acceptor.clone(), body.clone());
                    let (crc, upgrades) = (crc.clone(), upgrades.clone());
                    thread::spawn(move || serve(socket, acceptor, &body, &crc, &upgrades));
                }
            })
        };
        Self {
            port,
            connections,
            crc,
            upgrades,
            stop,
            accept: Some(accept),
        }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    fn crc_requests(&self) -> usize {
        self.crc.load(Ordering::Relaxed)
    }

    fn upgrade_requests(&self) -> usize {
        self.upgrades.load(Ordering::Relaxed)
    }
}

impl Drop for UpdateServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(accept) = self.accept.take() {
            accept.join().unwrap();
        }
    }
}

fn serve(
    socket: TcpStream,
    acceptor: Option<Arc<native_tls::TlsAcceptor>>,
    body: &[u8],
    crc: &AtomicUsize,
    upgrades: &AtomicUsize,
) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut stream: Box<dyn ReadWrite> = match acceptor {
        Some(acceptor) => match acceptor.accept(socket) {
            Ok(tls) => Box::new(tls),
            Err(_) => return,
        },
        None => Box::new(socket),
    };
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 || stream.read_exact(&mut byte).is_err() {
            // Not an HTTP request: hold it until the client closes it.
            let mut rest = [0u8; 64];
            while matches!(stream.read(&mut rest), Ok(n) if n > 0) {}
            return;
        }
        head.push(byte[0]);
    }
    if head.starts_with(b"GET /crc ") {
        crc.fetch_add(1, Ordering::Relaxed);
        let _ = write!(
            stream,
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
        let _ = stream.flush();
        return;
    }
    if String::from_utf8_lossy(&head)
        .to_ascii_lowercase()
        .contains("upgrade: websocket")
    {
        upgrades.fetch_add(1, Ordering::Relaxed);
    }
    let mut rest = [0u8; 64];
    while matches!(stream.read(&mut rest), Ok(n) if n > 0) {}
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// Counts the preparation attempts: each fill reports `PreparingCache 0/1`
/// when it starts.
fn attempt_counter() -> (ProfileProgressObserver, Arc<AtomicUsize>) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let observer = ProfileProgressObserver::new(move |progress| {
        if progress.stage == ProfileProgressStage::PreparingCache
            && progress.completed == 0
            && progress.total == 1
        {
            counted.fetch_add(1, Ordering::Relaxed);
        }
    });
    (observer, attempts)
}

/// Step `selection`'s template preparation with the host's step deadline
/// until `done` is true, and also measure the time that dropping the
/// preparation takes. `Err` is a preparation that completed before `done`.
fn prepare_until(
    selection: &ProfileSelection,
    observer: &ProfileProgressObserver,
    done: impl Fn() -> bool,
) -> Result<Duration, String> {
    let started = Instant::now();
    let mut preparation = selection.begin_template_preparation();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the preparation stalled"
        );
        if let TemplateStep::Complete(result) =
            preparation.step(Instant::now() + TEMPLATE_STEP_WAIT, observer)
        {
            return Err(format!("completed early: {:?}", result.map(|_| ())));
        }
    }
    let dropped = Instant::now();
    drop(preparation);
    Ok(dropped.elapsed())
}

/// World 1's game host cannot be looked up. World 2 uses the same game
/// host name on another port. The fallback passes over world 2 and moves
/// the fill to world 3 as a whole: world 3's asset endpoint for `/crc` and
/// world 3's game endpoint for the update-server fill. The failed name is
/// looked up once. World 3's asset endpoint differs from its game
/// endpoint, so the fallback must use the world's asset endpoint, not its
/// game endpoint.
#[test]
fn a_failed_game_host_moves_the_fill_to_the_next_world_as_a_whole() {
    trust_test_certificate();
    let packs = packs();
    let body = crc_body(&packs);
    let world_one = UpdateServer::start(&body, true);
    let world_two = UpdateServer::start(&body, true);
    let world_three_assets = UpdateServer::start(&body, true);
    let world_three_game = UpdateServer::start(&body, true);
    let root = TempRoot::new("fallback");
    let selection = wss_selection(
        &root,
        &packs,
        &[
            (
                FAILED_GAME_HOST,
                world_one.port,
                "127.0.0.1",
                world_one.port,
            ),
            (
                FAILED_GAME_HOST,
                world_two.port,
                "127.0.0.1",
                world_two.port,
            ),
            (
                "127.0.0.1",
                world_three_game.port,
                "127.0.0.1",
                world_three_assets.port,
            ),
        ],
    );
    assert_eq!(selection.game_host(), FAILED_GAME_HOST);
    let (observer, attempts) = attempt_counter();
    let failed_before = FAILED_LOOKUPS.load(Ordering::Relaxed);

    // Until the update-server fill connects to world 3's game endpoint.
    let outcome = prepare_until(&selection, &observer, || {
        world_three_game.upgrade_requests() > 0
    });

    let failed = FAILED_LOOKUPS.load(Ordering::Relaxed) - failed_before;
    eprintln!(
        "world fallback: attempts {}; /crc requests: world 1 {}, world 2 {}, \
         world 3 assets {}, world 3 game {}; world 3 game upgrades {}; \
         failed game-host lookups {failed}; outcome {outcome:?}",
        attempts.load(Ordering::Relaxed),
        world_one.crc_requests(),
        world_two.crc_requests(),
        world_three_assets.crc_requests(),
        world_three_game.crc_requests(),
        world_three_game.upgrade_requests(),
    );
    assert_eq!(failed, 1, "the failed game host is looked up once");
    assert_eq!(
        attempts.load(Ordering::Relaxed),
        2,
        "worlds 1 and 3 attempted"
    );
    assert_eq!(world_one.crc_requests(), 1);
    assert_eq!(world_one.upgrade_requests(), 0);
    assert_eq!(world_two.connections(), 0, "world 2 is passed over");
    assert_eq!(
        world_three_assets.crc_requests(),
        1,
        "world 3's asset endpoint"
    );
    assert_eq!(
        world_three_game.crc_requests(),
        0,
        "not world 3's game endpoint"
    );
    let dropped = outcome.unwrap();
    assert!(
        dropped < Duration::from_millis(250),
        "drop took {dropped:?}"
    );
}

/// World 1's update server does not take connections. The fallback moves
/// the fill to world 2 as a whole: the update-server fill connects to
/// world 2's game endpoint, not world 1's.
#[test]
fn an_unreachable_asset_server_moves_the_fill_to_the_next_world_as_a_whole() {
    trust_test_certificate();
    let packs = packs();
    let body = crc_body(&packs);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let world_one_game = UpdateServer::start(&body, true);
    let world_two_assets = UpdateServer::start(&body, true);
    let world_two_game = UpdateServer::start(&body, true);
    let root = TempRoot::new("asset-fallback");
    let selection = wss_selection(
        &root,
        &packs,
        &[
            ("127.0.0.1", world_one_game.port, "127.0.0.1", closed_port),
            (
                "127.0.0.1",
                world_two_game.port,
                "127.0.0.1",
                world_two_assets.port,
            ),
        ],
    );
    let (observer, attempts) = attempt_counter();

    // Until the update-server fill connects to a game endpoint.
    let outcome = prepare_until(&selection, &observer, || {
        world_one_game.connections() + world_two_game.upgrade_requests() > 0
    });

    eprintln!(
        "asset fallback: attempts {}; world 2 /crc requests {}; game connections: \
         world 1 {}, world 2 {} ({} upgrades); outcome {outcome:?}",
        attempts.load(Ordering::Relaxed),
        world_two_assets.crc_requests(),
        world_one_game.connections(),
        world_two_game.connections(),
        world_two_game.upgrade_requests(),
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 2, "two worlds attempted");
    assert_eq!(
        world_two_assets.crc_requests(),
        1,
        "world 2's asset endpoint"
    );
    assert_eq!(
        world_one_game.connections(),
        0,
        "not world 1's game endpoint"
    );
    assert!(
        world_two_game.upgrade_requests() > 0,
        "world 2's game endpoint"
    );
    outcome.unwrap();
}

/// Three worlds use the same asset host name on different ports, and the
/// name cannot be looked up. Each preparation, the stepped one and the
/// one-call one (`bind_runtime_with_progress`), looks the name up once,
/// attempts world 1 only and passes over worlds 2 and 3.
#[test]
fn a_failed_asset_host_is_looked_up_once_in_each_preparation() {
    trust_test_certificate();
    let root = TempRoot::new("failed-asset");
    let selection = wss_selection(
        &root,
        &packs(),
        &[
            ("127.0.0.1", 1, FAILED_ASSET_HOST, 1),
            ("127.0.0.1", 2, FAILED_ASSET_HOST, 2),
            ("127.0.0.1", 3, FAILED_ASSET_HOST, 3),
        ],
    );
    assert_eq!(selection.asset_host(), FAILED_ASSET_HOST);
    let (observer, attempts) = attempt_counter();

    let before = FAILED_ASSET_LOOKUPS.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut preparation = selection.begin_template_preparation();
    let stepped = loop {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the preparation stalled"
        );
        if let TemplateStep::Complete(result) =
            preparation.step(Instant::now() + TEMPLATE_STEP_WAIT, &observer)
        {
            break result.err().expect("the asset host lookup fails");
        }
    };
    drop(preparation);
    let stepped_lookups = FAILED_ASSET_LOOKUPS.load(Ordering::Relaxed) - before;
    let stepped_attempts = attempts.swap(0, Ordering::Relaxed);

    let before = FAILED_ASSET_LOOKUPS.load(Ordering::Relaxed);
    let one_call = selection
        .bind_runtime_with_progress(&observer)
        .expect_err("the asset host lookup fails");
    let one_call_lookups = FAILED_ASSET_LOOKUPS.load(Ordering::Relaxed) - before;
    let one_call_attempts = attempts.load(Ordering::Relaxed);

    eprintln!(
        "failed asset host: stepped: attempts {stepped_attempts}, lookups \
         {stepped_lookups}, error {stepped}; one-call: attempts \
         {one_call_attempts}, lookups {one_call_lookups}, error {one_call}"
    );
    assert_eq!(stepped_lookups, 1, "stepped: the name is looked up once");
    assert_eq!(stepped_attempts, 1, "stepped: world 1 only");
    assert_eq!(one_call_lookups, 1, "one-call: the name is looked up once");
    assert_eq!(one_call_attempts, 1, "one-call: world 1 only");
    assert_eq!(one_call, stepped, "the same error");
}

/// A wss profile with these worlds (game host, game port, asset host,
/// asset port), and its selection. The source cache holds `packs`, so that
/// the fill needs no JAG download but needs the update server for the
/// entries.
fn wss_selection(
    root: &TempRoot,
    packs: &[Vec<u8>],
    worlds: &[(&str, u16, &str, u16)],
) -> ProfileSelection {
    write_packs(&root.join("cache"), packs);
    let bot = root.join(".274bot");
    std::fs::create_dir_all(&bot).unwrap();
    let worlds: Vec<_> = worlds
        .iter()
        .zip(1..)
        .map(|(&(host, port, asset_host, asset_port), number)| {
            serde_json::json!({
                "number": number, "host": host, "port": port, "node_id": number,
                "asset_host": asset_host, "asset_port": asset_port
            })
        })
        .collect();
    let servers = serde_json::json!({
        "schema_version": 1,
        "servers": [{
            "name": "fallback-worlds",
            "revision": 274,
            "transport": "wss",
            "worlds": worlds,
            "login_key": "served",
            "vault": "fallback",
            "members": false
        }]
    });
    std::fs::write(bot.join("servers.json"), servers.to_string()).unwrap();
    ProfileOptions {
        profile: Some("fallback-worlds".into()),
        cache_dir: Some(root.join("cache")),
        unpack_dir: Some(root.join("unpack")),
        nav_pack: Some(root.join("missing.navpack")),
        ..Default::default()
    }
    .resolve_with_env(
        None,
        &ProfileEnvironment {
            home: Some(root.0.clone()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// Every line that reached the installed host-log sink.
static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct LogSink;

impl Sink for LogSink {
    fn record(&self, record: &Record<'_>) {
        LOGGED.lock().unwrap().push(record.message.to_string());
    }
}

static LOG_SINK: LogSink = LogSink;

/// A slow asset-host lookup ends its step after the step deadline. The
/// host logs that once through `host_log!`, so the line reaches the
/// installed host-log sink, with the host name and the lookup time.
#[test]
fn a_slow_name_lookup_reaches_the_host_log() {
    trust_test_certificate();
    assert!(api::hostlog::install_sink(&LOG_SINK));
    let packs = packs();
    let assets = UpdateServer::start(&crc_body(&packs), false);
    let game = UpdateServer::start(&[], false);
    let root = TempRoot::new("slow-lookup");
    write_packs(&root.join("cache"), &packs);
    let selection = ProfileOptions {
        revision: Some("274".into()),
        cache_dir: Some(root.join("cache")),
        unpack_dir: Some(root.join("unpack")),
        nav_pack: Some(root.join("missing.navpack")),
        host: Some("127.0.0.1".into()),
        port: Some(game.port),
        asset_host: Some(SLOW_ASSET_HOST.into()),
        http_port: Some(assets.port),
        ..Default::default()
    }
    .resolve_with_env(
        None,
        &ProfileEnvironment {
            home: Some(root.0.clone()),
            engine_dir: Some(root.join("engine")),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..Default::default()
        },
    )
    .unwrap();
    let (observer, _) = attempt_counter();
    let slow_before = SLOW_LOOKUPS.load(Ordering::Relaxed);

    // Until the update-server fill connects to the game endpoint: every
    // step that uses the asset host is done.
    let outcome = prepare_until(&selection, &observer, || game.connections() > 0);

    let lookups = SLOW_LOOKUPS.load(Ordering::Relaxed) - slow_before;
    let logged: Vec<String> = LOGGED
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains(SLOW_ASSET_HOST))
        .cloned()
        .collect();
    eprintln!("slow lookup: {lookups} lookup(s); host log: {logged:?}; outcome {outcome:?}");
    outcome.unwrap();
    assert_eq!(lookups, 1, "the asset host is looked up once");
    assert_eq!(assets.crc_requests(), 1);
    assert_eq!(logged.len(), 1, "one host-log line for the slow lookup");
    assert!(logged[0].contains("name lookup of the asset host"));
    assert!(logged[0].contains("past the step deadline"));
}

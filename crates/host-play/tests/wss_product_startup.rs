//! Product-path WSS startup proof for G2-M7.
//!
//! The process network service is configured before its first start in this
//! integration-test binary. Each case starts a real Play slot with a bound
//! WSS profile and uses a local rustls/WebSocket peer; no adapter endpoint is
//! constructed by the test.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use api::lifecycle::{AbortedAttempt, SessionEventOut, TransportFailure};
use client::util::JString;
use host_play::profile::ProfileEnvironment;
use host_play::{
    run_with_template, LaunchProfile, LaunchTransport, LaunchWorld, LoginKey, Play, ProfileClass,
    ProfileOptions, Servers, SharedClientTemplate, SlotArm,
};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{CipherSuite, ProtocolVersion, ServerConfig, ServerConnection};
use tungstenite::handshake::derive_accept_key;
use vault::{Profile, ProfileSettings, Vault};

const STARTUP_BOUND: Duration = Duration::from_secs(8);
const SOCKET_BOUND: Duration = Duration::from_secs(2);
const VAULT_PASSPHRASE: &str = "wss-product-startup-fixture";
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct ServerObservation {
    server_name: Option<String>,
    protocol: Option<ProtocolVersion>,
    suite: Option<CipherSuite>,
    tls_error: Option<String>,
    upgrade_request: Option<String>,
    hello: Option<Vec<u8>>,
    websocket_upgrade_complete: bool,
}

struct LocalWssServer {
    address: SocketAddr,
    result: Receiver<io::Result<ServerObservation>>,
    thread: Option<JoinHandle<()>>,
}

impl LocalWssServer {
    fn spawn(config: Arc<ServerConfig>, bind_host: &str, expect_wrong_host: bool) -> Self {
        let listener = TcpListener::bind((bind_host, 0)).expect("bind local WSS fixture address");
        let address = listener
            .local_addr()
            .expect("read local WSS fixture address");
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let server_thread = thread::spawn(move || {
            let _ = result_tx.send(serve_one_wss_connection(
                listener,
                config,
                expect_wrong_host,
            ));
        });
        Self {
            address,
            result: result_rx,
            thread: Some(server_thread),
        }
    }

    fn port(&self) -> u16 {
        self.address.port()
    }

    fn try_observation(&self) -> Result<Option<ServerObservation>, String> {
        match self.result.try_recv() {
            Ok(Ok(observation)) => Ok(Some(observation)),
            Ok(Err(error)) => Err(format!("local WSS fixture failed: {error}")),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err("local WSS fixture exited without an observation".into())
            }
        }
    }

    fn finish(&mut self) {
        if let Some(server_thread) = self.thread.take() {
            server_thread
                .join()
                .expect("local WSS fixture thread did not panic");
        }
    }
}

impl Drop for LocalWssServer {
    fn drop(&mut self) {
        if let Some(server_thread) = self.thread.take() {
            // Wake a blocking accept if a preceding assertion failed before
            // the product reached the listener. The accepted socket has a
            // bounded TLS read timeout, so this cleanup cannot wait forever.
            let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
            let _ = server_thread.join();
        }
    }
}

fn serve_one_wss_connection(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    expect_wrong_host: bool,
) -> io::Result<ServerObservation> {
    let mut fetched_login_key = false;
    let mut wrong_host_failures = 0;
    loop {
        let (mut socket, _) = listener.accept()?;
        socket.set_nodelay(true)?;
        socket.set_read_timeout(Some(SOCKET_BOUND))?;
        socket.set_write_timeout(Some(SOCKET_BOUND))?;

        let mut connection = ServerConnection::new(Arc::clone(&config))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let tls_error = loop {
            if !connection.is_handshaking() {
                break None;
            }
            if let Err(error) = connection.complete_io(&mut socket) {
                break Some(error.to_string());
            }
        };
        let mut observation = ServerObservation {
            server_name: connection.server_name().map(str::to_owned),
            protocol: connection.protocol_version(),
            suite: connection
                .negotiated_cipher_suite()
                .map(|negotiated| negotiated.suite()),
            tls_error,
            upgrade_request: None,
            hello: None,
            websocket_upgrade_complete: false,
        };
        if observation.tls_error.is_some() {
            if expect_wrong_host && wrong_host_failures == 0 {
                wrong_host_failures += 1;
                continue;
            }
            return Ok(observation);
        }

        let mut tls = rustls::StreamOwned::new(connection, socket);
        let request = read_http_head(&mut tls)?;
        let request = String::from_utf8(request)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if request.starts_with("GET /client/client.js HTTP/1.0\r\n") {
            if fetched_login_key {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the product fetched its login modulus more than once",
                ));
            }
            fetched_login_key = true;
            let body = format!("const loginKey = '{}';", client::PROD_LOGIN_RSAN);
            write!(
                tls,
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )?;
            tls.flush()?;
            continue;
        }
        let key = header_value(&request, "Sec-WebSocket-Key")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing WebSocket key"))?;
        let accept = derive_accept_key(key.as_bytes());
        write!(
            tls,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
        )?;
        tls.flush()?;

        // The product emits its game hello only after accepting the 101.
        let hello = read_client_binary_frame(&mut tls)?;
        observation.upgrade_request = Some(request);
        observation.hello = Some(hello);
        observation.websocket_upgrade_complete = true;
        return Ok(observation);
    }
}

fn read_http_head(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() == 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WSS fixture request header exceeded 4096 bytes",
            ));
        }
        reader.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    Ok(head)
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn read_client_binary_frame(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header)?;
    if header[0] & 0x80 == 0 || header[0] & 0x70 != 0 || header[0] & 0x0f != 0x02 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected one final, unextended WebSocket binary frame",
        ));
    }
    if header[1] & 0x80 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client WebSocket frame was not masked",
        ));
    }
    let short_length = u64::from(header[1] & 0x7f);
    let length = match short_length {
        126 => {
            let mut encoded = [0u8; 2];
            reader.read_exact(&mut encoded)?;
            u64::from(u16::from_be_bytes(encoded))
        }
        127 => {
            let mut encoded = [0u8; 8];
            reader.read_exact(&mut encoded)?;
            u64::from_be_bytes(encoded)
        }
        value => value,
    };
    let length = usize::try_from(length)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if length > 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client WebSocket frame exceeded the fixture bound",
        ));
    }
    let mut mask = [0u8; 4];
    reader.read_exact(&mut mask)?;
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % mask.len()];
    }
    Ok(payload)
}

struct ServerIdentity {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

fn checked_in_identity() -> ServerIdentity {
    let certificate = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
    ))
    .expect("parse checked-in RSA localhost certificate");
    let key = PrivateKeyDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
    ))
    .expect("parse checked-in RSA localhost private key");
    ServerIdentity { certificate, key }
}

/// A throwaway CA and a server certificate it signs for the DNS name
/// `localhost` only. The checked-in certificate also names the IP address
/// 127.0.0.1, so it cannot serve as a wrong-host peer on that address; this one
/// can. It lets the wrong-host case use 127.0.0.1, which every system routes
/// to the loopback interface (127.0.0.2 only exists on Linux and Windows, and
/// macOS would need an interface alias that a test must not create).
/// Returns the CA root the client must trust and the server identity.
fn name_only_identity() -> (CertificateDer<'static>, ServerIdentity) {
    let ca_key = KeyPair::generate().expect("generate fixture CA key");
    let mut ca_params = CertificateParams::new(vec!["wss-startup-name-only-ca".to_owned()])
        .expect("create fixture CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = ca_params
        .self_signed(&ca_key)
        .expect("self-sign fixture CA");
    let key = KeyPair::generate().expect("generate name-only server key");
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])
        .expect("create name-only server certificate parameters");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let certificate = params
        .signed_by(&key, &ca, &ca_key)
        .expect("sign name-only server certificate");
    (
        ca.der().clone(),
        ServerIdentity {
            certificate: certificate.der().clone(),
            key: PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        },
    )
}

fn pinned_server_config(
    protocol: ProtocolVersion,
    suite: CipherSuite,
    identity: ServerIdentity,
) -> Arc<ServerConfig> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider
        .cipher_suites
        .retain(|candidate| candidate.suite() == suite);
    assert_eq!(
        provider.cipher_suites.len(),
        1,
        "ring provider exposes the pinned fixture suite"
    );
    let builder = ServerConfig::builder_with_provider(Arc::new(provider));
    let builder = match protocol {
        ProtocolVersion::TLSv1_2 => builder
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("configure TLS 1.2-only fixture server"),
        ProtocolVersion::TLSv1_3 => builder
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("configure TLS 1.3-only fixture server"),
        other => panic!("unsupported fixture TLS protocol: {other:?}"),
    };
    Arc::new(
        builder
            .with_no_client_auth()
            .with_single_cert(vec![identity.certificate], identity.key)
            .expect("configure fixture server identity"),
    )
}

struct StartupFixture {
    root: PathBuf,
    home: PathBuf,
    cache: PathBuf,
}

impl StartupFixture {
    fn new(
        tls12_port: u16,
        tls13_port: u16,
        name_only_port: Option<u16>,
        wrong_host: (&str, u16),
    ) -> Self {
        let root = std::env::temp_dir().join(format!(
            "host-play-wss-product-startup-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let home = root.join("home");
        let bot_dir = home.join(".274bot");
        let cache = root.join("cache");
        std::fs::create_dir_all(&bot_dir).expect("create fixture config directory");
        std::fs::create_dir_all(&cache).expect("create fixture cache directory");

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
        for name in [
            "title",
            "config",
            "interface",
            "media",
            "versionlist",
            "textures",
            "wordenc",
            "sounds",
            "manifest-289.json",
        ] {
            std::fs::copy(source.join(name), cache.join(name))
                .unwrap_or_else(|error| panic!("copy fixture {name}: {error}"));
        }

        let mut profiles = vec![
            launch_profile("wss-startup-tls12", "localhost", tls12_port, "vault-tls12"),
            launch_profile("wss-startup-tls13", "localhost", tls13_port, "vault-tls13"),
        ];
        if let Some(port) = name_only_port {
            profiles.push(launch_profile(
                "wss-startup-name-only",
                "localhost",
                port,
                "vault-name-only",
            ));
        }
        profiles.push(launch_profile(
            "wss-startup-wrong-host",
            wrong_host.0,
            wrong_host.1,
            "vault-wrong-host",
        ));
        let servers = Servers {
            schema_version: 1,
            servers: profiles,
        };
        servers
            .validate()
            .expect("validate local WSS profile fixtures");
        std::fs::write(
            bot_dir.join("servers.json"),
            serde_json::to_vec_pretty(&servers).expect("serialize local WSS profiles"),
        )
        .expect("write local WSS profile fixtures");

        Self { root, home, cache }
    }

    fn template(&self, profile_name: &str, host: &str, port: u16) -> Arc<SharedClientTemplate> {
        let options = ProfileOptions {
            profile: Some(profile_name.to_owned()),
            revision: Some("289".into()),
            host: Some(host.to_owned()),
            port: Some(port),
            cache_dir: Some(self.cache.clone()),
            unpack_dir: Some(self.root.join("unpack")),
            nav_pack: Some(self.root.join("missing.navpack")),
            content_dir: Some(self.root.join("content")),
            cache_manifest: Some(self.cache.join("manifest-289.json")),
            world_members: Some(false),
            ..ProfileOptions::default()
        };
        let environment = ProfileEnvironment {
            home: Some(self.home.clone()),
            ..ProfileEnvironment::default()
        };
        let selection = options
            .resolve_with_env(None, &environment)
            .expect("resolve product WSS endpoint override");
        assert_eq!(selection.transport(), client::Transport::Wss);
        assert_eq!(selection.profile_class(), ProfileClass::Remote);
        assert_eq!(
            selection.selection().worlds[0].number,
            2,
            "endpoint override selected the non-default roster world"
        );
        assert_eq!(selection.game_host(), host);
        assert_eq!(selection.game_port(), port);
        let profile = selection.bind().expect("bind authentic WSS profile");
        assert_eq!(profile.client().game_host(), host);
        assert_eq!(profile.client().game_port(), port);
        SharedClientTemplate::load(profile).expect("load shared product client template")
    }

    fn account(&self, vault_name: &str, username: &str, uid: i32) -> Profile {
        let path = self.home.join(".274bot").join(vault_name);
        {
            let mut vault = Vault::create(&path, VAULT_PASSPHRASE).expect("create fixture vault");
            vault
                .upsert(Profile {
                    username: username.to_owned(),
                    password: format!("{username}-password").into(),
                    uid,
                    settings: ProfileSettings::default(),
                })
                .expect("store fixture account");
        }
        Vault::unlock(&path, VAULT_PASSPHRASE)
            .expect("unlock fixture vault")
            .get(username)
            .expect("read authentic fixture account")
            .clone()
    }
}

impl Drop for StartupFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn launch_profile(name: &str, host: &str, port: u16, vault: &str) -> LaunchProfile {
    LaunchProfile {
        name: name.into(),
        revision: 289,
        transport: LaunchTransport::Wss,
        worlds: vec![
            LaunchWorld {
                number: 1,
                host: "192.0.2.1".into(),
                port: 443,
                node_id: 10,
                asset_host: "192.0.2.1".into(),
                asset_port: 1,
            },
            LaunchWorld {
                number: 2,
                host: host.into(),
                port,
                node_id: 11,
                asset_host: "127.0.0.1".into(),
                asset_port: 1,
            },
        ],
        login_key: LoginKey::Inline {
            modulus: client::PROD_LOGIN_RSAN.into(),
            exponent: client::PROD_LOGIN_RSAE.into(),
        },
        vault: vault.into(),
        members: Some(false),
        allow_plaintext_offhost: false,
    }
}

fn connect_refusal_seen(arm: &SlotArm) -> bool {
    arm.latest_lifecycle_event().is_some_and(|event| {
        matches!(
            event.payload(),
            SessionEventOut::HandshakeAborted {
                reason: AbortedAttempt::Transport(TransportFailure::Connect)
            }
        )
    })
}

fn wait_for_server_observation(
    play: &Play,
    arm: &SlotArm,
    server: &LocalWssServer,
    require_connect_refusal: bool,
    deadline: Instant,
) -> (ServerObservation, bool) {
    let mut observation = None;
    let mut connect_refusal = false;
    loop {
        let now = Instant::now();
        play.maintain_services(now);
        if require_connect_refusal && !connect_refusal {
            connect_refusal = connect_refusal_seen(arm);
        }
        if observation.is_none() {
            observation = server
                .try_observation()
                .unwrap_or_else(|error| panic!("{error}"));
        }
        if let Some(received) = observation.take() {
            if !require_connect_refusal || connect_refusal {
                return (received, connect_refusal);
            }
            observation = Some(received);
        }
        assert!(
            Instant::now() < deadline,
            "product WSS startup did not finish within {STARTUP_BOUND:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn stop_slot_bounded(play: &mut Play, username: &str, deadline: Instant) {
    play.begin_stop_slot(username);
    while play.slot_stopping(username) {
        let now = Instant::now();
        play.maintain_services(now);
        assert!(
            Instant::now() < deadline,
            "product slot did not stop within {STARTUP_BOUND:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    play.reap_stopped_slots();
}

fn run_product_attempt(
    template: Arc<SharedClientTemplate>,
    account: Profile,
    mut server: LocalWssServer,
    expect_connect_refusal: bool,
) -> (ServerObservation, bool) {
    let username = account.username.clone();
    let mut play = run_with_template(template, false, Vec::new(), |_| (None, None), |_, _, _| {})
        .expect("start product Play composition");
    let arm = SlotArm::new(account.uid, true);
    // The fixture has a frozen cache but no asset update server: skip only
    // maininit asset loading, not Play login, TLS handoff, or WSS game I/O.
    arm.bypass_asset_startup_for_test();
    let deadline = Instant::now() + STARTUP_BOUND;
    play.spawn_slot(account, None, None, Some(Arc::clone(&arm)));
    assert!(
        play.statuses()
            .iter()
            .any(|status| status.username == username),
        "the real product Play slot was spawned"
    );

    let (observation, connect_refusal) =
        wait_for_server_observation(&play, &arm, &server, expect_connect_refusal, deadline);
    stop_slot_bounded(&mut play, &username, deadline);
    server.finish();
    (observation, connect_refusal)
}

fn expected_game_hello(username: &str) -> [u8; 2] {
    [14, ((JString::to_userhash(username) >> 16) & 0x1f) as u8]
}

#[test]
fn product_play_wss_startup_negotiates_tls_versions_sends_hello_and_rejects_wrong_host() {
    let root = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
    ))
    .expect("parse checked-in test CA");
    // The wrong-host peer. Linux and Windows route all of 127.0.0.0/8 to the
    // loopback interface, so it listens on 127.0.0.2 with the checked-in
    // certificate, which names localhost and 127.0.0.1 but not that address.
    // macOS routes only 127.0.0.1 unless an interface alias exists, and a test
    // must not create one: there the peer listens on 127.0.0.1 with a
    // throwaway-CA certificate that names only the DNS name localhost (the
    // checked-in one also names 127.0.0.1), so the IP address is the only
    // defect, and a control attempt reaches the same identity by its name.
    let name_only = cfg!(target_os = "macos").then(name_only_identity);
    let mut roots = vec![root];
    if let Some((name_only_root, _)) = &name_only {
        roots.push(name_only_root.clone());
    }
    let client_config = client_adapter::tls_handshake::build_test_client_config_with_roots(&roots)
        .expect("build platform-verifying test client with the fixture CAs");
    client_adapter::service::NetworkService::install_test_tls_config(client_config)
        .expect("install test TLS config before starting the process service");
    let _fixture_driver = client_adapter::service::NetworkService::start_composition()
        .expect("start the process-scoped product network service");

    let tls12_server = LocalWssServer::spawn(
        pinned_server_config(
            ProtocolVersion::TLSv1_2,
            CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            checked_in_identity(),
        ),
        "localhost",
        false,
    );
    let tls13_server = LocalWssServer::spawn(
        pinned_server_config(
            ProtocolVersion::TLSv1_3,
            CipherSuite::TLS13_AES_128_GCM_SHA256,
            checked_in_identity(),
        ),
        "localhost",
        false,
    );
    let (control_identity, wrong_host_identity, wrong_host_address) = match name_only {
        Some((_, identity)) => (
            Some(ServerIdentity {
                certificate: identity.certificate.clone(),
                key: identity.key.clone_key(),
            }),
            identity,
            "127.0.0.1",
        ),
        None => (None, checked_in_identity(), "127.0.0.2"),
    };
    let name_only_server = control_identity.map(|identity| {
        LocalWssServer::spawn(
            pinned_server_config(
                ProtocolVersion::TLSv1_3,
                CipherSuite::TLS13_AES_128_GCM_SHA256,
                identity,
            ),
            "localhost",
            false,
        )
    });
    let wrong_host_server = LocalWssServer::spawn(
        pinned_server_config(
            ProtocolVersion::TLSv1_3,
            CipherSuite::TLS13_AES_128_GCM_SHA256,
            wrong_host_identity,
        ),
        wrong_host_address,
        true,
    );

    let tls12_port = tls12_server.port();
    let tls13_port = tls13_server.port();
    let name_only_port = name_only_server.as_ref().map(LocalWssServer::port);
    let wrong_host_port = wrong_host_server.port();
    let fixture = StartupFixture::new(
        tls12_port,
        tls13_port,
        name_only_port,
        (wrong_host_address, wrong_host_port),
    );
    let tls12_account = fixture.account("vault-tls12", "t1m7tls12", 274_000_112);
    let tls13_account = fixture.account("vault-tls13", "t1m7tls13", 274_000_113);
    let wrong_host_account = fixture.account("vault-wrong-host", "t1m7wrong", 274_000_114);
    let tls12_template = fixture.template("wss-startup-tls12", "localhost", tls12_port);
    let tls13_template = fixture.template("wss-startup-tls13", "localhost", tls13_port);
    let wrong_host_template = fixture.template(
        "wss-startup-wrong-host",
        wrong_host_address,
        wrong_host_port,
    );

    let (tls12, _) =
        run_product_attempt(tls12_template, tls12_account.clone(), tls12_server, false);
    assert!(
        tls12.tls_error.is_none(),
        "TLS 1.2 server handshake succeeds"
    );
    assert_eq!(tls12.server_name.as_deref(), Some("localhost"));
    assert_eq!(tls12.protocol, Some(ProtocolVersion::TLSv1_2));
    assert_eq!(
        tls12.suite,
        Some(CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256)
    );
    assert!(
        tls12.websocket_upgrade_complete,
        "product completed WSS upgrade"
    );
    let tls12_request = tls12
        .upgrade_request
        .as_deref()
        .expect("product sent WebSocket upgrade request");
    let tls12_host = format!("localhost:{tls12_port}");
    assert_eq!(
        header_value(tls12_request, "Host"),
        Some(tls12_host.as_str())
    );
    assert_eq!(header_value(tls12_request, "Upgrade"), Some("websocket"));
    assert_eq!(header_value(tls12_request, "Connection"), Some("Upgrade"));
    assert_eq!(
        header_value(tls12_request, "Sec-WebSocket-Protocol"),
        Some("binary")
    );
    let tls12_hello = expected_game_hello(&tls12_account.username);
    assert_eq!(
        tls12.hello.as_deref(),
        Some(tls12_hello.as_slice()),
        "exact initial game hello follows the real product WebSocket upgrade"
    );

    let (tls13, _) =
        run_product_attempt(tls13_template, tls13_account.clone(), tls13_server, false);
    assert!(
        tls13.tls_error.is_none(),
        "TLS 1.3 server handshake succeeds"
    );
    assert_eq!(tls13.server_name.as_deref(), Some("localhost"));
    assert_eq!(tls13.protocol, Some(ProtocolVersion::TLSv1_3));
    assert_eq!(tls13.suite, Some(CipherSuite::TLS13_AES_128_GCM_SHA256));
    assert!(
        tls13.websocket_upgrade_complete,
        "TLS 1.3 product startup reaches WSS"
    );
    let tls13_request = tls13
        .upgrade_request
        .as_deref()
        .expect("TLS 1.3 product sent WebSocket upgrade request");
    let tls13_host = format!("localhost:{tls13_port}");
    assert_eq!(
        header_value(tls13_request, "Host"),
        Some(tls13_host.as_str())
    );
    assert_eq!(header_value(tls13_request, "Upgrade"), Some("websocket"));
    assert_eq!(header_value(tls13_request, "Connection"), Some("Upgrade"));
    assert_eq!(
        header_value(tls13_request, "Sec-WebSocket-Protocol"),
        Some("binary")
    );
    let tls13_hello = expected_game_hello(&tls13_account.username);
    assert_eq!(
        tls13.hello.as_deref(),
        Some(tls13_hello.as_slice()),
        "exact initial game hello follows TLS 1.3 startup"
    );

    // macOS only: the control for the wrong-host case below. The identical
    // server identity, reached by the DNS name it carries, starts a WSS session.
    if let Some(server) = name_only_server {
        let account = fixture.account("vault-name-only", "t1m7nameo", 274_000_115);
        let template = fixture.template(
            "wss-startup-name-only",
            "localhost",
            name_only_port.expect("name-only control port"),
        );
        let (control, _) = run_product_attempt(template, account.clone(), server, false);
        assert!(
            control.tls_error.is_none(),
            "the name-only identity is accepted by its DNS name"
        );
        assert_eq!(control.server_name.as_deref(), Some("localhost"));
        assert!(
            control.websocket_upgrade_complete,
            "the name-only identity reaches WSS by its DNS name"
        );
        let control_hello = expected_game_hello(&account.username);
        assert_eq!(
            control.hello.as_deref(),
            Some(control_hello.as_slice()),
            "exact initial game hello follows the name-only control startup"
        );
    }

    let (wrong_host, typed_refusal) = run_product_attempt(
        wrong_host_template,
        wrong_host_account,
        wrong_host_server,
        true,
    );
    assert!(
        typed_refusal,
        "wrong-host TLS refusal is published as typed Connect"
    );
    assert!(
        wrong_host.tls_error.is_some(),
        "wrong hostname is rejected in TLS"
    );
    assert_eq!(
        wrong_host.server_name, None,
        "IP endpoint does not send DNS SNI"
    );
    assert!(
        wrong_host.upgrade_request.is_none(),
        "wrong host never upgrades WebSocket"
    );
    assert!(
        wrong_host.hello.is_none(),
        "wrong host never sends a game hello"
    );
}

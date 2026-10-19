#![cfg(all(target_os = "linux", feature = "test-support"))]

//! Isolated Linux native-root tests through the real handshake worker.
//! This Linux-only test does not qualify Windows or macOS G2-M4 behavior.

use std::env;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use client_adapter::cancel::{AttemptRetire, CancelSource};
use client_adapter::control::BindingId;
use client_adapter::handoff::{Accepted, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, WorkerOutcome};
use client_adapter::health::ReactorHealth;
use client_adapter::park::Park;
use client_adapter::shared::SharedConnection;
use client_adapter::tls_handshake::{build_test_client_config_with_roots, GameTlsLane, GameTlsResource};
use mio::{Events, Interest, Poll, Token, Waker};
use rcgen::{BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, ProtocolVersion, ServerConfig, ServerConnection};

const SOCKET: Token = Token(1);
const WAKER: Token = Token(0);
const TIMEOUT: Duration = Duration::from_secs(6);
const CHILD: &str = "rustls_linux_trust::child_case";

fn temp_home() -> PathBuf {
    let path = env::temp_dir().join(format!("linux-trust-{}-{:?}", std::process::id(), thread::current().id()));
    fs::create_dir_all(&path).expect("create child-only HOME");
    path
}

fn invoke(case: &str) {
    let home = temp_home();
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env("RUSTLS_TRUST_CASE", case)
        .env("HOME", &home)
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .output().expect("launch isolated trust case");
    let _ = fs::remove_dir_all(&home);
    assert!(output.status.success(), "diagnostic=linux-trust/{case}; stdout={}; stderr={}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn linux_native_root_matrix_uses_isolated_worker_configs() {
    for version in ["tls12", "tls13"] {
        for trust in ["file", "untrusted-file", "neither", "missing-file", "invalid-file", "extra-root", "wrong-host"] {
            invoke(&format!("{version}-{trust}"));
        }
    }
}

#[test]
fn child_case() {
    let Ok(case) = env::var("RUSTLS_TRUST_CASE") else { return };
    let home = env::var_os("HOME").map(PathBuf::from).unwrap();
    let trusted = make_ca("fixture trusted root");
    let unrelated = make_ca("fixture unrelated root");
    let (trusted_cert, trusted_key) = issue_server(&trusted.0, &trusted.1);
    let trusted_pem = home.join("trusted.pem");
    let unrelated_pem = home.join("unrelated.pem");
    fs::write(&trusted_pem, trusted.0.pem()).unwrap();
    fs::write(&unrelated_pem, unrelated.0.pem()).unwrap();
    let (_, trust_kind) = case.split_once('-').expect("version and trust variant");
    match trust_kind {
        "file" => { env::set_var("SSL_CERT_FILE", &trusted_pem); }
        "untrusted-file" => { env::set_var("SSL_CERT_FILE", &unrelated_pem); }
        "missing-file" => { env::set_var("SSL_CERT_FILE", home.join("missing.pem")); }
        "invalid-file" => { let path = home.join("invalid.pem"); fs::write(&path, b"not PEM").unwrap(); env::set_var("SSL_CERT_FILE", path); }
        "neither" | "extra-root" | "wrong-host" => {}
        _ => panic!("unknown trust case {trust_kind}"),
    }
    let roots = if trust_kind == "extra-root" { vec![trusted.2.clone()] } else { Vec::new() };
    let config = build_test_client_config_with_roots(&roots).expect("construct fresh OS verifier config");
    let protocol = if case.starts_with("tls12-") { &rustls::version::TLS12 } else { &rustls::version::TLS13 };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_config = server_config(trusted_cert, trusted_key, protocol);
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(TIMEOUT)).unwrap();
        socket.set_write_timeout(Some(TIMEOUT)).unwrap();
        let mut conn = ServerConnection::new(server_config).unwrap();
        while conn.is_handshaking() {
            if conn.complete_io(&mut socket).is_err() {
                break;
            }
        }
    });
    let host = if trust_kind == "wrong-host" { "not-localhost.invalid" } else { "localhost" };
    let mut attempt = Attempt::new(address, config, host);
    let accepted = attempt.wait();
    server.join().expect("server handshake thread");
    let trusted_case = matches!(trust_kind, "file" | "extra-root");
    match accepted.outcome {
        WorkerOutcome::Success(resource) => {
            assert!(trusted_case, "diagnostic=unexpected-trust-accept case={case}");
            let conn = resource.connection.as_ref().expect("worker returned TLS state");
            assert!(!conn.is_handshaking(), "worker must complete TLS before transfer");
            assert_eq!(conn.protocol_version(), Some(if case.starts_with("tls12-") { ProtocolVersion::TLSv1_2 } else { ProtocolVersion::TLSv1_3 }));
        }
        WorkerOutcome::Failure(failure) => {
            assert!(!trusted_case, "diagnostic=unexpected-trust-reject case={case}: {failure:?}");
            assert!(matches!(failure, client_adapter::handoff::WorkerFailure::Verification), "diagnostic=wrong-failure-category case={case}: {failure:?}");
        }
    }
}

struct Attempt {
    poll: Poll,
    events: Events,
    lane: Arc<GameTlsLane>,
    key: HandoffKey,
}
impl Attempt {
    fn new(address: std::net::SocketAddr, config: Arc<ClientConfig>, host: &str) -> Self {
        let poll = Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), WAKER).unwrap());
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let binding = BindingId { incarnation: 1, connection: 1, binding: 1, attempt: token.generation() };
        let shared = SharedConnection::new(binding.connection, park);
        let lane = GameTlsLane::start_with_test_config(waker, Arc::new(ReactorHealth::new(Instant::now())), config).unwrap();
        let seq = lane.cell().next_transfer_seq().unwrap();
        let key = HandoffKey { identity: HandoffIdentity { generation: token.generation(), binding }, transfer_seq: seq };
        let proof = Arc::new(QueueReleaseProof::new(binding, seq));
        let retirement = Arc::new(AttemptRetire::new(token.generation()));
        let socket = TcpStream::connect(address).unwrap();
        socket.set_nodelay(true).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut socket = mio::net::TcpStream::from_std(socket);
        poll.registry().register(&mut socket, SOCKET, Interest::READABLE).unwrap();
        let resource = GameTlsResource::new(socket, host.to_owned(), None);
        lane.cell().submit(key, proof, token.clone(), retirement, Instant::now() + TIMEOUT, resource).unwrap();
        let _ = shared;
        Self { poll, events: Events::with_capacity(8), lane, key }
    }
    fn wait(&mut self) -> Accepted<GameTlsResource> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if self.lane.cell().phase() == HandoffPhase::Returned { return self.lane.cell().take_return(self.key).unwrap(); }
            assert!(Instant::now() < deadline, "diagnostic=worker-timeout");
            self.poll.poll(&mut self.events, Some(Duration::from_millis(25))).unwrap();
            for event in &self.events {
                if event.token() == SOCKET && event.is_readable() {
                    self.lane.cell().publish_ready(self.key, client_adapter::handoff::ReadyDirection::Read);
                }
            }
        }
    }
}
impl Drop for Attempt { fn drop(&mut self) { let _ = self.lane.shutdown(); } }

fn make_ca(name: &str) -> (Certificate, KeyPair, CertificateDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![name.into()]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let cert = params.self_signed(&key).unwrap();
    let der = cert.der().clone();
    (cert, key, der)
}
fn issue_server(ca: &Certificate, ca_key: &KeyPair) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let cert = params.signed_by(&key, ca, ca_key).unwrap();
    (cert.der().clone(), PrivateKeyDer::Pkcs8(key.serialize_der().into()))
}
fn server_config(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>, version: &'static rustls::SupportedProtocolVersion) -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider).with_protocol_versions(&[version]).unwrap().with_no_client_auth().with_single_cert(vec![cert], key).unwrap();
    Arc::new(config)
}

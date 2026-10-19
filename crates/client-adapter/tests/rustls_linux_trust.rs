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
use client_adapter::handoff::{
    Accepted, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, WorkerOutcome,
};
use client_adapter::health::ReactorHealth;
use client_adapter::park::Park;
use client_adapter::tls_handshake::{GameTlsLane, GameTlsResource};
use mio::{Events, Interest, Poll, Token, Waker};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ProtocolVersion, ServerConfig, ServerConnection};

const SOCKET: Token = Token(1);
const WAKER: Token = Token(0);
const TIMEOUT: Duration = Duration::from_secs(6);
const CHILD: &str = "child_case";

fn temp_home() -> PathBuf {
    let path = env::temp_dir().join(format!(
        "linux-trust-{}-{:?}",
        std::process::id(),
        thread::current().id()
    ));
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
        .output()
        .expect("launch isolated trust case");
    let _ = fs::remove_dir_all(&home);
    let marker = format!("qualified-linux-trust:{case}:");
    assert!(
        output.status.success()
            && output
                .stdout
                .windows(b"running 1 test".len())
                .any(|window| window == b"running 1 test")
            && output
                .stdout
                .windows(marker.len())
                .any(|window| window == marker.as_bytes()),
        "diagnostic=linux-trust/{case}; child status={}; stdout={}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn linux_native_root_matrix_uses_isolated_worker_configs() {
    for version in ["tls12", "tls13"] {
        for trust in [
            "file",
            "dir",
            "untrusted-file",
            "neither",
            "missing-file",
            "invalid-file",
            "extra-root",
            "native-plus-extra",
            "extra-root-invalid-file",
            "wrong-host",
        ] {
            invoke(&format!("{version}-{trust}"));
        }
    }
}

#[test]
fn child_case() {
    let Ok(case) = env::var("RUSTLS_TRUST_CASE") else {
        return;
    };
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
        "file" | "native-plus-extra" | "wrong-host" => {
            env::set_var("SSL_CERT_FILE", &trusted_pem);
        }
        "dir" => {
            let dir = home.join("trusted-cert-dir");
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("trusted.pem"), trusted.0.pem()).unwrap();
            env::set_var("SSL_CERT_DIR", dir);
        }
        "untrusted-file" => {
            env::set_var("SSL_CERT_FILE", &unrelated_pem);
        }
        "missing-file" => {
            env::set_var("SSL_CERT_FILE", home.join("missing.pem"));
        }
        "invalid-file" | "extra-root-invalid-file" => {
            let path = home.join("invalid.pem");
            fs::write(&path, b"not PEM").unwrap();
            env::set_var("SSL_CERT_FILE", path);
        }
        "neither" | "extra-root" => {}
        _ => panic!("unknown trust case {trust_kind}"),
    }
    let roots = match trust_kind {
        "extra-root" | "extra-root-invalid-file" => vec![trusted.2.clone()],
        "native-plus-extra" => vec![unrelated.2.clone()],
        _ => Vec::new(),
    };
    let protocol = if case.starts_with("tls12-") {
        &rustls::version::TLS12
    } else {
        &rustls::version::TLS13
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let host = if trust_kind == "wrong-host" {
        "not-localhost.invalid"
    } else {
        "localhost"
    };
    let Some(mut attempt) = Attempt::new(address, roots, host) else {
        assert!(
            matches!(trust_kind, "missing-file" | "invalid-file"),
            "diagnostic=unexpected-worker-config-refusal case={case}"
        );
        println!("qualified-linux-trust:{case}:constructor-refusal");
        return;
    };
    assert!(
        !matches!(trust_kind, "missing-file" | "invalid-file"),
        "diagnostic=missing-root-worker-config-accepted case={case}"
    );
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
    let accepted = attempt.wait();
    server.join().expect("server handshake thread");
    let trusted_case = matches!(
        trust_kind,
        "file" | "dir" | "extra-root" | "native-plus-extra" | "extra-root-invalid-file"
    );
    match accepted.outcome {
        WorkerOutcome::Success(resource) => {
            assert!(
                trusted_case,
                "diagnostic=unexpected-trust-accept case={case}"
            );
            let conn = resource
                .connection
                .as_ref()
                .expect("worker returned TLS state");
            assert!(
                !conn.is_handshaking(),
                "worker must complete TLS before transfer"
            );
            assert_eq!(
                conn.protocol_version(),
                Some(if case.starts_with("tls12-") {
                    ProtocolVersion::TLSv1_2
                } else {
                    ProtocolVersion::TLSv1_3
                })
            );
        }
        WorkerOutcome::Failure(failure) => {
            assert!(
                !trusted_case,
                "diagnostic=unexpected-trust-reject case={case}: {failure:?}"
            );
            assert!(
                matches!(
                    failure,
                    client_adapter::handoff::WorkerFailure::Verification
                ),
                "diagnostic=wrong-failure-category case={case}: {failure:?}"
            );
        }
    }
    println!("qualified-linux-trust:{case}:worker-return");
}

struct Attempt {
    poll: Poll,
    events: Events,
    lane: Arc<GameTlsLane>,
    key: HandoffKey,
}
impl Attempt {
    fn new(
        address: std::net::SocketAddr,
        roots: Vec<CertificateDer<'static>>,
        host: &str,
    ) -> Option<Self> {
        let poll = Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), WAKER).unwrap());
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().unwrap();
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: token.generation(),
        };
        let lane = GameTlsLane::start_with_test_roots(
            waker,
            Arc::new(ReactorHealth::new(Instant::now())),
            roots,
        )
        .unwrap();
        lane.config()?;
        let seq = lane.cell().next_transfer_seq().unwrap();
        let key = HandoffKey {
            identity: HandoffIdentity {
                generation: token.generation(),
                binding,
            },
            transfer_seq: seq,
        };
        let proof = Arc::new(QueueReleaseProof::new(binding, seq));
        let retirement = Arc::new(AttemptRetire::new(token.generation()));
        let socket = TcpStream::connect(address).unwrap();
        socket.set_nodelay(true).unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut socket = mio::net::TcpStream::from_std(socket);
        poll.registry()
            .register(&mut socket, SOCKET, Interest::READABLE)
            .unwrap();
        let resource = GameTlsResource::new(socket, host.to_owned(), None);
        lane.cell()
            .submit(
                key,
                proof,
                token.clone(),
                retirement,
                Instant::now() + TIMEOUT,
                resource,
            )
            .unwrap();
        Some(Self {
            poll,
            events: Events::with_capacity(8),
            lane,
            key,
        })
    }
    fn wait(&mut self) -> Accepted<GameTlsResource> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if self.lane.cell().phase() == HandoffPhase::Returned {
                return self.lane.cell().take_return(self.key).unwrap();
            }
            assert!(Instant::now() < deadline, "diagnostic=worker-timeout");
            self.poll
                .poll(&mut self.events, Some(Duration::from_millis(25)))
                .unwrap();
            for event in &self.events {
                if event.token() == SOCKET && event.is_readable() {
                    self.lane
                        .cell()
                        .publish_ready(self.key, client_adapter::handoff::ReadyDirection::Read);
                }
            }
        }
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        let _ = self.lane.shutdown();
    }
}

fn make_ca(name: &str) -> (Certificate, KeyPair, CertificateDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![name.into()]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let cert = params.self_signed(&key).unwrap();
    let der = cert.der().clone();
    (cert, key, der)
}
fn issue_server(
    ca: &Certificate,
    ca_key: &KeyPair,
) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let cert = params.signed_by(&key, ca, ca_key).unwrap();
    (
        cert.der().clone(),
        PrivateKeyDer::Pkcs8(key.serialize_der().into()),
    )
}
fn server_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    version: &'static rustls::SupportedProtocolVersion,
) -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[version])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    Arc::new(config)
}

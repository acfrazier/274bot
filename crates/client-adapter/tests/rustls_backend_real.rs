#![cfg(feature = "test-support")]

//! Real local rustls backend qualification for G2-M1, G2-M3, G2-M4, G2-M5,
//! and the protocol/suite portion of G2-M7. These tests deliberately use the
//! production platform verifier with only test CA roots added; they never
//! install a permissive verifier or alter the machine trust store.

#[path = "support/g2d1_fairness.rs"]
mod g2d1_fairness;

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use client_adapter::cancel::{AttemptRetire, CancelSource};
use client_adapter::control::BindingId;
use client_adapter::handoff::{
    Accepted, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, WorkerFailure,
    WorkerOutcome,
};
use client_adapter::health::ReactorHealth;
use client_adapter::park::Park;
use client_adapter::shared::SharedConnection;
use client_adapter::tls_handshake::{
    build_test_client_config_with_roots, build_test_client_config_with_verifier, GameTlsLane,
    GameTlsResource,
};
use client_adapter::ws_endpoint::{WssEndpoint, WssStep};
use mio::{Events, Interest, Poll, Token, Waker};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use rustls::{
    CipherSuite, ClientConfig, DigitallySignedStruct, Error as RustlsError, ProtocolVersion,
    ServerConfig, ServerConnection, SignatureScheme,
};
use rustls_platform_verifier::Verifier;
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::{Message, Role, WebSocket, WebSocketConfig};

const SOCKET_TOKEN: Token = Token(1);
const WAKER_TOKEN: Token = Token(0);
const IO_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_SLICE: Duration = Duration::from_millis(25);

struct ServerIdentity {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

struct LocalPki {
    trusted_root: CertificateDer<'static>,
    unrelated_root: CertificateDer<'static>,
    valid_identity: ServerIdentity,
    expired_identity: ServerIdentity,
}

fn subject(common_name: &str) -> DistinguishedName {
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, common_name);
    subject
}

fn make_ca(common_name: &str) -> (Certificate, KeyPair, CertificateDer<'static>) {
    let key = KeyPair::generate().expect("generate local CA key");
    let mut params =
        CertificateParams::new(vec![common_name.to_owned()]).expect("create local CA parameters");
    // Each certificate needs its own subject name. With rcgen's shared default
    // subject, Windows CryptoAPI reads a leaf as self-signed and rejects the
    // chain (CERT_E_SIGNATURE, os error -2146869244).
    params.distinguished_name = subject(common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let certificate = params.self_signed(&key).expect("self-sign local CA");
    let der = certificate.der().clone();
    (certificate, key, der)
}

fn issue_identity(issuer: &Certificate, issuer_key: &KeyPair, expired: bool) -> ServerIdentity {
    let key = KeyPair::generate().expect("generate local server key");
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])
        .expect("create local server certificate parameters");
    params.distinguished_name = subject("rustls backend test server");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    if expired {
        // rcgen's default not_before is a fixed historical time. Setting the
        // end to that same instant produces an always-expired test certificate
        // without a clock dependency or process-global time override.
        params.not_after = params.not_before;
    }
    let certificate = params
        .signed_by(&key, issuer, issuer_key)
        .expect("sign local server certificate");
    ServerIdentity {
        certificate: certificate.der().clone(),
        key: PrivateKeyDer::Pkcs8(key.serialize_der().into()),
    }
}

impl LocalPki {
    fn new() -> Self {
        let (trusted_ca, trusted_key, trusted_root) = make_ca("rustls-backend-test-root");
        let valid_identity = issue_identity(&trusted_ca, &trusted_key, false);
        let expired_identity = issue_identity(&trusted_ca, &trusted_key, true);
        let (_, _, unrelated_root) = make_ca("unrelated-rustls-backend-test-root");
        Self {
            trusted_root,
            unrelated_root,
            valid_identity,
            expired_identity,
        }
    }

    fn config_with_root(root: &CertificateDer<'static>) -> Arc<ClientConfig> {
        build_test_client_config_with_roots(std::slice::from_ref(root))
            .expect("build OS-verifying rustls test config with one extra CA root")
    }
}

#[derive(Debug, Default)]
struct VerifierAudit {
    calls: AtomicUsize,
    worker_names: Mutex<Vec<String>>,
}

impl VerifierAudit {
    fn record_call(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = thread::current().name().unwrap_or("<unnamed>").to_owned();
        self.worker_names
            .lock()
            .expect("lock verifier worker audit")
            .push(name);
    }

    fn assert_game_worker_calls(&self, expected: usize) {
        assert_eq!(
            self.calls.load(Ordering::SeqCst),
            expected,
            "certificate-verifier entry count"
        );
        let worker_names = self
            .worker_names
            .lock()
            .expect("lock verifier worker audit");
        assert_eq!(worker_names.len(), expected);
        assert!(
            worker_names.iter().all(|name| name == "net-tls-handshake"),
            "certificate verification must run on the game TLS worker: {worker_names:?}"
        );
    }
}

#[derive(Debug)]
struct CountingPlatformVerifier {
    delegate: Verifier,
    audit: Arc<VerifierAudit>,
    hold: Option<Arc<PlatformVerifierHold>>,
}

impl ServerCertVerifier for CountingPlatformVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        self.audit.record_call();
        if let Some(hold) = &self.hold {
            hold.entered
                .send(())
                .expect("report platform verifier entry");
            let released = hold.released.lock().expect("lock verifier release");
            let (released, timeout) = hold
                .changed
                .wait_timeout_while(released, Duration::from_secs(15), |released| !*released)
                .expect("wait for platform verifier release");
            assert!(*released && !timeout.timed_out(), "verifier hold timed out");
        }
        self.delegate
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.delegate.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.delegate.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.delegate.supported_verify_schemes()
    }
}

fn config_with_counting_platform_verifier(
    roots: &[CertificateDer<'static>],
) -> (Arc<ClientConfig>, Arc<VerifierAudit>) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let delegate = Verifier::new_with_extra_roots(roots.iter().cloned(), provider)
        .expect("construct real platform verifier with test-only extra roots");
    let audit = Arc::new(VerifierAudit::default());
    let counting_verifier = Arc::new(CountingPlatformVerifier {
        delegate,
        audit: Arc::clone(&audit),
        hold: None,
    });
    let verifier: Arc<dyn ServerCertVerifier> = counting_verifier;
    let config = build_test_client_config_with_verifier(verifier)
        .expect("build test-support client config with counted platform verifier");
    (config, audit)
}

fn server_config(
    identity: ServerIdentity,
    version: ProtocolVersion,
    suite: CipherSuite,
) -> Arc<ServerConfig> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider
        .cipher_suites
        .retain(|candidate| candidate.suite() == suite);
    assert_eq!(
        provider.cipher_suites.len(),
        1,
        "ring provider must expose the selected test suite"
    );
    let provider = Arc::new(provider);
    let builder = ServerConfig::builder_with_provider(provider);
    let builder = match version {
        ProtocolVersion::TLSv1_2 => builder
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("configure TLS 1.2-only server"),
        ProtocolVersion::TLSv1_3 => builder
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("configure TLS 1.3-only server"),
        other => panic!("unsupported test protocol version: {other:?}"),
    };
    let mut config = builder
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate], identity.key)
        .expect("configure local rustls server identity");
    config.send_tls13_tickets = 2;
    Arc::new(config)
}

#[derive(Debug)]
struct ServerHandshake {
    server_name: Option<String>,
    protocol: Option<ProtocolVersion>,
    suite: Option<CipherSuite>,
    failure: Option<String>,
}

enum ServerCommand {
    SendBinary(Vec<u8>),
    SendBinaryAfterKeyUpdate(Vec<u8>),
    ExpectBinary {
        expected: Vec<u8>,
        reply: SyncSender<Result<(), String>>,
    },
    Ping {
        payload: Vec<u8>,
        reply: SyncSender<Result<(), String>>,
    },
    Stop,
}

struct LocalServer {
    address: std::net::SocketAddr,
    commands: Sender<ServerCommand>,
    handshake: Receiver<ServerHandshake>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl LocalServer {
    fn spawn(config: Arc<ServerConfig>) -> Self {
        Self::spawn_with_timeout(config, IO_TIMEOUT)
    }

    fn spawn_with_timeout(config: Arc<ServerConfig>, timeout: Duration) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local TLS server");
        let address = listener.local_addr().expect("read local TLS address");
        let (command_tx, command_rx) = mpsc::channel();
        let (handshake_tx, handshake_rx) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            serve_local_wss(listener, config, command_rx, handshake_tx, timeout)
        });
        Self {
            address,
            commands: command_tx,
            handshake: handshake_rx,
            thread: Some(server),
        }
    }

    fn handshake(&self) -> ServerHandshake {
        self.handshake
            .recv_timeout(IO_TIMEOUT)
            .expect("local rustls server completed or rejected its handshake")
    }

    fn stop(mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            server
                .join()
                .expect("local rustls server thread did not panic")
                .expect("local rustls server completed its WSS fixture");
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            let _ = server.join();
        }
    }
}

fn serve_local_wss(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    commands: Receiver<ServerCommand>,
    handshake_tx: SyncSender<ServerHandshake>,
    timeout: Duration,
) -> io::Result<()> {
    let (mut socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    socket.set_nodelay(true)?;

    let mut connection = ServerConnection::new(config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let handshake_result = loop {
        if !connection.is_handshaking() {
            break Ok(());
        }
        if let Err(error) = connection.complete_io(&mut socket) {
            break Err(error.to_string());
        }
    };
    let handshake = ServerHandshake {
        server_name: connection.server_name().map(str::to_owned),
        protocol: connection.protocol_version(),
        suite: connection
            .negotiated_cipher_suite()
            .map(|negotiated| negotiated.suite()),
        failure: handshake_result.err(),
    };
    let accepted_tls = handshake.failure.is_none();
    handshake_tx
        .send(handshake)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "test dropped handshake result"))?;
    if !accepted_tls {
        return Ok(());
    }

    let mut tls = rustls::StreamOwned::new(connection, socket);
    let request = read_http_head(&mut tls)?;
    let request = std::str::from_utf8(&request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("Sec-WebSocket-Key")
                .then_some(value.trim())
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing WebSocket key"))?;
    let accept = derive_accept_key(key.as_bytes());
    write!(
        tls,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
    )?;
    tls.flush()?;
    let mut websocket =
        WebSocket::from_raw_socket(tls, Role::Server, Some(WebSocketConfig::default()));

    while let Ok(command) = commands.recv() {
        match command {
            ServerCommand::SendBinary(payload) => {
                websocket
                    .send(Message::Binary(payload))
                    .map_err(ws_io_error)?;
            }
            ServerCommand::SendBinaryAfterKeyUpdate(payload) => {
                websocket
                    .get_mut()
                    .conn
                    .refresh_traffic_keys()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                websocket
                    .send(Message::Binary(payload))
                    .map_err(ws_io_error)?;
            }
            ServerCommand::ExpectBinary { expected, reply } => {
                let result = match websocket.read() {
                    Ok(Message::Binary(actual)) if actual == expected => Ok(()),
                    Ok(message) => Err(format!("unexpected client WSS message: {message:?}")),
                    Err(error) => Err(format!("read client WSS message: {error}")),
                };
                let _ = reply.send(result);
            }
            ServerCommand::Ping { payload, reply } => {
                websocket
                    .send(Message::Ping(payload.clone()))
                    .map_err(ws_io_error)?;
                let result = match websocket.read() {
                    Ok(Message::Pong(actual)) if actual == payload => Ok(()),
                    Ok(message) => Err(format!("unexpected Ping response: {message:?}")),
                    Err(error) => Err(format!("read Pong response: {error}")),
                };
                let _ = reply.send(result);
            }
            ServerCommand::Stop => break,
        }
    }
    Ok(())
}

fn ws_io_error(error: tungstenite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn read_http_head(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() == 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test WSS request header exceeded 4096 bytes",
            ));
        }
        reader.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    Ok(head)
}

struct LaneAttempt {
    poll: Poll,
    events: Events,
    lane: Arc<GameTlsLane>,
    key: HandoffKey,
    token: client_adapter::cancel::CancelToken,
    binding: BindingId,
    shared: Arc<SharedConnection>,
    retirement: Arc<AttemptRetire>,
}

impl LaneAttempt {
    fn start(address: std::net::SocketAddr, host: &str, config: Arc<ClientConfig>) -> Self {
        let poll = Poll::new().expect("create local TLS poller");
        let waker = Arc::new(
            Waker::new(poll.registry(), WAKER_TOKEN).expect("create local TLS handoff waker"),
        );
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().expect("mint one local TLS attempt token");
        let binding = BindingId {
            incarnation: 1,
            connection: 1,
            binding: 1,
            attempt: token.generation(),
        };
        let shared = SharedConnection::new(binding.connection, park);
        let lane = GameTlsLane::start_with_test_config(
            waker,
            Arc::new(ReactorHealth::new(Instant::now())),
            config,
        )
        .expect("start the real game TLS handshake worker");
        assert!(
            lane.config().is_some(),
            "worker must publish its TLS config"
        );
        let transfer_seq = lane
            .cell()
            .next_transfer_seq()
            .expect("initial transfer sequence is available");
        let key = HandoffKey {
            identity: HandoffIdentity {
                generation: token.generation(),
                binding,
            },
            transfer_seq,
        };
        let proof = Arc::new(QueueReleaseProof::new(binding, transfer_seq));
        let retirement = Arc::new(AttemptRetire::new(token.generation()));

        let socket = TcpStream::connect(address).expect("connect to local rustls server");
        socket.set_nodelay(true).expect("set local TCP_NODELAY");
        socket
            .set_nonblocking(true)
            .expect("make handshake socket nonblocking");
        let mut socket = mio::net::TcpStream::from_std(socket);
        poll.registry()
            .register(&mut socket, SOCKET_TOKEN, Interest::READABLE)
            .expect("register socket for handshake input");
        let resource = GameTlsResource::new(socket, host.to_owned(), None);
        lane.cell()
            .submit(
                key,
                proof,
                token.clone(),
                Arc::clone(&retirement),
                Instant::now() + HANDSHAKE_TIMEOUT,
                resource,
            )
            .unwrap_or_else(|error| panic!("submit initial TLS resource: {error:?}"));

        Self {
            poll,
            events: Events::with_capacity(16),
            lane,
            key,
            token,
            binding,
            shared,
            retirement,
        }
    }

    fn wait_for_return(&mut self) -> Accepted<GameTlsResource> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            match self.lane.cell().phase() {
                HandoffPhase::Returned => {
                    return self
                        .lane
                        .cell()
                        .take_return(self.key)
                        .expect("take the one completed TLS worker result");
                }
                HandoffPhase::Released => panic!("initial TLS attempt was revoked"),
                _ => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "initial TLS handshake timed out");
            self.poll
                .poll(&mut self.events, Some(remaining.min(POLL_SLICE)))
                .expect("poll local handshake socket");
            for event in &self.events {
                if event.token() == SOCKET_TOKEN && event.is_readable() {
                    self.lane
                        .cell()
                        .publish_ready(self.key, client_adapter::handoff::ReadyDirection::Read);
                }
            }
        }
    }

    fn accept_success(
        &mut self,
        accepted: Accepted<GameTlsResource>,
        authority: &str,
    ) -> WssEndpoint {
        assert_eq!(accepted.key, self.key, "accept the exact transfer identity");
        let mut resource = match accepted.outcome {
            WorkerOutcome::Success(resource) => resource,
            WorkerOutcome::Failure(failure) => panic!("expected trusted TLS, got {failure:?}"),
        };
        let connection = resource
            .connection
            .take()
            .expect("successful worker result carries rustls state");
        assert!(
            !connection.is_handshaking() && !connection.wants_write(),
            "the worker must flush Client Finished before transfer"
        );
        self.poll
            .registry()
            .reregister(
                &mut resource.socket,
                SOCKET_TOKEN,
                Interest::READABLE.add(Interest::WRITABLE),
            )
            .expect("retain the registered socket across TLS transfer");
        self.shared.accept_binding(self.binding);
        let endpoint = WssEndpoint::new(
            self.binding,
            authority,
            resource.socket,
            connection,
            Arc::clone(&self.shared),
            self.token.clone(),
            None,
            Arc::clone(&self.retirement),
        )
        .expect("construct product WSS endpoint from transferred TLS");
        assert!(self.retirement.ack(), "accepted transfer retires once");
        self.lane
            .cell()
            .finish_accept(self.key)
            .expect("finish accepted TLS transfer");
        endpoint
    }
}

impl Drop for LaneAttempt {
    fn drop(&mut self) {
        let _ = self.lane.shutdown();
    }
}

fn complete_wss_upgrade(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint) {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let step = endpoint.begin_step(Instant::now());
        endpoint
            .apply_interest(attempt.poll.registry())
            .expect("apply WSS upgrade interest");
        if let WssStep::Failed(error) = step {
            panic!("local WSS upgrade failed: {error}");
        }
        if endpoint.is_connected() {
            assert!(attempt.shared.commit_session(attempt.binding));
            return;
        }
        if matches!(step, WssStep::Wait) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "local WSS upgrade timed out");
            attempt
                .poll
                .poll(&mut attempt.events, Some(remaining.min(POLL_SLICE)))
                .expect("poll local WSS upgrade");
            for event in &attempt.events {
                if event.token() == SOCKET_TOKEN {
                    if let WssStep::Failed(error) =
                        endpoint.on_event(event.is_readable(), event.is_writable(), Instant::now())
                    {
                        panic!("local WSS upgrade failed on readiness: {error}");
                    }
                    endpoint
                        .apply_interest(attempt.poll.registry())
                        .expect("update WSS upgrade interest");
                }
            }
        }
    }
}

fn wait_for_inbound(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint, expected: &[u8]) {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut received = vec![0; expected.len()];
    loop {
        let count = attempt.shared.take_inbound(&mut received);
        if count != 0 {
            assert_eq!(count, expected.len());
            assert_eq!(
                &received[..count],
                expected,
                "preserve inbound game byte order"
            );
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "local WSS payload timed out");
        attempt
            .poll
            .poll(&mut attempt.events, Some(remaining.min(POLL_SLICE)))
            .expect("poll local WSS payload");
        for event in &attempt.events {
            if event.token() == SOCKET_TOKEN {
                if let WssStep::Failed(error) =
                    endpoint.on_event(event.is_readable(), event.is_writable(), Instant::now())
                {
                    panic!("local WSS endpoint failed on readiness: {error}");
                }
                let turn = endpoint.drive().expect("drive real WSS TLS/WS endpoint");
                assert!(turn.read_bytes <= expected.len());
                endpoint
                    .apply_interest(attempt.poll.registry())
                    .expect("update established WSS interest");
            }
        }
    }
}

fn exchange_one_local_wss_payload(
    server: &LocalServer,
    attempt: &mut LaneAttempt,
    endpoint: &mut WssEndpoint,
    outbound: &[u8],
    inbound: &[u8],
    key_update_before_inbound: bool,
) {
    let bytes_out_before = attempt.shared.bytes_out();
    attempt
        .shared
        .publish_record(attempt.binding, outbound)
        .expect("publish one bounded game record");
    endpoint.kick_output();
    let turn = endpoint.drive().expect("write one real WSS game record");
    assert_eq!(turn.written_bytes, outbound.len());
    assert!(turn.written_bytes <= outbound.len());
    assert_eq!(
        attempt.shared.bytes_out(),
        bytes_out_before + outbound.len() as u64
    );
    endpoint
        .apply_interest(attempt.poll.registry())
        .expect("update WSS write interest");

    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    server
        .commands
        .send(ServerCommand::ExpectBinary {
            expected: outbound.to_vec(),
            reply: reply_tx,
        })
        .expect("ask server to check the client game bytes");
    reply_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("server checked client game bytes")
        .unwrap_or_else(|error| panic!("server rejected client game bytes: {error}"));

    let command = if key_update_before_inbound {
        ServerCommand::SendBinaryAfterKeyUpdate(inbound.to_vec())
    } else {
        ServerCommand::SendBinary(inbound.to_vec())
    };
    server
        .commands
        .send(command)
        .expect("ask server to send an authenticated WSS payload");
    wait_for_inbound(attempt, endpoint, inbound);
}

fn expect_worker_verification_failure(
    config: Arc<ClientConfig>,
    identity: ServerIdentity,
    host: &str,
    expected_sni: &str,
) {
    let server_config = server_config(
        identity,
        ProtocolVersion::TLSv1_3,
        CipherSuite::TLS13_AES_128_GCM_SHA256,
    );
    let server = LocalServer::spawn(server_config);
    let mut attempt = LaneAttempt::start(server.address, host, config);
    let accepted = attempt.wait_for_return();
    let info = server.handshake();
    assert_eq!(info.server_name.as_deref(), Some(expected_sni));
    assert!(info.failure.is_some(), "the peer must observe rejected TLS");
    assert_eq!(info.protocol, Some(ProtocolVersion::TLSv1_3));
    assert_eq!(info.suite, Some(CipherSuite::TLS13_AES_128_GCM_SHA256));

    let Accepted {
        key,
        outcome,
        retirement,
    } = accepted;
    assert_eq!(key, attempt.key);
    assert!(matches!(
        outcome,
        WorkerOutcome::Failure(WorkerFailure::Verification)
    ));
    assert!(
        retirement.acknowledged(),
        "failed trust drops its socket before return"
    );
    assert_eq!(retirement.ack_count(), 1);
    attempt
        .lane
        .cell()
        .finish_accept(key)
        .expect("consume the one connection-local TLS failure");
    server.stop();
}

#[test]
fn g2_m4_m5_platform_verifier_accepts_extra_root_and_rejects_bad_trust_or_name() {
    let pki = LocalPki::new();
    let trusted_config = LocalPki::config_with_root(&pki.trusted_root);

    // G2-M4/G2-M5 valid-local-root case: exercise the OS verifier augmented
    // through the required public test-support constructor and the actual TLS
    // handshake worker. The full G2-M5 environment override matrix is separate.
    let server = LocalServer::spawn(server_config(
        ServerIdentity {
            certificate: pki.valid_identity.certificate.clone(),
            key: pki.valid_identity.key.clone_key(),
        },
        ProtocolVersion::TLSv1_3,
        CipherSuite::TLS13_AES_128_GCM_SHA256,
    ));
    let mut attempt = LaneAttempt::start(server.address, "localhost", Arc::clone(&trusted_config));
    let accepted = attempt.wait_for_return();
    let info = server.handshake();
    assert!(
        info.failure.is_none(),
        "valid extra-root chain must be trusted"
    );
    assert_eq!(info.server_name.as_deref(), Some("localhost"));
    assert_eq!(info.protocol, Some(ProtocolVersion::TLSv1_3));
    assert_eq!(info.suite, Some(CipherSuite::TLS13_AES_128_GCM_SHA256));
    let mut endpoint =
        attempt.accept_success(accepted, &format!("localhost:{}", server.address.port()));
    complete_wss_upgrade(&mut attempt, &mut endpoint);
    server.stop();
    drop(endpoint);

    // G2-M4/G2-M5 hostname guard: the valid certificate is still rejected for
    // a different SNI/DNS name; this is not a custom-verifier success path.
    expect_worker_verification_failure(
        Arc::clone(&trusted_config),
        ServerIdentity {
            certificate: pki.valid_identity.certificate.clone(),
            key: pki.valid_identity.key.clone_key(),
        },
        "wrong-name.invalid",
        "wrong-name.invalid",
    );

    // Unknown issuer and expired leaf are separate actual certificate failures.
    let unrelated_config = LocalPki::config_with_root(&pki.unrelated_root);
    expect_worker_verification_failure(
        unrelated_config,
        ServerIdentity {
            certificate: pki.valid_identity.certificate.clone(),
            key: pki.valid_identity.key.clone_key(),
        },
        "localhost",
        "localhost",
    );
    expect_worker_verification_failure(
        Arc::clone(&trusted_config),
        ServerIdentity {
            certificate: pki.expired_identity.certificate,
            key: pki.expired_identity.key,
        },
        "localhost",
        "localhost",
    );
}

fn vendor_rsa_identity_and_root() -> (ServerIdentity, CertificateDer<'static>) {
    let certificate = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.pem"
    ))
    .expect("parse checked-in RSA localhost server certificate");
    let key = PrivateKeyDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/localhost.key"
    ))
    .expect("parse checked-in RSA localhost server key");
    let root = CertificateDer::from_pem_slice(include_bytes!(
        "../../../vendor/fr-client-rust/crates/client/tests/fixtures/tls/ca.pem"
    ))
    .expect("parse checked-in RSA test CA");
    (ServerIdentity { certificate, key }, root)
}

#[test]
fn g2_m1_m3_m4_m7_real_worker_and_wss_endpoint_negotiate_and_exchange_tls_records() {
    let (rsa_identity, rsa_root) = vendor_rsa_identity_and_root();
    let pki = LocalPki::new();
    let (counted_config, verifier_audit) =
        config_with_counting_platform_verifier(&[rsa_root, pki.trusted_root.clone()]);

    // G2-M7 TLS 1.2 path: the server is restricted to the gate's exact
    // ECDHE-RSA/AES-128-GCM suite and uses the checked-in test RSA identity.
    let tls12_server = LocalServer::spawn(server_config(
        rsa_identity,
        ProtocolVersion::TLSv1_2,
        CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
    ));
    let mut tls12_attempt = LaneAttempt::start(
        tls12_server.address,
        "localhost",
        Arc::clone(&counted_config),
    );
    let tls12_accepted = tls12_attempt.wait_for_return();
    verifier_audit.assert_game_worker_calls(1);
    let tls12_info = tls12_server.handshake();
    assert!(tls12_info.failure.is_none(), "TLS 1.2 trust must succeed");
    assert_eq!(tls12_info.server_name.as_deref(), Some("localhost"));
    assert_eq!(tls12_info.protocol, Some(ProtocolVersion::TLSv1_2));
    assert_eq!(
        tls12_info.suite,
        Some(CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256)
    );
    let mut tls12_endpoint = tls12_attempt.accept_success(
        tls12_accepted,
        &format!("localhost:{}", tls12_server.address.port()),
    );
    complete_wss_upgrade(&mut tls12_attempt, &mut tls12_endpoint);
    exchange_one_local_wss_payload(
        &tls12_server,
        &mut tls12_attempt,
        &mut tls12_endpoint,
        b"tls12-client-game-record-01",
        b"tls12-server-game-record-01",
        false,
    );
    verifier_audit.assert_game_worker_calls(1);
    tls12_server.stop();
    drop(tls12_endpoint);

    // G2-M1/M3/M4/M7 TLS 1.3 path: this is a second, same-host full
    // handshake using the same non-resuming test config. The rustls server
    // sends ordinary post-Finished tickets, then refreshes traffic keys
    // between WSS game records. Verification must remain on the worker and
    // must not re-enter after handoff.
    let tls13_server = LocalServer::spawn(server_config(
        ServerIdentity {
            certificate: pki.valid_identity.certificate,
            key: pki.valid_identity.key,
        },
        ProtocolVersion::TLSv1_3,
        CipherSuite::TLS13_AES_128_GCM_SHA256,
    ));
    let mut tls13_attempt = LaneAttempt::start(
        tls13_server.address,
        "localhost",
        Arc::clone(&counted_config),
    );
    let tls13_accepted = tls13_attempt.wait_for_return();
    verifier_audit.assert_game_worker_calls(2);
    let tls13_info = tls13_server.handshake();
    assert!(tls13_info.failure.is_none(), "TLS 1.3 trust must succeed");
    assert_eq!(tls13_info.server_name.as_deref(), Some("localhost"));
    assert_eq!(tls13_info.protocol, Some(ProtocolVersion::TLSv1_3));
    assert_eq!(
        tls13_info.suite,
        Some(CipherSuite::TLS13_AES_128_GCM_SHA256)
    );
    let mut tls13_endpoint = tls13_attempt.accept_success(
        tls13_accepted,
        &format!("localhost:{}", tls13_server.address.port()),
    );
    complete_wss_upgrade(&mut tls13_attempt, &mut tls13_endpoint);
    exchange_one_local_wss_payload(
        &tls13_server,
        &mut tls13_attempt,
        &mut tls13_endpoint,
        b"tls13-client-game-record-01",
        b"tls13-server-game-record-01",
        false,
    );
    verifier_audit.assert_game_worker_calls(2);
    exchange_one_local_wss_payload(
        &tls13_server,
        &mut tls13_attempt,
        &mut tls13_endpoint,
        b"tls13-client-game-record-02",
        b"tls13-after-key-update-record-02",
        true,
    );
    verifier_audit.assert_game_worker_calls(2);
    tls13_server.stop();
    drop(tls13_endpoint);
}

#[derive(Debug)]
struct PlatformVerifierHold {
    entered: Sender<()>,
    released: Mutex<bool>,
    changed: Condvar,
}

impl PlatformVerifierHold {
    fn release(&self) {
        *self.released.lock().expect("lock verifier release") = true;
        self.changed.notify_all();
    }
}

struct ReleasePlatformVerifier(Arc<PlatformVerifierHold>);

impl Drop for ReleasePlatformVerifier {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn g7_actual_platform_verifier_https_hold_obeys_step_and_quiet_deadlines() {
    use client::client::client::AssetFetchError;
    use client_adapter::http::start_fetch;
    use client_adapter::https_handshake::HttpsLane;

    for expires in [false, true] {
        let pki = LocalPki::new();
        let server_config = server_config(
            pki.valid_identity,
            ProtocolVersion::TLSv1_3,
            CipherSuite::TLS13_AES_128_GCM_SHA256,
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind HTTPS fixture");
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().expect("accept HTTPS fixture");
            socket
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            let tls = ServerConnection::new(server_config).unwrap();
            let mut stream = rustls::StreamOwned::new(tls, socket);
            let request = read_http_head(&mut stream);
            if expires {
                let error = request.expect_err("expired fetch must not send an HTTP request");
                assert!(
                    matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::BrokenPipe
                    ),
                    "late handshake must close physically, not time out: {error}"
                );
            } else {
                assert_eq!(
                    request.unwrap(),
                    b"GET /held-verifier HTTP/1.0\r\nHost: localhost\r\n\r\n"
                );
                stream
                    .write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 4\r\n\r\nheld")
                    .unwrap();
                stream.flush().unwrap();
                stream.conn.send_close_notify();
                stream.flush().unwrap();
            }
        });
        let (entered_tx, entered_rx) = mpsc::channel();
        let hold = Arc::new(PlatformVerifierHold {
            entered: entered_tx,
            released: Mutex::new(false),
            changed: Condvar::new(),
        });
        let release_guard = ReleasePlatformVerifier(Arc::clone(&hold));
        let audit = Arc::new(VerifierAudit::default());
        let delegate = Verifier::new_with_extra_roots(
            [pki.trusted_root],
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .expect("construct actual platform verifier");
        let config = build_test_client_config_with_verifier(Arc::new(CountingPlatformVerifier {
            delegate,
            audit: Arc::clone(&audit),
            hold: Some(Arc::clone(&hold)),
        }))
        .unwrap();
        let weak_config = Arc::downgrade(&config);
        let lane =
            HttpsLane::start(Some(config), Arc::new(ReactorHealth::new(Instant::now()))).unwrap();
        let started = Instant::now();
        let mut fetch = start_fetch(
            client::Transport::Wss,
            "localhost",
            &[address],
            "/held-verifier",
            &lane,
        )
        .unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("actual HTTPS platform-verifier entry");
        assert_eq!(audit.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*audit.worker_names.lock().unwrap(), ["net-https-handshake"]);
        let mut pending_steps = 0;
        loop {
            let until = Instant::now() + Duration::from_millis(8);
            let result = fetch.poll(until);
            let returned = Instant::now();
            assert!(
                returned <= until + Duration::from_millis(25),
                "held OS verifier blocked the stepped caller: expires={expires} \
                 pending_steps={pending_steps} overrun={:?} elapsed={:?} result={result:?}",
                returned.saturating_duration_since(until),
                started.elapsed()
            );
            match result {
                Ok(None) => pending_steps += 1,
                Err(AssetFetchError::Connection) if expires => {
                    assert!(started.elapsed() >= Duration::from_secs(8));
                    assert!(started.elapsed() < Duration::from_secs(9));
                    break;
                }
                other => panic!("unexpected held-verifier result: {other:?}"),
            }
            if !expires && pending_steps >= 20 {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(10));
        }
        assert!(pending_steps >= 20);
        hold.release();
        if expires {
            assert!(matches!(
                fetch.poll(Instant::now() + Duration::from_millis(8)),
                Err(AssetFetchError::Connection)
            ));
        } else {
            loop {
                match fetch.poll(Instant::now() + Duration::from_millis(8)) {
                    Ok(Some(body)) => {
                        assert_eq!(body, b"held");
                        break;
                    }
                    Ok(None) => assert!(started.elapsed() < Duration::from_secs(8)),
                    other => panic!("released platform verifier must succeed: {other:?}"),
                }
            }
        }
        drop(fetch);
        lane.shutdown();
        drop(lane);
        server.join().expect("HTTPS fixture thread");
        let cleanup_deadline = Instant::now() + Duration::from_secs(4);
        while weak_config.upgrade().is_some() {
            assert!(
                Instant::now() < cleanup_deadline,
                "HTTPS worker retained its config"
            );
            thread::yield_now();
        }
        drop(release_guard);
        println!(
            "G7 actual-platform-verifier expires={expires} pending_steps={pending_steps} elapsed_ms={} physical_cleanup=true",
            started.elapsed().as_millis()
        );
    }
}

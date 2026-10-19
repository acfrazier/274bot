#![cfg(feature = "test-support")]

//! Authenticated post-handshake TLS control records through the real client
//! handshake lane and WSS endpoint. The server-side test writer uses secrets
//! extracted from its completed rustls connection; it never mutates plaintext
//! socket bytes or substitutes a test verifier.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use client_adapter::cancel::{AttemptRetire, CancelSource};
use client_adapter::control::BindingId;
use client_adapter::handoff::{
    Accepted, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, WorkerOutcome,
};
use client_adapter::health::ReactorHealth;
use client_adapter::park::Park;
use client_adapter::shared::SharedConnection;
use client_adapter::tls_handshake::{
    build_test_client_config_with_roots, GameTlsLane, GameTlsResource,
};
use client_adapter::ws_endpoint::{WssEndpoint, WssStep};
use mio::{Events, Interest, Poll, Token, Waker};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::crypto::cipher::{
    InboundOpaqueMessage, MessageDecrypter, MessageEncrypter, OutboundChunks, OutboundPlainMessage,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{
    CipherSuite, ClientConfig, ConnectionTrafficSecrets, ContentType, ProtocolVersion,
    ServerConfig, ServerConnection, SupportedCipherSuite,
};
use tungstenite::handshake::derive_accept_key;

const SOCKET_TOKEN: Token = Token(1);
const WAKER_TOKEN: Token = Token(0);
const IO_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const POLL_SLICE: Duration = Duration::from_millis(25);
const MAX_TLS_RECORD: usize = 18_432;

const TLS12_SUITE: CipherSuite = CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256;
const TLS13_SUITE: CipherSuite = CipherSuite::TLS13_AES_128_GCM_SHA256;
const HELLO_REQUEST: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
const KEY_UPDATE_REQUESTED: [u8; 5] = [0x18, 0x00, 0x00, 0x01, 0x01];
const TLS_CLOSE_NOTIFY: [u8; 2] = [0x01, 0x00];

struct ServerIdentity {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl ServerIdentity {
    fn duplicate(&self) -> Self {
        Self {
            certificate: self.certificate.clone(),
            key: self.key.clone_key(),
        }
    }
}

struct TestPki {
    root: CertificateDer<'static>,
    identity: ServerIdentity,
}

impl TestPki {
    fn new() -> Self {
        let root_key = KeyPair::generate().expect("generate test CA key");
        let mut root_params = CertificateParams::new(vec!["rustls-control-test-root".to_owned()])
            .expect("create test CA parameters");
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let root_certificate = root_params
            .self_signed(&root_key)
            .expect("self-sign test CA");
        let root = root_certificate.der().clone();

        let identity_key = KeyPair::generate().expect("generate test server key");
        let mut identity_params = CertificateParams::new(vec!["localhost".to_owned()])
            .expect("create test server certificate parameters");
        identity_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        identity_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let identity_certificate = identity_params
            .signed_by(&identity_key, &root_certificate, &root_key)
            .expect("sign test server certificate");

        Self {
            root,
            identity: ServerIdentity {
                certificate: identity_certificate.der().clone(),
                key: PrivateKeyDer::Pkcs8(identity_key.serialize_der().into()),
            },
        }
    }

    fn client_config(&self) -> Arc<ClientConfig> {
        build_test_client_config_with_roots(std::slice::from_ref(&self.root))
            .expect("build OS-verifying client config with the test CA root")
    }
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
        "selected ring suite exists"
    );

    let builder = ServerConfig::builder_with_provider(Arc::new(provider));
    let builder = match version {
        ProtocolVersion::TLSv1_2 => builder
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("configure TLS 1.2-only server"),
        ProtocolVersion::TLSv1_3 => builder
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("configure TLS 1.3-only server"),
        other => panic!("unsupported control-test TLS version: {other:?}"),
    };
    let mut config = builder
        .with_no_client_auth()
        .with_single_cert(vec![identity.certificate], identity.key)
        .expect("configure local rustls server identity");
    config.enable_secret_extraction = true;
    config.send_tls13_tickets = 0;
    Arc::new(config)
}

#[derive(Debug)]
struct HandshakeInfo {
    server_name: Option<String>,
    version: Option<ProtocolVersion>,
    suite: Option<CipherSuite>,
    failure: Option<String>,
}

#[derive(Debug)]
struct ObservedRecord {
    outer_type: ContentType,
    content_type: ContentType,
    payload: Vec<u8>,
}

enum ServerCommand {
    SendRecord {
        content_type: ContentType,
        payload: Vec<u8>,
        reply: SyncSender<Result<(), String>>,
    },
    ReadClientRecord {
        timeout: Duration,
        reply: SyncSender<Result<Option<ObservedRecord>, String>>,
    },
    Stop,
}

struct ControlServer {
    address: std::net::SocketAddr,
    commands: Sender<ServerCommand>,
    handshake: Receiver<HandshakeInfo>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ControlServer {
    fn spawn(config: Arc<ServerConfig>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local TLS server");
        let address = listener.local_addr().expect("read local TLS address");
        let (command_tx, command_rx) = mpsc::channel();
        let (handshake_tx, handshake_rx) = mpsc::sync_channel(1);
        let server =
            thread::spawn(move || serve_control_server(listener, config, command_rx, handshake_tx));
        Self {
            address,
            commands: command_tx,
            handshake: handshake_rx,
            thread: Some(server),
        }
    }

    fn handshake(&self) -> HandshakeInfo {
        self.handshake
            .recv_timeout(IO_TIMEOUT)
            .expect("local rustls server reports the negotiated handshake")
    }

    fn send_record(&self, content_type: ContentType, payload: &[u8]) {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(ServerCommand::SendRecord {
                content_type,
                payload: payload.to_vec(),
                reply: reply_tx,
            })
            .expect("send authenticated TLS record command");
        reply_rx
            .recv_timeout(IO_TIMEOUT)
            .expect("server emitted the authenticated TLS record")
            .unwrap_or_else(|error| panic!("server failed to emit TLS record: {error}"));
    }

    fn read_client_record(&self, timeout: Duration) -> Option<ObservedRecord> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(ServerCommand::ReadClientRecord {
                timeout,
                reply: reply_tx,
            })
            .expect("send client-record inspection command");
        reply_rx
            .recv_timeout(IO_TIMEOUT)
            .expect("server completed client-record inspection")
            .unwrap_or_else(|error| panic!("server could not inspect client TLS record: {error}"))
    }

    fn stop(mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            server
                .join()
                .expect("local control server thread did not panic")
                .expect("local control server completed");
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.commands.send(ServerCommand::Stop);
        if let Some(server) = self.thread.take() {
            let _ = server.join();
        }
    }
}

struct RecordEncrypter {
    sequence: u64,
    cipher: Box<dyn MessageEncrypter>,
}

impl RecordEncrypter {
    fn new(
        version: ProtocolVersion,
        suite: SupportedCipherSuite,
        secrets: (u64, ConnectionTrafficSecrets),
    ) -> Self {
        let (sequence, secrets) = secrets;
        let cipher = match (version, suite, secrets) {
            (
                ProtocolVersion::TLSv1_2,
                SupportedCipherSuite::Tls12(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => {
                let iv = iv.as_ref();
                assert_eq!(iv.len(), 12, "TLS 1.2 extracted GCM nonce has 12 bytes");
                suite.aead_alg.encrypter(key, &iv[..4], &iv[4..])
            }
            (
                ProtocolVersion::TLSv1_3,
                SupportedCipherSuite::Tls13(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => suite.aead_alg.encrypter(key, iv),
            _ => panic!("unexpected test cipher/traffic-secret combination"),
        };
        Self { sequence, cipher }
    }

    fn encrypt(&mut self, content_type: ContentType, payload: &[u8]) -> Result<Vec<u8>, String> {
        let message = OutboundPlainMessage {
            typ: content_type,
            // TLS 1.3 uses TLS 1.2 as the legacy record-layer version too.
            version: ProtocolVersion::TLSv1_2,
            payload: OutboundChunks::from(payload),
        };
        let record = self
            .cipher
            .encrypt(message, self.sequence)
            .map_err(|error| error.to_string())?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "TLS transmit sequence overflowed".to_owned())?;
        Ok(record.encode())
    }
}

struct RecordDecrypter {
    sequence: u64,
    cipher: Box<dyn MessageDecrypter>,
}

impl RecordDecrypter {
    fn new(
        version: ProtocolVersion,
        suite: SupportedCipherSuite,
        secrets: (u64, ConnectionTrafficSecrets),
    ) -> Self {
        let (sequence, secrets) = secrets;
        let cipher = match (version, suite, secrets) {
            (
                ProtocolVersion::TLSv1_2,
                SupportedCipherSuite::Tls12(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => {
                let iv = iv.as_ref();
                assert_eq!(iv.len(), 12, "TLS 1.2 extracted GCM nonce has 12 bytes");
                suite.aead_alg.decrypter(key, &iv[..4])
            }
            (
                ProtocolVersion::TLSv1_3,
                SupportedCipherSuite::Tls13(suite),
                ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            ) => suite.aead_alg.decrypter(key, iv),
            _ => panic!("unexpected test cipher/traffic-secret combination"),
        };
        Self { sequence, cipher }
    }

    fn decrypt(&mut self, record: WireRecord) -> Result<ObservedRecord, String> {
        if record.version != [0x03, 0x03] {
            return Err(format!(
                "unexpected TLS legacy record version: {:?}",
                record.version
            ));
        }
        let outer_type = ContentType::from(record.content_type);
        let mut payload = record.payload;
        let plaintext = self
            .cipher
            .decrypt(
                InboundOpaqueMessage::new(outer_type, ProtocolVersion::TLSv1_2, &mut payload),
                self.sequence,
            )
            .map_err(|error| error.to_string())?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "TLS receive sequence overflowed".to_owned())?;
        Ok(ObservedRecord {
            outer_type,
            content_type: plaintext.typ,
            payload: plaintext.payload.to_vec(),
        })
    }
}

struct WireRecord {
    content_type: u8,
    version: [u8; 2],
    payload: Vec<u8>,
}

fn read_wire_record(stream: &mut TcpStream, timeout: Duration) -> io::Result<Option<WireRecord>> {
    stream.set_read_timeout(Some(timeout))?;
    let mut header = [0u8; 5];
    match stream.read(&mut header[..1]) {
        Ok(0) => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "peer closed before a TLS record header",
            ));
        }
        Ok(1) => {}
        Ok(_) => unreachable!("one-byte read cannot return more than one byte"),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    stream.read_exact(&mut header[1..])?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    if length > MAX_TLS_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("TLS record length {length} exceeds the test bound"),
        ));
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok(Some(WireRecord {
        content_type: header[0],
        version: [header[1], header[2]],
        payload,
    }))
}

fn serve_control_server(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    commands: Receiver<ServerCommand>,
    handshake_tx: SyncSender<HandshakeInfo>,
) -> io::Result<()> {
    let (mut socket, _) = listener.accept()?;
    socket.set_read_timeout(Some(IO_TIMEOUT))?;
    socket.set_write_timeout(Some(IO_TIMEOUT))?;
    socket.set_nodelay(true)?;

    let mut connection = ServerConnection::new(config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut handshake_failure = None;
    while connection.is_handshaking() {
        if let Err(error) = connection.complete_io(&mut socket) {
            handshake_failure = Some(error.to_string());
            break;
        }
    }
    let info = HandshakeInfo {
        server_name: connection.server_name().map(str::to_owned),
        version: connection.protocol_version(),
        suite: connection
            .negotiated_cipher_suite()
            .map(|negotiated| negotiated.suite()),
        failure: handshake_failure,
    };
    let accepted_tls = info.failure.is_none();
    handshake_tx
        .send(info)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "handshake observer dropped"))?;
    if !accepted_tls {
        return Ok(());
    }

    let version = connection
        .protocol_version()
        .expect("successful test handshake has a protocol version");
    let suite = connection
        .negotiated_cipher_suite()
        .expect("successful test handshake has a cipher suite");
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

    let (connection, mut socket) = tls.into_parts();
    let secrets = connection
        .dangerous_extract_secrets()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut tx = RecordEncrypter::new(version, suite, secrets.tx);
    let mut rx = RecordDecrypter::new(version, suite, secrets.rx);

    while let Ok(command) = commands.recv() {
        match command {
            ServerCommand::SendRecord {
                content_type,
                payload,
                reply,
            } => {
                let result = tx
                    .encrypt(content_type, &payload)
                    .and_then(|record| socket.write_all(&record).map_err(|error| error.to_string()))
                    .and_then(|()| socket.flush().map_err(|error| error.to_string()));
                let _ = reply.send(result);
            }
            ServerCommand::ReadClientRecord { timeout, reply } => {
                let result = match read_wire_record(&mut socket, timeout) {
                    Ok(Some(record)) => rx.decrypt(record).map(Some),
                    Ok(None) => Ok(None),
                    Err(error) => Err(error.to_string()),
                };
                let _ = reply.send(result);
            }
            ServerCommand::Stop => break,
        }
    }
    Ok(())
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
    fn start(address: std::net::SocketAddr, config: Arc<ClientConfig>) -> Self {
        let poll = Poll::new().expect("create local TLS poller");
        let waker =
            Arc::new(Waker::new(poll.registry(), WAKER_TOKEN).expect("create handoff waker"));
        let park = Arc::new(Park::new());
        let source = CancelSource::new(Arc::clone(&park));
        let token = source.child().expect("mint one TLS attempt token");
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
        .expect("start real game TLS handshake worker");
        assert!(lane.config().is_some(), "worker publishes its TLS config");

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
            .expect("register handshake socket");
        let resource = GameTlsResource::new(socket, "localhost".to_owned(), None);
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
                        .expect("take the completed TLS worker result");
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
        assert_eq!(accepted.key, self.key, "accept exact transfer identity");
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
            "worker flushes Client Finished before transfer"
        );
        self.poll
            .registry()
            .reregister(
                &mut resource.socket,
                SOCKET_TOKEN,
                Interest::READABLE.add(Interest::WRITABLE),
            )
            .expect("retain registered socket across TLS transfer");
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
        .expect("construct real WSS endpoint from transferred TLS");
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

fn connect_wss(
    server: &ControlServer,
    config: Arc<ClientConfig>,
) -> (LaneAttempt, WssEndpoint, HandshakeInfo) {
    let mut attempt = LaneAttempt::start(server.address, config);
    let accepted = attempt.wait_for_return();
    let info = server.handshake();
    assert!(
        info.failure.is_none(),
        "test server accepts the trusted client: {info:?}"
    );
    assert_eq!(info.server_name.as_deref(), Some("localhost"));
    let authority = format!("localhost:{}", server.address.port());
    let mut endpoint = attempt.accept_success(accepted, &authority);
    complete_wss_upgrade(&mut attempt, &mut endpoint);
    assert!(
        endpoint.is_connected(),
        "the real WSS endpoint completes upgrade"
    );
    (attempt, endpoint, info)
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

fn next_readable_turn(
    attempt: &mut LaneAttempt,
    endpoint: &mut WssEndpoint,
) -> io::Result<client_adapter::tcp::TcpTurnResult> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for authenticated TLS input"
        );
        attempt
            .poll
            .poll(&mut attempt.events, Some(remaining.min(POLL_SLICE)))?;
        let mut readable = false;
        for event in &attempt.events {
            if event.token() == SOCKET_TOKEN && event.is_readable() {
                endpoint.on_event(true, event.is_writable(), Instant::now());
                readable = true;
            }
        }
        if readable {
            let turn = endpoint.drive();
            if turn.is_ok() {
                endpoint.apply_interest(attempt.poll.registry())?;
            }
            return turn;
        }
    }
}

fn take_game_payload(attempt: &mut LaneAttempt, endpoint: &mut WssEndpoint, expected: &[u8]) {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    while attempt.shared.inbound.len() < expected.len() {
        assert!(
            Instant::now() < deadline,
            "authenticated follow-up WebSocket payload timed out"
        );
        next_readable_turn(attempt, endpoint)
            .unwrap_or_else(|error| panic!("established WSS drive failed: {error}"));
    }
    let mut received = vec![0; expected.len()];
    let count = attempt.shared.take_inbound(&mut received);
    assert_eq!(count, expected.len());
    assert_eq!(
        received.as_slice(),
        expected,
        "current session preserves game bytes"
    );
}

fn server_binary_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.push(0x82); // FIN + binary, server frames are not masked.
    match payload.len() {
        length @ 0..=125 => frame.push(length as u8),
        length @ 126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => panic!("control-test binary frame is too large: {length}"),
    }
    frame.extend_from_slice(payload);
    frame
}

fn assert_negotiated(handshake: &HandshakeInfo, version: ProtocolVersion, suite: CipherSuite) {
    assert!(
        handshake.failure.is_none(),
        "TLS handshake succeeded: {handshake:?}"
    );
    assert_eq!(handshake.server_name.as_deref(), Some("localhost"));
    assert_eq!(handshake.version, Some(version));
    assert_eq!(handshake.suite, Some(suite));
}

fn configure_tls12(identity: ServerIdentity) -> Arc<ServerConfig> {
    server_config(identity, ProtocolVersion::TLSv1_2, TLS12_SUITE)
}

fn configure_tls13(identity: ServerIdentity) -> Arc<ServerConfig> {
    server_config(identity, ProtocolVersion::TLSv1_3, TLS13_SUITE)
}

#[test]
fn g2_m2_authenticated_hello_request_keeps_session_and_excess_is_fatal() {
    let pki = TestPki::new();
    let client_config = pki.client_config();

    let server = ControlServer::spawn(configure_tls12(ServerIdentity {
        certificate: pki.identity.certificate.clone(),
        key: pki.identity.key.clone_key(),
    }));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, Arc::clone(&client_config));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let initial_game = b"before-hello-request-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();

    // The server's extracted TX key and sequence authenticate this record.
    server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    let turn = next_readable_turn(&mut attempt, &mut endpoint)
        .expect("one authenticated HelloRequest is refused without renegotiation");
    assert_eq!(turn.read_bytes, 0, "HelloRequest yields no game bytes");
    assert!(
        endpoint.is_connected(),
        "one request leaves the current WSS session live"
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(attempt.shared.inbound.is_empty());

    let response = server
        .read_client_record(IO_TIMEOUT)
        .expect("client emits its authenticated TLS response to HelloRequest");
    assert_eq!(response.outer_type, ContentType::Alert);
    assert_eq!(response.content_type, ContentType::Alert);
    assert_eq!(
        response.payload.as_slice(),
        &[0x01, 0x64][..],
        "TLS 1.2 no_renegotiation warning"
    );

    // A separately encrypted record, with the next TLS sequence number,
    // proves the original session continued after the refusal.
    let game = b"post-hello-request-game-data";
    server.send_record(ContentType::ApplicationData, &server_binary_frame(game));
    take_game_payload(&mut attempt, &mut endpoint, game);
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(
        attempt.shared.bytes_in(),
        bytes_in_before + game.len() as u64
    );
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "the client did not start a replacement handshake or emit game data"
    );
    endpoint.close(attempt.poll.registry());
    server.stop();

    // rustls permits one refusal response, but a second authenticated request
    // is an excessive-request protocol error that closes only this endpoint.
    let excessive_server = ControlServer::spawn(configure_tls12(pki.identity.duplicate()));
    let (mut excessive_attempt, mut excessive_endpoint, excessive_handshake) =
        connect_wss(&excessive_server, Arc::clone(&client_config));
    assert_negotiated(&excessive_handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let sibling_server = ControlServer::spawn(configure_tls12(pki.identity.duplicate()));
    let (mut sibling_attempt, mut sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, Arc::clone(&client_config));
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);
    let excessive_initial_game = b"before-excessive-hello-request-data";
    excessive_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(excessive_initial_game),
    );
    take_game_payload(
        &mut excessive_attempt,
        &mut excessive_endpoint,
        excessive_initial_game,
    );
    let bytes_in_before = excessive_attempt.shared.bytes_in();
    let bytes_out_before = excessive_attempt.shared.bytes_out();
    excessive_server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    excessive_server.send_record(ContentType::Handshake, &HELLO_REQUEST);
    let error = loop {
        match next_readable_turn(&mut excessive_attempt, &mut excessive_endpoint) {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let detail = error
        .get_ref()
        .expect("TLS protocol failure remains a typed error source")
        .to_string();
    assert!(
        detail.to_ascii_lowercase().contains("renegotiation"),
        "excessive HelloRequest failure is identified: {detail}"
    );
    assert_eq!(excessive_attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(excessive_attempt.shared.bytes_in(), bytes_in_before);
    assert!(excessive_attempt.shared.inbound.is_empty());
    assert!(excessive_attempt.shared.outbound.is_empty());
    if let Some(response) = excessive_server.read_client_record(Duration::from_millis(250)) {
        assert_eq!(response.content_type, ContentType::Alert);
        assert_eq!(
            response.payload.len(),
            2,
            "no handshake or game payload is emitted"
        );
    }
    excessive_endpoint.close(excessive_attempt.poll.registry());
    assert!(
        excessive_attempt.shared.is_closed(),
        "excessive stream closes locally"
    );

    let sibling_game = b"sibling-session-remains-healthy";
    sibling_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(sibling_game),
    );
    take_game_payload(&mut sibling_attempt, &mut sibling_endpoint, sibling_game);
    sibling_endpoint.close(sibling_attempt.poll.registry());
    excessive_server.stop();
    sibling_server.stop();
}

#[test]
fn g2_m2_authenticated_tls_close_notify_is_distinct_from_excessive_request() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls12(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_2, TLS12_SUITE);

    let initial_game = b"before-peer-close-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    server.send_record(ContentType::Alert, &TLS_CLOSE_NOTIFY);
    let error = next_readable_turn(&mut attempt, &mut endpoint)
        .expect_err("authenticated TLS close_notify ends the peer's WSS stream");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        format!("{error:?}").contains("ResetWithoutClosingHandshake"),
        "expected an authenticated peer-close error, got {error:?}"
    );
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert_eq!(attempt.shared.bytes_out(), 0);
    assert!(attempt.shared.inbound.is_empty());

    endpoint.close(attempt.poll.registry());
    server.stop();
}

#[test]
fn g2_m3_authenticated_requested_key_update_emits_reply_without_another_read() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, client_config);
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    let bytes_out_before = attempt.shared.bytes_out();

    server.send_record(ContentType::Handshake, &KEY_UPDATE_REQUESTED);
    next_readable_turn(&mut attempt, &mut endpoint)
        .expect("process authenticated requested KeyUpdate in one endpoint turn");

    // No second endpoint read/event or WebSocket write occurs before this peer
    // read. The reply itself is decrypted with the server's extracted RX key.
    let response = server
        .read_client_record(IO_TIMEOUT)
        .expect("rustls emits the requested KeyUpdate reply");
    assert_eq!(response.outer_type, ContentType::ApplicationData);
    assert_eq!(response.content_type, ContentType::Handshake);
    assert_eq!(
        response.payload.as_slice(),
        &[0x18, 0x00, 0x00, 0x01, 0x00][..]
    );
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), 0);
    assert!(attempt.shared.inbound.is_empty());
    assert!(
        server
            .read_client_record(Duration::from_millis(250))
            .is_none(),
        "no application/game payload follows the KeyUpdate reply"
    );

    endpoint.close(attempt.poll.registry());
    server.stop();
}

#[test]
fn g2_m6_authenticated_post_handshake_certificate_request_fails_closed() {
    let pki = TestPki::new();
    let client_config = pki.client_config();
    let server = ControlServer::spawn(configure_tls13(pki.identity.duplicate()));
    let sibling_server = ControlServer::spawn(configure_tls13(pki.identity.duplicate()));
    let (mut attempt, mut endpoint, handshake) = connect_wss(&server, Arc::clone(&client_config));
    assert_negotiated(&handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);
    let initial_game = b"before-certificate-request-game-data";
    server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(initial_game),
    );
    take_game_payload(&mut attempt, &mut endpoint, initial_game);
    let bytes_in_before = attempt.shared.bytes_in();
    let bytes_out_before = attempt.shared.bytes_out();
    let (mut sibling_attempt, mut sibling_endpoint, sibling_handshake) =
        connect_wss(&sibling_server, Arc::clone(&client_config));
    assert_negotiated(&sibling_handshake, ProtocolVersion::TLSv1_3, TLS13_SUITE);

    // RFC 8446 CertificateRequest with a unique nonempty post-handshake
    // context and a valid signature_algorithms extension (ecdsa_secp256r1_sha256).
    let certificate_request = [
        0x0d, 0x00, 0x00, 0x10, // HandshakeType + 16-byte body
        0x05, b'r', b'7', b'e', b'-', b'm', // nonempty request context
        0x00, 0x08, // extension vector
        0x00, 0x0d, 0x00, 0x04, // signature_algorithms extension
        0x00, 0x02, 0x04, 0x03, // one TLS 1.3-compatible scheme
    ];
    server.send_record(ContentType::Handshake, &certificate_request);
    let error = next_readable_turn(&mut attempt, &mut endpoint)
        .expect_err("post-handshake CertificateRequest is a local TLS failure");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let detail = error
        .get_ref()
        .expect("TLS failure retains its typed rustls source")
        .to_string();
    assert!(!detail.is_empty());
    assert_eq!(attempt.shared.bytes_out(), bytes_out_before);
    assert_eq!(attempt.shared.bytes_in(), bytes_in_before);
    assert!(attempt.shared.inbound.is_empty());
    assert!(attempt.shared.outbound.is_empty());

    // rustls may emit a fatal TLS alert. If it does, authenticate and inspect
    // it: no client Certificate, success data, or renewed server handshake is
    // allowed to leave this established session.
    if let Some(response) = server.read_client_record(Duration::from_millis(500)) {
        assert_eq!(response.content_type, ContentType::Alert);
        assert_eq!(response.payload.len(), 2);
        assert_eq!(
            response.payload[0], 0x02,
            "only a fatal TLS alert is emitted"
        );
        assert!(
            server
                .read_client_record(Duration::from_millis(250))
                .is_none(),
            "no client-auth or game record follows the TLS failure"
        );
    }

    endpoint.close(attempt.poll.registry());
    assert!(
        attempt.shared.is_closed(),
        "CertificateRequest failure closes locally"
    );
    server.stop();
    let sibling_game = b"certificate-request-failure-does-not-outage-b";
    sibling_server.send_record(
        ContentType::ApplicationData,
        &server_binary_frame(sibling_game),
    );
    take_game_payload(&mut sibling_attempt, &mut sibling_endpoint, sibling_game);
    sibling_endpoint.close(sibling_attempt.poll.registry());
    sibling_server.stop();
}

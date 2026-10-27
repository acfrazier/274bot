//! G2-D1: real reactor progress during an actual platform-verifier hold.

use super::*;
use client_adapter::control::{
    CloseCommand, CloseReason, ConnectCommand, PrimaryCommand, PrimaryReply,
};
use client_adapter::park::ParkOutcome;
use client_adapter::reactor::TransportService;
use client_adapter::resolver::ResolvedAddresses;
use client_adapter::tls_handshake::GameTlsWait;

#[derive(Debug)]
struct ThirdHandshakeHold {
    verifier: CountingPlatformVerifier,
    entered: Sender<Instant>,
    released: Mutex<bool>,
    changed: Condvar,
}

impl ThirdHandshakeHold {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

impl ServerCertVerifier for ThirdHandshakeHold {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        // The first two sessions are fully established before A starts.
        if self.verifier.audit.calls.load(Ordering::SeqCst) == 2 {
            self.entered.send(Instant::now()).unwrap();
            let released = self.released.lock().unwrap();
            let (released, _) = self
                .changed
                .wait_timeout_while(released, Duration::from_secs(45), |released| !*released)
                .unwrap();
            assert!(*released, "G2-D1 verifier release barrier timed out");
        }
        self.verifier
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.verifier
            .verify_tls12_signature(message, cert, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.verifier
            .verify_tls13_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.verifier.supported_verify_schemes()
    }
}

struct ReleaseHold(Arc<ThirdHandshakeHold>);
impl Drop for ReleaseHold {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct ReactorPeer {
    binding: BindingId,
    shared: Arc<SharedConnection>,
    retired: Arc<AttemptRetire>,
}

impl ReactorPeer {
    fn start(service: &Arc<TransportService>, address: std::net::SocketAddr) -> Self {
        let park = Arc::new(Park::new());
        let token = CancelSource::new(Arc::clone(&park)).child().unwrap();
        let retired = Arc::new(AttemptRetire::new(token.generation()));
        let deadline = Instant::now() + Duration::from_secs(30);
        let (binding, shared) = service
            .reserve(Arc::clone(&park), token.generation())
            .unwrap();
        let ticket = service
            .game_tls_waiter(park, deadline, token.clone())
            .unwrap();
        assert_eq!(ticket.poll(Instant::now()), GameTlsWait::Ready);
        let release = Arc::new(QueueReleaseProof::new(
            binding,
            service.tls_transfer_seq().unwrap(),
        ));
        let mut addresses = ResolvedAddresses::new();
        assert!(addresses.push(address));
        service
            .submit_connect(ConnectCommand {
                binding,
                transport: client::Transport::Wss,
                host: "localhost".to_owned(),
                port: address.port(),
                addresses,
                deadline: Some(deadline),
                cancel: token,
                retire: Arc::clone(&retired),
                queue_release: Some(release),
                tls_ticket: Some(ticket),
            })
            .unwrap();
        Self {
            binding,
            shared,
            retired,
        }
    }

    fn wait(&self, service: &TransportService, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + IO_TIMEOUT;
        loop {
            assert!(
                service.maintain(Instant::now()).is_none(),
                "false reactor loss"
            );
            if predicate() {
                return;
            }
            assert!(Instant::now() < deadline, "G2-D1 owner operation timed out");
            let result = self.shared.control.park.wait(
                deadline.min(Instant::now() + Duration::from_millis(50)),
                Instant::now,
                || service.health().loss().is_some(),
                || false,
                || false,
                || predicate().then_some(()),
                false,
                None,
            );
            if result == ParkOutcome::Ready(()) {
                return;
            }
        }
    }

    fn connect(&self, service: &TransportService) {
        self.wait(service, || self.shared.control.has_primary_reply());
        assert_eq!(
            self.shared.control.take_primary_reply(),
            Some(PrimaryReply::Connected {
                binding: self.binding
            })
        );
        self.shared
            .control
            .post_primary(
                PrimaryCommand::Seal {
                    binding: self.binding,
                },
                &service.bus(),
            )
            .unwrap();
        self.wait(service, || self.shared.control.has_primary_reply());
        assert!(matches!(
            self.shared.control.take_primary_reply(),
            Some(PrimaryReply::Sealed { binding, .. }) if binding == self.binding
        ));
        assert!(self.shared.commit_session(self.binding));
    }

    fn exchange(
        &self,
        service: &TransportService,
        server: &LocalServer,
        sequence: u64,
        update: bool,
    ) {
        let payload = sequence.to_be_bytes().to_vec();
        let command = if update {
            ServerCommand::SendBinaryAfterKeyUpdate(payload.clone())
        } else {
            ServerCommand::SendBinary(payload.clone())
        };
        server.commands.send(command).unwrap();
        let mut actual = Vec::new();
        while actual.len() < payload.len() {
            self.wait(service, || !self.shared.inbound.is_empty());
            let mut bytes = [0; 8];
            let count = self.shared.take_inbound(&mut bytes);
            actual.extend_from_slice(&bytes[..count]);
        }
        assert_eq!(actual, payload);
        let (reply, received) = mpsc::sync_channel(1);
        server
            .commands
            .send(ServerCommand::ExpectBinary {
                expected: payload.clone(),
                reply,
            })
            .unwrap();
        assert!(matches!(
            self.shared
                .append_record(self.binding, &payload, &service.bus()),
            client_adapter::shared::AppendResult::Accepted
        ));
        received.recv_timeout(IO_TIMEOUT).unwrap().unwrap();
        let (reply, received) = mpsc::sync_channel(1);
        server
            .commands
            .send(ServerCommand::Ping { payload, reply })
            .unwrap();
        received.recv_timeout(IO_TIMEOUT).unwrap().unwrap();
        assert!(!self.shared.is_closed());
        assert!(service.maintain(Instant::now()).is_none());
    }

    fn cancel(&self, service: &TransportService) {
        self.shared
            .control
            .post_close(
                CloseCommand {
                    binding: self.binding,
                    reason: CloseReason::Cancelled,
                },
                &service.bus(),
            )
            .unwrap();
        self.wait(service, || self.shared.control.has_close_reply());
        assert_eq!(self.shared.control.take_closed(), Some(self.binding));
        assert!(self.shared.is_closed());
        assert_eq!(self.retired.ack_count(), 1);
    }
}

#[test]
fn g2_d1_real_reactor_keeps_established_wss_live_during_6s_and_35s_verifier_holds() {
    for version in [ProtocolVersion::TLSv1_2, ProtocolVersion::TLSv1_3] {
        for seconds in [6, 35] {
            let pki = LocalPki::new();
            let suite = if version == ProtocolVersion::TLSv1_2 {
                CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            } else {
                CipherSuite::TLS13_AES_128_GCM_SHA256
            };
            let server_config = server_config(pki.valid_identity, version, suite);
            let audit = Arc::new(VerifierAudit::default());
            let (entered, entry) = mpsc::channel();
            let verifier = Arc::new(ThirdHandshakeHold {
                verifier: CountingPlatformVerifier {
                    delegate: Verifier::new_with_extra_roots(
                        [pki.trusted_root],
                        Arc::new(rustls::crypto::ring::default_provider()),
                    )
                    .unwrap(),
                    audit: Arc::clone(&audit),
                    hold: None,
                },
                entered,
                released: Mutex::new(false),
                changed: Condvar::new(),
            });
            let config = build_test_client_config_with_verifier(verifier.clone()).unwrap();
            let service = TransportService::start().unwrap();
            let lane = GameTlsLane::start_with_test_config(
                service.bus().reactor_waker(),
                service.health(),
                config,
            )
            .unwrap();
            service.install_tls_lane(Arc::clone(&lane));
            let healthy_server = LocalServer::spawn(Arc::clone(&server_config));
            let healthy = ReactorPeer::start(&service, healthy_server.address);
            healthy.connect(&service);
            let cancelled_server = LocalServer::spawn(Arc::clone(&server_config));
            let cancelled = ReactorPeer::start(&service, cancelled_server.address);
            cancelled.connect(&service);
            for server in [&healthy_server, &cancelled_server] {
                let handshake = server.handshake();
                assert_eq!(handshake.protocol, Some(version));
                assert_eq!(handshake.suite, Some(suite));
                assert!(handshake.failure.is_none());
            }
            healthy.exchange(&service, &healthy_server, 0, false);
            cancelled.exchange(&service, &cancelled_server, 0, false);
            let held_server =
                LocalServer::spawn_with_timeout(server_config, Duration::from_secs(40));
            let held = ReactorPeer::start(&service, held_server.address);
            let release = ReleaseHold(Arc::clone(&verifier));
            let entered_at = entry.recv_timeout(IO_TIMEOUT).unwrap();
            let mut cancelled_at = None;
            let mut sequence = 0;
            while entered_at.elapsed() < Duration::from_secs(seconds) {
                sequence += 1;
                healthy.exchange(
                    &service,
                    &healthy_server,
                    sequence,
                    version == ProtocolVersion::TLSv1_3,
                );
                if cancelled_at.is_none() && entered_at.elapsed() >= Duration::from_secs(1) {
                    cancelled.cancel(&service);
                    cancelled_at = Some(entered_at.elapsed());
                }
            }
            assert!(cancelled_at.is_some());
            assert!(
                !held.retired.acknowledged(),
                "held verifier still owns its socket"
            );
            if seconds == 35 {
                assert_eq!(
                    held.shared.control.take_primary_reply(),
                    Some(PrimaryReply::ConnectFailed {
                        binding: held.binding,
                        kind: io::ErrorKind::TimedOut,
                        errno: None,
                    }),
                    "initial TLS must report its deadline before verifier return"
                );
            }
            let released_at = entered_at.elapsed();
            verifier.release();
            if seconds == 6 {
                held.connect(&service);
                let handshake = held_server.handshake();
                assert_eq!(handshake.protocol, Some(version));
                assert_eq!(handshake.suite, Some(suite));
                assert!(handshake.failure.is_none());
                held.exchange(&service, &held_server, 0, false);
                held.cancel(&service);
            } else {
                held.wait(&service, || held.retired.acknowledged());
            }
            healthy.exchange(&service, &healthy_server, sequence + 1, false);
            audit.assert_game_worker_calls(3);
            healthy.cancel(&service);
            held_server.stop();
            cancelled_server.stop();
            healthy_server.stop();
            service.shutdown();
            lane.shutdown().unwrap();
            drop(release);
            println!("G2-D1 os={} version={version:?} suite={suite:?} hold_s={seconds} released_ms={} cancel_ms={} binary_rounds={sequence} pong_rounds={sequence} verifier_calls=3 result=PASS", std::env::consts::OS, released_at.as_millis(), cancelled_at.unwrap().as_millis());
        }
    }
}

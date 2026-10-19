//! A held TLS certificate verifier must not stop owner maintenance while a
//! cache fetch waits on the update server. This is a separate integration-test
//! binary because the process-scoped network service accepts its test TLS
//! configuration only before its first start.
//! The gate wraps a real WebPki verifier over a private test root; it holds
//! inside the HTTPS worker, not on the server. This is a focused deterministic
//! regression, not a platform-verifier soak.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::io::{JagFile, Packet};
use client::unpack::{self, FetchEndpoint, SnapshotPreparation};
use client::Transport;
use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};

const HOST: &str = "127.0.0.1";
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
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-held-verifier-{}-{}",
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

fn fixture_packs() -> HashMap<String, Vec<u8>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
    let mut members = Vec::new();
    for prefix in ["model", "anim", "midi", "map"] {
        members.push((format!("{prefix}_version"), vec![0, 1]));
        members.push((format!("{prefix}_crc"), 1u32.to_be_bytes().to_vec()));
        members.push((format!("{prefix}_index"), vec![0]));
    }
    let version_members: Vec<_> = members
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.clone()))
        .collect();
    JAG_SLOTS
        .into_iter()
        .map(|name| {
            let bytes = if name == "versionlist" {
                jag(&version_members)
            } else {
                std::fs::read(fixture.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
            };
            (name.to_string(), bytes)
        })
        .collect()
}

fn g3(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
}

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
    let mut packed = Vec::new();
    g3(&mut packed, raw.len());
    g3(&mut packed, compressed.len());
    packed.extend_from_slice(compressed);
    packed
}

fn crc_body(packs: &HashMap<String, Vec<u8>>) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (slot, name) in JAG_SLOTS.iter().enumerate() {
        let bytes = &packs[*name];
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

#[derive(Debug)]
struct HeldVerifierGate {
    entered: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
}

impl HeldVerifierGate {
    fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            released: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn wait_in_verifier(&self) -> bool {
        self.entered.store(true, Ordering::Release);
        let released = self.released.lock().unwrap();
        let (released, timeout) = self
            .changed
            .wait_timeout_while(released, Duration::from_secs(10), |released| !*released)
            .unwrap();
        *released || !timeout.timed_out()
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

#[derive(Debug)]
struct HeldServerCertVerifier {
    delegate: Arc<WebPkiServerVerifier>,
    gate: Arc<HeldVerifierGate>,
}

impl ServerCertVerifier for HeldServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !self.gate.wait_in_verifier() {
            return Err(rustls::Error::General(
                "test verifier gate timed out before maintenance released it".into(),
            ));
        }
        self.delegate.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.delegate.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.delegate.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.delegate.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.delegate.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[rustls::DistinguishedName]> {
        self.delegate.root_hint_subjects()
    }
}

struct UpdateServer {
    port: u16,
    stop: Arc<AtomicBool>,
    accept: Option<thread::JoinHandle<()>>,
}

impl UpdateServer {
    fn start(packs: Arc<HashMap<String, Vec<u8>>>, tls: Arc<rustls::ServerConfig>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let body = Arc::new(crc_body(&packs));
        let accept = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for accepted in listener.incoming() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(socket) = accepted else { continue };
                    serve(socket, Arc::clone(&tls), &body, &packs);
                }
            })
        };
        Self { port, stop, accept: Some(accept) }
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

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn serve(
    socket: TcpStream,
    tls: Arc<rustls::ServerConfig>,
    crc: &[u8],
    packs: &HashMap<String, Vec<u8>>,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = socket.set_write_timeout(Some(Duration::from_secs(5)));
    let connection = match rustls::ServerConnection::new(tls) {
        Ok(connection) => connection,
        Err(_) => return,
    };
    let mut stream: Box<dyn ReadWrite> = Box::new(rustls::StreamOwned::new(connection, socket));
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 || stream.read_exact(&mut byte).is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let request = String::from_utf8_lossy(&head);
    let path = request
        .split_ascii_whitespace()
        .nth(1)
        .unwrap_or_default()
        .trim_start_matches('/');
    let response = if path == "crc" {
        Some(crc)
    } else {
        JAG_SLOTS
            .iter()
            .find(|name| path.starts_with(*name))
            .and_then(|name| packs.get(*name))
            .map(Vec::as_slice)
    };
    let Some(response) = response else {
        let _ = write!(stream, "HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    let _ = write!(
        stream,
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    let _ = stream.write_all(response);
    let _ = stream.flush();
}

#[test]
fn maintenance_continues_while_the_update_tls_verifier_is_held() {
    let packs = Arc::new(fixture_packs());
    let identity = rcgen::generate_simple_self_signed(vec![HOST.to_owned()]).unwrap();
    let certificate = identity.cert.der().clone();
    let server_key = PrivateKeyDer::Pkcs8(identity.key_pair.serialize_der().into());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_tls = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], server_key)
            .unwrap(),
    );
    let roots = {
        let mut roots = RootCertStore::empty();
        roots.add(certificate.clone()).unwrap();
        Arc::new(roots)
    };
    let delegate = WebPkiServerVerifier::builder(roots).build().unwrap();
    let gate = Arc::new(HeldVerifierGate::new());
    let verifier: Arc<dyn ServerCertVerifier> = Arc::new(HeldServerCertVerifier {
        delegate,
        gate: Arc::clone(&gate),
    });
    let tls_config = client_adapter::tls_handshake::build_test_client_config_with_verifier(verifier)
        .expect("build held-verifier client TLS config");
    client_adapter::service::NetworkService::install_test_tls_config(tls_config)
        .expect("install held-verifier test TLS config before service start");
    let driver = client_adapter::service::NetworkService::start_composition()
        .expect("start test network service");

    let server = UpdateServer::start(Arc::clone(&packs), server_tls);
    let root = TempRoot::new();
    let cache_dir = root.0.join("cache");
    let out_dir = root.0.join("unpack");
    std::fs::create_dir_all(&cache_dir).unwrap();
    for (name, bytes) in packs.iter() {
        if name != "sounds" {
            std::fs::write(cache_dir.join(name), bytes).unwrap();
        }
    }

    let mut maintenance_calls = 0usize;
    let mut held_calls = 0usize;
    let prepared = unpack::prepare_snapshot_with_maintenance(
        &cache_dir.to_string_lossy(),
        &out_dir.to_string_lossy(),
        Some(FetchEndpoint {
            transport: Transport::Wss,
            host: HOST,
            port: server.port,
        }),
        None,
        |now| {
            let health = driver.maintain(now);
            assert_eq!(health.reactor, None, "reactor remains healthy during TLS verification");
            assert_eq!(health.resolver, None, "resolver remains healthy during TLS verification");
            maintenance_calls += 1;
            if gate.entered.load(Ordering::Acquire) {
                held_calls += 1;
                if held_calls == 4 {
                    gate.release();
                }
            }
        },
    );

    assert!(gate.entered.load(Ordering::Acquire), "the HTTPS lane invoked the verifier");
    assert_eq!(held_calls, 4, "four bounded host-maintenance turns occurred while verification was held");
    assert!(maintenance_calls >= held_calls);
    assert!(matches!(&*prepared, SnapshotPreparation::Ready { .. }), "cold fill completed after verifier release: {prepared:?}");
}

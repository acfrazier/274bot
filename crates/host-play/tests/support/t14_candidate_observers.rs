//! Candidate-only test-support observations. The baseline builds the identical
//! `t14_soak_live.rs` without this module or the `soak-observers` feature.
//! Only complete binary WSS payloads are hashed; no game bytes or credentials
//! enter receipts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use client_adapter::control::BindingId;
use client_adapter::service::NetworkService;
use client_adapter::soak_observer::{
    install_soak_observer, wss_ledger_samples, SoakDirection, WSS_RESERVED_TOTAL_BYTES,
};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use serde_json::json;
use sha2::{Digest, Sha256};

const EXPECTED_WSS_RESERVATION: usize = 250_760;

#[derive(Default)]
struct DigestStream {
    bytes: u64,
    messages: u64,
    hash: Sha256,
}

impl DigestStream {
    fn update(&mut self, payload: &[u8]) {
        self.bytes = self.bytes.saturating_add(payload.len() as u64);
        self.messages = self.messages.saturating_add(1);
        self.hash.update(payload);
    }

    fn record(&self) -> serde_json::Value {
        json!({
            "bytes": self.bytes,
            "messages": self.messages,
            "sha256": format!("{:x}", self.hash.clone().finalize()),
        })
    }
}

#[derive(Default)]
struct ConnectionDigest {
    inbound: DigestStream,
    outbound: DigestStream,
    /// Rebinds change the logical generation but keep the same TCP socket.
    /// Hash one physical stream by its local port to compare with proxy A.
    bindings: BTreeSet<(u64, u64, u64, u64)>,
}

pub(super) struct CandidateObservers {
    streams: Arc<Mutex<BTreeMap<Option<u16>, ConnectionDigest>>>,
    payload_output: Mutex<BufWriter<File>>,
    ledger_output: Mutex<BufWriter<File>>,
    last_payload_minute: Mutex<Option<u64>>,
}

/// Install a private trust root before Play starts the process network. The
/// production adapter never reads SSL_CERT_FILE; that env override is used
/// only by the unchanged dev native-TLS baseline.
pub(super) fn install_tls(ca_pem: &Path) -> Result<(), String> {
    let certificates = CertificateDer::pem_file_iter(ca_pem)
        .map_err(|error| format!("read isolated CA {}: {error}", ca_pem.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("parse isolated CA {}: {error}", ca_pem.display()))?;
    if certificates.len() != 1 {
        return Err(format!(
            "isolated CA {} has {} certificates, expected exactly one",
            ca_pem.display(),
            certificates.len()
        ));
    }
    let config = client_adapter::tls_handshake::build_test_client_config_with_roots(&certificates)
        .map_err(|error| format!("build isolated WSS trust config: {error}"))?;
    NetworkService::install_test_tls_config(config)
        .map_err(|error| format!("install isolated WSS trust config before Play startup: {error}"))
}

impl CandidateObservers {
    pub(super) fn install(output: &Path) -> Result<Self, String> {
        let payload_path = output.join("candidate-wss-payloads.jsonl");
        let payload_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&payload_path)
            .map_err(|error| {
                format!(
                    "candidate payload receipt {}: {error}",
                    payload_path.display()
                )
            })?;
        let ledger_path = output.join("candidate-five-compartment-ledger.jsonl");
        let ledger_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&ledger_path)
            .map_err(|error| {
                format!(
                    "candidate ledger receipt {}: {error}",
                    ledger_path.display()
                )
            })?;
        let streams = Arc::new(Mutex::new(BTreeMap::<Option<u16>, ConnectionDigest>::new()));
        let callback_streams = Arc::clone(&streams);
        install_soak_observer(move |binding: BindingId, local_port, direction, payload| {
            let mut streams = callback_streams.lock().unwrap();
            let stream = streams.entry(local_port).or_default();
            stream.bindings.insert((
                binding.incarnation,
                binding.connection,
                binding.binding,
                binding.attempt,
            ));
            match direction {
                SoakDirection::Inbound => stream.inbound.update(payload),
                SoakDirection::Outbound => stream.outbound.update(payload),
            }
        })
        .map_err(|error| format!("candidate payload observer: {error}"))?;
        Ok(Self {
            streams,
            payload_output: Mutex::new(BufWriter::new(payload_file)),
            ledger_output: Mutex::new(BufWriter::new(ledger_file)),
            last_payload_minute: Mutex::new(None),
        })
    }

    pub(super) fn sample(&self, elapsed_seconds: u64) -> Result<(), String> {
        self.sample_inner(elapsed_seconds, false)
    }

    pub(super) fn finish(&self, elapsed_seconds: u64) -> Result<(), String> {
        self.sample_inner(elapsed_seconds, true)
    }

    fn sample_inner(&self, elapsed_seconds: u64, force_payload: bool) -> Result<(), String> {
        // Do not hold the payload mutex while locking an endpoint's TlsIo:
        // the reactor can invoke the synchronous observer while holding TlsIo.
        let ledger = wss_ledger_samples();
        let mut ledger_output = self.ledger_output.lock().unwrap();
        for entry in ledger {
            if entry.reserved_total_bytes != EXPECTED_WSS_RESERVATION
                || entry.reserved_total_bytes != WSS_RESERVED_TOTAL_BYTES
                || entry.overrun
            {
                return Err(format!(
                    "WSS five-compartment ledger overrun for binding {:?}: {:?}",
                    entry.binding, entry
                ));
            }
            let value = json!({
                "elapsed_seconds": elapsed_seconds,
                "scope": "five_named_logical_compartments_only",
                "incarnation": entry.binding.incarnation,
                "connection": entry.binding.connection,
                "binding": entry.binding.binding,
                "attempt": entry.binding.attempt,
                "client_local_port": entry.local_port,
                "received_plaintext": {
                    "observed_bytes": entry.received_plaintext_bytes,
                    "reserved_bytes": entry.received_plaintext_reserved_bytes,
                },
                "pending_tls_output": {
                    "observed_bytes": entry.pending_tls_output_bytes,
                    "reserved_bytes": entry.pending_tls_output_reserved_bytes,
                },
                "handshake_deframer": {
                    "observed_bytes": serde_json::Value::Null,
                    "reserved_bytes": entry.handshake_deframer_reserved_bytes,
                },
                "outbound_ring": {
                    "observed_bytes": entry.outbound_ring_bytes,
                    "reserved_bytes": entry.outbound_ring_reserved_bytes,
                },
                "inbound_ring": {
                    "observed_bytes": entry.inbound_ring_bytes,
                    "reserved_bytes": entry.inbound_ring_reserved_bytes,
                },
                "reserved_total_bytes": entry.reserved_total_bytes,
                "actual_whole_connection_bytes": serde_json::Value::Null,
                "overrun": entry.overrun,
            });
            serde_json::to_writer(&mut *ledger_output, &value)
                .map_err(|error| format!("candidate ledger serialization: {error}"))?;
            ledger_output
                .write_all(b"\n")
                .map_err(|error| format!("candidate ledger write: {error}"))?;
        }
        ledger_output
            .flush()
            .map_err(|error| format!("candidate ledger flush: {error}"))?;
        drop(ledger_output);

        let minute = elapsed_seconds / 60;
        let mut last_minute = self.last_payload_minute.lock().unwrap();
        if !force_payload && *last_minute == Some(minute) {
            return Ok(());
        }
        *last_minute = Some(minute);
        let streams = self.streams.lock().unwrap();
        let mut output = self.payload_output.lock().unwrap();
        for (local_port, digest) in streams.iter() {
            if local_port.is_none() {
                return Err("candidate WSS payload has no physical socket port".to_owned());
            }
            let value = json!({
                "elapsed_seconds": elapsed_seconds,
                "client_local_port": local_port,
                "bindings": digest.bindings,
                "inbound": digest.inbound.record(),
                "outbound": digest.outbound.record(),
            });
            serde_json::to_writer(&mut *output, &value)
                .map_err(|error| format!("candidate payload serialization: {error}"))?;
            output
                .write_all(b"\n")
                .map_err(|error| format!("candidate payload write: {error}"))?;
        }
        output
            .flush()
            .map_err(|error| format!("candidate payload flush: {error}"))
    }
}

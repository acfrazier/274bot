//! Harness-only receipt barrier. It never infers closure from a quiet socket.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Default)]
struct Connection {
    closed: bool,
    websocket: bool,
    close_frames: [u32; 2],
    invalid_close: bool,
}

pub(super) struct CloseBarrier {
    path: PathBuf,
    offset: u64,
    connections: BTreeMap<String, Connection>,
}

impl CloseBarrier {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            connections: BTreeMap::new(),
        }
    }

    fn observe(&mut self, event: &Value) -> Result<(), String> {
        let kind = event["type"].as_str().unwrap_or("");
        if !matches!(
            kind,
            "connection_open" | "connection_close" | "websocket_close_frame"
        ) {
            return Ok(());
        }
        let id = event["connection_id"]
            .as_str()
            .ok_or("proxy connection event lacks ID")?;
        if kind == "connection_open" {
            if self
                .connections
                .insert(id.to_owned(), Connection::default())
                .is_some()
            {
                return Err(format!("duplicate proxy connection {id}"));
            }
            return Ok(());
        }
        let connection = self
            .connections
            .get_mut(id)
            .ok_or("proxy Close without connection_open")?;
        if kind == "connection_close" {
            if connection.closed {
                return Err(format!("duplicate proxy connection_close {id}"));
            }
            connection.closed = true;
            connection.websocket =
                event["websocket_handshake"]["upgrade_status"].as_u64() == Some(101);
            if event["closed_monotonic_ns"].as_u64().is_none() {
                return Err(format!("proxy connection {id} lacks precise Close time"));
            }
        } else {
            let direction = match event["direction"].as_str() {
                Some("client_to_engine") => 0,
                Some("engine_to_client") => 1,
                _ => return Err("proxy Close lacks direction".into()),
            };
            connection.close_frames[direction] += 1;
            connection.invalid_close |= event["payload_len"].as_u64() != Some(0);
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        !self.connections.is_empty()
            && self.connections.values().all(|connection| {
                connection.closed
                    && (!connection.websocket
                        || (connection.close_frames == [1, 1] && !connection.invalid_close))
            })
    }

    pub(super) fn poll(&mut self) -> Result<bool, String> {
        let mut file =
            File::open(&self.path).map_err(|error| format!("open Close receipts: {error}"))?;
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|error| format!("seek Close receipts: {error}"))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = reader
                .read_until(b'\n', &mut line)
                .map_err(|error| format!("read Close receipt: {error}"))?;
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            self.offset += read as u64;
            if line.len() > 1 {
                let event: Value = serde_json::from_slice(&line)
                    .map_err(|error| format!("parse Close receipt: {error}"))?;
                self.observe(&event)?;
            }
        }
        Ok(self.complete())
    }
}

pub(super) fn publish(
    output: &Path,
    completed: bool,
    barrier_ns: Option<u64>,
    error: Option<&str>,
) -> Result<(), String> {
    let path = output.join("summary.json");
    let mut summary: Value = serde_json::from_slice(
        &fs::read(&path).map_err(|e| format!("read summary for barrier: {e}"))?,
    )
    .map_err(|e| format!("parse summary for barrier: {e}"))?;
    summary["run"]["final_receive_close_barrier"] =
        json!(completed && barrier_ns.is_some() && error.is_none());
    summary["run"]["final_receive_close_barrier_monotonic_ns"] = json!(barrier_ns);
    summary["run"]["final_receive_close_barrier_error"] = json!(error);
    if error.is_some() {
        summary["result"] = json!("failed");
    }
    let temporary = output.join("summary.barrier.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&summary).map_err(|e| format!("serialize final barrier: {e}"))?,
    )
    .map_err(|e| format!("write final barrier: {e}"))?;
    fs::rename(temporary, path).map_err(|e| format!("publish final barrier: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> CloseBarrier {
        let mut barrier = CloseBarrier::new(PathBuf::new());
        barrier
            .observe(&json!({"type":"connection_open", "connection_id":"c1"}))
            .unwrap();
        barrier
    }

    fn close(barrier: &mut CloseBarrier, direction: &str, length: u64) {
        barrier.observe(&json!({"type":"websocket_close_frame", "connection_id":"c1", "direction":direction, "payload_len":length})).unwrap();
    }

    fn finish(barrier: &mut CloseBarrier) {
        barrier.observe(&json!({"type":"connection_close", "connection_id":"c1", "closed_monotonic_ns":123, "websocket_handshake":{"upgrade_status":101}})).unwrap();
    }

    #[test]
    fn final_barrier_accepts_client_first_close_only_after_engine_close_and_receipt() {
        let mut barrier = opened();
        assert!(!barrier.complete());
        close(&mut barrier, "client_to_engine", 0);
        assert!(!barrier.complete());
        close(&mut barrier, "engine_to_client", 0);
        assert!(!barrier.complete());
        finish(&mut barrier);
        assert!(barrier.complete());
    }

    #[test]
    fn final_barrier_accepts_engine_first_close_only_after_client_close_and_receipt() {
        let mut barrier = opened();
        close(&mut barrier, "engine_to_client", 0);
        assert!(!barrier.complete());
        close(&mut barrier, "client_to_engine", 0);
        assert!(!barrier.complete());
        finish(&mut barrier);
        assert!(barrier.complete());
    }

    #[test]
    fn final_barrier_rejects_missing_nonempty_and_duplicate_closes() {
        for (length, duplicate) in [(1, false), (0, true)] {
            let mut barrier = opened();
            close(&mut barrier, "client_to_engine", length);
            close(&mut barrier, "engine_to_client", 0);
            if duplicate {
                close(&mut barrier, "engine_to_client", 0);
            }
            finish(&mut barrier);
            assert!(!barrier.complete());
        }
        for direction in ["client_to_engine", "engine_to_client"] {
            let mut barrier = opened();
            close(&mut barrier, direction, 0);
            finish(&mut barrier);
            assert!(!barrier.complete());
        }
    }

    #[test]
    fn publication_never_promotes_incomplete_or_failed_run() {
        let output = std::env::temp_dir().join(format!("t14-barrier-{}", std::process::id()));
        fs::create_dir(&output).unwrap();
        for (completed, error, expected) in [
            (true, None, true),
            (false, None, false),
            (true, Some("missing bytes"), false),
        ] {
            fs::write(
                output.join("summary.json"),
                br#"{"result":"incomplete","run":{}}"#,
            )
            .unwrap();
            publish(&output, completed, Some(123), error).unwrap();
            let summary: Value =
                serde_json::from_slice(&fs::read(output.join("summary.json")).unwrap()).unwrap();
            assert_eq!(summary["run"]["final_receive_close_barrier"], expected);
            assert_eq!(
                summary["result"],
                if error.is_some() {
                    "failed"
                } else {
                    "incomplete"
                }
            );
        }
        fs::remove_dir_all(output).unwrap();
    }
}

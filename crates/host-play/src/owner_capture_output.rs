//! Worker-side bounded persistence. Never called from an owner borrow.
use api::owner_capture::{OwnerFragment, Reason, OWNER_JSONL_MAX, SCHEMA};
use std::io::Write;

// Serialize scalar rows directly, not a heap-allocated Value tree per field.
#[derive(serde::Serialize)]
struct FragmentRecord<'a> {
    schema: &'static str,
    kind: &'static str,
    request_id: u32,
    phase: u8,
    slot_token: u64,
    frame_serial: u64,
    source: &'static str,
    source_tick: u32,
    epochs: &'a [Option<api::owner_capture::OwnerEpoch>; 3],
    script_state: Option<&'static str>,
    fingerprint_present: Option<bool>,
    begin_ns: u64,
    end_ns: u64,
    visits: u32,
    complete: bool,
    reason: Reason,
    rows: &'a [api::owner_capture::FieldRow],
    coverage_unknown: &'static [&'static str],
}

#[derive(Default)]
struct ByteCount(usize);
impl Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("output overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct Output {
    file: std::fs::File,
    bytes: usize,
    sent: [bool; 3],
    received: [u8; 3],
    frames: [Option<u64>; 3],
    encoded: [bool; 4],
    stop: Option<(u64, u64)>,
    finished: bool,
}

impl Output {
    pub fn new(path: &std::path::Path) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self {
            file,
            bytes: 0,
            sent: [false; 3],
            received: [0; 3],
            frames: [None; 3],
            encoded: [false; 4],
            stop: None,
            finished: false,
        })
    }

    fn write(&mut self, value: &serde_json::Value) -> Result<(), Reason> {
        self.write_record(value, value["kind"] == "owner_terminal")
    }

    fn write_record(
        &mut self,
        value: &impl serde::Serialize,
        terminal: bool,
    ) -> Result<(), Reason> {
        let mut count = ByteCount::default();
        serde_json::to_writer(&mut count, value).map_err(|_| Reason::Malformed)?;
        let bytes = count.0.checked_add(1).ok_or(Reason::CapDrift)?;
        let limit = if terminal {
            OWNER_JSONL_MAX
        } else {
            OWNER_JSONL_MAX - 512
        };
        if self.bytes.checked_add(bytes).is_none_or(|n| n > limit) {
            return Err(Reason::CapDrift);
        }
        serde_json::to_writer(&mut self.file, value).map_err(|_| Reason::Partial)?;
        self.file.write_all(b"\n").map_err(|_| Reason::Partial)?;
        self.bytes += bytes;
        Ok(())
    }

    fn fragment(
        &mut self,
        f: OwnerFragment,
        token: u64,
        requests: [Option<u64>; 3],
    ) -> Result<(), Reason> {
        let phase = usize::from(f.phase);
        if phase >= 3
            || !self.sent[phase]
            || f.request_id != f.phase as u32 + 1
            || f.slot_token != token
        {
            return Err(Reason::WrongEpoch);
        }
        let bit = match f.source {
            "nav_snapshot" => 1,
            "slotscript" => 2,
            _ => return Err(Reason::Malformed),
        };
        if self.received[phase] & bit != 0 {
            return Err(Reason::Duplicate);
        }
        if self.frames[phase].is_some_and(|frame| frame != f.frame_serial) {
            return Err(Reason::StaleFrame);
        }
        self.frames[phase] = Some(f.frame_serial);
        self.received[phase] |= bit;
        self.write_record(
            &FragmentRecord {
                schema: SCHEMA,
                kind: "owner_fragment",
                request_id: f.request_id,
                phase: f.phase,
                slot_token: f.slot_token,
                frame_serial: f.frame_serial,
                source: f.source,
                source_tick: f.source_tick,
                epochs: &f.epochs,
                script_state: f.script_state,
                fingerprint_present: f.fingerprint_present,
                begin_ns: f.begin_ns,
                end_ns: f.end_ns,
                visits: f.visits,
                complete: f.complete,
                reason: f.reason,
                rows: &f.rows,
                coverage_unknown: f.coverage_unknown,
            },
            false,
        )?;
        if !f.complete || f.reason != Reason::Ok {
            return Err(Reason::Incomplete);
        }
        if f.end_ns < f.begin_ns || f.end_ns - f.begin_ns > 5_000_000 || f.visits > 262_144 {
            return Err(Reason::BudgetDeadline);
        }
        let issued = requests[phase].ok_or(Reason::Missing)?;
        if f.begin_ns < issued || f.end_ns > issued + 1_000_000_000 {
            return Err(Reason::WrongEpoch);
        }
        if f.source == "slotscript"
            && (f.script_state != Some(if phase == 2 { "Idle" } else { "Running" })
                || f.fingerprint_present != Some(phase != 2))
        {
            return Err(Reason::WrongEpoch);
        }
        Ok(())
    }

    pub fn record_stop(&mut self, begin: u64, end: u64) -> Result<(), Reason> {
        if self.stop.is_some() || end < begin {
            return Err(Reason::WrongEpoch);
        }
        self.stop = Some((begin, end));
        self.write(&serde_json::json!({"schema":SCHEMA,"kind":"script_stop","begin_ns":begin,"end_ns":end}))
    }

    pub fn finish(&mut self, failure: Option<Reason>) -> Result<(), Reason> {
        if self.finished {
            return Err(Reason::Duplicate);
        }
        let failure = failure.or_else(|| {
            (self.received != [3; 3] || self.encoded != [true; 4] || self.stop.is_none())
                .then_some(Reason::Missing)
        });
        // Reserve terminal receipt space even when a preceding row hit the cap.
        self.write(&serde_json::json!({"schema":SCHEMA,"kind":"owner_terminal", "complete":failure.is_none(),"reason":failure.unwrap_or(Reason::Ok)}))?;
        self.file.flush().map_err(|_| Reason::Partial)?;
        self.finished = true;
        failure.map_or(Ok(()), Err)
    }

    pub fn poll(
        &mut self,
        requests: [Option<u64>; 3],
        token: u64,
        now_ns: u64,
    ) -> Result<(), Reason> {
        for (phase, request) in requests.into_iter().enumerate() {
            if let Some(issued_ns) = request {
                if !self.sent[phase] {
                    self.write(&serde_json::json!({"schema":SCHEMA,"kind":"owner_request",
                        "phase":phase,"request_id":phase+1,"slot_token":token,"issued_ns":issued_ns,"deadline_ns":issued_ns+1_000_000_000}))?;
                    self.sent[phase] = true;
                }
            }
        }
        for f in host::owner_capture::global_mailbox().drain_fragments() {
            self.fragment(f, token, requests)?;
        }
        for e in host::owner_capture::global_mailbox().drain_encoded() {
            self.write(&serde_json::json!({"schema":SCHEMA,"kind":"encoded_buf_meta", "request_id":e.request_id,
                "slot_token":e.slot_token,"frame_serial":e.frame_serial,"buf_kind":e.kind as u8,
                "len":e.len,"capacity":e.capacity,"mono_ns":e.mono_ns}))?;
            use api::owner_capture::EncodedBufKind;
            if e.slot_token != token || e.len > e.capacity {
                return Err(Reason::WrongEpoch);
            }
            let index = match (e.kind, e.request_id) {
                (EncodedBufKind::InitialKeyframe, 0) => 0,
                (EncodedBufKind::FirstDelta, 0) if self.encoded[0] => 1,
                (EncodedBufKind::FirstPostInWindow, id @ 1..=2) => {
                    let issued = requests[id as usize - 1].ok_or(Reason::Missing)?;
                    if e.mono_ns < issued || e.mono_ns > issued + 2_000_000_000 {
                        return Err(Reason::WrongEpoch);
                    }
                    id as usize + 1
                }
                _ => return Err(Reason::Malformed),
            };
            if self.encoded[index] {
                return Err(Reason::Duplicate);
            }
            self.encoded[index] = true;
        }
        for (phase, issued) in requests.into_iter().enumerate() {
            if issued.is_some_and(|issued| now_ns.saturating_sub(issued) > 1_000_000_000)
                && self.received[phase] != 3
            {
                return Err(Reason::Missing);
            }
            if phase < 2
                && issued.is_some_and(|issued| now_ns.saturating_sub(issued) > 2_000_000_000)
                && !self.encoded[phase + 2]
            {
                return Err(Reason::Missing);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Output {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "owner-output-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let out = Output::new(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        out
    }

    #[test]
    fn completed_fragments_alone_never_certify_missing_encoded_rows() {
        let mut out = output();
        out.sent = [true; 3];
        out.received = [3; 3];
        out.poll([Some(0); 3], 1, 0).unwrap();
        assert!(
            !out.finished,
            "encoded metadata and normal teardown are still required"
        );
    }

    #[test]
    fn cumulative_owner_output_cap_is_not_the_run_cap() {
        let mut out = output();
        out.bytes = OWNER_JSONL_MAX - 1;
        assert_eq!(
            out.write(&serde_json::json!({"a":1})),
            Err(Reason::CapDrift)
        );
    }
}

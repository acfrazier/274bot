//! Host-play owner capture: COW ifaces, nav shell join, output (feature
//! `memory-owner-capture`).

use api::owner_capture::{
    owned_string_capacity_bytes, v_bytes, Budget, EncodedBufKind, EncodedBufMeta, FieldRow,
    OwnerFragment, Reason, SCHEMA,
};
use client::config::IfTypeMut;
use host::owner_capture::{self, CowScratch, OwnerRequest, SlotToken};
#[path = "owner_capture_output.rs"]
mod output;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

static REQUEST_SEQ: AtomicU32 = AtomicU32::new(1);

/// Init once: runtime env + reject snapshot-dedup combo.
pub fn init() -> Result<(), &'static str> {
    api::owner_capture::reject_dedup_combo()?;
    owner_capture::init_from_env();
    Ok(())
}

pub fn enabled() -> bool {
    owner_capture::enabled()
}

pub fn register_slot() -> SlotToken {
    let t = owner_capture::register_slot_token();
    owner_capture::bind_slot_token(t);
    t
}

/// Publish phase A/B/C request (nonblocking).
pub fn request_phase(phase: u8, token: SlotToken) -> Result<u32, Reason> {
    let id = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    owner_capture::global_mailbox().try_publish_request(OwnerRequest {
        request_id: id,
        phase,
        slot_token: token,
        armed_frame: 0,
    })?;
    Ok(id)
}

/// At observe-closure entry: census retained nav_snapshot (actual shell).
pub fn observe_entry_nav(
    nav_snapshot: &api::snapshot::GameSnapshot,
    token: SlotToken,
    template: &Arc<Vec<Option<Arc<IfTypeMut>>>>,
    overlay: &Arc<Vec<Option<Arc<IfTypeMut>>>>,
    scratch: &mut CowScratch,
) {
    if !enabled() {
        return;
    }
    let Some((mut host_frag, mut budget)) = owner_capture::take_staging_fragment() else {
        return;
    };
    if host_frag.slot_token != token.0 {
        return;
    }
    owner_capture::arm_script_request(&host_frag);
    if host_frag.phase < 2 {
        ENCODED_CAPTURE.with(|state| {
            state.borrow_mut().window = Some((host_frag.request_id, host_frag.frame_serial))
        });
    }
    let mut frag = nav_snapshot.owner_payload(&mut budget);
    frag.source = "nav_snapshot";
    frag.epochs = host_frag.epochs;
    frag.epochs[2] = Some(nav_snapshot.owner_epoch("nav_snapshot"));
    frag.begin_ns = host_frag.begin_ns;
    frag.slot_token = token.0;
    // Join with pre-observe staging if present (same frame best-effort).
    {
        frag.request_id = host_frag.request_id;
        frag.phase = host_frag.phase;
        frag.frame_serial = host_frag.frame_serial;
        // Keep each shell's rows; do not claim content equality.
        let mut combined = std::mem::take(&mut host_frag.rows);
        combined.extend(std::mem::take(&mut frag.rows).into_iter().map(|mut row| {
            row.owner = "nav_snapshot";
            row
        }));
        frag.rows = combined;
        frag.complete = frag.complete && host_frag.complete;
        if !host_frag.complete {
            frag.reason = host_frag.reason;
        }
    }
    let cow = account_ifaces_cow(template, overlay, scratch, &mut budget);
    frag.complete &= cow.complete;
    if !cow.complete {
        frag.reason = cow.reason;
    }
    frag.rows.extend(cow.rows);
    frag.visits = budget.visits();
    frag.end_ns = api::owner_capture::mono_ns();
    let _ = owner_capture::global_mailbox().try_push_fragment(frag);
}

/// Borrowed template + overlay COW accounting (no Arc strong copy).
pub fn account_ifaces_cow(
    template: &Arc<Vec<Option<Arc<IfTypeMut>>>>,
    overlay: &Arc<Vec<Option<Arc<IfTypeMut>>>>,
    scratch: &mut CowScratch,
    budget: &mut Budget,
) -> OwnerFragment {
    let mut frag = OwnerFragment::new("cow_ifaces");
    scratch.clear();
    let t0 = budget.elapsed_ns();

    let outer_shared = Arc::ptr_eq(template, overlay);
    let outer_cap = overlay.capacity() as u64;
    let outer_len = overlay.len() as u64;
    let template_bytes = match v_bytes(template.as_ref()) {
        Ok(bytes) => bytes,
        Err(reason) => {
            frag.complete = false;
            frag.reason = reason;
            return frag;
        }
    };
    let outer_bytes = match v_bytes(overlay.as_ref()) {
        Ok(b) => b,
        Err(r) => {
            frag.complete = false;
            frag.reason = r;
            return frag;
        }
    };
    let mut row = FieldRow::ok(
        "ifaces_mut",
        "outer",
        outer_len,
        outer_cap,
        outer_len,
        if outer_shared {
            0
        } else {
            (overlay.len() * std::mem::size_of::<Option<Arc<IfTypeMut>>>()) as u64
        },
        if outer_shared { 0 } else { outer_bytes },
        if outer_shared {
            0
        } else {
            std::mem::size_of::<Vec<Option<Arc<IfTypeMut>>>>() as u64
        },
        0,
        0,
        budget.elapsed_ns().saturating_sub(t0),
    );
    row.shared = Some(outer_shared);
    let _ = budget.push_row(&mut frag.rows, row);

    // The template remains alive even after the overlay diverges.
    {
        let _ = budget.push_row(
            &mut frag.rows,
            FieldRow::ok(
                "ifaces_mut",
                "template_outer_shared",
                template.len() as u64,
                template.capacity() as u64,
                template.len() as u64,
                (template.len() * std::mem::size_of::<Option<Arc<IfTypeMut>>>()) as u64,
                template_bytes,
                std::mem::size_of::<Vec<Option<Arc<IfTypeMut>>>>() as u64,
                0,
                0,
                budget.elapsed_ns().saturating_sub(t0),
            ),
        );
    }

    let mut private_bytes = 0u64;
    let mut shared_entries = 0u64;
    let mut private_entries = 0u64;
    let mut holders_dup = 0u64;

    let mut template_entries = 0u64;
    let mut template_bytes = 0u64;
    let result = (|| {
        // Index the complete template first, including aliases at other indices.
        for entry in template.iter() {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            if let Some(entry) = entry {
                let (new, _) = scratch.classify(Arc::as_ptr(entry) as usize, true, budget)?;
                if new {
                    template_entries = Budget::checked_add(template_entries, 1)?;
                    template_bytes =
                        Budget::checked_add(template_bytes, iftype_mut_bytes(entry, budget)?)?;
                }
            }
        }
        // Do not truncate a longer overlay to template.len().
        for entry in overlay.iter() {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            if let Some(entry) = entry {
                let (new, shared) = scratch.classify(Arc::as_ptr(entry) as usize, false, budget)?;
                if shared {
                    shared_entries = Budget::checked_add(shared_entries, 1)?;
                } else if new {
                    private_entries = Budget::checked_add(private_entries, 1)?;
                    private_bytes =
                        Budget::checked_add(private_bytes, iftype_mut_bytes(entry, budget)?)?;
                } else {
                    holders_dup = Budget::checked_add(holders_dup, 1)?;
                }
            }
        }
        Ok::<(), Reason>(())
    })();
    // Addresses never survive this read, including failed walks.
    scratch.clear();
    if let Err(reason) = result {
        frag.complete = false;
        frag.reason = reason;
        return frag;
    }
    let mut template_row = FieldRow::ok(
        "ifaces_mut",
        "template_entries",
        template_entries,
        template_entries,
        template_entries,
        template_bytes,
        template_bytes,
        0,
        0,
        0,
        budget.elapsed_ns(),
    );
    template_row.shared = Some(true);
    let _ = budget.push_row(&mut frag.rows, template_row);

    let mut priv_row = FieldRow::ok(
        "ifaces_mut",
        "private_entries",
        private_entries,
        private_entries,
        private_entries,
        private_entries * std::mem::size_of::<IfTypeMut>() as u64,
        private_entries * std::mem::size_of::<IfTypeMut>() as u64,
        private_bytes - private_entries * std::mem::size_of::<IfTypeMut>() as u64,
        0,
        0,
        budget.elapsed_ns().saturating_sub(t0),
    );
    priv_row.holder_count = Some(holders_dup.saturating_add(private_entries));
    priv_row.shared = Some(false);
    let _ = budget.push_row(&mut frag.rows, priv_row);

    let mut sh_row = FieldRow::ok(
        "ifaces_mut",
        "shared_entries",
        shared_entries,
        shared_entries,
        shared_entries,
        0,
        0,
        0,
        0,
        0,
        budget.elapsed_ns().saturating_sub(t0),
    );
    sh_row.shared = Some(true);
    let _ = budget.push_row(&mut frag.rows, sh_row);

    // Cache/immutable ifaces Arc nested payload unmeasured.
    let _ = budget.push_row(
        &mut frag.rows,
        FieldRow::unknown(
            "ifaces",
            "immutable_nested",
            Reason::OpaqueUnknown,
            budget.elapsed_ns().saturating_sub(t0),
        ),
    );

    if let Some(r) = budget.failed() {
        frag.complete = false;
        frag.reason = r;
    }
    frag.visits = budget.visits();
    frag.end_ns = budget.elapsed_ns();
    frag
}

fn iftype_mut_bytes(m: &IfTypeMut, budget: &mut Budget) -> Result<u64, Reason> {
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut n = std::mem::size_of::<IfTypeMut>() as u64;
    n = Budget::checked_add(n, owned_string_capacity_bytes(&m.text))?;
    n = Budget::checked_add(n, owned_string_capacity_bytes(&m.graphic_name))?;
    if let Some(v) = &m.link_obj_type {
        n = Budget::checked_add(n, v_bytes(v)?)?;
    }
    if let Some(v) = &m.link_obj_number {
        n = Budget::checked_add(n, v_bytes(v)?)?;
    }
    Ok(n)
}

#[derive(Default)]
struct EncodedCapture {
    initial: bool,
    delta: bool,
    window: Option<(u32, u64)>,
}

thread_local! {
    static ENCODED_CAPTURE: std::cell::RefCell<EncodedCapture> = std::cell::RefCell::new(EncodedCapture::default());
}

/// Record only naturally encountered keyframe/delta/window metadata.
pub fn note_encoded(keyframe: bool, len: usize, capacity: usize) {
    if !enabled() {
        return;
    }
    ENCODED_CAPTURE.with(|state| {
        let mut state = state.borrow_mut();
        let kind = if keyframe && !state.initial {
            state.initial = true;
            Some(EncodedBufKind::InitialKeyframe)
        } else if !keyframe && state.initial && !state.delta {
            state.delta = true;
            Some(EncodedBufKind::FirstDelta)
        } else {
            None
        };
        if let Some(kind) = kind {
            let _ = owner_capture::global_mailbox().try_push_encoded(EncodedBufMeta {
                kind,
                mono_ns: api::owner_capture::mono_ns(),
                slot_token: owner_capture::current_slot_token().map_or(0, |t| t.0),
                len: len as u64,
                capacity: capacity as u64,
                frame_serial: owner_capture::current_frame(),
                request_id: 0,
            });
        }
        if let Some((request_id, _)) = state.window.take() {
            let _ = owner_capture::global_mailbox().try_push_encoded(EncodedBufMeta {
                kind: EncodedBufKind::FirstPostInWindow,
                mono_ns: api::owner_capture::mono_ns(),
                slot_token: owner_capture::current_slot_token().map_or(0, |t| t.0),
                len: len as u64,
                capacity: capacity as u64,
                frame_serial: owner_capture::current_frame(),
                request_id,
            });
        }
    });
}

/// Serialize drained fragments after borrows end (harness poll).
pub fn drain_jsonl_lines() -> Vec<String> {
    let frags = owner_capture::global_mailbox().drain_fragments();
    let encoded = owner_capture::global_mailbox().drain_encoded();
    let mut lines = Vec::new();
    for f in frags {
        lines.push(fragment_to_jsonl(&f));
    }
    for e in encoded {
        lines.push(format!(
            r#"{{"schema":"{SCHEMA}","kind":"encoded_buf","buf_kind":"{:?}","len":{},"capacity":{}}}"#,
            e.kind, e.len, e.capacity
        ));
    }
    lines
}

fn fragment_to_jsonl(f: &OwnerFragment) -> String {
    let mut rows = String::from("[");
    for (i, r) in f.rows.iter().enumerate() {
        if i > 0 {
            rows.push(',');
        }
        rows.push_str(&format!(
            r#"{{"owner":"{}","field":"{}","complete":{},"reason":"{}","element_len":{},"element_capacity":{},"occupied_count":{},"occupied_element_bytes":{},"capacity_bytes":{},"nested_capacity_bytes":{},"box_count":{},"box_bytes":{},"elapsed_observer_ns":{}}}"#,
            r.owner,
            r.field,
            r.complete,
            r.reason.as_str(),
            opt_u(r.element_len),
            opt_u(r.element_capacity),
            opt_u(r.occupied_count),
            opt_u(r.occupied_element_bytes),
            opt_u(r.capacity_bytes),
            opt_u(r.nested_capacity_bytes),
            opt_u(r.box_count),
            opt_u(r.box_bytes),
            r.elapsed_observer_ns,
        ));
    }
    rows.push(']');
    format!(
        r#"{{"schema":"{SCHEMA}","source":"{}","slot_token":{},"request_id":{},"frame_serial":{},"phase":{},"source_tick":{},"complete":{},"reason":"{}","visits":{},"rows":{rows}}}"#,
        f.source,
        f.slot_token,
        f.request_id,
        f.frame_serial,
        f.phase,
        f.source_tick,
        f.complete,
        f.reason.as_str(),
        f.visits,
    )
}

fn opt_u(v: Option<u64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "null".into(),
    }
}

/// Harness phase publisher used from memory::Run::poll.
#[derive(Debug, Default)]
pub struct PhasePublisher {
    output: Option<output::Output>,
    requests: [Option<u64>; 3],
    pub token: Option<SlotToken>,
    pub a_at_s: f64,
    pub b_at_s: f64,
    pub c_at_teardown_s: f64,
    pub a_sent: bool,
    pub b_sent: bool,
    pub c_sent: bool,
    pub observe_start: Option<std::time::Instant>,
    pub teardown_start: Option<std::time::Instant>,
    pub failure: Option<Reason>,
}

impl PhasePublisher {
    pub fn with_output(path: &std::path::Path) -> Result<Self, String> {
        Ok(Self {
            token: None,
            output: Some(output::Output::new(path)?),
            ..Self::new(SlotToken(0))
        })
    }
    pub fn new(token: SlotToken) -> Self {
        Self {
            token: Some(token),
            a_at_s: 30.0,
            b_at_s: 90.0,
            c_at_teardown_s: 30.0,
            ..Default::default()
        }
    }

    /// Nonblocking: publish A/B at observe offsets; C during teardown. Never waits.
    pub fn poll(&mut self, observing: bool, teardown: bool) {
        if !enabled() || self.failure.is_some() {
            return;
        }
        self.poll_at(
            std::time::Instant::now(),
            observing,
            teardown,
            request_phase,
        );
        if let (Some(output), Some(token)) = (&mut self.output, self.token) {
            if let Err(reason) = output.poll(self.requests, token.0, api::owner_capture::mono_ns())
            {
                self.failure = Some(reason);
            }
        }
    }

    pub fn record_stop(&mut self, begin: u64, end: u64) {
        if let Some(output) = &mut self.output {
            if let Err(reason) = output.record_stop(begin, end) {
                self.failure.get_or_insert(reason);
            }
        }
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.output
            .as_mut()
            .ok_or("owner output missing")?
            .finish(self.failure)
            .map_err(|reason| format!("owner capture unqualified: {}", reason.as_str()))
    }

    fn poll_at(
        &mut self,
        now: std::time::Instant,
        observing: bool,
        teardown: bool,
        mut publish: impl FnMut(u8, SlotToken) -> Result<u32, Reason>,
    ) {
        let Some(token) = self.token else {
            return;
        };
        if observing {
            let Some(start) = self.observe_start else {
                return;
            };
            let s = now.saturating_duration_since(start).as_secs_f64();
            if !self.a_sent && s >= self.a_at_s {
                self.requests[0] = Some(api::owner_capture::mono_ns());
                if let Err(reason) = publish(0, token) {
                    self.failure = Some(reason);
                }
                self.a_sent = true;
            }
            if !self.b_sent && s >= self.b_at_s {
                self.requests[1] = Some(api::owner_capture::mono_ns());
                if let Err(reason) = publish(1, token) {
                    self.failure = Some(reason);
                }
                self.b_sent = true;
            }
        }
        if teardown {
            let Some(start) = self.teardown_start else {
                return;
            };
            if !self.c_sent
                && now.saturating_duration_since(start).as_secs_f64() >= self.c_at_teardown_s
            {
                if let Err(reason) = publish(2, token) {
                    self.failure = Some(reason);
                }
                self.requests[2] = Some(api::owner_capture::mono_ns());
                self.c_sent = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::owner_capture::{CaptureConfig, COW_SCRATCH_CAP};
    use std::sync::Arc;

    #[test]
    fn phases_use_existing_boundaries_and_never_retry_busy_mailbox() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut phases = PhasePublisher::new(SlotToken(42));
        phases.observe_start = Some(start);
        let mut requests = Vec::new();
        for seconds in [0, 29, 30, 31, 89, 90, 119] {
            phases.poll_at(
                start + Duration::from_secs(seconds),
                true,
                false,
                |phase, token| {
                    requests.push((phase, token));
                    Err(Reason::MailboxFull)
                },
            );
        }
        assert_eq!(requests, [(0, SlotToken(42)), (1, SlotToken(42))]);
        let stop = start + Duration::from_secs(120);
        phases.teardown_start = Some(stop);
        for seconds in [0, 29, 30, 31, 60] {
            phases.poll_at(
                stop + Duration::from_secs(seconds),
                false,
                true,
                |phase, token| {
                    requests.push((phase, token));
                    Ok(3)
                },
            );
        }
        assert_eq!(
            requests,
            [(0, SlotToken(42)), (1, SlotToken(42)), (2, SlotToken(42))]
        );
        assert_eq!(phases.failure, Some(Reason::MailboxFull));
        assert_eq!(phases.teardown_start, Some(stop));
    }

    #[test]
    fn cow_shared_outer_marked() {
        let mut budget = Budget::new();
        let mut scratch = CowScratch::new();
        let template: Arc<Vec<Option<Arc<IfTypeMut>>>> = Arc::new(vec![
            Some(Arc::new(IfTypeMut {
                text: "a".into(),
                ..Default::default()
            })),
            None,
        ]);
        let overlay = Arc::clone(&template);
        let frag = account_ifaces_cow(&template, &overlay, &mut scratch, &mut budget);
        let outer = frag.rows.iter().find(|r| r.field == "outer").unwrap();
        assert_eq!(outer.shared, Some(true));
    }

    #[test]
    fn cow_diverged_private_counts_text() {
        let mut budget = Budget::new();
        let mut scratch = CowScratch::new();
        let shared = Arc::new(IfTypeMut::default());
        let template: Arc<Vec<Option<Arc<IfTypeMut>>>> =
            Arc::new(vec![Some(Arc::clone(&shared)), Some(Arc::clone(&shared))]);
        let private = Arc::new(IfTypeMut {
            text: "private-label".into(),
            ..Default::default()
        });
        let overlay: Arc<Vec<Option<Arc<IfTypeMut>>>> =
            Arc::new(vec![Some(private), Some(Arc::clone(&shared))]);
        let frag = account_ifaces_cow(&template, &overlay, &mut scratch, &mut budget);
        let priv_row = frag
            .rows
            .iter()
            .find(|r| r.field == "private_entries")
            .unwrap();
        assert!(priv_row.occupied_count.unwrap() >= 1);
        assert!(priv_row.nested_capacity_bytes.unwrap_or(0) >= "private-label".len() as u64);
    }

    #[test]
    fn scratch_cap_is_fixed() {
        let s = CowScratch::new();
        assert!(s.reserved_bytes() >= COW_SCRATCH_CAP * std::mem::size_of::<usize>());
    }

    #[test]
    fn capture_config_off_default() {
        let c = CaptureConfig::off();
        assert!(!c.enabled);
    }
}

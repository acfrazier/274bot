//! Host-play owner capture: COW ifaces, nav shell join, output (feature
//! `memory-owner-capture`).

use api::owner_capture::{
    owned_string_capacity_bytes, v_bytes, Budget, CaptureConfig, EncodedBufKind, EncodedBufMeta,
    FieldRow, OwnerFragment, Reason, SCHEMA, COW_SCRATCH_CAP,
};
use client::config::IfTypeMut;
use host::owner_capture::{self, CowScratch, OwnerRequest, SlotToken};
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
pub fn observe_entry_nav(nav_snapshot: &api::snapshot::GameSnapshot, token: SlotToken) {
    if !enabled() {
        return;
    }
    let _ = token;
    let mut budget = Budget::new();
    let mut frag = nav_snapshot.owner_payload(&mut budget);
    frag.source = "nav_snapshot";
    frag.slot_token = token.0;
    // Join with pre-observe staging if present (same frame best-effort).
    if let Some(mut host_frag) = owner_capture::take_staging_fragment() {
        frag.request_id = host_frag.request_id;
        frag.phase = host_frag.phase;
        frag.frame_serial = host_frag.frame_serial;
        // Keep each shell's rows; do not claim content equality.
        let mut combined = std::mem::take(&mut host_frag.rows);
        combined.append(&mut frag.rows);
        frag.rows = combined;
        frag.complete = frag.complete && host_frag.complete;
        if !host_frag.complete {
            frag.reason = host_frag.reason;
        }
    }
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
        outer_bytes,
        outer_bytes,
        0,
        0,
        0,
        budget.elapsed_ns().saturating_sub(t0),
    );
    row.shared = Some(outer_shared);
    let _ = budget.push_row(&mut frag.rows, row);

    // Count template storage once when shared outer.
    if outer_shared {
        let _ = budget.push_row(
            &mut frag.rows,
            FieldRow::ok(
                "ifaces_mut",
                "template_outer_shared",
                outer_len,
                outer_cap,
                outer_len,
                outer_bytes,
                outer_bytes,
                0,
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

    let n = overlay.len().min(template.len());
    for i in 0..n {
        if !budget.visit() {
            frag.complete = false;
            frag.reason = budget.failed().unwrap_or(Reason::BudgetVisits);
            break;
        }
        let o = &overlay[i];
        let t = &template[i];
        match (o, t) {
            (Some(oa), Some(ta)) if Arc::ptr_eq(oa, ta) => {
                shared_entries += 1;
                // Count once in template only — record identity in scratch.
                let addr = Arc::as_ptr(oa) as usize;
                match scratch.insert(addr) {
                    Ok(true) => {
                        // first holder of this identity (template side)
                    }
                    Ok(false) => holders_dup += 1,
                    Err(r) => {
                        frag.complete = false;
                        frag.reason = r;
                        break;
                    }
                }
            }
            (Some(oa), _) => {
                // Private or diverged — check alias to another template entry first.
                let addr = Arc::as_ptr(oa) as usize;
                let mut aliased = false;
                for tj in template.iter().flatten() {
                    if Arc::as_ptr(tj) as usize == addr {
                        aliased = true;
                        break;
                    }
                }
                if aliased {
                    shared_entries += 1;
                    match scratch.insert(addr) {
                        Ok(false) => holders_dup += 1,
                        Ok(true) => {}
                        Err(r) => {
                            frag.complete = false;
                            frag.reason = r;
                            break;
                        }
                    }
                    continue;
                }
                match scratch.insert(addr) {
                    Ok(true) => {
                        private_entries += 1;
                        match iftype_mut_bytes(oa, budget) {
                            Ok(b) => private_bytes = private_bytes.saturating_add(b),
                            Err(r) => {
                                frag.complete = false;
                                frag.reason = r;
                                break;
                            }
                        }
                    }
                    Ok(false) => {
                        // Duplicate private identity — count one allocation, bump holders.
                        holders_dup += 1;
                    }
                    Err(r) => {
                        frag.complete = false;
                        frag.reason = r;
                        break;
                    }
                }
            }
            (None, _) => {}
        }
    }

    let mut priv_row = FieldRow::ok(
        "ifaces_mut",
        "private_entries",
        private_entries,
        private_entries,
        private_entries,
        private_bytes,
        private_bytes,
        private_bytes,
        private_entries,
        private_bytes,
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

/// Record natural encoded buffer metadata (no clone of bytes).
pub fn note_encoded(kind: EncodedBufKind, len: usize, capacity: usize) {
    if !enabled() {
        return;
    }
    let meta = EncodedBufMeta {
        kind,
        len: len as u64,
        capacity: capacity as u64,
        frame_serial: 0,
        request_id: 0,
    };
    let _ = owner_capture::global_mailbox().try_push_encoded(meta);
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
    pub token: Option<SlotToken>,
    pub a_at_s: f64,
    pub b_at_s: f64,
    pub c_at_teardown_s: f64,
    pub a_sent: bool,
    pub b_sent: bool,
    pub c_sent: bool,
    pub observe_start: Option<std::time::Instant>,
    pub teardown_start: Option<std::time::Instant>,
}

impl PhasePublisher {
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
        if !enabled() {
            return;
        }
        let Some(token) = self.token else {
            return;
        };
        if observing {
            let start = *self
                .observe_start
                .get_or_insert_with(std::time::Instant::now);
            let s = start.elapsed().as_secs_f64();
            if !self.a_sent && s >= self.a_at_s {
                let _ = request_phase(0, token);
                self.a_sent = true;
            }
            if !self.b_sent && s >= self.b_at_s {
                let _ = request_phase(1, token);
                self.b_sent = true;
            }
        }
        if teardown {
            let start = *self
                .teardown_start
                .get_or_insert_with(std::time::Instant::now);
            if !self.c_sent && start.elapsed().as_secs_f64() >= self.c_at_teardown_s {
                let _ = request_phase(2, token);
                self.c_sent = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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

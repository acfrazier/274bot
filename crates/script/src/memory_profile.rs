//! Passive per-isolate accounting. Snapshot gauges cover queued and decoding
//! buffers, including capacity; no extra channel messages or JS probes.
//!
//! When `BOT_MEMORY_DIAGNOSTICS=1` or `BOT_MEMORY_FAILURE_CAPTURE=1` is resolved
//! once at isolate creation, a bounded optional stop-reason string may be
//! retained on this Arc for the `memory_progress` payload only. It is never
//! part of [`Counters::snapshot`]. Failure-capture enables this cache without
//! the host-play periodic diagnostic sidecar.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering::Relaxed}};

/// Max Unicode scalars retained for an opt-in diagnostic stop reason.
pub const STOP_REASON_CAP: usize = 1024;

/// Opt-in diagnostic capture of `ScriptRunner.stop` / `host.stopReason`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StopReasonCapture {
    /// No stop observed yet, or stop-reason capture was off (nothing written).
    #[default]
    Absent,
    /// Capture on; own data-property string missing, non-string, accessor, or eval failed.
    Unavailable,
    /// Own data-property string (may be empty); length ≤ [`STOP_REASON_CAP`].
    Value(String),
}

/// Bounded failure-only context captured before V8 termination is cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureAttribution {
    pub tick: u64,
    pub call_path: &'static str,
    pub error_variant: String,
    pub error_debug: String,
    pub terminating_before_cancel: bool,
    pub interrupt_id: Option<u64>,
}

#[derive(Default)]
pub struct Counters {
    pub bytes: AtomicU64,
    pub capacity: AtomicU64,
    pub peak_capacity: AtomicU64,
    pub heap_live: AtomicU64,
    pub heap_used: AtomicU64,
    pub heap_total: AtomicU64,
    pub heap_samples: AtomicU64,
    pub heap_updated_ms: AtomicU64,
    pub tick_count: AtomicU64,
    pub tick_ns: AtomicU64,
    pub tick_max_ns: AtomicU64,
    /// Resolved once at isolate creation from `BOT_MEMORY_DIAGNOSTICS=1`,
    /// `BOT_MEMORY_FAILURE_CAPTURE=1`, or test enable. Gates stop-reason cache only.
    stop_reason_capture: AtomicBool,
    /// Bounded stop reason for progress payloads only; not retained on snapshot rows.
    stop_reason: Mutex<StopReasonCapture>,
    interrupt_id: AtomicU64,
    interrupt_tick: AtomicU64,
    failure_attribution: Mutex<Option<FailureAttribution>>,
}
impl Counters {
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"snapshot_inflight_bytes":self.bytes.load(Relaxed),
            "snapshot_inflight_capacity":self.capacity.load(Relaxed),
            "snapshot_peak_capacity":self.peak_capacity.load(Relaxed),
            "v8_live":self.heap_live.load(Relaxed),"v8_used_bytes":self.heap_used.load(Relaxed),"v8_total_bytes":self.heap_total.load(Relaxed),
            "v8_heap_samples":self.heap_samples.load(Relaxed),"v8_updated_ms":self.heap_updated_ms.load(Relaxed),
            "script_tick_count":self.tick_count.load(Relaxed),"script_tick_total_ns":self.tick_ns.load(Relaxed),
            "script_tick_max_ns":self.tick_max_ns.load(Relaxed)})
    }
    pub fn tick(&self, elapsed: std::time::Duration) {
        let ns=elapsed.as_nanos().min(u64::MAX as u128) as u64;
        self.tick_ns.fetch_add(ns,Relaxed);self.tick_max_ns.fetch_max(ns,Relaxed);self.tick_count.fetch_add(1,Relaxed);
    }
    /// Enable bounded stop-reason retention (diagnostics and/or failure-capture).
    pub fn enable_diagnostics(&self) {
        self.stop_reason_capture.store(true, Relaxed);
    }
    /// Same as [`Self::enable_diagnostics`]; name for failure-only harness mode.
    pub fn enable_stop_reason_capture(&self) {
        self.enable_diagnostics();
    }
    pub fn diagnostics_enabled(&self) -> bool {
        self.stop_reason_capture.load(Relaxed)
    }
    pub fn stop_reason_capture_enabled(&self) -> bool {
        self.diagnostics_enabled()
    }
    /// Write once on the first stop observation; later calls are no-ops.
    pub fn record_stop_reason(&self, capture: StopReasonCapture) {
        let mut slot = self.stop_reason.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*slot, StopReasonCapture::Absent) {
            *slot = capture;
        }
    }
    pub fn stop_reason(&self) -> StopReasonCapture {
        self.stop_reason.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Bound a captured string to [`STOP_REASON_CAP`] Unicode scalars.
    pub fn bound_stop_reason(s: String) -> String {
        if s.chars().count() <= STOP_REASON_CAP {
            s
        } else {
            s.chars().take(STOP_REASON_CAP).collect()
        }
    }

    /// Allocate a monotonic identity for a host-issued interrupt.
    pub fn next_interrupt_id(&self, tick: u64) -> u64 {
        self.interrupt_tick.store(tick, Relaxed);
        self.interrupt_id.fetch_add(1, Relaxed) + 1
    }

    pub fn interrupt_id_for_tick(&self, tick: u64) -> Option<u64> {
        if self.interrupt_tick.load(Relaxed) == tick {
            self.latest_interrupt_id()
        } else {
            None
        }
    }

    pub fn latest_interrupt_id(&self) -> Option<u64> {
        match self.interrupt_id.load(Relaxed) {
            0 => None,
            id => Some(id),
        }
    }

    /// Keep only the first bounded failure attribution.
    pub fn record_failure_attribution(&self, mut attribution: FailureAttribution) {
        attribution.error_debug = Self::bound_stop_reason(attribution.error_debug);
        let mut slot = self.failure_attribution.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(attribution);
        }
    }

    pub fn failure_attribution(&self) -> Option<FailureAttribution> {
        self.failure_attribution
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
pub struct SnapshotLease { counters: Arc<Counters>, len:u64, capacity:u64 }
impl SnapshotLease {
    pub fn new(counters:Arc<Counters>, len:usize, capacity:usize)->Self {
        counters.bytes.fetch_add(len as u64,Relaxed);
        let current=counters.capacity.fetch_add(capacity as u64,Relaxed)+capacity as u64;
        counters.peak_capacity.fetch_max(current,Relaxed);
        Self{counters,len:len as u64,capacity:capacity as u64}
    }
}
impl Drop for SnapshotLease {
    fn drop(&mut self) { self.counters.bytes.fetch_sub(self.len,Relaxed);self.counters.capacity.fetch_sub(self.capacity,Relaxed); }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn failure_attribution_is_bounded_first_only_and_capture_off_is_empty() {
        let c = Arc::new(Counters::default());
        assert!(c.failure_attribution().is_none());
        c.record_failure_attribution(FailureAttribution {
            tick: 7,
            call_path: "sync",
            error_variant: "JsError".into(),
            error_debug: "x".repeat(STOP_REASON_CAP + 1),
            terminating_before_cancel: true,
            interrupt_id: Some(3),
        });
        c.record_failure_attribution(FailureAttribution {
            tick: 8,
            call_path: "async-parked",
            error_variant: "Runtime".into(),
            error_debug: "later".into(),
            terminating_before_cancel: false,
            interrupt_id: None,
        });
        let value = c.failure_attribution().expect("first failure retained");
        assert_eq!(value.tick, 7);
        assert_eq!(value.call_path, "sync");
        assert_eq!(value.interrupt_id, Some(3));
        assert_eq!(value.error_debug.chars().count(), STOP_REASON_CAP);
        let off = Counters::default();
        assert!(off.failure_attribution().is_none());
    }

    #[test] fn thread_ownership_survives_handle_release_without_registry_retention() {
        let counters=registered();let weak=Arc::downgrade(&counters);
        let (tx,rx)=std::sync::mpsc::channel();
        let thread=std::thread::spawn(move || {let _owner=counters;rx.recv().unwrap();});
        assert!(weak.upgrade().is_some());
        tx.send(()).unwrap();thread.join().unwrap();
        assert!(weak.upgrade().is_none());
    }
    #[test] fn runtime_teardown_clears_heap_gauges() {
        let c=Arc::new(Counters::default());c.heap_live.store(1,Relaxed);c.heap_used.store(12,Relaxed);c.heap_total.store(16,Relaxed);
        drop(HeapLifetime(c.clone()));assert_eq!(c.heap_live.load(Relaxed),0);assert_eq!(c.heap_used.load(Relaxed),0);assert_eq!(c.heap_total.load(Relaxed),0);
    }
    #[test] fn snapshot_lifetime_counts_capacity_and_releases_on_channel_drop() {
        let c=Arc::new(Counters::default());
        let (tx,rx)=std::sync::mpsc::channel();
        tx.send(SnapshotLease::new(c.clone(),3,16)).unwrap();
        tx.send(SnapshotLease::new(c.clone(),5,32)).unwrap();
        assert_eq!(c.bytes.load(Relaxed),8);assert_eq!(c.capacity.load(Relaxed),48);
        let consumed=rx.recv().unwrap();assert_eq!(c.capacity.load(Relaxed),48);
        drop(consumed);assert_eq!(c.capacity.load(Relaxed),32);
        drop(rx);assert_eq!(c.capacity.load(Relaxed),0);
        assert!(tx.send(SnapshotLease::new(c.clone(),1,8)).is_err());
        assert_eq!(c.bytes.load(Relaxed),0);assert_eq!(c.capacity.load(Relaxed),0);
    }
}

pub struct HeapLifetime(pub Arc<Counters>);
impl Drop for HeapLifetime { fn drop(&mut self) { self.0.heap_used.store(0,Relaxed);self.0.heap_total.store(0,Relaxed);self.0.heap_live.store(0,Relaxed); } }

static REGISTRY: std::sync::OnceLock<std::sync::Mutex<Vec<std::sync::Weak<Counters>>>> = std::sync::OnceLock::new();
pub fn registered() -> Arc<Counters> {
    let counters=Arc::new(Counters::default());
    let mut rows=REGISTRY.get_or_init(Default::default).lock().unwrap();
    rows.retain(|r|r.strong_count()>0);rows.push(Arc::downgrade(&counters));counters
}
/// Includes detached isolate threads until their real resources are released.
/// Weak registration never keeps an isolate or its counters alive.
pub fn snapshots() -> Vec<serde_json::Value> {
    let mut rows=REGISTRY.get_or_init(Default::default).lock().unwrap();
    rows.retain(|r|r.strong_count()>0);
    rows.iter().filter_map(|r|r.upgrade()).map(|r|r.snapshot()).collect()
}

//! Passive per-isolate accounting. Snapshot gauges cover queued and decoding
//! buffers, including capacity; no extra channel messages or JS probes.
//!
//! When `BOT_MEMORY_DIAGNOSTICS=1` is resolved once at isolate creation, a
//! bounded optional stop-reason string may be retained on this Arc for the
//! diagnostic `memory_progress` payload only. It is never part of [`Counters::snapshot`].
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering::Relaxed}};

/// Max Unicode scalars retained for an opt-in diagnostic stop reason.
pub const STOP_REASON_CAP: usize = 1024;

/// Opt-in diagnostic capture of `ScriptRunner.stop` / `host.stopReason`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StopReasonCapture {
    /// No stop observed yet, or diagnostics were off (nothing written).
    #[default]
    Absent,
    /// Diagnostics on; own data-property string missing, non-string, accessor, or eval failed.
    Unavailable,
    /// Own data-property string (may be empty); length ≤ [`STOP_REASON_CAP`].
    Value(String),
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
    /// Resolved once at isolate creation from `BOT_MEMORY_DIAGNOSTICS=1` (or test enable).
    diagnostics: AtomicBool,
    /// Bounded stop reason for diagnostic progress only; not retained on snapshot rows.
    stop_reason: Mutex<StopReasonCapture>,
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
    pub fn enable_diagnostics(&self) {
        self.diagnostics.store(true, Relaxed);
    }
    pub fn diagnostics_enabled(&self) -> bool {
        self.diagnostics.load(Relaxed)
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

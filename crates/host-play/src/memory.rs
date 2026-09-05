//! Opt-in frontend memory benchmark harness.
//!
//! No effect unless `BOT_MEMORY_N` is set. Counting allocator is for
//! frontend binaries to install later — no logging or allocation inside
//! allocator callbacks. Sample fields are separate domains; never sum them
//! and never equate allocation counts with RSS.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

/// Installed only by benchmark-enabled frontend binaries; no logging or
/// allocation inside allocator callbacks. Requested Rust bytes exclude
/// V8 / native / GPU allocators.
pub struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            LIVE_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            LIVE_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new = System.realloc(p, layout, size);
        if !new.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            ALLOC_BYTES.fetch_add(size as u64, Relaxed);
            LIVE_BYTES.fetch_add(size as u64, Relaxed);
            LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
        }
        new
    }
}

/// Snapshot of the counting allocator atomics.
///
/// Meaningful only after a frontend installs [`CountingAllocator`] as the
/// global allocator. Until then, prefer JSON null over these zeros.
pub fn rust_allocator_counts() -> (u64, u64, u64) {
    (
        ALLOCS.load(Relaxed),
        ALLOC_BYTES.load(Relaxed),
        LIVE_BYTES.load(Relaxed),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    Idle,
    Active,
    Lifecycle,
}

impl Workload {
    pub fn as_str(self) -> &'static str {
        match self {
            Workload::Idle => "idle",
            Workload::Active => "active",
            Workload::Lifecycle => "lifecycle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub n: usize,
    pub workload: Workload,
    pub warmup: Duration,
    pub observe: Duration,
}

impl Config {
    /// Parse opt-in env. `Ok(None)` when `BOT_MEMORY_N` is unset (normal play).
    pub fn from_env() -> Result<Option<Self>, String> {
        let Ok(n) = std::env::var("BOT_MEMORY_N") else {
            return Ok(None);
        };
        let n = parse_n(&n)?;
        let workload = match std::env::var("BOT_MEMORY_WORKLOAD").as_deref() {
            Err(_) | Ok("idle") => Workload::Idle,
            Ok("active") => Workload::Active,
            Ok("lifecycle") => Workload::Lifecycle,
            _ => return Err("BOT_MEMORY_WORKLOAD must be idle, active, or lifecycle".into()),
        };
        let duration = |key: &str, default: u64| -> Result<Duration, String> {
            let secs = match std::env::var(key) {
                Ok(s) => s.parse::<u64>().map_err(|_| format!("invalid {key}"))?,
                Err(_) => default,
            };
            if secs == 0 {
                return Err(format!("{key} must be positive"));
            }
            Ok(Duration::from_secs(secs))
        };
        Ok(Some(Self {
            n,
            workload,
            warmup: duration("BOT_MEMORY_WARMUP_S", 120)?,
            observe: duration("BOT_MEMORY_OBSERVE_S", 600)?,
        }))
    }
}

pub fn parse_n(n: &str) -> Result<usize, String> {
    match n {
        "1" => Ok(1),
        "32" => Ok(32),
        "128" => Ok(128),
        _ => Err("BOT_MEMORY_N must be 1, 32, or 128".into()),
    }
}

/// One jsonl sample line. Metric fields are siblings — never summed.
///
/// Missing components are JSON `null` (not zero pretending to be a measurement).
#[derive(Clone, Debug)]
pub struct Sample {
    pub frontend: String,
    pub n: usize,
    pub workload: Workload,
    pub elapsed_s: f64,
    pub phase: String,
    pub ready: usize,
    pub active: usize,
    pub resident_bytes: Option<u64>,
    pub peak_resident_bytes: Option<u64>,
    pub rust_allocations: Option<u64>,
    pub rust_allocated_bytes: Option<u64>,
    pub rust_live_bytes: Option<u64>,
    pub snapshot_inflight_bytes: Option<u64>,
    pub snapshot_inflight_capacity: Option<u64>,
    pub v8_used_bytes: Option<u64>,
    pub v8_total_bytes: Option<u64>,
    pub gpu_tracked_bytes: Option<u64>,
}

impl Sample {
    /// Serialize as a JSON object with sibling metric fields (null when missing).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "frontend": self.frontend,
            "n": self.n,
            "workload": self.workload.as_str(),
            "elapsed_s": self.elapsed_s,
            "phase": self.phase,
            "ready": self.ready,
            "active": self.active,
            "resident_bytes": self.resident_bytes,
            "peak_resident_bytes": self.peak_resident_bytes,
            "rust_allocations": self.rust_allocations,
            "rust_allocated_bytes": self.rust_allocated_bytes,
            "rust_live_bytes": self.rust_live_bytes,
            "snapshot_inflight_bytes": self.snapshot_inflight_bytes,
            "snapshot_inflight_capacity": self.snapshot_inflight_capacity,
            "v8_used_bytes": self.v8_used_bytes,
            "v8_total_bytes": self.v8_total_bytes,
            "gpu_tracked_bytes": self.gpu_tracked_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env mutation is process-global; serialize these tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<&'static str>,
    }

    impl EnvGuard {
        fn clear(keys: &[&'static str]) -> Self {
            for k in keys {
                std::env::remove_var(k);
            }
            Self {
                keys: keys.to_vec(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.keys {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    fn parse_n_accepts_1_32_128() {
        assert_eq!(parse_n("1").unwrap(), 1);
        assert_eq!(parse_n("32").unwrap(), 32);
        assert_eq!(parse_n("128").unwrap(), 128);
    }

    #[test]
    fn parse_n_rejects_invalid() {
        for s in ["0", "2", "50", "", "-1"] {
            assert!(parse_n(s).is_err(), "expected err for {s:?}");
        }
    }

    #[test]
    fn config_from_env_none_without_bot_memory_n() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        assert_eq!(Config::from_env().unwrap(), None);
    }

    #[test]
    fn config_from_env_rejects_invalid_n() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "2");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_rejects_invalid_workload() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WORKLOAD", "sprint");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_rejects_zero_durations() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "1");
        std::env::set_var("BOT_MEMORY_WARMUP_S", "0");
        assert!(Config::from_env().is_err());
        std::env::remove_var("BOT_MEMORY_WARMUP_S");
        std::env::set_var("BOT_MEMORY_OBSERVE_S", "0");
        assert!(Config::from_env().is_err());
    }

    #[test]
    fn config_from_env_defaults_idle_and_durations() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::clear(&[
            "BOT_MEMORY_N",
            "BOT_MEMORY_WORKLOAD",
            "BOT_MEMORY_WARMUP_S",
            "BOT_MEMORY_OBSERVE_S",
        ]);
        std::env::set_var("BOT_MEMORY_N", "32");
        let cfg = Config::from_env().unwrap().expect("Some");
        assert_eq!(cfg.n, 32);
        assert_eq!(cfg.workload, Workload::Idle);
        assert_eq!(cfg.warmup.as_secs(), 120);
        assert_eq!(cfg.observe.as_secs(), 600);
    }

    #[test]
    fn sample_json_has_sibling_metric_fields_not_one_sum() {
        let sample = Sample {
            frontend: "unit".into(),
            n: 1,
            workload: Workload::Idle,
            elapsed_s: 1.5,
            phase: "observe".into(),
            ready: 1,
            active: 0,
            resident_bytes: Some(100),
            peak_resident_bytes: Some(200),
            rust_allocations: Some(3),
            rust_allocated_bytes: Some(40),
            rust_live_bytes: Some(30),
            snapshot_inflight_bytes: Some(10),
            snapshot_inflight_capacity: Some(20),
            v8_used_bytes: Some(50),
            v8_total_bytes: Some(60),
            gpu_tracked_bytes: Some(70),
        };
        let v = sample.to_json();
        let obj = v.as_object().expect("object");
        for key in [
            "frontend",
            "n",
            "workload",
            "elapsed_s",
            "phase",
            "ready",
            "active",
            "resident_bytes",
            "peak_resident_bytes",
            "rust_allocations",
            "rust_allocated_bytes",
            "rust_live_bytes",
            "snapshot_inflight_bytes",
            "snapshot_inflight_capacity",
            "v8_used_bytes",
            "v8_total_bytes",
            "gpu_tracked_bytes",
        ] {
            assert!(obj.contains_key(key), "missing sibling field {key}");
        }
        // Not one summed byte count: the distinct metric keys stay separate.
        assert!(obj.get("total_bytes").is_none());
        assert!(obj.get("bytes").is_none());
        assert_ne!(
            obj.get("resident_bytes").and_then(|x| x.as_u64()),
            obj.get("peak_resident_bytes").and_then(|x| x.as_u64())
        );
        assert_ne!(
            obj.get("rust_live_bytes").and_then(|x| x.as_u64()),
            obj.get("resident_bytes").and_then(|x| x.as_u64())
        );
    }

    #[test]
    fn sample_json_nulls_missing_components_not_zero() {
        let sample = Sample {
            frontend: "unit".into(),
            n: 1,
            workload: Workload::Active,
            elapsed_s: 0.0,
            phase: "seed".into(),
            ready: 0,
            active: 0,
            resident_bytes: None,
            peak_resident_bytes: None,
            rust_allocations: None,
            rust_allocated_bytes: None,
            rust_live_bytes: None,
            snapshot_inflight_bytes: None,
            snapshot_inflight_capacity: None,
            v8_used_bytes: None,
            v8_total_bytes: None,
            gpu_tracked_bytes: None,
        };
        let v = sample.to_json();
        let obj = v.as_object().expect("object");
        for key in [
            "resident_bytes",
            "peak_resident_bytes",
            "rust_allocations",
            "rust_allocated_bytes",
            "rust_live_bytes",
            "snapshot_inflight_bytes",
            "snapshot_inflight_capacity",
            "v8_used_bytes",
            "v8_total_bytes",
            "gpu_tracked_bytes",
        ] {
            assert!(obj.get(key).unwrap().is_null(), "{key} should be null");
        }
    }
}

//! Opt-in per-slot host work timers for the performance harness.
//!
//! The slot loop already owns cumulative wall-work counters. This module only
//! publishes those counters once per second when the `performance-profile`
//! feature is enabled. Production builds do not compile the registry or its
//! atomic stores.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, LazyLock, Weak};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingSnapshot {
    pub username: String,
    pub loop_ns: u64,
    pub observe_ns: u64,
    pub raster_ns: u64,
}

pub(crate) struct SlotTiming {
    loop_ns: AtomicU64,
    observe_ns: AtomicU64,
    raster_ns: AtomicU64,
}

type SlotRegistry = Vec<(String, Weak<SlotTiming>)>;

static SLOTS: LazyLock<Mutex<SlotRegistry>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub(crate) fn register(username: &str) -> Arc<SlotTiming> {
    let timing = Arc::new(SlotTiming {
        loop_ns: AtomicU64::new(0),
        observe_ns: AtomicU64::new(0),
        raster_ns: AtomicU64::new(0),
    });
    let mut slots = SLOTS.lock();
    slots.retain(|(_, timing)| timing.strong_count() > 0);
    slots.push((username.to_owned(), Arc::downgrade(&timing)));
    timing
}

impl SlotTiming {
    pub(crate) fn publish(&self, loop_ns: u64, observe_ns: u64, raster_ns: u64) {
        self.loop_ns.store(loop_ns, Relaxed);
        self.observe_ns.store(observe_ns, Relaxed);
        self.raster_ns.store(raster_ns, Relaxed);
    }
}

/// Cumulative timer snapshots for currently running slots.
pub fn snapshots() -> Vec<TimingSnapshot> {
    let mut slots = SLOTS.lock();
    let mut snapshots = Vec::with_capacity(slots.len());
    slots.retain(|(username, timing)| {
        let Some(timing) = timing.upgrade() else {
            return false;
        };
        snapshots.push(TimingSnapshot {
            username: username.clone(),
            loop_ns: timing.loop_ns.load(Relaxed),
            observe_ns: timing.observe_ns.load(Relaxed),
            raster_ns: timing.raster_ns.load(Relaxed),
        });
        true
    });
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_slot_publishes_cumulative_components_and_drop_removes_it() {
        let username = format!("perf-profile-{}", std::process::id());
        let timing = register(&username);
        timing.publish(11, 22, 33);
        let row = snapshots()
            .into_iter()
            .find(|row| row.username == username)
            .expect("registered slot");
        assert_eq!((row.loop_ns, row.observe_ns, row.raster_ns), (11, 22, 33));
        drop(timing);
        assert!(snapshots().into_iter().all(|row| row.username != username));
    }
}

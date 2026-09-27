//! Opt-in per-slot host work timers for the performance harness.
//!
//! The slot loop already owns cumulative wall-work counters. This module only
//! publishes those counters once per second when the `performance-profile`
//! feature is enabled. Production builds do not compile the registry or its
//! atomic stores.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, LazyLock, Mutex, Weak};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingSnapshot {
    pub username: String,
    pub loop_ns: u64,
    pub observe_ns: u64,
    pub raster_ns: u64,
}

#[derive(Default)]
struct CumulativeTiming {
    loop_ns: AtomicU64,
    observe_ns: AtomicU64,
    raster_ns: AtomicU64,
}

pub(crate) struct SlotTiming {
    totals: Arc<CumulativeTiming>,
    base_loop_ns: u64,
    base_observe_ns: u64,
    base_raster_ns: u64,
}

struct SlotEntry {
    username: String,
    totals: Arc<CumulativeTiming>,
    active: Weak<SlotTiming>,
}

type SlotRegistry = Vec<SlotEntry>;

static SLOTS: LazyLock<Mutex<SlotRegistry>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub(crate) fn register(username: &str) -> Arc<SlotTiming> {
    let mut slots = SLOTS.lock().unwrap();
    let totals = slots
        .iter()
        .find(|entry| entry.username == username)
        .map(|entry| entry.totals.clone())
        .unwrap_or_default();
    let timing = Arc::new(SlotTiming {
        base_loop_ns: totals.loop_ns.load(Relaxed),
        base_observe_ns: totals.observe_ns.load(Relaxed),
        base_raster_ns: totals.raster_ns.load(Relaxed),
        totals: totals.clone(),
    });
    if let Some(entry) = slots.iter_mut().find(|entry| entry.username == username) {
        entry.active = Arc::downgrade(&timing);
    } else {
        slots.push(SlotEntry {
            username: username.to_owned(),
            totals,
            active: Arc::downgrade(&timing),
        });
    }
    timing
}

impl SlotTiming {
    pub(crate) fn publish(&self, loop_ns: u64, observe_ns: u64, raster_ns: u64) {
        self.totals
            .loop_ns
            .fetch_max(self.base_loop_ns.saturating_add(loop_ns), Relaxed);
        self.totals
            .observe_ns
            .fetch_max(self.base_observe_ns.saturating_add(observe_ns), Relaxed);
        self.totals
            .raster_ns
            .fetch_max(self.base_raster_ns.saturating_add(raster_ns), Relaxed);
    }
}

/// Cumulative timer snapshots for currently running slots.
///
/// Re-registering a username after a relog preserves its retired slot's
/// counters, while the returned row still represents one active slot.
pub fn snapshots() -> Vec<TimingSnapshot> {
    let slots = SLOTS.lock().unwrap();
    slots
        .iter()
        .filter(|entry| entry.active.upgrade().is_some())
        .map(|entry| TimingSnapshot {
            username: entry.username.clone(),
            loop_ns: entry.totals.loop_ns.load(Relaxed),
            observe_ns: entry.totals.observe_ns.load(Relaxed),
            raster_ns: entry.totals.raster_ns.load(Relaxed),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relog_preserves_retired_components_without_counting_an_inactive_slot() {
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

        let replacement = register(&username);
        replacement.publish(5, 7, 9);
        let row = snapshots()
            .into_iter()
            .find(|row| row.username == username)
            .expect("re-registered slot");
        assert_eq!((row.loop_ns, row.observe_ns, row.raster_ns), (16, 29, 42));
    }
}

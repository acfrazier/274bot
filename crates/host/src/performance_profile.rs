//! Opt-in per-slot host work and observe-cost counters for the performance harness.
//!
//! The slot loop publishes cumulative wall-work counters once per second when
//! `performance-profile` is enabled. `observe-cost-profile` additionally
//! samples thread CPU and allocation totals around each observe call, then
//! writes cumulative TSV rows only when `BOT_OBSERVE_COST_OUT` is set.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, LazyLock, Mutex, Weak};

#[cfg(feature = "observe-cost-profile")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "observe-cost-profile")]
use std::io::{BufWriter, Write};
#[cfg(feature = "observe-cost-profile")]
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingSnapshot {
    pub username: String,
    pub loop_ns: u64,
    pub observe_ns: u64,
    pub raster_ns: u64,
    pub observe_cpu_ns: u64,
    pub observe_cpu_samples: u64,
    pub observe_frames: u64,
    pub observe_allocations: u64,
    pub observe_allocated_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObserveCost {
    pub cpu_ns: u64,
    pub cpu_samples: u64,
    pub frames: u64,
    pub allocations: u64,
    pub allocated_bytes: u64,
}

#[derive(Default)]
struct CumulativeTiming {
    loop_ns: AtomicU64,
    observe_ns: AtomicU64,
    raster_ns: AtomicU64,
    observe_cpu_ns: AtomicU64,
    observe_cpu_samples: AtomicU64,
    observe_frames: AtomicU64,
    observe_allocations: AtomicU64,
    observe_allocated_bytes: AtomicU64,
}

pub(crate) struct SlotTiming {
    totals: Arc<CumulativeTiming>,
    base_loop_ns: u64,
    base_observe_ns: u64,
    base_raster_ns: u64,
    base_observe_cpu_ns: u64,
    base_observe_cpu_samples: u64,
    base_observe_frames: u64,
    base_observe_allocations: u64,
    base_observe_allocated_bytes: u64,
}

struct SlotEntry {
    username: String,
    totals: Arc<CumulativeTiming>,
    active: Weak<SlotTiming>,
}

type SlotRegistry = Vec<SlotEntry>;

static SLOTS: LazyLock<Mutex<SlotRegistry>> = LazyLock::new(|| Mutex::new(Vec::new()));

#[cfg(feature = "observe-cost-profile")]
static COST_OUTPUT: LazyLock<Option<Mutex<BufWriter<File>>>> = LazyLock::new(|| {
    std::env::var_os("BOT_OBSERVE_COST_OUT").map(|path| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("create observe cost output");
        let mut output = BufWriter::new(file);
        writeln!(
            output,
            "username\telapsed_ns\tloop_ns\tobserve_wall_ns\traster_ns\tobserve_cpu_ns\tobserve_cpu_samples\tobserve_frames\tobserve_allocations\tobserve_allocated_bytes\tcache_bypass"
        )
        .expect("write observe cost header");
        output.flush().expect("flush observe cost header");
        Mutex::new(output)
    })
});

#[cfg(feature = "observe-cost-profile")]
static COST_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

#[cfg(feature = "observe-cost-profile")]
pub(crate) fn observe_cost_profile_enabled() -> bool {
    COST_OUTPUT.is_some()
}

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
        base_observe_cpu_ns: totals.observe_cpu_ns.load(Relaxed),
        base_observe_cpu_samples: totals.observe_cpu_samples.load(Relaxed),
        base_observe_frames: totals.observe_frames.load(Relaxed),
        base_observe_allocations: totals.observe_allocations.load(Relaxed),
        base_observe_allocated_bytes: totals.observe_allocated_bytes.load(Relaxed),
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
    pub(crate) fn publish(
        &self,
        username: &str,
        loop_ns: u64,
        observe_ns: u64,
        raster_ns: u64,
        observe_cost: ObserveCost,
    ) {
        self.totals
            .loop_ns
            .fetch_max(self.base_loop_ns.saturating_add(loop_ns), Relaxed);
        self.totals
            .observe_ns
            .fetch_max(self.base_observe_ns.saturating_add(observe_ns), Relaxed);
        self.totals
            .raster_ns
            .fetch_max(self.base_raster_ns.saturating_add(raster_ns), Relaxed);
        self.totals.observe_cpu_ns.fetch_max(
            self.base_observe_cpu_ns.saturating_add(observe_cost.cpu_ns),
            Relaxed,
        );
        self.totals.observe_cpu_samples.fetch_max(
            self.base_observe_cpu_samples
                .saturating_add(observe_cost.cpu_samples),
            Relaxed,
        );
        self.totals.observe_frames.fetch_max(
            self.base_observe_frames.saturating_add(observe_cost.frames),
            Relaxed,
        );
        self.totals.observe_allocations.fetch_max(
            self.base_observe_allocations
                .saturating_add(observe_cost.allocations),
            Relaxed,
        );
        self.totals.observe_allocated_bytes.fetch_max(
            self.base_observe_allocated_bytes
                .saturating_add(observe_cost.allocated_bytes),
            Relaxed,
        );
        #[cfg(not(feature = "observe-cost-profile"))]
        let _ = username;
        #[cfg(feature = "observe-cost-profile")]
        record_observe_cost(username, &self.totals);
    }
}

#[cfg(feature = "observe-cost-profile")]
fn record_observe_cost(username: &str, totals: &CumulativeTiming) {
    let Some(output) = COST_OUTPUT.as_ref() else {
        return;
    };
    let mut output = output.lock().unwrap();
    writeln!(
        output,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        username,
        u64::try_from(COST_EPOCH.elapsed().as_nanos()).unwrap_or(u64::MAX),
        totals.loop_ns.load(Relaxed),
        totals.observe_ns.load(Relaxed),
        totals.raster_ns.load(Relaxed),
        totals.observe_cpu_ns.load(Relaxed),
        totals.observe_cpu_samples.load(Relaxed),
        totals.observe_frames.load(Relaxed),
        totals.observe_allocations.load(Relaxed),
        totals.observe_allocated_bytes.load(Relaxed),
        std::env::var("BOT_WORLD_STATE_CACHE_BYPASS").is_ok_and(|value| value == "1"),
    )
    .expect("write observe cost row");
    output.flush().expect("flush observe cost row");
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
            observe_cpu_ns: entry.totals.observe_cpu_ns.load(Relaxed),
            observe_cpu_samples: entry.totals.observe_cpu_samples.load(Relaxed),
            observe_frames: entry.totals.observe_frames.load(Relaxed),
            observe_allocations: entry.totals.observe_allocations.load(Relaxed),
            observe_allocated_bytes: entry.totals.observe_allocated_bytes.load(Relaxed),
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
        timing.publish(
            &username,
            11,
            22,
            33,
            ObserveCost {
                cpu_ns: 13,
                cpu_samples: 1,
                frames: 2,
                allocations: 17,
                allocated_bytes: 19,
            },
        );
        let row = snapshots()
            .into_iter()
            .find(|row| row.username == username)
            .expect("registered slot");
        assert_eq!((row.loop_ns, row.observe_ns, row.raster_ns), (11, 22, 33));
        assert_eq!(
            (
                row.observe_cpu_ns,
                row.observe_cpu_samples,
                row.observe_frames,
                row.observe_allocations,
                row.observe_allocated_bytes
            ),
            (13, 1, 2, 17, 19)
        );
        drop(timing);
        assert!(snapshots().into_iter().all(|row| row.username != username));

        let replacement = register(&username);
        replacement.publish(
            &username,
            5,
            7,
            9,
            ObserveCost {
                cpu_ns: 3,
                cpu_samples: 2,
                frames: 4,
                allocations: 5,
                allocated_bytes: 7,
            },
        );
        let row = snapshots()
            .into_iter()
            .find(|row| row.username == username)
            .expect("re-registered slot");
        assert_eq!((row.loop_ns, row.observe_ns, row.raster_ns), (16, 29, 42));
        assert_eq!(
            (
                row.observe_cpu_ns,
                row.observe_cpu_samples,
                row.observe_frames,
                row.observe_allocations,
                row.observe_allocated_bytes
            ),
            (16, 3, 6, 22, 26)
        );
    }
}

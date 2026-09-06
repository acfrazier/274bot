//! Opt-in active-loop diagnostics.
//!
//! # Legacy process-wide groups
//!
//! Thread-local batches avoid a shared lock on each tick. Counters are
//! cumulative and may lag by up to 49 cycles/slot before the batch merges into
//! the process-wide drawing / non-drawing groups. Those group meanings and
//! excess-bucket schema are unchanged.
//!
//! # Per-slot start-to-start intervals (additional)
//!
//! When enabled, each slot also registers a bounded generation row. Local
//! counters accumulate on the slot thread; the registry is updated about every
//! 1 s, on park / drawing-mode dirty, every 50 cycles (with the legacy flush),
//! and on drop. Interval histograms measure **absolute** tick start-to-start
//! duration (not excess over the 20 ms budget). Samples pair only consecutive
//! same-drawing starts with no intervening park. Drawing-mode flips and parks
//! break the anchor and are counted explicitly — they are never interval
//! samples. Scene/focus transitions that change `client.draw` therefore appear
//! as mode breaks, not mixed-cadence intervals.
//!
//! Histogram bounds use ≤1 ms resolution from 18–42 ms so a conservative p99
//! upper bound can sit next to the 40 ms gate with ~2 ms paired headroom.
//! Overflow is an explicit final bucket (no finite upper bound).
//!
//! Disabled path: atomics false; `Local::new` returns `None`; no registry work.
//! Measurement never changes sleep budget, sleep calls, or loop structure.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static TOTAL: Mutex<[Counts; 2]> = Mutex::new([Counts::ZERO; 2]);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());
/// Ended generation rows discarded because the unread-ended cap was hit.
static ENDED_LOST: AtomicU64 = AtomicU64::new(0);

/// Cap ended-but-unread rows so slot restart storms cannot grow the registry
/// without a reader. Live rows are bounded by concurrent slots.
const MAX_ENDED_UNREAD: usize = 64;

/// Inclusive upper bounds in ms for absolute start-to-start intervals, then
/// overflow. 1 ms steps from 18..=42 support the 20 ms target and 40 ms p99
/// gate with ~2 ms paired-comparison headroom; tails are coarser.
pub const INTERVAL_BOUNDS_MS: [u64; 35] = [
    5, 10, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
    37, 38, 39, 40, 42, 45, 50, 100, 250, 500, 1000,
];
const INTERVAL_BUCKETS: usize = INTERVAL_BOUNDS_MS.len() + 1;

/// Legacy excess histogram upper bounds: 1, 2, 5, 10, 20 ms, then unbounded.
const EXCESS_BOUNDS_MS: [u64; 5] = [1, 2, 5, 10, 20];

#[derive(Clone, Copy, Debug)]
pub struct Counts {
    pub cycles: u64,
    pub work_ns: u64,
    pub requested_sleep_ns: u64,
    pub actual_sleep_ns: u64,
    pub work_overruns: u64,
    pub intervals: u64,
    pub interval_ns: u64,
    pub sleep_excess: [u64; 6],
    pub interval_excess: [u64; 6],
}

impl Counts {
    const ZERO: Self = Self {
        cycles: 0,
        work_ns: 0,
        requested_sleep_ns: 0,
        actual_sleep_ns: 0,
        work_overruns: 0,
        intervals: 0,
        interval_ns: 0,
        sleep_excess: [0; 6],
        interval_excess: [0; 6],
    };

    fn merge(&mut self, other: Self) {
        self.cycles += other.cycles;
        self.work_ns += other.work_ns;
        self.requested_sleep_ns += other.requested_sleep_ns;
        self.actual_sleep_ns += other.actual_sleep_ns;
        self.work_overruns += other.work_overruns;
        self.intervals += other.intervals;
        self.interval_ns += other.interval_ns;
        for i in 0..6 {
            self.sleep_excess[i] += other.sleep_excess[i];
            self.interval_excess[i] += other.interval_excess[i];
        }
    }
}

/// Published per-slot snapshot (host-play serializes; host has no serde).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotObservation {
    pub slot_id: u64,
    pub generation: u64,
    /// Wall ms of last registry publish (sample age = now − this).
    pub updated_ms: u64,
    pub ended: bool,
    /// Last recorded drawing latch (`client.draw`).
    pub drawing: bool,

    /// Actual `record` iterations this generation.
    pub cycle_n: u64,
    pub drawing_cycle_n: u64,
    pub non_drawing_cycle_n: u64,

    pub work_ns: u64,
    pub requested_sleep_ns: u64,
    pub actual_sleep_ns: u64,
    pub work_overrun_n: u64,

    /// Start-to-start samples (same drawing, no park between).
    pub interval_n: u64,
    pub interval_ns: u64,
    pub interval_buckets: [u64; INTERVAL_BUCKETS],
    /// Drawing-mode flip while an anchor existed (excluded, not an interval).
    pub mode_break_n: u64,
    /// `parked()` calls that cleared the interval anchor.
    pub park_n: u64,
    /// Cycles that ran with no prior anchor (first after start/park/mode break).
    pub anchor_miss_n: u64,

    /// Wall ms of first / last **interval sample** (0 if none).
    pub first_interval_ms: u64,
    pub last_interval_ms: u64,
    /// Wall ms of first / last cycle `record` (0 if none).
    pub first_cycle_ms: u64,
    pub last_cycle_ms: u64,

    /// Legacy-style excess histograms on this slot (same 6-bucket schema).
    pub sleep_excess: [u64; 6],
    pub interval_excess: [u64; 6],
}

impl SlotObservation {
    const fn zero(slot_id: u64, generation: u64) -> Self {
        Self {
            slot_id,
            generation,
            updated_ms: 0,
            ended: false,
            drawing: false,
            cycle_n: 0,
            drawing_cycle_n: 0,
            non_drawing_cycle_n: 0,
            work_ns: 0,
            requested_sleep_ns: 0,
            actual_sleep_ns: 0,
            work_overrun_n: 0,
            interval_n: 0,
            interval_ns: 0,
            interval_buckets: [0; INTERVAL_BUCKETS],
            mode_break_n: 0,
            park_n: 0,
            anchor_miss_n: 0,
            first_interval_ms: 0,
            last_interval_ms: 0,
            first_cycle_ms: 0,
            last_cycle_ms: 0,
            sleep_excess: [0; 6],
            interval_excess: [0; 6],
        }
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn excess_bucket(d: Duration) -> usize {
    EXCESS_BOUNDS_MS
        .iter()
        .position(|&ms| d <= Duration::from_millis(ms))
        .unwrap_or(EXCESS_BOUNDS_MS.len())
}

/// Compare full `Duration` values — do not floor to integer milliseconds.
fn interval_bucket(d: Duration) -> usize {
    INTERVAL_BOUNDS_MS
        .iter()
        .position(|&bound| d <= Duration::from_millis(bound))
        .unwrap_or(INTERVAL_BOUNDS_MS.len())
}

fn ns(d: Duration) -> u64 {
    d.as_nanos().min(u64::MAX as u128) as u64
}

fn prune_ended_overflow(reg: &mut Vec<SlotObservation>) {
    let ended = reg.iter().filter(|s| s.ended).count();
    if ended <= MAX_ENDED_UNREAD {
        return;
    }
    let mut drop_n = ended - MAX_ENDED_UNREAD;
    reg.retain(|s| {
        if s.ended && drop_n > 0 {
            drop_n -= 1;
            ENDED_LOST.fetch_add(1, Relaxed);
            false
        } else {
            true
        }
    });
}

pub fn enable() {
    ENABLED.store(true, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

/// FNV-1a 64 over username bytes — stable slot id without retaining the string.
pub fn slot_id_for(username: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in username.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Legacy process-wide drawing / non-drawing groups. `None` while disabled.
/// May lag local batches by up to 49 cycles per live slot.
pub fn read() -> Option<[Counts; 2]> {
    ENABLED.load(Relaxed).then(|| *TOTAL.lock().unwrap())
}

/// Per-slot observations when enabled; `None` while disabled.
/// Ended slots are included once, then pruned. Unread ended rows are also
/// hard-capped (`MAX_ENDED_UNREAD`) with `ended_lost_n()` accounting.
pub fn read_slots() -> Option<Vec<SlotObservation>> {
    if !ENABLED.load(Relaxed) {
        return None;
    }
    let mut reg = REGISTRY.lock().unwrap();
    let out = reg.clone();
    reg.retain(|s| !s.ended);
    Some(out)
}

/// Cumulative ended generation rows discarded under the unread-ended cap.
pub fn ended_lost_n() -> u64 {
    ENDED_LOST.load(Relaxed)
}

/// Conservative p99 from fixed absolute-interval buckets: returns the **upper
/// bound** of the bucket that first reaches cumulative count ≥ ceil(0.99 * n).
/// Overflow bucket yields `None` (unbounded / unavailable precise p99).
/// Empty samples yield `None`.
pub fn p99_upper_bound_ms(buckets: &[u64; INTERVAL_BUCKETS]) -> Option<u64> {
    let n: u64 = buckets.iter().sum();
    if n == 0 {
        return None;
    }
    let mut cum = 0u64;
    for (i, &c) in buckets.iter().enumerate() {
        cum = cum.saturating_add(c);
        if cum.saturating_mul(100) >= n.saturating_mul(99) {
            if i < INTERVAL_BOUNDS_MS.len() {
                return Some(INTERVAL_BOUNDS_MS[i]);
            }
            return None;
        }
    }
    None
}

/// True when every cycle is accounted as an interval, a mode break, or an
/// anchor miss (first after start/park/mode break). Does **not** imply the
/// 40 ms budget.
pub fn interval_coverage_complete(s: &SlotObservation) -> bool {
    s.cycle_n
        == s.interval_n
            .saturating_add(s.mode_break_n)
            .saturating_add(s.anchor_miss_n)
}

/// Per-slot local counters + legacy batch. Created only while enabled.
pub(crate) struct Local {
    generation: u64,
    snap: SlotObservation,
    /// Legacy process-wide pending (drawing index 0/1).
    pending: [Counts; 2],
    previous: Option<(Instant, bool)>,
    cycles: u32,
    last_flush: Instant,
    dirty: bool,
}

impl Local {
    pub fn new(slot_id: u64) -> Option<Self> {
        if !ENABLED.load(Relaxed) {
            return None;
        }
        let generation = NEXT_GENERATION.fetch_add(1, Relaxed);
        let snap = SlotObservation::zero(slot_id, generation);
        {
            let mut reg = REGISTRY.lock().unwrap();
            prune_ended_overflow(&mut reg);
            reg.push(snap.clone());
        }
        Some(Self {
            generation,
            snap,
            pending: [Counts::ZERO; 2],
            previous: None,
            cycles: 0,
            last_flush: Instant::now(),
            dirty: true,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn slot_id(&self) -> u64 {
        self.snap.slot_id
    }

    /// Clear the start-to-start anchor (idle park / focus sleep path).
    pub fn parked(&mut self) {
        if self.previous.is_some() {
            self.snap.park_n = self.snap.park_n.wrapping_add(1);
            self.previous = None;
            self.dirty = true;
            self.maybe_flush_slot();
        } else {
            self.previous = None;
        }
    }

    pub fn record(
        &mut self,
        start: Instant,
        drawing: bool,
        work: Duration,
        requested: Duration,
        slept: Duration,
        budget: Duration,
    ) {
        let now_ms = wall_ms();

        // --- legacy group batch (unchanged semantics) ---
        let c = &mut self.pending[drawing as usize];
        c.cycles += 1;
        c.work_ns += ns(work);
        c.requested_sleep_ns += ns(requested);
        c.actual_sleep_ns += ns(slept);
        c.work_overruns += u64::from(work > budget);
        if !requested.is_zero() {
            c.sleep_excess[excess_bucket(slept.saturating_sub(requested))] += 1;
        }
        if let Some((previous, previous_drawing)) = self.previous {
            if previous_drawing == drawing {
                let interval = start.duration_since(previous);
                c.intervals += 1;
                c.interval_ns += ns(interval);
                c.interval_excess[excess_bucket(interval.saturating_sub(budget))] += 1;
            }
        }

        // --- per-slot absolute start-to-start ---
        self.snap.drawing = drawing;
        self.snap.cycle_n = self.snap.cycle_n.wrapping_add(1);
        if drawing {
            self.snap.drawing_cycle_n = self.snap.drawing_cycle_n.wrapping_add(1);
        } else {
            self.snap.non_drawing_cycle_n = self.snap.non_drawing_cycle_n.wrapping_add(1);
        }
        self.snap.work_ns = self.snap.work_ns.wrapping_add(ns(work));
        self.snap.requested_sleep_ns = self.snap.requested_sleep_ns.wrapping_add(ns(requested));
        self.snap.actual_sleep_ns = self.snap.actual_sleep_ns.wrapping_add(ns(slept));
        if work > budget {
            self.snap.work_overrun_n = self.snap.work_overrun_n.wrapping_add(1);
        }
        if !requested.is_zero() {
            let b = excess_bucket(slept.saturating_sub(requested));
            self.snap.sleep_excess[b] = self.snap.sleep_excess[b].wrapping_add(1);
        }
        if self.snap.first_cycle_ms == 0 {
            self.snap.first_cycle_ms = now_ms;
        }
        self.snap.last_cycle_ms = now_ms;

        match self.previous {
            Some((previous, previous_drawing)) if previous_drawing == drawing => {
                let interval = start.saturating_duration_since(previous);
                let b = interval_bucket(interval);
                self.snap.interval_n = self.snap.interval_n.wrapping_add(1);
                self.snap.interval_ns = self.snap.interval_ns.wrapping_add(ns(interval));
                self.snap.interval_buckets[b] = self.snap.interval_buckets[b].wrapping_add(1);
                let eb = excess_bucket(interval.saturating_sub(budget));
                self.snap.interval_excess[eb] = self.snap.interval_excess[eb].wrapping_add(1);
                if self.snap.first_interval_ms == 0 {
                    self.snap.first_interval_ms = now_ms;
                }
                self.snap.last_interval_ms = now_ms;
            }
            Some((_, previous_drawing)) if previous_drawing != drawing => {
                self.snap.mode_break_n = self.snap.mode_break_n.wrapping_add(1);
                self.dirty = true;
            }
            _ => {
                // No anchor: first cycle, post-park, or post-mode-break start.
                self.snap.anchor_miss_n = self.snap.anchor_miss_n.wrapping_add(1);
            }
        }

        self.previous = Some((start, drawing));
        self.cycles += 1;
        if self.cycles >= 50 {
            self.flush_legacy();
            self.flush_slot(false);
        } else if self.dirty || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.maybe_flush_slot();
        }
    }

    fn maybe_flush_slot(&mut self) {
        if self.dirty || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.flush_slot(false);
        }
    }

    fn flush_legacy(&mut self) {
        let mut total = TOTAL.lock().unwrap();
        for i in 0..2 {
            total[i].merge(self.pending[i]);
        }
        self.pending = [Counts::ZERO; 2];
        self.cycles = 0;
    }

    fn flush_slot(&mut self, ended: bool) {
        self.snap.ended = ended;
        self.snap.updated_ms = wall_ms();
        let mut reg = REGISTRY.lock().unwrap();
        if let Some(entry) = reg.iter_mut().find(|s| s.generation == self.generation) {
            *entry = self.snap.clone();
        } else if !ended {
            reg.push(self.snap.clone());
        }
        if ended {
            prune_ended_overflow(&mut reg);
        }
        self.last_flush = Instant::now();
        self.dirty = false;
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        self.flush_legacy();
        self.flush_slot(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Serialize tests that mutate process-wide ENABLED/REGISTRY under parallel cargo test.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_lock<R>(f: impl FnOnce() -> R) -> R {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn reset() {
        ENABLED.store(false, Relaxed);
        *TOTAL.lock().unwrap() = [Counts::ZERO; 2];
        REGISTRY.lock().unwrap().clear();
        ENDED_LOST.store(0, Relaxed);
        NEXT_GENERATION.store(1, Relaxed);
    }

    fn enable_clean() {
        reset();
        ENABLED.store(true, Relaxed);
    }

    #[test]
    fn excess_buckets_include_boundaries() {
        assert_eq!(excess_bucket(Duration::ZERO), 0);
        assert_eq!(excess_bucket(Duration::from_millis(5)), 2);
        assert_eq!(excess_bucket(Duration::from_micros(5001)), 3);
        assert_eq!(excess_bucket(Duration::from_millis(21)), 5);
    }

    #[test]
    fn interval_buckets_1ms_around_20_40_without_ms_flooring() {
        assert_eq!(interval_bucket(Duration::ZERO), 0);
        assert_eq!(interval_bucket(Duration::from_millis(5)), 0);
        assert_eq!(interval_bucket(Duration::from_millis(20)), 7); // bound index of 20
        assert_eq!(interval_bucket(Duration::from_millis(40)), 27);
        // Sub-ms above the bound must not floor into the ≤40ms bucket.
        assert_eq!(
            interval_bucket(Duration::from_millis(40) + Duration::from_nanos(1)),
            28
        );
        assert_eq!(interval_bucket(Duration::from_millis(19)), 6);
        assert_eq!(interval_bucket(Duration::from_millis(18)), 5);
        // Sub-20 ms still lands in a defined bucket (not overflow): 12 → ≤15.
        assert_eq!(interval_bucket(Duration::from_millis(12)), 2);
        assert!(INTERVAL_BOUNDS_MS.contains(&20));
        assert!(INTERVAL_BOUNDS_MS.contains(&40));
        assert!(INTERVAL_BOUNDS_MS.contains(&42));
        // Overflow
        assert_eq!(
            interval_bucket(Duration::from_millis(1001)),
            INTERVAL_BOUNDS_MS.len()
        );
    }

    #[test]
    fn legacy_records_oversleep_and_excludes_park_and_focus_transitions() {
        with_lock(|| {
            enable_clean();
            let mut l = Local::new(1).expect("enabled");
            let t = Instant::now();
            let ms = Duration::from_millis;
            l.record(t, false, ms(3), ms(17), ms(22), ms(20));
            l.record(t + ms(25), false, ms(23), ms(0), ms(0), ms(20));
            assert_eq!(l.pending[0].work_overruns, 1);
            assert_eq!(l.pending[0].interval_ns, 25_000_000);
            assert_eq!(l.pending[0].sleep_excess[2], 1);
            assert_eq!(l.pending[0].interval_excess[2], 1);
            l.parked();
            l.record(t + ms(1000), false, ms(1), ms(19), ms(19), ms(20));
            l.record(t + ms(1020), true, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.pending[0].intervals, 1);
            assert_eq!(l.pending[1].intervals, 0);
            // Keep process counters independent of synthetic samples.
            l.pending = [Counts::ZERO; 2];
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn disabled_path_is_none() {
        with_lock(|| {
            reset();
            assert!(read().is_none());
            assert!(read_slots().is_none());
            assert!(Local::new(1).is_none());
            assert_eq!(ended_lost_n(), 0);
        });
    }

    #[test]
    fn per_slot_start_to_start_and_coverage() {
        with_lock(|| {
            enable_clean();
            let mut l = Local::new(42).expect("enabled");
            let t = Instant::now();
            let ms = Duration::from_millis;
            // first: anchor miss
            l.record(t, false, ms(1), ms(19), ms(19), ms(20));
            // 20 ms steady
            l.record(t + ms(20), false, ms(1), ms(19), ms(19), ms(20));
            // 25 ms
            l.record(t + ms(45), false, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.snap.cycle_n, 3);
            assert_eq!(l.snap.interval_n, 2);
            assert_eq!(l.snap.anchor_miss_n, 1);
            assert_eq!(l.snap.mode_break_n, 0);
            assert!(interval_coverage_complete(&l.snap));
            assert_eq!(l.snap.interval_buckets[interval_bucket(ms(20))], 1);
            assert_eq!(l.snap.interval_buckets[interval_bucket(ms(25))], 1);
            l.flush_slot(false);
            let slots = read_slots().expect("on");
            assert_eq!(slots.len(), 1);
            assert_eq!(slots[0].slot_id, 42);
            assert!(!slots[0].ended);
            assert!(slots[0].updated_ms > 0);
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn park_and_drawing_mode_exclude_intervals() {
        with_lock(|| {
            enable_clean();
            let mut l = Local::new(7).expect("enabled");
            let t = Instant::now();
            let ms = Duration::from_millis;
            l.record(t, false, ms(1), ms(19), ms(19), ms(20));
            l.record(t + ms(20), false, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.snap.interval_n, 1);
            l.parked();
            assert_eq!(l.snap.park_n, 1);
            // post-park first is anchor miss, not an interval across the park gap
            l.record(t + ms(5000), false, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.snap.interval_n, 1);
            assert_eq!(l.snap.anchor_miss_n, 2);
            // drawing flip excludes
            l.record(t + ms(5020), true, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.snap.mode_break_n, 1);
            assert_eq!(l.snap.interval_n, 1);
            // same drawing continues
            l.record(t + ms(5040), true, ms(1), ms(19), ms(19), ms(20));
            assert_eq!(l.snap.interval_n, 2);
            assert!(interval_coverage_complete(&l.snap));
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn restart_gets_new_generation_same_slot_id() {
        with_lock(|| {
            enable_clean();
            let id = slot_id_for("bot-a");
            let g1 = {
                let mut a = Local::new(id).expect("a");
                let t = Instant::now();
                a.record(
                    t,
                    false,
                    Duration::from_millis(1),
                    Duration::ZERO,
                    Duration::ZERO,
                    Duration::from_millis(20),
                );
                a.generation()
            }; // drop ends gen 1
            let b = Local::new(id).expect("b");
            assert_eq!(b.slot_id(), id);
            let g2 = b.generation();
            assert_ne!(g1, g2);
            let slots = read_slots().expect("on");
            // ended gen1 included once; live gen2 present
            assert!(slots.iter().any(|s| s.generation == g1 && s.ended));
            assert!(slots.iter().any(|s| s.generation == g2 && !s.ended));
            // second read prunes ended
            let slots2 = read_slots().expect("on");
            assert!(!slots2.iter().any(|s| s.ended));
            assert!(slots2.iter().any(|s| s.generation == g2 && !s.ended));
            drop(b);
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn registry_bounds_ended_with_lost_accounting() {
        with_lock(|| {
            enable_clean();
            for i in 0..(MAX_ENDED_UNREAD + 20) {
                let local = Local::new(i as u64).expect("on");
                drop(local);
            }
            let reg = REGISTRY.lock().unwrap();
            let ended = reg.iter().filter(|s| s.ended).count();
            assert!(ended <= MAX_ENDED_UNREAD, "ended={ended}");
            drop(reg);
            assert!(ended_lost_n() >= 20);
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn p99_upper_bound_from_buckets() {
        let mut b = [0u64; INTERVAL_BUCKETS];
        // 98 at ≤20ms, 2 overflow → need cum≥99 so p99 is overflow → None
        b[interval_bucket(Duration::from_millis(20))] = 98;
        b[INTERVAL_BOUNDS_MS.len()] = 2;
        assert_eq!(p99_upper_bound_ms(&b), None);
        // 100 at ≤40ms
        let mut b2 = [0u64; INTERVAL_BUCKETS];
        b2[interval_bucket(Duration::from_millis(40))] = 100;
        assert_eq!(p99_upper_bound_ms(&b2), Some(40));
        // empty
        assert_eq!(p99_upper_bound_ms(&[0u64; INTERVAL_BUCKETS]), None);
        // 99 at 20, 1 at 40 → cum hits ≥99 at 20 first
        let mut b3 = [0u64; INTERVAL_BUCKETS];
        b3[interval_bucket(Duration::from_millis(20))] = 99;
        b3[interval_bucket(Duration::from_millis(40))] = 1;
        assert_eq!(p99_upper_bound_ms(&b3), Some(20));
    }

    #[test]
    fn legacy_group_schema_still_merges() {
        with_lock(|| {
            enable_clean();
            let mut l = Local::new(1).expect("on");
            let t = Instant::now();
            let ms = Duration::from_millis;
            for i in 0..50 {
                l.record(t + ms(i * 20), false, ms(1), ms(19), ms(19), ms(20));
            }
            // flush_legacy ran at 50
            let groups = read().expect("on");
            assert!(groups[0].cycles >= 50);
            assert_eq!(l.pending[0].cycles, 0);
            ENABLED.store(false, Relaxed);
        });
    }

    #[test]
    fn slot_id_stable() {
        assert_eq!(slot_id_for("alice"), slot_id_for("alice"));
        assert_ne!(slot_id_for("alice"), slot_id_for("bob"));
    }
}

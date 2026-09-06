//! Opt-in active-loop diagnostics. Thread-local batches avoid a shared lock
//! on each tick. Counters are cumulative and may lag by up to 49 cycles/slot.
use std::sync::{
    atomic::{AtomicBool, Ordering::Relaxed},
    Mutex,
};
use std::time::{Duration, Instant};
static ENABLED: AtomicBool = AtomicBool::new(false);
static TOTAL: Mutex<[Counts; 2]> = Mutex::new([Counts::ZERO; 2]);
pub fn enable() {
    ENABLED.store(true, Relaxed);
}
pub fn read() -> Option<[Counts; 2]> {
    ENABLED.load(Relaxed).then(|| *TOTAL.lock().unwrap())
}
/// Histogram upper bounds: 1, 2, 5, 10, 20 ms, then unbounded.
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
fn bucket(d: Duration) -> usize {
    [1, 2, 5, 10, 20]
        .iter()
        .position(|&ms| d <= Duration::from_millis(ms))
        .unwrap_or(5)
}
fn ns(d: Duration) -> u64 {
    d.as_nanos().min(u64::MAX as u128) as u64
}
pub(crate) struct Local {
    pending: [Counts; 2],
    previous: Option<(Instant, bool)>,
    cycles: u32,
}
impl Local {
    pub fn new() -> Option<Self> {
        ENABLED.load(Relaxed).then(|| Self {
            pending: [Counts::ZERO; 2],
            previous: None,
            cycles: 0,
        })
    }
    pub fn parked(&mut self) {
        self.previous = None;
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
        let c = &mut self.pending[drawing as usize];
        c.cycles += 1;
        c.work_ns += ns(work);
        c.requested_sleep_ns += ns(requested);
        c.actual_sleep_ns += ns(slept);
        c.work_overruns += u64::from(work > budget);
        if !requested.is_zero() {
            c.sleep_excess[bucket(slept.saturating_sub(requested))] += 1;
        }
        if let Some((previous, previous_drawing)) = self.previous {
            if previous_drawing == drawing {
                let interval = start.duration_since(previous);
                c.intervals += 1;
                c.interval_ns += ns(interval);
                c.interval_excess[bucket(interval.saturating_sub(budget))] += 1;
            }
        }
        self.previous = Some((start, drawing));
        self.cycles += 1;
        if self.cycles >= 50 {
            self.flush();
        }
    }
    fn flush(&mut self) {
        let mut total = TOTAL.lock().unwrap();
        for i in 0..2 {
            total[i].merge(self.pending[i]);
        }
        self.pending = [Counts::ZERO; 2];
        self.cycles = 0;
    }
}
impl Drop for Local {
    fn drop(&mut self) {
        self.flush();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_include_boundaries() {
        assert_eq!(bucket(Duration::ZERO), 0);
        assert_eq!(bucket(Duration::from_millis(5)), 2);
        assert_eq!(bucket(Duration::from_micros(5001)), 3);
        assert_eq!(bucket(Duration::from_millis(21)), 5);
    }
    #[test]
    fn records_oversleep_and_excludes_park_and_focus_transitions() {
        let mut l = Local {
            pending: [Counts::ZERO; 2],
            previous: None,
            cycles: 0,
        };
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
        l.pending = [Counts::ZERO; 2]; // keep the process counters independent of synthetic samples
    }
}

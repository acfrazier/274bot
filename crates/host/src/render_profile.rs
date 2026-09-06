//! Opt-in per-slot renderer residency and paint-cadence observations.
//!
//! Default off. Enable before clients start. Local counters accumulate on the
//! slot thread; publication is bounded (~1s or state change). Counts are host
//! `mainredraw`/paint completions, not GPU completed or presented frames.
//! Histogram bucket edges support a conservative upper bound around the 40 ms
//! gate and ~1 s background cadence; they are not labeled percentiles.
//!
//! Interval linkage breaks on draw / full-rate / backend / stable-scene mode
//! changes even when the transition frame does not paint, so mixed 1 fps and
//! full-rate gaps are never treated as one steady cadence sample.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());

/// Cap ended-but-unread rows so slot restart storms cannot grow the registry
/// without a reader. Live (non-ended) rows are bounded by concurrent slots.
const MAX_ENDED_UNREAD: usize = 64;

/// Histogram upper bounds in milliseconds (inclusive on exact Duration), then
/// unbounded. Covers full-rate (~20 ms), the 40 ms gate, and ~1 s watch paint.
pub const INTERVAL_BOUNDS_MS: [u64; 10] = [10, 20, 25, 40, 50, 100, 250, 500, 1000, 2000];
const INTERVAL_BUCKETS: usize = INTERVAL_BOUNDS_MS.len() + 1;

/// Observed backend residency after attach. `CpuFallback` is a GPU-preferred
/// head that landed on the CPU path; distinct from an explicit CPU request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendObs {
    Absent,
    Cpu,
    CpuFallback,
    Gpu,
}

impl BackendObs {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Absent => None,
            Self::Cpu => Some("cpu"),
            Self::CpuFallback => Some("cpu_fallback"),
            Self::Gpu => Some("gpu"),
        }
    }
}

/// Published per-slot snapshot (host-play serializes; host has no serde).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotObservation {
    pub slot_id: u64,
    pub generation: u64,
    pub renderer_present: bool,
    pub backend: BackendObs,
    /// Latched prefer-cpu request while a head is attached; `None` if absent.
    pub prefer_cpu: Option<bool>,
    pub ingame: bool,
    pub scene_state: i32,
    pub draw: bool,
    pub full_rate: bool,
    pub client_loop_n: u64,
    /// Host completed `mainredraw` calls (not GPU present completions).
    pub paint_n: u64,
    pub skip_n: u64,
    pub stable_paint_n: u64,
    pub transition_paint_n: u64,
    pub stable_paint_intervals: u64,
    pub stable_paint_interval_ns: u64,
    pub stable_paint_interval_buckets: [u64; INTERVAL_BUCKETS],
    pub transition_paint_intervals: u64,
    pub transition_paint_interval_ns: u64,
    pub transition_paint_interval_buckets: [u64; INTERVAL_BUCKETS],
    pub attach_n: u64,
    pub detach_n: u64,
    pub backend_change_n: u64,
    /// Wall ms of last publish (sample age = now - this).
    pub updated_ms: u64,
    pub ended: bool,
}

impl SlotObservation {
    const fn zero(slot_id: u64, generation: u64) -> Self {
        Self {
            slot_id,
            generation,
            renderer_present: false,
            backend: BackendObs::Absent,
            prefer_cpu: None,
            ingame: false,
            scene_state: 0,
            draw: false,
            full_rate: false,
            client_loop_n: 0,
            paint_n: 0,
            skip_n: 0,
            stable_paint_n: 0,
            transition_paint_n: 0,
            stable_paint_intervals: 0,
            stable_paint_interval_ns: 0,
            stable_paint_interval_buckets: [0; INTERVAL_BUCKETS],
            transition_paint_intervals: 0,
            transition_paint_interval_ns: 0,
            transition_paint_interval_buckets: [0; INTERVAL_BUCKETS],
            attach_n: 0,
            detach_n: 0,
            backend_change_n: 0,
            updated_ms: 0,
            ended: false,
        }
    }
}

/// Inputs observed at the end of one `client_frame` paint decision.
#[derive(Clone, Copy, Debug)]
pub struct FrameObs {
    pub now: Instant,
    pub painted: bool,
    pub renderer_present: bool,
    pub backend: BackendObs,
    pub prefer_cpu: Option<bool>,
    pub ingame: bool,
    pub scene_state: i32,
    pub draw: bool,
    pub full_rate: bool,
}

/// Cadence mode that must match on consecutive paints to form one interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CadenceMode {
    stable: bool,
    draw: bool,
    full_rate: bool,
    backend: BackendObs,
}

impl CadenceMode {
    fn from_obs(obs: &FrameObs) -> Self {
        Self {
            stable: stable_scene(obs.ingame, obs.scene_state),
            draw: obs.draw,
            full_rate: obs.full_rate,
            backend: obs.backend,
        }
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Compare full `Duration` values — do not floor to integer milliseconds.
/// `40ms + 1ns` must not land in the ≤40ms bucket.
fn interval_bucket(d: Duration) -> usize {
    INTERVAL_BOUNDS_MS
        .iter()
        .position(|&bound| d <= Duration::from_millis(bound))
        .unwrap_or(INTERVAL_BOUNDS_MS.len())
}

fn ns(d: Duration) -> u64 {
    d.as_nanos().min(u64::MAX as u128) as u64
}

/// Stable scene: ingame with fully built scene (`scene_state == 2`).
fn stable_scene(ingame: bool, scene_state: i32) -> bool {
    ingame && scene_state == 2
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

/// Current published observations when enabled; `None` while disabled.
/// Ended slots are included once, then pruned. Register/flush also caps
/// unread ended rows so the registry stays bounded without a reader.
pub fn read() -> Option<Vec<SlotObservation>> {
    if !ENABLED.load(Relaxed) {
        return None;
    }
    let mut reg = REGISTRY.lock().unwrap();
    let out = reg.clone();
    reg.retain(|s| !s.ended);
    Some(out)
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

/// Per-slot local accumulator. Register once when the slot thread starts.
pub struct Local {
    generation: u64,
    snap: SlotObservation,
    last_backend: BackendObs,
    /// Last paint that can pair with a future paint; cleared on mode change.
    last_paint: Option<(Instant, CadenceMode)>,
    /// Previous frame mode (even without paint) for transition detection.
    last_mode: Option<CadenceMode>,
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
            last_backend: BackendObs::Absent,
            last_paint: None,
            last_mode: None,
            last_flush: Instant::now(),
            dirty: true,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn record(&mut self, obs: FrameObs) {
        self.snap.client_loop_n = self.snap.client_loop_n.wrapping_add(1);
        self.snap.ingame = obs.ingame;
        self.snap.scene_state = obs.scene_state;
        self.snap.draw = obs.draw;
        self.snap.full_rate = obs.full_rate;
        self.snap.renderer_present = obs.renderer_present;
        self.snap.backend = obs.backend;
        self.snap.prefer_cpu = obs.prefer_cpu;

        if obs.backend != self.last_backend {
            match (self.last_backend, obs.backend) {
                (BackendObs::Absent, b) if b != BackendObs::Absent => {
                    self.snap.attach_n = self.snap.attach_n.wrapping_add(1);
                }
                (b, BackendObs::Absent) if b != BackendObs::Absent => {
                    self.snap.detach_n = self.snap.detach_n.wrapping_add(1);
                }
                _ => {
                    self.snap.backend_change_n = self.snap.backend_change_n.wrapping_add(1);
                }
            }
            self.last_backend = obs.backend;
            self.dirty = true;
        }

        let mode = CadenceMode::from_obs(&obs);
        // Break interval linkage on any cadence-mode change, including frames
        // that only skip-paint (draw/full-rate/backend/scene transitions).
        if self.last_mode.map(|m| m != mode).unwrap_or(false) {
            self.last_paint = None;
            self.dirty = true;
        }
        self.last_mode = Some(mode);

        if obs.painted {
            self.snap.paint_n = self.snap.paint_n.wrapping_add(1);
            if mode.stable {
                self.snap.stable_paint_n = self.snap.stable_paint_n.wrapping_add(1);
            } else {
                self.snap.transition_paint_n = self.snap.transition_paint_n.wrapping_add(1);
            }
            if let Some((prev, prev_mode)) = self.last_paint {
                // Same full cadence mode only — never mix 1fps with full-rate.
                if prev_mode == mode {
                    let interval = obs.now.saturating_duration_since(prev);
                    let b = interval_bucket(interval);
                    if mode.stable {
                        self.snap.stable_paint_intervals =
                            self.snap.stable_paint_intervals.wrapping_add(1);
                        self.snap.stable_paint_interval_ns = self
                            .snap
                            .stable_paint_interval_ns
                            .wrapping_add(ns(interval));
                        self.snap.stable_paint_interval_buckets[b] =
                            self.snap.stable_paint_interval_buckets[b].wrapping_add(1);
                    } else {
                        self.snap.transition_paint_intervals =
                            self.snap.transition_paint_intervals.wrapping_add(1);
                        self.snap.transition_paint_interval_ns = self
                            .snap
                            .transition_paint_interval_ns
                            .wrapping_add(ns(interval));
                        self.snap.transition_paint_interval_buckets[b] =
                            self.snap.transition_paint_interval_buckets[b].wrapping_add(1);
                    }
                }
            }
            self.last_paint = Some((obs.now, mode));
        } else {
            self.snap.skip_n = self.snap.skip_n.wrapping_add(1);
        }

        if self.dirty || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.flush(false);
        }
    }

    fn flush(&mut self, ended: bool) {
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
        self.flush(true);
    }
}

/// Classify residency from the live head and the prefer-cpu latch.
pub fn classify_backend(
    present: bool,
    prefer_cpu: Option<bool>,
    kind: Option<client::render::backend::BackendKind>,
) -> BackendObs {
    if !present {
        return BackendObs::Absent;
    }
    match kind {
        Some(client::render::backend::BackendKind::Gpu) => BackendObs::Gpu,
        Some(client::render::backend::BackendKind::Cpu) => {
            if prefer_cpu == Some(false) {
                BackendObs::CpuFallback
            } else {
                BackendObs::Cpu
            }
        }
        None => BackendObs::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Serialize tests that mutate process-wide ENABLED/REGISTRY under
    /// `cargo test` parallel execution.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_lock<R>(f: impl FnOnce() -> R) -> R {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn reset_registry() {
        REGISTRY.lock().unwrap().clear();
    }

    fn base_obs(now: Instant) -> FrameObs {
        FrameObs {
            now,
            painted: true,
            renderer_present: true,
            backend: BackendObs::Cpu,
            prefer_cpu: Some(true),
            ingame: true,
            scene_state: 2,
            draw: true,
            full_rate: true,
        }
    }

    #[test]
    fn disabled_read_is_none() {
        with_lock(|| {
            ENABLED.store(false, Relaxed);
            reset_registry();
            assert!(read().is_none());
            assert!(Local::new(1).is_none());
        });
    }

    #[test]
    fn buckets_cover_40ms_and_1s_without_ms_flooring() {
        assert_eq!(interval_bucket(Duration::from_millis(0)), 0);
        assert_eq!(interval_bucket(Duration::from_millis(10)), 0);
        assert_eq!(interval_bucket(Duration::from_millis(20)), 1);
        assert_eq!(interval_bucket(Duration::from_millis(40)), 3);
        // Sub-ms above the bound must not floor into the ≤40ms bucket.
        assert_eq!(
            interval_bucket(Duration::from_millis(40) + Duration::from_nanos(1)),
            4
        );
        assert_eq!(interval_bucket(Duration::from_millis(41)), 4);
        assert_eq!(interval_bucket(Duration::from_millis(1000)), 8);
        assert_eq!(
            interval_bucket(Duration::from_millis(1000) + Duration::from_nanos(1)),
            9
        );
        assert_eq!(interval_bucket(Duration::from_millis(2001)), 10);
        assert!(INTERVAL_BOUNDS_MS.contains(&40));
        assert!(INTERVAL_BOUNDS_MS.contains(&1000));
    }

    #[test]
    fn counters_monotonic_and_intervals_separate_stable_from_transition() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            let mut local = Local::new(42).expect("enabled");
            let t0 = Instant::now();
            let ms = Duration::from_millis;
            local.record(FrameObs {
                now: t0,
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 1,
                draw: true,
                full_rate: true,
            });
            local.record(FrameObs {
                now: t0 + ms(20),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 1,
                draw: true,
                full_rate: true,
            });
            assert_eq!(local.snap.transition_paint_n, 2);
            assert_eq!(local.snap.transition_paint_intervals, 1);
            assert_eq!(local.snap.stable_paint_intervals, 0);
            local.record(FrameObs {
                now: t0 + ms(40),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            assert_eq!(local.snap.stable_paint_n, 1);
            assert_eq!(local.snap.stable_paint_intervals, 0);
            local.record(FrameObs {
                now: t0 + ms(60),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            assert_eq!(local.snap.stable_paint_intervals, 1);
            assert_eq!(local.snap.client_loop_n, 4);
            assert_eq!(local.snap.paint_n, 4);
            local.record(FrameObs {
                now: t0 + ms(80),
                painted: false,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: false,
            });
            assert_eq!(local.snap.skip_n, 1);
            // full_rate flip on a skip must break the next paint interval.
            local.record(FrameObs {
                now: t0 + ms(100),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: false,
            });
            assert_eq!(local.snap.stable_paint_intervals, 1);
            local.record(FrameObs {
                now: t0 + ms(1100),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: false,
            });
            assert_eq!(local.snap.stable_paint_intervals, 2);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn mode_change_without_paint_breaks_interval_linkage() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            let mut local = Local::new(9).unwrap();
            let t0 = Instant::now();
            let ms = Duration::from_millis;
            let mut a = base_obs(t0);
            a.full_rate = true;
            local.record(a);
            // Skip while flipping full_rate off — no paint on the transition.
            local.record(FrameObs {
                now: t0 + ms(20),
                painted: false,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: false,
            });
            local.record(FrameObs {
                now: t0 + ms(1020),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Cpu,
                prefer_cpu: Some(true),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: false,
            });
            assert_eq!(
                local.snap.stable_paint_intervals, 0,
                "must not join full-rate paint to later 1fps paint across a skip transition"
            );
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn records_detach_and_backend_changes() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            let mut local = Local::new(7).unwrap();
            let t = Instant::now();
            local.record(FrameObs {
                now: t,
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            assert_eq!(local.snap.attach_n, 1);
            local.record(FrameObs {
                now: t,
                painted: true,
                renderer_present: true,
                backend: BackendObs::CpuFallback,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            assert_eq!(local.snap.backend_change_n, 1);
            local.record(FrameObs {
                now: t,
                painted: false,
                renderer_present: false,
                backend: BackendObs::Absent,
                prefer_cpu: None,
                ingame: true,
                scene_state: 2,
                draw: false,
                full_rate: false,
            });
            assert_eq!(local.snap.detach_n, 1);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn two_slots_independent_and_prune_after_end() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            let a = Local::new(1).unwrap();
            let b = Local::new(2).unwrap();
            assert_ne!(a.generation(), b.generation());
            let gen_a = a.generation();
            let gen_b = b.generation();
            drop(a);
            let once = read().expect("enabled");
            assert!(once.iter().any(|s| s.generation == gen_a && s.ended));
            assert!(once.iter().any(|s| s.generation == gen_b && !s.ended));
            let again = read().unwrap();
            assert!(!again.iter().any(|s| s.generation == gen_a));
            assert!(again.iter().any(|s| s.generation == gen_b));
            drop(b);
            let _ = read();
        });
    }

    #[test]
    fn ended_unread_entries_are_hard_capped() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            for i in 0..(MAX_ENDED_UNREAD + 20) {
                let local = Local::new(i as u64).unwrap();
                drop(local); // ends without a reader
            }
            let reg = REGISTRY.lock().unwrap();
            let ended = reg.iter().filter(|s| s.ended).count();
            assert!(ended <= MAX_ENDED_UNREAD, "ended={ended}");
            drop(reg);
            let _ = read();
        });
    }

    #[test]
    fn classify_distinguishes_cpu_fallback() {
        use client::render::backend::BackendKind;
        assert_eq!(classify_backend(false, None, None), BackendObs::Absent);
        assert_eq!(
            classify_backend(true, Some(true), Some(BackendKind::Cpu)),
            BackendObs::Cpu
        );
        assert_eq!(
            classify_backend(true, Some(false), Some(BackendKind::Cpu)),
            BackendObs::CpuFallback
        );
        assert_eq!(
            classify_backend(true, Some(false), Some(BackendKind::Gpu)),
            BackendObs::Gpu
        );
    }
}

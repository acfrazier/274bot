//! Opt-in per-slot renderer residency and paint-cadence observations.
//!
//! Default off. Enable before clients start. Local counters accumulate on the
//! slot thread; publication is bounded (~1s or state change). Counts are host
//! `mainredraw`/paint completions, not GPU completed or presented frames.
//! Histogram bucket edges support a conservative upper bound around the 40 ms
//! gate and ~1 s background cadence; they are not labeled percentiles.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());

/// Histogram upper bounds in milliseconds (inclusive), then unbounded.
/// Covers full-rate (~20 ms), the 40 ms gate, and ~1 s watch/background paint.
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

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn interval_bucket(d: Duration) -> usize {
    let ms = d.as_millis() as u64;
    INTERVAL_BOUNDS_MS
        .iter()
        .position(|&bound| ms <= bound)
        .unwrap_or(INTERVAL_BOUNDS_MS.len())
}

fn ns(d: Duration) -> u64 {
    d.as_nanos().min(u64::MAX as u128) as u64
}

/// Stable scene: ingame with fully built scene (`scene_state == 2`).
fn stable_scene(ingame: bool, scene_state: i32) -> bool {
    ingame && scene_state == 2
}

pub fn enable() {
    ENABLED.store(true, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

/// Current published observations when enabled; `None` while disabled.
/// Ended slots are included once, then pruned so the registry cannot grow
/// without bound after live slots finish.
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
    last_paint: Option<(Instant, bool)>,
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
        REGISTRY.lock().unwrap().push(snap.clone());
        Some(Self {
            generation,
            snap,
            last_backend: BackendObs::Absent,
            last_paint: None,
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

        let stable = stable_scene(obs.ingame, obs.scene_state);
        if obs.painted {
            self.snap.paint_n = self.snap.paint_n.wrapping_add(1);
            if stable {
                self.snap.stable_paint_n = self.snap.stable_paint_n.wrapping_add(1);
            } else {
                self.snap.transition_paint_n = self.snap.transition_paint_n.wrapping_add(1);
            }
            if let Some((prev, prev_stable)) = self.last_paint {
                // Separate stable-scene intervals from startup/transition ones.
                if prev_stable == stable {
                    let interval = obs.now.saturating_duration_since(prev);
                    let b = interval_bucket(interval);
                    if stable {
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
            self.last_paint = Some((obs.now, stable));
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

    fn reset_registry() {
        REGISTRY.lock().unwrap().clear();
        // Leave ENABLED as tests set it; generations may advance.
    }

    #[test]
    fn disabled_read_is_none() {
        ENABLED.store(false, Relaxed);
        reset_registry();
        assert!(read().is_none());
        assert!(Local::new(1).is_none());
    }

    #[test]
    fn buckets_cover_40ms_and_1s() {
        assert_eq!(interval_bucket(Duration::from_millis(0)), 0);
        assert_eq!(interval_bucket(Duration::from_millis(10)), 0);
        assert_eq!(interval_bucket(Duration::from_millis(20)), 1);
        assert_eq!(interval_bucket(Duration::from_millis(40)), 3);
        assert_eq!(interval_bucket(Duration::from_millis(41)), 4);
        assert_eq!(interval_bucket(Duration::from_millis(1000)), 8);
        assert_eq!(interval_bucket(Duration::from_millis(1001)), 9);
        assert_eq!(interval_bucket(Duration::from_millis(2001)), 10);
        // Edges exist so a conservative p99 upper bound can sit at 40ms / 1s.
        assert!(INTERVAL_BOUNDS_MS.contains(&40));
        assert!(INTERVAL_BOUNDS_MS.contains(&1000));
    }

    #[test]
    fn counters_monotonic_and_intervals_separate_stable_from_transition() {
        ENABLED.store(true, Relaxed);
        reset_registry();
        let mut local = Local::new(42).expect("enabled");
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        // Transition paints (scene rebuilding).
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
        // Crossing into stable does not join intervals across the boundary.
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
        // Skip path.
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
        assert_eq!(local.snap.client_loop_n, 5);
        // Keep process registry clean for other tests.
        drop(local);
        let _ = read();
    }

    #[test]
    fn records_detach_and_backend_changes() {
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
    }

    #[test]
    fn two_slots_independent_and_prune_after_end() {
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
        // Ended entry pruned after the read that published it.
        let again = read().unwrap();
        assert!(!again.iter().any(|s| s.generation == gen_a));
        assert!(again.iter().any(|s| s.generation == gen_b));
        drop(b);
        let _ = read();
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

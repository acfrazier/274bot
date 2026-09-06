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
//!
//! # GPU queue-completion (separate opt-in)
//!
//! `enable_gpu_completion` (requires render profile) registers
//! `queue.on_submitted_work_done` after a Texture `mainredraw`, before the
//! frame moves into the mailbox. Callback timestamps are **CPU delivery**
//! after prior GPU work, delivered on a later existing submit/poll — not
//! hardware GPU timestamps or display scanout. Latency is measured from the
//! host `mainredraw` start Instant through callback delivery (a conservative
//! upper bound on that interval only — not panel present/scanout, and not a
//! claim on total frame time before `mainredraw`). Callback intervals are
//! observed delivery cadence within one mode epoch. Outstanding callbacks
//! are bounded per slot and process-wide; permits release when the callback
//! runs or the closure is dropped. Callbacks hold only weak telemetry + an
//! RAII permit — never client/renderer/frame owners. Routine register/
//! complete traffic merges into the local snap without forcing a global
//! registry publish; publication stays on the parent ~1s / state-change path.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static GPU_COMPLETION: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());
/// Process-wide outstanding GPU completion callbacks (all slots / restarts).
static PROCESS_GPU_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Cap ended-but-unread rows so slot restart storms cannot grow the registry
/// without a reader. Live (non-ended) rows are bounded by concurrent slots.
const MAX_ENDED_UNREAD: usize = 64;

/// Max in-flight GPU completion callbacks for one slot generation.
const MAX_GPU_PENDING_SLOT: usize = 8;
/// Max in-flight GPU completion callbacks across the whole process.
const MAX_GPU_PENDING_PROCESS: usize = 256;

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

    // --- GPU queue-completion (zeros unless gpu completion enabled) ---
    /// `FrameOutput::Texture` frames observed at the host seam.
    pub gpu_frame_n: u64,
    /// `FrameOutput::PixMap` frames observed (never counted as GPU completions).
    pub cpu_frame_n: u64,
    /// Callbacks successfully registered with the queue.
    pub gpu_registered_n: u64,
    /// Callbacks delivered on the CPU (after prior submit completed).
    pub gpu_completed_n: u64,
    /// Texture frames that could not register (slot/process permit overflow).
    pub gpu_dropped_n: u64,
    /// Registered callbacks dropped/canceled without a successful complete.
    pub gpu_lost_n: u64,
    /// Outstanding registered callbacks at last merge/publish.
    pub gpu_pending_n: u64,
    /// Age of oldest outstanding submit sample in ms; 0 if none.
    pub gpu_oldest_pending_age_ms: u64,
    /// Completions while stable scene mode at submit.
    pub gpu_stable_completed_n: u64,
    /// Completions while transition mode at submit.
    pub gpu_transition_completed_n: u64,
    /// Submit/register → callback-delivery latency samples.
    pub gpu_completion_latency_n: u64,
    pub gpu_completion_latency_ns: u64,
    pub gpu_completion_latency_buckets: [u64; INTERVAL_BUCKETS],
    /// Delivery-to-delivery intervals (same submit cadence mode only).
    pub gpu_stable_completion_intervals: u64,
    pub gpu_stable_completion_interval_ns: u64,
    pub gpu_stable_completion_interval_buckets: [u64; INTERVAL_BUCKETS],
    pub gpu_transition_completion_intervals: u64,
    pub gpu_transition_completion_interval_ns: u64,
    pub gpu_transition_completion_interval_buckets: [u64; INTERVAL_BUCKETS],
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
            gpu_frame_n: 0,
            cpu_frame_n: 0,
            gpu_registered_n: 0,
            gpu_completed_n: 0,
            gpu_dropped_n: 0,
            gpu_lost_n: 0,
            gpu_pending_n: 0,
            gpu_oldest_pending_age_ms: 0,
            gpu_stable_completed_n: 0,
            gpu_transition_completed_n: 0,
            gpu_completion_latency_n: 0,
            gpu_completion_latency_ns: 0,
            gpu_completion_latency_buckets: [0; INTERVAL_BUCKETS],
            gpu_stable_completion_intervals: 0,
            gpu_stable_completion_interval_ns: 0,
            gpu_stable_completion_interval_buckets: [0; INTERVAL_BUCKETS],
            gpu_transition_completion_intervals: 0,
            gpu_transition_completion_interval_ns: 0,
            gpu_transition_completion_interval_buckets: [0; INTERVAL_BUCKETS],
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

/// Opt-in GPU queue-completion measurement. Intended only with render profile
/// enabled (launcher enforces panel + `--render-profile`).
pub fn enable_gpu_completion() {
    GPU_COMPLETION.store(true, Relaxed);
}

pub fn gpu_completion_enabled() -> bool {
    GPU_COMPLETION.load(Relaxed)
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

/// Coherent completion/lost/latency deltas drained under one lock so a
/// concurrent callback cannot partially publish count vs histogram.
#[derive(Clone, Default)]
struct GpuDeltaBatch {
    completed: u64,
    lost: u64,
    stable_completed: u64,
    transition_completed: u64,
    latency_n: u64,
    latency_ns: u64,
    latency_buckets: [u64; INTERVAL_BUCKETS],
    stable_interval_n: u64,
    stable_interval_ns: u64,
    stable_interval_buckets: [u64; INTERVAL_BUCKETS],
    transition_interval_n: u64,
    transition_interval_ns: u64,
    transition_interval_buckets: [u64; INTERVAL_BUCKETS],
}

/// Shared callback target: weak-held by queue callbacks; strong only in `Local`.
struct GpuShared {
    generation: u64,
    live: AtomicBool,
    /// Bumped on cadence-mode change and Local drop so A→B→A late callbacks
    /// cannot pair intervals with a later same-valued mode era.
    mode_epoch: AtomicU64,
    deltas: Mutex<GpuDeltaBatch>,
    pending: AtomicUsize,
    /// Wall-ms submit stamps for outstanding permits (0 = empty). Fixed size.
    pending_submit_ms: [AtomicU64; MAX_GPU_PENDING_SLOT],
    /// Last delivered completion for interval pairing (same mode epoch only).
    last_delivery: Mutex<Option<(Instant, u64, CadenceMode)>>,
}

impl GpuShared {
    fn new(generation: u64) -> Self {
        const ZERO: AtomicU64 = AtomicU64::new(0);
        Self {
            generation,
            live: AtomicBool::new(true),
            mode_epoch: AtomicU64::new(0),
            deltas: Mutex::new(GpuDeltaBatch::default()),
            pending: AtomicUsize::new(0),
            pending_submit_ms: [ZERO; MAX_GPU_PENDING_SLOT],
            last_delivery: Mutex::new(None),
        }
    }

    fn bump_mode_epoch(&self) {
        self.mode_epoch.fetch_add(1, Relaxed);
        if let Ok(mut last) = self.last_delivery.lock() {
            *last = None;
        }
    }

    fn oldest_pending_age_ms(&self, now_ms: u64) -> u64 {
        let mut oldest = 0u64;
        for cell in &self.pending_submit_ms {
            let t = cell.load(Relaxed);
            if t == 0 {
                continue;
            }
            let age = now_ms.saturating_sub(t);
            if oldest == 0 || age > oldest {
                oldest = age;
            }
        }
        oldest
    }

    /// Record a successful CPU delivery. Does **not** clear `pending_submit_ms`
    /// — only the owning `GpuPermit` release may clear the stamp so a reused
    /// cell cannot lose a newer registration's timestamp.
    fn on_complete(&self, sample_at: Instant, mode: CadenceMode, epoch: u64) -> bool {
        if !self.live.load(Relaxed) {
            return false;
        }
        let now = Instant::now();
        let latency = now.saturating_duration_since(sample_at);
        let b = interval_bucket(latency);

        let mut interval: Option<(Duration, bool)> = None;
        if let Ok(mut last) = self.last_delivery.lock() {
            if let Some((prev, prev_epoch, prev_mode)) = *last {
                if prev_epoch == epoch && prev_mode == mode {
                    interval = Some((now.saturating_duration_since(prev), mode.stable));
                }
            }
            *last = Some((now, epoch, mode));
        }

        let Ok(mut d) = self.deltas.lock() else {
            return false;
        };
        d.completed = d.completed.wrapping_add(1);
        d.latency_n = d.latency_n.wrapping_add(1);
        d.latency_ns = d.latency_ns.wrapping_add(ns(latency));
        d.latency_buckets[b] = d.latency_buckets[b].wrapping_add(1);
        if mode.stable {
            d.stable_completed = d.stable_completed.wrapping_add(1);
        } else {
            d.transition_completed = d.transition_completed.wrapping_add(1);
        }
        if let Some((iv, stable)) = interval {
            let ib = interval_bucket(iv);
            if stable {
                d.stable_interval_n = d.stable_interval_n.wrapping_add(1);
                d.stable_interval_ns = d.stable_interval_ns.wrapping_add(ns(iv));
                d.stable_interval_buckets[ib] = d.stable_interval_buckets[ib].wrapping_add(1);
            } else {
                d.transition_interval_n = d.transition_interval_n.wrapping_add(1);
                d.transition_interval_ns = d.transition_interval_ns.wrapping_add(ns(iv));
                d.transition_interval_buckets[ib] =
                    d.transition_interval_buckets[ib].wrapping_add(1);
            }
        }
        true
    }

    fn note_lost(&self) {
        if let Ok(mut d) = self.deltas.lock() {
            d.lost = d.lost.wrapping_add(1);
        }
    }

    fn take_deltas(&self) -> GpuDeltaBatch {
        self.deltas
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default()
    }
}

/// RAII permit: process + slot pending. Releases if callback runs or drops.
/// Pending timestamp is cleared **only** here (not in `on_complete`).
struct GpuPermit {
    shared: Weak<GpuShared>,
    pending_idx: usize,
    process_held: bool,
    slot_held: bool,
    /// Set when `on_complete` succeeded so Drop does not count lost/canceled.
    completed: bool,
}

impl GpuPermit {
    fn mark_completed(&mut self) {
        self.completed = true;
    }

    fn release(&mut self) {
        if self.slot_held {
            if let Some(s) = self.shared.upgrade() {
                s.pending.fetch_sub(1, Relaxed);
                if self.pending_idx < MAX_GPU_PENDING_SLOT {
                    s.pending_submit_ms[self.pending_idx].store(0, Relaxed);
                }
                if !self.completed {
                    s.note_lost();
                }
            }
            self.slot_held = false;
        }
        if self.process_held {
            PROCESS_GPU_PENDING.fetch_sub(1, Relaxed);
            self.process_held = false;
        }
    }
}

impl Drop for GpuPermit {
    fn drop(&mut self) {
        self.release();
    }
}

fn try_acquire_gpu_permit(shared: &Arc<GpuShared>) -> Option<GpuPermit> {
    // Process-wide bound.
    loop {
        let cur = PROCESS_GPU_PENDING.load(Relaxed);
        if cur >= MAX_GPU_PENDING_PROCESS {
            return None;
        }
        if PROCESS_GPU_PENDING
            .compare_exchange_weak(cur, cur + 1, Relaxed, Relaxed)
            .is_ok()
        {
            break;
        }
    }
    // Per-slot bound.
    let prev = shared.pending.fetch_add(1, Relaxed);
    if prev >= MAX_GPU_PENDING_SLOT {
        shared.pending.fetch_sub(1, Relaxed);
        PROCESS_GPU_PENDING.fetch_sub(1, Relaxed);
        return None;
    }
    // Claim a pending timestamp slot (fixed array, no heap growth).
    let submit_ms = wall_ms().max(1); // 0 is empty sentinel
    let mut idx = MAX_GPU_PENDING_SLOT;
    for i in 0..MAX_GPU_PENDING_SLOT {
        if shared.pending_submit_ms[i]
            .compare_exchange(0, submit_ms, Relaxed, Relaxed)
            .is_ok()
        {
            idx = i;
            break;
        }
    }
    if idx == MAX_GPU_PENDING_SLOT {
        // Array full (shouldn't happen if pending count is honest) — roll back.
        shared.pending.fetch_sub(1, Relaxed);
        PROCESS_GPU_PENDING.fetch_sub(1, Relaxed);
        return None;
    }
    Some(GpuPermit {
        shared: Arc::downgrade(shared),
        pending_idx: idx,
        process_held: true,
        slot_held: true,
        completed: false,
    })
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
    /// Present only when GPU completion profiling is enabled at register time.
    gpu: Option<Arc<GpuShared>>,
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
        let gpu = if GPU_COMPLETION.load(Relaxed) {
            Some(Arc::new(GpuShared::new(generation)))
        } else {
            None
        };
        Some(Self {
            generation,
            snap,
            last_backend: BackendObs::Absent,
            last_paint: None,
            last_mode: None,
            last_flush: Instant::now(),
            dirty: true,
            gpu,
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
            if let Some(g) = self.gpu.as_ref() {
                g.bump_mode_epoch();
            }
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

        self.merge_gpu_deltas();
        if self.dirty || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.flush(false);
        }
    }

    /// After a real `mainredraw`, observe the frame kind and optionally register
    /// a GPU queue-completion callback **before** the frame enters the mailbox.
    ///
    /// `sample_at` is the host Instant at `mainredraw` start (or an earlier
    /// precise frame-start). Latency is that sample → callback delivery only —
    /// not hardware completion, scanout, or pre-`mainredraw` host work.
    ///
    /// `register_done` must call `queue.on_submitted_work_done(cb)` and nothing
    /// else. Disabled / CPU paths never allocate a callback or call `register_done`.
    ///
    /// Routine register/complete traffic updates the local snap only; it does
    /// **not** mark dirty or take the global `REGISTRY` lock. Publication stays
    /// on the parent ~1s / real state-change policy in `record`/`flush`.
    pub fn observe_painted_output(
        &mut self,
        is_gpu_texture: bool,
        sample_at: Instant,
        register_done: impl FnOnce(Box<dyn FnOnce() + Send + 'static>),
    ) {
        let Some(shared) = self.gpu.as_ref() else {
            return;
        };
        if !is_gpu_texture {
            self.snap.cpu_frame_n = self.snap.cpu_frame_n.wrapping_add(1);
            return;
        }
        self.snap.gpu_frame_n = self.snap.gpu_frame_n.wrapping_add(1);
        let mode = self.last_mode.unwrap_or(CadenceMode {
            stable: false,
            draw: false,
            full_rate: false,
            backend: BackendObs::Absent,
        });
        let epoch = shared.mode_epoch.load(Relaxed);
        let Some(permit) = try_acquire_gpu_permit(shared) else {
            self.snap.gpu_dropped_n = self.snap.gpu_dropped_n.wrapping_add(1);
            // Overflow invalidates coverage — publish on next state/1Hz path.
            self.dirty = true;
            return;
        };
        self.snap.gpu_registered_n = self.snap.gpu_registered_n.wrapping_add(1);
        let weak = Arc::downgrade(shared);
        let generation = shared.generation;
        register_done(Box::new(move || {
            let mut permit = permit;
            if let Some(s) = weak.upgrade() {
                if s.generation == generation && s.on_complete(sample_at, mode, epoch) {
                    permit.mark_completed();
                }
            }
            // Permit Drop releases outstanding counts and clears the pending
            // stamp once; lost is counted if not mark_completed.
        }));
        // Local merge only — no REGISTRY flush on the hot paint path.
        self.merge_gpu_deltas();
    }

    fn merge_gpu_deltas(&mut self) {
        let Some(shared) = self.gpu.as_ref() else {
            return;
        };
        let batch = shared.take_deltas();
        // Single coherent take: count + latency + buckets never partially drop.
        if batch.completed != 0 {
            self.snap.gpu_completed_n = self.snap.gpu_completed_n.wrapping_add(batch.completed);
        }
        if batch.lost != 0 {
            self.snap.gpu_lost_n = self.snap.gpu_lost_n.wrapping_add(batch.lost);
            // Lost/canceled invalidates throughput qualification.
            self.dirty = true;
        }
        self.snap.gpu_stable_completed_n = self
            .snap
            .gpu_stable_completed_n
            .wrapping_add(batch.stable_completed);
        self.snap.gpu_transition_completed_n = self
            .snap
            .gpu_transition_completed_n
            .wrapping_add(batch.transition_completed);
        if batch.latency_n != 0 {
            self.snap.gpu_completion_latency_n = self
                .snap
                .gpu_completion_latency_n
                .wrapping_add(batch.latency_n);
            self.snap.gpu_completion_latency_ns = self
                .snap
                .gpu_completion_latency_ns
                .wrapping_add(batch.latency_ns);
            for i in 0..INTERVAL_BUCKETS {
                self.snap.gpu_completion_latency_buckets[i] = self.snap
                    .gpu_completion_latency_buckets[i]
                    .wrapping_add(batch.latency_buckets[i]);
            }
        }
        if batch.stable_interval_n != 0 {
            self.snap.gpu_stable_completion_intervals = self
                .snap
                .gpu_stable_completion_intervals
                .wrapping_add(batch.stable_interval_n);
            self.snap.gpu_stable_completion_interval_ns = self
                .snap
                .gpu_stable_completion_interval_ns
                .wrapping_add(batch.stable_interval_ns);
            for i in 0..INTERVAL_BUCKETS {
                self.snap.gpu_stable_completion_interval_buckets[i] = self.snap
                    .gpu_stable_completion_interval_buckets[i]
                    .wrapping_add(batch.stable_interval_buckets[i]);
            }
        }
        if batch.transition_interval_n != 0 {
            self.snap.gpu_transition_completion_intervals = self
                .snap
                .gpu_transition_completion_intervals
                .wrapping_add(batch.transition_interval_n);
            self.snap.gpu_transition_completion_interval_ns = self
                .snap
                .gpu_transition_completion_interval_ns
                .wrapping_add(batch.transition_interval_ns);
            for i in 0..INTERVAL_BUCKETS {
                self.snap.gpu_transition_completion_interval_buckets[i] = self.snap
                    .gpu_transition_completion_interval_buckets[i]
                    .wrapping_add(batch.transition_interval_buckets[i]);
            }
        }
        self.snap.gpu_pending_n = shared.pending.load(Relaxed) as u64;
        self.snap.gpu_oldest_pending_age_ms = shared.oldest_pending_age_ms(wall_ms());
    }

    fn flush(&mut self, ended: bool) {
        self.merge_gpu_deltas();
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
        if let Some(shared) = self.gpu.as_ref() {
            shared.live.store(false, Relaxed);
            shared.bump_mode_epoch();
            // Break interval pairing so a restarted generation cannot pair
            // against this one's last delivery if any Arc briefly remains.
            if let Ok(mut last) = shared.last_delivery.lock() {
                *last = None;
            }
        }
        self.flush(true);
        // Drop Arc after flush so in-flight callbacks still upgrading see
        // live=false; once Local is gone, Weak upgrades fail (no contamination).
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

/// True when every GPU texture frame was registered with no overflow and no
/// lost/canceled callbacks. Still not hardware present proof. Throughput
/// claims also need `gpu_completion_coverage_complete` (completed == registered,
/// nothing pending).
pub fn gpu_registration_complete(s: &SlotObservation) -> bool {
    s.gpu_frame_n > 0
        && s.gpu_dropped_n == 0
        && s.gpu_lost_n == 0
        && s.gpu_registered_n == s.gpu_frame_n
}

/// True when registration was complete and every registered callback has
/// delivered (pending cleared). Required to qualify completion throughput.
pub fn gpu_completion_coverage_complete(s: &SlotObservation) -> bool {
    gpu_registration_complete(s)
        && s.gpu_pending_n == 0
        && s.gpu_completed_n == s.gpu_registered_n
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
        // Drain any leaked process pending from prior aborted tests.
        PROCESS_GPU_PENDING.store(0, Relaxed);
        GPU_COMPLETION.store(false, Relaxed);
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

    #[test]
    fn gpu_disabled_allocates_and_registers_nothing() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(false, Relaxed);
            let mut local = Local::new(1).unwrap();
            assert!(local.gpu.is_none());
            let mut fired = false;
            local.record(base_obs(Instant::now()));
            local.observe_painted_output(true, Instant::now(), |_| {
                fired = true;
            });
            assert!(!fired);
            assert_eq!(local.snap.gpu_frame_n, 0);
            assert_eq!(local.snap.gpu_registered_n, 0);
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 0);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_cpu_path_never_counts_as_completion() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(3).unwrap();
            local.record(base_obs(Instant::now()));
            let mut fired = false;
            local.observe_painted_output(false, Instant::now(), |_| {
                fired = true;
            });
            assert!(!fired);
            assert_eq!(local.snap.cpu_frame_n, 1);
            assert_eq!(local.snap.gpu_frame_n, 0);
            assert_eq!(local.snap.gpu_completed_n, 0);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_one_callback_per_registration_and_permit_release() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(5).unwrap();
            local.record(base_obs(Instant::now()));
            let mut cbs: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
            local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            assert_eq!(local.snap.gpu_frame_n, 1);
            assert_eq!(local.snap.gpu_registered_n, 1);
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 1);
            assert_eq!(cbs.len(), 1);
            cbs.pop().unwrap()();
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_completed_n, 1);
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 0);
            assert_eq!(local.snap.gpu_pending_n, 0);
            assert!(gpu_registration_complete(&local.snap));
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_overflow_drops_and_permit_drop_without_fire_releases() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(6).unwrap();
            local.record(base_obs(Instant::now()));
            let mut held: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
            for _ in 0..MAX_GPU_PENDING_SLOT {
                local.observe_painted_output(true, Instant::now(), |cb| held.push(cb));
            }
            assert_eq!(local.snap.gpu_registered_n, MAX_GPU_PENDING_SLOT as u64);
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), MAX_GPU_PENDING_SLOT);
            // Next must drop.
            let mut attempted = false;
            local.observe_painted_output(true, Instant::now(), |_| {
                attempted = true;
            });
            assert!(!attempted);
            assert_eq!(local.snap.gpu_dropped_n, 1);
            assert_eq!(local.snap.gpu_frame_n, MAX_GPU_PENDING_SLOT as u64 + 1);
            // Dropping closures without firing must release permits and count lost.
            held.clear();
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 0);
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_pending_n, 0);
            assert_eq!(local.snap.gpu_completed_n, 0);
            assert_eq!(local.snap.gpu_lost_n, MAX_GPU_PENDING_SLOT as u64);
            assert!(!gpu_registration_complete(&local.snap));
            assert!(!gpu_completion_coverage_complete(&local.snap));
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_lost_unfired_hides_registration_complete() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(61).unwrap();
            local.record(base_obs(Instant::now()));
            let mut held = Vec::new();
            for _ in 0..3 {
                local.observe_painted_output(true, Instant::now(), |cb| held.push(cb));
            }
            assert_eq!(local.snap.gpu_registered_n, 3);
            assert_eq!(local.snap.gpu_dropped_n, 0);
            // Without lost, registration would look complete while completed==0.
            held.clear();
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_lost_n, 3);
            assert_eq!(local.snap.gpu_completed_n, 0);
            assert!(!gpu_registration_complete(&local.snap));
            assert!(!gpu_completion_coverage_complete(&local.snap));
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_observe_does_not_flush_registry_every_frame() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(62).unwrap();
            local.record(base_obs(Instant::now()));
            // Force a clean post-flush baseline with known published counters.
            local.flush(false);
            let gen = local.generation();
            let before = REGISTRY
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.generation == gen)
                .map(|s| s.gpu_frame_n)
                .unwrap_or(u64::MAX);
            let mut cbs = Vec::new();
            for _ in 0..4 {
                local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            }
            assert_eq!(local.snap.gpu_frame_n, 4);
            // Hot path must not publish routine register traffic.
            let after = REGISTRY
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.generation == gen)
                .map(|s| s.gpu_frame_n)
                .unwrap_or(u64::MAX);
            assert_eq!(after, before, "observe must not REGISTRY-flush every GPU frame");
            for cb in cbs {
                cb();
            }
            local.merge_gpu_deltas();
            // Completions alone still do not force registry publish.
            let after_complete = REGISTRY
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.generation == gen)
                .map(|s| s.gpu_completed_n)
                .unwrap_or(u64::MAX);
            assert_eq!(after_complete, 0, "complete traffic must not force registry flush");
            local.record(base_obs(Instant::now())); // ~state path can publish
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_merge_deltas_keeps_count_and_latency_coherent() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(63).unwrap();
            local.record(base_obs(Instant::now()));
            let shared = local.gpu.as_ref().unwrap().clone();
            let mode = CadenceMode {
                stable: true,
                draw: true,
                full_rate: true,
                backend: BackendObs::Gpu,
            };
            // Concurrent completes while merge drains repeatedly.
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let shared_t = shared.clone();
            let b2 = barrier.clone();
            let t = std::thread::spawn(move || {
                b2.wait();
                for _ in 0..200 {
                    assert!(shared_t.on_complete(Instant::now(), mode, 0));
                }
            });
            barrier.wait();
            for _ in 0..50 {
                local.merge_gpu_deltas();
            }
            t.join().unwrap();
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_completed_n, 200);
            assert_eq!(local.snap.gpu_completion_latency_n, 200);
            let bucket_sum: u64 = local.snap.gpu_completion_latency_buckets.iter().sum();
            assert_eq!(bucket_sum, 200);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_on_complete_does_not_clear_pending_stamp() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(64).unwrap();
            local.record(base_obs(Instant::now()));
            let shared = local.gpu.as_ref().unwrap().clone();
            let mut p1 = try_acquire_gpu_permit(&shared).expect("p1");
            let idx = p1.pending_idx;
            let stamp = shared.pending_submit_ms[idx].load(Relaxed);
            assert!(stamp > 0);
            let mode = CadenceMode {
                stable: true,
                draw: true,
                full_rate: true,
                backend: BackendObs::Gpu,
            };
            assert!(shared.on_complete(Instant::now(), mode, 0));
            assert_eq!(
                shared.pending_submit_ms[idx].load(Relaxed),
                stamp,
                "on_complete must not clear pending stamp"
            );
            // Reclaim attempt must fail while p1 still holds the cell.
            assert!(
                shared.pending_submit_ms[idx]
                    .compare_exchange(0, 1, Relaxed, Relaxed)
                    .is_err()
            );
            p1.mark_completed();
            drop(p1);
            assert_eq!(shared.pending_submit_ms[idx].load(Relaxed), 0);
            // New registration can reuse the cell after single clear.
            let p2 = try_acquire_gpu_permit(&shared).expect("p2");
            assert_eq!(p2.pending_idx, idx);
            assert!(shared.pending_submit_ms[idx].load(Relaxed) > 0);
            drop(p2); // lost ok
            local.merge_gpu_deltas();
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_late_callback_after_slot_end_does_not_contaminate_restart() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut first = Local::new(10).unwrap();
            let gen1 = first.generation();
            first.record(base_obs(Instant::now()));
            let mut cb = None;
            first.observe_painted_output(true, Instant::now(), |c| cb = Some(c));
            drop(first);
            let _ = read(); // prune ended

            let mut second = Local::new(10).unwrap();
            let gen2 = second.generation();
            assert_ne!(gen1, gen2);
            second.record(base_obs(Instant::now()));
            // Fire late callback from first generation.
            cb.take().unwrap()();
            second.merge_gpu_deltas();
            assert_eq!(
                second.snap.gpu_completed_n, 0,
                "late callback must not credit the restarted slot"
            );
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 0);
            drop(second);
            let _ = read();
        });
    }

    #[test]
    fn gpu_completion_intervals_separate_stable_from_transition() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(11).unwrap();
            let t0 = Instant::now();
            // Transition scene.
            local.record(FrameObs {
                now: t0,
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 1,
                draw: true,
                full_rate: true,
            });
            let mut cbs = Vec::new();
            local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            local.record(FrameObs {
                now: t0 + Duration::from_millis(20),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 1,
                draw: true,
                full_rate: true,
            });
            local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            for cb in cbs.drain(..) {
                cb();
            }
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_transition_completed_n, 2);
            assert_eq!(local.snap.gpu_transition_completion_intervals, 1);
            assert_eq!(local.snap.gpu_stable_completion_intervals, 0);

            // Stable scene — mode change breaks pairing with transition completions.
            local.record(FrameObs {
                now: t0 + Duration::from_millis(40),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            local.record(FrameObs {
                now: t0 + Duration::from_millis(60),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            local.observe_painted_output(true, Instant::now(), |cb| cbs.push(cb));
            for cb in cbs.drain(..) {
                cb();
            }
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_stable_completed_n, 2);
            assert_eq!(local.snap.gpu_stable_completion_intervals, 1);
            assert_eq!(local.snap.gpu_transition_completion_intervals, 1);
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_mode_epoch_blocks_a_b_a_late_callback_interval() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            let mut local = Local::new(65).unwrap();
            let t0 = Instant::now();
            // Stable A
            local.record(FrameObs {
                now: t0,
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            let mut first_era = None;
            local.observe_painted_output(true, Instant::now(), |c| first_era = Some(c));
            // Hold first-era callback; transition B
            local.record(FrameObs {
                now: t0 + Duration::from_millis(20),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 1,
                draw: true,
                full_rate: true,
            });
            let mut mid = None;
            local.observe_painted_output(true, Instant::now(), |c| mid = Some(c));
            mid.take().unwrap()();
            // Stable A again
            local.record(FrameObs {
                now: t0 + Duration::from_millis(40),
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            let mut second_era = None;
            local.observe_painted_output(true, Instant::now(), |c| second_era = Some(c));
            second_era.take().unwrap()();
            // Late first-era callback after A→B→A must not invent a stable interval.
            first_era.take().unwrap()();
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_stable_completed_n, 2);
            assert_eq!(
                local.snap.gpu_stable_completion_intervals, 0,
                "A→B→A late callback must not pair across mode epochs"
            );
            drop(local);
            let _ = read();
        });
    }

    #[test]
    fn gpu_process_wide_bound_includes_multiple_slots() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);
            // Force process near cap then assert next registration drops.
            PROCESS_GPU_PENDING.store(MAX_GPU_PENDING_PROCESS, Relaxed);
            let mut local = Local::new(12).unwrap();
            local.record(base_obs(Instant::now()));
            let mut attempted = false;
            local.observe_painted_output(true, Instant::now(), |_| {
                attempted = true;
            });
            assert!(!attempted);
            assert_eq!(local.snap.gpu_dropped_n, 1);
            PROCESS_GPU_PENDING.store(0, Relaxed);
            drop(local);
            let _ = read();
        });
    }

    /// Real wgpu queue proof with bounded submitted work. Fail-closed when the
    /// adapter/device is unavailable — never soft-pass. Run explicitly:
    /// `cargo test -p host --lib gpu_real_queue -- --ignored --nocapture`
    /// Production path never polls/waits solely to improve completion delivery.
    #[test]
    #[ignore = "real GPU adapter proof; run with --ignored"]
    fn gpu_real_queue_on_submitted_work_done_delivers_once() {
        with_lock(|| {
            ENABLED.store(true, Relaxed);
            reset_registry();
            GPU_COMPLETION.store(true, Relaxed);

            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = pollster::block_on(instance.request_adapter(
                &wgpu::RequestAdapterOptions::default(),
            ))
            .expect("gpu_real_queue proof requires an adapter (unavailable on this host)");
            let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
                .expect("gpu_real_queue proof requires a device");

            // Bounded real queue work (not an empty submit-only API check).
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_completion_smoke"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &[1u8; 16]);
            queue.submit(std::iter::empty());

            let sample_at = Instant::now();
            let mut local = Local::new(99).unwrap();
            local.record(FrameObs {
                now: sample_at,
                painted: true,
                renderer_present: true,
                backend: BackendObs::Gpu,
                prefer_cpu: Some(false),
                ingame: true,
                scene_state: 2,
                draw: true,
                full_rate: true,
            });
            let done = Arc::new(StdMutex::new(false));
            let done2 = done.clone();
            local.observe_painted_output(true, sample_at, |cb| {
                queue.on_submitted_work_done(move || {
                    cb();
                    *done2.lock().unwrap() = true;
                });
            });
            // Drive delivery the same way production does: a later submit/poll.
            // This test waits only as proof; host never adds a completion poll.
            queue.submit(std::iter::empty());
            let start = Instant::now();
            while !*done.lock().unwrap() {
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("callback not delivered within 5s");
                }
            }
            local.merge_gpu_deltas();
            assert_eq!(local.snap.gpu_registered_n, 1);
            assert_eq!(local.snap.gpu_completed_n, 1);
            assert_eq!(local.snap.gpu_completion_latency_n, 1);
            assert_eq!(PROCESS_GPU_PENDING.load(Relaxed), 0);
            assert!(gpu_completion_coverage_complete(&local.snap));
            drop(local);
            let _ = read();
            GPU_COMPLETION.store(false, Relaxed);
        });
    }
}

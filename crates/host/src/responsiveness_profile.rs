//! Opt-in bounded responsiveness observations.
//!
//! Default off. Two independent metrics:
//!
//! 1. **decode → script dispatch** — host `PLAYER_INFO` gen edge after
//!    `mainloop`/drain (decoded update that drives the tick) through host-play
//!    entry into [`script::SlotScript::on_game_tick`]. Not every dirty family;
//!    not snapshot encode alone; not isolate JS body completion.
//! 2. **focused input → UI endpoint** — frontend-specific, labeled honestly:
//!    - **Panel:** focused capture enqueue (`Down` / key-down) →
//!      `GameView::present` of a newer mailbox generation. This is host
//!      texture bind/upload for the focused image, **not** display scanout /
//!      swapchain present / human-visible pixel light.
//!    - **TUI (PTY):** key-press handling that mutates app state → successful
//!      `terminal.draw` flush. Headless TUI has no draw; that path is
//!      unavailable.
//!
//! Display/compositor scanout remains an **explicit unavailable** capability
//! on panel (`missing_capability` in published JSON). Do not treat paint call,
//! queue admission, or UI construction alone as visible completion.
//!
//! Disabled path is free (atomics false; no Local / no pending rings).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Fine (≤1 ms) latency histograms — default off; independent of coarse legacy bins.
static FINE_ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());
/// Process-wide pending focused-input samples (panel + TUI), all slots.
static INPUT_PENDING: Mutex<VecDeque<InputPending>> = Mutex::new(VecDeque::new());
/// Frontend surface that last armed input endpoints (for JSON semantics).
static INPUT_SURFACE: Mutex<InputSurface> = Mutex::new(InputSurface::Unknown);
/// Shared process-local monotonic origin for publisher capture + sample/read brackets.
/// Not comparable across process runs; durations within one process are.
static MONO_ORIGIN: OnceLock<Instant> = OnceLock::new();

const MAX_ENDED_UNREAD: usize = 64;
/// Max outstanding decode→dispatch stamps per slot generation.
const MAX_DECODE_PENDING: usize = 8;
/// Max outstanding focused-input stamps process-wide.
const MAX_INPUT_PENDING: usize = 256;

/// Histogram upper bounds in ms (inclusive on exact Duration), then overflow.
/// Covers the 100 ms responsiveness gate and slower outliers.
/// Legacy coarse bins retained for existing consumers; cannot prove a 2 ms margin.
pub const LATENCY_BOUNDS_MS: [u64; 10] = [5, 10, 20, 25, 40, 50, 100, 250, 500, 1000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_MS.len() + 1;

/// Fine latency inclusive upper bounds: 1, 2, …, 100 ms, then overflow.
/// Sibling of [`LATENCY_BOUNDS_MS`]; never interpolated from coarse bins.
pub const FINE_LATENCY_BOUND_COUNT: usize = 100;
pub const FINE_LATENCY_BUCKETS: usize = FINE_LATENCY_BOUND_COUNT + 1;

const fn fine_latency_bounds_ms_const() -> [u64; FINE_LATENCY_BOUND_COUNT] {
    let mut a = [0u64; FINE_LATENCY_BOUND_COUNT];
    let mut i = 0;
    while i < FINE_LATENCY_BOUND_COUNT {
        a[i] = (i as u64) + 1;
        i += 1;
    }
    a
}

pub const FINE_LATENCY_BOUNDS_MS: [u64; FINE_LATENCY_BOUND_COUNT] = fine_latency_bounds_ms_const();

/// Process-local clock domain label emitted with sample/read brackets.
pub const CLOCK_DOMAIN: &str = "responsiveness_process_mono";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSurface {
    Unknown,
    Panel,
    Tui,
    /// Headless / no controlling terminal — draw flush cannot complete.
    TuiHeadless,
}

impl InputSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Panel => "panel",
            Self::Tui => "tui_pty",
            Self::TuiHeadless => "tui_headless",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotObservation {
    pub slot_id: u64,
    pub generation: u64,
    pub updated_ms: u64,
    pub ended: bool,
    /// Decode-counter publication bracket (ns since process mono origin).
    /// Stamped only when Local.flush writes decode state; not retimed by input.
    /// 0/0 = never decode-published. Floor/ceil ms siblings are derived at emit.
    pub decode_capture_mono_ns_lower: u64,
    pub decode_capture_mono_ns_upper: u64,
    /// Input-counter publication bracket (ns). Stamped only on input registry
    /// mutations; lower begins before INPUT_PENDING cut when applicable.
    pub input_capture_mono_ns_lower: u64,
    pub input_capture_mono_ns_upper: u64,

    // --- decode → script dispatch ---
    /// PLAYER_INFO edges observed at host after_drain.
    pub decode_edge_n: u64,
    /// Samples that reached on_game_tick entry.
    pub dispatch_n: u64,
    /// Tick edges dropped because script was not Running / no dispatch.
    pub decode_canceled_n: u64,
    /// Cancels recorded while decode_pending was empty (unmatched). Subtract
    /// from coverage identity: edge = dispatch + canceled - unmatched_canceled
    /// + pending + dropped + lost.
    pub decode_unmatched_canceled_n: u64,
    /// Pending stamps discarded on slot end / overflow without dispatch.
    pub decode_lost_n: u64,
    pub decode_dropped_n: u64,
    pub decode_pending_n: u64,
    pub decode_latency_n: u64,
    pub decode_latency_ns: u64,
    pub decode_latency_buckets: [u64; LATENCY_BUCKETS],
    /// Fine sibling of decode_latency_buckets (1 ms bins 0–100 + overflow).
    /// Counts only while fine profile is on; zeros when fine off.
    pub decode_fine_latency_buckets: [u64; FINE_LATENCY_BUCKETS],

    // --- focused input → UI endpoint (surface-specific) ---
    pub input_start_n: u64,
    pub input_complete_n: u64,
    pub input_canceled_n: u64,
    pub input_lost_n: u64,
    pub input_dropped_n: u64,
    pub input_pending_n: u64,
    pub input_latency_n: u64,
    pub input_latency_ns: u64,
    pub input_latency_buckets: [u64; LATENCY_BUCKETS],
    pub input_fine_latency_buckets: [u64; FINE_LATENCY_BUCKETS],
}

impl SlotObservation {
    const fn zero(slot_id: u64, generation: u64) -> Self {
        Self {
            slot_id,
            generation,
            updated_ms: 0,
            ended: false,
            decode_capture_mono_ns_lower: 0,
            decode_capture_mono_ns_upper: 0,
            input_capture_mono_ns_lower: 0,
            input_capture_mono_ns_upper: 0,
            decode_edge_n: 0,
            dispatch_n: 0,
            decode_canceled_n: 0,
            decode_unmatched_canceled_n: 0,
            decode_lost_n: 0,
            decode_dropped_n: 0,
            decode_pending_n: 0,
            decode_latency_n: 0,
            decode_latency_ns: 0,
            decode_latency_buckets: [0; LATENCY_BUCKETS],
            decode_fine_latency_buckets: [0; FINE_LATENCY_BUCKETS],
            input_start_n: 0,
            input_complete_n: 0,
            input_canceled_n: 0,
            input_lost_n: 0,
            input_dropped_n: 0,
            input_pending_n: 0,
            input_latency_n: 0,
            input_latency_ns: 0,
            input_latency_buckets: [0; LATENCY_BUCKETS],
            input_fine_latency_buckets: [0; FINE_LATENCY_BUCKETS],
        }
    }
}

/// Registry clone plus the mono bracket enclosing the registry lock/read.
/// Bracket uses ns; ms helpers floor lower / ceil upper so ms still encloses.
#[derive(Clone, Debug)]
pub struct ReadSnapshot {
    pub slots: Vec<SlotObservation>,
    pub read_mono_ns_lower: u64,
    pub read_mono_ns_upper: u64,
}

impl ReadSnapshot {
    pub fn read_mono_ms_lower(&self) -> u64 {
        mono_ns_floor_ms(self.read_mono_ns_lower)
    }
    pub fn read_mono_ms_upper(&self) -> u64 {
        mono_ns_ceil_ms(self.read_mono_ns_upper)
    }
}

#[derive(Clone, Copy)]
struct DecodePending {
    at: Instant,
    cohort_id: Option<crate::responsiveness_cohort::EventId>,
}

struct InputPending {
    slot_id: u64,
    at: Instant,
    /// Mailbox generation that must be presented (panel); 0 = any next draw (TUI).
    require_gen: u64,
    surface: InputSurface,
    cohort_id: Option<crate::responsiveness_cohort::EventId>,
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mono_origin() -> Instant {
    *MONO_ORIGIN.get_or_init(Instant::now)
}

/// Mono ns of `at` relative to the process responsiveness origin.
pub fn mono_ns(at: Instant) -> u64 {
    at.saturating_duration_since(mono_origin())
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

/// Mono ns of `Instant::now()` on the shared domain.
pub fn mono_ns_now() -> u64 {
    mono_ns(Instant::now())
}

/// Floor ms from mono ns — use for inclusive **lower** edges only.
pub fn mono_ns_floor_ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// Ceil ms from mono ns — use for inclusive **upper** edges so the ms interval
/// still encloses the real instant (plain as_millis on both ends does not).
pub fn mono_ns_ceil_ms(ns: u64) -> u64 {
    if ns == 0 {
        return 0;
    }
    ns.div_ceil(1_000_000)
}

/// @deprecated prefer mono_ns; kept for tests that only need coarse order.
pub fn mono_ms(at: Instant) -> u64 {
    mono_ns_floor_ms(mono_ns(at))
}

pub fn mono_ms_now() -> u64 {
    mono_ns_floor_ms(mono_ns_now())
}

fn stamp_ns_bracket(lo: &mut u64, hi: &mut u64, t0: Instant, t1: Instant) {
    let a = mono_ns(t0);
    let b = mono_ns(t1).max(a);
    *lo = a;
    *hi = b;
}

fn latency_bucket(d: Duration) -> usize {
    LATENCY_BOUNDS_MS
        .iter()
        .position(|&bound| d <= Duration::from_millis(bound))
        .unwrap_or(LATENCY_BOUNDS_MS.len())
}

fn fine_latency_bucket(d: Duration) -> usize {
    FINE_LATENCY_BOUNDS_MS
        .iter()
        .position(|&bound| d <= Duration::from_millis(bound))
        .unwrap_or(FINE_LATENCY_BOUND_COUNT)
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
            false
        } else {
            true
        }
    });
}

pub fn enable() {
    let _ = mono_origin(); // pin domain before any capture/read stamps
    ENABLED.store(true, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

/// Opt-in fine (≤1 ms) latency histograms. No-op cost when false: coarse path
/// only. Requires [`enable`] for any recording.
pub fn enable_fine() {
    FINE_ENABLED.store(true, Relaxed);
}

pub fn fine_enabled() -> bool {
    FINE_ENABLED.load(Relaxed)
}

pub fn set_input_surface(surface: InputSurface) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    *INPUT_SURFACE.lock().unwrap() = surface;
}

pub fn input_surface() -> InputSurface {
    *INPUT_SURFACE.lock().unwrap()
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

pub fn read() -> Option<Vec<SlotObservation>> {
    read_bracketed().map(|s| s.slots)
}

/// Clone live registry rows and record the mono bracket around that lock/read.
/// Ended rows are returned once then dropped from the registry (same as [`read`]).
pub fn read_bracketed() -> Option<ReadSnapshot> {
    if !ENABLED.load(Relaxed) {
        return None;
    }
    let t0 = Instant::now();
    let mut reg = REGISTRY.lock().unwrap();
    let out = reg.clone();
    reg.retain(|s| !s.ended);
    drop(reg);
    let t1 = Instant::now();
    let lo = mono_ns(t0);
    let hi = mono_ns(t1).max(lo);
    Some(ReadSnapshot {
        slots: out,
        read_mono_ns_lower: lo,
        read_mono_ns_upper: hi,
    })
}

/// Conservative p99 from fixed histogram buckets: returns the **upper bound**
/// of the bucket that first reaches cumulative count ≥ ceil(0.99 * n).
/// Overflow bucket yields `None` (unbounded / unavailable precise p99).
/// Empty samples yield `None`.
pub fn p99_upper_bound_ms(buckets: &[u64; LATENCY_BUCKETS]) -> Option<u64> {
    p99_upper_bound_from(buckets, &LATENCY_BOUNDS_MS)
}

/// Same rule as [`p99_upper_bound_ms`] for the fine (1 ms) histogram.
pub fn fine_p99_upper_bound_ms(buckets: &[u64; FINE_LATENCY_BUCKETS]) -> Option<u64> {
    p99_upper_bound_from(buckets, &FINE_LATENCY_BOUNDS_MS)
}

fn p99_upper_bound_from(buckets: &[u64], bounds_ms: &[u64]) -> Option<u64> {
    let n: u64 = buckets.iter().sum();
    if n == 0 {
        return None;
    }
    // Smallest k such that cum(k) / n >= 0.99 ⇔ cum * 100 >= n * 99
    let mut cum = 0u64;
    for (i, &c) in buckets.iter().enumerate() {
        cum = cum.saturating_add(c);
        if cum.saturating_mul(100) >= n.saturating_mul(99) {
            if i < bounds_ms.len() {
                return Some(bounds_ms[i]);
            }
            return None; // overflow bucket — no finite upper bound
        }
    }
    None
}

/// Coverage: every decode edge was either dispatched, canceled, or lost;
/// no pending; no drops. Does not imply the 100 ms budget.
pub fn decode_coverage_complete(s: &SlotObservation) -> bool {
    s.decode_dropped_n == 0
        && s.decode_pending_n == 0
        && s.decode_edge_n
            == s.dispatch_n
                .saturating_add(s.decode_canceled_n)
                .saturating_add(s.decode_lost_n)
}

/// Exact native accounting identity using the unmatched-cancel sibling:
/// `edge == dispatch + canceled - unmatched_canceled + pending + dropped + lost`.
/// Legacy [`decode_coverage_complete`] is unchanged and does not use unmatched.
pub fn decode_accounting_exact(s: &SlotObservation) -> bool {
    let rhs = s
        .dispatch_n
        .saturating_add(
            s.decode_canceled_n
                .saturating_sub(s.decode_unmatched_canceled_n),
        )
        .saturating_add(s.decode_pending_n)
        .saturating_add(s.decode_dropped_n)
        .saturating_add(s.decode_lost_n);
    s.decode_edge_n == rhs && s.decode_unmatched_canceled_n <= s.decode_canceled_n
}

/// Coverage for input samples: starts accounted as complete/canceled/lost;
/// no pending/drops.
pub fn input_coverage_complete(s: &SlotObservation) -> bool {
    s.input_dropped_n == 0
        && s.input_pending_n == 0
        && s.input_start_n
            == s.input_complete_n
                .saturating_add(s.input_canceled_n)
                .saturating_add(s.input_lost_n)
}

/// Whether the configured surface can complete a true display-visible ack.
/// Panel and headless TUI cannot; PTY TUI completes terminal buffer flush only.
pub fn visible_ack_status() -> VisibleAckStatus {
    match input_surface() {
        InputSurface::Tui => VisibleAckStatus {
            available: true,
            endpoint: "terminal_draw_flush_after_state_mutating_key",
            missing_capability: None,
            means: "crossterm/ratatui draw flush to PTY after key handling; not OS compositor pixel light",
        },
        InputSurface::Panel => VisibleAckStatus {
            available: false,
            endpoint: "game_view_present_after_focused_input",
            missing_capability: Some(
                "no surface frame-presented / display scanout / swapchain feedback hook on panel path",
            ),
            means: "measured endpoint is GameView::present (bind/upload) after focused Down/key-down enqueue; not display scanout",
        },
        InputSurface::TuiHeadless => VisibleAckStatus {
            available: false,
            endpoint: "none",
            missing_capability: Some(
                "headless TUI has no controlling terminal; terminal.draw never runs",
            ),
            means: "input→visible unavailable without PTY draw",
        },
        InputSurface::Unknown => VisibleAckStatus {
            available: false,
            endpoint: "none",
            missing_capability: Some("input surface not armed (panel/TUI did not set surface)"),
            means: "no focused-input endpoint selected",
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VisibleAckStatus {
    pub available: bool,
    pub endpoint: &'static str,
    pub missing_capability: Option<&'static str>,
    pub means: &'static str,
}

/// Per-slot local counters; created only while enabled.
pub struct Local {
    generation: u64,
    snap: SlotObservation,
    decode_pending: VecDeque<DecodePending>,
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
            decode_pending: VecDeque::with_capacity(MAX_DECODE_PENDING),
            last_flush: Instant::now(),
            dirty: true,
        })
    }

    pub fn slot_id(&self) -> u64 {
        self.snap.slot_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Host after_drain: PLAYER_INFO gen moved this frame.
    pub fn note_decode_edge(&mut self, at: Instant) {
        self.snap.decode_edge_n = self.snap.decode_edge_n.wrapping_add(1);
        if self.decode_pending.len() >= MAX_DECODE_PENDING {
            crate::responsiveness_cohort::dropped_start(
                self.snap.slot_id,
                self.generation,
                mono_ns(at),
                crate::responsiveness_cohort::Surface::Decode,
            );
            self.snap.decode_dropped_n = self.snap.decode_dropped_n.wrapping_add(1);
            self.dirty = true;
            self.maybe_flush();
            return;
        }
        let cohort_id = crate::responsiveness_cohort::start(
            self.snap.slot_id,
            self.generation,
            mono_ns(at),
            crate::responsiveness_cohort::Surface::Decode,
        );
        self.decode_pending
            .push_back(DecodePending { at, cohort_id });
        self.snap.decode_pending_n = self.decode_pending.len() as u64;
        self.dirty = true;
        self.maybe_flush();
    }

    /// host-play entered on_game_tick for this slot (pairs oldest pending).
    pub fn note_script_dispatch(&mut self, at: Instant) {
        let Some(pending) = self.decode_pending.pop_front() else {
            // Dispatch without a recorded edge — ignore (ordering glitch / disabled mid-run).
            return;
        };
        let d = at.saturating_duration_since(pending.at);
        if let Some(id) = pending.cohort_id {
            crate::responsiveness_cohort::complete(id, mono_ns(at));
        }
        self.record_decode_latency(d);
        self.snap.dispatch_n = self.snap.dispatch_n.wrapping_add(1);
        self.snap.decode_pending_n = self.decode_pending.len() as u64;
        self.dirty = true;
        self.maybe_flush();
    }

    /// Tick edge will not dispatch (Idle/Paused/Stopping/no instance, etc.).
    /// Cancels one pending stamp if any; otherwise counts cancel against an
    /// edge that was never stamped (should not happen if callers pair).
    pub fn note_script_canceled(&mut self) {
        if let Some(pending) = self.decode_pending.pop_front() {
            if let Some(id) = pending.cohort_id {
                crate::responsiveness_cohort::cancel(id);
            }
            self.snap.decode_canceled_n = self.snap.decode_canceled_n.wrapping_add(1);
            self.snap.decode_pending_n = self.decode_pending.len() as u64;
        } else {
            // Unmatched cancel: pending empty. Still increments canceled for
            // legacy coverage math; sibling unmatched counter restores identity.
            self.snap.decode_canceled_n = self.snap.decode_canceled_n.wrapping_add(1);
            self.snap.decode_unmatched_canceled_n =
                self.snap.decode_unmatched_canceled_n.wrapping_add(1);
        }
        self.dirty = true;
        self.maybe_flush();
    }

    fn record_decode_latency(&mut self, d: Duration) {
        let b = latency_bucket(d);
        self.snap.decode_latency_n = self.snap.decode_latency_n.wrapping_add(1);
        self.snap.decode_latency_ns = self.snap.decode_latency_ns.wrapping_add(ns(d));
        self.snap.decode_latency_buckets[b] = self.snap.decode_latency_buckets[b].wrapping_add(1);
        if fine_enabled() {
            let fb = fine_latency_bucket(d);
            self.snap.decode_fine_latency_buckets[fb] =
                self.snap.decode_fine_latency_buckets[fb].wrapping_add(1);
        }
    }

    #[allow(dead_code)] // kept for Local-side pairing if a future path completes on Local
    fn record_input_latency(&mut self, d: Duration) {
        let b = latency_bucket(d);
        self.snap.input_latency_n = self.snap.input_latency_n.wrapping_add(1);
        self.snap.input_latency_ns = self.snap.input_latency_ns.wrapping_add(ns(d));
        self.snap.input_latency_buckets[b] = self.snap.input_latency_buckets[b].wrapping_add(1);
        if fine_enabled() {
            let fb = fine_latency_bucket(d);
            self.snap.input_fine_latency_buckets[fb] =
                self.snap.input_fine_latency_buckets[fb].wrapping_add(1);
        }
    }

    /// Merge process-wide input completions addressed to this slot.
    pub fn merge_input_from_global(&mut self) {
        // Completions are applied immediately in global helpers via registry
        // mutation for live rows; this keeps local pending counts coherent
        // when the Local is the sole writer for decode. Input counts that
        // land only on the registry are refreshed on flush from registry —
        // see publish_input_* helpers that update registry by slot_id.
    }

    fn maybe_flush(&mut self) {
        if self.dirty || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.flush(false);
        }
    }

    fn flush(&mut self, ended: bool) {
        self.flush_from(Instant::now(), ended);
    }

    /// `t0` must precede any INPUT_PENDING cut or counter merge this publication
    /// claims to enclose (Drop loses pending before calling this).
    fn flush_from(&mut self, t0: Instant, ended: bool) {
        // Decode bracket: [t0, t1] around pending-count + registry decode write.
        // Input bracket: conservative UNION of prior input-capture (counter cuts
        // from with_live_slot) and this flush's pending refresh/merge window —
        // preserving the old stamp alone would leave entry.input_pending_n from
        // the fresh INPUT_PENDING cut outside the claimed input bracket.
        self.snap.ended = ended;
        self.snap.updated_ms = wall_ms();
        self.snap.decode_pending_n = self.decode_pending.len() as u64;
        let pending = INPUT_PENDING.lock().unwrap();
        self.snap.input_pending_n = pending
            .iter()
            .filter(|p| p.slot_id == self.snap.slot_id)
            .count() as u64;
        drop(pending);

        let mut reg = REGISTRY.lock().unwrap();
        if let Some(entry) = reg.iter_mut().find(|s| s.generation == self.generation) {
            let input_start_n = entry.input_start_n.max(self.snap.input_start_n);
            let input_complete_n = entry.input_complete_n.max(self.snap.input_complete_n);
            let input_canceled_n = entry.input_canceled_n.max(self.snap.input_canceled_n);
            let input_lost_n = entry.input_lost_n.max(self.snap.input_lost_n);
            let input_dropped_n = entry.input_dropped_n.max(self.snap.input_dropped_n);
            let input_latency_n = entry.input_latency_n.max(self.snap.input_latency_n);
            let input_latency_ns = entry.input_latency_ns.max(self.snap.input_latency_ns);
            let mut input_latency_buckets = self.snap.input_latency_buckets;
            for i in 0..LATENCY_BUCKETS {
                input_latency_buckets[i] =
                    entry.input_latency_buckets[i].max(input_latency_buckets[i]);
            }
            let mut input_fine_latency_buckets = self.snap.input_fine_latency_buckets;
            for i in 0..FINE_LATENCY_BUCKETS {
                input_fine_latency_buckets[i] =
                    entry.input_fine_latency_buckets[i].max(input_fine_latency_buckets[i]);
            }
            let prior_in_lo = entry.input_capture_mono_ns_lower;
            let prior_in_hi = entry.input_capture_mono_ns_upper;
            *entry = self.snap.clone();
            entry.input_start_n = input_start_n;
            entry.input_complete_n = input_complete_n;
            entry.input_canceled_n = input_canceled_n;
            entry.input_lost_n = input_lost_n;
            entry.input_dropped_n = input_dropped_n;
            entry.input_latency_n = input_latency_n;
            entry.input_latency_ns = input_latency_ns;
            entry.input_latency_buckets = input_latency_buckets;
            entry.input_fine_latency_buckets = input_fine_latency_buckets;
            entry.input_pending_n = self.snap.input_pending_n;
            let t1 = Instant::now();
            stamp_ns_bracket(
                &mut entry.decode_capture_mono_ns_lower,
                &mut entry.decode_capture_mono_ns_upper,
                t0,
                t1,
            );
            // Union prior input bracket with this flush window (pending refresh).
            let flush_lo = mono_ns(t0);
            let flush_hi = mono_ns(t1).max(flush_lo);
            let input_touched = prior_in_lo != 0
                || prior_in_hi != 0
                || input_start_n != 0
                || input_complete_n != 0
                || input_canceled_n != 0
                || input_lost_n != 0
                || input_dropped_n != 0
                || entry.input_pending_n != 0;
            if input_touched {
                if prior_in_lo == 0 && prior_in_hi == 0 {
                    entry.input_capture_mono_ns_lower = flush_lo;
                    entry.input_capture_mono_ns_upper = flush_hi;
                } else {
                    entry.input_capture_mono_ns_lower = prior_in_lo.min(flush_lo);
                    entry.input_capture_mono_ns_upper = prior_in_hi.max(flush_hi);
                }
            } else {
                entry.input_capture_mono_ns_lower = prior_in_lo;
                entry.input_capture_mono_ns_upper = prior_in_hi;
            }
            self.snap.decode_capture_mono_ns_lower = entry.decode_capture_mono_ns_lower;
            self.snap.decode_capture_mono_ns_upper = entry.decode_capture_mono_ns_upper;
            self.snap.input_capture_mono_ns_lower = entry.input_capture_mono_ns_lower;
            self.snap.input_capture_mono_ns_upper = entry.input_capture_mono_ns_upper;
            self.snap.input_start_n = input_start_n;
            self.snap.input_complete_n = input_complete_n;
            self.snap.input_canceled_n = input_canceled_n;
            self.snap.input_lost_n = input_lost_n;
            self.snap.input_dropped_n = input_dropped_n;
            self.snap.input_latency_n = input_latency_n;
            self.snap.input_latency_ns = input_latency_ns;
            self.snap.input_latency_buckets = input_latency_buckets;
            self.snap.input_fine_latency_buckets = input_fine_latency_buckets;
        } else if !ended {
            let t1 = Instant::now();
            stamp_ns_bracket(
                &mut self.snap.decode_capture_mono_ns_lower,
                &mut self.snap.decode_capture_mono_ns_upper,
                t0,
                t1,
            );
            if self.snap.input_start_n != 0
                || self.snap.input_complete_n != 0
                || self.snap.input_lost_n != 0
                || self.snap.input_pending_n != 0
            {
                stamp_ns_bracket(
                    &mut self.snap.input_capture_mono_ns_lower,
                    &mut self.snap.input_capture_mono_ns_upper,
                    t0,
                    t1,
                );
            }
            reg.push(self.snap.clone());
        } else {
            let t1 = Instant::now();
            stamp_ns_bracket(
                &mut self.snap.decode_capture_mono_ns_lower,
                &mut self.snap.decode_capture_mono_ns_upper,
                t0,
                t1,
            );
            if self.snap.input_start_n != 0
                || self.snap.input_complete_n != 0
                || self.snap.input_lost_n != 0
                || self.snap.input_pending_n != 0
            {
                stamp_ns_bracket(
                    &mut self.snap.input_capture_mono_ns_lower,
                    &mut self.snap.input_capture_mono_ns_upper,
                    t0,
                    t1,
                );
            }
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
        // Drop in-flight bridge events for this generation so a restart of
        // the same username cannot mis-pair stale dispatch/cancel.
        discard_bridge_for(self.snap.slot_id, self.generation);
        while let Some(pending) = self.decode_pending.pop_front() {
            if let Some(id) = pending.cohort_id {
                crate::responsiveness_cohort::lost(id);
            }
            self.snap.decode_lost_n = self.snap.decode_lost_n.wrapping_add(1);
        }
        self.snap.decode_pending_n = 0;
        // Bracket lower before INPUT_PENDING cut + lost accounting + flush merge.
        let t0 = Instant::now();
        let mut pending = INPUT_PENDING.lock().unwrap();
        let before = pending.len();
        let cohort_ids: Vec<_> = pending
            .iter()
            .filter(|p| p.slot_id == self.snap.slot_id)
            .filter_map(|p| p.cohort_id)
            .collect();
        pending.retain(|p| p.slot_id != self.snap.slot_id);
        let lost = (before - pending.len()) as u64;
        drop(pending);
        for id in cohort_ids {
            crate::responsiveness_cohort::lost(id);
        }
        if lost > 0 {
            self.snap.input_lost_n = self.snap.input_lost_n.wrapping_add(lost);
        }
        self.flush_from(t0, true);
    }
}

/// Apply `f` to the live (or most recent) row for `slot_id`, stamping the
/// **input** capture bracket from `t0` through registry unlock. Callers that
/// cut INPUT_PENDING must pass a `t0` taken **before** that cut so the
/// bracket encloses the pending/counter cut, not only the registry write.
fn with_live_slot_from(t0: Instant, slot_id: u64, f: impl FnOnce(&mut SlotObservation)) {
    let mut reg = REGISTRY.lock().unwrap();
    if let Some(entry) = reg
        .iter_mut()
        .rev()
        .find(|s| s.slot_id == slot_id && !s.ended)
    {
        f(entry);
        entry.updated_ms = wall_ms();
        let t1 = Instant::now();
        stamp_ns_bracket(
            &mut entry.input_capture_mono_ns_lower,
            &mut entry.input_capture_mono_ns_upper,
            t0,
            t1,
        );
    } else if let Some(entry) = reg.iter_mut().rev().find(|s| s.slot_id == slot_id) {
        f(entry);
        entry.updated_ms = wall_ms();
        let t1 = Instant::now();
        stamp_ns_bracket(
            &mut entry.input_capture_mono_ns_lower,
            &mut entry.input_capture_mono_ns_upper,
            t0,
            t1,
        );
    }
}

/// Focused actionable input start (panel Down/key-down or TUI state-mutating key).
/// `require_gen`: panel mailbox generation that must advance (≥ this after store);
/// use 0 for TUI (next draw completes). Returns false if disabled or dropped.
pub fn note_input_start(slot_id: u64, at: Instant, require_gen: u64) -> bool {
    if !ENABLED.load(Relaxed) {
        return false;
    }
    // Bracket lower must precede pending cut + counter bump.
    let t0 = Instant::now();
    let surface = *INPUT_SURFACE.lock().unwrap();
    let cohort_surface = match surface {
        InputSurface::Panel => crate::responsiveness_cohort::Surface::Panel,
        _ => crate::responsiveness_cohort::Surface::Tui,
    };
    let mut q = INPUT_PENDING.lock().unwrap();
    if q.len() >= MAX_INPUT_PENDING {
        drop(q);
        if crate::responsiveness_cohort::enabled() {
            match live_generation_for(slot_id) {
                Some(generation) => crate::responsiveness_cohort::dropped_start(
                    slot_id,
                    generation,
                    mono_ns(at),
                    cohort_surface,
                ),
                None => crate::responsiveness_cohort::missing_generation_start(
                    slot_id,
                    mono_ns(at),
                    cohort_surface,
                ),
            }
        }
        with_live_slot_from(t0, slot_id, |s| {
            s.input_dropped_n = s.input_dropped_n.wrapping_add(1);
            s.input_start_n = s.input_start_n.wrapping_add(1);
        });
        return false;
    }
    let cohort_id = if crate::responsiveness_cohort::enabled() {
        match live_generation_for(slot_id) {
            Some(generation) => crate::responsiveness_cohort::start(
                slot_id,
                generation,
                mono_ns(at),
                cohort_surface,
            ),
            None => {
                crate::responsiveness_cohort::missing_generation_start(
                    slot_id,
                    mono_ns(at),
                    cohort_surface,
                );
                None
            }
        }
    } else {
        None
    };
    q.push_back(InputPending {
        slot_id,
        at,
        require_gen,
        surface,
        cohort_id,
    });
    let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
    drop(q);
    with_live_slot_from(t0, slot_id, |s| {
        s.input_start_n = s.input_start_n.wrapping_add(1);
        s.input_pending_n = pending_n;
    });
    true
}

/// Cancel the oldest pending input for `slot_id` (ignored key, capture off mid-flight).
pub fn note_input_canceled(slot_id: u64) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    let t0 = Instant::now();
    let mut q = INPUT_PENDING.lock().unwrap();
    if let Some(pos) = q.iter().position(|p| p.slot_id == slot_id) {
        let pending = q.remove(pos).unwrap();
        if let Some(id) = pending.cohort_id {
            crate::responsiveness_cohort::cancel(id);
        }
        let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
        drop(q);
        with_live_slot_from(t0, slot_id, |s| {
            s.input_canceled_n = s.input_canceled_n.wrapping_add(1);
            s.input_pending_n = pending_n;
        });
    }
}

/// Panel: a new mailbox generation was presented for the focused slot.
/// Completes pending inputs only after [`bind_input_to_mailbox_gen`] set a
/// non-zero `require_gen` and that generation has been presented (`<=`).
/// Unbound (`require_gen == 0`) samples wait so a pre-input freeze present
/// cannot pair early.
pub fn note_panel_present(slot_id: u64, presented_gen: u64, at: Instant) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    complete_input(slot_id, at, |p| {
        p.surface == InputSurface::Panel && p.require_gen != 0 && p.require_gen <= presented_gen
    });
}

/// TUI: terminal.draw finished successfully after possible key handling.
pub fn note_tui_draw_flush(slot_id: u64, at: Instant) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    complete_input(slot_id, at, |p| {
        p.surface == InputSurface::Tui && p.slot_id == slot_id
    });
}

/// Complete all matching pending inputs for slot (FIFO among matches).
fn complete_input(slot_id: u64, at: Instant, pred: impl Fn(&InputPending) -> bool) {
    // t0 before pending cut so input bracket encloses the deque mutation.
    let t0 = Instant::now();
    let mut q = INPUT_PENDING.lock().unwrap();
    let mut completed: Vec<(Duration, Option<crate::responsiveness_cohort::EventId>)> = Vec::new();
    let mut i = 0;
    while i < q.len() {
        if q[i].slot_id == slot_id && pred(&q[i]) {
            let p = q.remove(i).unwrap();
            completed.push((at.saturating_duration_since(p.at), p.cohort_id));
        } else {
            i += 1;
        }
    }
    let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
    drop(q);
    if completed.is_empty() {
        return;
    }
    with_live_slot_from(t0, slot_id, |s| {
        for (d, cohort_id) in completed {
            if let Some(id) = cohort_id {
                crate::responsiveness_cohort::complete(id, mono_ns(at));
            }
            let b = latency_bucket(d);
            s.input_complete_n = s.input_complete_n.wrapping_add(1);
            s.input_latency_n = s.input_latency_n.wrapping_add(1);
            s.input_latency_ns = s.input_latency_ns.wrapping_add(ns(d));
            s.input_latency_buckets[b] = s.input_latency_buckets[b].wrapping_add(1);
            if fine_enabled() {
                let fb = fine_latency_bucket(d);
                s.input_fine_latency_buckets[fb] = s.input_fine_latency_buckets[fb].wrapping_add(1);
            }
        }
        s.input_pending_n = pending_n;
    });
}

/// After panel mailbox store, stamp pending panel inputs that still have
/// `require_gen==0` with this generation so a later present can pair.
/// Call from the slot thread whenever a frame is stored after drain/mainloop
/// (not only on same-frame actionable input) so starts that landed on a
/// skipped-paint tick still bind on the next store.
pub fn bind_input_to_mailbox_gen(slot_id: u64, gen: u64) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    let mut q = INPUT_PENDING.lock().unwrap();
    for p in q.iter_mut() {
        if p.slot_id == slot_id && p.surface == InputSurface::Panel && p.require_gen == 0 {
            p.require_gen = gen;
        }
    }
}

// --- Cross-thread decode completion without holding Local (host-play) ---

/// Process-wide bound on host-play → slot-thread decode bridge events.
const MAX_DECODE_BRIDGE: usize = 256;

static DECODE_BRIDGE: Mutex<VecDeque<DecodeBridgeEv>> = Mutex::new(VecDeque::new());

enum DecodeBridgeEv {
    Dispatch {
        slot_id: u64,
        /// Local generation that owned the decode edge; mismatches are ignored.
        generation: u64,
        at: Instant,
    },
    Cancel {
        slot_id: u64,
        generation: u64,
    },
}

fn live_generation_for(slot_id: u64) -> Option<u64> {
    let reg = REGISTRY.lock().unwrap();
    reg.iter()
        .rev()
        .find(|s| s.slot_id == slot_id && !s.ended)
        .map(|s| s.generation)
}

fn push_decode_bridge(ev: DecodeBridgeEv) {
    let mut q = DECODE_BRIDGE.lock().unwrap();
    if q.len() >= MAX_DECODE_BRIDGE {
        // Drop oldest so a stuck foreign slot cannot grow the queue unboundedly.
        let _ = q.pop_front();
    }
    q.push_back(ev);
}

/// host-play: script dispatch entered. Stamped with the live Local generation
/// so a stop/restart of the same username cannot pair stale events.
pub fn note_script_dispatch_global(slot_id: u64, at: Instant) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    let Some(generation) = live_generation_for(slot_id) else {
        return;
    };
    push_decode_bridge(DecodeBridgeEv::Dispatch {
        slot_id,
        generation,
        at,
    });
}

pub fn note_script_canceled_global(slot_id: u64) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    let Some(generation) = live_generation_for(slot_id) else {
        return;
    };
    push_decode_bridge(DecodeBridgeEv::Cancel {
        slot_id,
        generation,
    });
}

/// Discard in-flight bridge events for one Local generation (slot end / restart).
fn discard_bridge_for(slot_id: u64, generation: u64) {
    let mut q = DECODE_BRIDGE.lock().unwrap();
    q.retain(|ev| match ev {
        DecodeBridgeEv::Dispatch {
            slot_id: s,
            generation: g,
            ..
        }
        | DecodeBridgeEv::Cancel {
            slot_id: s,
            generation: g,
        } => !(*s == slot_id && *g == generation),
    });
}

impl Local {
    /// Drain bridge events for this slot **generation** (call each tick).
    pub fn drain_bridge(&mut self) {
        let mut q = DECODE_BRIDGE.lock().unwrap();
        let mut i = 0;
        while i < q.len() {
            let take = match &q[i] {
                DecodeBridgeEv::Dispatch {
                    slot_id,
                    generation,
                    ..
                }
                | DecodeBridgeEv::Cancel {
                    slot_id,
                    generation,
                } => *slot_id == self.snap.slot_id && *generation == self.generation,
            };
            if take {
                match q.remove(i).unwrap() {
                    DecodeBridgeEv::Dispatch { at, .. } => self.note_script_dispatch(at),
                    DecodeBridgeEv::Cancel { .. } => self.note_script_canceled(),
                }
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    pub(crate) static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    pub(crate) fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Isolate fine flag + leftover registry rows between tests.
        FINE_ENABLED.store(false, Relaxed);
        REGISTRY.lock().unwrap().clear();
        INPUT_PENDING.lock().unwrap().clear();
        DECODE_BRIDGE.lock().unwrap().clear();
        crate::responsiveness_cohort::test_reset();
        g
    }

    fn fresh_local(name: &str) -> Local {
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        DECODE_BRIDGE.lock().unwrap().clear();
        Local::new(slot_id_for(name)).expect("enabled")
    }

    #[test]
    fn disabled_path_is_none() {
        let _g = lock_tests();
        // Cannot safely unset ENABLED for other tests; only check new without enable
        // is None when we construct before enable in isolation — use a unique path:
        // Local::new returns None only when ENABLED is false. If prior tests enabled,
        // skip hard assert when already on.
        if !enabled() {
            assert!(Local::new(1).is_none());
        }
    }

    #[test]
    fn buckets_include_boundaries() {
        let _g = lock_tests();
        assert_eq!(latency_bucket(Duration::ZERO), 0);
        assert_eq!(latency_bucket(Duration::from_millis(100)), 6);
        assert_eq!(latency_bucket(Duration::from_micros(100_001)), 7);
        assert_eq!(latency_bucket(Duration::from_millis(1001)), 10);
    }

    #[test]
    fn p99_upper_bound_from_buckets() {
        let _g = lock_tests();
        let mut b = [0u64; LATENCY_BUCKETS];
        // 100 samples all in ≤50ms → p99 ≤50
        b[5] = 100; // index of 50ms bound
        assert_eq!(p99_upper_bound_ms(&b), Some(50));
        // 100 samples: 98 in ≤50, 2 in ≤100 → p99 ≤100
        b = [0u64; LATENCY_BUCKETS];
        b[5] = 98;
        b[6] = 2;
        assert_eq!(p99_upper_bound_ms(&b), Some(100));
        // all overflow → None
        b = [0u64; LATENCY_BUCKETS];
        b[10] = 10;
        assert_eq!(p99_upper_bound_ms(&b), None);
        assert_eq!(p99_upper_bound_ms(&[0u64; LATENCY_BUCKETS]), None);
    }

    #[test]
    fn decode_pair_dispatch_and_cancel_and_order() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-decode-a");
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        l.note_decode_edge(t0 + Duration::from_millis(1));
        l.note_script_dispatch(t0 + Duration::from_millis(30));
        assert_eq!(l.snap.dispatch_n, 1);
        assert_eq!(l.snap.decode_latency_n, 1);
        assert_eq!(l.snap.decode_pending_n, 1);
        l.note_script_canceled();
        assert_eq!(l.snap.decode_canceled_n, 1);
        assert_eq!(l.snap.decode_pending_n, 0);
        assert!(decode_coverage_complete(&l.snap));
        // prevent Drop lost noise into registry for other tests
        l.decode_pending.clear();
    }

    #[test]
    fn decode_overflow_drops_and_breaks_coverage() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-decode-ovf");
        let t0 = Instant::now();
        for _ in 0..MAX_DECODE_PENDING {
            l.note_decode_edge(t0);
        }
        l.note_decode_edge(t0);
        assert_eq!(l.snap.decode_dropped_n, 1);
        assert_eq!(l.snap.decode_pending_n, MAX_DECODE_PENDING as u64);
        assert!(!decode_coverage_complete(&l.snap));
        while l.decode_pending.pop_front().is_some() {}
        l.snap.decode_pending_n = 0;
    }

    #[test]
    fn decode_delayed_sample_records_full_duration() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-decode-delay");
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        l.note_script_dispatch(t0 + Duration::from_millis(120));
        assert_eq!(l.snap.decode_latency_n, 1);
        // 120ms → bucket index for 250ms bound
        assert_eq!(l.snap.decode_latency_buckets[7], 1);
        assert_eq!(
            p99_upper_bound_ms(&l.snap.decode_latency_buckets),
            Some(250)
        );
    }

    #[test]
    fn drop_loses_pending_decode() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-decode-lost");
        l.note_decode_edge(Instant::now());
        drop(l);
        let rows = read().expect("on");
        let row = rows
            .iter()
            .find(|s| s.slot_id == slot_id_for("resp-decode-lost"))
            .expect("row");
        assert!(row.ended);
        assert_eq!(row.decode_lost_n, 1);
        assert_eq!(row.decode_pending_n, 0);
    }

    #[test]
    fn bridge_dispatch_pairs_on_drain() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-bridge");
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        note_script_dispatch_global(l.slot_id(), t0 + Duration::from_millis(15));
        l.drain_bridge();
        assert_eq!(l.snap.dispatch_n, 1);
        assert_eq!(l.snap.decode_pending_n, 0);
    }

    #[test]
    fn input_panel_present_pairs_and_tui_draw() {
        let _g = lock_tests();
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        set_input_surface(InputSurface::Panel);
        let mut l = fresh_local("resp-input-panel");
        let sid = l.slot_id();
        let t0 = Instant::now();
        assert!(note_input_start(sid, t0, 0));
        bind_input_to_mailbox_gen(sid, 7);
        note_panel_present(sid, 7, t0 + Duration::from_millis(40));
        // refresh from registry
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(row.input_complete_n, 1);
        assert_eq!(row.input_start_n, 1);
        assert!(input_coverage_complete(row));
        let st = visible_ack_status();
        assert!(!st.available);
        assert!(st.missing_capability.is_some());

        set_input_surface(InputSurface::Tui);
        let mut l2 = fresh_local("resp-input-tui");
        let sid2 = l2.slot_id();
        let t1 = Instant::now();
        assert!(note_input_start(sid2, t1, 0));
        note_tui_draw_flush(sid2, t1 + Duration::from_millis(8));
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid2).expect("row");
        assert_eq!(row.input_complete_n, 1);
        let st = visible_ack_status();
        assert!(st.available);
        // silence drop lost
        l.decode_pending.clear();
        l2.decode_pending.clear();
    }

    #[test]
    fn input_overflow_and_cancel() {
        let _g = lock_tests();
        enable();
        set_input_surface(InputSurface::Tui);
        let mut l = fresh_local("resp-input-ovf");
        let sid = l.slot_id();
        // Fill process queue
        {
            let mut q = INPUT_PENDING.lock().unwrap();
            q.clear();
            for _ in 0..MAX_INPUT_PENDING {
                q.push_back(InputPending {
                    slot_id: 0xdead,
                    at: Instant::now(),
                    require_gen: 0,
                    surface: InputSurface::Tui,
                    cohort_id: None,
                });
            }
        }
        assert!(!note_input_start(sid, Instant::now(), 0));
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(row.input_dropped_n, 1);
        // drain foreign pending so other tests are clean
        INPUT_PENDING.lock().unwrap().clear();

        assert!(note_input_start(sid, Instant::now(), 0));
        note_input_canceled(sid);
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(row.input_canceled_n, 1);
        assert_eq!(row.input_pending_n, 0);
        l.decode_pending.clear();
    }

    #[test]
    fn headless_visible_ack_unavailable() {
        let _g = lock_tests();
        enable();
        set_input_surface(InputSurface::TuiHeadless);
        let st = visible_ack_status();
        assert!(!st.available);
        assert!(st
            .missing_capability
            .unwrap()
            .contains("no controlling terminal"));
    }

    #[test]
    fn panel_present_ignores_unbound_until_bind() {
        let _g = lock_tests();
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        set_input_surface(InputSurface::Panel);
        let mut l = fresh_local("resp-deferred-bind");
        let sid = l.slot_id();
        let t0 = Instant::now();
        assert!(note_input_start(sid, t0, 0));
        // Present before bind must not complete (require_gen still 0).
        note_panel_present(sid, 3, t0 + Duration::from_millis(5));
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(row.input_complete_n, 0);
        assert_eq!(row.input_start_n, 1);
        // Later bind + present (skipped-paint then store) completes.
        bind_input_to_mailbox_gen(sid, 9);
        note_panel_present(sid, 9, t0 + Duration::from_millis(40));
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(row.input_complete_n, 1);
        assert!(input_coverage_complete(row));
        l.decode_pending.clear();
    }

    #[test]
    fn stale_bridge_after_drop_does_not_pair_restart() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-bridge-restart");
        let sid = l.slot_id();
        let gen1 = l.generation();
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        note_script_dispatch_global(sid, t0 + Duration::from_millis(10));
        // Drop without draining: bridge event for gen1 must be discarded.
        drop(l);
        let mut l2 = fresh_local("resp-bridge-restart");
        assert_eq!(l2.slot_id(), sid);
        assert_ne!(l2.generation(), gen1);
        l2.note_decode_edge(t0 + Duration::from_millis(20));
        l2.drain_bridge();
        assert_eq!(
            l2.snap.dispatch_n, 0,
            "stale gen1 dispatch must not pair into gen2"
        );
        assert_eq!(l2.snap.decode_pending_n, 1);
        // Fresh dispatch for gen2 still pairs.
        note_script_dispatch_global(sid, t0 + Duration::from_millis(30));
        l2.drain_bridge();
        assert_eq!(l2.snap.dispatch_n, 1);
        assert_eq!(l2.snap.decode_pending_n, 0);
    }

    #[test]
    fn capture_and_read_brackets_share_mono_domain() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-clock-bracket");
        let t_before = mono_ns_now();
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        l.note_script_dispatch(t0 + Duration::from_millis(3));
        assert!(l.snap.decode_capture_mono_ns_upper >= l.snap.decode_capture_mono_ns_lower);
        assert!(l.snap.decode_capture_mono_ns_lower >= t_before);
        // Floor/ceil ms still enclose ns.
        let lo_ms = mono_ns_floor_ms(l.snap.decode_capture_mono_ns_lower);
        let hi_ms = mono_ns_ceil_ms(l.snap.decode_capture_mono_ns_upper);
        assert!(hi_ms >= lo_ms);
        assert!(lo_ms * 1_000_000 <= l.snap.decode_capture_mono_ns_lower);
        assert!(hi_ms * 1_000_000 >= l.snap.decode_capture_mono_ns_upper);

        std::thread::sleep(Duration::from_millis(5));
        let snap = read_bracketed().expect("on");
        assert!(snap.read_mono_ns_upper >= snap.read_mono_ns_lower);
        assert!(snap.read_mono_ns_lower >= l.snap.decode_capture_mono_ns_lower);
        let row = snap
            .slots
            .iter()
            .find(|s| s.slot_id == l.slot_id())
            .expect("row");
        assert!(row.decode_capture_mono_ns_upper > 0);
        assert!(row.decode_capture_mono_ns_upper <= snap.read_mono_ns_upper);
        // Input never mutated — input bracket stays unset.
        assert_eq!(row.input_capture_mono_ns_lower, 0);
        assert_eq!(row.input_capture_mono_ns_upper, 0);
        l.decode_pending.clear();
    }

    #[test]
    fn mono_ms_floor_ceil_encloses_ns() {
        let _g = lock_tests();
        assert_eq!(mono_ns_floor_ms(0), 0);
        assert_eq!(mono_ns_ceil_ms(0), 0);
        assert_eq!(mono_ns_floor_ms(1), 0);
        assert_eq!(mono_ns_ceil_ms(1), 1);
        assert_eq!(mono_ns_floor_ms(1_000_000), 1);
        assert_eq!(mono_ns_ceil_ms(1_000_000), 1);
        assert_eq!(mono_ns_floor_ms(1_000_001), 1);
        assert_eq!(mono_ns_ceil_ms(1_000_001), 2);
        // Both ends as plain floor would miss the upper enclosure:
        let ns = 1_999_999u64;
        assert!(mono_ns_floor_ms(ns) * 1_000_000 <= ns);
        assert!(mono_ns_ceil_ms(ns) * 1_000_000 >= ns);
        assert!(mono_ns_floor_ms(ns) < mono_ns_ceil_ms(ns));
    }

    #[test]
    fn input_mutation_does_not_retime_decode_capture_bracket() {
        let _g = lock_tests();
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        set_input_surface(InputSurface::Tui);
        let mut l = fresh_local("resp-separate-brackets");
        let sid = l.slot_id();
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        l.note_script_dispatch(t0 + Duration::from_millis(1));
        let decode_lo = l.snap.decode_capture_mono_ns_lower;
        let decode_hi = l.snap.decode_capture_mono_ns_upper;
        assert!(decode_hi > 0);
        std::thread::sleep(Duration::from_millis(3));
        assert!(note_input_start(sid, Instant::now(), 0));
        note_tui_draw_flush(sid, Instant::now() + Duration::from_millis(1));
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        // Decode bracket frozen at flush; input has its own later bracket.
        assert_eq!(row.decode_capture_mono_ns_lower, decode_lo);
        assert_eq!(row.decode_capture_mono_ns_upper, decode_hi);
        assert!(row.input_capture_mono_ns_upper > 0);
        assert!(
            row.input_capture_mono_ns_lower >= decode_hi
                || row.input_capture_mono_ns_lower > decode_lo
        );
        assert_eq!(row.input_complete_n, 1);
        assert_eq!(row.dispatch_n, 1);
        l.decode_pending.clear();
    }

    #[test]
    fn concurrent_input_and_local_flush_interleave_with_barrier() {
        let _g = lock_tests();
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        set_input_surface(InputSurface::Tui);
        let mut l = fresh_local("resp-clock-input-conc");
        let sid = l.slot_id();
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        // Barrier: input complete runs concurrent with decode dispatch flush.
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let b_in = barrier.clone();
        let handle = std::thread::spawn(move || {
            assert!(note_input_start(sid, Instant::now(), 0));
            b_in.wait();
            note_tui_draw_flush(sid, Instant::now() + Duration::from_millis(1));
        });
        barrier.wait();
        l.note_script_dispatch(t0 + Duration::from_millis(2));
        handle.join().expect("input thread");
        let rows = read().expect("on");
        let row = rows.iter().find(|s| s.slot_id == sid).expect("row");
        assert!(row.decode_capture_mono_ns_upper > 0);
        assert!(row.input_capture_mono_ns_upper > 0);
        assert_eq!(row.input_complete_n, 1);
        assert_eq!(row.dispatch_n, 1);
        // Separate families: either order is fine; both must be non-zero and valid.
        assert!(row.decode_capture_mono_ns_upper >= row.decode_capture_mono_ns_lower);
        assert!(row.input_capture_mono_ns_upper >= row.input_capture_mono_ns_lower);
        l.decode_pending.clear();
    }

    #[test]
    fn flush_expands_input_bracket_over_fresh_pending_cut() {
        // Operator note: old input bracket alone does not enclose overwritten
        // input_pending_n from a fresh INPUT_PENDING cut on flush. Flush must
        // union old input bracket with the pending-cut/merge bracket.
        let _g = lock_tests();
        enable();
        INPUT_PENDING.lock().unwrap().clear();
        set_input_surface(InputSurface::Tui);
        let mut l = fresh_local("resp-input-pending-union");
        let sid = l.slot_id();
        assert!(note_input_start(sid, Instant::now(), 0));
        let rows0 = read().expect("on");
        let r0 = rows0.iter().find(|s| s.slot_id == sid).expect("row");
        let after_first_hi = r0.input_capture_mono_ns_upper;
        assert!(after_first_hi > 0);
        assert_eq!(r0.input_pending_n, 1);
        assert_eq!(r0.input_start_n, 1);
        // Second start: input path replaces publication stamp; pending becomes 2.
        std::thread::sleep(Duration::from_millis(3));
        assert!(note_input_start(sid, Instant::now(), 0));
        let rows1 = read().expect("on");
        let r1 = rows1.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(r1.input_start_n, 2);
        assert_eq!(r1.input_pending_n, 2);
        let after_second_lo = r1.input_capture_mono_ns_lower;
        let after_second_hi = r1.input_capture_mono_ns_upper;
        assert!(after_second_hi > after_first_hi);
        // Decode flush refreshes pending from INPUT_PENDING and must expand
        // the input bracket past the input-only stamp (forced interleaving of
        // stale start counts + pending cut, not equality/zero-delta alone).
        std::thread::sleep(Duration::from_millis(3));
        l.note_decode_edge(Instant::now());
        let rows2 = read().expect("on");
        let r2 = rows2.iter().find(|s| s.slot_id == sid).expect("row");
        assert_eq!(r2.input_start_n, 2, "stale start count retained");
        assert_eq!(
            r2.input_pending_n, 2,
            "pending gauge still 2 after flush cut"
        );
        assert!(r2.input_capture_mono_ns_lower <= after_second_lo);
        assert!(
            r2.input_capture_mono_ns_upper > after_second_hi,
            "flush must expand input upper past input-only bracket; before={after_second_hi} after={}",
            r2.input_capture_mono_ns_upper
        );
        l.decode_pending.clear();
        INPUT_PENDING.lock().unwrap().clear();
    }

    #[test]
    fn generation_restart_ends_prior_row_with_capture_bracket() {
        let _g = lock_tests();
        let mut l = fresh_local("resp-clock-restart");
        let sid = l.slot_id();
        let gen1 = l.generation();
        l.note_decode_edge(Instant::now());
        drop(l);
        let rows = read().expect("on");
        let ended = rows
            .iter()
            .find(|s| s.slot_id == sid && s.generation == gen1)
            .expect("ended gen1");
        assert!(ended.ended);
        assert!(ended.decode_capture_mono_ns_upper >= ended.decode_capture_mono_ns_lower);
        assert!(ended.decode_capture_mono_ns_upper > 0);
        let mut l2 = fresh_local("resp-clock-restart");
        assert_ne!(l2.generation(), gen1);
        l2.note_decode_edge(Instant::now());
        l2.note_script_dispatch(Instant::now());
        assert!(l2.snap.decode_capture_mono_ns_upper > 0);
        l2.decode_pending.clear();
    }

    #[test]
    fn fine_bins_boundary_overflow_and_legacy_unchanged_when_fine_off() {
        let _g = lock_tests();
        // Fine off: legacy only.
        FINE_ENABLED.store(false, Relaxed);
        let mut l = fresh_local("resp-fine-off");
        let t0 = Instant::now();
        l.note_decode_edge(t0);
        l.note_script_dispatch(t0 + Duration::from_millis(3));
        assert_eq!(l.snap.decode_latency_buckets.iter().sum::<u64>(), 1);
        assert_eq!(l.snap.decode_fine_latency_buckets.iter().sum::<u64>(), 0);
        l.decode_pending.clear();

        enable_fine();
        assert!(fine_enabled());
        let mut l2 = fresh_local("resp-fine-on");
        let t1 = Instant::now();
        // Exact 1 ms → fine bucket 0 (≤1), legacy bucket 0 (≤5).
        l2.note_decode_edge(t1);
        l2.note_script_dispatch(t1 + Duration::from_millis(1));
        assert_eq!(l2.snap.decode_fine_latency_buckets[0], 1);
        assert_eq!(l2.snap.decode_latency_buckets[0], 1);
        // 100 ms exact → fine bucket 99 (≤100), legacy index of 100 ms bound.
        l2.note_decode_edge(t1);
        l2.note_script_dispatch(t1 + Duration::from_millis(100));
        assert_eq!(l2.snap.decode_fine_latency_buckets[99], 1);
        assert_eq!(l2.snap.decode_latency_buckets[6], 1);
        // 101 ms → fine overflow (100), legacy ≤250.
        l2.note_decode_edge(t1);
        l2.note_script_dispatch(t1 + Duration::from_millis(101));
        assert_eq!(l2.snap.decode_fine_latency_buckets[100], 1);
        assert_eq!(l2.snap.decode_latency_buckets[7], 1);
        assert_eq!(
            fine_p99_upper_bound_ms(&l2.snap.decode_fine_latency_buckets),
            None
        ); // overflow in p99 path when overflow holds the cum
        l2.decode_pending.clear();
        FINE_ENABLED.store(false, Relaxed);
    }

    #[test]
    fn fine_bucket_edges_and_p99() {
        let _g = lock_tests();
        assert_eq!(fine_latency_bucket(Duration::ZERO), 0);
        assert_eq!(fine_latency_bucket(Duration::from_millis(1)), 0);
        assert_eq!(fine_latency_bucket(Duration::from_micros(1_001)), 1);
        assert_eq!(fine_latency_bucket(Duration::from_millis(100)), 99);
        assert_eq!(fine_latency_bucket(Duration::from_millis(101)), 100);
        let mut b = [0u64; FINE_LATENCY_BUCKETS];
        b[4] = 100; // ≤5 ms
        assert_eq!(fine_p99_upper_bound_ms(&b), Some(5));
    }
}

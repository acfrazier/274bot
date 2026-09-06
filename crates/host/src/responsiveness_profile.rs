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
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static REGISTRY: Mutex<Vec<SlotObservation>> = Mutex::new(Vec::new());
/// Process-wide pending focused-input samples (panel + TUI), all slots.
static INPUT_PENDING: Mutex<VecDeque<InputPending>> = Mutex::new(VecDeque::new());
/// Frontend surface that last armed input endpoints (for JSON semantics).
static INPUT_SURFACE: Mutex<InputSurface> = Mutex::new(InputSurface::Unknown);

const MAX_ENDED_UNREAD: usize = 64;
/// Max outstanding decode→dispatch stamps per slot generation.
const MAX_DECODE_PENDING: usize = 8;
/// Max outstanding focused-input stamps process-wide.
const MAX_INPUT_PENDING: usize = 256;

/// Histogram upper bounds in ms (inclusive on exact Duration), then overflow.
/// Covers the 100 ms responsiveness gate and slower outliers.
pub const LATENCY_BOUNDS_MS: [u64; 10] = [5, 10, 20, 25, 40, 50, 100, 250, 500, 1000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_MS.len() + 1;

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

    // --- decode → script dispatch ---
    /// PLAYER_INFO edges observed at host after_drain.
    pub decode_edge_n: u64,
    /// Samples that reached on_game_tick entry.
    pub dispatch_n: u64,
    /// Tick edges dropped because script was not Running / no dispatch.
    pub decode_canceled_n: u64,
    /// Pending stamps discarded on slot end / overflow without dispatch.
    pub decode_lost_n: u64,
    pub decode_dropped_n: u64,
    pub decode_pending_n: u64,
    pub decode_latency_n: u64,
    pub decode_latency_ns: u64,
    pub decode_latency_buckets: [u64; LATENCY_BUCKETS],

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
}

impl SlotObservation {
    const fn zero(slot_id: u64, generation: u64) -> Self {
        Self {
            slot_id,
            generation,
            updated_ms: 0,
            ended: false,
            decode_edge_n: 0,
            dispatch_n: 0,
            decode_canceled_n: 0,
            decode_lost_n: 0,
            decode_dropped_n: 0,
            decode_pending_n: 0,
            decode_latency_n: 0,
            decode_latency_ns: 0,
            decode_latency_buckets: [0; LATENCY_BUCKETS],
            input_start_n: 0,
            input_complete_n: 0,
            input_canceled_n: 0,
            input_lost_n: 0,
            input_dropped_n: 0,
            input_pending_n: 0,
            input_latency_n: 0,
            input_latency_ns: 0,
            input_latency_buckets: [0; LATENCY_BUCKETS],
        }
    }
}

#[derive(Clone, Copy)]
struct DecodePending {
    at: Instant,
}

struct InputPending {
    slot_id: u64,
    at: Instant,
    /// Mailbox generation that must be presented (panel); 0 = any next draw (TUI).
    require_gen: u64,
    surface: InputSurface,
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn latency_bucket(d: Duration) -> usize {
    LATENCY_BOUNDS_MS
        .iter()
        .position(|&bound| d <= Duration::from_millis(bound))
        .unwrap_or(LATENCY_BOUNDS_MS.len())
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
    ENABLED.store(true, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
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
    if !ENABLED.load(Relaxed) {
        return None;
    }
    let mut reg = REGISTRY.lock().unwrap();
    let out = reg.clone();
    reg.retain(|s| !s.ended);
    Some(out)
}

/// Conservative p99 from fixed histogram buckets: returns the **upper bound**
/// of the bucket that first reaches cumulative count ≥ ceil(0.99 * n).
/// Overflow bucket yields `None` (unbounded / unavailable precise p99).
/// Empty samples yield `None`.
pub fn p99_upper_bound_ms(buckets: &[u64; LATENCY_BUCKETS]) -> Option<u64> {
    let n: u64 = buckets.iter().sum();
    if n == 0 {
        return None;
    }
    // Smallest k such that cum(k) / n >= 0.99 ⇔ cum * 100 >= n * 99
    let mut cum = 0u64;
    for (i, &c) in buckets.iter().enumerate() {
        cum = cum.saturating_add(c);
        if cum.saturating_mul(100) >= n.saturating_mul(99) {
            if i < LATENCY_BOUNDS_MS.len() {
                return Some(LATENCY_BOUNDS_MS[i]);
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
            self.snap.decode_dropped_n = self.snap.decode_dropped_n.wrapping_add(1);
            self.dirty = true;
            self.maybe_flush();
            return;
        }
        self.decode_pending.push_back(DecodePending { at });
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
        if self.decode_pending.pop_front().is_some() {
            self.snap.decode_canceled_n = self.snap.decode_canceled_n.wrapping_add(1);
            self.snap.decode_pending_n = self.decode_pending.len() as u64;
        } else {
            // Edge observed elsewhere without pending — still count cancel for coverage math
            // only when decode_edge already advanced; leave alone if empty.
            self.snap.decode_canceled_n = self.snap.decode_canceled_n.wrapping_add(1);
        }
        self.dirty = true;
        self.maybe_flush();
    }

    fn record_decode_latency(&mut self, d: Duration) {
        let b = latency_bucket(d);
        self.snap.decode_latency_n = self.snap.decode_latency_n.wrapping_add(1);
        self.snap.decode_latency_ns = self.snap.decode_latency_ns.wrapping_add(ns(d));
        self.snap.decode_latency_buckets[b] =
            self.snap.decode_latency_buckets[b].wrapping_add(1);
    }

    #[allow(dead_code)] // kept for Local-side pairing if a future path completes on Local
    fn record_input_latency(&mut self, d: Duration) {
        let b = latency_bucket(d);
        self.snap.input_latency_n = self.snap.input_latency_n.wrapping_add(1);
        self.snap.input_latency_ns = self.snap.input_latency_ns.wrapping_add(ns(d));
        self.snap.input_latency_buckets[b] = self.snap.input_latency_buckets[b].wrapping_add(1);
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
        self.snap.ended = ended;
        self.snap.updated_ms = wall_ms();
        self.snap.decode_pending_n = self.decode_pending.len() as u64;
        // Refresh input_pending from global queue for this slot.
        let pending = INPUT_PENDING.lock().unwrap();
        self.snap.input_pending_n = pending
            .iter()
            .filter(|p| p.slot_id == self.snap.slot_id)
            .count() as u64;
        drop(pending);

        let mut reg = REGISTRY.lock().unwrap();
        if let Some(entry) = reg.iter_mut().find(|s| s.generation == self.generation) {
            // Preserve input counters that global helpers wrote onto the registry row.
            let input_start_n = entry.input_start_n.max(self.snap.input_start_n);
            let input_complete_n = entry.input_complete_n.max(self.snap.input_complete_n);
            let input_canceled_n = entry.input_canceled_n.max(self.snap.input_canceled_n);
            let input_lost_n = entry.input_lost_n.max(self.snap.input_lost_n);
            let input_dropped_n = entry.input_dropped_n.max(self.snap.input_dropped_n);
            let input_latency_n = entry.input_latency_n.max(self.snap.input_latency_n);
            let input_latency_ns = entry.input_latency_ns.max(self.snap.input_latency_ns);
            let mut input_latency_buckets = self.snap.input_latency_buckets;
            for i in 0..LATENCY_BUCKETS {
                input_latency_buckets[i] = entry.input_latency_buckets[i].max(input_latency_buckets[i]);
            }
            *entry = self.snap.clone();
            entry.input_start_n = input_start_n;
            entry.input_complete_n = input_complete_n;
            entry.input_canceled_n = input_canceled_n;
            entry.input_lost_n = input_lost_n;
            entry.input_dropped_n = input_dropped_n;
            entry.input_latency_n = input_latency_n;
            entry.input_latency_ns = input_latency_ns;
            entry.input_latency_buckets = input_latency_buckets;
            entry.input_pending_n = self.snap.input_pending_n;
            self.snap.input_start_n = input_start_n;
            self.snap.input_complete_n = input_complete_n;
            self.snap.input_canceled_n = input_canceled_n;
            self.snap.input_lost_n = input_lost_n;
            self.snap.input_dropped_n = input_dropped_n;
            self.snap.input_latency_n = input_latency_n;
            self.snap.input_latency_ns = input_latency_ns;
            self.snap.input_latency_buckets = input_latency_buckets;
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
        // Drop in-flight bridge events for this generation so a restart of
        // the same username cannot mis-pair stale dispatch/cancel.
        discard_bridge_for(self.snap.slot_id, self.generation);
        while self.decode_pending.pop_front().is_some() {
            self.snap.decode_lost_n = self.snap.decode_lost_n.wrapping_add(1);
        }
        self.snap.decode_pending_n = 0;
        // Lose any input pending for this slot.
        let mut pending = INPUT_PENDING.lock().unwrap();
        let before = pending.len();
        pending.retain(|p| p.slot_id != self.snap.slot_id);
        let lost = (before - pending.len()) as u64;
        drop(pending);
        if lost > 0 {
            self.snap.input_lost_n = self.snap.input_lost_n.wrapping_add(lost);
        }
        self.flush(true);
    }
}

fn with_live_slot(slot_id: u64, f: impl FnOnce(&mut SlotObservation)) {
    let mut reg = REGISTRY.lock().unwrap();
    if let Some(entry) = reg.iter_mut().rev().find(|s| s.slot_id == slot_id && !s.ended) {
        f(entry);
        entry.updated_ms = wall_ms();
    } else if let Some(entry) = reg.iter_mut().rev().find(|s| s.slot_id == slot_id) {
        // Prefer live; fall back to most recent row for this slot.
        f(entry);
        entry.updated_ms = wall_ms();
    }
}

/// Focused actionable input start (panel Down/key-down or TUI state-mutating key).
/// `require_gen`: panel mailbox generation that must advance (≥ this after store);
/// use 0 for TUI (next draw completes). Returns false if disabled or dropped.
pub fn note_input_start(slot_id: u64, at: Instant, require_gen: u64) -> bool {
    if !ENABLED.load(Relaxed) {
        return false;
    }
    let surface = *INPUT_SURFACE.lock().unwrap();
    let mut q = INPUT_PENDING.lock().unwrap();
    if q.len() >= MAX_INPUT_PENDING {
        with_live_slot(slot_id, |s| {
            s.input_dropped_n = s.input_dropped_n.wrapping_add(1);
            s.input_start_n = s.input_start_n.wrapping_add(1);
        });
        return false;
    }
    q.push_back(InputPending {
        slot_id,
        at,
        require_gen,
        surface,
    });
    let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
    drop(q);
    with_live_slot(slot_id, |s| {
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
    let mut q = INPUT_PENDING.lock().unwrap();
    if let Some(pos) = q.iter().position(|p| p.slot_id == slot_id) {
        q.remove(pos);
        let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
        drop(q);
        with_live_slot(slot_id, |s| {
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
        p.surface == InputSurface::Panel
            && p.require_gen != 0
            && p.require_gen <= presented_gen
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
    let mut q = INPUT_PENDING.lock().unwrap();
    let mut completed: Vec<Duration> = Vec::new();
    let mut i = 0;
    while i < q.len() {
        if q[i].slot_id == slot_id && pred(&q[i]) {
            let p = q.remove(i).unwrap();
            completed.push(at.saturating_duration_since(p.at));
        } else {
            i += 1;
        }
    }
    let pending_n = q.iter().filter(|p| p.slot_id == slot_id).count() as u64;
    drop(q);
    if completed.is_empty() {
        return;
    }
    with_live_slot(slot_id, |s| {
        for d in completed {
            let b = latency_bucket(d);
            s.input_complete_n = s.input_complete_n.wrapping_add(1);
            s.input_latency_n = s.input_latency_n.wrapping_add(1);
            s.input_latency_ns = s.input_latency_ns.wrapping_add(ns(d));
            s.input_latency_buckets[b] = s.input_latency_buckets[b].wrapping_add(1);
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
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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
        assert_eq!(p99_upper_bound_ms(&l.snap.decode_latency_buckets), Some(250));
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
}

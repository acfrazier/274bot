//! Host service-wake lateness: one bounded per-slot ring, no per-tick
//! allocation. The slot loop records how late it ran past the park/frame
//! deadline it already chose; this module decides when the fleet is behind.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sliding window that counts late services.
pub const WINDOW: Duration = Duration::from_secs(10);
/// Warning stays up until this long of delays under [`CALM`].
pub const CLEAR_AFTER: Duration = Duration::from_secs(30);
/// A service this far past its chosen wake counts as late.
pub const LATE: Duration = Duration::from_millis(600);
/// Delays at or above this reset the calm timer.
pub const CALM: Duration = Duration::from_millis(300);
/// A bot needs this many late services in [`WINDOW`] to count.
pub const MIN_LATE_SERVICES: u32 = 3;
/// Floor on how many late bots trip the warning.
pub const MIN_LATE_BOTS: usize = 3;
/// Late bots as a percentage of active bots, with [`MIN_LATE_BOTS`] as floor.
pub const LATE_BOT_PERCENT: usize = 10;

const LATE_RING: usize = 8;
const LAST_DELAY_BOTS: usize = 128;

thread_local! {
    static MONITOR: RefCell<Option<Arc<CapacityMonitor>>> = const { RefCell::new(None) };
    static CLASS: Cell<ServiceClass> = const { Cell::new(ServiceClass::Active) };
}

/// Bind this slot thread to a play's capacity monitor.
pub fn bind_capacity(monitor: Arc<CapacityMonitor>) {
    MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
}

/// Drop the thread-local monitor (slot thread exit).
pub fn unbind_capacity() {
    MONITOR.with(|slot| *slot.borrow_mut() = None);
}

/// Class for the next [`record_service`] on this thread (observe sets it).
pub fn set_service_class(class: ServiceClass) {
    CLASS.with(|slot| slot.set(class));
}

/// Record one service on the bound monitor. No-op when none is bound.
pub fn record_service(name: &str, now: Instant, delay: Duration, saw_tick: bool) {
    MONITOR.with(|slot| {
        if let Some(monitor) = slot.borrow().as_ref() {
            monitor.record(name, now, delay, CLASS.with(|c| c.get()), saw_tick);
        }
    });
}

/// Whether this bot counts toward the fleet threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceClass {
    /// In-game, not latched, not paused: counted.
    Active,
    /// Title-parked or login-latched: ignored.
    Parked,
    /// Operator-paused script: ignored.
    Paused,
    /// Frozen / stopping: ignored.
    Suspended,
}

/// How the front end is drawing, for the warning line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderPolicyLabel {
    /// Panel default: only the focused bot rasters.
    #[default]
    OnlySelected,
    /// Every wall member rasters (render-all).
    RenderAll,
    /// TUI / headless: no game raster.
    Headless,
}

impl RenderPolicyLabel {
    /// End-user label in the warning line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnlySelected => "only selected",
            Self::RenderAll => "render-all",
            Self::Headless => "headless",
        }
    }
}

/// Snapshot shown while the host is behind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapacityWarning {
    /// Active bots with at least [`MIN_LATE_SERVICES`] late wakes in [`WINDOW`].
    pub late_bots: u16,
    /// Active bots (parked / paused / suspended excluded).
    pub active_bots: u16,
    /// `max(3, 10% of active)`.
    pub threshold: u16,
    /// p95 of each active bot's latest service delay.
    pub p95_delay: Duration,
    /// Median milliseconds per observed game tick across active bots.
    pub tick_ms: u32,
    /// What the front end is rendering.
    pub render: RenderPolicyLabel,
}

/// Result of one evaluate: fire, hide for server silence, or clear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CapacityEval {
    /// No warning.
    Clear,
    /// Active bots, but no game packets in [`WINDOW`]: do not blame the host.
    Silence,
    /// Show the warning.
    Warn(CapacityWarning),
}

/// Process-wide (per Play) lateness tracker.
pub struct CapacityMonitor {
    inner: Mutex<Inner>,
}

struct Inner {
    slots: HashMap<String, SlotTrack>,
    warning: Option<Instant>,
}

struct SlotTrack {
    class: ServiceClass,
    last_seen: Instant,
    last_unrest: Option<Instant>,
    last_packet: Option<Instant>,
    last_delay_ms: u16,
    lates: [Option<Instant>; LATE_RING],
    late_i: u8,
    late_n: u8,
    packet_n: u16,
    packet_origin: Instant,
}

impl SlotTrack {
    fn new(now: Instant) -> Self {
        Self {
            class: ServiceClass::Active,
            last_seen: now,
            last_unrest: None,
            last_packet: None,
            last_delay_ms: 0,
            lates: [None; LATE_RING],
            late_i: 0,
            late_n: 0,
            packet_n: 0,
            packet_origin: now,
        }
    }

    fn observe(&mut self, now: Instant, delay: Duration, class: ServiceClass, saw_tick: bool) {
        self.class = class;
        self.last_seen = now;
        let delay_ms = millis_u16(delay);
        self.last_delay_ms = delay_ms;
        if delay >= CALM {
            self.last_unrest = Some(now);
        }
        if delay >= LATE {
            self.lates[self.late_i as usize] = Some(now);
            self.late_i = (self.late_i + 1) % LATE_RING as u8;
            if (self.late_n as usize) < LATE_RING {
                self.late_n += 1;
            }
        }
        if saw_tick {
            self.last_packet = Some(now);
            if now.saturating_duration_since(self.packet_origin) >= WINDOW {
                self.packet_n = 1;
                self.packet_origin = now;
            } else {
                self.packet_n = self.packet_n.saturating_add(1);
            }
        }
    }

    fn late_in_window(&self, now: Instant) -> u32 {
        self.lates
            .iter()
            .take(self.late_n as usize)
            .flatten()
            .filter(|at| now.saturating_duration_since(**at) <= WINDOW)
            .count() as u32
    }

    fn counts(&self) -> bool {
        self.class == ServiceClass::Active
    }

    fn packet_recent(&self, now: Instant) -> bool {
        self.last_packet
            .is_some_and(|at| now.saturating_duration_since(at) <= WINDOW)
    }

    fn tick_ms(&self, now: Instant) -> Option<u32> {
        if self.packet_n == 0 {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.packet_origin);
        if elapsed.is_zero() {
            return None;
        }
        Some((elapsed.as_millis() as u32) / u32::from(self.packet_n))
    }
}

impl Default for CapacityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CapacityMonitor {
    /// Empty tracker.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slots: HashMap::new(),
                warning: None,
            }),
        }
    }

    /// One service. First call for a name allocates the slot row; later
    /// calls are a mutex acquire and integer stores.
    pub fn record(
        &self,
        name: &str,
        now: Instant,
        delay: Duration,
        class: ServiceClass,
        saw_tick: bool,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = inner.slots.get_mut(name) {
            slot.observe(now, delay, class, saw_tick);
            return;
        }
        let mut slot = SlotTrack::new(now);
        slot.observe(now, delay, class, saw_tick);
        inner.slots.insert(name.to_owned(), slot);
    }

    /// Slot thread exit: drop its ring.
    pub fn remove(&self, name: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .slots
            .remove(name);
    }

    /// Decide whether to show the warning. `render` is display-only.
    pub fn evaluate(&self, now: Instant, render: RenderPolicyLabel) -> CapacityEval {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.slots.retain(|_, slot| {
            now.saturating_duration_since(slot.last_seen) <= WINDOW + CLEAR_AFTER
        });

        let mut active = 0usize;
        let mut late_bots = 0usize;
        let mut packets = 0usize;
        let mut last_delays = [0u16; LAST_DELAY_BOTS];
        let mut n_delays = 0usize;
        let mut tick_ms = [0u32; LAST_DELAY_BOTS];
        let mut n_ticks = 0usize;
        let mut unrest: Option<Instant> = None;

        for slot in inner.slots.values() {
            if !slot.counts() {
                continue;
            }
            active += 1;
            if slot.late_in_window(now) >= MIN_LATE_SERVICES {
                late_bots += 1;
            }
            if slot.packet_recent(now) {
                packets += 1;
            }
            if n_delays < LAST_DELAY_BOTS {
                last_delays[n_delays] = slot.last_delay_ms;
                n_delays += 1;
            }
            if let Some(ms) = slot.tick_ms(now) {
                if n_ticks < LAST_DELAY_BOTS {
                    tick_ms[n_ticks] = ms;
                    n_ticks += 1;
                }
            }
            match (unrest, slot.last_unrest) {
                (None, Some(u)) => unrest = Some(u),
                (Some(a), Some(b)) if b > a => unrest = Some(b),
                _ => {}
            }
        }

        if active > 0 && packets == 0 {
            return CapacityEval::Silence;
        }

        let threshold = MIN_LATE_BOTS.max(active.div_ceil(LATE_BOT_PERCENT));
        let fire = active > 0 && late_bots >= threshold;
        let warning = match inner.warning {
            None if fire => {
                inner.warning = Some(now);
                true
            }
            Some(_) if fire => true,
            Some(_) => {
                let calm = unrest
                    .map(|at| now.saturating_duration_since(at) >= CLEAR_AFTER)
                    .unwrap_or(true);
                if calm {
                    inner.warning = None;
                    false
                } else {
                    true
                }
            }
            None => false,
        };
        if !warning {
            return CapacityEval::Clear;
        }

        last_delays[..n_delays].sort_unstable();
        let p95_ms = if n_delays == 0 {
            0
        } else {
            last_delays[(n_delays - 1) * 95 / 100]
        };
        tick_ms[..n_ticks].sort_unstable();
        let tick = if n_ticks == 0 {
            600
        } else {
            tick_ms[n_ticks / 2]
        };
        CapacityEval::Warn(CapacityWarning {
            late_bots: u16::try_from(late_bots).unwrap_or(u16::MAX),
            active_bots: u16::try_from(active).unwrap_or(u16::MAX),
            threshold: u16::try_from(threshold).unwrap_or(u16::MAX),
            p95_delay: Duration::from_millis(u64::from(p95_ms)),
            tick_ms: tick,
            render,
        })
    }
}

fn millis_u16(delay: Duration) -> u16 {
    u16::try_from(delay.as_millis().min(u128::from(u16::MAX))).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(
        mon: &CapacityMonitor,
        name: &str,
        at: Instant,
        delay_ms: u64,
        class: ServiceClass,
        tick: bool,
    ) {
        mon.record(name, at, Duration::from_millis(delay_ms), class, tick);
    }

    fn late_n(
        mon: &CapacityMonitor,
        name: &str,
        t0: Instant,
        n: u32,
        class: ServiceClass,
        tick: bool,
    ) {
        for i in 0..n {
            rec(
                mon,
                name,
                t0 + Duration::from_millis(u64::from(i) * 200),
                700,
                class,
                tick,
            );
        }
    }

    fn eval(mon: &CapacityMonitor, at: Instant) -> CapacityEval {
        mon.evaluate(at, RenderPolicyLabel::OnlySelected)
    }

    fn is_warn(eval: CapacityEval) -> bool {
        matches!(eval, CapacityEval::Warn(_))
    }

    #[test]
    fn three_bots_trip_the_floor_two_do_not() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        late_n(&mon, "a", t0, 3, ServiceClass::Active, true);
        late_n(&mon, "b", t0, 3, ServiceClass::Active, true);
        let t = t0 + Duration::from_millis(600);
        assert!(
            !is_warn(eval(&mon, t)),
            "2 late bots is below the floor of 3"
        );
        late_n(&mon, "c", t0, 3, ServiceClass::Active, true);
        assert!(is_warn(eval(&mon, t)), "3 late bots meet the floor");
    }

    #[test]
    fn ten_percent_beats_the_floor_on_a_large_fleet() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for i in 0..40 {
            rec(&mon, &format!("ok{i}"), t0, 50, ServiceClass::Active, true);
        }
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, true);
        }
        let t = t0 + Duration::from_millis(600);
        assert!(
            !is_warn(eval(&mon, t)),
            "3 of 43 is below 10% (threshold 5)"
        );
        late_n(&mon, "d", t0, 3, ServiceClass::Active, true);
        late_n(&mon, "e", t0, 3, ServiceClass::Active, true);
        assert!(is_warn(eval(&mon, t)), "5 of 45 meets 10%");
    }

    #[test]
    fn two_delays_do_not_count_a_bot_three_do() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 2, ServiceClass::Active, true);
        }
        let t = t0 + Duration::from_millis(500);
        assert!(
            !is_warn(eval(&mon, t)),
            "2 delays of 700 ms are below the 3-delay bar"
        );
        for name in ["a", "b", "c"] {
            rec(
                &mon,
                name,
                t0 + Duration::from_millis(400),
                700,
                ServiceClass::Active,
                true,
            );
        }
        assert!(is_warn(eval(&mon, t0 + Duration::from_millis(500))));
    }

    #[test]
    fn parked_paused_and_suspended_bots_are_excluded() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        late_n(&mon, "parked", t0, 3, ServiceClass::Parked, true);
        late_n(&mon, "paused", t0, 3, ServiceClass::Paused, true);
        late_n(&mon, "held", t0, 3, ServiceClass::Suspended, true);
        late_n(&mon, "a", t0, 3, ServiceClass::Active, true);
        late_n(&mon, "b", t0, 3, ServiceClass::Active, true);
        let t = t0 + Duration::from_millis(600);
        assert!(
            !is_warn(eval(&mon, t)),
            "only 2 active late bots; the others must not count"
        );
        late_n(&mon, "c", t0, 3, ServiceClass::Active, true);
        assert!(is_warn(eval(&mon, t)));
    }

    #[test]
    fn parked_bots_are_not_in_the_active_denominator() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for i in 0..30 {
            rec(
                &mon,
                &format!("live{i}"),
                t0,
                40,
                ServiceClass::Active,
                true,
            );
        }
        for i in 0..10 {
            late_n(&mon, &format!("park{i}"), t0, 3, ServiceClass::Parked, true);
        }
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, true);
        }
        let t = t0 + Duration::from_millis(600);
        // 33 active (30 calm + 3 late); 10% is 4, so 3 late must not fire.
        // If parked inflated the denominator to 43, 10% would be 5 — still no
        // fire — so assert the threshold on the warning once it does fire.
        late_n(&mon, "d", t0, 3, ServiceClass::Active, true);
        match eval(&mon, t) {
            CapacityEval::Warn(w) => {
                assert_eq!(w.active_bots, 34, "parked bots must not be active");
                assert_eq!(w.threshold, 4);
                assert_eq!(w.late_bots, 4);
            }
            other => panic!("expected warn, got {other:?}"),
        }
    }

    #[test]
    fn warning_clears_after_thirty_seconds_under_300ms() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, true);
        }
        let fired = t0 + Duration::from_millis(600);
        assert!(is_warn(eval(&mon, fired)));
        rec(
            &mon,
            "a",
            fired + Duration::from_secs(1),
            200,
            ServiceClass::Active,
            true,
        );
        rec(
            &mon,
            "b",
            fired + Duration::from_secs(1),
            200,
            ServiceClass::Active,
            true,
        );
        rec(
            &mon,
            "c",
            fired + Duration::from_secs(1),
            200,
            ServiceClass::Active,
            true,
        );
        for name in ["a", "b", "c"] {
            rec(
                &mon,
                name,
                fired + Duration::from_secs(28),
                200,
                ServiceClass::Active,
                true,
            );
        }
        assert!(
            is_warn(eval(&mon, fired + Duration::from_secs(29))),
            "29 s of calm is not enough"
        );
        assert_eq!(
            eval(&mon, fired + Duration::from_secs(31)),
            CapacityEval::Clear
        );
    }

    #[test]
    fn a_delay_of_400ms_extends_the_calm_timer() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, true);
        }
        let fired = t0 + Duration::from_millis(600);
        assert!(is_warn(eval(&mon, fired)));
        let unrest = fired + Duration::from_secs(5);
        rec(&mon, "a", unrest, 400, ServiceClass::Active, true);
        rec(&mon, "b", unrest, 200, ServiceClass::Active, true);
        rec(&mon, "c", unrest, 200, ServiceClass::Active, true);
        for name in ["a", "b", "c"] {
            rec(
                &mon,
                name,
                unrest + Duration::from_secs(28),
                200,
                ServiceClass::Active,
                true,
            );
        }
        assert!(
            is_warn(eval(&mon, unrest + Duration::from_secs(29))),
            "400 ms is not calm"
        );
        assert_eq!(
            eval(&mon, unrest + Duration::from_secs(31)),
            CapacityEval::Clear
        );
    }

    #[test]
    fn server_silence_does_not_blame_the_host() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, false);
        }
        assert_eq!(
            eval(&mon, t0 + Duration::from_millis(600)),
            CapacityEval::Silence
        );
    }

    #[test]
    fn warning_carries_counts_p95_and_render_policy() {
        let mon = CapacityMonitor::new();
        let t0 = Instant::now();
        for name in ["a", "b", "c"] {
            late_n(&mon, name, t0, 3, ServiceClass::Active, true);
        }
        rec(
            &mon,
            "a",
            t0 + Duration::from_millis(800),
            900,
            ServiceClass::Active,
            true,
        );
        match mon.evaluate(
            t0 + Duration::from_millis(800),
            RenderPolicyLabel::RenderAll,
        ) {
            CapacityEval::Warn(w) => {
                assert_eq!(w.late_bots, 3);
                assert_eq!(w.active_bots, 3);
                assert_eq!(w.threshold, 3);
                assert_eq!(w.render, RenderPolicyLabel::RenderAll);
                assert!(
                    w.p95_delay >= Duration::from_millis(700),
                    "{:?}",
                    w.p95_delay
                );
            }
            other => panic!("{other:?}"),
        }
    }
}

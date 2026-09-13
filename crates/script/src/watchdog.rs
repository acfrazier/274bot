//! Native progress watchdog: dual monotonic clocks, freeze, and fenced
//! recovery/restart. Load isolates only. Host `Instant` is the only clock;
//! JS never stamps policy.

use std::time::{Duration, Instant};

/// Warning-only hung-loop threshold (scheduler channel).
pub const SCHEDULER_WARN: Duration = Duration::from_secs(10);
/// Hard scheduler stall: recreate the isolate.
pub const HARD_STALL: Duration = Duration::from_secs(15 * 60);
/// Gameplay wedge: sample recoveryAnchor / walk / restart.
pub const WEDGE: Duration = Duration::from_secs(10 * 60);
/// Minimum gap between completed recovery attempts.
pub const RECOVERY_COOLDOWN: Duration = Duration::from_secs(15 * 60);
/// Chebyshev xz distance treated as "near anchor" (no walk).
pub const ANCHOR_NEAR: i32 = 8;
/// Native walk-near radius (one attempt, no teleports).
pub const WALK_RADIUS: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

impl Tile {
    pub fn xz(self) -> (i32, i32) {
        (self.x, self.z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartReason {
    Stall,
    Wedge,
    WedgeWalk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogState {
    Idle,
    Armed,
    SamplingAnchor,
    Recovering { anchor: Tile, started: Instant },
    RestartPending { reason: RestartReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    None,
    WarnHungLoop,
    RequestAnchor,
    ArmWalk { x: i32, z: i32, level: i32 },
    AbortWalk,
    Restart { reason: RestartReason },
}

#[derive(Debug)]
pub struct ProgressWatchdog {
    state: WatchdogState,
    last_scheduler: Option<Instant>,
    last_gameplay: Option<Instant>,
    last_recovery: Option<Instant>,
    last_tile: Option<Tile>,
    last_xp: Vec<i32>,
    wait_inflight: u32,
    warned: bool,
    frozen: bool,
}

impl Default for ProgressWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressWatchdog {
    pub fn new() -> Self {
        Self {
            state: WatchdogState::Idle,
            last_scheduler: None,
            last_gameplay: None,
            last_recovery: None,
            last_tile: None,
            last_xp: Vec::new(),
            wait_inflight: 0,
            warned: false,
            frozen: false,
        }
    }

    pub fn state(&self) -> WatchdogState {
        self.state
    }

    pub fn last_recovery(&self) -> Option<Instant> {
        self.last_recovery
    }

    pub fn recovering_anchor(&self) -> Option<Tile> {
        match self.state {
            WatchdogState::Recovering { anchor, .. } => Some(anchor),
            _ => None,
        }
    }

    pub fn wait_active(&self) -> bool {
        self.wait_inflight > 0
    }

    pub fn frozen(&self) -> bool {
        self.frozen
    }

    /// Fresh Load start: both clocks now, cooldown history cleared.
    pub fn arm_fresh(&mut self, now: Instant) {
        self.state = WatchdogState::Armed;
        self.last_scheduler = Some(now);
        self.last_gameplay = Some(now);
        self.last_recovery = None;
        self.last_tile = None;
        self.last_xp.clear();
        self.wait_inflight = 0;
        self.warned = false;
        self.frozen = false;
    }

    /// Isolate recreated after a completed watchdog restart: clocks now,
    /// cooldown history kept.
    pub fn arm_after_restart(&mut self, now: Instant) {
        self.state = WatchdogState::Armed;
        self.last_scheduler = Some(now);
        self.last_gameplay = Some(now);
        self.last_tile = None;
        self.last_xp.clear();
        self.wait_inflight = 0;
        self.warned = false;
        self.frozen = false;
    }

    /// Operator Stop / new manual Start / Drop: Idle and forget cooldown.
    pub fn cancel_clear(&mut self) {
        *self = Self::new();
    }

    /// Session reset / generation bump: drop in-flight recovery, restamp,
    /// keep cooldown history. Returns AbortWalk when a recovery walk was live.
    pub fn on_session_reset(&mut self, now: Instant) -> WatchdogAction {
        let abort = matches!(self.state, WatchdogState::Recovering { .. });
        self.state = match self.state {
            WatchdogState::Idle => WatchdogState::Idle,
            _ => WatchdogState::Armed,
        };
        self.wait_inflight = 0;
        self.warned = false;
        if self.state != WatchdogState::Idle {
            self.restamp(now);
        }
        if abort {
            WatchdogAction::AbortWalk
        } else {
            WatchdogAction::None
        }
    }

    pub fn set_frozen(&mut self, frozen: bool, now: Instant) -> WatchdogAction {
        if frozen == self.frozen {
            return WatchdogAction::None;
        }
        if frozen {
            self.frozen = true;
            let abort = matches!(self.state, WatchdogState::Recovering { .. });
            if matches!(
                self.state,
                WatchdogState::Recovering { .. } | WatchdogState::SamplingAnchor
            ) {
                self.state = WatchdogState::Armed;
            }
            if abort {
                WatchdogAction::AbortWalk
            } else {
                WatchdogAction::None
            }
        } else {
            self.frozen = false;
            self.warned = false;
            if self.state != WatchdogState::Idle {
                self.restamp(now);
            }
            WatchdogAction::None
        }
    }

    pub fn stamp_scheduler(&mut self, now: Instant) {
        if self.state == WatchdogState::Idle {
            return;
        }
        self.last_scheduler = Some(now);
        self.warned = false;
    }

    pub fn stamp_gameplay(&mut self, now: Instant) {
        if self.state == WatchdogState::Idle {
            return;
        }
        self.last_gameplay = Some(now);
    }

    pub fn stamp_note_progress(&mut self, now: Instant) {
        self.stamp_scheduler(now);
        self.stamp_gameplay(now);
    }

    pub fn on_wait_enqueued(&mut self, now: Instant) {
        self.wait_inflight = self.wait_inflight.saturating_add(1);
        self.stamp_scheduler(now);
    }

    pub fn on_wait_settled(&mut self, now: Instant) {
        self.wait_inflight = self.wait_inflight.saturating_sub(1);
        self.stamp_scheduler(now);
    }

    pub fn on_tile(&mut self, now: Instant, tile: Tile) -> WatchdogAction {
        if self.state == WatchdogState::Idle {
            self.last_tile = Some(tile);
            return WatchdogAction::None;
        }
        if self.last_tile != Some(tile) {
            self.last_tile = Some(tile);
            self.stamp_gameplay(now);
        }
        if let WatchdogState::Recovering { anchor, .. } = self.state {
            if chebyshev_xz(tile.xz(), anchor.xz()) <= WALK_RADIUS {
                return self.on_walk_arrived(now);
            }
        }
        WatchdogAction::None
    }

    pub fn on_xp(&mut self, now: Instant, xp: &[i32]) {
        if self.state == WatchdogState::Idle {
            self.last_xp = xp.to_vec();
            return;
        }
        if self.last_xp != xp {
            let increased = xp
                .iter()
                .enumerate()
                .any(|(i, v)| self.last_xp.get(i).is_none_or(|prev| *v > *prev));
            self.last_xp = xp.to_vec();
            if increased {
                self.stamp_gameplay(now);
            }
        }
    }

    pub fn on_anchor(
        &mut self,
        now: Instant,
        player: Option<(i32, i32)>,
        anchor: Option<Tile>,
    ) -> WatchdogAction {
        if self.state != WatchdogState::SamplingAnchor || self.frozen {
            return WatchdogAction::None;
        }
        let Some(anchor) = anchor else {
            return self.enter_restart(now, RestartReason::Wedge);
        };
        let far = player
            .map(|p| chebyshev_xz(p, anchor.xz()) > ANCHOR_NEAR)
            .unwrap_or(true);
        if far {
            self.state = WatchdogState::Recovering {
                anchor,
                started: now,
            };
            WatchdogAction::ArmWalk {
                x: anchor.x,
                z: anchor.z,
                level: anchor.level,
            }
        } else {
            self.enter_restart(now, RestartReason::Wedge)
        }
    }

    pub fn on_walk_arrived(&mut self, now: Instant) -> WatchdogAction {
        if !matches!(self.state, WatchdogState::Recovering { .. }) {
            return WatchdogAction::None;
        }
        self.last_recovery = Some(now);
        self.stamp_gameplay(now);
        self.state = WatchdogState::Armed;
        WatchdogAction::None
    }

    pub fn on_walk_failed(&mut self, now: Instant) -> WatchdogAction {
        if !matches!(self.state, WatchdogState::Recovering { .. }) {
            return WatchdogAction::None;
        }
        self.enter_restart(now, RestartReason::WedgeWalk)
    }

    /// Guardian hold (or pause) while a recovery walk is in flight: defer,
    /// do not consume cooldown.
    pub fn on_hold_during_walk(&mut self) -> WatchdogAction {
        if !matches!(self.state, WatchdogState::Recovering { .. }) {
            return WatchdogAction::None;
        }
        self.state = WatchdogState::Armed;
        WatchdogAction::AbortWalk
    }

    /// Restart was actually applied (isolate recreated). Consumes cooldown.
    pub fn on_restart_applied(&mut self, now: Instant) {
        self.last_recovery = Some(now);
        self.arm_after_restart(now);
    }

    pub fn observe(&mut self, now: Instant, running: bool) -> WatchdogAction {
        if self.state == WatchdogState::Idle || self.frozen || !running {
            return WatchdogAction::None;
        }
        if let WatchdogState::RestartPending { reason } = self.state {
            return WatchdogAction::Restart { reason };
        }
        if self.scheduler_elapsed(now) >= HARD_STALL {
            return self.enter_restart(now, RestartReason::Stall);
        }
        match self.state {
            WatchdogState::Armed => {
                if self.gameplay_elapsed(now) >= WEDGE && self.cooldown_ok(now) {
                    self.state = WatchdogState::SamplingAnchor;
                    return WatchdogAction::RequestAnchor;
                }
                if !self.wait_active()
                    && !self.warned
                    && self.scheduler_elapsed(now) >= SCHEDULER_WARN
                {
                    self.warned = true;
                    return WatchdogAction::WarnHungLoop;
                }
                WatchdogAction::None
            }
            WatchdogState::SamplingAnchor | WatchdogState::Recovering { .. } => {
                WatchdogAction::None
            }
            WatchdogState::Idle | WatchdogState::RestartPending { .. } => WatchdogAction::None,
        }
    }

    fn enter_restart(&mut self, _now: Instant, reason: RestartReason) -> WatchdogAction {
        self.state = WatchdogState::RestartPending { reason };
        WatchdogAction::Restart { reason }
    }

    fn restamp(&mut self, now: Instant) {
        self.last_scheduler = Some(now);
        self.last_gameplay = Some(now);
        self.warned = false;
    }

    fn scheduler_elapsed(&self, now: Instant) -> Duration {
        self.last_scheduler
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::ZERO)
    }

    fn gameplay_elapsed(&self, now: Instant) -> Duration {
        self.last_gameplay
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::ZERO)
    }

    fn cooldown_ok(&self, now: Instant) -> bool {
        match self.last_recovery {
            None => true,
            Some(t) => now.saturating_duration_since(t) >= RECOVERY_COOLDOWN,
        }
    }
}

pub fn chebyshev_xz(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn assert_no_recover(action: WatchdogAction) {
        assert!(
            !matches!(
                action,
                WatchdogAction::RequestAnchor
                    | WatchdogAction::ArmWalk { .. }
                    | WatchdogAction::Restart { .. }
            ),
            "unexpected recover/restart: {action:?}"
        );
    }

    #[test]
    fn tile_change_inside_wedge_never_wedges() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.on_tile(
            t + Duration::from_secs(60),
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
        );
        w.stamp_scheduler(t + Duration::from_secs(60));
        assert_no_recover(w.observe(t + WEDGE, true));
        assert_eq!(w.state(), WatchdogState::Armed);
    }

    #[test]
    fn xp_increase_inside_wedge_never_wedges() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.on_xp(t, &[10]);
        w.on_xp(t + Duration::from_secs(60), &[20]);
        w.stamp_scheduler(t + Duration::from_secs(60));
        assert_no_recover(w.observe(t + WEDGE, true));
        assert_eq!(w.state(), WatchdogState::Armed);
    }

    #[test]
    fn note_progress_holds_wedge_and_feeds_stall() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.stamp_note_progress(t + Duration::from_secs(60));
        assert_no_recover(w.observe(t + WEDGE, true));
        assert_eq!(
            w.observe(t + Duration::from_secs(60) + HARD_STALL, true),
            WatchdogAction::Restart {
                reason: RestartReason::Stall
            }
        );
    }

    #[test]
    fn first_wedge_has_no_cooldown() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert_eq!(w.observe(t + WEDGE, true), WatchdogAction::RequestAnchor);
        assert_eq!(w.state(), WatchdogState::SamplingAnchor);
    }

    #[test]
    fn second_wedge_inside_cooldown_does_not_recover() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.on_restart_applied(t + WEDGE);
        w.stamp_scheduler(t + WEDGE + Duration::from_secs(60));
        assert_no_recover(w.observe(t + WEDGE + Duration::from_secs(60), true));
        w.stamp_scheduler(t + WEDGE + RECOVERY_COOLDOWN);
        assert_eq!(
            w.observe(t + WEDGE + RECOVERY_COOLDOWN, true),
            WatchdogAction::RequestAnchor
        );
    }

    #[test]
    fn early_start_instant_still_first_fires_after_one_wedge() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert!(w.last_recovery().is_none());
        assert_eq!(w.observe(t + WEDGE, true), WatchdogAction::RequestAnchor);
    }

    #[test]
    fn pause_freeze_then_resume_does_not_stall_or_wedge() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert_eq!(
            w.set_frozen(true, t + Duration::from_secs(1)),
            WatchdogAction::None
        );
        assert_eq!(
            w.observe(t + Duration::from_secs(20 * 60), true),
            WatchdogAction::None
        );
        w.set_frozen(false, t + Duration::from_secs(20 * 60));
        assert_no_recover(w.observe(t + Duration::from_secs(21 * 60), true));
        assert!(!matches!(w.state(), WatchdogState::RestartPending { .. }));
        assert_ne!(w.state(), WatchdogState::SamplingAnchor);
    }

    #[test]
    fn hold_ready_and_session_freeze_like_pause() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.set_frozen(true, t);
        assert_eq!(w.observe(t + HARD_STALL, true), WatchdogAction::None);
        w.set_frozen(false, t + HARD_STALL);
        assert_no_recover(w.observe(t + HARD_STALL + Duration::from_secs(60), true));
        let abort = w.on_session_reset(t + HARD_STALL + Duration::from_secs(120));
        assert_eq!(abort, WatchdogAction::None);
        assert_no_recover(w.observe(t + HARD_STALL + Duration::from_secs(180), true));
    }

    #[test]
    fn fifteen_min_no_scheduler_restarts() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.stamp_gameplay(t + Duration::from_secs(30));
        assert_eq!(
            w.observe(t + HARD_STALL, true),
            WatchdogAction::Restart {
                reason: RestartReason::Stall
            }
        );
    }

    #[test]
    fn parked_wait_without_settle_still_hard_stalls() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.on_wait_enqueued(t + Duration::from_millis(1));
        assert!(w.wait_active());
        assert_eq!(
            w.observe(t + SCHEDULER_WARN + Duration::from_secs(1), true),
            WatchdogAction::None,
            "10s warning suppressed while a wait is active"
        );
        assert_eq!(
            w.observe(t + Duration::from_millis(1) + HARD_STALL, true),
            WatchdogAction::Restart {
                reason: RestartReason::Stall
            }
        );
    }

    #[test]
    fn ten_second_warning_is_once_and_does_not_restart() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.stamp_gameplay(t + Duration::from_secs(1));
        assert_eq!(
            w.observe(t + SCHEDULER_WARN, true),
            WatchdogAction::WarnHungLoop
        );
        assert_eq!(
            w.observe(t + SCHEDULER_WARN + Duration::from_secs(1), true),
            WatchdogAction::None
        );
        assert!(!matches!(w.state(), WatchdogState::RestartPending { .. }));
    }

    #[test]
    fn settle_and_repark_both_stamp_scheduler() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.on_wait_enqueued(t);
        w.on_wait_settled(t + Duration::from_secs(5));
        w.on_wait_enqueued(t + Duration::from_secs(5));
        assert!(w.wait_active());
        assert_eq!(
            w.observe(
                t + Duration::from_secs(5) + SCHEDULER_WARN - Duration::from_secs(1),
                true
            ),
            WatchdogAction::None
        );
    }

    #[test]
    fn idle_note_progress_is_noop() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.stamp_note_progress(t);
        assert_eq!(w.state(), WatchdogState::Idle);
        assert_eq!(w.observe(t + WEDGE, true), WatchdogAction::None);
    }

    #[test]
    fn operator_stop_cancels_restart_pending() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert!(matches!(
            w.observe(t + HARD_STALL, true),
            WatchdogAction::Restart { .. }
        ));
        w.cancel_clear();
        assert_eq!(w.state(), WatchdogState::Idle);
        assert!(w.last_recovery().is_none());
        assert_eq!(
            w.observe(t + HARD_STALL + WEDGE, true),
            WatchdogAction::None
        );
    }

    #[test]
    fn null_anchor_restarts_without_walk() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        assert_eq!(
            w.on_anchor(t + WEDGE, Some((100, 100)), None),
            WatchdogAction::Restart {
                reason: RestartReason::Wedge
            }
        );
    }

    #[test]
    fn near_anchor_restarts_without_walk() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        let anchor = Tile {
            x: 100,
            z: 100,
            level: 0,
        };
        assert_eq!(
            w.on_anchor(t + WEDGE, Some((105, 100)), Some(anchor)),
            WatchdogAction::Restart {
                reason: RestartReason::Wedge
            }
        );
    }

    #[test]
    fn far_anchor_arms_walk_and_arrival_consumes_cooldown() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        let anchor = Tile {
            x: 200,
            z: 200,
            level: 1,
        };
        assert_eq!(
            w.on_anchor(t + WEDGE, Some((100, 100)), Some(anchor)),
            WatchdogAction::ArmWalk {
                x: 200,
                z: 200,
                level: 1
            }
        );
        let arrived = w.on_tile(
            t + WEDGE + Duration::from_secs(30),
            Tile {
                x: 201,
                z: 200,
                level: 1,
            },
        );
        assert_eq!(arrived, WatchdogAction::None);
        assert_eq!(w.state(), WatchdogState::Armed);
        assert!(w.last_recovery().is_some());
        w.stamp_scheduler(t + WEDGE + Duration::from_secs(30));
        assert_no_recover(w.observe(t + WEDGE + Duration::from_secs(30) + WEDGE, true));
    }

    #[test]
    fn walk_failure_consumes_cooldown_and_restarts() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(
            t + WEDGE,
            Some((0, 0)),
            Some(Tile {
                x: 50,
                z: 50,
                level: 0,
            }),
        );
        assert_eq!(
            w.on_walk_failed(t + WEDGE + Duration::from_secs(5)),
            WatchdogAction::Restart {
                reason: RestartReason::WedgeWalk
            }
        );
        w.on_restart_applied(t + WEDGE + Duration::from_secs(5));
        w.stamp_scheduler(t + WEDGE + Duration::from_secs(5) + WEDGE);
        assert_no_recover(w.observe(t + WEDGE + Duration::from_secs(5) + WEDGE, true));
    }

    #[test]
    fn hold_mid_walk_defers_without_consuming_cooldown() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(
            t + WEDGE,
            Some((0, 0)),
            Some(Tile {
                x: 50,
                z: 50,
                level: 0,
            }),
        );
        assert_eq!(w.on_hold_during_walk(), WatchdogAction::AbortWalk);
        assert_eq!(w.state(), WatchdogState::Armed);
        assert!(w.last_recovery().is_none());
        w.set_frozen(true, t + WEDGE + Duration::from_secs(1));
        w.set_frozen(false, t + WEDGE + Duration::from_secs(2));
        assert_eq!(
            w.observe(t + WEDGE + Duration::from_secs(2) + WEDGE, true),
            WatchdogAction::RequestAnchor,
            "interrupted attempt does not consume cooldown"
        );
    }

    #[test]
    fn restart_history_survives_rearm_after_restart() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(t + WEDGE, None, None);
        w.on_restart_applied(t + WEDGE);
        let kept = w.last_recovery();
        assert!(kept.is_some());
        w.arm_after_restart(t + WEDGE + Duration::from_secs(1));
        assert_eq!(w.last_recovery(), kept);
        assert_eq!(w.state(), WatchdogState::Armed);
    }

    #[test]
    fn stale_anchor_reply_ignored_when_not_sampling() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert_eq!(
            w.on_anchor(
                t,
                Some((0, 0)),
                Some(Tile {
                    x: 9,
                    z: 9,
                    level: 0
                })
            ),
            WatchdogAction::None
        );
    }

    #[test]
    fn freeze_while_recovering_aborts_walk() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(
            t + WEDGE,
            Some((0, 0)),
            Some(Tile {
                x: 50,
                z: 50,
                level: 0,
            }),
        );
        assert_eq!(
            w.set_frozen(true, t + WEDGE + Duration::from_secs(1)),
            WatchdogAction::AbortWalk
        );
        assert_eq!(w.state(), WatchdogState::Armed);
        assert!(w.last_recovery().is_none());
    }

    #[test]
    fn not_running_does_not_arm_stall_or_wedge() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert_eq!(w.observe(t + HARD_STALL, false), WatchdogAction::None);
        assert_eq!(w.state(), WatchdogState::Armed);
    }
}

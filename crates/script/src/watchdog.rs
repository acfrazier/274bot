//! Progress watchdog shared by Load isolates and compiled cards: dual
//! monotonic clocks, freeze, and fenced plane-aware recovery/restart. Host
//! `Instant` is the only clock; scripts never stamp policy.

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

/// Skill slots in the client's stat table.
pub const SKILL_SLOTS: usize = 25;

/// What the Fleet window shows about one run: how long it has run, how long
/// since gameplay last progressed, and the levels gained since Start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptProgress {
    pub running_for: Duration,
    /// Time since the watchdog's gameplay clock last moved (tile change, XP
    /// gain, script progress note). `None` while the clock is frozen (Pause,
    /// not in game) and while no run is armed.
    pub idle_for: Option<Duration>,
    /// Levels gained per skill slot since the run's first in-game
    /// observation.
    pub gained: [u8; SKILL_SLOTS],
}

impl ScriptProgress {
    /// Total levels gained across every skill.
    pub fn levels_gained(&self) -> u32 {
        self.gained.iter().map(|g| u32::from(*g)).sum()
    }
}

/// The per-run copy of the levels taken at Start. Allocated by a fresh
/// Start only (a stopped or never-started slot holds none) and kept across
/// relogs and watchdog restarts within the run.
#[derive(Debug)]
struct RunProgress {
    started: Instant,
    baseline: [u8; SKILL_SLOTS],
    latest: [u8; SKILL_SLOTS],
    /// The baseline is copied from the first in-game observation, not from
    /// the pre-login all-zero table.
    seen: bool,
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
    /// The recovery an operator Pause interrupted: re-armed on Resume
    /// instead of waiting out a fresh [`WEDGE`] ([`Self::defer_recovery`]).
    deferred: Option<Tile>,
    /// The re-armed recovery's walk is owed on the next [`Self::observe`].
    rearm_walk: bool,
    /// Present while a run is armed; see [`RunProgress`].
    progress: Option<Box<RunProgress>>,
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
            deferred: None,
            rearm_walk: false,
            progress: None,
        }
    }

    /// Record the current base levels (one per skill slot, in slot order).
    /// The first call of a run fixes its baseline. Call only from a
    /// game-ready observation.
    pub fn note_levels(&mut self, levels: impl Iterator<Item = i32>) {
        let Some(progress) = self.progress.as_deref_mut() else {
            return;
        };
        for (slot, level) in progress.latest.iter_mut().zip(levels) {
            *slot = level.clamp(0, i32::from(u8::MAX)) as u8;
        }
        if !progress.seen {
            progress.baseline = progress.latest;
            progress.seen = true;
        }
    }

    /// The Fleet window's view of the armed run, or `None` when none is
    /// armed. Reads clocks only; it never stamps them.
    pub fn progress(&self, now: Instant) -> Option<ScriptProgress> {
        let run = self.progress.as_deref()?;
        let mut gained = [0u8; SKILL_SLOTS];
        if run.seen {
            for (slot, (latest, base)) in
                gained.iter_mut().zip(run.latest.iter().zip(&run.baseline))
            {
                *slot = latest.saturating_sub(*base);
            }
        }
        Some(ScriptProgress {
            running_for: now.saturating_duration_since(run.started),
            idle_for: (self.state != WatchdogState::Idle && !self.frozen)
                .then(|| self.gameplay_elapsed(now)),
            gained,
        })
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

    /// Host-owned recovery is live: suspend ordinary script action
    /// dispatch/continuation. Distinct from guardian hold.
    pub fn holds_script_actions(&self) -> bool {
        matches!(
            self.state,
            WatchdogState::SamplingAnchor
                | WatchdogState::Recovering { .. }
                | WatchdogState::RestartPending { .. }
        )
    }

    /// Drop in-flight recovery without consuming cooldown. AbortWalk when a
    /// recovery walk was live, even if clocks are already frozen.
    pub fn abort_owned_recovery(&mut self) -> WatchdogAction {
        self.rearm_walk = false;
        match self.state {
            WatchdogState::Recovering { .. } => {
                self.state = WatchdogState::Armed;
                WatchdogAction::AbortWalk
            }
            WatchdogState::SamplingAnchor | WatchdogState::RestartPending { .. } => {
                self.state = WatchdogState::Armed;
                WatchdogAction::None
            }
            _ => WatchdogAction::None,
        }
    }

    /// A recovery Resume re-entered still owes its walk ([`Self::observe`]
    /// returns it): the route is idle because it has not been re-armed yet,
    /// not because it failed.
    pub fn rearm_pending(&self) -> bool {
        self.rearm_walk
    }

    /// Operator Pause: the recovery walk stops with the script (the host
    /// ends its route) but is not abandoned. Resume re-enters it with a
    /// fresh walk to the same anchor. Without a live recovery this is
    /// [`Self::abort_owned_recovery`].
    pub fn defer_recovery(&mut self) -> WatchdogAction {
        if let WatchdogState::Recovering { anchor, .. } = self.state {
            self.deferred = Some(anchor);
        }
        self.abort_owned_recovery()
    }

    pub fn wait_active(&self) -> bool {
        self.wait_inflight > 0
    }

    pub fn frozen(&self) -> bool {
        self.frozen
    }

    /// Fresh Load start: both clocks now, cooldown history cleared.
    pub fn arm_fresh(&mut self, now: Instant) {
        self.deferred = None;
        self.rearm_walk = false;
        self.state = WatchdogState::Armed;
        self.last_scheduler = Some(now);
        self.last_gameplay = Some(now);
        self.progress = Some(Box::new(RunProgress {
            started: now,
            baseline: [0; SKILL_SLOTS],
            latest: [0; SKILL_SLOTS],
            seen: false,
        }));
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
        self.deferred = None;
        self.rearm_walk = false;
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
        self.deferred = None;
        self.rearm_walk = false;
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
            if let Some(anchor) = self.deferred.take() {
                if self.state == WatchdogState::Armed {
                    self.state = WatchdogState::Recovering {
                        anchor,
                        started: now,
                    };
                    self.rearm_walk = true;
                }
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
            if tile.level == anchor.level && chebyshev_xz(tile.xz(), anchor.xz()) <= WALK_RADIUS {
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
        player: Option<Tile>,
        anchor: Option<Tile>,
    ) -> WatchdogAction {
        if self.state != WatchdogState::SamplingAnchor || self.frozen {
            return WatchdogAction::None;
        }
        let Some(anchor) = anchor else {
            return self.enter_restart(now, RestartReason::Wedge);
        };
        let far = player
            .map(|p| p.level != anchor.level || chebyshev_xz(p.xz(), anchor.xz()) > ANCHOR_NEAR)
            .unwrap_or(true);
        if far {
            self.rearm_walk = false;
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
        self.rearm_walk = false;
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
        self.rearm_walk = false;
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
        if let WatchdogState::Recovering { anchor, .. } = self.state {
            if std::mem::take(&mut self.rearm_walk) {
                return WatchdogAction::ArmWalk {
                    x: anchor.x,
                    z: anchor.z,
                    level: anchor.level,
                };
            }
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
        self.rearm_walk = false;
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
            w.on_anchor(
                t + WEDGE,
                Some(Tile {
                    x: 100,
                    z: 100,
                    level: 0
                }),
                None
            ),
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
            w.on_anchor(
                t + WEDGE,
                Some(Tile {
                    x: 105,
                    z: 100,
                    level: 0
                }),
                Some(anchor)
            ),
            WatchdogAction::Restart {
                reason: RestartReason::Wedge
            }
        );
    }

    #[test]
    fn anchor_directly_above_player_requires_recovery_walk() {
        let mut watchdog = ProgressWatchdog::new();
        let now = t0();
        watchdog.arm_fresh(now);
        watchdog.observe(now + WEDGE, true);
        let anchor = Tile {
            x: 100,
            z: 100,
            level: 1,
        };
        assert_eq!(
            watchdog.on_anchor(now + WEDGE, Some(Tile { level: 0, ..anchor }), Some(anchor),),
            WatchdogAction::ArmWalk {
                x: 100,
                z: 100,
                level: 1
            },
        );
        assert!(watchdog.last_recovery().is_none());
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
            w.on_anchor(
                t + WEDGE,
                Some(Tile {
                    x: 100,
                    z: 100,
                    level: 0
                }),
                Some(anchor)
            ),
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
    fn recovery_on_another_plane_does_not_arrive_or_consume_cooldown() {
        let mut watchdog = ProgressWatchdog::new();
        let now = t0();
        watchdog.arm_fresh(now);
        watchdog.observe(now + WEDGE, true);
        let anchor = Tile {
            x: 200,
            z: 200,
            level: 1,
        };
        watchdog.on_anchor(
            now + WEDGE,
            Some(Tile {
                x: 100,
                z: 100,
                level: 0,
            }),
            Some(anchor),
        );
        watchdog.on_tile(
            now + WEDGE + Duration::from_secs(1),
            Tile { level: 0, ..anchor },
        );
        assert_eq!(watchdog.recovering_anchor(), Some(anchor));
        assert!(watchdog.last_recovery().is_none());
        watchdog.on_tile(now + WEDGE + Duration::from_secs(2), anchor);
        assert_eq!(watchdog.state(), WatchdogState::Armed);
        assert!(watchdog.last_recovery().is_some());
    }

    #[test]
    fn walk_failure_consumes_cooldown_and_restarts() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(
            t + WEDGE,
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
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
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
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
                Some(Tile {
                    x: 0,
                    z: 0,
                    level: 0
                }),
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
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
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

    /// An operator Pause interrupts a recovery walk; Resume walks to the
    /// same anchor again at once instead of waiting out another WEDGE.
    #[test]
    fn a_paused_recovery_walk_is_re_armed_on_resume() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        let anchor = Tile {
            x: 50,
            z: 50,
            level: 0,
        };
        w.on_anchor(
            t + WEDGE,
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
            Some(anchor),
        );
        let paused = t + WEDGE + Duration::from_secs(1);
        assert_eq!(w.defer_recovery(), WatchdogAction::AbortWalk);
        assert_eq!(w.set_frozen(true, paused), WatchdogAction::None);
        let resumed = paused + Duration::from_secs(60);
        w.set_frozen(false, resumed);
        assert_eq!(
            w.observe(resumed, true),
            WatchdogAction::ArmWalk {
                x: 50,
                z: 50,
                level: 0
            }
        );
        assert_eq!(w.recovering_anchor(), Some(anchor));
        assert_eq!(w.observe(resumed, true), WatchdogAction::None, "once");
        assert!(w.last_recovery().is_none(), "no cooldown consumed");

        // A session reset while paused drops the deferred recovery.
        let mut w = ProgressWatchdog::new();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        w.on_anchor(
            t + WEDGE,
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
            Some(anchor),
        );
        w.defer_recovery();
        w.set_frozen(true, paused);
        w.on_session_reset(paused);
        w.set_frozen(false, resumed);
        assert_eq!(w.observe(resumed, true), WatchdogAction::None);
        assert_eq!(w.state(), WatchdogState::Armed);
    }

    /// A guardian hold that aborts a resumed recovery before its owed walk
    /// was sent drops that walk: the next recovery arms exactly once.
    #[test]
    fn an_abandoned_resumed_recovery_owes_no_walk_to_the_next_one() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        w.observe(t + WEDGE, true);
        let anchor = Tile {
            x: 50,
            z: 50,
            level: 0,
        };
        w.on_anchor(
            t + WEDGE,
            Some(Tile {
                x: 0,
                z: 0,
                level: 0,
            }),
            Some(anchor),
        );
        let paused = t + WEDGE + Duration::from_secs(1);
        w.defer_recovery();
        w.set_frozen(true, paused);
        let resumed = paused + Duration::from_secs(60);
        w.set_frozen(false, resumed);
        assert!(w.rearm_pending());
        assert_eq!(w.on_hold_during_walk(), WatchdogAction::AbortWalk);
        assert!(!w.rearm_pending());

        let wedged = resumed + WEDGE;
        assert_eq!(w.observe(wedged, true), WatchdogAction::RequestAnchor);
        assert_eq!(
            w.on_anchor(
                wedged,
                Some(Tile {
                    x: 0,
                    z: 0,
                    level: 0
                }),
                Some(anchor)
            ),
            WatchdogAction::ArmWalk {
                x: 50,
                z: 50,
                level: 0
            }
        );
        assert_eq!(w.observe(wedged, true), WatchdogAction::None, "once");
    }

    #[test]
    fn abort_owned_recovery_clears_recovering_and_pending() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert!(!w.holds_script_actions());
        assert_eq!(w.observe(t + WEDGE, true), WatchdogAction::RequestAnchor);
        assert!(w.holds_script_actions());
        assert_eq!(w.abort_owned_recovery(), WatchdogAction::None);
        assert_eq!(w.state(), WatchdogState::Armed);
        assert!(!w.holds_script_actions());
        w.observe(t + WEDGE, true);
        assert_eq!(
            w.on_anchor(
                t + WEDGE,
                Some(Tile {
                    x: 0,
                    z: 0,
                    level: 0
                }),
                Some(Tile {
                    x: 50,
                    z: 50,
                    level: 0,
                }),
            ),
            WatchdogAction::ArmWalk {
                x: 50,
                z: 50,
                level: 0,
            }
        );
        assert!(w.holds_script_actions());
        assert_eq!(w.abort_owned_recovery(), WatchdogAction::AbortWalk);
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

    fn levels(values: &[i32]) -> impl Iterator<Item = i32> + '_ {
        values.iter().copied()
    }

    /// Levels gained count from the first game-ready observation of the run,
    /// survive a watchdog restart, and start over on a new Start.
    #[test]
    fn levels_gained_count_from_the_first_ready_observation_of_the_run() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        assert!(w.progress(t).is_none(), "no run armed");
        w.note_levels(levels(&[50]));
        assert!(w.progress(t).is_none(), "a note without a run is dropped");

        w.arm_fresh(t);
        assert_eq!(w.progress(t).unwrap().levels_gained(), 0);
        w.note_levels(levels(&[10, 20, 30]));
        w.note_levels(levels(&[11, 20, 32]));
        let progress = w.progress(t + Duration::from_secs(90)).unwrap();
        assert_eq!(progress.running_for, Duration::from_secs(90));
        assert_eq!(&progress.gained[..4], &[1, 0, 2, 0]);
        assert_eq!(progress.levels_gained(), 3);

        w.arm_after_restart(t + Duration::from_secs(100));
        w.note_levels(levels(&[11, 21, 32]));
        assert_eq!(
            w.progress(t + Duration::from_secs(120))
                .unwrap()
                .levels_gained(),
            4,
            "a watchdog restart keeps the run's baseline"
        );

        w.arm_fresh(t + Duration::from_secs(200));
        w.note_levels(levels(&[11, 21, 32]));
        assert_eq!(
            w.progress(t + Duration::from_secs(201))
                .unwrap()
                .levels_gained(),
            0,
            "a new Start takes a new baseline"
        );
        w.cancel_clear();
        assert!(w.progress(t + Duration::from_secs(300)).is_none());
    }

    /// Time since progress follows the gameplay clock, resets on progress
    /// and is absent while the clock is frozen.
    #[test]
    fn idle_time_follows_the_gameplay_clock_and_hides_while_frozen() {
        let mut w = ProgressWatchdog::new();
        let t = t0();
        w.arm_fresh(t);
        assert_eq!(
            w.progress(t + Duration::from_secs(30)).unwrap().idle_for,
            Some(Duration::from_secs(30))
        );
        w.stamp_note_progress(t + Duration::from_secs(40));
        assert_eq!(
            w.progress(t + Duration::from_secs(45)).unwrap().idle_for,
            Some(Duration::from_secs(5))
        );
        w.set_frozen(true, t + Duration::from_secs(50));
        assert_eq!(
            w.progress(t + Duration::from_secs(60)).unwrap().idle_for,
            None
        );
        w.set_frozen(false, t + Duration::from_secs(70));
        assert_eq!(
            w.progress(t + Duration::from_secs(75)).unwrap().idle_for,
            Some(Duration::from_secs(5)),
            "Resume restarts the clock"
        );
    }
}

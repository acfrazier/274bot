//! Shared host login-readiness: settle the native welcome modal before
//! ordinary script work.
//!
//! Identity is the client's public `welcome_interface_id` (LAST_LOGIN_INFO
//! client codes 650/655) matching the current main modal. The layer id is
//! never guessed (5993 is one observed layer, not a catalog constant).
//! Dismissal uses the existing `Interactions::close_modal` path; unrelated
//! bank/trade/quest dialogs are left alone. Scripts stay held until the
//! closed state is observed. Operator Pause/Stop/Logout are not resumed
//! here.

use api::interact::{Driver, Interactions, SendReason, SendResult};
use api::snapshot::GameSnapshot;
use std::time::{Duration, Instant};

/// Native welcome is showing when the LAST_LOGIN_INFO-selected interface
/// is the current main modal.
pub fn welcome_is_open(welcome_interface_id: i32, main_modal_id: i32) -> bool {
    welcome_interface_id != -1 && main_modal_id == welcome_interface_id
}

/// Bound the dismiss loop: a few spaced attempts, then a visible stop.
const MAX_CLOSE_ATTEMPTS: u8 = 3;
const RETRY_TICKS: u64 = 2;
const WELCOME_DISMISS_TIMEOUT: Duration = Duration::from_secs(10);

/// Facts the slot pump already has; no extra snapshot/FlatBuffer field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WelcomeObservation {
    /// Bumped on logout/reconnect so a pending close cannot cross sessions.
    pub session_epoch: u64,
    pub tick: u64,
    /// Monotonic clock sampled once by the host observe frame.
    pub now: Instant,
    pub ingame: bool,
    pub scene_state: i32,
    /// `Client::welcome_interface_id`, not a hardcoded layer.
    pub welcome_interface_id: i32,
    pub main_modal_id: i32,
    /// Close only after `ingame && scene_state == 2` and not on the
    /// session-boundary frame (stale snapshot / producer gate closed).
    pub allow_close: bool,
}

impl WelcomeObservation {
    pub fn welcome_open(self) -> bool {
        welcome_is_open(self.welcome_interface_id, self.main_modal_id)
    }
}

/// Result of one attempted close (the caller owns `Interactions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseAttempt {
    Sent,
    Refused(SendReason),
}

/// Existing supported close: `CLOSE_BUTTON` → `Client::close_modal`.
pub fn try_close_welcome(snapshot: &GameSnapshot, driver: &mut dyn Driver) -> CloseAttempt {
    match Interactions::new(snapshot, driver).close_modal() {
        SendResult::Sent { .. } => CloseAttempt::Sent,
        SendResult::Refused { reason, .. } => CloseAttempt::Refused(reason),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeAction {
    None,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeStep {
    /// Freeze ordinary script ticks / nav follow until ack or a bounded fail.
    pub hold: bool,
    pub action: WelcomeAction,
    pub failure: Option<String>,
    /// Operator-visible phase line (panel/TUI log), if it changed.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Open,
    Dismissing,
    Settled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct LoginReadiness {
    session_epoch: u64,
    phase: Phase,
    close_attempts: u8,
    last_close_tick: Option<u64>,
    close_epoch: Option<u64>,
    episode_started: Option<Instant>,
    failure: Option<String>,
    last_notice: Option<String>,
}

impl Default for LoginReadiness {
    fn default() -> Self {
        Self {
            session_epoch: 0,
            phase: Phase::Idle,
            close_attempts: 0,
            last_close_tick: None,
            close_epoch: None,
            episode_started: None,
            failure: None,
            last_notice: None,
        }
    }
}

impl LoginReadiness {
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    pub fn hold(&self) -> bool {
        matches!(self.phase, Phase::Open | Phase::Dismissing | Phase::Failed)
    }

    /// Drop any in-flight close. The next [`Self::step`] sees a new epoch.
    pub fn on_session_boundary(&mut self) {
        let epoch = self.session_epoch.wrapping_add(1);
        *self = Self {
            session_epoch: epoch,
            ..Self::default()
        };
        self.session_epoch = epoch;
    }

    pub fn step(
        &mut self,
        obs: &WelcomeObservation,
        mut attempt: impl FnMut() -> CloseAttempt,
    ) -> WelcomeStep {
        if obs.session_epoch != self.session_epoch {
            let epoch = obs.session_epoch;
            *self = Self {
                session_epoch: epoch,
                ..Self::default()
            };
        }
        if !obs.ingame {
            *self = Self {
                session_epoch: obs.session_epoch,
                ..Self::default()
            };
            return self.emit(false, WelcomeAction::None, None);
        }

        let open = obs.welcome_open();
        if !open {
            if matches!(
                self.phase,
                Phase::Open | Phase::Dismissing | Phase::Failed | Phase::Settled
            ) {
                self.phase = Phase::Settled;
                self.close_attempts = 0;
                self.last_close_tick = None;
                self.close_epoch = None;
                self.episode_started = None;
                self.failure = None;
                return self.emit(
                    false,
                    WelcomeAction::None,
                    Some("welcome: settled".to_string()),
                );
            }
            return self.emit(false, WelcomeAction::None, None);
        }

        // Welcome is the live main modal. Hold script work; never close an
        // unrelated dialog that happens to share a numeric layer.
        if self.phase == Phase::Failed {
            return self.emit(true, WelcomeAction::None, None);
        }
        if self.phase == Phase::Idle || self.phase == Phase::Settled {
            self.phase = Phase::Open;
            self.close_attempts = 0;
            self.last_close_tick = None;
            self.close_epoch = None;
            self.episode_started = None;
            self.failure = None;
        }

        if !obs.allow_close {
            // Scene/session rebuilding is outside the elapsed close window.
            // Keep the attempt count so eligibility flapping cannot bypass
            // the independent MAX_CLOSE_ATTEMPTS bound.
            self.episode_started = None;
            return self.emit(
                true,
                WelcomeAction::None,
                Some(format!(
                    "welcome: waiting to dismiss interface {}",
                    obs.welcome_interface_id
                )),
            );
        }

        let episode_started = self.episode_started.get_or_insert(obs.now);
        if obs.now.saturating_duration_since(*episode_started) >= WELCOME_DISMISS_TIMEOUT {
            return self.fail_with(format!(
                "welcome: dismissal timed out after {}s for interface {}; close it manually or reconnect",
                WELCOME_DISMISS_TIMEOUT.as_secs(),
                obs.welcome_interface_id,
            ));
        }

        let can_try = match self.last_close_tick {
            None => true,
            Some(tick) if obs.tick.wrapping_sub(tick) >= RETRY_TICKS => true,
            Some(_) => false,
        };
        if !can_try {
            return self.emit(true, WelcomeAction::None, None);
        }
        if self.close_attempts >= MAX_CLOSE_ATTEMPTS {
            return self.fail(obs.welcome_interface_id);
        }

        // A close queued on a prior epoch is invalid.
        if self
            .close_epoch
            .is_some_and(|epoch| epoch != obs.session_epoch)
        {
            self.close_attempts = 0;
            self.last_close_tick = None;
            self.close_epoch = None;
        }

        match attempt() {
            CloseAttempt::Sent => {
                self.close_attempts = self.close_attempts.saturating_add(1);
                self.last_close_tick = Some(obs.tick);
                self.close_epoch = Some(obs.session_epoch);
                self.phase = Phase::Dismissing;
                if self.close_attempts >= MAX_CLOSE_ATTEMPTS {
                    // Last send: wait one more observe for ack before failing.
                    return self.emit(
                        true,
                        WelcomeAction::Close,
                        Some(format!(
                            "welcome: dismissing interface {}",
                            obs.welcome_interface_id
                        )),
                    );
                }
                self.emit(
                    true,
                    WelcomeAction::Close,
                    Some(format!(
                        "welcome: dismissing interface {}",
                        obs.welcome_interface_id
                    )),
                )
            }
            CloseAttempt::Refused(reason) => {
                self.close_attempts = self.close_attempts.saturating_add(1);
                self.last_close_tick = Some(obs.tick);
                self.close_epoch = Some(obs.session_epoch);
                if self.close_attempts >= MAX_CLOSE_ATTEMPTS {
                    self.fail_with(format!(
                        "welcome: close refused ({reason:?}) for interface {} after {} attempts",
                        obs.welcome_interface_id, self.close_attempts
                    ))
                } else {
                    self.emit(
                        true,
                        WelcomeAction::None,
                        Some(format!(
                            "welcome: close refused ({reason:?}) for interface {}",
                            obs.welcome_interface_id
                        )),
                    )
                }
            }
        }
    }

    fn fail(&mut self, interface_id: i32) -> WelcomeStep {
        self.fail_with(format!(
            "welcome: close not acknowledged for interface {interface_id} after {MAX_CLOSE_ATTEMPTS} attempts"
        ))
    }

    fn fail_with(&mut self, reason: String) -> WelcomeStep {
        self.phase = Phase::Failed;
        self.failure = Some(reason.clone());
        self.emit(true, WelcomeAction::None, Some(reason))
    }

    fn emit(&mut self, hold: bool, action: WelcomeAction, notice: Option<String>) -> WelcomeStep {
        let notice = match notice {
            Some(line) if self.last_notice.as_deref() != Some(line.as_str()) => {
                self.last_notice = Some(line.clone());
                Some(line)
            }
            _ => None,
        };
        WelcomeStep {
            hold,
            action,
            failure: self.failure.clone(),
            notice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use script::{RunState, SlotScript};

    fn obs(epoch: u64, tick: u64, welcome: i32, main: i32) -> WelcomeObservation {
        WelcomeObservation {
            session_epoch: epoch,
            tick,
            now: Instant::now(),
            ingame: true,
            scene_state: 2,
            welcome_interface_id: welcome,
            main_modal_id: main,
            allow_close: true,
        }
    }

    fn open_at(tick: u64, id: i32) -> WelcomeObservation {
        obs(1, tick, id, id)
    }

    fn closed_at(tick: u64, welcome: i32) -> WelcomeObservation {
        obs(1, tick, welcome, -1)
    }

    #[test]
    fn identity_is_native_welcome_not_a_guessed_layer() {
        // 5993 is only welcome when the client selected it.
        assert!(!welcome_is_open(-1, 5993));
        assert!(!welcome_is_open(42, 5993));
        assert!(welcome_is_open(5993, 5993));
        assert!(welcome_is_open(42, 42));
        assert!(!welcome_is_open(42, -1));
        assert!(!welcome_is_open(-1, -1));
    }

    #[test]
    fn close_then_ack_releases_hold_before_script_work() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut closes = 0;
        let step = r.step(&open_at(1, 42), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(step.hold, "script work waits for ack");
        assert_eq!(step.action, WelcomeAction::Close);
        assert_eq!(closes, 1);
        assert!(step
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("dismissing interface 42")));

        let mut closes = 0;
        let step = r.step(&closed_at(2, 42), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(!step.hold, "observed close delivers script work");
        assert_eq!(step.action, WelcomeAction::None);
        assert_eq!(closes, 0);
        assert_eq!(step.notice.as_deref(), Some("welcome: settled"));
        assert!(r.failure().is_none());
    }

    #[test]
    fn late_welcome_after_scene2_settled_holds_again() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let _ = r.step(&closed_at(1, -1), || panic!("no close"));
        let step = r.step(&open_at(4, 142), || CloseAttempt::Sent);
        assert!(step.hold);
        assert_eq!(step.action, WelcomeAction::Close);
        let step = r.step(&closed_at(6, 142), || panic!("already closed"));
        assert!(!step.hold);
        assert_eq!(step.notice.as_deref(), Some("welcome: settled"));
    }

    #[test]
    fn unrelated_modal_is_not_closed() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut closes = 0;
        // Bank/trade/quest: main modal is not the native welcome id.
        let step = r.step(&obs(1, 1, 42, 600), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(!step.hold);
        assert_eq!(step.action, WelcomeAction::None);
        assert_eq!(closes, 0);

        let step = r.step(&obs(1, 2, -1, 5993), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(!step.hold);
        assert_eq!(closes, 0);
    }

    #[test]
    fn reconnect_epoch_invalidates_stale_close() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let step = r.step(&open_at(1, 42), || CloseAttempt::Sent);
        assert_eq!(step.action, WelcomeAction::Close);
        r.on_session_boundary();
        assert_eq!(r.session_epoch, 2);
        let mut closes = 0;
        // New session, no welcome yet: must not replay the prior close.
        let step = r.step(&obs(2, 2, -1, -1), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(!step.hold);
        assert_eq!(step.action, WelcomeAction::None);
        assert_eq!(closes, 0);
        // Welcome on the new session is a fresh dismiss, not the stale one.
        let step = r.step(&obs(2, 3, 42, 42), || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert_eq!(step.action, WelcomeAction::Close);
        assert_eq!(closes, 1);
    }

    #[test]
    fn logout_resets_and_does_not_close() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let _ = r.step(&open_at(1, 42), || CloseAttempt::Sent);
        let mut closes = 0;
        let logged_out = WelcomeObservation {
            session_epoch: 1,
            tick: 2,
            now: Instant::now(),
            ingame: false,
            scene_state: 0,
            welcome_interface_id: -1,
            main_modal_id: -1,
            allow_close: false,
        };
        let step = r.step(&logged_out, || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(!step.hold);
        assert_eq!(step.action, WelcomeAction::None);
        assert_eq!(closes, 0);
        assert!(r.failure().is_none());
    }

    #[test]
    fn pause_and_stop_are_not_resumed_by_settlement() {
        let mut paused = SlotScript::new();
        paused
            .start_compiled(Box::new(NoopScript), None)
            .expect("start");
        assert_eq!(paused.state(), RunState::Running);
        paused.pause();
        assert_eq!(paused.state(), RunState::Paused);

        let mut stopped = SlotScript::new();
        stopped
            .start_compiled(Box::new(NoopScript), None)
            .expect("start");
        stopped.stop();
        assert_eq!(stopped.state(), RunState::Idle);

        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let _ = r.step(&open_at(1, 42), || CloseAttempt::Sent);
        let _ = r.step(&closed_at(3, 42), || panic!("no extra close"));

        // Readiness never calls resume/start. Operator intent is untouched.
        assert_eq!(paused.state(), RunState::Paused);
        assert!(!paused.want_run);
        assert_eq!(stopped.state(), RunState::Idle);
    }

    #[test]
    fn bounded_failure_is_visible_and_stops_clicking() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut closes = 0;
        let mut last = r.step(&open_at(1, 42), || {
            closes += 1;
            CloseAttempt::Sent
        });
        for tick in [3u64, 5, 7, 9] {
            last = r.step(&open_at(tick, 42), || {
                closes += 1;
                CloseAttempt::Sent
            });
        }
        assert!(
            last.failure
                .as_deref()
                .is_some_and(|f| f.contains("close not acknowledged") && f.contains("42")),
            "failure must name the native interface: {:?}",
            last.failure
        );
        assert!(last.hold, "stuck welcome still blocks script work");
        assert_eq!(last.action, WelcomeAction::None);
        assert_eq!(closes, MAX_CLOSE_ATTEMPTS);

        let mut extra = 0;
        let last = r.step(&open_at(20, 42), || {
            extra += 1;
            CloseAttempt::Sent
        });
        assert_eq!(extra, 0, "no blind loop after the bound");
        assert!(last.hold);
        assert!(last.failure.is_some());
    }

    #[test]
    fn refused_closes_are_bounded_and_visible() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut last = WelcomeStep {
            hold: false,
            action: WelcomeAction::None,
            failure: None,
            notice: None,
        };
        for tick in [1u64, 3, 5] {
            last = r.step(&open_at(tick, 42), || {
                CloseAttempt::Refused(SendReason::SceneUnavailable)
            });
        }
        assert!(last.hold);
        assert_eq!(last.action, WelcomeAction::None);
        assert!(
            last.failure
                .as_deref()
                .is_some_and(|f| f.contains("close refused") && f.contains("SceneUnavailable")),
            "{:?}",
            last.failure
        );
    }

    #[test]
    fn no_close_before_scene_ready() {
        let mut r = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut closes = 0;
        let early = WelcomeObservation {
            allow_close: false,
            scene_state: 1,
            ..open_at(1, 42)
        };
        let step = r.step(&early, || {
            closes += 1;
            CloseAttempt::Sent
        });
        assert!(step.hold);
        assert_eq!(step.action, WelcomeAction::None);
        assert_eq!(closes, 0);
    }

    #[test]
    fn slow_scene_loading_does_not_spend_the_elapsed_dismissal_bound() {
        let started = Instant::now();
        let mut readiness = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut loading = WelcomeObservation {
            now: started,
            allow_close: false,
            scene_state: 1,
            ..open_at(1, 42)
        };
        let first = readiness.step(&loading, || panic!("loading scene cannot close"));
        assert!(first.failure.is_none());

        loading.now = started + WELCOME_DISMISS_TIMEOUT + Duration::from_secs(4);
        let still_loading = readiness.step(&loading, || panic!("loading scene cannot close"));
        assert!(
            still_loading.failure.is_none(),
            "scene build time is outside the dismissal episode"
        );

        let eligible_started = loading.now;
        let mut eligible = WelcomeObservation {
            allow_close: true,
            scene_state: 2,
            ..loading
        };
        let first_attempt = readiness.step(&eligible, || {
            CloseAttempt::Refused(SendReason::SceneUnavailable)
        });
        assert!(first_attempt.failure.is_none());

        eligible.now = eligible_started + WELCOME_DISMISS_TIMEOUT;
        let timed_out = readiness.step(&eligible, || panic!("tick spacing blocks another close"));
        assert!(timed_out.hold);
        assert!(
            timed_out
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("timed out") && failure.contains("42")),
            "timeout must be visible and name the interface: {:?}",
            timed_out.failure
        );

        let closed = WelcomeObservation {
            now: eligible.now + Duration::from_millis(1),
            main_modal_id: -1,
            ..eligible
        };
        let settled = readiness.step(&closed, || panic!("closed modal needs no action"));
        assert!(!settled.hold);
        assert!(
            settled.failure.is_none(),
            "closure clears the episode failure"
        );
    }

    #[test]
    fn losing_close_eligibility_restarts_time_but_not_attempt_budget() {
        let started = Instant::now();
        let mut readiness = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut first = open_at(1, 42);
        first.now = started;
        let mut attempts = 0;
        let refused = readiness.step(&first, || {
            attempts += 1;
            CloseAttempt::Refused(SendReason::SceneUnavailable)
        });
        assert!(refused.failure.is_none());

        let rebuilding = WelcomeObservation {
            tick: 3,
            now: started + Duration::from_secs(12),
            allow_close: false,
            scene_state: 1,
            ..first
        };
        let waiting = readiness.step(&rebuilding, || panic!("scene rebuild cannot close"));
        assert!(
            waiting.failure.is_none(),
            "time outside close eligibility must not spend the episode bound"
        );

        let resumed = WelcomeObservation {
            tick: 5,
            allow_close: true,
            scene_state: 2,
            ..rebuilding
        };
        let retried = readiness.step(&resumed, || {
            attempts += 1;
            CloseAttempt::Refused(SendReason::SceneUnavailable)
        });
        assert!(
            retried.failure.is_none(),
            "eligibility must start a fresh elapsed-time window"
        );

        let deadline = WelcomeObservation {
            tick: 7,
            now: resumed.now + WELCOME_DISMISS_TIMEOUT,
            ..resumed
        };
        let timed_out = readiness.step(&deadline, || {
            attempts += 1;
            CloseAttempt::Sent
        });
        assert!(
            timed_out
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("timed out")),
            "{:?}",
            timed_out.failure
        );
        assert_eq!(attempts, 2, "the exact elapsed deadline sends no close");

        let mut bounded = LoginReadiness {
            session_epoch: 1,
            ..Default::default()
        };
        let mut bounded_attempts = 0;
        let mut last = None;
        for index in 0..MAX_CLOSE_ATTEMPTS {
            let eligible = WelcomeObservation {
                tick: u64::from(index) * 3 + 1,
                now: started + Duration::from_secs(u64::from(index) * 12),
                ..open_at(1, 42)
            };
            last = Some(bounded.step(&eligible, || {
                bounded_attempts += 1;
                CloseAttempt::Refused(SendReason::SceneUnavailable)
            }));
            if index + 1 < MAX_CLOSE_ATTEMPTS {
                let rebuilding = WelcomeObservation {
                    tick: eligible.tick + 1,
                    now: eligible.now + Duration::from_secs(11),
                    allow_close: false,
                    scene_state: 1,
                    ..eligible
                };
                let _ = bounded.step(&rebuilding, || panic!("scene rebuild cannot close"));
            }
        }
        assert_eq!(bounded_attempts, MAX_CLOSE_ATTEMPTS);
        assert!(
            last.unwrap()
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("close refused")),
            "eligibility gaps must not reset the attempt budget"
        );
    }

    struct NoopScript;

    impl script::Script for NoopScript {
        fn name(&self) -> &str {
            "Noop"
        }
        fn tick(&mut self, _ctx: &mut script::ScriptCtx<'_>) {}
    }
}

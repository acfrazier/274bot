//! Process transport conditions published by the Play composition's own
//! event-loop health turn. The client services report typed causes, but never
//! borrow the host's condition ledger or log on service worker threads.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{ProcessIncarnation, ProcessOrigin};
use api::process_condition::{
    ConditionCause, ConditionId, ProcessConditions, ProcessServiceCode, ProcessServiceCondition,
};
use client_adapter::health::{ReactorLoss, WakeFailure};
use client_adapter::resolver::{ResolverUnavailable, ResolverUnavailableCause};

/// Only new publications need a one-time Park wake after every owner lock is
/// released. Observing a latched condition on subsequent turns has no effect.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HealthEffects {
    pub reactor_unavailable: bool,
    pub resolver_unavailable: bool,
}

/// Cumulative bounded WSS close drain counts, copied from one
/// [`client_adapter::service::NetworkHealth`] turn at the Play health cadence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrainCounters {
    /// Bounded reply-flush or drain deadlines reached since service start.
    pub timeouts: u64,
    /// Peer resets or fatal TLS alerts during the post-Close drain.
    pub drain_errors: u64,
    /// Unfinished drains retired when the reactor stopped.
    pub shutdown_drops: u64,
}

impl DrainCounters {
    /// Copy the three bounded-close counters out of one network health turn.
    pub fn from_network_health(health: client_adapter::service::NetworkHealth) -> Self {
        Self {
            timeouts: health.graceful_close_timeouts,
            drain_errors: health.graceful_close_drain_errors,
            shutdown_drops: health.graceful_close_shutdown_drops,
        }
    }

    fn increased(&self, previous: &Self) -> bool {
        self.timeouts != previous.timeouts
            || self.drain_errors != previous.drain_errors
            || self.shutdown_drops != previous.shutdown_drops
    }
}

pub struct ServiceHealth {
    origin: ProcessOrigin,
    conditions: Mutex<ProcessConditions>,
    last_wake_failure: AtomicU64,
    last_drain: Mutex<DrainCounters>,
}

impl ServiceHealth {
    pub fn new(incarnation: ProcessIncarnation, origin: ProcessOrigin) -> Self {
        Self {
            origin,
            conditions: Mutex::new(ProcessConditions::new(incarnation)),
            last_wake_failure: AtomicU64::new(0),
            last_drain: Mutex::new(DrainCounters::default()),
        }
    }

    /// Reduce only owner-published observations; no service lock or network
    /// call occurs here. The caller handles the returned wake effects after
    /// this method drops the condition lock.
    pub fn observe(
        &self,
        now: Instant,
        reactor: Option<ReactorLoss>,
        resolver: Option<ResolverUnavailable>,
        wake_failure: Option<WakeFailure>,
        drain: DrainCounters,
    ) -> HealthEffects {
        let mut effects = HealthEffects::default();
        let mut conditions = self.conditions.lock().unwrap();
        let incarnation = conditions.incarnation();
        let detected_at = self.origin.elapsed(now);
        if let Some(loss) = reactor.filter(|_| {
            conditions
                .latched(ProcessServiceCode::ReactorUnavailable)
                .is_none()
        }) {
            let condition = ProcessServiceCondition::new(
                ConditionId::new(incarnation, ProcessServiceCode::ReactorUnavailable),
                reactor_cause(loss),
                detected_at,
                None,
            );
            effects.reactor_unavailable = conditions.publish(condition).is_ok();
        }
        if let Some(loss) = resolver.filter(|_| {
            conditions
                .latched(ProcessServiceCode::ResolverUnavailable)
                .is_none()
        }) {
            let condition = ProcessServiceCondition::new(
                ConditionId::new(incarnation, ProcessServiceCode::ResolverUnavailable),
                resolver_cause(loss.cause),
                self.origin.elapsed(loss.detected_at),
                None,
            );
            effects.resolver_unavailable = conditions.publish(condition).is_ok();
        }
        drop(conditions);
        if effects.reactor_unavailable {
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "process condition code=ReactorUnavailable cause={} detected_at_ms={} incarnation={incarnation:?} restart_required=true",
                reactor_cause(reactor.expect("published reactor condition has cause")).label(),
                detected_at.as_duration().as_millis(),
            );
        }
        if effects.resolver_unavailable {
            let loss = resolver.expect("published resolver condition has cause");
            host_log!(
                Category::Lifecycle,
                Level::Warn,
                "process condition code=ResolverUnavailable cause={} detected_at_ms={} incarnation={incarnation:?} restart_required=true",
                resolver_cause(loss.cause).label(),
                self.origin.elapsed(loss.detected_at).as_duration().as_millis(),
            );
        }
        if let Some(failure) = wake_failure {
            let previous = self
                .last_wake_failure
                .fetch_max(failure.total, Ordering::AcqRel);
            if failure.total > previous {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "game reactor wake failed count={} total={} kind={:?} os_error={:?}; committed writes remain accepted",
                    failure.total - previous,
                    failure.total,
                    failure.kind,
                    failure.raw_os_error,
                );
            }
        }
        {
            let mut last = self.last_drain.lock().unwrap();
            if drain.increased(&last) {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "bounded WSS close drain advanced timeouts={} (+{}) drain_errors={} (+{}) shutdown_drops={} (+{})",
                    drain.timeouts,
                    drain.timeouts - last.timeouts,
                    drain.drain_errors,
                    drain.drain_errors - last.drain_errors,
                    drain.shutdown_drops,
                    drain.shutdown_drops - last.shutdown_drops,
                );
                *last = drain;
            }
        }
        effects
    }

    /// T1 owns this ledger; frontends borrow it to project the frozen typed
    /// condition fields. They must not hold the borrow across service calls.
    pub fn with_conditions<R>(&self, inspect: impl FnOnce(&ProcessConditions) -> R) -> R {
        inspect(&self.conditions.lock().unwrap())
    }
}

fn reactor_cause(loss: ReactorLoss) -> ConditionCause {
    match loss {
        ReactorLoss::StartFailed => ConditionCause::StartFailed,
        ReactorLoss::Panicked => ConditionCause::Panicked,
        ReactorLoss::ProgressLost => ConditionCause::ProgressLost,
    }
}

fn resolver_cause(loss: ResolverUnavailableCause) -> ConditionCause {
    match loss {
        ResolverUnavailableCause::StartFailed => ConditionCause::StartFailed,
        ResolverUnavailableCause::Panicked => ConditionCause::Panicked,
        ResolverUnavailableCause::ProgressLost => ConditionCause::ProgressLost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;
    use std::time::Duration;

    #[test]
    fn each_typed_condition_latches_once_at_process_time() {
        let start = Instant::now();
        let health = ServiceHealth::new(
            ProcessIncarnation::from_start_nonce(NonZeroU64::new(9).unwrap()),
            ProcessOrigin::at(start),
        );
        let detected = start + Duration::from_millis(125);
        let resolver = ResolverUnavailable {
            cause: ResolverUnavailableCause::Panicked,
            detected_at: detected,
        };
        assert_eq!(
            health.observe(
                start + Duration::from_millis(200),
                Some(ReactorLoss::ProgressLost),
                Some(resolver),
                None,
                DrainCounters::default(),
            ),
            HealthEffects {
                reactor_unavailable: true,
                resolver_unavailable: true
            },
        );
        assert_eq!(
            health.observe(
                start + Duration::from_secs(3),
                Some(ReactorLoss::StartFailed),
                Some(resolver),
                None,
                DrainCounters::default(),
            ),
            HealthEffects::default(),
        );
        health.with_conditions(|conditions| {
            assert_eq!(
                conditions
                    .latched(ProcessServiceCode::ReactorUnavailable)
                    .unwrap()
                    .cause(),
                ConditionCause::ProgressLost,
            );
            assert_eq!(
                conditions
                    .latched(ProcessServiceCode::ResolverUnavailable)
                    .unwrap()
                    .detected_at()
                    .as_duration(),
                Duration::from_millis(125),
            );
        });
    }

    #[test]
    fn committed_wake_failure_is_observed_once_without_a_loss_condition() {
        let start = Instant::now();
        let health = ServiceHealth::new(
            ProcessIncarnation::from_start_nonce(NonZeroU64::new(10).unwrap()),
            ProcessOrigin::at(start),
        );
        let failure = WakeFailure {
            total: 2,
            kind: std::io::ErrorKind::BrokenPipe,
            raw_os_error: Some(32),
        };
        assert_eq!(
            health.observe(start, None, None, Some(failure), DrainCounters::default()),
            HealthEffects::default()
        );
        assert_eq!(health.last_wake_failure.load(Ordering::Acquire), 2);
        assert_eq!(
            health.observe(start, None, None, Some(failure), DrainCounters::default()),
            HealthEffects::default()
        );
        health.with_conditions(|conditions| {
            assert!(conditions
                .latched(ProcessServiceCode::ReactorUnavailable)
                .is_none());
        });
    }

    #[test]
    fn drain_counters_reach_transport_health_from_one_service_turn() {
        let driver = client_adapter::service::NetworkService::start_composition()
            .expect("network composition starts for the health test");
        let service = driver.service();
        let reactor_health = service.reactor().health();
        let before = service.maintain(Instant::now());
        reactor_health.record_graceful_close_timeout();
        reactor_health.record_graceful_close_drain_error();
        reactor_health.record_graceful_close_shutdown_drop();
        let observed = service.maintain(Instant::now());
        assert!(
            observed.graceful_close_timeouts > before.graceful_close_timeouts,
            "the network health turn must report the timeout counter"
        );
        assert!(
            observed.graceful_close_drain_errors > before.graceful_close_drain_errors,
            "the network health turn must report the drain-error counter"
        );
        assert!(
            observed.graceful_close_shutdown_drops > before.graceful_close_shutdown_drops,
            "the network health turn must report the shutdown-drop counter"
        );
        let expected = DrainCounters::from_network_health(observed);
        let start = Instant::now();
        let health = ServiceHealth::new(
            ProcessIncarnation::from_start_nonce(NonZeroU64::new(11).unwrap()),
            ProcessOrigin::at(start),
        );
        assert_eq!(
            health.observe(start, None, None, None, expected),
            HealthEffects::default()
        );
        assert_eq!(*health.last_drain.lock().unwrap(), expected);
    }
}

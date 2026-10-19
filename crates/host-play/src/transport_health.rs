//! Process transport conditions published by the Play composition's own
//! event-loop health turn. The client services report typed causes, but never
//! borrow the host's condition ledger or log on service worker threads.

use std::sync::Mutex;
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use api::identity::{ProcessIncarnation, ProcessOrigin};
use api::process_condition::{
    ConditionCause, ConditionId, ProcessConditions, ProcessServiceCode, ProcessServiceCondition,
};
use client_adapter::health::ReactorLoss;
use client_adapter::resolver::{ResolverUnavailable, ResolverUnavailableCause};

/// Only new publications need a one-time Park wake after every owner lock is
/// released. Observing a latched condition on subsequent turns has no effect.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HealthEffects {
    pub reactor_unavailable: bool,
    pub resolver_unavailable: bool,
}

pub struct ServiceHealth {
    origin: ProcessOrigin,
    conditions: Mutex<ProcessConditions>,
}

impl ServiceHealth {
    pub fn new(incarnation: ProcessIncarnation, origin: ProcessOrigin) -> Self {
        Self {
            origin,
            conditions: Mutex::new(ProcessConditions::new(incarnation)),
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
            health.observe(start + Duration::from_millis(200), Some(ReactorLoss::ProgressLost), Some(resolver)),
            HealthEffects { reactor_unavailable: true, resolver_unavailable: true },
        );
        assert_eq!(
            health.observe(start + Duration::from_secs(3), Some(ReactorLoss::StartFailed), Some(resolver)),
            HealthEffects::default(),
        );
        health.with_conditions(|conditions| {
            assert_eq!(
                conditions.latched(ProcessServiceCode::ReactorUnavailable).unwrap().cause(),
                ConditionCause::ProgressLost,
            );
            assert_eq!(
                conditions.latched(ProcessServiceCode::ResolverUnavailable).unwrap().detected_at().as_duration(),
                Duration::from_millis(125),
            );
        });
    }
}

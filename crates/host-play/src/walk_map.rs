//! Application-owned map data and input state shared by the panel and TUI.
//! This is not a second router: only an explicit, consumed confirmation reaches
//! `arm_walk_on`. No imagery, cache jobs, UI framework or per-bot catalogue lives here.
mod actions;
mod catalogue;
mod observations;
mod routes;

pub(crate) use actions::{
    emit_walk_aborted, emit_walk_cancelled, emit_walk_terminal, route_supply_shortfall_detail,
    LEGACY_ZONES_DETAIL,
};
pub use actions::{
    ActionError, ActionKind, FocusToken, GroupWalkReport, Layers, MapCommand, MapContext, MapModel,
    MapWalkPlan, Selection, WalkExclude, WalkRequest, WalkSlotOutcome, WalkSlotOutcomeKind,
    WalkSlotReady, WalkSlotRequest, WalkSlotStatus,
};
pub use catalogue::{
    display_name_into, AuthenticatedServices, Catalogue, DisplayName, Entry, Meaning, Search,
    SourceStatus, BANK_API_NOTE,
};
pub use observations::{observed_services, ObservedService, MAX_OBSERVED_SERVICES};
pub use routes::{select_route_source, RouteProjection, RouteSource, RouteStamp};

/// Debug Teleport is admitted only for a Local launch profile on loopback.
/// Panel enablement and [`crate::Play::map_teleport`] both call this.
pub fn debug_teleport_authorized(class: crate::ProfileClass, host: &str) -> bool {
    class == crate::ProfileClass::Local && crate::is_loopback_host(host)
}

/// A new route owner must not reuse the same cache stamp as a retired arm at
/// the same destination. This clock is shared across manual/script owners.
pub(crate) fn next_map_route_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) mod test_log {
    use std::thread::{self, ThreadId};

    type Record = (ThreadId, Option<String>, String);

    #[derive(Default)]
    struct Capture {
        records: parking_lot::Mutex<Vec<Record>>,
    }

    impl api::hostlog::Sink for Capture {
        fn record(&self, record: &api::hostlog::Record<'_>) {
            if record.message.starts_with("WalkTo ") || record.message.starts_with("debug ::") {
                self.records.lock().push((
                    thread::current().id(),
                    record.slot.map(str::to_owned),
                    record.message.to_owned(),
                ));
            }
        }
    }

    static LOG: std::sync::LazyLock<Capture> = std::sync::LazyLock::new(Capture::default);

    /// Install the one process-wide test sink and return a watermark for this
    /// test. Records are also tagged by test-thread id, so parallel tests and
    /// a later test reusing the same harness thread cannot contaminate reads.
    pub(crate) fn mark() -> usize {
        let log = &*LOG;
        let _ = api::hostlog::install_sink(log);
        log.records.lock().len()
    }

    pub(crate) fn records_since(mark: usize) -> Vec<(String, String)> {
        let thread = thread::current().id();
        LOG.records
            .lock()
            .iter()
            .skip(mark)
            .filter(|(record_thread, _, _)| *record_thread == thread)
            .map(|(_, slot, message)| (slot.clone().unwrap_or_default(), message.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests;

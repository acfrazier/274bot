//! Application-owned map data and input state shared by the panel and TUI.
//! This is not a second router: only an explicit, consumed confirmation reaches
//! `arm_walk_on`. No imagery, cache jobs, UI framework or per-bot catalogue lives here.
mod actions;
mod catalogue;
mod observations;
mod routes;

pub use actions::{
    ActionError, ActionKind, FocusToken, GroupWalkReport, Layers, MapCommand, MapContext, MapModel,
    MapWalkPlan, Selection, WalkExclude, WalkSlotOutcome, WalkSlotOutcomeKind, WalkSlotReady,
    WalkSlotRequest, WalkSlotStatus,
};
pub use catalogue::{
    AuthenticatedServices, Catalogue, Entry, Meaning, Search, SourceStatus, BANK_API_NOTE,
};
pub use observations::{observed_services, ObservedService, MAX_OBSERVED_SERVICES};
pub use routes::{select_route_source, RouteProjection, RouteSource, RouteStamp};

/// Debug Teleport is a local-engine cheat: Local target and a loopback host.
/// Panel enablement and [`crate::Play::map_teleport`] both call this; do not copy it.
pub fn debug_teleport_authorized(target: client::BotTarget, host: &str) -> bool {
    target == client::BotTarget::Local && crate::is_loopback_host(host)
}

/// A new route owner must not reuse the same cache stamp as a retired arm at
/// the same destination. This clock is shared across manual/script owners.
pub(crate) fn next_map_route_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests;

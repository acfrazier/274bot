//! Application-owned map data and input state shared by the panel and TUI.
//! This is not a second router: only an explicit, consumed confirmation reaches
//! `arm_walk_on`. No imagery, cache jobs, UI framework or per-bot catalogue lives here.
mod actions;
mod catalogue;
mod observations;
mod routes;

pub use actions::{
    ActionError, ActionKind, FocusToken, Layers, MapCommand, MapContext, MapModel, Selection,
};
pub use catalogue::{
    AuthenticatedServices, Catalogue, Entry, Meaning, Search, SourceStatus, BANK_API_NOTE,
};
pub use observations::{observed_services, ObservedService, MAX_OBSERVED_SERVICES};
pub use routes::{select_route_source, RouteProjection, RouteSource, RouteStamp};

/// A new route owner must not reuse the same cache stamp as a retired arm at
/// the same destination. This clock is shared across manual/script owners.
pub(crate) fn next_map_route_generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests;

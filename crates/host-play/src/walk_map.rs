//! Application-owned map data and input state shared by the panel and TUI.
//! This is not a second router: only an explicit, consumed confirmation reaches
//! `arm_walk_on`. No imagery, cache jobs, UI framework or per-bot catalogue lives here.
mod actions;
mod catalogue;
mod routes;

pub use actions::{
    ActionError, ActionKind, FocusToken, Layers, MapCommand, MapContext, MapModel, Selection,
};
pub use catalogue::{
    AuthenticatedServices, Catalogue, Entry, Meaning, Search, SourceStatus, BANK_API_NOTE,
};
pub use routes::{RouteProjection, RouteSource, RouteStamp};

#[cfg(test)]
mod tests;

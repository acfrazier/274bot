pub mod app;
pub mod build_info;
pub mod chrome;
pub mod clipboard;
pub mod debug_panel;
pub mod fleet;
mod fleet_actions;
mod fleet_columns;
mod fleet_toolbar;
pub mod focus;
pub mod game_view;
pub mod grid;
pub mod headed_record;
mod input_capture;
pub mod loadouts;
pub mod log_pane;
pub mod name_picker;
mod nav_paint_cache;
pub mod nav_settings;
pub mod overlay;
pub mod paint;
pub mod picker;
pub mod profile_script;
pub mod rail;
pub mod script_picker;
pub mod session;
mod session_catalog;
pub mod theme;
pub mod ui_state;
pub mod walk_map;
pub mod wall;
pub mod window;
mod world_picker;

pub use app::*;
pub use chrome::*;
pub use focus::*;
pub use game_view::*;
pub use theme::*;

#[cfg(test)]
mod srgb_present;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod test_alloc;

#[cfg(test)]
#[path = "manual_click_live_tests.rs"]
mod manual_click_live_tests;

#[cfg(test)]
/// Serializes tests that hold a Dear ImGui context: only one can be
/// active at a time (`Context::create` panics on `ContextAlreadyActive`),
/// so parallel tests that each create one race.
pub(crate) static IMGUI_CTX_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
/// Serializes tests that share the process-wide WorkService ledger.
pub(crate) fn work_service_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub mod app;
pub mod build_info;
pub mod chrome;
pub mod focus;
pub mod game_view;
pub mod grid;
pub mod nav_settings;
pub mod overlay;
pub mod paint;
pub mod picker;
pub mod queue_card;
pub mod rail;
pub mod resource;
pub mod script_picker;
pub mod session;
pub mod theme;
pub mod ui_state;
pub mod wall;
pub mod window;

pub use app::*;
pub use chrome::*;
pub use focus::*;
pub use game_view::*;
pub use theme::*;

#[cfg(test)]
/// Serializes tests that hold a Dear ImGui context: only one can be
/// active at a time (`Context::create` panics on `ContextAlreadyActive`),
/// so parallel tests that each create one race.
pub(crate) static IMGUI_CTX_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

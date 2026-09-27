//! Headless TUI view of the operator panel. Second view of the shared
//! `frontend_core` operator session beside the headed `panel` crate: same
//! slots, no GPU.
//!
//! **Raster Off:** slots spawned from the TUI run `RasterMode::Off` and
//! attach no `Renderer`; there is nothing to paint or freeze. The headed
//! `panel` crate keeps Gpu/Cpu. CI tests render to `TestBackend`; nothing
//! here needs a real terminal until `tui-play` wires crossterm events.
//!
//! The shell is responsive (80x24 compact, 120x40 standard, large): a
//! fleet table, the selected bot's tabs (Overview, Map, Script, Chat,
//! Logs), a log drawer and a footer that always names the keyboard scope.
//! `input` routes keys and the mouse through one model, `commands` is the
//! command vocabulary shared by keys, buttons, the palette and help,
//! `layout` is the geometry and `shell` draws it. `tui-play` (bin.rs) wires
//! the actions to the shared session: chat Continue/Answer and manual walks
//! go through [`host_play::WireCmd`], map Walk-confirm routes via
//! `host_play::arm_walk_on`.

pub mod app;
pub mod bin;
pub mod chat;
pub mod commands;
pub mod fleet;
pub mod help;
pub mod input;
pub mod layout;
pub mod loadouts;
pub mod log_pane;
pub mod map;
pub mod overlay;
pub mod palette;
pub mod script_params;
pub mod script_shape;
pub mod settings;
pub mod shell;
pub mod status;
pub(crate) mod stderr_capture;
#[cfg(test)]
pub(crate) mod test_support;

pub use app::{AppAction, TuiApp};
pub use bin::RunMode;
pub use chat::{Chat, ChatAction, ChatState, ChatView};
pub use commands::Command;
pub use layout::{Pane, Screen, SizeClass};
pub use loadouts::{LoadoutsKey, LoadoutsPane, LoadoutsState};
pub use map::{Map, MapAction, MapView};
pub use script_params::{ParamsKey, ParamsPane, ParamsState};
pub use script_shape::ScriptPane;
pub use settings::{SettingsPane, SettingsState};
pub use status::StatusPane;

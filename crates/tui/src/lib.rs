//! Headless TUI view of the operator panel (v0.1.2). Second view of
//! `host_play::Play` beside the headed `panel` crate: same slots, no GPU.
//!
//! **Raster Off:** slots spawned from the TUI run `RasterMode::Off` and
//! attach no `Renderer`; there is nothing to paint or freeze. The headed
//! `panel` crate keeps Gpu/Cpu. CI tests render to `TestBackend`; nothing
//! here needs a real terminal until `tui-play` (Task 10) wires crossterm
//! events.
//!
//! Panes: the WalkTo map (spec `2026-09-01-headless-tui-design.md`), the
//! chat ring / NPC dialogue, the status + inv/stats/locs readout, the
//! disabled script shape, and the settings popup. `tui-play` (bin.rs)
//! wires the panes to `host_play::Play`: chat Continue/Answer and WASD
//! walks go through [`host_play::WireCmd`], map Walk-confirm routes via
//! `host_play::arm_walk_on`.

pub mod app;
pub mod bin;
pub mod chat;
pub mod loadouts;
pub mod map;
pub mod script_params;
pub mod script_shape;
pub mod settings;
pub mod status;

pub use app::{AppAction, TuiApp};
pub use bin::RunMode;
pub use chat::{Chat, ChatAction, ChatState, ChatView};
pub use loadouts::{LoadoutsKey, LoadoutsPane, LoadoutsState};
pub use map::{Map, MapAction, MapView};
pub use script_params::{ParamsKey, ParamsPane, ParamsState};
pub use script_shape::ScriptPane;
pub use settings::{SettingsPane, SettingsState};
pub use status::StatusPane;

//! Headless TUI view of the operator panel (v0.1.2). Second view of
//! `host_play::Play` beside the headed `panel` crate: same slots, no GPU.
//!
//! **Raster Off:** slots spawned from the TUI run `RasterMode::Off` and
//! attach no `Renderer`; there is nothing to paint or freeze. The headed
//! `panel` crate keeps Gpu/Cpu. CI tests render to `TestBackend`; nothing
//! here needs a real terminal until `tui-play` (Task 10) wires crossterm
//! events.

pub mod app;

pub use app::TuiApp;

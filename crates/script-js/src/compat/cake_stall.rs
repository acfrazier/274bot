//! Snapshot-driven Baker stall sequence for the JS compatibility surface.

#[path = "cake_stall/runtime.rs"]
mod runtime;
pub use runtime::{
    carried_cakes, needs_cake_restock_from_snapshot, on_hold, on_pause, on_reset, on_resume,
    CakeStall,
};

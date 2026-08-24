//! Compiled script runner: the `Script` trait, per-tick context, and the
//! per-uid `SlotScript` state machine. No V8 in-tree; JS Load is a later
//! task. This task: compiled only.

pub mod ctx;
pub mod slot;

pub use ctx::{Script, ScriptCtx};
pub use slot::{RunState, SlotScript};

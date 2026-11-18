//! The Luau script surface (ADR 0016, Decision item 1).
//!
//! A surface crate binds `script`, the native core, to one script runtime and
//! holds no machine. This crate is the wiring of that surface. It has no VM,
//! no thread and no process-wide state: a VM exists only inside a run, and a
//! run exists only after the spawn of an admitted Start of a Luau card.
//! `tests/startup_state.rs` keeps it so.
#![deny(unsafe_code)]

mod prelude;
mod runtime;
#[expect(
    unsafe_code,
    reason = "the Luau single-step FFI is isolated in this module"
)]
mod single_step;

pub use runtime::{LuauStartCard, LuauSurfaceFactory, LuauSurfaceRun};

// A run is a `Box<dyn SurfaceRun>`, and `SurfaceRun: Send`, so the VM handle
// must be `Send`. `mlua` gives `Lua: Send` only with its `send` feature, which
// the manifest enables. Dropping the feature fails this build, not a later run.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<mlua::Lua>();
};

//! Operator session shared by the native panel and the TUI.
//!
//! [`OperatorSession`] owns the production operator lifecycle above
//! `host-play`: the unlocked vault and [`host_play::Play`], fleet
//! membership and the logout latch, the selected bot, per-slot surface IO,
//! non-blocking removals, polled status rows and operation results. Front
//! ends supply a small [`SlotSurface`] (the panel's input/framebuffer
//! adapter, or [`HeadlessSurface`]) and render what the session exposes.
//!
//! Slot, connection, login-queue, script-execution and navigation ownership
//! stay in `host-play`; this crate adds no second world or script runtime
//! and never copies a `GameSnapshot`.

pub mod fleet;
pub mod operations;
mod profiles;
pub mod session;
pub mod surface;

pub use fleet::Fleet;
pub use operations::{ActionKind, MemberOutcome, OperationId, OperationReport, Outcome};
pub use session::{
    ArmMirror, OperatorSession, Removal, ScriptStart, Selection, SlotTransition, StartSettled,
    Transition, SLOT_REMOVE_TIMEOUT,
};
pub use surface::{HeadlessSurface, SlotAttach, SlotSurface};

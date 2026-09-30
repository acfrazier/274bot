//! Operator session shared by the native panel and the TUI.
//!
//! [`OperatorSession`] owns the production operator lifecycle above
//! `host-play`: the unlocked vault and [`host_play::Play`], fleet
//! membership and the logout latch, the selected bot, per-slot surface IO,
//! non-blocking removals, polled status rows and operation results. Front
//! ends supply a small [`SlotSurface`] (the panel's input/framebuffer
//! adapter, or [`HeadlessSurface`]) and render what the session exposes:
//! the fleet rows and selected-slot detail ([`FleetView`]) and the process
//! resource meter ([`ResourceView`]), both refreshed by its poll.
//! [`Scripts`] coordinates scripts over that session: the card library,
//! per-profile assignment and parameters, Start all / Stop all, reload and
//! Apply to all.
//!
//! Slot, connection, login-queue, script-execution and navigation ownership
//! stay in `host-play`; this crate adds no second world or script runtime
//! and never copies a `GameSnapshot`.

pub mod bulk;
pub mod fleet;
pub mod group_walk;
pub mod log;
pub mod log_file;
pub mod map_bake;
pub mod marked;
pub mod operations;
pub mod profile_form;
mod profiles;
pub mod progress;
pub mod resources;
pub mod scripts;
pub mod selection;
pub mod session;
pub mod surface;
pub mod views;

pub use bulk::{BulkOutcome, BulkReport, BulkRow};
pub use fleet::Fleet;
pub use group_walk::{walk_marked, MarkedWalk, WalkInputs};
pub use map_bake::{
    load_map_bake_choice, persist_map_bake_choice, MapBakeChoice, MapBakeGate, MapBakePrompt,
    MAP_BAKE_TITLE, MAP_BAKE_WARNING,
};
pub use marked::{
    assign_and_restart_marked, assign_marked, login_marked, logout_marked, restart_scope,
    RestartScope,
};
pub use operations::{ActionKind, MemberOutcome, OperationId, OperationReport, Outcome};
pub use profile_form::{FormNotice, ProfileFormSave, SavedProfile, NOTHING_SAVED};
pub use resources::{Metric, ResourceView};
pub use scripts::{Notice, Scripts};
pub use selection::{
    start_marked, stop_marked, BulkSkip, MarkedSelection, ProfileIdentity, StopReport,
};
pub use session::{
    ArmMirror, OperatorSession, Removal, ScriptStart, Selection, SlotTransition, StartSettled,
    Transition, SLOT_REMOVE_TIMEOUT,
};
pub use surface::{HeadlessSurface, SlotAttach, SlotSurface};
pub use views::{FleetCounts, FleetRow, FleetView, Light, OpBrief, Phase, QueuePlace, SlotDetail};

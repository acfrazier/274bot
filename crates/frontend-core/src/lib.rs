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
pub mod nav_prefs;
pub mod operations;
pub mod profile_form;
pub mod profile_saves;
mod profiles;
pub mod progress;
pub mod quester_paths;
pub mod resources;
pub mod scripts;
pub mod selection;
pub mod session;
pub mod surface;
pub mod views;
pub mod walk_permissions;

pub use bulk::{BulkOutcome, BulkReport, BulkRow};
pub use fleet::Fleet;
pub use group_walk::{walk_marked, MarkedWalk, WalkInputs};
pub use map_bake::{
    load_map_bake_choice, persist_map_bake_choice, MapBakeChoice, MapBakeGate, MapBakePrompt,
    MAP_BAKE_TITLE, MAP_BAKE_WARNING,
};
pub use marked::{
    assign_and_restart_marked, assign_and_restart_profiles, assign_marked, login_marked,
    logout_marked, prepare_apply_settings_marked, restart_scope, run_marked_command,
    MarkedCommandReport, RestartScope,
};
pub use nav_prefs::{nav_preference_at, NavPreference};
pub use operations::{ActionKind, MemberOutcome, OperationId, OperationReport, Outcome};
pub use profile_form::{
    FailedSave, FormNotice, FormSettled, ProfileFormSave, SavedProfile, NOTHING_SAVED,
};
pub use profile_saves::{SaveRecord, SaveResult, SaveSettled, WriteFailure};
pub use quester_paths::{
    QuesterPathsController, QuesterPathsView, LOAD_PATHS_LABEL, QUESTER_HEADING,
    QUEST_PATHS_HEADING, RELOAD_PATHS_LABEL,
};
pub use resources::{Metric, ResourceView};
pub use scripts::{Notice, Scripts};
pub use selection::{
    start_marked, stop_marked, BulkSkip, MarkedSelection, ProfileIdentity, StopReport,
    EMPTY_MARKS_HINT,
};
pub use session::{
    ArmMirror, MemoryNotice, OperatorSession, Removal, ScriptStart, Selection, SlotTransition,
    StartSettled, Transition, SLOT_REMOVE_TIMEOUT,
};
pub use surface::{HeadlessSurface, SlotAttach, SlotSurface};
pub use views::{FleetCounts, FleetRow, FleetView, Light, OpBrief, Phase, QueuePlace, SlotDetail};
pub use walk_permissions::{
    DangerLevel, WalkGlobalsView, BANK_FETCH_PERMISSION_SCOPE, DANGER_THIS_WALK_LABEL,
    GLOBAL_DANGER_WARNING, GLOBAL_PERMISSION_LABELS, ROUTING_SCOPE_NOTE, SCRIPT_SCOPE_NOTICE,
    SURVIVABLE_ROUTING_NOTICE, SURVIVABLE_ROUTING_TOOLTIP,
};

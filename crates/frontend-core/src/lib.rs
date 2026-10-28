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

#[cfg(feature = "memory-profile")]
pub mod bench;
pub mod bot_projection;
pub mod bulk;
pub mod capacity;
pub mod command;
pub mod driver;
pub mod error_expiry;
pub mod fleet;
pub mod fleet_plan;
pub mod frames;
pub mod group_walk;
pub mod launch;
#[cfg(feature = "live-harness")]
pub mod live;
pub mod log;
pub mod log_file;
mod login_notice;
pub mod map_bake;
pub mod map_decode;
pub mod marked;
pub mod models;
pub mod nav_prefs;
pub mod operations;
pub mod prefs;
pub mod process;
pub mod process_status;
pub mod profile_draft;
pub mod profile_editor;
pub mod profile_form;
pub mod profile_order;
pub mod profile_saves;
mod profiles;
pub mod progress;
pub mod quester_paths;
pub mod resources;
pub mod rs2b0t_import;
pub mod scripts;
pub mod selection;
pub mod session;
pub mod setting;
pub mod slot_view;
pub mod surface;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod views;
pub mod walk_map;
pub mod walk_permissions;

pub use bot_projection::{BotProjection, LogoutEligibility, StopEligibility};
pub use bulk::{BulkOutcome, BulkReport, BulkRow};
pub use capacity::{CapacityNotice, CapacityWarning, RenderPolicyLabel};
pub use command::{CommandBook, CommandReport, FleetOrder, OperatorCommand};
pub use driver::HeadlessDriver;
pub use error_expiry::{ErrorExpiry, ERROR_MESSAGE_TIMEOUT};
pub use fleet::{
    contains_ci, fill_visible_row_indices, fleet_row_matches, state_label, Fleet, FleetCard,
    FleetFilter, FleetMarkCounts, FleetModel, MarkOp, MarkPredicate, ShownMarks,
    FOCUSED_CARD_LABEL, NOT_LOADED, OWN_CARD_LABEL,
};
pub use fleet_plan::{FleetAction, FleetPlan, FleetResult, PlanScope, ResultCounts};
pub use group_walk::{MarkedWalk, WalkInputs};
pub use map_bake::{
    load_map_bake_choice, MapBakeChoice, MapBakeGate, MapBakePrompt, MAP_BAKE_TITLE,
    MAP_BAKE_WARNING,
};
pub use map_decode::{WalkMapDecodeController, WalkMapDecodeError, WalkMapDecodeResult};
pub use marked::{
    assign_and_restart_marked, assign_marked, bounded_names, combined_bulk_scope,
    marked_availability, prepare_apply_settings_marked, restart_scope, run_marked_command,
    BulkScope, MarkedAction, MarkedBlock, MarkedCommandReport, RestartScope, ASSIGN_NO_CARD,
    ASSIGN_RESTART_NO_CARD, PROMPT_NAMES_SHOWN,
};
pub use models::Models;
pub use nav_prefs::{nav_preference, NavPreference};
pub use operations::{ActionKind, MemberOutcome, OperationId, OperationReport, Outcome};
pub use prefs::UiPreferences;
pub use profile_draft::{
    CurePoison, GuardianPolicy, ProfileDraft, ProfileView, SettingsDraft, SurvivalDraft,
    SurvivalSettings, GUARDIAN_EVENTS, SURVIVAL_FIELDS,
};
pub use profile_editor::{FormLeave, FormOpen, LeavePrompt, ProfileEditor, SaveRefused};
pub use profile_form::{
    FailedSave, FormNotice, FormSettled, ProfileFormSave, SavedProfile, NOTHING_SAVED,
};
pub use profile_order::{Placement, ProfileOrder, ReorderOp, PROFILE_ORDER_KEY};
pub use profile_saves::{SaveRecord, SaveResult, SaveSettled, WriteFailure};
pub use quester_paths::{
    QuesterPathsController, QuesterPathsView, LOAD_PATHS_LABEL, QUESTER_HEADING,
    QUEST_PATHS_HEADING, RELOAD_PATHS_LABEL,
};
pub use resources::{Metric, ResourceView};
pub use scripts::{Notice, Scripts};
pub use selection::{
    marked_startable, profile_id_at, profile_id_for_name, stop_marked, BulkSkip, MarkedSelection,
    ProfileIdentity, StopReport, EMPTY_MARKS_HINT,
};
pub use session::{
    pick_focus, ArmMirror, DrawPolicy, GameInputEvent, GuardianEdit, MapDesk, MemoryNotice,
    Notice as OperatorNotice, OperatorSession, Removal, ScriptStart, Selection, SettingEvent,
    SettingEventState, SlotTransition, StartSettled, SubmitError, Submitted, SurvivalEdit, Targets,
    Transition, SLOT_REMOVE_TIMEOUT,
};
pub use setting::{Setting, SettingStatus};
pub use slot_view::{SlotView, StatusBuf};
pub use surface::{HeadlessSurface, SlotAttach, SlotSurface, SpawnProfile};
pub use views::{FleetCounts, FleetRow, FleetView, Light, OpBrief, Phase, QueuePlace, SlotDetail};
pub use walk_permissions::{
    DangerLevel, WalkGlobals, WalkGlobalsView, BANK_FETCH_PERMISSION_SCOPE, DANGER_THIS_WALK_LABEL,
    GLOBAL_DANGER_WARNING, GLOBAL_PERMISSION_LABELS, ROUTING_SCOPE_NOTE, SCRIPT_SCOPE_NOTICE,
    SURVIVABLE_ROUTING_NOTICE, SURVIVABLE_ROUTING_TOOLTIP,
};

#[cfg(test)]
mod survival_live_tests;

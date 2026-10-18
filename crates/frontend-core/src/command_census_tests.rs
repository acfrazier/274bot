//! The operator-decision census (F0 item 5, review round 3): every operator
//! entry point of this crate, and every panel/TUI function that reaches
//! one, has exactly one [`OperatorCommand`] row or an explicit, justified
//! exclusion. Completeness is checked mechanically, not enumerated by hand:
//!
//! - **Core universe:** every externally reachable `pub fn` of this crate
//!   (module path all `pub mod`, or an inherent method of a public or
//!   re-exported type, or a re-exported free fn) that either takes a `&mut`
//!   parameter or has no `self` receiver at all (free and associated fns
//!   such as `persist_map_bake_choice` write files without `&mut`). The
//!   book itself (`command.rs`, `process_status.rs`) is F0-5's own protocol
//!   and the census scanner is test support: neither is an operator entry
//!   point. Constructors (`-> Self`) and items under a test/test-support
//!   `cfg` are excluded by class and listed.
//! - **Surface universe:** every fn of `panel` and `tui` whose body calls a
//!   reachable core entry (through a field or parameter typed as a core
//!   owner, or a re-exported free fn) or hits an effect probe (a store
//!   `.save()`, a catalogue defer/clear, a vault create/unlock/reset, a
//!   file write, a map demand open/release, a legacy `set_value`). A
//!   surface fn whose core calls resolve to exactly one row and no probe
//!   is that row's handler; every other one must be registered as a
//!   composition, a dispatcher or an exclusion.
//! - **Rows:** every row of [`OperatorCommand::ALL`] is reached by at least
//!   one registered entry, or is a normative row with no current entry
//!   point (`SwitchWorld`).
//!
//! The scan is syntactic (comments and literals blanked, braces tracked);
//! it reads the sibling crates' sources through `CARGO_MANIFEST_DIR`.

#[path = "command_census_scan.rs"]
mod scan;

use std::collections::BTreeMap;
use std::path::Path;

use self::scan::{display_surface, scan, Census, CoreEntry};
use super::*;
use crate::rs2b0t_import::ConflictPolicy;

/// What the census records for one entry point.
#[derive(Debug, Clone, Copy)]
enum Decision {
    /// The entry point is this row (one row, however many members).
    Row(OperatorCommand),
    /// The entry point issues one of these rows, chosen by a parameter, by
    /// the state it finds, or as a composition on the surface: why.
    Rows(&'static [OperatorCommand], &'static str),
    /// Not an operator decision: why.
    Excluded(&'static str),
}

use Decision::{Excluded, Row, Rows};
use OperatorCommand as C;

const PUMP: &str = "pump: adopts owner results and refreshes projections; no operator decision";
const RESULT: &str = "result consumption: takes settled owner results for display";
const PROJECTION: &str = "projection copy into the front end's buffer; presentation";
const FORMAT: &str = "pure formatting or parsing; no state";
const LOG_VIEW: &str =
    "control of the log read projection (scope, filter, follow); not transport, work admission or a durable write";
const STARTUP: &str = "startup wiring of a store path or adapter; no operator decision";
const ACCESSOR: &str = "accessor exposing an owned model (T3.1 accessor migration), not a decision";
const SETTINGS_KIND: &str = "the card kind of the selection decides the row";
const WRITER: &str = "the durable writer's entry: the caller's mirror and label name the row (credentials → SaveProfile / SetWorld, tutorial → MarkTutorialSkipped, render prefs → SetRaster)";

/// Rows the contract keeps although no current entry point issues them.
const NORMATIVE_ONLY: &[OperatorCommand] = &[C::SwitchWorld(1)];

/// Every reachable core entry point outside the by-class exclusions.
const CORE: &[(&str, Decision)] = &[
    // fleet.rs: the session's fleet model, moved by the rows named.
    ("fleet.rs::Fleet::seed_running", Row(C::SetMultiBox(true))),
    ("fleet.rs::Fleet::add", Row(C::Load)),
    ("fleet.rs::Fleet::remove", Row(C::Remove)),
    ("fleet.rs::Fleet::latch_logout", Row(C::Logout)),
    ("fleet.rs::Fleet::clear_latch", Row(C::Login)),
    // fleet.rs: the fleet view model over the staged filter/order.
    ("fleet.rs::contains_ci", Excluded(FORMAT)),
    ("fleet.rs::state_label", Excluded(FORMAT)),
    (
        "fleet.rs::fleet_row_matches",
        Excluded("pure filter predicate over the staged filter text; SetFleetFilter stages the text"),
    ),
    (
        "fleet.rs::member_row",
        Excluded("pure lookup of a member's projected row; presentation"),
    ),
    (
        "fleet.rs::fill_visible_indices",
        Excluded("pure computation of the shown rows from the staged filter text and order"),
    ),
    (
        "fleet.rs::fill_visible_row_indices",
        Excluded("pure computation of the shown rows from the staged filter text and order"),
    ),
    // group_walk.rs
    ("group_walk.rs::walk_marked", Row(C::GroupWalk)),
    (
        "group_walk.rs::none_marked",
        Excluded("the report shown for a group walk with nothing marked; presentation"),
    ),
    // log.rs: the log store and its read projection.
    ("log.rs::LogEntry::write_line", Excluded(PROJECTION)),
    ("log.rs::LogFilter::set_source", Excluded(LOG_VIEW)),
    ("log.rs::LogFilter::set_min_level", Excluded(LOG_VIEW)),
    ("log.rs::LogFilter::set_sources", Excluded(LOG_VIEW)),
    ("log.rs::LogFilter::set_text", Excluded(LOG_VIEW)),
    ("log.rs::LogView::set_follow", Excluded(LOG_VIEW)),
    ("log.rs::LogView::follow_slot", Excluded(LOG_VIEW)),
    ("log.rs::LogView::set_scope", Excluded(LOG_VIEW)),
    ("log.rs::LogView::edit_filter", Excluded(LOG_VIEW)),
    ("log.rs::LogStore::refresh", Excluded(PUMP)),
    ("log.rs::LogStore::slot_last_message", Excluded(PROJECTION)),
    ("log.rs::classify_script_line", Excluded(FORMAT)),
    ("log.rs::default_save_path", Excluded("path derivation; no effect")),
    ("log.rs::global", Excluded(ACCESSOR)),
    ("log.rs::logs_dir", Excluded("path derivation; no effect")),
    ("log.rs::save_text", Row(C::SaveLog)),
    // log_file.rs
    ("log_file.rs::apply_session_log", Row(C::SetSessionLog(true))),
    (
        "log_file.rs::persist_session_log_setting",
        Row(C::SetSessionLog(true)),
    ),
    (
        "log_file.rs::session_log_setting",
        Excluded("reads the persisted setting"),
    ),
    // map_bake.rs
    (
        "map_bake.rs::load_map_bake_choice",
        Excluded("reads the persisted choice"),
    ),
    (
        "map_bake.rs::persist_map_bake_choice",
        Row(C::SetMapBakeChoice(MapBakeChoice::Ask)),
    ),
    (
        "map_bake.rs::MapBakeGate::set_choice",
        Row(C::SetMapBakeChoice(MapBakeChoice::Ask)),
    ),
    ("map_bake.rs::MapBakeGate::accept", Row(C::BakeMapNow)),
    ("map_bake.rs::MapBakeGate::decline", Row(C::DeclineMapBake)),
    ("map_bake.rs::MapBakeGate::open_profile", Row(C::OpenMapDemand)),
    ("map_bake.rs::MapBakeGate::open", Row(C::OpenMapDemand)),
    (
        "map_bake.rs::MapBakeGate::note_ready_without_terrain",
        Excluded("adopts the ready demand's result (no terrain) into the prompt: owner result"),
    ),
    // map_decode.rs: frame-driven image work, not operator commands.
    (
        "map_decode.rs::WalkMapDecodeController::request",
        Excluded("visible-tile PNG demand from the map frame's visible-area scan; not an operator decision"),
    ),
    (
        "map_decode.rs::WalkMapDecodeController::retry_pending",
        Excluded("bounded retry of the frame's pending PNG after capacity or visibility change; not an operator decision"),
    ),
    (
        "map_decode.rs::WalkMapDecodeController::poll",
        Excluded("frame pump consumes the visible tile's settled RGBA owner result; not an operator decision"),
    ),
    (
        "map_decode.rs::WalkMapDecodeController::cancel_active",
        Excluded("view visibility or lifecycle invalidates an accepted tile decode; not an operator decision"),
    ),
    (
        "map_decode.rs::WalkMapDecodeController::discard_pending",
        Excluded("drops the pending tile made obsolete by visible-area or image change; not an operator decision"),
    ),
    (
        "map_decode.rs::WalkMapDecodeController::invalidate",
        Excluded("map close or image, revision, or generation lifecycle invalidation; not an operator decision"),
    ),
    // marked.rs
    ("marked.rs::assign_marked", Row(C::AssignScript)),
    ("marked.rs::assign_and_restart_marked", Row(C::AssignAndRestart)),
    ("marked.rs::login_marked", Row(C::Login)),
    ("marked.rs::logout_marked", Row(C::Logout)),
    (
        "marked.rs::prepare_apply_settings_marked",
        Rows(
            &[
                C::ApplySettingsToMarked(CardKind::Script),
                C::ApplySettingsToMarked(CardKind::Native),
            ],
            SETTINGS_KIND,
        ),
    ),
    (
        "marked.rs::restart_scope",
        Excluded("pure computation of which marked members a restart covers"),
    ),
    (
        "marked.rs::run_marked_command",
        Excluded("generic marked-row driver applying the caller's admission; the row is the caller's"),
    ),
    (
        "marked.rs::marked_availability",
        Excluded("pure gating of the marked actions; each action's row is its caller's"),
    ),
    (
        "marked.rs::combined_bulk_scope",
        Excluded("pure scope choice for combined all/marked commands; the command's row is the caller's"),
    ),
    ("marked.rs::bounded_names", Excluded(FORMAT)),
    // nav_prefs.rs
    (
        "nav_prefs.rs::nav_preference",
        Row(C::SetNavPreference(NavPreference::AllowTeleports, true)),
    ),
    // profile_form.rs: the form-save machine.
    (
        "profile_form.rs::ProfileFormSave::form_changed",
        Rows(
            &[
                C::OpenProfileForm,
                C::OpenNewProfileForm,
                C::CloseProfileForm(FormClose::Form),
            ],
            "the chooser retargeted or closed the form: that row's effect on the save notice",
        ),
    ),
    (
        "profile_form.rs::ProfileFormSave::edited",
        Row(C::EditProfileDraft(ProfileDraftField::Settings)),
    ),
    (
        "profile_form.rs::ProfileFormSave::refused",
        Excluded("owner result: the save was refused before anything was written (SaveProfile's Rejected terminal)"),
    ),
    ("profile_form.rs::ProfileFormSave::submitted", Row(C::SaveProfile)),
    (
        "profile_form.rs::ProfileFormSave::settle",
        Excluded("adopts the durable writer's results (SaveProfile's Owner terminal)"),
    ),
    // profiles.rs
    (
        "profiles.rs::fresh_uid",
        Excluded("derives the next profile UID from the vault; identity calculation only"),
    ),
    // progress.rs, resources.rs: labels and meters.
    ("progress.rs::span_label", Excluded(FORMAT)),
    ("progress.rs::runtime_label", Excluded(FORMAT)),
    ("progress.rs::idle_label", Excluded(FORMAT)),
    ("progress.rs::levels_label", Excluded(FORMAT)),
    ("progress.rs::levels_detail", Excluded(FORMAT)),
    ("resources.rs::probe_process", Excluded("reads the process counters")),
    ("resources.rs::cpu_from_delta", Excluded(FORMAT)),
    ("resources.rs::format_bytes", Excluded(FORMAT)),
    ("resources.rs::format_bots", Excluded(FORMAT)),
    ("resources.rs::format_background", Excluded(FORMAT)),
    ("resources.rs::background_ack_text", Excluded(FORMAT)),
    ("resources.rs::ResourceView::copy_from", Excluded(PROJECTION)),
    // rs2b0t_import.rs
    (
        "rs2b0t_import.rs::parse_profile_file",
        Excluded("validates the export and registers parsed passwords for redaction; parsing/privacy handling, not an operator decision or vault write"),
    ),
    (
        "rs2b0t_import.rs::read_profile_file",
        Excluded("reads and validates the selected export for a preview; no operator decision or vault write"),
    ),
    (
        "rs2b0t_import.rs::plan_import",
        Excluded("maps the parsed export and vault into a preview plan; writes nothing"),
    ),
    (
        "rs2b0t_import.rs::apply_import",
        Rows(
            &[C::CreateProfile, C::SaveProfile],
            "each account's planned status selects CreateProfile for a new profile or SaveProfile for an existing one",
        ),
    ),
    (
        "rs2b0t_import.rs::delete_export_file",
        Row(C::DeleteProfileImportExport),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::path_mut",
        Row(C::EditProfileImportPath),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::preview",
        Row(C::PreviewProfileImport),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::set_policy",
        Row(C::SetProfileImportPolicy(ConflictPolicy::Skip)),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::set_include_unusable",
        Row(C::SetProfileImportIncludeUnusable(true)),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::confirm",
        Rows(
            &[C::CreateProfile, C::SaveProfile],
            "the confirmed import writes each planned account as a create or save according to its status",
        ),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::back",
        Row(C::ReturnToProfileImportPath),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::ask_delete",
        Row(C::SetProfileImportDeletePrompt(true)),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::cancel_delete",
        Row(C::SetProfileImportDeletePrompt(false)),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::confirm_delete",
        Row(C::DeleteProfileImportExport),
    ),
    // quester_paths.rs
    (
        "quester_paths.rs::QuesterPathsController::restore",
        Excluded("startup restore of the persisted settings"),
    ),
    (
        "quester_paths.rs::QuesterPathsController::apply_changed",
        Row(C::SetQuesterPaths),
    ),
    (
        "quester_paths.rs::QuesterPathsController::request_reload",
        Row(C::ReloadQuesterPaths),
    ),
    (
        "quester_paths.rs::QuesterPathsController::advance",
        Excluded(PUMP),
    ),
    (
        "quester_paths.rs::QuesterPathsController::clear_notice",
        Row(C::AcknowledgeNotice),
    ),
    (
        "quester_paths.rs::QuesterPathsView::set_enabled",
        Excluded("draft mutation of the paths form; persist is SetQuesterPaths"),
    ),
    (
        "quester_paths.rs::QuesterPathsView::set_folder",
        Excluded("draft mutation of the paths form; persist is SetQuesterPaths"),
    ),
    // scripts/mod.rs: the script coordinator.
    (
        "scripts/mod.rs::Notice::apply",
        Excluded("presentation: applies the notice to the front end's banner"),
    ),
    (
        "scripts/mod.rs::Scripts::assignment",
        Excluded("reads the profile's assignment"),
    ),
    ("scripts/mod.rs::sel_from_assignment", Excluded(FORMAT)),
    ("scripts/mod.rs::Scripts::take_notice", Excluded(RESULT)),
    (
        "scripts/mod.rs::Scripts::dismiss_restart_prompt",
        Row(C::DeferSettingsRestart),
    ),
    (
        "scripts/mod.rs::Scripts::restart_pending_settings",
        Row(C::RestartForSettings),
    ),
    (
        "scripts/mod.rs::Scripts::show_load_failures",
        Excluded("presentation: shows the library's load failures on the banner once a Start is accepted"),
    ),
    ("scripts/mod.rs::Scripts::take_start_failures", Excluded(RESULT)),
    (
        "scripts/mod.rs::Scripts::mark_catalog_filled",
        Excluded("marks the first-run catalogue prompt as handled for this session (Browse open with no root to import); no durable write, the durable defer is DeferCatalogImport"),
    ),
    ("scripts/mod.rs::Scripts::fill_catalog_once", Row(C::ImportCatalog)),
    (
        "scripts/reload.rs::Scripts::request_catalog_fill",
        Row(C::ImportCatalog),
    ),
    (
        "scripts/reload.rs::Scripts::request_catalog_import",
        Row(C::ImportCatalog),
    ),
    (
        "scripts/reload.rs::Scripts::request_transpile",
        Excluded("bounded background preparation for an already selected card; no operator decision"),
    ),
    (
        "scripts/reload.rs::Scripts::request_external_loader_transpile",
        Excluded("bounded background preparation for an already selected file; no operator decision"),
    ),
    (
        "scripts/reload.rs::Scripts::take_catalog_import_result",
        Excluded(RESULT),
    ),
    (
        "scripts/reload.rs::Scripts::take_transpile_result",
        Excluded(RESULT),
    ),
    (
        "scripts/reload.rs::Scripts::take_external_loader_transpile_result",
        Excluded(RESULT),
    ),
    (
        "scripts/reload.rs::Scripts::fill_catalog_blocking",
        Excluded("synchronous non-interactive live setup before the session starts"),
    ),
    (
        "scripts/reload.rs::Scripts::transpile_blocking",
        Excluded("synchronous non-interactive live setup before the session starts"),
    ),
    (
        "scripts/mod.rs::Scripts::set_pending_browse",
        Row(C::SelectScriptCard),
    ),
    ("scripts/mod.rs::Scripts::persist_assignment", Row(C::AssignScript)),
    (
        "scripts/mod.rs::Scripts::mark_assignment_unavailable",
        Excluded("owner result: a reload that lost a catalogue card marks its assignments unavailable"),
    ),
    (
        "scripts/mod.rs::Scripts::profile_overrides",
        Excluded("read of the profile's parameter bag; its first use of a card claims the legacy overrides into the profile (`claim_legacy_for`), a migration write with no operator decision beyond showing the parameters"),
    ),
    (
        "scripts/mod.rs::Scripts::merged_profile_bag",
        Excluded("read of the bag a Start receives; same first-use legacy claim as profile_overrides"),
    ),
    (
        "scripts/mod.rs::Scripts::set_profile_setting",
        Row(C::SaveScriptSettings(CardKind::Script)),
    ),
    ("scripts/mod.rs::Scripts::cancel_queued", Row(C::CancelQueuedStart)),
    (
        "scripts/mod.rs::Scripts::cancel_queued_as",
        Row(C::CancelQueuedStart),
    ),
    ("scripts/mod.rs::Scripts::start_profile", Row(C::StartScript)),
    ("scripts/mod.rs::Scripts::start_selected", Row(C::StartScript)),
    ("scripts/mod.rs::Scripts::start_all", Row(C::StartScript)),
    ("scripts/mod.rs::Scripts::stop_all", Row(C::StopScript)),
    ("scripts/mod.rs::Scripts::set_js", Excluded(STARTUP)),
    ("scripts/mod.rs::Scripts::set_legacy", Excluded(STARTUP)),
    ("scripts/mod.rs::Scripts::js_mut", Excluded(ACCESSOR)),
    ("scripts/mod.rs::Scripts::legacy_mut", Excluded(ACCESSOR)),
    (
        "scripts/mod.rs::Scripts::live_start_parts",
        Excluded(ACCESSOR),
    ),
    (
        "scripts/mod.rs::Scripts::set_inject",
        Excluded("harness inject bag; no operator decision"),
    ),
    ("scripts/mod.rs::Scripts::poll", Excluded(PUMP)),
    (
        "scripts/mod.rs::Scripts::publish_start_places",
        Excluded("projection publication: queue places into the fleet rows"),
    ),
    // scripts/native.rs
    (
        "scripts/native.rs::Scripts::set_compiled_setting",
        Row(C::SaveScriptSettings(CardKind::Native)),
    ),
    (
        "scripts/native.rs::Scripts::set_compiled_overrides",
        Row(C::SaveScriptSettings(CardKind::Native)),
    ),
    (
        "scripts/native.rs::Scripts::native_command",
        Rows(
            &[C::PauseScript, C::ResumeScript, C::StopScript],
            "the NativeCommand parameter selects the row",
        ),
    ),
    // scripts/parameter_edit.rs: the parameter editor.
    (
        "scripts/parameter_edit.rs::parse_parameter_text",
        Excluded(FORMAT),
    ),
    (
        "scripts/parameter_edit.rs::resolve_parameter_options",
        Excluded("resolves a schema row's options from the registry snapshot; no state"),
    ),
    (
        "scripts/parameter_edit.rs::ParameterOptions::toggle_array_choice",
        Excluded("pure helper over the caller's selection buffer"),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::parameter_text_mut",
        Row(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::parameter_text_mut_for",
        Row(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::validate_parameter_buffer",
        Row(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::validate_parameter_buffer_for",
        Row(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::discard_parameter_text_for",
        Row(C::DiscardParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::discard_parameter_text",
        Row(C::DiscardParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::commit_parameter_value",
        Rows(
            &[
                C::SaveScriptSettings(CardKind::Script),
                C::SaveScriptSettings(CardKind::Native),
            ],
            SETTINGS_KIND,
        ),
    ),
    // scripts/reload.rs
    ("scripts/reload.rs::Scripts::take_reload_outcome", Excluded(RESULT)),
    ("scripts/reload.rs::Scripts::begin_reload", Row(C::ReloadScripts)),
    ("scripts/reload.rs::Scripts::cancel_reload", Row(C::CancelReload)),
    (
        "scripts/reload.rs::Scripts::begin_refresh_catalog",
        Row(C::ImportCatalog),
    ),
    // scripts/sync.rs: Apply to all.
    (
        "scripts/sync.rs::Scripts::prepare_settings_sync",
        Row(C::ApplySettingsToMarked(CardKind::Script)),
    ),
    (
        "scripts/sync.rs::Scripts::prepare_compiled_settings_sync",
        Row(C::ApplySettingsToMarked(CardKind::Native)),
    ),
    (
        "scripts/sync.rs::Scripts::cancel_settings_sync",
        Row(C::CancelSettingsSync),
    ),
    (
        "scripts/sync.rs::Scripts::apply_settings_sync",
        Rows(
            &[
                C::ApplySettingsToMarked(CardKind::Script),
                C::ApplySettingsToMarked(CardKind::Native),
            ],
            "the prepared scope's card kind selects the row",
        ),
    ),
    // selection.rs: marks and the marked bulk commands.
    ("selection.rs::MarkedSelection::toggle", Row(C::SetMark(true))),
    ("selection.rs::MarkedSelection::set", Row(C::SetMark(true))),
    ("selection.rs::MarkedSelection::clear", Row(C::ClearMarks)),
    ("selection.rs::MarkedSelection::mark_all", Row(C::MarkAll)),
    ("selection.rs::MarkedSelection::remove", Row(C::DeleteProfile)),
    ("selection.rs::MarkedSelection::rename", Row(C::CreateProfile)),
    (
        "selection.rs::MarkedSelection::retain",
        Excluded("projection maintenance: drops marks of rows no longer shown"),
    ),
    (
        "selection.rs::MarkedSelection::retain_sorted",
        Excluded("projection maintenance: drops marks of rows no longer shown"),
    ),
    ("selection.rs::start_marked", Row(C::StartScript)),
    ("selection.rs::stop_marked", Row(C::StopScript)),
    ("selection.rs::MarkedSelection::mark_shown", Row(C::MarkAll)),
    (
        "selection.rs::profile_id_at",
        Excluded("pure identity resolution over the caller's projected rows"),
    ),
    (
        "selection.rs::profile_id_for_name",
        Excluded("pure identity resolution over the caller's projected rows"),
    ),
    (
        "selection.rs::marked_startable",
        Excluded("pure computation of whether a marked Start could start anything"),
    ),
    // session.rs: the operator session.
    ("session.rs::MemoryNotice::mode_name", Excluded(FORMAT)),
    (
        "session.rs::pick_focus",
        Excluded("pure choice of the focus after Load all / MultiBox adoption"),
    ),
    ("session.rs::MemoryNotice::status_text", Excluded(FORMAT)),
    ("session.rs::OperatorSession::start", Row(C::UnlockVault)),
    (
        "session.rs::OperatorSession::set_pause_script_on_manual_walk_abort",
        Row(C::SetNavPreference(
            NavPreference::PauseScriptOnManualWalkAbort,
            true,
        )),
    ),
    (
        "session.rs::OperatorSession::set_walk_globals",
        Row(C::SetDangerLevel(DangerLevel::Never)),
    ),
    (
        "session.rs::OperatorSession::set_walk_globals_store",
        Excluded(STARTUP),
    ),
    (
        "session.rs::OperatorSession::bind_local_state",
        Excluded(STARTUP),
    ),
    ("session.rs::OperatorSession::play_mut", Excluded(ACCESSOR)),
    ("session.rs::OperatorSession::close_play", Row(C::Quit)),
    (
        "session.rs::OperatorSession::copy_statuses_into",
        Excluded(PROJECTION),
    ),
    (
        "session.rs::OperatorSession::set_capacity_render",
        Row(C::SetRenderAll(true)),
    ),
    (
        "session.rs::OperatorSession::set_walk_arms",
        Excluded("wiring: hands the front end's walk arms to the row projection"),
    ),
    ("session.rs::OperatorSession::take_settled_starts", Excluded(RESULT)),
    ("session.rs::OperatorSession::select", Row(C::Select)),
    ("session.rs::OperatorSession::ensure_slot", Row(C::Load)),
    ("session.rs::OperatorSession::open_slot", Row(C::Load)),
    ("session.rs::OperatorSession::load", Row(C::Load)),
    ("session.rs::OperatorSession::load_all", Row(C::Load)),
    ("session.rs::OperatorSession::login", Row(C::Login)),
    ("session.rs::OperatorSession::login_all", Row(C::Login)),
    ("session.rs::OperatorSession::login_members", Row(C::Login)),
    ("session.rs::OperatorSession::logout", Row(C::Logout)),
    ("session.rs::OperatorSession::logout_all", Row(C::Logout)),
    ("session.rs::OperatorSession::logout_members", Row(C::Logout)),
    (
        "session.rs::OperatorSession::set_memory_mode",
        Row(C::SetMemoryMode(MemoryMode::Low)),
    ),
    (
        "session.rs::OperatorSession::request_memory_relog",
        Row(C::RelogNow),
    ),
    (
        "session.rs::OperatorSession::seed_running",
        Row(C::SetMultiBox(true)),
    ),
    ("session.rs::OperatorSession::remove", Row(C::Remove)),
    ("session.rs::OperatorSession::poll", Excluded(PUMP)),
    ("session.rs::OperatorSession::poll_at", Excluded(PUMP)),
    ("session.rs::OperatorSession::poll_host", Excluded(PUMP)),
    (
        "session.rs::OperatorSession::publish_start_queue",
        Excluded("projection publication: queue places into the fleet rows"),
    ),
    ("session.rs::OperatorSession::advance_removals", Excluded(PUMP)),
    ("session.rs::OperatorSession::start_script", Row(C::StartScript)),
    (
        "session.rs::OperatorSession::toggle_pause",
        Rows(
            &[C::PauseScript, C::ResumeScript],
            "the run's state selects the row",
        ),
    ),
    ("session.rs::OperatorSession::stop_script", Row(C::StopScript)),
    ("session.rs::OperatorSession::stop_scripts", Row(C::StopScript)),
    (
        "session.rs::OperatorSession::save_profile",
        Rows(
            &[
                C::SaveProfile,
                C::SetWorld(WorldChoice::Automatic),
                C::MarkTutorialSkipped,
                C::SetRaster(RasterMode::Gpu),
            ],
            WRITER,
        ),
    ),
    ("session.rs::OperatorSession::create_profile", Row(C::CreateProfile)),
    ("session.rs::OperatorSession::rename_profile", Row(C::CreateProfile)),
    ("session.rs::OperatorSession::vault_remove", Row(C::DeleteProfile)),
    (
        "session.rs::OperatorSession::set_auto_login",
        Row(C::SetAutoLogin(true)),
    ),
    (
        "session.rs::OperatorSession::set_random_settings",
        Row(C::SetGuardian),
    ),
    (
        "session.rs::OperatorSession::set_random_event_policy",
        Row(C::SetGuardian),
    ),
    ("session.rs::OperatorSession::take_write_failures", Excluded(RESULT)),
    ("session.rs::OperatorSession::track_save", Row(C::SaveProfile)),
    ("session.rs::OperatorSession::take_settled_saves", Excluded(RESULT)),
    ("session.rs::OperatorSession::take_settings_writes", Excluded(RESULT)),
    (
        "session.rs::OperatorSession::flush_writes",
        Excluded("startup/harness boundary: blocks until the queued writes settle"),
    ),
    // views.rs
    ("views.rs::QueuePlace::write_wait", Excluded(PROJECTION)),
    ("views.rs::FleetRow::write_world_login", Excluded(PROJECTION)),
    ("views.rs::FleetView::copy_rows_into", Excluded(PROJECTION)),
    ("views.rs::FleetView::copy_detail_into", Excluded(PROJECTION)),
    ("views.rs::FleetRow::copy_from", Excluded(PROJECTION)),
    ("views.rs::SlotDetail::copy_from", Excluded(PROJECTION)),
    ("views.rs::random_kind_name", Excluded(FORMAT)),
    ("views.rs::run_state_label", Excluded(FORMAT)),
    ("views.rs::script_status_label", Excluded(FORMAT)),
    ("views.rs::script_status_reason", Excluded(FORMAT)),
    ("views.rs::script_status_rows", Excluded(FORMAT)),
    // walk_permissions.rs
    (
        "walk_permissions.rs::WalkGlobalsView::persist_changed",
        Rows(
            &[
                C::SetNavPreference(NavPreference::AllowTeleports, true),
                C::SetDangerLevel(DangerLevel::Never),
            ],
            "which walk global changed selects the row",
        ),
    ),
    ("walk_permissions.rs::danger_routing_held", Excluded(FORMAT)),
    ("walk_permissions.rs::danger_routing_label", Excluded(FORMAT)),
    ("walk_permissions.rs::manual_walk_may_follow", Excluded(FORMAT)),
    ("walk_permissions.rs::survivable_routing_notice", Excluded(FORMAT)),
    ("walk_permissions.rs::survivable_routing_tooltip", Excluded(FORMAT)),
];

/// Panel and TUI functions that are not simply one row's handler: a
/// composition of several rows, a dispatcher, a handler that makes its
/// decision outside this crate (a store write, a demand, the vault), or an
/// exclusion. A function whose core calls resolve to exactly one row and
/// that hits no effect probe needs no entry: it is that row's handler.
const SURFACE: &[(&str, Decision)] = &[
    // --- panel: process entry, harnesses ---
    ("panel/main.rs::main", Excluded(PROCESS)),
    ("panel/main.rs::parse_args", Excluded(PROCESS)),
    ("panel/app.rs::fail_startup", Excluded(PROCESS)),
    (
        "panel/app.rs::exit_after_work_shutdown",
        Excluded("final process exit after WorkAdmission shutdown; no operator decision"),
    ),
    ("panel/app.rs::init_panel_running", Excluded(PROCESS)),
    ("panel/app.rs::run_panel", Excluded(PROCESS)),
    (
        "panel/app.rs::ui_frame",
        Excluded("the frame driver; its exits are the `--memory` observation harness's PASS/FAIL"),
    ),
    ("panel/headed_record.rs::HeadedRecord::finish", Excluded(HARNESS)),
    ("panel/session.rs::Session::live_prepare_stress", Excluded(HARNESS)),
    ("panel/session.rs::temp_live_vault_from", Excluded(HARNESS)),
    ("panel/session.rs::Session::with_instance", Excluded(STARTUP)),
    (
        "panel/session.rs::Session::pump_status",
        Excluded("per-frame pump: forwards the window's render label and pause preference to the session and polls the host; the decisions are the controls' (SetRenderAll, SetNavPreference)"),
    ),
    // --- panel: vault ---
    (
        "panel/session.rs::Session::request_unlock",
        Rows(
            &[C::UnlockVault, C::CreateVault],
            "the vault prompt's pass: unlock the existing vault or create a new one",
        ),
    ),
    ("panel/session.rs::Session::reset_vault", Row(C::ResetVault)),
    // --- panel: fleet view ---
    ("panel/fleet.rs::window", Row(C::SetFleetFilter)),
    ("panel/fleet.rs::sort_combo", Row(C::SetFleetSort(FleetOrder::Name))),
    // --- panel: session compositions ---
    (
        "panel/session.rs::Session::logout",
        Rows(&[C::CancelQueuedStart, C::Logout], DROP_QUEUED),
    ),
    (
        "panel/session.rs::Session::rail_remove_at",
        Rows(&[C::CancelQueuedStart, C::Remove], DROP_QUEUED),
    ),
    (
        "panel/session.rs::Session::request_focused_memory_relog",
        Rows(&[C::CancelQueuedStart, C::RelogNow], DROP_QUEUED),
    ),
    (
        "panel/session.rs::Session::script_stop",
        Rows(&[C::CancelQueuedStart, C::StopScript], DROP_QUEUED),
    ),
    (
        "panel/session.rs::Session::script_toggle_pause",
        Rows(&[C::PauseScript, C::ResumeScript], RUN_STATE),
    ),
    ("panel/session.rs::Session::cache_tutorial", Row(C::MarkTutorialSkipped)),
    (
        "panel/session.rs::Session::persist_game_render_prefs",
        Row(C::SetRaster(RasterMode::Gpu)),
    ),
    ("panel/session.rs::Session::set_renderer", Row(C::SetDraw(DrawSwitch::GameRenderer, true))),
    ("panel/session.rs::Session::set_capture", Row(C::SetDraw(DrawSwitch::CaptureInput, true))),
    ("panel/session.rs::Session::set_sidecar_50", Row(C::SetDraw(DrawSwitch::SidecarFullRate, true))),
    (
        "panel/session.rs::Session::set_live_full_rate",
        Row(C::SetDraw(DrawSwitch::LiveFullRate, true)),
    ),
    ("panel/app.rs::only_render_selected_clicked", Row(C::SetRenderAll(true))),
    ("panel/session.rs::Session::ack_background_bots", Row(C::AcknowledgeNotice)),
    ("panel/input_capture.rs::KeyboardOwner::capture_keys", Row(C::GameInput)),
    ("panel/session.rs::Session::send_debug_command", Row(C::DebugCommand)),
    (
        "panel/session.rs::Session::send_debug_command_marked_snapshot",
        Row(C::DebugCommand),
    ),
    ("panel/session.rs::Session::script_paint_click", Row(C::ScriptPaintInput)),
    ("panel/session.rs::Session::script_paint_select", Row(C::ScriptPaintInput)),
    // --- panel: map ---
    ("panel/session.rs::Session::confirm_picker_walk", Row(C::WalkTo)),
    ("panel/session.rs::Session::confirm_picker_teleport", Row(C::Teleport)),
    ("panel/session.rs::Session::sync_walk_map", Row(C::OpenMapDemand)),
    ("panel/session.rs::Session::release_walk_map", Row(C::ReleaseMapDemand)),
    (
        "panel/session.rs::Session::accept_map_bake",
        Rows(
            &[C::BakeMapNow, C::OpenMapDemand, C::SetMapBakeChoice(MapBakeChoice::Always)],
            "Bake now: consent, then the reopened images demand that starts the bake; Always bake persists the choice first",
        ),
    ),
    // --- panel: scripts ---
    (
        "panel/app.rs::persist_profile_setting",
        Rows(
            &[
                C::SaveScriptSettings(CardKind::Native),
                C::SaveScriptSettings(CardKind::Script),
                C::SaveLegacyScriptSettings,
            ],
            "the selection's card kind and the focused profile select the row: a native card, a loaded card for the focused profile, or the legacy shared overrides without one",
        ),
    ),
    (
        "panel/app.rs::script_parameter_text_input",
        Rows(
            &[
                C::EditParameterText,
                C::DiscardParameterText,
                C::SaveScriptSettings(CardKind::Script),
                C::SaveScriptSettings(CardKind::Native),
            ],
            "the parameter text field: edits the staged text, discards on Escape, commits on Enter",
        ),
    ),
    (
        "panel/session_catalog.rs::Session::select_script_selection",
        Rows(
            &[C::PickBrowseCard, C::SelectScriptCard, C::PrepareScriptCard],
            "the shared Browse pick; with a focused profile also its pending choice; a loaded card also queues its preparation",
        ),
    ),
    ("panel/session_catalog.rs::Session::enqueue_transpile", Row(C::PrepareScriptCard)),
    ("panel/session_catalog.rs::Session::queue_transpile_all", Row(C::TranspileAll)),
    ("panel/session_catalog.rs::Session::load_js", Row(C::LoadScriptFile)),
    ("panel/session_catalog.rs::Session::defer_rs2b0t_catalog", Row(C::DeferCatalogImport)),
    ("panel/session_catalog.rs::Session::import_rs2b0t_catalog", Row(C::ImportCatalog)),
    (
        "panel/profile_script.rs::Session::begin_script_reload_clicked",
        Rows(&[C::ReloadScripts, C::ConfirmReload], RELOAD_CLICK),
    ),
    // --- panel: rs2b0t import ---
    (
        "panel/app.rs::profile_import_body",
        Rows(
            &[
                C::EditProfileImportPath,
                C::SetProfileImportPolicy(ConflictPolicy::Skip),
                C::SetProfileImportIncludeUnusable(true),
            ],
            "the shared flow owns the edited path and the two preview choices",
        ),
    ),
    (
        "panel/app.rs::profile_import_bar",
        Rows(
            &[
                C::PreviewProfileImport,
                C::CreateProfile,
                C::SaveProfile,
                C::ReturnToProfileImportPath,
                C::DeleteProfileImportExport,
            ],
            "Preview reads and maps; Import creates or saves each planned profile; Back drops the preview; confirmed deletion removes the export",
        ),
    ),
    // --- panel: profile form and chooser ---
    (
        "panel/session/chooser.rs::Session::begin_edit_profile",
        Rows(&[C::OpenProfileForm, C::OpenNewProfileForm], FORM_TARGET),
    ),
    (
        "panel/session/chooser.rs::Session::open_edit_profile",
        Rows(&[C::OpenProfileForm, C::OpenNewProfileForm], FORM_TARGET),
    ),
    (
        "panel/session/chooser.rs::Session::cancel_edit_profile",
        Row(C::CloseProfileForm(FormClose::Form)),
    ),
    (
        "panel/session/chooser.rs::Session::request_cancel_edit",
        Row(C::CloseProfileForm(FormClose::Form)),
    ),
    (
        "panel/session/chooser.rs::Session::request_close_profiles",
        Row(C::CloseProfileForm(FormClose::Window)),
    ),
    (
        "panel/session/chooser.rs::Session::confirm_pending_edit_switch",
        Row(C::DiscardProfileEdits),
    ),
    ("panel/session/chooser.rs::Session::cancel_pending_edit_switch", Row(C::KeepEditing)),
    (
        "panel/session/chooser.rs::Session::save_credentials",
        Rows(
            &[C::CreateProfile, C::SaveProfile, C::SetWorld(WorldChoice::Automatic)],
            "a new row creates the profile; an existing one saves the form, its world mirrored",
        ),
    ),
    // --- panel: loadouts (the shared loadout set lives in `script` today) ---
    ("panel/loadouts.rs::sync_draft", Row(C::SelectLoadout)),
    ("panel/loadouts.rs::save_draft", Row(C::SaveLoadout)),
    ("panel/loadouts.rs::add_loadout", Row(C::CreateLoadout)),
    ("panel/loadouts.rs::duplicate_loadout", Row(C::DuplicateLoadout)),
    ("panel/loadouts.rs::delete_loadout", Row(C::DeleteLoadout)),
    ("panel/loadouts.rs::copy_equipment", Row(C::CopyEquipmentToLoadout)),
    ("panel/loadouts.rs::assign_slot", Row(C::EditLoadoutDraft(LoadoutField::Equipment))),
    ("panel/loadouts.rs::window", Row(C::EditLoadoutDraft(LoadoutField::Name))),
    ("panel/loadouts.rs::draw_supplies", Row(C::EditLoadoutDraft(LoadoutField::Supplies))),
    // --- tui: process entry, harnesses ---
    ("tui/bin.rs::parse_args", Excluded(PROCESS)),
    ("tui/bin.rs::usage", Excluded(PROCESS)),
    (
        "tui/bin.rs::exit_after_work_shutdown",
        Excluded("final process exit after WorkAdmission shutdown; no operator decision"),
    ),
    ("tui/bin.rs::temp_live_vault", Excluded(HARNESS)),
    (
        "tui/bin.rs::map_bundle",
        Excluded("`--map-bundle` packaging flag: bakes the shipped map images; not an operator command"),
    ),
    // --- tui: routers and the pump ---
    (
        "tui/bin.rs::dispatch",
        Rows(
            &[
                C::CancelSettingsSync,
                C::RestartForSettings,
                C::DeferSettingsRestart,
                C::Quit,
                C::ReleaseMapDemand,
            ],
            "the AppAction router: these rows are inlined; every other action calls a TuiSession handler registered on its own",
        ),
    ),
    (
        "tui/input.rs::TuiApp::run_command",
        Rows(
            &[
                C::ClearMarks,
                C::MarkAll,
                C::Quit,
                C::OpenProfileForm,
                C::OpenNewProfileForm,
                C::CloseProfileForm(FormClose::Form),
            ],
            "the palette/command router: clears or stages marks and modals inline (the form notice follows the modal), and returns the AppAction the dispatcher routes",
        ),
    ),
    (
        "tui/input.rs::TuiApp::settings_key",
        Rows(
            &[C::EditProfileDraft(ProfileDraftField::Settings), C::AcknowledgeNotice],
            "the settings popup keys: stage a changed row for persist_settings and clear the quester notice",
        ),
    ),
    ("tui/input.rs::TuiApp::fleet_filter_key", Row(C::SetFleetFilter)),
    (
        "tui/bin.rs::TuiSession::pump",
        Rows(
            &[
                C::SelectScriptCard,
                C::Select,
                C::SetRenderAll(true),
                C::OpenProfileForm,
                C::OpenNewProfileForm,
                C::CloseProfileForm(FormClose::Form),
            ],
            "the frame pump: adopts the Browse pick as the focused profile's pending choice; focus, the render label and the form notice follow the app state; the exits are the `--memory` harness's",
        ),
    ),
    // --- tui: session compositions ---
    (
        "tui/bin.rs::TuiSession::load_and_login",
        Rows(&[C::Load, C::Login], "Load then Log in"),
    ),
    (
        "tui/bin.rs::TuiSession::load_and_login_all",
        Rows(&[C::Load, C::Login], "Load then Log in, every profile"),
    ),
    (
        "tui/bin.rs::TuiSession::logout",
        Rows(&[C::CancelQueuedStart, C::Logout], DROP_QUEUED),
    ),
    (
        "tui/bin.rs::TuiSession::memory_relog_now",
        Rows(&[C::CancelQueuedStart, C::RelogNow], DROP_QUEUED),
    ),
    (
        "tui/bin.rs::TuiSession::remove",
        Rows(&[C::CancelQueuedStart, C::Remove], DROP_QUEUED),
    ),
    (
        "tui/bin.rs::TuiSession::script_stop",
        Rows(&[C::CancelQueuedStart, C::StopScript], DROP_QUEUED),
    ),
    (
        "tui/bin.rs::TuiSession::script_toggle_pause",
        Rows(&[C::PauseScript, C::ResumeScript], RUN_STATE),
    ),
    (
        "tui/bin.rs::TuiSession::script_start",
        Rows(
            &[C::SelectScriptCard, C::StartScript],
            "Start: records the Browse choice as the profile's pending selection, then starts",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::persist_settings",
        Rows(
            &[C::SetMemoryMode(MemoryMode::Low), C::SetGuardian, C::SaveProfile],
            "the settings popup's profile rows: guardian fields and memory mode, written through the profile row",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::script_reload",
        Rows(&[C::ReloadScripts, C::ConfirmReload], RELOAD_CLICK),
    ),
    ("tui/bin.rs::TuiSession::script_load", Row(C::LoadScriptFile)),
    ("tui/bin.rs::TuiSession::defer_rs2b0t_catalog", Row(C::DeferCatalogImport)),
    ("tui/bin.rs::TuiSession::import_rs2b0t_catalog", Row(C::ImportCatalog)),
    (
        "panel/session_catalog.rs::Session::settle_catalog_import",
        Excluded("settles a completed catalog import and clears its deferred prompt marker; the Import Catalog decision was made when the import was requested"),
    ),
    (
        "tui/bin.rs::TuiSession::profile_import",
        Rows(
            &[
                C::ImportCatalog,
                C::EditProfileImportPath,
                C::PreviewProfileImport,
                C::SetProfileImportPolicy(ConflictPolicy::Skip),
                C::SetProfileImportIncludeUnusable(true),
                C::CreateProfile,
                C::SaveProfile,
                C::ReturnToProfileImportPath,
                C::SetProfileImportDeletePrompt(true),
                C::DeleteProfileImportExport,
            ],
            "the Preview action may fill the catalog; its branches edit the path, choose preview options, confirm profile writes, return to the path, stage or cancel deletion, and confirm export deletion",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::params_key",
        Rows(
            &[
                C::EditParameterText,
                C::DiscardParameterText,
                C::SaveScriptSettings(CardKind::Script),
                C::SaveScriptSettings(CardKind::Native),
            ],
            "the parameters popup keys: edit, discard or commit the staged text",
        ),
    ),
    ("tui/app.rs::TuiApp::move_script_sel", Row(C::PickBrowseCard)),
    ("tui/app.rs::TuiApp::script_click", Row(C::PickBrowseCard)),
    (
        "tui/bin.rs::TuiSession::poll_scripts",
        Rows(
            &[C::ImportCatalog, C::PreviewProfileImport],
            "polls the requested catalog import, clears its deferred prompt marker on success, and previews the profile import when catalog preparation completes",
        ),
    ),
    // --- tui: map, input ---
    ("tui/bin.rs::TuiSession::open_map_catalogue", Row(C::OpenMapDemand)),
    ("tui/bin.rs::TuiSession::release_map_catalogue", Row(C::ReleaseMapDemand)),
    ("tui/bin.rs::TuiSession::arm_walk_on", Row(C::WalkTo)),
    ("tui/bin.rs::TuiSession::map_teleport", Row(C::Teleport)),
    ("tui/bin.rs::TuiSession::wasd_walk", Row(C::GameInput)),
    (
        "tui/bin.rs::TuiSession::chat_send",
        Rows(
            &[C::ScriptPaintInput, C::GameInput],
            "a paint answer (1-9, Enter) or a dialogue Continue/Answer",
        ),
    ),
    ("tui/bin.rs::TuiSession::ack_background_bots", Row(C::AcknowledgeNotice)),
    // --- tui: loadouts ---
    ("tui/loadouts.rs::LoadoutsPane::commit_scratch", Row(C::SaveLoadout)),
    (
        "tui/loadouts.rs::LoadoutsPane::on_key",
        Rows(
            &[
                C::SelectLoadout,
                C::EditLoadoutDraft(LoadoutField::Unassigned),
                C::SaveLoadout,
                C::CreateLoadout,
                C::DuplicateLoadout,
                C::DeleteLoadout,
            ],
            "the loadouts popup: row 0 selects, rows 1-4 edit the scratch text and commit on Enter, rows 5-8 create, duplicate, delete and save",
        ),
    ),
];

const PROCESS: &str =
    "process entry, argument errors or start-up failure exits; not an operator command";
const HARNESS: &str = "`--live`/headed harness preparation and records; not an operator command";
const DROP_QUEUED: &str = "drops the member's queued Start first, then issues the row";
const RUN_STATE: &str = "the run's state selects the row";
const RELOAD_CLICK: &str = "the first click begins the reload; while a warning awaits confirmation the same click confirms it";
const FORM_TARGET: &str = "the target selects the row: an existing profile or the blank new row";

/// A `cfg` that keeps an item out of production builds.
fn is_test_cfg(cfg: &str) -> bool {
    cfg.contains("test")
}

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/frontend-core sits two levels below the workspace root")
        .to_path_buf()
}

fn same_row(a: OperatorCommand, b: OperatorCommand) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

/// The rows a decision names.
fn rows_of(decision: Decision) -> Vec<OperatorCommand> {
    match decision {
        Row(row) => vec![row],
        Rows(rows, _) => rows.to_vec(),
        Excluded(_) => Vec::new(),
    }
}

fn surface_key(item: &scan::SurfaceFn) -> String {
    display_surface(item)
}

/// Whether a core entry is subject to the register at all: reachable, in
/// production builds, and not a constructor.
fn in_universe(entry: &CoreEntry) -> bool {
    entry.reachable && !entry.returns_self && !entry.cfg.as_deref().is_some_and(is_test_cfg)
}

fn census() -> Census {
    let census = scan(&workspace_root());
    assert!(
        census.errors.is_empty(),
        "the census scan reported errors:\n{}",
        census.errors.join("\n")
    );
    assert!(
        census.core.len() > 100 && census.surface.len() > 500,
        "implausibly small scan: {} core entries, {} surface fns",
        census.core.len(),
        census.surface.len()
    );
    census
}

/// Every reachable operator entry point of this crate has exactly one
/// registered decision, every registered key still exists, and the register
/// names nothing the class rules already exclude.
#[test]
fn every_core_entry_point_has_exactly_one_row_or_a_named_exclusion() {
    let census = census();
    let mut problems = Vec::new();
    let mut register = BTreeMap::new();
    for (key, decision) in CORE {
        if register.insert(*key, *decision).is_some() {
            problems.push(format!("{key}: registered twice"));
        }
    }
    let found: BTreeMap<&str, &CoreEntry> = census
        .core
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect();
    for key in register.keys() {
        match found.get(key) {
            None => problems.push(format!(
                "{key}: registered, but no such entry point (renamed, removed or never public)"
            )),
            Some(entry) if !in_universe(entry) => problems.push(format!(
                "{key}: registered, but excluded by class (reachable {}, constructor {}, cfg {:?})",
                entry.reachable, entry.returns_self, entry.cfg
            )),
            Some(_) => {}
        }
    }
    for entry in census.core.iter().filter(|entry| in_universe(entry)) {
        if !register.contains_key(entry.key.as_str()) {
            problems.push(format!(
                "{}:{} {}: operator entry point without a row or a named exclusion",
                entry.file, entry.line, entry.key
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// Every row of the vocabulary is reached by a registered core entry point
/// or a surface handler, except the normative-only rows; and every row a
/// register names is a row of [`OperatorCommand::ALL`].
#[test]
fn every_row_is_reached_by_an_entry_point() {
    let census = census();
    let core: BTreeMap<&str, Decision> = CORE.iter().copied().collect();
    let surface: BTreeMap<&str, Decision> = SURFACE.iter().copied().collect();
    let mut reached: Vec<OperatorCommand> = Vec::new();
    for decision in CORE
        .iter()
        .map(|(_, d)| *d)
        .chain(SURFACE.iter().map(|(_, d)| *d))
    {
        reached.extend(rows_of(decision));
    }
    let unlisted: Vec<OperatorCommand> = reached
        .iter()
        .chain(NORMATIVE_ONLY)
        .copied()
        .filter(|row| !OperatorCommand::ALL.iter().any(|r| same_row(*r, *row)))
        .collect();
    assert!(
        unlisted.is_empty(),
        "rows the registers name but ALL does not list: {unlisted:?}"
    );
    for item in &census.surface {
        if surface.contains_key(surface_key(item).as_str()) {
            continue;
        }
        for call in &item.core_calls {
            if let Some(decision) = core.get(call.key.as_str()) {
                reached.extend(rows_of(*decision));
            }
        }
    }
    let unreached: Vec<OperatorCommand> = OperatorCommand::ALL
        .into_iter()
        .filter(|row| !NORMATIVE_ONLY.iter().any(|n| same_row(*n, *row)))
        .filter(|row| !reached.iter().any(|r| same_row(*r, *row)))
        .collect();
    assert!(
        unreached.is_empty(),
        "rows no entry point issues: {unreached:?}"
    );
    for row in NORMATIVE_ONLY {
        assert!(
            !reached.iter().any(|r| same_row(*r, *row)),
            "{row:?} is listed as normative-only but an entry point issues it"
        );
    }
}

/// Every panel/TUI function that calls into this crate or hits an effect
/// probe is one row's handler, or is registered as a composition, a
/// dispatcher, an outside-core decision or an exclusion; every registered
/// surface key still names a function.
#[test]
fn every_surface_handler_is_one_rows_or_registered() {
    let census = census();
    let core: BTreeMap<&str, Decision> = CORE.iter().copied().collect();
    let mut problems = Vec::new();
    let mut register = BTreeMap::new();
    for (key, decision) in SURFACE {
        if register.insert(*key, *decision).is_some() {
            problems.push(format!("{key}: registered twice"));
        }
    }
    let keys: Vec<String> = census.surface.iter().map(surface_key).collect();
    for key in register.keys() {
        if !keys.iter().any(|k| k == key) {
            problems.push(format!("{key}: registered, but no such panel/TUI function"));
        }
    }
    for item in &census.surface {
        let key = surface_key(item);
        // Per core call, the rows it can issue (a dispatching entry point
        // lists several); excluded entry points issue none.
        let mut calls: Vec<(u32, &str, Vec<OperatorCommand>)> = Vec::new();
        for call in &item.core_calls {
            match core.get(call.key.as_str()) {
                Some(decision) => {
                    let rows = rows_of(*decision);
                    if !rows.is_empty() {
                        calls.push((call.line, call.key.as_str(), rows));
                    }
                }
                None if census.core.iter().any(|e| e.key == call.key) => {}
                None => problems.push(format!(
                    "{key}:{} calls {} which the core register does not know",
                    call.line, call.key
                )),
            }
        }
        let issues = |rows: &[OperatorCommand], alternatives: &[OperatorCommand]| {
            alternatives
                .iter()
                .any(|a| rows.iter().any(|r| same_row(*r, *a)))
        };
        match register.get(key.as_str()) {
            Some(Row(row)) => {
                for (line, callee, alternatives) in &calls {
                    if !issues(std::slice::from_ref(row), alternatives) {
                        problems.push(format!(
                            "{key}: registered as {row:?} but line {line} calls {callee} ({alternatives:?})"
                        ));
                    }
                }
            }
            Some(Rows(rows, _)) => {
                for (line, callee, alternatives) in &calls {
                    if !issues(rows, alternatives) {
                        problems.push(format!(
                            "{key}: registered rows {rows:?} miss line {line} calling {callee} ({alternatives:?})"
                        ));
                    }
                }
            }
            Some(Excluded(_)) => {}
            None => {
                // Unregistered: every call must name exactly one row (a
                // parameter value is not a different row), all the same
                // one, and no probe may fire.
                let mut single: Option<OperatorCommand> = None;
                let mut plain = true;
                for (_, _, alternatives) in &calls {
                    let first = alternatives[0];
                    let one_row = alternatives.iter().all(|a| same_row(*a, first));
                    match (one_row, single) {
                        (true, None) => single = Some(first),
                        (true, Some(seen)) if same_row(first, seen) => {}
                        _ => plain = false,
                    }
                }
                if calls.is_empty() && item.probes.is_empty() {
                    continue;
                }
                if !plain || single.is_none() || !item.probes.is_empty() {
                    let summary: Vec<String> = calls
                        .iter()
                        .map(|(line, callee, rows)| format!("{line}: {callee} {rows:?}"))
                        .collect();
                    problems.push(format!(
                        "{}:{} {key}: calls [{}] and hits probes {:?}: register it",
                        item.file,
                        item.line,
                        summary.join("; "),
                        item.probes
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// The census as a table, for the report: `cargo test -p frontend-core
/// --lib -- command::census --nocapture`.
#[test]
fn print_the_census_table() {
    let census = census();
    let core: BTreeMap<&str, Decision> = CORE.iter().copied().collect();
    let surface: BTreeMap<&str, Decision> = SURFACE.iter().copied().collect();
    let mut by_row: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for row in OperatorCommand::ALL {
        by_row.insert(format!("{row:?}"), (Vec::new(), Vec::new()));
    }
    let name_of = |row: OperatorCommand| {
        OperatorCommand::ALL
            .iter()
            .find(|r| same_row(**r, row))
            .map(|r| format!("{r:?}"))
            .expect("row in ALL")
    };
    for (key, decision) in CORE {
        for row in rows_of(*decision) {
            let entries = &mut by_row.get_mut(&name_of(row)).expect("row").0;
            if !entries.iter().any(|e| e == key) {
                entries.push((*key).to_string());
            }
        }
    }
    let mut excluded_core: Vec<String> = Vec::new();
    let mut dispatchers: Vec<String> = Vec::new();
    let mut by_class: Vec<String> = Vec::new();
    for entry in &census.core {
        if !in_universe(entry) {
            by_class.push(format!(
                "{} ({:?}; reachable {}, constructor {}, cfg {})",
                entry.key,
                entry.receiver,
                entry.reachable,
                entry.returns_self,
                entry.cfg.as_deref().unwrap_or("-")
            ));
        } else {
            match core.get(entry.key.as_str()) {
                Some(Excluded(why)) => {
                    excluded_core.push(format!("{} ({:?}): {why}", entry.key, entry.receiver));
                }
                Some(Rows(rows, why)) => {
                    dispatchers.push(format!("{}: {rows:?}: {why}", entry.key));
                }
                _ => {}
            }
        }
    }
    for (key, decision) in SURFACE {
        if let Rows(rows, why) = decision {
            dispatchers.push(format!("{key}: {rows:?}: {why}"));
        }
    }
    let mut excluded_surface: Vec<String> = Vec::new();
    for item in &census.surface {
        let key = surface_key(item);
        let decision = surface.get(key.as_str()).copied();
        let mut rows: Vec<OperatorCommand> = Vec::new();
        match decision {
            Some(Excluded(why)) => {
                excluded_surface.push(format!("{key}: {why}"));
                continue;
            }
            Some(d) => rows.extend(rows_of(d)),
            None => {
                for call in &item.core_calls {
                    if let Some(d) = core.get(call.key.as_str()) {
                        rows.extend(rows_of(*d));
                    }
                }
            }
        }
        for row in rows {
            let handlers = &mut by_row.get_mut(&name_of(row)).expect("row").1;
            if !handlers.contains(&key) {
                handlers.push(key.clone());
            }
        }
    }
    println!("CENSUS-TABLE-BEGIN");
    for (row, (entries, handlers)) in &by_row {
        println!(
            "ROW\t{row}\t{}\t{}",
            entries.join(", "),
            handlers.join(", ")
        );
    }
    for line in &dispatchers {
        println!("DISPATCH\t{line}");
    }
    for line in &excluded_core {
        println!("EXCLUDED-CORE\t{line}");
    }
    for line in &by_class {
        println!("EXCLUDED-BY-CLASS\t{line}");
    }
    for line in &excluded_surface {
        println!("EXCLUDED-SURFACE\t{line}");
    }
    for line in &census.notes {
        println!("NOTE\t{line}");
    }
    println!(
        "COUNTS\trows {}\tcore entries {} (in universe {})\tsurface fns {} (with core calls or probes {})",
        OperatorCommand::ALL.len(),
        census.core.len(),
        census.core.iter().filter(|e| in_universe(e)).count(),
        census.surface.len(),
        census
            .surface
            .iter()
            .filter(|f| !f.core_calls.is_empty() || !f.probes.is_empty())
            .count()
    );
    println!("CENSUS-TABLE-END");
}

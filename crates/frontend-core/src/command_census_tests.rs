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
//!   one registered entry.
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
    /// Stages an input this row commits (a draft buffer, a staged edit):
    /// it changes only the staged model, and the surface fn that calls it
    /// must also submit the row.
    Staged(OperatorCommand),
    /// Stages an input that one of these rows commits (a staging input
    /// shared by several rows): why. The surface fn that calls it must
    /// also submit one of them.
    StagedFor(&'static [OperatorCommand], &'static str),
}

use Decision::{Excluded, Row, Rows, Staged, StagedFor};
use OperatorCommand as C;

const PUMP: &str = "pump: adopts owner results and refreshes projections; no operator decision";
const RESULT: &str = "result consumption: takes settled owner results for display";
const PROJECTION: &str = "projection copy into the front end's buffer; presentation";
const MAP_DRAFT: &str =
    "map selection/search is a draft; WalkTo/TeleportTo dispatch occurs only on confirmation";
const GAME_INPUT_STAGING: &str =
    "stages GameInput events into the core queue only (no dispatch); `ui_frame` submits GameInput once per frame";
const FORMAT: &str = "pure formatting or parsing; no state";
const UI_TIMER: &str =
    "presentation timer: clears a front end's own message line after a timeout; no operator decision";
const LOG_VIEW: &str =
    "control of the log read projection (scope, filter, follow); not transport, work admission or a durable write";
const SEAM_ROWS: &str = "the command seam: every row, chosen by its literal argument";
const STARTUP: &str = "startup wiring of a store path or adapter; no operator decision";
const ACCESSOR: &str = "accessor exposing an owned model (T3.1 accessor migration), not a decision";
const PATH_READ: &str = "resolves an operator file location; no state change";
const SETTINGS_KIND: &str = "the card kind of the selection decides the row";
const RELEASE_BUNDLE: &str =
    "release packaging of the shipped map bundle; no interactive operator decision";

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
        "fleet.rs::fill_visible_row_indices",
        Excluded("pure computation of the shown rows from the staged filter text and order"),
    ),
    // group_walk.rs
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
    // log_file.rs
    (
        "log_file.rs::session_log_setting",
        Excluded("reads the persisted setting"),
    ),
    // map_bake.rs
    (
        "map_bake.rs::bake_shipped_map_bundle",
        Excluded(RELEASE_BUNDLE),
    ),
    (
        "map_bake.rs::load_map_bake_choice",
        Excluded("reads the persisted choice"),
    ),
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
    (
        "marked.rs::prepare_apply_settings_shown",
        Row(C::PlanFleetAction(crate::fleet_plan::FleetAction::CopySettings)),
    ),
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
    // profile_form.rs: the form-save machine.
    // profile_draft.rs: the staged form draft's edit buffers.
    ("profile_draft.rs::ProfileDraft::username_mut", Staged(C::EditProfileDraft(ProfileDraftField::Username))),
    ("profile_draft.rs::ProfileDraft::password_mut", Staged(C::EditProfileDraft(ProfileDraftField::Password))),
    ("profile_draft.rs::ProfileDraft::settings_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::lowmem_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::auto_login_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::world_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::random_events_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::lamp_skill_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::lamp_auto_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::clue_duel_partner_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::set_random_event_policy", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SettingsDraft::survival_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SurvivalDraft::set_cure_poison", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_draft.rs::SurvivalSettings::cure_poison_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    // profile_editor.rs: the profile edit form (moved from the panel chooser).
    ("profile_editor.rs::ProfileEditor::draft_mut", Staged(C::EditProfileDraft(ProfileDraftField::Settings))),
    ("profile_editor.rs::ProfileEditor::username_mut", Staged(C::EditProfileDraft(ProfileDraftField::Username))),
    ("profile_editor.rs::ProfileEditor::password_mut", Staged(C::EditProfileDraft(ProfileDraftField::Password))),
    (
        "profile_editor.rs::ProfileEditor::settle",
        Excluded("adopts the durable writer's results (SaveProfile's Owner terminal)"),
    ),
    ("resources.rs::background_bots_acked", Excluded("reads the background-bots acknowledgement; no state change")),
    ("resources.rs::clear_background_bots_ack_error", Excluded("drops a stale banner line; presentation")),
    ("session.rs::OperatorSession::open_process_state", Excluded(STARTUP)),
    ("session/commands.rs::OperatorSession::stage_new_profile", Staged(C::CreateProfile)),
    ("session/commands.rs::OperatorSession::stage_guardian", Staged(C::SetGuardian)),
    ("session/commands.rs::OperatorSession::stage_survival", Staged(C::SetSurvival)),
    (
        "surface.rs::SpawnProfile::set_lowmem",
        Excluded("a surface adjusts its disposable spawn copy (session-only memory override); never written back"),
    ),
    (
        "surface.rs::SpawnProfile::set_raster",
        Excluded("a surface adjusts its disposable spawn copy (headless raster off); never written back"),
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
    ("error_expiry.rs::ErrorExpiry::observe", Excluded(UI_TIMER)),
    ("error_expiry.rs::ErrorExpiry::waits_for", Excluded(UI_TIMER)),
    ("error_expiry.rs::ErrorExpiry::resolve", Excluded(UI_TIMER)),
    (
        "session/commands.rs::OperatorSession::refuse_unsubmitted",
        Excluded("logs and counts a front end's refusal of a command it could not submit; changes no operator state"),
    ),
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
        "rs2b0t_import.rs::ProfileImport::path_mut",
        Staged(C::EditProfileImportPath),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::prepare_confirmation",
        Excluded("prepares the confirmed import report before its account writer rows; no profile is written here"),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::stage_create_account",
        Staged(C::CreateProfile),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::stage_save_account",
        Staged(C::SaveProfile),
    ),
    (
        "rs2b0t_import.rs::ProfileImport::cancel_staged_account",
        Excluded("records an import writer submit that did not dispatch; no profile write"),
    ),
    // quester_paths.rs
    (
        "quester_paths.rs::QuesterPathsController::restore",
        Excluded("startup restore of the persisted settings"),
    ),
    (
        "quester_paths.rs::QuesterPathsController::advance",
        Excluded(PUMP),
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
        "scripts/mod.rs::Scripts::assignment_name",
        Excluded("reads the profile's assignment display name"),
    ),
    ("scripts/mod.rs::Scripts::stage_cancel_queued", Staged(C::CancelQueuedStart)),
    ("scripts/mod.rs::Scripts::stage_browse_pick", Staged(C::PickBrowseCard)),
    ("scripts/mod.rs::Scripts::stage_browse_pick_clear", Staged(C::PickBrowseCard)),
    ("scripts/mod.rs::Scripts::stage_load_file", Staged(C::LoadScriptFile)),
    ("scripts/mod.rs::Scripts::stage_paint_input", Staged(C::ScriptPaintInput)),
    (
        "scripts/mod.rs::Scripts::stage_setting",
        StagedFor(
            &[
                C::SaveScriptSettings(CardKind::Native),
                C::SaveScriptSettings(CardKind::Script),
                C::SaveLegacyScriptSettings,
            ],
            "the staged profile and card kind choose the settings save row",
        ),
    ),
    (
        "scripts/mod.rs::Scripts::stage_parameter_validation",
        StagedFor(
            &[
                C::EditParameterText,
                C::SaveScriptSettings(CardKind::Native),
                C::SaveScriptSettings(CardKind::Script),
                C::SaveLegacyScriptSettings,
            ],
            "a draft text edit validates; Enter or deactivation commits the validated value",
        ),
    ),
    ("scripts/mod.rs::Scripts::stage_catalog_action", Staged(C::ImportCatalog)),
    ("scripts/mod.rs::Scripts::stage_defer_catalog", Staged(C::DeferCatalogImport)),
    (
        "scripts/mod.rs::Scripts::stage_start_catalog_root",
        StagedFor(
            &[
                C::StartScript,
                C::AssignScript,
                C::AssignAndRestart,
                C::RestartForSettings,
                C::PlanFleetAction(crate::fleet_plan::FleetAction::Start),
            ],
            "the Start, assignment, assignment-and-restart or settings-restart row consumes the staged catalog root; a fleet plan freezes it for the row that runs the plan",
        ),
    ),
    (
        "scripts/mod.rs::Scripts::stage_reload_target",
        StagedFor(&[C::ReloadScripts, C::ConfirmReload], RELOAD_CLICK),
    ),
    ("scripts/mod.rs::Scripts::stage_transpile_target", Staged(C::PrepareScriptCard)),
    (
        "scripts/mod.rs::Scripts::stage_parameter_discard",
        Staged(C::DiscardParameterText),
    ),
    (
        "scripts/mod.rs::Scripts::stage_profile_parameter_discard",
        Staged(C::DiscardParameterText),
    ),
    (
        "scripts/sync.rs::Scripts::stage_settings_sync_prepare",
        StagedFor(
            &[
                C::ApplySettingsToMarked(CardKind::Native),
                C::ApplySettingsToMarked(CardKind::Script),
            ],
            "the selected card kind chooses the preparation row",
        ),
    ),
    (
        "scripts/sync.rs::Scripts::stage_settings_sync_apply",
        StagedFor(
            &[
                C::ApplySettingsToMarked(CardKind::Native),
                C::ApplySettingsToMarked(CardKind::Script),
            ],
            "the frozen prepared card kind chooses the apply row",
        ),
    ),
    // scripts/vocab.rs, scripts/paint.rs: script vocabulary and paint views.
    ("scripts/vocab.rs::format_setting_value", Excluded(FORMAT)),
    ("scripts/vocab.rs::OwnedSchema::copy_from", Excluded(PROJECTION)),
    ("scripts/vocab.rs::OwnedSchema::clear", Excluded(PROJECTION)),
    ("scripts/paint.rs::map_applet_rect", Excluded(FORMAT)),
    ("scripts/paint.rs::CanvasCache::store", Excluded(PROJECTION)),
    ("scripts/paint.rs::CanvasCache::clear", Excluded(PROJECTION)),
    (
        "scripts/paint_layout.rs::PaintLayout::compute",
        Excluded(PROJECTION),
    ),
    ("scripts/paint_layout.rs::clipped_prefix", Excluded(FORMAT)),
    ("scripts/paint_style.rs::parse_accent", Excluded(FORMAT)),
    // scripts/paths.rs: operator file locations.
    ("scripts/paths.rs::bot_home", Excluded(PATH_READ)),
    ("scripts/paths.rs::bot_file", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_path_file", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_import_file", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_root", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_root_checked", Excluded(PATH_READ)),
    ("scripts/paths.rs::registry_index_path", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_import_deferred", Excluded(PATH_READ)),
    ("scripts/paths.rs::rs2b0t_import_deferred_at", Excluded(PATH_READ)),
    (
        "scripts/paths.rs::set_rs2b0t_import_deferred_at",
        Row(C::DeferCatalogImport),
    ),
    (
        "scripts/paths.rs::clear_rs2b0t_import_at",
        Excluded("clears the deferral marker when a catalogue import settles; the decision was ImportCatalog"),
    ),
    // launch.rs: the server-profile binding, instance lock and passphrase.
    ("launch.rs::ProfileLauncher::configure", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::bind", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::bind_with", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::bump_generation", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::finish_preparation", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::install_validated", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::set_preparing", Excluded(STARTUP)),
    ("launch.rs::ProfileLauncher::take_launch", Excluded(STARTUP)),
    ("launch.rs::PlayConfig::set_mainland", Excluded(STARTUP)),
    (
        "launch.rs::Passphrase::clear",
        Excluded("zeroes the typed passphrase buffer; presentation"),
    ),
    ("launch.rs::acquire_instance", Excluded(PROCESS)),
    ("launch.rs::obtain_passphrase", Excluded(STARTUP)),
    ("launch.rs::warn_legacy_passphrase_env", Excluded(PROCESS)),
    ("launch.rs::parse_profile_args", Excluded(FORMAT)),
    ("launch.rs::is_removed_passphrase_flag", Excluded(FORMAT)),
    ("launch.rs::removed_passphrase_flag_error", Excluded(FORMAT)),
    ("launch.rs::debug_teleport_authorized", Excluded(FORMAT)),
    ("launch.rs::default_scatter_tile_for", Excluded(FORMAT)),
    ("process.rs::debug_enabled", Excluded(PROCESS)),
    ("process.rs::bootstrap", Excluded(PROCESS)),
    ("process.rs::raise_fd_limit", Excluded(PROCESS)),
    ("process.rs::shutdown_background_work", Excluded(PROCESS)),
    // bench.rs: the opt-in memory benchmark's run and records.
    ("bench.rs::BenchRun::poll", Excluded(BENCH)),
    ("bench.rs::mark_startup", Excluded(BENCH)),
    ("bench.rs::record_adapter", Excluded(BENCH)),
    ("bench.rs::require_live_benchmark", Excluded(BENCH)),
    // live/mod.rs: headed harness preparation and proof reporting.
    ("live/mod.rs::WatchDecision::pass_hold", Excluded(HARNESS)),
    ("live/mod.rs::RunnerHandle::stage_loaded_card", Excluded(HARNESS)),
    ("live/mod.rs::Scenario::take_settings_overrides", Excluded(HARNESS)),
    ("live/mod.rs::Scenario::is_registered", Excluded(HARNESS)),
    ("live/mod.rs::default_fixture_path", Excluded(HARNESS)),
    ("live/mod.rs::live_file_fixture", Excluded(HARNESS)),
    ("live/mod.rs::screen_name", Excluded(HARNESS)),
    ("live/mod.rs::profile_password", Excluded(HARNESS)),
    ("live/mod.rs::null_raster_entries", Excluded(HARNESS)),
    ("live/mod.rs::stress_entries", Excluded(HARNESS)),
    ("live/mod.rs::panel_live_vault", Excluded(HARNESS)),
    ("live/mod.rs::budget_s_from_env", Excluded(HARNESS)),
    ("live/mod.rs::shot_stamp_utc", Excluded(HARNESS)),
    ("live/mod.rs::create_shot_run_dir", Excluded(HARNESS)),
    ("live/mod.rs::write_shot", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_scenario_name", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_live_name", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_terminal_shot", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_prereq_shot", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_script_name", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_nothing_changed", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_frozen_sha256", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_bones_id", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_prayer_stat_id", Excluded(HARNESS)),
    ("live/mod.rs::external_loader_bones_count", Excluded(HARNESS)),
    ("live/mod.rs::resolve_external_loader_source", Excluded(HARNESS)),
    ("live/mod.rs::materialize_external_loader_source", Excluded(HARNESS)),
    ("live/mod.rs::count_tcp_to", Excluded(HARNESS)),
    ("live/mod.rs::process_rss_kib", Excluded(HARNESS)),
    ("live/mod.rs::OperatorSession::pump_live_external_loader", Excluded(HARNESS)),
    ("live/mod.rs::OperatorSession::prepare_live_stress_profiles", Excluded(HARNESS)),
    ("slot_view.rs::StatusBuf::refresh", Excluded(PROJECTION)),
    ("surface.rs::map_image_to_applet", Excluded(FORMAT)),
    ("surface.rs::pump_render", Excluded(PUMP)),
    // scripts/loadouts.rs: the loadout form's staged inputs; the loadout rows commit them.
    ("scripts/loadouts.rs::LoadoutForm::stage_selection", Staged(C::SelectLoadout)),
    (
        "scripts/loadouts.rs::LoadoutForm::draft_mut",
        Staged(C::EditLoadoutDraft(LoadoutField::Name)),
    ),
    ("scripts/loadouts.rs::LoadoutForm::stage_draft", Staged(C::SaveLoadout)),
    ("scripts/mod.rs::Scripts::take_notice", Excluded(RESULT)),
    (
        "scripts/mod.rs::Scripts::show_load_failures",
        Excluded("presentation: shows the library's load failures on the banner once a Start is accepted"),
    ),
    ("scripts/mod.rs::Scripts::take_start_failures", Excluded(RESULT)),
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
        "scripts/mod.rs::Scripts::profile_overrides",
        Excluded("read of the profile's parameter bag; its first use of a card claims the legacy overrides into the profile (`claim_legacy_for`), a migration write with no operator decision beyond showing the parameters"),
    ),
    (
        "scripts/mod.rs::Scripts::merged_profile_bag",
        Excluded("read of the bag a Start receives; same first-use legacy claim as profile_overrides"),
    ),
    (
        "scripts/mod.rs::Scripts::load_live_example_identity",
        Excluded(HARNESS),
    ),
    (
        "scripts/mod.rs::Scripts::set_inject",
        Excluded("harness inject bag; no operator decision"),
    ),
    ("scripts/mod.rs::Scripts::poll", Excluded(PUMP)),
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
        Staged(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::parameter_text_mut_for",
        Staged(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::validate_parameter_buffer",
        Row(C::EditParameterText),
    ),
    (
        "scripts/parameter_edit.rs::Scripts::validate_parameter_buffer_for",
        Row(C::EditParameterText),
    ),
    // scripts/reload.rs
    ("scripts/reload.rs::Scripts::take_reload_outcome", Excluded(RESULT)),
    // selection.rs: marks and the marked bulk commands.
    ("selection.rs::stop_marked", Row(C::StopScript)),
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
    (
        "session.rs::OperatorSession::bind_local_state",
        Excluded(STARTUP),
    ),
    (
        "session.rs::OperatorSession::copy_statuses_into",
        Excluded(PROJECTION),
    ),
    ("session.rs::OperatorSession::take_settled_starts", Excluded(RESULT)),
    // session/map.rs: the map desk stages row inputs and adopts owner results.
    ("session/map.rs::MapDesk::clear_walk_dest", Excluded(PROJECTION)),
    (
        "session/map.rs::MapDesk::adopt_bake_choice",
        Excluded("adopts the process writer's ready preference without saving it"),
    ),
    (
        "session/map.rs::MapDesk::note_ready_without_terrain",
        Excluded("adopts the ready demand's result (no terrain) into the prompt: owner result"),
    ),
    ("session/map.rs::MapDesk::stage_walk_to", Staged(C::WalkTo)),
    ("session/map.rs::MapDesk::stage_group_walk", Staged(C::GroupWalk)),
    ("session/map.rs::MapDesk::stage_teleport", Staged(C::Teleport)),
    ("session/map.rs::MapDesk::stage_open_demand", Staged(C::OpenMapDemand)),
    ("session/map.rs::MapDesk::stage_bake_now", Staged(C::BakeMapNow)),
    ("session/map.rs::MapDesk::stage_debug_command", Staged(C::DebugCommand)),
    (
        "session/map.rs::MapDesk::stage_internal_debug_command",
        Staged(C::DebugCommand),
    ),
    (
        "session/map.rs::MapDesk::begin_game_input_frame",
        Staged(C::GameInput),
    ),
    ("session/map.rs::MapDesk::stage_game_input", Staged(C::GameInput)),
    (
        "session/map.rs::MapDesk::stage_captured_input_to",
        Staged(C::GameInput),
    ),
    (
        "session/map.rs::MapDesk::set_input_sender",
        Excluded("device plumbing: attaches the capture channel; no operator decision"),
    ),
    (
        "session/map.rs::OperatorSession::poll_map_demand",
        Excluded(PUMP),
    ),
    // session/commands.rs: the command seam (T3.3). The row is the literal
    // argument; surface calls are checked through their row names.
    ("session/commands.rs::OperatorSession::submit", Rows(&OperatorCommand::ALL, SEAM_ROWS)),
    ("session/commands.rs::OperatorSession::submit_at", Rows(&OperatorCommand::ALL, SEAM_ROWS)),
    (
        "session/commands.rs::OperatorSession::submit_detached",
        Rows(&OperatorCommand::ALL, SEAM_ROWS),
    ),
    (
        "session/commands.rs::OperatorSession::set_focus_hint",
        Staged(C::Load),
    ),
    ("driver.rs::HeadlessDriver::submit", Rows(&OperatorCommand::ALL, SEAM_ROWS)),
    ("driver.rs::HeadlessDriver::poll", Excluded(PUMP)),
    ("driver.rs::HeadlessDriver::poll_at", Excluded(PUMP)),
    ("session.rs::OperatorSession::poll", Excluded(PUMP)),
    ("session.rs::OperatorSession::prepare_startup", Excluded(PUMP)),
    ("session.rs::OperatorSession::poll_at", Excluded(PUMP)),
    ("session.rs::OperatorSession::poll_host", Excluded(PUMP)),
    (
        "session.rs::OperatorSession::publish_start_queue",
        Excluded("projection publication: queue places into the fleet rows"),
    ),
    ("session.rs::OperatorSession::advance_removals", Excluded(PUMP)),
    ("session.rs::OperatorSession::take_write_failures", Excluded(RESULT)),
    ("session.rs::OperatorSession::take_settled_saves", Excluded(RESULT)),
    ("session.rs::OperatorSession::take_settings_writes", Excluded(RESULT)),
    (
        "session.rs::OperatorSession::flush_writes",
        Excluded("startup/harness boundary: blocks until the queued writes settle"),
    ),
    // T3.3 r3: staged inputs and startup/pump entries of the parent's rows.
    ("profile_form.rs::ProfileFormSave::settle", Excluded("adopts the durable writer's results (SaveProfile's Owner terminal)")),
    ("scripts/mod.rs::Scripts::take_catalog_import_update", Excluded(RESULT)),
    ("session/commands.rs::Submitted::take_bulk", Excluded(RESULT)),
    ("session/commands.rs::Submitted::take_marked", Excluded(RESULT)),
    ("session/prefs.rs::DrawPolicy::key", Excluded(FORMAT)),
    ("nav_prefs.rs::nav_preference", Excluded("reads the stored preference; SetNavPreference writes it")),
    ("session/commands.rs::OperatorSession::stage_select_opens_slot", Staged(C::Select)),
    ("session/fleet.rs::OperatorSession::stage_mark_all", Staged(C::MarkAll)),
    ("session/fleet.rs::OperatorSession::stage_reorder", Staged(C::ReorderProfiles)),
    ("session/prefs.rs::OperatorSession::stage_notice", Staged(C::AcknowledgeNotice)),
    ("session/prefs.rs::OperatorSession::stage_log_save", Staged(C::SaveLog)),
    ("session/prefs.rs::OperatorSession::stage_quester_paths", Staged(C::SetQuesterPaths)),
    ("session/prefs.rs::OperatorSession::stage_quester_reload", Staged(C::ReloadQuesterPaths)),
    ("session/prefs.rs::OperatorSession::watch_setting_events", Excluded(HARNESS)),
    ("session/prefs.rs::OperatorSession::take_setting_events", Excluded(HARNESS)),
    ("session/prefs.rs::OperatorSession::advance_quester_paths", Excluded(PUMP)),
    ("session/prefs.rs::OperatorSession::adopt_operator_preferences", Excluded(STARTUP)),
    ("session/prefs.rs::OperatorSession::refresh_walk_globals", Excluded(PUMP)),
    ("session/prefs.rs::OperatorSession::poll_preferences", Excluded(PUMP)),
    ("session/prefs.rs::OperatorSession::poll_preferences_at", Excluded(PUMP)),
    (
        "session/prefs.rs::OperatorSession::dismiss_setting_failure",
        Excluded("presentation: clears a front end's own setting message once the action that resolves it happened"),
    ),
    ("session/prefs.rs::OperatorSession::declare_headless_render", Excluded(STARTUP)),
    (
        "session/prefs.rs::DrawPolicy::set_persist",
        Excluded("a live harness run's draw setup is never written; configuration, not a decision"),
    ),
    ("session/vault_rows.rs::OperatorSession::stage_vault_unlock", Staged(C::UnlockVault)),
    ("session/vault_rows.rs::OperatorSession::stage_vault_create", Staged(C::CreateVault)),
    ("session/vault_rows.rs::OperatorSession::stage_vault_reset", Staged(C::ResetVault)),
    (
        "session.rs::OperatorSession::adopt_tutorial_reading",
        Excluded("adopts the engine's getvar reading of TutSkip: an owner result; the operator's TutSkip is MarkTutorialSkipped"),
    ),
    ("log_file.rs::restore_session_log", Excluded(STARTUP)),
    ("log_file.rs::session_log_path", Excluded(ACCESSOR)),
    ("log_file.rs::session_file_status", Excluded(ACCESSOR)),
    ("log_file.rs::session_file_phase", Excluded(ACCESSOR)),
    ("log_file.rs::session_file_generation", Excluded(ACCESSOR)),
    ("fleet.rs::FleetFilter::staged_mut", Staged(C::SetFleetFilter)),
    ("fleet.rs::FleetModel::staged_filter_mut", Staged(C::SetFleetFilter)),
    ("fleet.rs::FleetModel::stage_card", Staged(C::SetFleetCard)),
    ("fleet.rs::FleetModel::sync_card_label", Excluded(PROJECTION)),
    ("driver.rs::HeadlessDriver::sync_card_label", Excluded(PROJECTION)),
    (
        "session/fleet_plan.rs::OperatorSession::stage_plan_card",
        Staged(C::PlanFleetAction(crate::fleet_plan::FleetAction::CopySettings)),
    ),
    (
        "session/fleet_plan.rs::OperatorSession::stage_plan_target",
        Staged(C::PlanFleetAction(crate::fleet_plan::FleetAction::SwitchWorld(1))),
    ),
    ("fleet.rs::FleetModel::refresh", Excluded(PROJECTION)),
    ("driver.rs::HeadlessDriver::unlock", Rows(&[C::UnlockVault, C::CreateVault], "stages the vault and the play launch, then submits UnlockVault or, for a new file, CreateVault")),
    ("driver.rs::HeadlessDriver::session_mut", Excluded(ACCESSOR)),
    ("driver.rs::HeadlessDriver::models_mut", Excluded(ACCESSOR)),
    ("driver.rs::DriverModels::fleet_mut", Excluded(ACCESSOR)),
    ("driver.rs::DriverModels::map_mut", Excluded(ACCESSOR)),
    ("driver.rs::DriverModels::scripts_mut", Excluded(ACCESSOR)),
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
    // walk_map.rs: staged selection/view state and read-only projections.
    ("walk_map.rs::WalkMap::bind", Excluded(PROJECTION)),
    ("walk_map.rs::WalkMap::clear_selection", Excluded(MAP_DRAFT)),
    ("walk_map.rs::WalkMap::close", Excluded(MAP_DRAFT)),
    ("walk_map.rs::WalkMap::parse_coordinates", Excluded(FORMAT)),
    ("walk_map.rs::WalkMap::recenter", Excluded(PROJECTION)),
    ("walk_map.rs::WalkMap::select_poi", Excluded(MAP_DRAFT)),
    ("walk_map.rs::WalkMap::select_tile", Excluded(MAP_DRAFT)),
    ("walk_map.rs::WalkMap::set_center", Excluded(PROJECTION)),
    ("walk_map.rs::WalkMap::set_layers", Excluded(PROJECTION)),
    ("walk_map.rs::WalkMap::set_plane", Excluded(PROJECTION)),
    ("walk_map.rs::WalkMap::set_scale", Excluded(PROJECTION)),
    ("walk_map.rs::WalkSearch::clear", Excluded(MAP_DRAFT)),
    ("walk_map.rs::WalkSearch::update", Excluded(MAP_DRAFT)),
    ("walk_map.rs::default_walk_nav_flags_path", Excluded(PATH_READ)),
    ("walk_map.rs::display_walk_name_into", Excluded(FORMAT)),
    ("walk_map.rs::observed_walk_services", Excluded(PROJECTION)),
    ("walk_map.rs::peek_walk_catalogue", Excluded(ACCESSOR)),
    ("walk_map.rs::reap_walk_map_demand", Excluded(PUMP)),
    ("walk_map.rs::select_walk_route_source", Excluded(PROJECTION)),
    ("walk_map.rs::validate_walk_search", Excluded(FORMAT)),
    ("walk_map.rs::walk_teleport_authorized", Excluded(FORMAT)),
    // walk_permissions.rs
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
    (
        "panel/session.rs::Session::start_vault",
        Rows(
            &[C::UnlockVault, C::CreateVault],
            "refuses the vault prompt's pass before it submits (no bound profile): Unlock when the vault file exists, Create when it does not",
        ),
    ),
    // --- panel: fleet view ---
    (
        "panel/fleet.rs::table",
        Rows(
            &[C::MarkFleet(crate::MarkOp::Shown), C::SetMark(true)],
            "the Mark column header is the tri-state shown-marks box; each row's checkbox sets that row's mark",
        ),
    ),
    (
        "panel/fleet_toolbar.rs::filter_row",
        Rows(
            &[C::SetFleetFilter, C::SetFleetSort(crate::FleetOrder::Status)],
            "the fleet toolbar's filter field and sort drop-down, one row each",
        ),
    ),
    // --- panel: fleet plan flow ---
    (
        "panel/fleet_actions.rs::Session::confirm_fleet_plan",
        Rows(
            &[C::SwitchWorld(1)],
            "Confirm submits the waiting plan's own command (`FleetPlan::command`, `Targets::Plan`): the row follows the plan's action, and Switch world's row is issued only through a plan",
        ),
    ),
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
    ("panel/session.rs::Session::set_capture", Row(C::SetDraw(DrawSwitch::CaptureInput, true))),
    ("panel/session.rs::Session::set_sidecar_50", Row(C::SetDraw(DrawSwitch::SidecarFullRate, true))),
    (
        "panel/session.rs::Session::set_live_full_rate",
        Row(C::SetDraw(DrawSwitch::LiveFullRate, true)),
    ),
    ("panel/app.rs::only_render_selected_clicked", Row(C::SetRenderAll(true))),
    // --- panel: map ---
    (
        "panel/input_capture.rs::KeyboardOwner::process_ownership",
        Excluded(GAME_INPUT_STAGING),
    ),
    (
        "panel/input_capture.rs::KeyboardOwner::capture_keys",
        Excluded(GAME_INPUT_STAGING),
    ),
    ("panel/input_capture.rs::maybe_send_click", Excluded(GAME_INPUT_STAGING)),
    ("panel/input_capture.rs::stream_capture", Excluded(GAME_INPUT_STAGING)),
    (
        "panel/input_capture.rs::PaintMouseGate::route",
        Excluded(GAME_INPUT_STAGING),
    ),
    (
        "panel/input_capture.rs::PaintMouseGate::route_button",
        Excluded(GAME_INPUT_STAGING),
    ),
    (
        "panel/session.rs::Session::maybe_drain_getvar_dialogue",
        Excluded(GAME_INPUT_STAGING),
    ),
    (
        "panel/session.rs::Session::accept_map_bake",
        Rows(
            &[C::SetMapBakeChoice(MapBakeChoice::Always), C::BakeMapNow],
            "Always bake persists the choice first; both choices submit BakeMapNow, which opens the image demand",
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
                C::SaveLegacyScriptSettings,
            ],
            "the parameter text field: edits the staged text, discards on Escape, commits on Enter (the legacy overrides without a focused profile)",
        ),
    ),
    (
        "panel/session_catalog.rs::Session::select_script_selection",
        Rows(
            &[C::PickBrowseCard, C::SelectScriptCard, C::PrepareScriptCard],
            "the shared Browse pick; with a focused profile also its pending choice; a loaded card also queues its preparation",
        ),
    ),
    (
        "panel/session_catalog.rs::Session::load_js",
        Rows(
            &[C::LoadScriptFile, C::SelectScriptCard],
            "Load registers the file card, then selects it for the focused profile",
        ),
    ),
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
                C::ImportCatalog,
                C::PreviewProfileImport,
                C::CreateProfile,
                C::SaveProfile,
                C::ReturnToProfileImportPath,
                C::SetProfileImportDeletePrompt(true),
                C::DeleteProfileImportExport,
            ],
            "Preview first fills the catalogue when it has not been, then reads and maps; Import creates or saves each planned profile; Back drops the preview; confirmed deletion removes the export",
        ),
    ),
    // --- panel: profile form and chooser ---
    (
        "panel/app.rs::profile_form_body",
        Rows(
            &[
                C::EditProfileDraft(ProfileDraftField::Username),
                C::EditProfileDraft(ProfileDraftField::Password),
            ],
            "the profile form text fields stage username and password edits",
        ),
    ),
    (
        "panel/app.rs::slot_capture_section",
        Rows(
            &[
                C::SetAutoLogin(true),
                C::EditProfileDraft(ProfileDraftField::Settings),
            ],
            "the capture controls stage auto-login and the selected capture settings",
        ),
    ),
    (
        "panel/app.rs::slot_random_section",
        Rows(
            &[
                C::SetGuardian,
                C::EditProfileDraft(ProfileDraftField::Settings),
            ],
            "the random-event controls stage guardian and policy edits",
        ),
    ),
    (
        "panel/app.rs::slot_survival_section",
        Rows(
            &[
                C::SetSurvival,
                C::EditProfileDraft(ProfileDraftField::Settings),
            ],
            "the cure poison control stages the survival edit",
        ),
    ),
    (
        "panel/session/chooser.rs::Session::clear_credentials",
        Rows(
            &[
                C::EditProfileDraft(ProfileDraftField::Username),
                C::EditProfileDraft(ProfileDraftField::Password),
            ],
            "clears the two credential draft inputs",
        ),
    ),
    (
        "panel/session/chooser.rs::Session::begin_edit_profile",
        Rows(&[C::OpenProfileForm, C::OpenNewProfileForm], FORM_TARGET),
    ),
    (
        "panel/session/chooser.rs::Session::save_credentials",
        Rows(
            &[C::CreateProfile, C::SaveProfile, C::SetWorld(WorldChoice::Automatic)],
            "a new row creates the profile; an existing one saves the form, its world mirrored",
        ),
    ),
    // --- tui: process entry, harnesses ---
    ("tui/bin.rs::parse_args", Excluded(PROCESS)),
    ("tui/bin.rs::usage", Excluded(PROCESS)),
    (
        "tui/bin.rs::exit_after_work_shutdown",
        Excluded("final process exit after WorkAdmission shutdown; no operator decision"),
    ),
    (
        "tui/bin.rs::map_bundle",
        Excluded("`--map-bundle` packaging flag: bakes the shipped map bundle; not an operator command"),
    ),
    // --- tui: routers and the pump ---
    (
        "tui/bin.rs::dispatch",
        Rows(
            &[
                C::CancelSettingsSync,
                C::RestartForSettings,
                C::DeferSettingsRestart,
                C::ReleaseMapDemand,
                C::EditProfileDraft(ProfileDraftField::Settings),
            ],
            "the AppAction router: these rows are inlined; every other action calls a TuiSession handler registered on its own",
        ),
    ),
    (
        "tui/input.rs::TuiApp::run_command",
        Rows(
            &[
                C::OpenProfileForm,
                C::OpenNewProfileForm,
                C::CloseProfileForm(FormClose::Form),
            ],
            "the palette/command router: opens modals inline (the form notice follows the modal), and returns the AppAction the dispatcher routes",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::pump",
        Rows(
            &[
                C::PickBrowseCard,
                C::SelectScriptCard,
                C::OpenProfileForm,
                C::OpenNewProfileForm,
                C::CloseProfileForm(FormClose::Form),
            ],
            "the frame pump: adopts the Browse pick as the shared pick and as the focused profile's pending choice and the form notice follows the app state; the exits are the `--memory` harness's",
        ),
    ),
    // --- tui: vault and preferences ---
    (
        "panel/app.rs::commit_nav_settings",
        Rows(
            &[
                C::SetNavPreference(NavPreference::AllowTeleports, true),
                C::SetDangerLevel(DangerLevel::Never),
            ],
            "the Nav config window: one command per changed navigation preference, and the danger level",
        ),
    ),
    (
        "panel/session.rs::Session::start_play",
        Rows(
            &[C::UnlockVault, C::CreateVault],
            "the vault pass: unlock the existing vault file, or create the vault on a first run (no file yet)",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::unlock_at",
        Rows(
            &[C::UnlockVault, C::CreateVault],
            "the vault pass: unlock the existing vault file, or create the vault on a first run (no file yet)",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::persist_walk_globals",
        Rows(
            &[
                C::SetNavPreference(NavPreference::AllowTeleports, true),
                C::SetDangerLevel(DangerLevel::Never),
            ],
            "which walk permission the operator changed selects the row: a grant or an acknowledgement is a navigation preference, the danger routing level its own row",
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
        "tui/bin.rs::TuiSession::run_fleet_plan",
        Rows(
            &[C::SwitchWorld(1), C::PlanFleetAction(crate::fleet_plan::FleetAction::Start)],
            "Confirm submits the waiting plan's own command (`FleetPlan::command`, `Targets::Plan`): the row follows the plan's action, and Switch world's row is issued only through a plan; with no plan waiting, the confirmed action's PlanFleetAction is refused at the command boundary",
        ),
    ),
    (
        "tui/commands.rs::Command::operator_command",
        Excluded("names the operator command a palette command runs, for the refusal line of an unavailable one; issues none"),
    ),
    (
        "tui/bin.rs::TuiSession::script_start",
        Rows(
            &[C::PickBrowseCard, C::SelectScriptCard, C::StartScript],
            "Start picks the Browse card, records it as the profile's pending choice, then starts",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::persist_settings",
        Rows(
            &[C::SetMemoryMode(MemoryMode::Low), C::SetGuardian, C::SetSurvival],
            "the settings popup's profile rows: guardian fields, memory mode and cure poison, written through the profile row",
        ),
    ),
    (
        "tui/bin.rs::TuiSession::script_reload",
        Rows(&[C::ReloadScripts, C::ConfirmReload], RELOAD_CLICK),
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
        "tui/bin.rs::TuiSession::new_profile_key",
        Rows(
            &[
                C::EditProfileDraft(ProfileDraftField::Username),
                C::EditProfileDraft(ProfileDraftField::Password),
                C::CloseProfileForm(FormClose::Form),
            ],
            "the New profile overlay keys: typing stages the focused field's draft edit, Enter saves through `new_profile_save` (the CreateProfile handler), Esc closes the form",
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
    // --- tui: map, input ---
    (
        "tui/bin.rs::TuiSession::chat_send",
        Rows(
            &[C::ScriptPaintInput, C::GameInput],
            "a paint answer (1-9, Enter; a host Pause/Resume/Stop control routes to its own row inside the ScriptPaintInput submission) or a dialogue Continue/Answer",
        ),
    ),
    // --- tui: loadouts ---
    (
        "tui/bin.rs::TuiSession::loadouts_key",
        Rows(
            &[
                C::SelectLoadout,
                C::SaveLoadout,
                C::CreateLoadout,
                C::DuplicateLoadout,
                C::DeleteLoadout,
            ],
            "the loadouts popup: row 0 selects the next loadout, Enter on a text row or [save] stages the text as the draft and saves it, the buttons create, duplicate and delete",
        ),
    ),
];

const PROCESS: &str =
    "process entry, argument errors or start-up failure exits; not an operator command";
const HARNESS: &str = "`--live`/headed harness preparation and records; not an operator command";
const BENCH: &str =
    "opt-in memory benchmark (`memory-profile`) run and records; not an operator command";
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

/// The command seam's dispatcher entries: their row is the literal
/// `OperatorCommand::Row` argument (`SurfaceFn::row_calls`).
const SEAM: &[&str] = &[
    "session/commands.rs::OperatorSession::submit",
    "session/commands.rs::OperatorSession::submit_at",
    "session/commands.rs::OperatorSession::submit_detached",
];

/// The headless driver's forwarding entry into the seam.
const DRIVER_SUBMIT: &str = "driver.rs::HeadlessDriver::submit";

/// The row of [`OperatorCommand::ALL`] whose variant is `name`.
fn row_named(name: &str) -> Option<OperatorCommand> {
    OperatorCommand::ALL.into_iter().find(|row| {
        let debug = format!("{row:?}");
        debug
            .split(['(', '{'])
            .next()
            .is_some_and(|variant| variant.trim() == name)
    })
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
        Staged(row) => vec![row],
        StagedFor(rows, _) => rows.to_vec(),
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
    // The dispatchers name every row; a row counts as reached through a
    // literal argument at a call site instead.
    for decision in CORE
        .iter()
        .filter(|(key, _)| !SEAM.contains(key) && *key != DRIVER_SUBMIT)
        .map(|(_, d)| *d)
        .chain(SURFACE.iter().map(|(_, d)| *d))
    {
        reached.extend(rows_of(decision));
    }
    let unlisted: Vec<OperatorCommand> = reached
        .iter()
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
        reached.extend(
            item.row_calls
                .iter()
                .filter_map(|(name, _)| row_named(name)),
        );
        for call in &item.core_calls {
            if SEAM.contains(&call.key.as_str()) {
                continue;
            }
            if let Some(decision) = core.get(call.key.as_str()) {
                reached.extend(rows_of(*decision));
            }
        }
    }
    let unreached: Vec<OperatorCommand> = OperatorCommand::ALL
        .into_iter()
        .filter(|row| !reached.iter().any(|r| same_row(*r, *row)))
        .collect();
    assert!(
        unreached.is_empty(),
        "rows no entry point issues: {unreached:?}"
    );
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
        // A call of the command seam issues the literal row it names; the
        // dispatcher entry itself names no row.
        for (name, line) in &item.row_calls {
            match row_named(name) {
                Some(row) => calls.push((*line, "OperatorCommand", vec![row])),
                None => problems.push(format!("{key}:{line} names unknown row {name}")),
            }
        }
        for call in &item.core_calls {
            if SEAM.contains(&call.key.as_str()) {
                continue;
            }
            match core.get(call.key.as_str()) {
                // A staged input is not a decision; the bypass test checks
                // that the same fn submits the row it stages for.
                Some(Staged(_) | StagedFor(..)) => {}
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
            Some(Staged(row)) => problems.push(format!(
                "{key}: Staged({row:?}) is a core decision; a surface fn is a handler, a composition or an exclusion"
            )),
            Some(StagedFor(rows, _)) => problems.push(format!(
                "{key}: StagedFor({rows:?}) is a core decision; a surface fn is a handler, a composition or an exclusion"
            )),
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

/// The surface register shrinks with the seam: a function registered as
/// one row's handler that would pass unregistered (every call names that
/// one row through the seam, no probe) is not listed.
#[test]
fn the_surface_register_lists_only_functions_that_need_it() {
    let census = census();
    let core: BTreeMap<&str, Decision> = CORE.iter().copied().collect();
    let mut redundant = Vec::new();
    for item in &census.surface {
        let key = surface_key(item);
        let Some((_, Row(registered))) = SURFACE.iter().find(|(k, _)| *k == key) else {
            continue;
        };
        if !item.probes.is_empty() {
            continue;
        }
        let mut rows: Vec<OperatorCommand> = item
            .row_calls
            .iter()
            .filter_map(|(name, _)| row_named(name))
            .collect();
        let mut plain = true;
        for call in &item.core_calls {
            if SEAM.contains(&call.key.as_str()) {
                continue;
            }
            match core.get(call.key.as_str()) {
                Some(Staged(_) | StagedFor(..) | Excluded(_)) | None => {}
                Some(decision) => {
                    let alternatives = rows_of(*decision);
                    if alternatives.len() != 1 {
                        plain = false;
                    }
                    rows.extend(alternatives);
                }
            }
        }
        if plain && !rows.is_empty() && rows.iter().all(|row| same_row(*row, *registered)) {
            redundant.push(format!(
                "{key}: a plain {registered:?} handler needs no entry"
            ));
        }
    }
    assert!(redundant.is_empty(), "\n{}", redundant.join("\n"));
}

/// The seam is complete: no panel/TUI function reaches a row through
/// anything but the command seam. A call of a core entry that names a row
/// (other than the seam itself and the staging inputs a row commits) and
/// an effect probe (a store `.save()`, a vault or file write, a map demand)
/// are bypasses; only process, harness and startup exclusions may hold
/// them.
#[test]
fn no_surface_mutation_bypasses_the_seam() {
    let census = census();
    let core: BTreeMap<&str, Decision> = CORE.iter().copied().collect();
    let surface: BTreeMap<&str, Decision> = SURFACE.iter().copied().collect();
    let mut bypasses = Vec::new();
    for item in &census.surface {
        let key = surface_key(item);
        if matches!(surface.get(key.as_str()), Some(Excluded(_))) {
            continue;
        }
        for call in &item.core_calls {
            if SEAM.contains(&call.key.as_str()) {
                continue;
            }
            match core.get(call.key.as_str()) {
                Some(Staged(row)) => {
                    let submits = item
                        .row_calls
                        .iter()
                        .filter_map(|(name, _)| row_named(name))
                        .any(|submitted| same_row(submitted, *row));
                    if !submits {
                        bypasses.push(format!(
                            "{}:{} {key} stages {} for {row:?} but does not submit it",
                            item.file, call.line, call.key
                        ));
                    }
                }
                Some(StagedFor(rows, _)) => {
                    let submits = item
                        .row_calls
                        .iter()
                        .filter_map(|(name, _)| row_named(name))
                        .any(|submitted| rows.iter().any(|row| same_row(submitted, *row)));
                    let row = rows;
                    if !submits {
                        bypasses.push(format!(
                            "{}:{} {key} stages {} for {row:?} but does not submit it",
                            item.file, call.line, call.key
                        ));
                    }
                }
                Some(decision) => {
                    let rows = rows_of(*decision);
                    if !rows.is_empty() {
                        bypasses.push(format!(
                            "{}:{} {key} calls {} {rows:?}",
                            item.file, call.line, call.key
                        ));
                    }
                }
                None => {}
            }
        }
        for (probe, line) in &item.probes {
            bypasses.push(format!("{}:{line} {key} hits probe {probe:?}", item.file));
        }
    }
    for line in &bypasses {
        println!("BYPASS\t{line}");
    }
    assert!(
        bypasses.is_empty(),
        "{} surface mutations bypass the command seam",
        bypasses.len()
    );
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
    let mut staged_for: Vec<String> = Vec::new();
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
                Some(StagedFor(rows, why)) => {
                    staged_for.push(format!("{}: {rows:?}: {why}", entry.key));
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
    for line in &staged_for {
        println!("STAGED-FOR\t{line}");
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

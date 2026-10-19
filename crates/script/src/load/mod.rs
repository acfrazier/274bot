//! JS Load: shape detection, the picker library of loaded JS cards, and the
//! out-of-tree `LoadIsolate` (rustyscript V8 on its own thread).
//!
//! This module is feature-gated because its cache-backed `JsLibrary`, sibling
//! resolution and V8 isolate need `load`. Static registry parsing and settings
//! schemas live in `rs2b0t_registry` and compile without that feature.
//!
//! Loading (`JsLibrary::load`) reads and classifies source, registers the card,
//! and persists `{name, path}`. Reload validates prepared candidates through
//! the process ValidationSeat. Gameplay gets a separate isolate only on JS
//! Start via the slot-owned factory; validation has no gameplay authority.
//! Nothing here `include_str!`s a script tree. 0.1.5 listed TS is an operator
//! `$RS2B0T` path.

#[cfg(feature = "load")]
mod area;
#[cfg(feature = "load")]
mod bank_locations_v8;
#[cfg(feature = "load")]
mod bank_tasks_v8;
#[cfg(feature = "load")]
mod bindings;
#[cfg(feature = "load")]
mod boost_potions_v8;
#[cfg(feature = "load")]
mod buyout_plan;
#[cfg(feature = "load")]
mod cake_stall_v8;
#[cfg(feature = "load")]
pub(crate) mod callback_v8;
#[cfg(feature = "load")]
mod canvas_tape;
#[cfg(feature = "load")]
mod chatbox_v8;
mod clue_facts_v8;
mod clue_logic_v8;
mod clue_pack_v8;
#[cfg(feature = "load")]
mod combat_style_v8;
#[cfg(feature = "load")]
mod crypto_stub;
#[cfg(feature = "load")]
mod dialog_v8;
#[cfg(feature = "load")]
mod distance;
#[cfg(feature = "load")]
mod fire_v8;
mod gather_methods_v8;
#[cfg(feature = "load")]
mod gathering_v8;
#[cfg(feature = "load")]
mod hunt_v8;
#[cfg(feature = "load")]
mod inventory_v8;
#[cfg(feature = "load")]
mod isolate;
mod library;
#[cfg(feature = "load")]
mod line_of_sight;
#[cfg(feature = "load")]
mod lineage_v8;
mod loadout_v8;
#[cfg(feature = "load")]
mod machine_v8;
#[cfg(feature = "load")]
mod melee_weapons_v8;
#[cfg(feature = "load")]
mod paint_chrome;
#[cfg(feature = "load")]
mod paint_jive;
#[cfg(feature = "load")]
mod partner_trade_v8;
#[cfg(feature = "load")]
mod platform;
#[cfg(feature = "load")]
mod progress_methods_v8;
mod quest_facts_v8;
#[cfg(feature = "load")]
pub(crate) mod reach_query;
#[cfg(feature = "load")]
mod recovery_hints_v8;
#[cfg(feature = "load")]
mod run_policy_v8;
#[cfg(feature = "load")]
mod rune_routes_v8;
#[cfg(feature = "load")]
mod runtime_bridge;
#[cfg(feature = "load")]
mod scene_v8;
mod selected_facts_v8;
mod shape;
#[cfg(feature = "load")]
mod shop_ops_v8;
#[cfg(feature = "load")]
mod skills_v8;
#[cfg(feature = "load")]
mod snapshot;
mod supply_v8;
#[cfg(feature = "load")]
mod targets_v8;
#[cfg(feature = "load")]
mod tools_v8;
#[cfg(feature = "load")]
mod wait_clock;
#[cfg(feature = "load")]
mod walk_opening_v8;

pub use shape::{
    collect_raw_sibling_hashes, detect_shape, first_unloadable_for_card,
    first_unloadable_specifier, is_catalog_dim, is_reserved, live_example_path,
    live_file_fixture_path, live_file_fixture_stem, parse_declared_api_version,
    raw_content_fingerprint, resolve_api_family, resolve_sibling_path, scan_import_specifiers,
    scan_same_folder_js_imports, scan_scripts_sibling_js_imports, sibling_module_url, ApiFamily,
    LoadShape, ScriptSel, VersionDiag, CATALOG_DIM,
};
#[cfg(feature = "load")]
pub use shape::{resolve_sibling_modules, transpile_ts};

#[cfg(feature = "load")]
pub use library::JsLibrary;
pub use library::{
    default_js_store, format_load_failures, CatalogApplyReport, CatalogDiff, JsCard, LoadFailure,
    LoadStage, PreparedCard,
};
#[cfg(feature = "load")]
pub use library::{
    CardPreparationOutcome, CardPreparationSnapshot, CatalogPreparationResult,
    CatalogPreparationSnapshot, CatalogSnapshotError, CatalogWorkerError,
};

#[cfg(feature = "load")]
pub(crate) use isolate::GameplayFactory;
#[cfg(feature = "load")]
pub(crate) use isolate::TickOutcome;
#[cfg(feature = "load")]
pub use isolate::{
    isolate_capacity_status, HeldWaitFacts, IsolateCapacityStatus, LoadIsolate, LoadIsolateError,
    QueuedInteract, QueuedSeatBatch, Ready, ScriptStopReceipt, TeardownProof, ValidationBatch,
    ValidationError,
};
#[cfg(feature = "load")]
pub use platform::bootstrap_v8_platform;
#[cfg(feature = "load")]
pub(crate) use recovery_hints_v8::{
    post as post_recovery_hints, startup_complete as recovery_startup_complete,
};
#[cfg(feature = "load")]
pub use recovery_hints_v8::{HintTile, RecoveryHintsCell};

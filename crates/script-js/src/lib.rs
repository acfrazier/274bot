//! JavaScript script surface: source loading, the picker library and V8 isolates.
//!
//! Loading and validation use the process ValidationSeat. Gameplay gets a
//! separate isolate only on Start, through the native slot's surface factory.
//! Nothing here embeds a catalog script tree; catalog sources come from the
//! operator's rs2b0t checkout.

mod api_v2;
pub mod compat;
pub mod library;
/// Per-isolate step-machine host: families Rust begins, steps and aborts.
mod machine;
mod materialize;
pub mod runtime;
pub mod scene;
pub mod shim;
pub mod surface;
pub mod tooling;
pub mod wire;

pub use library::shape::{
    collect_raw_sibling_hashes, detect_shape, first_unloadable_for_card,
    first_unloadable_specifier, is_catalog_dim, is_reserved, live_example_path,
    live_file_fixture_path, live_file_fixture_stem, parse_declared_api_version,
    raw_content_fingerprint, resolve_api_family, resolve_sibling_path, scan_import_specifiers,
    scan_same_folder_js_imports, scan_scripts_sibling_js_imports, sibling_module_url, ApiFamily,
    LoadShape, VersionDiag, CATALOG_DIM,
};

pub use library::shape::{resolve_sibling_modules, transpile_ts};

pub use library::store::JsLibrary;
pub use library::store::{
    default_js_store, format_load_failures, CatalogApplyReport, CatalogDiff, JsCard, LoadFailure,
    LoadStage, PreparedCard,
};

pub use library::store::{
    CardPreparationOutcome, CardPreparationSnapshot, CatalogPreparationResult,
    CatalogPreparationSnapshot, CatalogSnapshotError, CatalogWorkerError,
};

pub use runtime::isolate::{
    isolate_capacity_status, IsolateCapacityStatus, LoadIsolate, LoadIsolateError, Ready,
    TeardownProof, ValidationBatch, ValidationError,
};

pub use runtime::platform::bootstrap_v8_platform;

pub use library::js_cache::{default_js_cache_root, CacheMeta, CachedJs, JsCache};
pub use library::rs2b0t_registry::{
    clear_rs2b0t_import_at, default_rs2b0t_import_file, default_rs2b0t_path_file, parse_registry,
    parse_registry_with_sources, persist_rs2b0t_root, persist_rs2b0t_root_at, registry_index_path,
    rs2b0t_import_deferred, rs2b0t_import_deferred_at, rs2b0t_root, rs2b0t_root_at,
    rs2b0t_root_checked, rs2b0t_root_checked_at, script_file_path, set_rs2b0t_import_deferred_at,
    settings_schema_from_source, try_settings_schema_from_source,
};

/// Canonical JS-backed cards installed by [`install_cards`].
pub use compat::sherlock::CARDS;

/// Install the canonical JS-backed cards into the native registry.
pub fn install_cards() -> Result<(), script::registry::CardInstallError> {
    script::registry::install_cards(&CARDS)
}

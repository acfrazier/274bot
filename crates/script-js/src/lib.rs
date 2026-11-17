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
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    pub(crate) fn local_api_asset(relative_path: &str, description: &str) -> bool {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../api")
            .join(relative_path);
        let path = path.to_string_lossy();
        api::test_support::local_asset_path("", &path, description).is_some()
    }

    /// Checks a scope-cache control call that opened `depth` nested V8 scopes
    /// (`cold`) and one that opened one level more (`extra`). The cache keeps
    /// one cell per new nesting level, so the cold call adds 1..=`depth`
    /// retained cells and the extra level adds one more cell of the same size.
    /// Cell counts below `depth` and the cell size depend on the V8 binding.
    pub(crate) fn assert_scope_cache_growth(
        cold: &allocation_counter::AllocationInfo,
        extra: &allocation_counter::AllocationInfo,
        depth: u64,
    ) {
        assert_eq!(
            extra.count_total, 1,
            "one more level adds one cell: {extra:?}"
        );
        assert!(
            (1..=depth).contains(&cold.count_total),
            "{depth} scopes add 1..={depth} cells: {cold:?}"
        );
        assert_eq!(
            cold.bytes_total,
            cold.count_total * extra.bytes_total,
            "cells share one size: cold={cold:?} extra={extra:?}"
        );
        for info in [cold, extra] {
            assert_eq!(info.count_current, info.count_total as i64, "{info:?}");
            assert_eq!(info.count_max, info.count_total, "{info:?}");
            assert_eq!(info.bytes_current, info.bytes_total as i64, "{info:?}");
            assert_eq!(info.bytes_max, info.bytes_total, "{info:?}");
        }
    }
}
pub mod wire;

pub use library::shape::{
    collect_raw_sibling_hashes, first_unloadable_for_card, first_unloadable_specifier,
    is_catalog_dim, is_reserved, live_example_path, live_file_fixture_path, live_file_fixture_stem,
    raw_content_fingerprint, resolve_sibling_path, scan_import_specifiers,
    scan_same_folder_js_imports, scan_scripts_sibling_js_imports, sibling_module_url, ApiFamily,
    LoadShape, UnloadReason, VersionDiag, CATALOG_DIM,
};

pub use library::prepare::{
    check_origin, compile_origin, CheckedLoad, CheckedOrigin, OriginRefusal,
};

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use library::prepare::{
    native_fixture, prepare_raw_origin_for_test, NativeFixture, PreparedTestOrigin,
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
    isolate_capacity_status, validation_batch_prefix_len, validation_card_fits, CardValidation,
    IsolateCapacityStatus, LoadIsolate, LoadIsolateError, Ready, TeardownProof, ValidationBatch,
    ValidationError, MAX_VALIDATION_BATCH_CARDS,
};

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use runtime::isolate::ThreadBlock;

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub use runtime::bindings::main_module_for_import_gate_test;

pub use runtime::platform::bootstrap_v8_platform;

pub use library::js_cache::{default_js_cache_root, CacheMeta, CacheRole, CachedJs, JsCache};
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

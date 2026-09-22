//! JS Load: shape detection, the picker library of loaded JS cards, and the
//! out-of-tree `LoadIsolate` (rustyscript V8 on its own thread).
//!
//! The classify surface (`detect_shape`, `LoadShape`, `is_reserved`,
//! `is_catalog_dim`, import scans, fingerprints, `JsCard`) compiles without
//! the `load` feature; the cache-backed `JsLibrary`, sibling resolve and the
//! isolate need it.
//!
//! Loading (`JsLibrary::load`) only reads, classifies, validates the source
//! in a throwaway Runtime (dropped before `load()` returns), registers the
//! card, and persists `{name, path}`. The isolate is spawned **only** on
//! Start of a JS card (`LoadIsolate::spawn`); nothing here `include_str!`s
//! a script tree. 0.1.5 listed TS is an operator `$RS2B0T` path.

mod shape;
mod library;
#[cfg(feature = "load")]
mod bindings;
#[cfg(feature = "load")]
mod buyout_plan;
mod supply_v8;
mod gather_methods_v8;
mod clue_facts_v8;
mod quest_facts_v8;
mod loadout_v8;
#[cfg(feature = "load")]
mod paint_chrome;
#[cfg(feature = "load")]
mod paint_jive;
#[cfg(feature = "load")]
mod snapshot;
#[cfg(feature = "load")]
mod isolate;
#[cfg(feature = "load")]
mod line_of_sight;

pub use shape::{
    collect_raw_sibling_hashes, detect_shape, first_unloadable_for_card,
    first_unloadable_specifier, is_catalog_dim, is_reserved, live_example_path,
    live_file_fixture_path, live_file_fixture_stem, parse_declared_api_version,
    raw_content_fingerprint, resolve_api_family, resolve_sibling_path, scan_import_specifiers,
    scan_same_folder_js_imports, scan_scripts_sibling_js_imports, sibling_module_url,
    ApiFamily, LoadShape, ScriptSel, VersionDiag, CATALOG_DIM,
};
#[cfg(feature = "load")]
pub use shape::{resolve_sibling_modules, transpile_ts};

pub use library::{
    default_js_store, format_load_failures, CatalogApplyReport, CatalogDiff, JsCard, LoadFailure,
    LoadStage, PreparedCard,
};
#[cfg(feature = "load")]
pub use library::JsLibrary;

#[cfg(feature = "load")]
pub use isolate::{LoadIsolate, ScriptStopReceipt, TeardownProof};

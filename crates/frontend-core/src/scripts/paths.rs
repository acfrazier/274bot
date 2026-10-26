//! Operator file locations the front ends show, open or browse from: the
//! bot home, files under `~/.274bot`, and the rs2b0t catalogue root with
//! its import-deferral marker.

use std::path::{Path, PathBuf};

/// The operator's home directory (the isolated home under tests).
pub fn bot_home() -> PathBuf {
    script::bot_home()
}

/// `~/.274bot/<name>`.
pub fn bot_file(name: &str) -> PathBuf {
    script::bot_file(name)
}

/// The file that records the rs2b0t catalogue root.
pub fn rs2b0t_path_file() -> PathBuf {
    script_js::default_rs2b0t_path_file()
}

/// The file that records an operator's "not now" on the catalogue import.
pub fn rs2b0t_import_file() -> PathBuf {
    script_js::default_rs2b0t_import_file()
}

/// The configured rs2b0t catalogue root, if any.
pub fn rs2b0t_root() -> Option<PathBuf> {
    script_js::rs2b0t_root()
}

/// Why a configured catalogue root cannot be used: the persisted root file
/// is unreadable, untrusted, not an absolute path or contains `..`. The
/// sentence names the file and the reason.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CatalogRootError(String);

/// The configured rs2b0t catalogue root, if any, with the reason a
/// configured root cannot be used.
pub fn rs2b0t_root_checked() -> Result<Option<PathBuf>, CatalogRootError> {
    script_js::rs2b0t_root_checked().map_err(CatalogRootError)
}

/// The registry index a catalogue root must contain.
pub fn registry_index_path(root: &Path) -> PathBuf {
    script_js::registry_index_path(root)
}

/// Whether the operator deferred the catalogue import (default marker).
pub fn rs2b0t_import_deferred() -> bool {
    script_js::rs2b0t_import_deferred()
}

/// Whether the marker at `path` records a deferred catalogue import.
pub fn rs2b0t_import_deferred_at(path: &Path) -> bool {
    script_js::rs2b0t_import_deferred_at(path)
}

/// Why the catalogue-import marker could not be written or removed (the
/// function called says which); the sentence is the script layer's.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ImportMarkerError(String);

/// Record a deferred catalogue import at `path`.
pub fn set_rs2b0t_import_deferred_at(path: &Path) -> Result<(), ImportMarkerError> {
    script_js::set_rs2b0t_import_deferred_at(path).map_err(ImportMarkerError)
}

/// Clear the catalogue-import marker at `path`.
pub fn clear_rs2b0t_import_at(path: &Path) -> Result<(), ImportMarkerError> {
    script_js::clear_rs2b0t_import_at(path).map_err(ImportMarkerError)
}

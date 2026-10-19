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
    script::default_rs2b0t_path_file()
}

/// The file that records an operator's "not now" on the catalogue import.
pub fn rs2b0t_import_file() -> PathBuf {
    script::default_rs2b0t_import_file()
}

/// The configured rs2b0t catalogue root, if any.
pub fn rs2b0t_root() -> Option<PathBuf> {
    script::rs2b0t_root()
}

/// Why a configured catalogue root cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogRootError {
    /// The persisted root file is unreadable, untrusted, not an absolute
    /// path or contains `..`; the sentence names the file and the reason.
    #[error("{0}")]
    Unusable(String),
}

/// The configured rs2b0t catalogue root, if any, with the reason a
/// configured root cannot be used.
pub fn rs2b0t_root_checked() -> Result<Option<PathBuf>, CatalogRootError> {
    script::rs2b0t_root_checked().map_err(CatalogRootError::Unusable)
}

/// The registry index a catalogue root must contain.
pub fn registry_index_path(root: &Path) -> PathBuf {
    script::registry_index_path(root)
}

/// Whether the operator deferred the catalogue import (default marker).
pub fn rs2b0t_import_deferred() -> bool {
    script::rs2b0t_import_deferred()
}

/// Whether the marker at `path` records a deferred catalogue import.
pub fn rs2b0t_import_deferred_at(path: &Path) -> bool {
    script::rs2b0t_import_deferred_at(path)
}

/// Why the catalogue-import marker could not be changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportMarkerError {
    /// The deferral marker could not be written.
    #[error("{0}")]
    Write(String),
    /// The marker could not be removed.
    #[error("{0}")]
    Clear(String),
}

/// Record a deferred catalogue import at `path`.
pub fn set_rs2b0t_import_deferred_at(path: &Path) -> Result<(), ImportMarkerError> {
    script::set_rs2b0t_import_deferred_at(path).map_err(ImportMarkerError::Write)
}

/// Clear the catalogue-import marker at `path`.
pub fn clear_rs2b0t_import_at(path: &Path) -> Result<(), ImportMarkerError> {
    script::clear_rs2b0t_import_at(path).map_err(ImportMarkerError::Clear)
}

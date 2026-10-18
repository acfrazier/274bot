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

/// The configured rs2b0t catalogue root, if any, with the reason a
/// configured root cannot be used.
pub fn rs2b0t_root_checked() -> Result<Option<PathBuf>, String> {
    script::rs2b0t_root_checked()
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

/// Record a deferred catalogue import at `path`.
pub fn set_rs2b0t_import_deferred_at(path: &Path) -> Result<(), String> {
    script::set_rs2b0t_import_deferred_at(path)
}

/// Clear the catalogue-import marker at `path`.
pub fn clear_rs2b0t_import_at(path: &Path) -> Result<(), String> {
    script::clear_rs2b0t_import_at(path)
}

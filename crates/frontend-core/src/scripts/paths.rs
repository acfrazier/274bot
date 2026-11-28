//! Operator file locations the front ends show, open or browse from: the
//! bot home and files under `~/.274bot`.

#[cfg(feature = "js")]
use std::path::Path;
use std::path::PathBuf;

/// The operator's home directory (the isolated home under tests).
pub fn bot_home() -> PathBuf {
    script::bot_home()
}

/// `~/.274bot/<name>`.
pub fn bot_file(name: &str) -> PathBuf {
    script::bot_file(name)
}

#[cfg(feature = "js")]
/// The registry index a catalogue root must contain.
pub fn registry_index_path(root: &Path) -> PathBuf {
    script_js::registry_index_path(root)
}

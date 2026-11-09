#[path = "test_support/local_asset_path.rs"]
mod local_asset_path;
#[path = "test_support/local_asset_path_377.rs"]
mod local_asset_path_377;

pub use local_asset_path::local_asset_path;
pub use local_asset_path_377::local_asset_path_377;

/// Check the local inputs needed by tests that use embedded game data.
///
/// Missing assets skip only on hosted GitHub Actions. Local runs retain the
/// actionable failure from [`local_asset_path`].
pub fn game_data_available() -> bool {
    for revision in ["274", "289"] {
        let path = format!(
            "{}/data/game-data/{revision}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        if local_asset_path(
            "",
            &path,
            &format!("revision {revision} game data for asset-backed tests"),
        )
        .is_none()
        {
            return false;
        }
    }
    embedded_game_data_available()
}

/// Check this after probing required files with `local_asset_path`, so missing
/// local inputs still fail and hosted skips still identify the missing path.
pub fn embedded_game_data_available() -> bool {
    let available = crate::game_data::local_game_data_available();
    if !available {
        eprintln!(
            "Skipping embedded game-data assertions: this API build does not embed local game data"
        );
    }
    available
}

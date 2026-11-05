#[path = "test_support/local_asset_path.rs"]
mod local_asset_path;

pub use local_asset_path::local_asset_path;

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

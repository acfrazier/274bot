pub fn local_game_data_available(revision: i32) -> bool {
    let relative_path = format!("data/game-data/{revision}.json");
    let description = format!(
        "API local asset `{relative_path}`; generate it with `python tools/local-data/build.py`"
    );
    crate::local_api_asset::local_api_asset(&relative_path, &description)
}

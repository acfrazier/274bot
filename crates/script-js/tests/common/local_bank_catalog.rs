pub fn local_bank_catalog_available() -> bool {
    crate::local_api_asset::local_api_asset(
        "data/game-data/bank-catalog.json",
        "API local asset `data/game-data/bank-catalog.json`; generate it with `python tools/local-data/build.py`",
    )
}

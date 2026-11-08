pub fn local_fact_catalogs_available() -> bool {
    let banks = crate::local_bank_catalog::local_bank_catalog_available();
    let cooking = crate::local_api_asset::local_api_asset(
        "data/game-data/cook-catalog.json",
        "API local asset `data/game-data/cook-catalog.json`; generate it with `python tools/local-data/build.py`",
    );
    banks && cooking
}

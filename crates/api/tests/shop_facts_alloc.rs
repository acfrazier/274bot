//! The borrowed shop vendor iterator allocates nothing after its shared facts are warm.
use api::game_data::for_revision;
use api::selected::ClientRevision;
use api::shop_facts::vendors_for;
use api::test_support::local_asset_path;

#[test]
fn warmed_vendors_for_pass_allocates_nothing() {
    if local_asset_path(
        "",
        concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-data/289.json"),
        "revision 289 game data for shop facts allocation test",
    )
    .is_none()
    {
        return;
    }
    let data = for_revision(ClientRevision::R289).expect("selected game data");
    let harpoon_id = data.item_by_alias("harpoon").expect("harpoon").id;

    assert_eq!(vendors_for(&data, harpoon_id).count(), 2);
    let mut vendor_count = 0;
    let measured = allocation_counter::measure(|| {
        vendor_count = vendors_for(&data, harpoon_id).count();
    });

    assert_eq!(vendor_count, 2);

    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.count_current, 0);
}

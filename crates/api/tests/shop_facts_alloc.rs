//! The borrowed shop vendor iterator allocates nothing after its shared facts are warm.
use api::game_data::for_revision;
use api::selected::ClientRevision;
use api::shop_facts::{keepers, vendors_for};

#[test]
fn warmed_vendors_for_pass_allocates_nothing() {
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

#[test]
fn warmed_keeper_pass_allocates_nothing_on_both_revisions() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected game data");
        let expected = keepers(&data).count();
        assert!(expected > 0);
        let mut count = 0;
        let measured = allocation_counter::measure(|| {
            count = keepers(&data).count();
        });
        assert_eq!(count, expected);
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.count_current, 0);
    }
}

use std::sync::Arc;

use api::game_data::{for_optional_profile, for_profile, for_revision};
use client::io::ClientRevision;

#[test]
fn generated_revision_data_is_static_distinct_and_fail_closed() {
    let first_274 = for_revision(ClientRevision::R274).expect("revision 274 data");
    let second_274 = for_revision(ClientRevision::R274).expect("revision 274 data again");
    let data_289 = for_revision(ClientRevision::R289).expect("revision 289 data");

    assert!(Arc::ptr_eq(&first_274, &second_274));
    assert!(!Arc::ptr_eq(&first_274, &data_289));
    assert_eq!(first_274.revision(), 274);
    assert_eq!(data_289.revision(), 289);
    assert!(first_274.item_by_alias("castlewars_armour_body").is_none());
    assert!(data_289.item_by_alias("castlewars_armour_body").is_some());

    let err = for_profile(ClientRevision::R274, "not-the-selected-cache")
        .expect_err("foreign cache identity must fail");
    assert!(err.contains("cache identity"), "unexpected error: {err}");
    assert!(
        for_optional_profile(ClientRevision::R274, "not-the-selected-cache")
            .expect("generated data still decodes")
            .is_none()
    );
    assert!(
        for_optional_profile(ClientRevision::R274, first_274.cache_id())
            .expect("matching profile")
            .is_some()
    );
}

#[test]
fn generated_items_food_and_pickpocket_facts_preserve_selected_content() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected data");
        let plate = data
            .item_by_alias("rune_platebody")
            .expect("rune platebody");
        let chain = data
            .item_by_alias("rune_chainbody")
            .expect("rune chainbody");
        assert_eq!(plate.name.as_deref(), Some("Rune platebody"));
        assert_eq!(chain.name.as_deref(), Some("Rune chainbody"));
        assert!(plate.cost > chain.cost);

        let dragonhide: Vec<_> = data
            .items()
            .iter()
            .filter(|item| item.name.as_deref() == Some("Dragonhide"))
            .collect();
        assert!(dragonhide.len() >= 8);
        assert_ne!(dragonhide[0].id, dragonhide[1].id);
        assert_ne!(dragonhide[0].alias, dragonhide[1].alias);

        assert_eq!(data.fixed_food_heal("Lobster"), Some(12));
        assert_eq!(data.fixed_food_heal("Bread"), Some(4));
        assert_eq!(data.fixed_food_heal("Anchovies"), Some(3));
        assert_eq!(
            data.fixed_food_heal("Cabbage"),
            None,
            "a display name shared with a conditional variant is ambiguous"
        );
        assert_eq!(
            data.fixed_food_heal("Ugthanki kebab"),
            None,
            "same display name with conflicting fixed heals is ambiguous"
        );
        assert_eq!(data.fixed_food_heal("Not a food"), None);
        assert_eq!(data.required_thieving("Guard"), Some(40));
        assert_eq!(data.required_thieving("Unknown target"), None);
    }
}

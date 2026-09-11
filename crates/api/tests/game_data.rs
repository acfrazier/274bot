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

#[test]
fn generated_spell_and_staff_facts_match_selected_content() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected data");
        let wind = data.spell("Wind Strike").expect("wind strike");
        assert_eq!(wind.ssb, 0);
        assert_eq!(wind.level, 1);
        assert_eq!(wind.runes[0].name, "Mind rune");
        assert_eq!(wind.runes[0].count, 1);
        assert_eq!(wind.runes[1].name, "Air rune");
        let remaining = data
            .runes_per_cast("Wind Strike", &["Staff of air"])
            .expect("known spell");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].rune, "Mind rune");
        assert!(data.runes_per_cast("Not a spell", &[] as &[&str]).is_none());
        let fire_wave = data
            .runes_per_cast("Fire Wave", &["Mystic fire staff"])
            .expect("fire wave");
        assert_eq!(fire_wave[0].rune, "Blood rune");
        assert_eq!(fire_wave[1].rune, "Air rune");
        assert_eq!(data.spell_button_com("Wind Strike"), 1830);
        assert_eq!(data.spell_button_com("unknown"), -1);
        let autocast = data.autocast_controls().expect("autocast controls");
        assert_eq!(autocast.staff_tab_root, 328);
        assert_eq!(autocast.choose_com, 353);
        assert_eq!(autocast.spell_panel_root, 1829);
        assert_eq!(autocast.toggle_com, 349);
        assert_eq!(autocast.spell_grid_base, 1830);
        assert_eq!(autocast.magic_varp, 108);
        assert_eq!(autocast.selected_value, 2);
        assert_eq!(autocast.armed_value, 3);
        let duel = data.duel_controls().expect("duel controls");
        assert_eq!(duel.select_modal, 6575);
        assert_eq!(duel.confirm_modal, 6412);
        assert_eq!(duel.win_modal, 6733);
        assert_eq!(duel.select_accept, 6674);
        assert_eq!(duel.confirm_accept, 6520);
        assert_eq!(duel.select_partner, 6671);
        assert_eq!(duel.select_status, 6684);
        assert_eq!(duel.confirm_status, 6571);
        let fire: Vec<_> = data
            .staves()
            .iter()
            .filter(|staff| staff.runes.iter().any(|rune| rune.name == "Fire rune"))
            .map(|staff| staff.name.as_str())
            .collect();
        for name in [
            "Staff of fire",
            "Fire battlestaff",
            "Lava battlestaff",
            "Mystic fire staff",
            "Mystic lava staff",
        ] {
            assert!(fire.contains(&name), "missing fire staff {name}");
        }
        assert!(!fire.contains(&"Staff of air"));
        let special = data.special_controls().expect("special controls");
        assert_eq!(special.energy_varp, 300);
        assert_eq!(special.armed_varp, 301);
        assert_eq!(special.max_energy, 1000);
        assert_eq!(data.special_cost("Dragon dagger"), Some(250));
        assert_eq!(data.special_cost("Magic shortbow"), Some(350));
        assert_eq!(data.special_cost("Rune scimitar"), None);
        assert_eq!(data.special_cost("Dragon battleaxe"), None);
        assert_eq!(data.special_bar(425), 7462);
        assert_eq!(data.special_bar(328), -1);
        let varrock = data.teleport("Varrock").expect("Varrock teleport");
        assert_eq!(varrock.component_id, 1164);
        assert_eq!(varrock.level, 25);
        assert_eq!(varrock.x, 3213);
        assert_eq!(varrock.z, 3424);
        assert!(data.teleport("wind strike").is_none());
        assert_eq!(data.teleports().len(), 7);
    }
}

#[test]
fn generated_wearpos_maps_to_loadout_slots_without_guessing_names() {
    use api::game_data::{loadout_slot_for_wearpos, WEARPOS_QUIVER, WEARPOS_RIGHTHAND};

    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected data");
        let scim = data.item_by_alias("rune_scimitar").expect("rune scimitar");
        let helm = data
            .item_by_alias("rune_full_helm")
            .expect("rune full helm");
        let twoh = data.item_by_alias("iron_2h_sword").expect("iron 2h");
        let arrow = data.item_by_alias("bronze_arrow").expect("bronze arrow");
        let cert = data
            .item_by_alias("cert_rune_scimitar")
            .expect("noted scimitar");
        assert_eq!(scim.wear_position, WEARPOS_RIGHTHAND);
        assert_eq!(scim.loadout_slot(), Some("righthand"));
        assert_eq!(helm.loadout_slot(), Some("hat"));
        assert_eq!(helm.wear_position_2, 8);
        assert_eq!(helm.wear_position_3, 11);
        assert_eq!(loadout_slot_for_wearpos(8), None, "head is appearance-only");
        assert_eq!(loadout_slot_for_wearpos(11), None, "jaw is appearance-only");
        assert!(twoh.is_two_handed());
        assert_eq!(arrow.wear_position, WEARPOS_QUIVER);
        assert_eq!(arrow.loadout_slot(), Some("quiver"));
        assert!(cert.is_certificate());
        assert_eq!(cert.loadout_slot(), None);
        let hits = data.search_slot_items("righthand", "rune scimitar", 8);
        assert!(hits
            .iter()
            .any(|hit| hit.id == 1333 && hit.alias == "rune_scimitar"));
        assert!(hits.iter().all(|hit| hit.id != cert.id));
    }
}

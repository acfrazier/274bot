use std::sync::Arc;

use api::game_data::{for_optional_profile, for_profile, for_revision};
use client::io::ClientRevision;

/// Local 289 known-cache identity (versionlist 37214163…).
const LOCAL_289_CACHE_ID: &str = "c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09";
/// Public 289 known-cache identity (versionlist 6dcb7c4a…; config identical).
const PUBLIC_289_CACHE_ID: &str =
    "37cdafd200703150d0f66339b7609944e1ea19d64ad729a72d94e3bcdfa89d92";

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
fn runtime_decoded_content_id_binds_while_deleted_packed_alias_stays_closed() {
    let data_289 = for_revision(ClientRevision::R289).expect("revision 289 data");
    assert!(
        for_profile(
            ClientRevision::R289,
            "cdb2f161c35239f09bf5175648e15e7dbbc4bbf9be4419cea41f7053ccf8b044"
        )
        .is_ok(),
        "the pinned decoded 289 content id must bind generated facts"
    );
    assert_eq!(
        data_289.content_id(),
        Some("cdb2f161c35239f09bf5175648e15e7dbbc4bbf9be4419cea41f7053ccf8b044")
    );
    // The deleted packed equivalence id must stay closed: decoded identity,
    // not a public packed hash allowlist, is the compatibility criterion.
    assert!(
        for_optional_profile(ClientRevision::R289, PUBLIC_289_CACHE_ID)
            .expect("decode")
            .is_none(),
        "the packed public alias no longer unlocks facts on its own"
    );
}

#[test]
fn public_289_audited_identity_binds_and_unknown_same_revision_stays_closed() {
    let data_289 = for_revision(ClientRevision::R289).expect("revision 289 data");
    assert_eq!(data_289.cache_id(), LOCAL_289_CACHE_ID);
    assert_ne!(PUBLIC_289_CACHE_ID, LOCAL_289_CACHE_ID);

    // Decoded identity replaced the packed equivalence list: the runtime
    // binds generated facts by the pinned decoded content id, and a packed
    // transfer id alone (local or public) no longer unlocks anything.
    assert!(
        for_optional_profile(ClientRevision::R289, PUBLIC_289_CACHE_ID)
            .expect("decode")
            .is_none(),
        "the packed public alias must stay closed after decoded identity replaced it"
    );
    assert!(
        for_optional_profile(ClientRevision::R289, LOCAL_289_CACHE_ID)
            .expect("decode")
            .is_some(),
        "local primary identity still binds"
    );

    // Unknown same-revision identity stays closed (not an arbitrary bypass).
    assert!(
        for_optional_profile(ClientRevision::R289, "not-an-audited-289-cache")
            .expect("decode")
            .is_none()
    );
    assert!(
        for_profile(ClientRevision::R289, "not-an-audited-289-cache")
            .unwrap_err()
            .contains("cache identity")
    );

    // Public 289 id must not unlock revision 274 facts.
    assert!(
        for_optional_profile(ClientRevision::R274, PUBLIC_289_CACHE_ID)
            .expect("decode")
            .is_none()
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

        assert_eq!(data.herb_level_default(), Some(3));
        let herbs = data.herbs();
        assert!(
            herbs.len() >= 14,
            "revision {} herb row count",
            revision.as_i32()
        );
        let guam = data.herb_by_key("guam").expect("guam");
        assert_eq!(guam.name, "Guam leaf");
        assert_eq!(guam.level, 3);
        assert_eq!(guam.id, 249);
        assert_eq!(guam.unid_id, 199);
        let marrentill = data.herb_by_key("marrentill").expect("marrentill");
        assert_eq!(marrentill.level, 5);
        let snake = data.herb_by_key("snake weed").expect("snake weed");
        assert_eq!(snake.level, 3);
        assert_eq!(snake.unid_id, 1525);
        assert_eq!(snake.id, 1526);
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

const GREEN_DISPLAY: &[&str] = &[
    "Adamant full helm",
    "Adamantite ore",
    "Bass",
    "Chaos talisman",
    "Coins",
    "Dragon bones",
    "Dragon spear",
    "Dragonhide",
    "Fire rune",
    "Half of a key",
    "Herb",
    "Law rune",
    "Mithril axe",
    "Mithril kiteshield",
    "Mithril spear",
    "Nature rune",
    "Nature talisman",
    "Rune dagger",
    "Rune javelin",
    "Rune spear",
    "Shield left half",
    "Steel battleaxe",
    "Steel platelegs",
    "Uncut diamond",
    "Uncut emerald",
    "Uncut ruby",
    "Uncut sapphire",
    "Water rune",
];

#[test]
fn generated_drop_tables_publish_four_combat_rows_with_alias_evidence() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected data");
        assert_eq!(
            data.drop_tables()
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            ["Giant", "Moss giant", "Fire giant", "Green dragon"]
        );
        assert!(data.drop_table("Hill Giant").is_none());

        let green = data.drop_table("Green dragon").expect("green dragon row");
        assert_eq!(green.npc_alias, "green_dragon");
        assert_eq!(green.npc_id, 941);
        assert_eq!(
            green
                .display_names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            GREEN_DISPLAY
        );
        assert!(
            !green.display_names.iter().any(|name| name == "Bones"),
            "invented Green Bones must stay absent"
        );
        let hide = green
            .items
            .iter()
            .find(|item| item.name == "Dragonhide")
            .expect("Dragonhide alias/id evidence");
        assert_eq!(hide.alias, "dragonhide_green");
        assert_eq!(hide.id, 1753);

        let giant = data.drop_table("Giant").expect("giant row");
        assert!(giant
            .display_names
            .iter()
            .any(|name| name == "Limpwurt root"));
        assert!(giant.display_names.iter().any(|name| name == "Big bones"));
        let bones = giant
            .items
            .iter()
            .find(|item| item.alias == "big_bones")
            .expect("Giant big_bones join");
        assert_eq!(bones.id, 532);
        assert_eq!(bones.name, "Big bones");

        let moss = data.drop_table("Moss giant").expect("moss row");
        assert!(moss.display_names.iter().any(|name| name == "Spinach roll"));
        assert!(moss.display_names.iter().any(|name| name == "Big bones"));

        let fire = data.drop_table("Fire giant").expect("fire row");
        assert!(fire.display_names.iter().any(|name| name == "Big bones"));
        assert!(fire.display_names.iter().any(|name| name == "Lobster"));
        assert!(data.drop_table("unknown").is_none());
    }
}

#[test]
fn generated_prayer_facts_join_fifteen_rows_on_both_revisions() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected data");
        let prayers = data.prayers();
        assert_eq!(prayers.len(), 15, "revision {}", revision.as_i32());
        assert_eq!(prayers[0].name, "Thick Skin");
        assert_eq!(prayers[0].level, 1);
        assert_eq!(prayers[0].button_com, 5609);
        assert_eq!(prayers[0].varp, 83);
        assert_eq!(prayers[0].varp_alias, "prayer0");
        assert_eq!(prayers[14].name, "Protect from Melee");
        assert_eq!(prayers[14].level, 43);
        assert_eq!(prayers[14].button_com, 5623);
        assert_eq!(prayers[14].varp, 97);
        assert_eq!(prayers[14].varp_alias, "prayer14");
        for (index, row) in prayers.iter().enumerate() {
            assert_eq!(row.button_com, 5609 + index as i32);
            assert_eq!(row.varp, 83 + index as i32);
        }
        let burst = data
            .prayer_by_name("Burst of Strength")
            .expect("burst of strength");
        assert_eq!(burst.button_com, 5610);
        assert_eq!(burst.varp, 84);
        assert_eq!(burst.level, 4);
        assert!(data.prayer_by_name("Not a prayer").is_none());
    }
}

//! Behaviour tests for the neutral loadout values: every test fails on a
//! plausible wrong definition (a dropped item, a guessed slot, a changed
//! wire form, a wrong slot table).

use super::*;
use serde_json::json;

#[test]
fn the_slot_table_names_the_eleven_content_slots() {
    assert_eq!(WORN_SLOTS.len(), 11);
    for slot in [
        "hat",
        "back",
        "front",
        "righthand",
        "torso",
        "lefthand",
        "legs",
        "hands",
        "feet",
        "ring",
        "quiver",
    ] {
        assert!(is_worn_slot(slot), "{slot:?} is a worn slot");
    }
    for slot in ["", "cape", "HAT", "mainhand", "ammo"] {
        assert!(!is_worn_slot(slot), "{slot:?} is not a worn slot");
    }
}

#[test]
fn a_carry_row_always_carries_at_least_one() {
    assert_eq!(CarryEntry::new("Lobster", 0).qty, 1);
    assert_eq!(CarryEntry::new("Lobster", 10).qty, 10);
}

#[test]
fn an_unknown_slot_keeps_its_gear_instead_of_dropping_it() {
    let row = Loadout::new("melee").with_slot("cape", "Fire cape");
    assert!(row.worn.is_empty());
    assert_eq!(row.unassigned, vec!["Fire cape"]);
    let row = Loadout::new("melee").with_slot("hat", "Helm");
    assert_eq!(row.worn.get("hat").map(String::as_str), Some("Helm"));
    assert!(row.unassigned.is_empty());
}

#[test]
fn setting_an_unknown_slot_changes_nothing_and_empty_clears() {
    let mut row = Loadout::new("melee").with_slot("hat", "Helm");
    row.set_slot("cape", Some("Fire cape".into()));
    assert!(!row.worn.contains_key("cape"));
    assert!(row.unassigned.is_empty());
    row.set_slot("hat", Some("  ".into()));
    assert!(!row.worn.contains_key("hat"));
    row.set_slot("hat", Some("Coif".into()));
    assert_eq!(row.worn.get("hat").map(String::as_str), Some("Coif"));
}

#[test]
fn the_wire_omits_an_empty_unassigned_row_and_keeps_a_full_one() {
    let bare = Loadout::new("melee").with_slot("hat", "Helm");
    let raw = serde_json::to_value(&bare).unwrap();
    assert_eq!(raw["name"], json!("melee"));
    assert_eq!(raw["worn"], json!({ "hat": "Helm" }));
    assert!(raw.get("unassigned").is_none());
    assert_eq!(raw["carry"], json!([]));

    let full = Loadout::new("trip")
        .with_slot("cape", "Fire cape")
        .with_carry("Lobster", 10);
    let raw = serde_json::to_value(&full).unwrap();
    assert_eq!(raw["unassigned"], json!(["Fire cape"]));
    assert_eq!(raw["carry"], json!([{ "item": "Lobster", "qty": 10 }]));
}

#[test]
fn pre_0_2_8_loadout_rows_are_not_current_loadouts() {
    for row in [
        json!({
            "name": "old",
            "worn": ["helm", "plate"],
            "carry": [{"item": "Lobster", "qty": 1}],
        }),
        json!({
            "name": "old",
            "worn": {},
            "carry": ["Lobster"],
        }),
    ] {
        assert!(parse_loadout(&row).is_none(), "{row}");
        assert!(
            serde_json::from_value::<Loadout>(row).is_err(),
            "older rows are not deserialized as current loadouts"
        );
    }
}

#[test]
fn the_saved_file_probe_recognizes_only_the_older_row_shapes() {
    assert!(loadouts_have_pre_0_2_8_shape(
        br#"[{"name":"old","worn":["Helm"],"carry":[]}]"#
    ));
    assert!(loadouts_have_pre_0_2_8_shape(
        br#"[{"name":"old","worn":{},"carry":["Shark"]}]"#
    ));

    let current = [Loadout::new("melee")
        .with_slot("hat", "Helm")
        .with_carry("Shark", 2)];
    assert!(!loadouts_have_pre_0_2_8_shape(
        &serde_json::to_vec(&current).unwrap()
    ));
    assert!(!loadouts_have_pre_0_2_8_shape(
        br#"[{"name":"current","worn":{"hat":"Helm"},"carry":[{"item":"Shark","qty":2}],"extension":{"worn":["ignored"],"carry":["ignored"]}}]"#
    ));
    assert!(!loadouts_have_pre_0_2_8_shape(br#"[{"worn":["truncated"]"#));
    assert!(!loadouts_have_pre_0_2_8_shape(br#"not json"#));
}

#[test]
fn object_slots_split_known_slots_from_unknown_gear() {
    let row = parse_loadout(&json!({
        "name": "melee",
        "worn": { "hat": "Helm", "cape": "Fire cape", "legs": "  " },
        "unassigned": ["Boots"],
        "carry": [{ "item": "Lobster", "qty": 10 }, { "item": "", "qty": 3 }],
    }))
    .expect("object row parses");
    assert_eq!(row.worn.get("hat").map(String::as_str), Some("Helm"));
    assert!(!row.worn.contains_key("legs"), "blank items are skipped");
    assert_eq!(row.unassigned, vec!["Fire cape", "Boots"]);
    assert_eq!(row.carry, vec![CarryEntry::new("Lobster", 10)]);
}

#[test]
fn rows_without_a_usable_name_are_not_loadouts() {
    assert!(parse_loadout(&json!({ "worn": {} })).is_none());
    assert!(parse_loadout(&json!({ "name": "   " })).is_none());
    assert!(parse_loadout(&json!(null)).is_none());
    let err = serde_json::from_value::<Loadout>(json!({ "worn": {} }));
    assert!(err.is_err(), "the strict form rejects a nameless row");
}

#[test]
fn a_saved_file_parses_row_by_row_so_one_bad_row_keeps_the_rest() {
    let rows: Vec<Loadout> = [json!({ "name": "good" }), json!({ "worn": {} })]
        .iter()
        .filter_map(parse_loadout)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "good");
}

#[test]
fn quantities_survive_a_serialize_deserialize_round_trip() {
    let row = Loadout::new("melee")
        .with_slot("righthand", "Rune scimitar")
        .with_carry("Lobster", 10)
        .with_carry("Prayer potion(4)", 2);
    let raw = serde_json::to_value(&row).unwrap();
    let back: Loadout = serde_json::from_value(raw).unwrap();
    assert_eq!(back, row);
}

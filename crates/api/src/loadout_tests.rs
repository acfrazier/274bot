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
fn a_legacy_array_worn_never_invents_slots() {
    let row = parse_loadout(&json!({
        "name": "old",
        "worn": ["helm", "plate"],
        "carry": ["Lobster"],
    }))
    .expect("legacy row parses");
    assert!(row.worn.is_empty(), "array order must not invent slots");
    assert_eq!(row.unassigned, vec!["helm", "plate"]);
    assert_eq!(row.carry, vec![CarryEntry::new("Lobster", 1)]);
}

#[test]
fn object_slots_split_known_slots_from_unknown_gear() {
    let row = parse_loadout(&json!({
        "name": "melee",
        "worn": { "hat": "Helm", "cape": "Fire cape", "legs": "  " },
        "unassigned": ["Boots"],
        "carry": [{ "item": "Lobster", "qty": 10 }, "Shark", { "item": "", "qty": 3 }],
    }))
    .expect("object row parses");
    assert_eq!(row.worn.get("hat").map(String::as_str), Some("Helm"));
    assert!(!row.worn.contains_key("legs"), "blank items are skipped");
    assert_eq!(row.unassigned, vec!["Fire cape", "Boots"]);
    assert_eq!(
        row.carry,
        vec![CarryEntry::new("Lobster", 10), CarryEntry::new("Shark", 1)]
    );
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

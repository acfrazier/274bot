// Task 1: the item/loc definition views the snapshot reads, plus the thin
// obj-id → name table compiled scripts resolve inventory ids against.

use client::config::LocType;

#[test]
fn table_resolves_id_to_name_and_missing_id_to_none() {
    let mut objs = vec![client::config::ObjType::default(); 3];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    assert_eq!(names.name(1), Some("Bones"));
    assert_eq!(names.name(0), None); // default name is empty
    assert_eq!(names.name(999), None);
}

#[test]
fn table_resolves_name_to_id_for_has_item_lookups() {
    let mut objs = vec![client::config::ObjType::default(); 3];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    assert_eq!(names.by_name("Bones"), Some(1));
    assert_eq!(names.by_name("bones"), None); // exact match only
    assert_eq!(names.by_name("Coins"), None);
}

/// `ObjNames` is now an id → `ItemDefView` table: flags, base value and
/// the name map in alongside the id.
#[test]
fn item_def_view_pins_id_and_flags() {
    let mut objs = vec![client::config::ObjType::default(); 3];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    objs[1].stackable = true;
    objs[1].members = true;
    objs[1].cost = 50;
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let bones = names.item(1).expect("id 1 present");
    assert_eq!(bones.name.as_deref(), Some("Bones"));
    assert!(bones.stackable && bones.members);
    assert_eq!(bones.base_value, 50);
    assert!(names.item(0).is_none());
}

/// An empty `ObjType.name` reads as `None`; the 2004 cache has no
/// explicit `noted` flag, so `noted` is derived as `certlink != -1`.
#[test]
fn item_def_view_maps_empty_name_and_cert_links() {
    let mut objs = vec![client::config::ObjType::default(); 3];
    objs[0].id = 0; // default: empty name, certlink -1 (a normal item)
    objs[2].id = 2;
    objs[2].name = String::new();
    objs[2].certlink = 526;
    objs[2].certtemplate = 799;
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let note = names.item(2).expect("id 2 present");
    assert_eq!(note.name, None, "empty obj name maps to None");
    assert!(note.noted, "2004 noted approximation: certlink != -1");
    assert_eq!(note.certificate_link, 526);
    assert_eq!(note.certificate_template, 799);
    assert!(!names.item(0).expect("id 0 present").noted);
}

/// Unnoted objs have `certlink == -1`; the note sibling (`certtemplate`
/// set, `certlink` pointing back) is the id bank note-mode lands as.
#[test]
fn unnoted_item_certificate_link_is_the_note_sibling() {
    let mut objs = vec![client::config::ObjType::default(); 12];
    objs[10].id = 10;
    objs[10].name = "Rune chainbody".into();
    objs[11].id = 11;
    objs[11].name = "Rune chainbody".into();
    objs[11].certlink = 10;
    objs[11].certtemplate = 799;
    let names = api::obj_names::ObjNames::from_objs(&objs);
    let unnoted = names.item(10).expect("unnoted");
    assert!(!unnoted.noted);
    assert_eq!(
        unnoted.certificate_link, 11,
        "unnoted rows must name the note id so notedOf.get(unnoted) works"
    );
    let note = names.item(11).expect("note");
    assert!(note.noted);
    assert_eq!(note.certificate_link, 10);
}

/// The compat shim survives the generalization: the script crate still
/// resolves ids to names off the same table.
#[test]
fn name_and_by_name_still_resolve_off_the_view_table() {
    let mut objs = vec![client::config::ObjType::default(); 2];
    objs[1].id = 1;
    objs[1].name = "Bones".into();
    let names = api::obj_names::ObjNames::from_objs(&objs);
    assert_eq!(names.name(1), Some("Bones"));
    assert_eq!(names.name(0), None); // empty name reads as None now
    assert_eq!(names.by_name("Bones"), Some(1));
}

/// `LocDefs` is the loc-side id → definition table: name, filtered ops,
/// footprint, block flags, active, force approach.
#[test]
fn loc_defs_maps_name_ops_and_flags() {
    let mut locs = vec![LocType::default(), LocType::default()];
    locs[0].id = 0;
    locs[0].name = "Gate".into();
    locs[0].op = vec![Some("Open".into()), None, Some("Pick-lock".into())];
    locs[0].width = 2;
    locs[0].length = 1;
    locs[0].blockwalk = true;
    locs[0].blockrange = false;
    locs[0].active = true;
    locs[0].forceapproach = 1;
    locs[1].id = 1; // default: empty name, no ops
    let defs = api::obj_names::LocDefs::from_locs(&locs);
    let gate = defs.loc(0).expect("id 0 present");
    assert_eq!(gate.name.as_deref(), Some("Gate"));
    assert_eq!(
        gate.ops,
        vec!["Open", "Pick-lock"],
        "Some ops kept in order, None filtered out"
    );
    assert_eq!(gate.width, 2);
    assert_eq!(gate.length, 1);
    assert!(gate.block_walk);
    assert!(!gate.block_range);
    assert!(gate.active);
    assert_eq!(gate.force_approach, 1);
    assert_eq!(defs.loc(1).expect("id 1 present").name, None);
    assert!(defs.loc(99).is_none());
}

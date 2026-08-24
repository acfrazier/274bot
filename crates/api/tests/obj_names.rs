// Task 1: the thin obj-id → name table compiled scripts resolve inventory
// ids against. Id only — no model/sprites/ops/desc.

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

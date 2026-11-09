#[path = "../build_support/catalog_codegen.rs"]
mod catalog_codegen;

use catalog_codegen::{bank_catalog_rust, cook_catalog_rust, gather_catalog_rust};

#[test]
fn bank_catalog_codegen_preserves_rows_and_escapes_strings() {
    let json = r#"{
        "schema": 1,
        "facts": [
            {
                "name": "First \"Bank\"\\path\nnext",
                "tile": {"x": -1, "z": 2, "level": 3},
                "approach": null,
                "skill": [10, 77],
                "quest": null,
                "setting": "useMageBank",
                "object": {"name": "Bank booth", "op": "Bank"},
                "open_first": null,
                "npc": {"name": "Guard", "op": "Talk-to"},
                "choose": "Bank"
            },
            {
                "name": "Second",
                "tile": {"x": 4, "z": 5, "level": 0},
                "approach": {"x": 6, "z": 7, "level": 1},
                "skill": null,
                "quest": "quest-key",
                "setting": null,
                "object": null,
                "open_first": {"name": "Gate", "op": "Open"},
                "npc": null,
                "choose": null
            }
        ]
    }"#;

    let generated = bank_catalog_rust(json).expect("synthetic bank catalog should parse");
    assert!(generated.contains(r#"name: "First \"Bank\"\\path\nnext""#));
    assert!(generated.contains("tile: WorldTile { x: -1, z: 2, level: 3 }"));
    assert!(generated.contains("skill: Some((10, 77))"));
    assert!(generated.contains("approach: Some(WorldTile { x: 6, z: 7, level: 1 })"));
    assert!(generated.contains("open_first: Some(BankOperation { name: \"Gate\", op: \"Open\" })"));
    assert!(generated.contains("npc: Some(BankOperation { name: \"Guard\", op: \"Talk-to\" })"));
    assert!(generated.find("name: \"First").unwrap() < generated.find("name: \"Second").unwrap());
}

#[test]
fn cook_catalog_codegen_preserves_constants_camps_and_escaped_strings() {
    let json = r#"{
        "schema": 1,
        "facts": {
            "max_surface_cheb": 13,
            "derived_arrive_radius": 15,
            "default_obstacles": ["Tree", "Rock \"big\"\\path\nline"],
            "camps": [
                {
                    "bank": "First",
                    "stand": {"x": 1, "z": 2, "level": 0},
                    "approach": null,
                    "loc": {"x": 3, "z": 4, "level": 1},
                    "loc_name": "First oven",
                    "kind": "oven",
                    "label": "Cook here"
                },
                {
                    "bank": "Second",
                    "stand": {"x": 5, "z": 6, "level": 0},
                    "approach": {"x": 7, "z": 8, "level": 0},
                    "loc": {"x": 9, "z": 10, "level": 0},
                    "loc_name": "Second fire",
                    "kind": "fire",
                    "label": "Nearby"
                }
            ]
        }
    }"#;

    let generated = cook_catalog_rust(json).expect("synthetic cook catalog should parse");
    assert!(generated.contains("pub const MAX_SURFACE_CHEB: i32 = 13;"));
    assert!(generated.contains("pub const DERIVED_ARRIVE_RADIUS: i32 = 15;"));
    assert!(generated.contains(r#""Rock \"big\"\\path\nline""#));
    assert!(generated.contains("kind: CookSurfaceKind::Oven"));
    assert!(generated.contains("kind: CookSurfaceKind::Fire"));
    assert!(generated.contains("approach: Some(WorldTile { x: 7, z: 8, level: 0 })"));
    assert!(generated.find("bank: \"First").unwrap() < generated.find("bank: \"Second").unwrap());
    assert!(!generated.contains("kinds"));
}

#[test]
fn catalog_schema_and_type_errors_name_the_catalog() {
    let bank_schema_error = bank_catalog_rust(r#"{"schema":2,"facts":[]}"#).unwrap_err();
    assert!(bank_schema_error.contains("bank-catalog.json"));
    assert!(bank_schema_error.contains("unsupported schema 2"));

    let bank_type_error = bank_catalog_rust(r#"{"schema":1,"facts":{}}"#).unwrap_err();
    assert!(bank_type_error.contains("bank-catalog.json"));

    let cook_type_error =
        cook_catalog_rust(r#"{"schema":1,"facts":{"max_surface_cheb":"large"}}"#).unwrap_err();
    assert!(cook_type_error.contains("cook-catalog.json"));
}

#[test]
fn optional_catalog_fields_must_be_present_even_when_nullable() {
    let json = r#"{
        "schema": 1,
        "facts": [{
            "name": "Bank",
            "tile": {"x": 1, "z": 2, "level": 0},
            "skill": null,
            "quest": null,
            "setting": null,
            "object": null,
            "open_first": null,
            "npc": null,
            "choose": null
        }]
    }"#;
    let error = bank_catalog_rust(json).unwrap_err();
    assert!(error.contains("bank-catalog.json"));
    assert!(error.contains("missing field `approach`"));
}

#[test]
fn gather_catalog_codegen_preserves_tables_keys_and_escaped_strings() {
    let camp = |name: &str, extra: &str| {
        format!(
            r#"{{
                "name": "{name}",
                "spot": {{"x": 1, "z": 2, "level": 0}},
                "bank_stand": {{"x": 3, "z": 4, "level": 1}},
                "booth_name": null,
                "booth_op": null,
                "verified": true,
                "obstacles": ["door", "gate \"x\""],
                "camp_radius": 12,
                "chase_radius": null,
                "resources": null,
                "avoid_spots": null,
                "sweep": [{{"x": 5, "z": 6, "level": 0}}],
                "bait_vendor": null,
                "notes": null,
                "recommended_combat": null,
                "keys": ["Name", "Spot", "BankStand", "Verified", "Obstacles"{extra}]
            }}"#
        )
    };
    let json = format!(
        r#"{{
        "schema": 1,
        "facts": {{
            "woodcutting_camps": [{}],
            "mining_camps": [{}],
            "fishing_camps": [{}],
            "shilo_water_vendor": {{"keeper": "Obli", "stand": {{"x": 7, "z": 8, "level": 0}}, "item": "Vial"}},
            "fishing": {{
                "methods": [{{"name": "Net", "op": "Net", "pair": "", "gear": [{{"name": "Small fishing net", "min": 1, "restock": 1}}]}}],
                "whirlpool_ids": [403, 404],
                "shop_costs": [["Harpoon", 5]],
                "gerrant_only": ["feather"],
                "gerrant": {{"keeper": "Gerrant", "stand": {{"x": 1, "z": 1, "level": 0}}, "bank_stand": {{"x": 2, "z": 2, "level": 0}}}},
                "harry": {{"keeper": "Harry", "stand": {{"x": 3, "z": 3, "level": 0}}, "bank_stand": {{"x": 4, "z": 4, "level": 0}}}}
            }},
            "cook": {{
                "surfaces": [{{"approach_first": true, "approach": {{"x": 9, "z": 9, "level": 0}}, "stand": {{"x": 1, "z": 2, "level": 0}}, "loc": null, "loc_name": "Range", "kind": "fireplace", "label": null, "notes": "n"}}],
                "plans": [{{"camp": "Seers", "pier": 0, "bank": null}}]
            }},
            "quest_junk": [{{"id": 2399, "name": "Key", "quest": "Demon \"Slayer\""}}]
        }}
    }}"#,
        camp("Wood", ""),
        camp("Mine", ", \"RecommendedCombat\""),
        camp("Pier", "")
    );

    let generated = gather_catalog_rust(&json).expect("synthetic gather catalog should parse");
    assert!(generated
        .camps
        .contains("pub const WOODCUTTING_CAMPS: &[GatherCamp]"));
    assert!(generated
        .camps
        .contains("pub const MINING_CAMPS: &[GatherCamp]"));
    assert!(generated
        .camps
        .contains("pub const FISHING_CAMPS: &[GatherCamp]"));
    assert!(generated
        .camps
        .contains(r#"obstacles: Some(&["door", "gate \"x\""])"#));
    assert!(generated
        .camps
        .contains("camp_radius: Some(12), chase_radius: None"));
    assert!(generated
        .camps
        .contains("sweep: Some(&[WorldTile { x: 5, z: 6, level: 0 }])"));
    assert!(generated.camps.contains(
        "keys: &[CampKey::Name, CampKey::Spot, CampKey::BankStand, CampKey::Verified, CampKey::Obstacles, CampKey::RecommendedCombat]"
    ));
    assert!(generated
        .camps
        .contains("pub const SHILO_WATER_VENDOR: WaterVendor"));
    assert!(generated
        .fishing
        .contains("pub const WHIRLPOOL_IDS: &[i32] = &[403, 404];"));
    assert!(generated
        .fishing
        .contains(r#"GearPiece { name: "Small fishing net", min: 1, restock: 1 }"#));
    assert!(generated
        .fishing
        .contains(r#"pub const FISHING_SHOP_COSTS: &[(&str, i32)] = &[("Harpoon", 5)];"#));
    assert!(generated
        .fishing
        .contains(r#"pub const GERRANT_ONLY_FISHING: &[&str] = &["feather"];"#));
    assert!(generated.cook.contains("kind: FishCookKind::Fireplace"));
    assert!(generated
        .cook
        .contains(r#"FishCampCookPlan { camp: "Seers", pier: Some(0), bank: None }"#));
    assert!(generated
        .quest_junk
        .contains(r#"QuestJunkEntry { id: 2399, name: "Key", quest: "Demon \"Slayer\"" }"#));

    let wrong_schema = json.replacen("\"schema\": 1", "\"schema\": 2", 1);
    assert!(gather_catalog_rust(&wrong_schema)
        .err()
        .expect("unsupported schema is refused")
        .contains("gather-catalog.json"));
}

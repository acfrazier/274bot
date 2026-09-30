//! The JSON query adapters over the typed catalog: error tokens, ordering, coverage and cursor pages, read from
//! the checked 289/274 assets.
use api::game_data::for_revision;
use api::gather_methods::{
    gas_rock_ids, gather_methods, gather_placements, gather_resource, GatherCatalog,
    SceneRegionInput,
};
use api::selected::{ClientRevision, FamilyPreparation};
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};

fn prepare(revision: ClientRevision) -> Arc<GatherCatalog> {
    let data = for_revision(revision).expect("selected data");
    FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
        .expect("worker")
        .join()
        .expect("worker panicked")
        .expect("gathering family prepares")
}

static C289: LazyLock<Arc<GatherCatalog>> = LazyLock::new(|| prepare(ClientRevision::R289));
static C274: LazyLock<Arc<GatherCatalog>> = LazyLock::new(|| prepare(ClientRevision::R274));

fn both() -> [(&'static str, &'static GatherCatalog); 2] {
    [("274", &C274), ("289", &C289)]
}

fn region(min_x: i32, min_z: i32, max_x: i32, max_z: i32, level: i32) -> SceneRegionInput {
    SceneRegionInput {
        min_x,
        min_z,
        max_x,
        max_z,
        level,
    }
}

fn wide() -> SceneRegionInput {
    region(0, 0, 99_999, 99_999, 0)
}

fn rows(value: &Value) -> &Vec<Value> {
    value["rows"].as_array().expect("rows")
}

fn method_ids(value: &Value) -> Vec<&str> {
    rows(value)
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect()
}

fn placed(value: &Value) -> Vec<(i64, i64, i64, i64)> {
    rows(value)
        .iter()
        .map(|row| {
            (
                row["id"].as_i64().unwrap(),
                row["x"].as_i64().unwrap(),
                row["z"].as_i64().unwrap(),
                row["plane"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn row<'a>(value: &'a Value, id: &str) -> &'a Value {
    rows(value)
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("no row {id}"))
}

#[test]
fn an_absent_family_is_a_token_never_an_empty_list() {
    assert_eq!(
        gather_methods(None, Some("wc")),
        Err("family-unavailable:gather_methods"),
        "absence outranks a bad skill"
    );
    assert_ne!(
        gather_methods(None, None),
        Ok(json!({ "rows": [], "coverage": [] }))
    );
    assert_eq!(
        gather_resource(None, "iron"),
        Err("family-unavailable:gather_methods")
    );
    assert_eq!(
        gather_resource(None, "not-a-resource"),
        Err("family-unavailable:gather_methods")
    );
    assert_eq!(gas_rock_ids(None), Err("family-unavailable:gather_methods"));
    for resource in ["oak", "nope", "", "   "] {
        assert_eq!(
            gather_placements(None, resource, &wide(), None, 64),
            Err("family-unavailable:gather_placements"),
            "{resource:?}"
        );
    }
}

#[test]
fn methods_group_by_skill_and_coverage_ignores_the_skill_filter() {
    for (revision, catalog) in both() {
        let all = gather_methods(Some(catalog), None).unwrap();
        assert_eq!(rows(&all).len(), catalog.methods().len(), "{revision}");
        let mut order: Vec<&str> = rows(&all)
            .iter()
            .map(|row| row["skill"].as_str().unwrap())
            .collect();
        order.dedup();
        assert_eq!(
            order,
            ["woodcutting", "mining", "fishing"],
            "{revision}: content order, one block each"
        );

        for (raw, skill) in [
            ("woodcutting", "woodcutting"),
            ("  Mining ", "mining"),
            ("FISHING", "fishing"),
        ] {
            let some = gather_methods(Some(catalog), Some(raw)).unwrap();
            let expected: Vec<&str> = rows(&all)
                .iter()
                .filter(|row| row["skill"] == skill)
                .map(|row| row["id"].as_str().unwrap())
                .collect();
            assert_eq!(method_ids(&some), expected, "{revision} {raw:?}");
            assert_eq!(
                some["coverage"], all["coverage"],
                "{revision}: coverage is the pin's, not the filter's"
            );
        }
        for bad in ["wc", "", "  ", "woods", "fishing skill"] {
            assert_eq!(
                gather_methods(Some(catalog), Some(bad)),
                Err("unknown-skill"),
                "{revision} {bad:?}"
            );
        }
    }
}

#[test]
fn coverage_lists_every_gap_and_every_unclassified_rock() {
    let all = gather_methods(Some(&C289), None).unwrap();
    let coverage = all["coverage"].as_array().unwrap();
    let find = |class: &str, method: &str, cell: &str| {
        coverage
            .iter()
            .find(|entry| {
                entry["class"] == class && entry["method"] == method && entry["cell"] == cell
            })
            .cloned()
    };
    let jungle =
        find("unknown", "woodcutting.jungle", "targets").expect("jungle targets are unknown");
    assert_eq!(jungle["code"], "no-resource-target");
    assert!(!jungle["sources"].as_array().unwrap().is_empty());
    assert_eq!(
        find("partial", "mining.copper", "products").expect("gem rolls make copper partial")
            ["code"],
        "incidental-gem-roll"
    );
    let slide = coverage
        .iter()
        .find(|entry| entry["class"] == "unclassified" && entry["id"] == 2634)
        .expect("the rockslide is listed");
    assert_eq!(
        (slide["alias"].as_str(), slide["code"].as_str()),
        (Some("herorockslide"), Some("custom-handler"))
    );
    assert_eq!(slide["skill"], "mining");
    assert!(
        coverage
            .iter()
            .all(|entry| entry["method"] != "woodcutting.oak"),
        "a fully known method has no coverage entry"
    );
    assert!(coverage
        .iter()
        .all(|entry| !entry["code"].as_str().unwrap().is_empty()));
}

#[test]
fn method_rows_keep_every_fact_next_to_its_completeness() {
    for (revision, catalog) in both() {
        let all = gather_methods(Some(catalog), None).unwrap();
        let oak = row(&all, "woodcutting.oak");
        assert_eq!(oak["qualification"], "complete", "{revision}");
        assert_eq!(oak["resource_key"], "oak");
        assert_eq!(oak["op"], json!({ "slot": 1, "label": "Chop down" }));
        assert_eq!(oak["loc_ids"], json!([{ "alias": "oaktree", "id": 1281 }]));
        assert_eq!(
            oak["empty_ids"],
            json!([{ "alias": "evergreen_large_stump", "id": 1355 }])
        );
        assert_eq!(oak["products"]["state"], "known");
        assert_eq!(
            oak["products"]["value"],
            json!([{ "alias": "oak_logs", "id": 1521, "level": 15 }])
        );

        let copper = row(&all, "mining.copper");
        assert_eq!(copper["qualification"], "partial");
        assert_eq!(copper["products"]["state"], "partial");
        assert_eq!(copper["products"]["gaps"][0]["code"], "incidental-gem-roll");
        let hazards: Vec<i64> = copper["hazard_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id["id"].as_i64().unwrap())
            .collect();
        assert_eq!(
            hazards,
            [2119, 2120],
            "{revision}: gas variants are hazards, not resources"
        );
        let resources: Vec<i64> = copper["loc_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id["id"].as_i64().unwrap())
            .collect();
        assert_eq!(
            resources,
            [2090, 2091, 3042],
            "{revision}: the tutorial copper rock is typed"
        );

        let tin = row(&all, "mining.tin");
        let tin_resources: Vec<i64> = tin["loc_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id["id"].as_i64().unwrap())
            .collect();
        assert_eq!(
            tin_resources,
            [2094, 2095, 3043],
            "{revision}: the tutorial tin rock is typed"
        );

        // Respawn is per target: three limestone rocks, three rates.
        let limestone = row(&all, "mining.limestone");
        let raws: Vec<i64> = limestone["targets"]["value"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|target| target["class"] == "resource")
            .map(|target| target["respawn"]["value"]["raw"].as_i64().unwrap())
            .collect();
        assert_eq!(raws, [10, 20, 40], "{revision}");
        let essence = row(&all, "mining.rune stones");
        assert_eq!(
            essence["targets"]["value"][0]["respawn"],
            json!({ "state": "known", "value": null })
        );

        // A wood content cannot resolve keeps its identity and says every fact about it is unknown.
        let jungle = row(&all, "woodcutting.jungle");
        assert_eq!(jungle["resource_key"], "jungle");
        for cell in [
            "targets",
            "products",
            "tools",
            "consumes",
            "requirements",
            "placements",
        ] {
            assert_eq!(jungle[cell]["state"], "unknown", "{revision} {cell}");
        }
        assert_eq!(jungle["qualification"], "partial");
        assert_eq!(jungle["op"], Value::Null);

        let lure = row(&all, "fishing.freshfish.op1");
        assert_eq!(lure["op"], json!({ "slot": 1, "label": "Lure" }));
        assert_eq!(
            lure["consumes"]["value"],
            json!([{ "alias": "feather", "id": 314, "count": 1 }])
        );
        assert_eq!(lure["tools"]["value"][0]["alias"], "fly_fishing_rod");
        assert_eq!(lure["tools"]["value"][0]["use_gate"], Value::Null);
        let spots: Vec<i64> = lure["npc_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id["id"].as_i64().unwrap())
            .collect();
        assert!(spots.contains(&309) && spots.contains(&1189), "{revision}");
        assert!(lure["loc_ids"].as_array().unwrap().is_empty());
    }
}

#[test]
fn resource_lookup_matches_the_trimmed_key_only() {
    for (revision, catalog) in both() {
        for name in ["iron", "Iron", "  IRON "] {
            let hit = gather_resource(Some(catalog), name).unwrap();
            assert_eq!(method_ids(&hit), ["mining.iron"], "{revision} {name:?}");
        }
        for name in [
            "Rocks",
            "",
            "   ",
            "Iron ore",
            "ironrock1",
            "oaktree",
            "1281",
            "freshfish",
        ] {
            assert_eq!(
                gather_resource(Some(catalog), name),
                Err("unknown-resource"),
                "{revision} {name:?}"
            );
        }
        // A key content does not resolve to a placed resource still names its method.
        let jungle = gather_resource(Some(catalog), "jungle").unwrap();
        assert_eq!(method_ids(&jungle), ["woodcutting.jungle"]);
        // Fish are named by what they yield, and two methods can yield the same fish.
        let trout = gather_resource(Some(catalog), "raw_trout").unwrap();
        assert_eq!(
            method_ids(&trout),
            ["fishing.loc_2027.op1", "fishing.freshfish.op1"],
            "{revision}"
        );
    }
}

#[test]
fn placements_are_content_rows_in_ascending_order() {
    for (revision, catalog) in both() {
        let oak = gather_placements(
            Some(catalog),
            "  OAK  ",
            &region(2355, 3412, 2356, 3425, 0),
            None,
            64,
        )
        .unwrap();
        assert_eq!(
            placed(&oak),
            [(1281, 2355, 3425, 0), (1281, 2356, 3412, 0)],
            "{revision}"
        );
        assert_eq!(oak["truncated"], false);
        assert_eq!(oak["next"], Value::Null);
        assert_eq!(oak["qualification"], "complete");
        assert_eq!(
            oak["resource_ids"],
            json!([{ "alias": "oaktree", "id": 1281 }]),
            "{revision}: the stump is not a resource"
        );
        for row in rows(&oak) {
            assert_eq!(row["method"], "woodcutting.oak");
            assert_eq!(row["loc_id"], 1281);
            assert_eq!(row["alias"], "oaktree");
            assert!(row.get("npc_id").is_none());
        }

        let magic = gather_placements(
            Some(catalog),
            "magic",
            &region(2696, 3396, 2698, 3424, 0),
            None,
            64,
        )
        .unwrap();
        assert_eq!(
            placed(&magic),
            [
                (1306, 2696, 3423, 0),
                (1306, 2698, 3396, 0),
                (1306, 2698, 3398, 0)
            ],
            "{revision}"
        );
        assert_eq!(
            magic["resource_ids"],
            json!([{ "alias": "magictree", "id": 1306 }])
        );
    }
}

#[test]
fn a_region_with_no_placements_is_a_complete_empty_page_that_still_names_the_resource() {
    for (revision, catalog) in both() {
        let names = json!([{ "alias": "oaktree", "id": 1281 }]);
        for empty in [
            region(3000, 3000, 3001, 3001, 0),
            region(2356, 3425, 2355, 3412, 0),
            region(2355, 3412, 2356, 3425, 2),
        ] {
            let page = gather_placements(Some(catalog), "oak", &empty, None, 64).unwrap();
            assert!(rows(&page).is_empty(), "{revision} {empty:?}");
            assert_eq!(page["truncated"], false);
            assert_eq!(
                page["qualification"], "complete",
                "{revision}: coverage is complete, so empty is honest"
            );
            assert_eq!(page["resource_ids"], names);
        }
    }
}

#[test]
fn placement_pages_resume_by_cursor_and_say_when_they_are_cut() {
    for (revision, catalog) in both() {
        let first = gather_placements(Some(catalog), "normal", &wide(), None, 64).unwrap();
        assert_eq!(rows(&first).len(), 64, "{revision}");
        assert_eq!(first["truncated"], true);
        let last = rows(&first).last().unwrap()["spot"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            first["next"],
            last.as_str(),
            "the cursor is the last returned spot"
        );

        let mut spots: Vec<u32> = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page =
                gather_placements(Some(catalog), "normal", &wide(), after.as_deref(), 64).unwrap();
            spots.extend(
                rows(&page)
                    .iter()
                    .map(|row| row["spot"].as_str().unwrap().parse::<u32>().unwrap()),
            );
            if page["truncated"] == false {
                assert_eq!(page["next"], Value::Null);
                break;
            }
            after = page["next"].as_str().map(str::to_owned);
        }
        let expected = catalog
            .methods()
            .iter()
            .find(|m| &*m.id.0 == "woodcutting.normal")
            .unwrap();
        let total = match &expected.spots {
            api::selected::Knowledge::Known(rows) => {
                rows.iter().filter(|spot| spot.origin.level == 0).count()
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(
            spots.len(),
            total,
            "{revision}: pages tile every level-0 placement"
        );
        assert!(spots.windows(2).all(|pair| pair[0] < pair[1]));

        // A tighter limit is a prefix of the larger page.
        let one = gather_placements(Some(catalog), "normal", &wide(), None, 1).unwrap();
        assert_eq!(rows(&one)[0], rows(&first)[0]);
        assert_eq!(one["truncated"], true);
        // Fewer rows than the limit is not truncated.
        let small = gather_placements(
            Some(catalog),
            "magic",
            &region(2700, 3390, 2710, 3400, 0),
            None,
            64,
        )
        .unwrap();
        assert_eq!(small["truncated"], false, "{revision}");
        assert_eq!(rows(&small).len(), 2);

        for bad in ["", "abc", "-1", "1.5", "99999999999"] {
            assert_eq!(
                gather_placements(Some(catalog), "normal", &wide(), Some(bad), 64),
                Err("invalid-args"),
                "{revision} cursor {bad:?}"
            );
        }
    }
}

#[test]
fn every_gather_skill_has_placements_and_ambiguous_fish_keys_span_their_methods() {
    for (revision, catalog) in both() {
        let iron = gather_placements(Some(catalog), "Iron", &wide(), None, 64).unwrap();
        assert_eq!(
            iron["qualification"], "complete",
            "{revision}: mining placements are no longer an unknown mark"
        );
        assert!(!rows(&iron).is_empty());
        assert!(rows(&iron).iter().all(|row| row["method"] == "mining.iron"));
        let ids: Vec<i64> = iron["resource_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id["id"].as_i64().unwrap())
            .collect();
        assert!(rows(&iron)
            .iter()
            .all(|row| ids.contains(&row["id"].as_i64().unwrap())));
        assert!(
            !ids.iter().any(|id| (2119..=2140).contains(id)),
            "gas variants are not resource ids"
        );

        // Shrimp come from a net spot type and from a categorised spot type: both methods answer.
        let shrimp = gather_placements(Some(catalog), "raw_shrimp", &wide(), None, 64).unwrap();
        let mut methods: Vec<&str> = rows(&shrimp)
            .iter()
            .map(|row| row["method"].as_str().unwrap())
            .collect();
        methods.dedup();
        assert_eq!(
            methods,
            ["fishing.category_632.op1", "fishing.saltfish.op1"],
            "{revision}"
        );
        assert!(rows(&shrimp)
            .iter()
            .all(|row| row["kind"] == "npc" && row.get("npc_id").is_some()));
        let spots: Vec<u32> = rows(&shrimp)
            .iter()
            .map(|row| row["spot"].as_str().unwrap().parse().unwrap())
            .collect();
        assert!(
            spots.windows(2).all(|pair| pair[0] < pair[1]),
            "one ascending id space across methods"
        );
    }
}

#[test]
fn unresolved_keys_and_unknown_coverage_are_never_an_empty_answer() {
    for (revision, catalog) in both() {
        for resource in [
            "freshfish",
            "rarefish",
            "category_453",
            "",
            "   ",
            "oak tree",
            "oaktree",
            "1281",
            "Iron ore",
            "ironrock1",
            "logs",
        ] {
            assert_eq!(
                gather_placements(Some(catalog), resource, &wide(), None, 64),
                Err("unknown-resource"),
                "{revision} {resource:?}"
            );
        }
        // Jungle trees are a known key whose placements content cannot resolve.
        let jungle = gather_placements(Some(catalog), "jungle", &wide(), None, 64).unwrap();
        assert!(rows(&jungle).is_empty());
        assert_eq!(jungle["qualification"], "unknown", "{revision}");
        assert_eq!(jungle["truncated"], false);
        assert_eq!(jungle["resource_ids"], json!([]));
        assert_eq!(jungle["gaps"][0]["method"], "woodcutting.jungle");
        assert_eq!(jungle["gaps"][0]["gap"]["code"], "no-resource-target");
    }
}

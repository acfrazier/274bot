use crate::grid::StepGrid;
use crate::tile::{chebyshev, Tile};

#[test]
fn fixture_center_is_walkable_and_origin_is_not_out_of_range() {
    let g = StepGrid::fixture_open_3x3();
    let c = Tile {
        x: 1,
        z: 1,
        level: 0,
    };
    assert!(g.walkable(c));
    assert!(!g.walkable(Tile {
        x: -1,
        z: 0,
        level: 0
    }));
    assert_eq!(
        chebyshev(
            c,
            Tile {
                x: 3,
                z: 1,
                level: 0
            }
        ),
        2
    );
}

#[test]
fn contains_is_bounds_and_level_only() {
    let g = StepGrid::fixture_open_3x3();
    assert!(g.contains(Tile {
        x: 0,
        z: 0,
        level: 0
    }));
    assert!(g.contains(Tile {
        x: 2,
        z: 2,
        level: 0
    }));
    assert!(!g.contains(Tile {
        x: 3,
        z: 0,
        level: 0
    }));
    assert!(!g.contains(Tile {
        x: 0,
        z: 0,
        level: 1
    }));
}

#[test]
fn planner_source_scanner_rejects_production_seed_value_reads() {
    use crate::test_support::source_scan::scan_package;
    use std::path::Path;

    let inventory: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tools/architecture/nav-clippy-inventory.json"
    ))
    .expect("the generated Clippy inventory is valid JSON");
    let value_paths = inventory["value_items"]
        .as_array()
        .expect("the generated inventory lists seed values")
        .iter()
        .map(|item| item["path"].as_str().expect("each seed value has a path"))
        .collect::<Vec<_>>();
    let report = scan_package(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        "nav",
        &["api", "client"],
        &value_paths,
    );
    assert!(
        report.findings.is_empty(),
        "planner source has unsupported, unattached, or production seed-value paths: {:#?}",
        report.findings
    );
}

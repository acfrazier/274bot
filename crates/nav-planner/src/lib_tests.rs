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

#[test]
fn workspace_path_gate_scans_without_planner_scope_rules() {
    use crate::test_support::source_scan::{
        scan_workspace_paths, scan_workspace_sources, SourceMap,
    };
    use std::path::{Path, PathBuf};

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the planner crate is under the workspace crates directory");
    let paths = scan_workspace_paths(workspace_root, "nav")
        .expect("the workspace path scan accepts unrelated source syntax");
    let has_unrenamed_package = workspace_root
        .join("crates")
        .join("nav")
        .join("Cargo.toml")
        .is_file();
    assert_eq!(
        !paths.is_empty(),
        has_unrenamed_package,
        "the legacy root is present only before the crate rename: {paths:#?}"
    );

    let planted_path = ["nav", "router", "Route"].join("::");
    let source = [
        r#"
            #[cfg_attr(any(), allow(dead_code))]
            mod local {
                macro_rules! home {
                    () => { $crate::home() };
                }
            }
            use "#,
        planted_path.as_str(),
        ";
",
    ]
    .concat();
    let mut sources = SourceMap::new();
    sources.insert(PathBuf::from("crates/synthetic.rs"), source);
    let planted = scan_workspace_sources(&sources, "nav")
        .expect("cfg_attr and $crate do not block path collection");
    assert!(
        planted.iter().any(|path| path.path == planted_path),
        "the scanner finds a planted crate path: {planted:#?}"
    );
}

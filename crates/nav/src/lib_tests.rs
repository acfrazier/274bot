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

const ACTION_SEEDS: &[&str] = &[
    "api::interact",
    "api::settle",
    "api::prot",
    "client::client",
    "client::io::client_prot",
    "client::io::client_prot_289",
    "client::io::client_stream",
];

const EXPECTED_ACTION_CLOSURE: &[&str] = &[
    "api::interact",
    "api::prot",
    "api::settle",
    "client::CheatAdmission",
    "client::CheatRefusal",
    "client::CheatSend",
    "client::client",
    "client::io::ClientProt",
    "client::io::ClientProt289",
    "client::io::ClientStream",
    "client::io::client_prot",
    "client::io::client_prot_289",
    "client::io::client_stream",
    "client::io::map_client_prot",
];

fn action_closure() -> crate::test_support::source_scan::ClosureReport {
    let package_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let api_src = package_root.join("../api/src");
    let client_src = package_root.join("../../vendor/fr-client-rust/crates/client/src");
    crate::test_support::source_scan::compute_deny_closure(
        &[("api", api_src.as_path()), ("client", client_src.as_path())],
        ACTION_SEEDS,
    )
}

fn action_report(source: &str) -> crate::test_support::source_scan::ScanReport {
    crate::test_support::source_scan::scan_text(
        source,
        "src/lib.rs",
        env!("CARGO_PKG_NAME"),
        crate::test_support::source_scan::Scope::Production,
        &["api", "client"],
        EXPECTED_ACTION_CLOSURE,
    )
}

fn assert_scanner_findings(
    fixture: &str,
    report: &crate::test_support::source_scan::ScanReport,
    expected: &[(crate::test_support::source_scan::FindingKind, Option<&str>)],
) {
    let mut actual: Vec<_> = report
        .findings
        .iter()
        .map(|finding| (finding.kind, finding.path.as_deref()))
        .collect();
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected, "fixture {fixture}: {:#?}", report.findings);
}

#[test]
fn production_source_does_not_name_action_capabilities() {
    let closure = action_closure();
    assert!(
        closure.findings.is_empty(),
        "deny closure could not be resolved: {:#?}",
        closure.findings
    );
    let denied: Vec<_> = closure.denied.iter().map(String::as_str).collect();
    let report = crate::test_support::source_scan::scan_package(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        env!("CARGO_PKG_NAME"),
        &["api", "client"],
        &denied,
    );
    assert!(
        report.findings.is_empty(),
        "planner source has denied or unsupported paths: {:#?}",
        report.findings
    );
}

#[test]
fn action_deny_closure_matches_public_reexports_exactly() {
    let closure = action_closure();
    assert!(
        closure.findings.is_empty(),
        "deny closure could not be resolved: {:#?}",
        closure.findings
    );
    let expected: std::collections::BTreeSet<_> = EXPECTED_ACTION_CLOSURE
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    assert_eq!(closure.denied, expected);
}

#[test]
fn public_glob_inside_a_denied_namespace_does_not_expand_closure() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let files: SourceMap = BTreeMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod interact;\n".to_owned(),
        ),
        (
            PathBuf::from("interact.rs"),
            "pub mod driver;\npub use driver::*;\npub struct Internal;\n".to_owned(),
        ),
        (
            PathBuf::from("interact/driver.rs"),
            "pub fn walk() {}\n".to_owned(),
        ),
    ]);
    let closure =
        compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["api::interact".to_owned()])
    );
}

#[test]
fn unresolved_local_glob_in_an_allowed_namespace_fails_closed() {
    use crate::test_support::source_scan::{
        compute_deny_closure_sources, FindingKind, SourceMap,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let files: SourceMap = BTreeMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod exposed;\n".to_owned(),
        ),
        (
            PathBuf::from("exposed.rs"),
            "pub use crate::missing::*;\n".to_owned(),
        ),
    ]);
    let closure =
        compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(
        closure.findings[0].kind,
        FindingKind::UnsupportedReexportGlob
    );
    assert_eq!(
        closure.findings[0].path.as_deref(),
        Some("api::missing::*")
    );
}

#[test]
fn glob_chain_exposes_and_tracks_reexports_from_denied_items() {
    use crate::test_support::source_scan::{
        compute_deny_closure_sources, SourceMap,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let files: SourceMap = BTreeMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod exposed;\npub mod relay;\npub mod interact;\n".to_owned(),
        ),
        (
            PathBuf::from("exposed.rs"),
            "pub use crate::relay::*;\n".to_owned(),
        ),
        (
            PathBuf::from("relay.rs"),
            "pub use crate::interact::Driver;\n".to_owned(),
        ),
        (
            PathBuf::from("interact.rs"),
            "pub struct Driver;\n".to_owned(),
        ),
    ]);
    let closure =
        compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from([
            "api::exposed::Driver".to_owned(),
            "api::interact".to_owned(),
            "api::relay::Driver".to_owned(),
        ])
    );
}
#[test]
fn aliases_are_scoped_to_inline_modules() {
    let report = action_report(
        "mod a { use api::interact as x; fn f() { x::walk(); } }\
         mod b { use api::snapshot as x; fn f() { x::read(); } }",
    );
    assert_scanner_findings(
        "module-local aliases",
        &report,
        &[
            (crate::test_support::source_scan::FindingKind::UsePath, Some("api::interact")),
            (
                crate::test_support::source_scan::FindingKind::QualifiedPath,
                Some("api::interact::walk"),
            ),
        ],
    );
}


#[test]
fn source_scanner_matches_all_twenty_four_design_fixtures() {
    use crate::test_support::source_scan::{scan_sources, FindingKind, SourceMap};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    let cases = vec![
        (
            "1",
            "use api::{interact::Interactions, settle::Settle};",
            vec![
                (FindingKind::UsePath, Some("api::interact::Interactions")),
                (FindingKind::UsePath, Some("api::settle::Settle")),
            ],
        ),
        (
            "2",
            "use api::interact as ix; fn f() { ix::walk(1, 2); }",
            vec![
                (FindingKind::UsePath, Some("api::interact")),
                (FindingKind::QualifiedPath, Some("api::interact::walk")),
            ],
        ),
        (
            "3",
            "pub use api::settle::Outcome;",
            vec![(FindingKind::UsePath, Some("api::settle::Outcome"))],
        ),
        (
            "4",
            "macro_rules! m { () => { api::interact::walk(0, 0) }; }",
            vec![(FindingKind::MacroTokenPath, Some("api::interact::walk"))],
        ),
        (
            "5",
            "//! uses api::interact\n/// see [api::settle]\npub struct Docs;",
            vec![],
        ),
        (
            "6",
            "#[cfg(test)] mod t { use api::interact::Driver; }",
            vec![],
        ),
        (
            "7",
            "use client::client::Client;",
            vec![(FindingKind::UsePath, Some("client::client::Client"))],
        ),
        (
            "8",
            "use client::dash3d::CollisionFlag;",
            vec![],
        ),
        (
            "9",
            "fn f() { let _ = api::interact::walk; }",
            vec![(FindingKind::QualifiedPath, Some("api::interact::walk"))],
        ),
        (
            "10",
            "use api::*;",
            vec![(FindingKind::AncestorGlob, Some("api::*"))],
        ),
        (
            "11",
            "#[cfg(any(test, unix))] mod m { use api::interact::Driver; }",
            vec![(FindingKind::UsePath, Some("api::interact::Driver"))],
        ),
        (
            "12",
            "#[cfg(all(test, target_os = \"macos\"))] mod m { use api::interact::Driver; }",
            vec![],
        ),
        (
            "13",
            "use client::io::client_prot::ClientProt;",
            vec![(
                FindingKind::UsePath,
                Some("client::io::client_prot::ClientProt"),
            )],
        ),
        (
            "14",
            "use client::io::ClientProt289; use client::CheatSend;",
            vec![
                (FindingKind::UsePath, Some("client::io::ClientProt289")),
                (FindingKind::UsePath, Some("client::CheatSend")),
            ],
        ),
        (
            "15",
            "use client::io::*; use client::dash3d::*;",
            vec![(FindingKind::AncestorGlob, Some("client::io::*"))],
        ),
        (
            "16",
            "macro_rules! m { () => { use api::{interact::Driver}; }; }",
            vec![(FindingKind::MacroTokenPath, Some("api::interact::Driver"))],
        ),
        (
            "17",
            "macro_rules! m { ($p:path) => { $p::walk(0, 0) }; }",
            vec![(FindingKind::UnsupportedMacroForm, None)],
        ),
        (
            "18",
            "use api as a; fn f() { a::interact::walk(0, 0); }",
            vec![(
                FindingKind::QualifiedPath,
                Some("api::interact::walk"),
            )],
        ),
        (
            "19",
            "extern crate client as c; fn f() -> c::client::Client { loop {} }",
            vec![(
                FindingKind::QualifiedPath,
                Some("client::client::Client"),
            )],
        ),
        (
            "22",
            "#[cfg_attr(feature = \"x\", cfg(test))] mod m {}",
            vec![(FindingKind::UnsupportedCfgAttr, None)],
        ),
        (
            "23",
            "mod foo { pub struct Bar; } pub use crate::foo::Bar;",
            vec![],
        ),
    ];
    for (number, source, expected) in cases {
        assert_scanner_findings(number, &action_report(source), &expected);
    }

    let mut unattached = SourceMap::new();
    unattached.insert(PathBuf::from("src/lib.rs"), "mod a;".to_owned());
    unattached.insert(PathBuf::from("src/a.rs"), "pub struct A;".to_owned());
    unattached.insert(
        PathBuf::from("src/stray.rs"),
        "use api::interact::Driver;".to_owned(),
    );
    let report = scan_sources(&unattached, "nav", &["api", "client"], EXPECTED_ACTION_CLOSURE);
    assert_scanner_findings(
        "20 unattached file",
        &report,
        &[(FindingKind::UnattachedFile, None)],
    );

    let mut attached_test = SourceMap::new();
    attached_test.insert(
        PathBuf::from("src/lib.rs"),
        "#[cfg(test)] #[path = \"x_tests.rs\"] mod tests;".to_owned(),
    );
    attached_test.insert(
        PathBuf::from("src/x_tests.rs"),
        "use api::interact::Driver;".to_owned(),
    );
    let report = scan_sources(&attached_test, "nav", &["api", "client"], EXPECTED_ACTION_CLOSURE);
    assert!(report.findings.is_empty(), "{:#?}", report.findings);

    let mut attached_production = SourceMap::new();
    attached_production.insert(
        PathBuf::from("src/lib.rs"),
        "#[path = \"x_tests.rs\"] mod tests;".to_owned(),
    );
    attached_production.insert(
        PathBuf::from("src/x_tests.rs"),
        "use api::interact::Driver;".to_owned(),
    );
    let report = scan_sources(
        &attached_production,
        "nav",
        &["api", "client"],
        EXPECTED_ACTION_CLOSURE,
    );
    assert_scanner_findings(
        "21 production attachment",
        &report,
        &[(FindingKind::UsePath, Some("api::interact::Driver"))],
    );

}

#[test]
fn source_scanner_visits_trait_bounds_types_patterns_and_attribute_metas() {
    use crate::test_support::source_scan::FindingKind;

    let report = action_report(
        "#[api::interact::check] fn f<T: api::interact::Bound>(value: api::interact::Value) {\
         let api::interact::Variant = value;\
         }",
    );
    assert_scanner_findings(
        "path node coverage",
        &report,
        &[
            (
                FindingKind::AttributePath,
                Some("api::interact::check"),
            ),
            (
                FindingKind::QualifiedPath,
                Some("api::interact::Bound"),
            ),
            (
                FindingKind::QualifiedPath,
                Some("api::interact::Value"),
            ),
            (
                FindingKind::QualifiedPath,
                Some("api::interact::Variant"),
            ),
        ],
    );
}

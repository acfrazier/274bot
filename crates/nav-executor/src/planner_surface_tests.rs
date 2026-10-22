use nav::test_support::source_scan::{
    scan_package, scan_sources, scan_text, FindingKind, PathKind, ScanReport, Scope, SourceMap,
    SourcePath,
};
use std::path::{Path, PathBuf};

// S2 changes this one root from `nav` to `nav_planner`.
const PLANNER_ROOT: &str = "nav";

const PRODUCTION_ALLOW: &[&str] = &[
    "contract::RoutePlanLease",
    "router::Route",
    "router::Leg",
    "router::CompactRouteLeg",
    "transport::TransportEdge",
    "transport::TransportKind",
    "transport::DoorDir",
    "executor_adapter::plan_leg_count",
    "executor_adapter::plan_retained_leg",
    "executor_adapter::plan_owned_leg",
    "executor_adapter::is_slashable_web",
    "executor_adapter::is_dialogue_door_loc",
    "executor_adapter::select_web_action",
    "executor_adapter::CELLAR_SHIFT",
    "executor_adapter::SHANTAY_HENGE_LOC_ID",
    "executor_adapter::AL_KHARID_TOLL_LEFT_LOC_ID",
    "executor_adapter::AL_KHARID_TOLL_RIGHT_LOC_ID",
    "executor_adapter::ARDOUGNE_WILDERNESS_LEVER_LOC_ID",
    "world_state::WorldState",
    "world_state::EntranceRequirement",
    "quest_gates::QuestEvidence",
    "quest_gates::QuestGates",
    "essence::EssenceSession",
    "essence::essence_session_for_wizard",
    "essence::in_essence_mine",
    "essence::is_essence_entry_edge",
    "essence::ESSENCE_MINE_EXIT_ARRIVE_RADIUS",
    "router::GridRoute",
    "router::GridLeg",
    "tile::Tile",
    "tile::chebyshev",
    "arrival::arrived",
];

const TEST_ALLOW: &[&str] = &[
    "test_support::family",
    "test_support::range",
    "test_support::tbwt_evidence",
    "test_support::window",
    "test_support::source_scan",
    "grid::StepGrid",
    "router::find_on_grid",
    "tile::Tile",
    "world::NavWorld::load_default_pack_or_skip",
];

fn planner_path(path: &str) -> bool {
    path == PLANNER_ROOT
        || path
            .strip_prefix(PLANNER_ROOT)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

fn prefix_matches(path: &str, relative_prefix: &str) -> bool {
    let prefix = format!("{PLANNER_ROOT}::{relative_prefix}");
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

fn path_is_allowed(path: &SourcePath) -> bool {
    if !planner_path(&path.path) || path.path == PLANNER_ROOT {
        return true;
    }
    if path.kind == PathKind::Glob {
        return false;
    }
    PRODUCTION_ALLOW
        .iter()
        .any(|prefix| prefix_matches(&path.path, prefix))
        || (path.scope == Scope::Test
            && TEST_ALLOW
                .iter()
                .any(|prefix| prefix_matches(&path.path, prefix)))
}

fn surface_violations(report: &ScanReport) -> Vec<SourcePath> {
    report
        .paths
        .iter()
        .filter(|path| planner_path(&path.path) && !path_is_allowed(path))
        .cloned()
        .collect()
}

fn scan_fixture(source: &str) -> ScanReport {
    scan_text(source, "src/lib.rs", "fixture", Scope::Production, &[PLANNER_ROOT], &[])
}

#[test]
fn executor_names_only_the_planner_surface_lists() {
    let report = scan_package(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        "nav-executor",
        &[PLANNER_ROOT],
        &[],
    );
    assert!(
        report.findings.is_empty(),
        "executor source has unsupported or unattached syntax: {:#?}",
        report.findings
    );
    let violations = surface_violations(&report);
    assert!(
        violations.is_empty(),
        "executor names planner paths outside lists A, B, and C: {violations:#?}"
    );
}

#[test]
fn listed_aliases_and_grouped_paths_resolve_to_allowed_items() {
    let report = scan_fixture(
        "use nav::executor_adapter::plan_leg_count as lc;\
         use nav::{contract::RoutePlanLease, router::{Leg, Route}};\
         fn f(plan: &RoutePlanLease) { let _ = lc(plan); let _ = Route::Walk; let _ = Leg::Walk; }",
    );
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
    assert!(
        surface_violations(&report).is_empty(),
        "{:#?}",
        surface_violations(&report)
    );
}

#[test]
fn planner_root_alias_does_not_hide_an_unlisted_path() {
    let report = scan_fixture("use nav as p; fn f() { p::router::find(..); }");
    let violations = surface_violations(&report);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert_eq!(violations[0].path, "nav::router::find");
}

#[test]
fn the_scanner_import_is_allowed_only_in_test_scope() {
    let test_report = scan_fixture(
        "#[cfg(test)] mod tests { use nav::test_support::source_scan; }",
    );
    assert!(
        surface_violations(&test_report).is_empty(),
        "{:#?}",
        surface_violations(&test_report)
    );

    let production_report = scan_fixture("use nav::test_support::source_scan;");
    let violations = surface_violations(&production_report);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert_eq!(violations[0].path, "nav::test_support::source_scan");
}

#[test]
fn every_planner_glob_fails_even_when_its_module_is_listed() {
    let report = scan_fixture("use nav::router::*;");
    let violations = surface_violations(&report);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert_eq!(violations[0].kind, PathKind::Glob);
}

#[test]
fn planner_scan_rejects_the_equivalent_action_path_fixtures() {
    let cases = [
        "use nav::producer::Producer;",
        "use nav::producer;",
        "use nav::producer as p; fn f() { p::build(); }",
        "pub use nav::contract::RouteView;",
        "use nav::router::find;",
        "use nav::executor_adapter::anything_not_listed;",
        "macro_rules! m { () => { nav::producer::build() }; }",
        "fn f() { let _ = nav::producer::build; }",
        "use nav::*;",
        "#[cfg(any(test, unix))] mod m { use nav::producer::Producer; }",
        "use nav::producer::*;",
        "macro_rules! m { () => { use nav::{producer::Producer}; }; }",
        "use nav as p; fn f() { p::producer::build(); }",
        "extern crate nav as n; fn f() { n::producer::build(); }",
        "fn f<T: nav::producer::Bound>(value: nav::producer::Value) { let nav::producer::Variant = value; }",
        "#[nav::producer::capability] fn f() {}",
        "macro_rules! m { () => { $crate::producer::build() }; }",
        "macro_rules! m { () => { nav::$root::build() }; }",
    ];
    for (index, source) in cases.iter().enumerate() {
        let report = scan_fixture(source);
        assert!(
            !surface_violations(&report).is_empty() || !report.findings.is_empty(),
            "equivalent fixture {} unexpectedly passed: {source}",
            index + 1
        );
    }
}

#[test]
fn test_scope_accepts_only_list_b_and_cfg_attr_fails_closed() {
    let test_list = scan_fixture(
        "#[cfg(all(test, target_os = \"macos\"))] mod tests {\
         use nav::test_support::family; use nav::tile::Tile;\
         }",
    );
    assert!(
        surface_violations(&test_list).is_empty(),
        "{:#?}",
        surface_violations(&test_list)
    );

    let production_tile = scan_fixture("use nav::tile::Tile;");
    assert_eq!(surface_violations(&production_tile).len(), 1);

    let cfg_attr = scan_fixture("#[cfg_attr(feature = \"x\", cfg(test))] mod m {}");
    assert!(cfg_attr
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::UnsupportedCfgAttr));
}

#[test]
fn attached_test_path_scope_and_unattached_files_are_enforced() {
    let mut attached_test = SourceMap::new();
    attached_test.insert(
        PathBuf::from("src/lib.rs"),
        "#[cfg(test)] #[path = \"x_tests.rs\"] mod tests;".to_owned(),
    );
    attached_test.insert(
        PathBuf::from("src/x_tests.rs"),
        "use nav::test_support::family;".to_owned(),
    );
    let report = scan_sources(&attached_test, "nav-executor", &[PLANNER_ROOT], &[]);
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
    assert!(surface_violations(&report).is_empty());

    let mut production = SourceMap::new();
    production.insert(
        PathBuf::from("src/lib.rs"),
        "#[path = \"x_tests.rs\"] mod tests;".to_owned(),
    );
    production.insert(
        PathBuf::from("src/x_tests.rs"),
        "use nav::producer::Producer;".to_owned(),
    );
    let report = scan_sources(&production, "nav-executor", &[PLANNER_ROOT], &[]);
    assert_eq!(surface_violations(&report).len(), 1);

    production.insert(
        PathBuf::from("src/stray.rs"),
        "use nav::producer::Producer;".to_owned(),
    );
    let report = scan_sources(&production, "nav-executor", &[PLANNER_ROOT], &[]);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::UnattachedFile));
}

#[test]
fn section_469_examples_are_enforced() {
    let aliased_allowed = scan_fixture(
        "use nav::executor_adapter::plan_leg_count as lc; fn f(x: &RoutePlanLease) { let _ = lc(x); }",
    );
    assert!(surface_violations(&aliased_allowed).is_empty());

    let grouped_allowed = scan_fixture(
        "use nav::{contract::RoutePlanLease, router::{Leg, Route}};",
    );
    assert!(surface_violations(&grouped_allowed).is_empty());

    let root_alias_disallowed = scan_fixture("use nav as p; fn f() { p::router::find(); }");
    let violations = surface_violations(&root_alias_disallowed);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert_eq!(violations[0].path, "nav::router::find");
}

#[test]
fn macro_dynamic_paths_and_macro_root_aliases_are_rejected() {
    let fragment = scan_fixture("macro_rules! m { ($p:path) => { $p::walk() }; }");
    assert!(fragment
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::UnsupportedMacroForm));

    let root_alias = scan_fixture("macro_rules! m { () => { use nav as p; }; }");
    assert!(root_alias
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::UnsupportedMacroForm));

    let extern_crate = scan_fixture("macro_rules! m { () => { extern crate nav; }; }");
    assert!(extern_crate
        .findings
        .iter()
        .any(|finding| finding.kind == FindingKind::UnsupportedMacroForm));
}

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
    assert_eq!(
        actual, expected,
        "fixture {fixture}: {:#?}",
        report.findings
    );
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
        (PathBuf::from("lib.rs"), "pub mod interact;\n".to_owned()),
        (
            PathBuf::from("interact.rs"),
            "pub mod driver;\npub use driver::*;\npub struct Internal;\n".to_owned(),
        ),
        (
            PathBuf::from("interact/driver.rs"),
            "pub fn walk() {}\n".to_owned(),
        ),
    ]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["api::interact".to_owned()])
    );
}

#[test]
fn unresolved_local_glob_in_an_allowed_namespace_fails_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let files: SourceMap = BTreeMap::from([
        (PathBuf::from("lib.rs"), "pub mod exposed;\n".to_owned()),
        (
            PathBuf::from("exposed.rs"),
            "pub use crate::missing::*;\n".to_owned(),
        ),
    ]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(
        closure.findings[0].kind,
        FindingKind::UnsupportedReexportGlob
    );
    assert_eq!(closure.findings[0].path.as_deref(), Some("api::missing::*"));
}

#[test]
fn glob_chain_exposes_and_tracks_reexports_from_denied_items() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
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
    let closure = compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
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

fn assert_reexported_seed_is_denied(
    closure: &crate::test_support::source_scan::ClosureReport,
    seed_alias: &str,
    source: &str,
    expected: (crate::test_support::source_scan::FindingKind, Option<&str>),
) {
    use crate::test_support::source_scan::{scan_text, Scope};

    assert!(
        closure.denied.contains(seed_alias),
        "missing re-exported seed {seed_alias}: {closure:#?}"
    );
    let denied: Vec<_> = closure.denied.iter().map(String::as_str).collect();
    let report = scan_text(
        source,
        "src/lib.rs",
        "nav",
        Scope::Production,
        &["api", "client"],
        &denied,
    );
    assert_scanner_findings(seed_alias, &report, &[expected]);
}

#[test]
fn ancestor_reexport_client_io_glob_denies_public_seed_module() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod io; pub use crate::io::*;".to_owned(),
        ),
        (PathBuf::from("io.rs"), "pub mod client_prot;".to_owned()),
        (
            PathBuf::from("io/client_prot.rs"),
            "pub enum ClientProt {}".to_owned(),
        ),
    ]);
    let closure = compute_deny_closure_sources(&[("client", &files)], &["client::io::client_prot"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_reexported_seed_is_denied(
        &closure,
        "client::client_prot",
        "use client::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("client::client_prot::ClientProt"),
        ),
    );
}

#[test]
fn ancestor_reexport_api_root_glob_denies_public_seed_module() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod interact; pub mod prelude { pub use crate::*; }".to_owned(),
        ),
        (PathBuf::from("interact.rs"), "pub fn walk() {}".to_owned()),
    ]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
    // The root glob also exposes prelude through itself. A finite closure
    // cannot enumerate that recursive alias, so closure construction must fail.
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
    assert_eq!(closure.findings[0].path.as_deref(), Some("api::prelude"));
    assert_reexported_seed_is_denied(
        &closure,
        "api::prelude::interact",
        "fn f() { api::prelude::interact::walk(); }",
        (
            FindingKind::QualifiedPath,
            Some("api::prelude::interact::walk"),
        ),
    );
}

#[test]
fn ancestor_reexport_client_io_alias_denies_public_seed_module() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod io; pub use crate::io as wire;".to_owned(),
        ),
        (PathBuf::from("io.rs"), "pub mod client_prot;".to_owned()),
        (
            PathBuf::from("io/client_prot.rs"),
            "pub enum ClientProt {}".to_owned(),
        ),
    ]);
    let closure = compute_deny_closure_sources(&[("client", &files)], &["client::io::client_prot"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_reexported_seed_is_denied(
        &closure,
        "client::wire::client_prot",
        "use client::wire::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("client::wire::client_prot::ClientProt"),
        ),
    );
}

#[test]
fn ancestor_reexport_tracks_nested_public_modules_and_alias_chains() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod io;
             pub mod api { pub use crate::io as wire; }
             pub use crate::api as facade;"
                .to_owned(),
        ),
        (
            PathBuf::from("io.rs"),
            "pub mod nested { pub mod client_prot {} }".to_owned(),
        ),
    ]);
    let closure =
        compute_deny_closure_sources(&[("client", &files)], &["client::io::nested::client_prot"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_reexported_seed_is_denied(
        &closure,
        "client::facade::wire::nested::client_prot",
        "use client::facade::wire::nested::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("client::facade::wire::nested::client_prot::ClientProt"),
        ),
    );
}

#[test]
fn ancestor_reexport_does_not_expose_private_module_descendants() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod io; pub use crate::io::*; pub use crate::io as wire;".to_owned(),
        ),
        (
            PathBuf::from("io.rs"),
            "mod hidden { pub mod client_prot {} }".to_owned(),
        ),
    ]);
    let closure =
        compute_deny_closure_sources(&[("client", &files)], &["client::io::hidden::client_prot"]);
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["client::io::hidden::client_prot".to_owned()])
    );
}

#[test]
fn unresolved_ancestor_module_alias_fails_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub use crate::missing as wire;".to_owned(),
    )]);
    let closure =
        compute_deny_closure_sources(&[("client", &files)], &["client::missing::client_prot"]);
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
    assert_eq!(closure.findings[0].path.as_deref(), Some("client::missing"));
}

#[test]
fn reexport_item_extern_crate_alias_tracks_denied_seed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let client = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod io { pub mod client_prot {} }".to_owned(),
    )]);
    let api = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub extern crate client as c;".to_owned(),
    )]);
    let closure = compute_deny_closure_sources(
        &[("api", &api), ("client", &client)],
        &["client::io::client_prot"],
    );
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_reexported_seed_is_denied(
        &closure,
        "api::c::io::client_prot",
        "use api::c::io::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("api::c::io::client_prot::ClientProt"),
        ),
    );
}

#[test]
fn reexport_item_extern_crate_self_alias_fails_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let client = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod io { pub mod client_prot {} } pub extern crate self as me;".to_owned(),
    )]);
    let closure =
        compute_deny_closure_sources(&[("client", &client)], &["client::io::client_prot"]);
    assert_reexported_seed_is_denied(
        &closure,
        "client::me::io::client_prot",
        "use client::me::io::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("client::me::io::client_prot::ClientProt"),
        ),
    );
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
    assert_eq!(closure.findings[0].path.as_deref(), Some("client"));
    assert!(closure.findings[0].detail.contains("recursive"));
}

#[test]
fn reexport_item_extern_crate_without_rename_tracks_denied_seed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let client = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod io { pub mod client_prot {} }".to_owned(),
    )]);
    let api = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod prelude { pub extern crate client; }".to_owned(),
    )]);
    let closure = compute_deny_closure_sources(
        &[("api", &api), ("client", &client)],
        &["client::io::client_prot"],
    );
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_reexported_seed_is_denied(
        &closure,
        "api::prelude::client::io::client_prot",
        "use api::prelude::client::io::client_prot::ClientProt;",
        (
            FindingKind::UsePath,
            Some("api::prelude::client::io::client_prot::ClientProt"),
        ),
    );
}

#[test]
fn reexport_item_unsupported_public_trait_alias_fails_closed() {
    use crate::test_support::source_scan::{
        compute_deny_closure_sources, FindingKind, Scope, SourceMap,
    };
    use std::path::PathBuf;

    let files = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub trait Wire = client::io::client_prot::Out;".to_owned(),
    )]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    let finding = &closure.findings[0];
    assert_eq!(finding.kind, FindingKind::UnsupportedReexport);
    assert_eq!(finding.path.as_deref(), Some("api::Wire"));
    assert_eq!(finding.file, PathBuf::from("lib.rs"));
    assert_eq!(finding.scope, Scope::Production);
    assert!(finding.detail.contains("trait alias"));
}

#[test]
fn reexport_item_opaque_public_macro_fails_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub macro wire() { pub extern crate client as c; }".to_owned(),
    )]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
    assert_eq!(closure.findings.len(), 1, "{:#?}", closure.findings);
    assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
    assert_eq!(closure.findings[0].path.as_deref(), Some("api"));
    assert!(closure.findings[0].detail.contains("unclassified"));
}

#[test]
fn reexport_item_respects_public_visibility_and_production_scope() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::path::PathBuf;

    let client = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod io { pub mod client_prot {} }".to_owned(),
    )]);
    let api = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "extern crate client as private;
         pub(crate) extern crate client as restricted;
         #[cfg(test)] pub extern crate client as test_only;
         trait Private = client::io::client_prot::Out;
         pub(crate) trait Restricted = client::io::client_prot::Out;
         #[cfg(test)] pub trait TestOnly = client::io::client_prot::Out;
         #[cfg(test)] mod tests { pub macro wire() {} }"
            .to_owned(),
    )]);
    let closure = compute_deny_closure_sources(
        &[("api", &api), ("client", &client)],
        &["client::io::client_prot"],
    );
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["client::io::client_prot".to_owned()])
    );
}

#[test]
fn reexport_r4_type_paths_deny_local_and_external_aliases() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let client = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub mod io { pub mod client_prot { pub struct ClientProt; } }
         pub type Prot = crate::io::client_prot::ClientProt;"
            .to_owned(),
    )]);
    let api = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "pub type Prot = client::io::client_prot::ClientProt;
         pub mod nested {
             use client::io::client_prot as wire;
             pub type Generic<T> = wire::ClientProt<T>;
         }"
        .to_owned(),
    )]);
    let closure = compute_deny_closure_sources(
        &[("api", &api), ("client", &client)],
        &["client::io::client_prot"],
    );
    assert!(closure.findings.is_empty(), "{:#?}", closure.findings);
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from([
            "client::io::client_prot".to_owned(),
            "client::Prot".to_owned(),
            "api::Prot".to_owned(),
            "api::nested::Generic".to_owned(),
        ])
    );
    for alias in ["client::Prot", "api::Prot", "api::nested::Generic"] {
        assert_reexported_seed_is_denied(
            &closure,
            alias,
            &format!("use {alias};"),
            (FindingKind::UsePath, Some(alias)),
        );
    }
}

#[test]
fn reexport_r4_non_path_aliases_of_denied_items_fail_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    for ty in [
        "&'static client::io::client_prot::ClientProt",
        "(client::io::client_prot::ClientProt, u8)",
        "[client::io::client_prot::ClientProt; 1]",
        "fn() -> client::io::client_prot::ClientProt",
        "<client::io::client_prot::ClientProt as Trait>::Associated",
        "&'static crate::Prot",
        "choose_type!()",
    ] {
        let files = SourceMap::from([(
            PathBuf::from("lib.rs"),
            format!(
                "pub use client::io::client_prot::ClientProt as Prot;
                 pub type Wrapped = {ty};"
            ),
        )]);
        let closure =
            compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
        assert_eq!(closure.findings.len(), 1, "{ty}: {closure:#?}");
        assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
        assert_eq!(closure.findings[0].path.as_deref(), Some("api::Wrapped"));
    }
}

#[test]
fn reexport_r4_alias_visibility_scope_and_harmless_types_pass() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "type Private = client::io::client_prot::ClientProt;
         pub(crate) type Restricted = &'static client::io::client_prot::ClientProt;
         #[cfg(test)] pub type TestOnly = client::io::client_prot::ClientProt;
         #[cfg(test)] pub type TestWrapper = &'static client::io::client_prot::ClientProt;
         pub type Board = [u8; 9];
         pub type Text = &'static str;
         pub type Values = Vec<u8>;"
            .to_owned(),
    )]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
    assert!(closure.findings.is_empty(), "{closure:#?}");
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["client::io::client_prot".to_owned()])
    );
}

#[test]
fn reexport_r4_include_reexports_share_the_including_module() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (
            PathBuf::from("lib.rs"),
            "pub mod io { pub mod client_prot {} }
             pub mod facade;"
                .to_owned(),
        ),
        (
            PathBuf::from("facade.rs"),
            "use crate::io as source;
             include!(\"extra/exports.inc\");
             pub use wire as renamed;"
                .to_owned(),
        ),
        (
            PathBuf::from("extra/exports.inc"),
            "pub use source as wire;
             include!(\"nested/more.inc\",);"
                .to_owned(),
        ),
        (
            PathBuf::from("extra/nested/more.inc"),
            "pub type Prot = crate::io::client_prot::ClientProt;".to_owned(),
        ),
    ]);
    let closure = compute_deny_closure_sources(&[("client", &files)], &["client::io::client_prot"]);
    assert!(closure.findings.is_empty(), "{closure:#?}");
    for alias in [
        "client::facade::wire::client_prot",
        "client::facade::renamed::client_prot",
        "client::facade::Prot",
    ] {
        assert_reexported_seed_is_denied(
            &closure,
            alias,
            &format!("use {alias};"),
            (FindingKind::UsePath, Some(alias)),
        );
    }
    assert_eq!(closure.denied.len(), 4);
}

#[test]
fn reexport_r4_include_parent_paths_match_physical_source_paths() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    for target in ["../shared.inc", "../../shared.inc"] {
        let files = SourceMap::from([
            (
                PathBuf::from("lib.rs"),
                format!("pub mod facade {{ include!(\"{target}\"); }}"),
            ),
            (
                PathBuf::from(target),
                "pub use client::io::client_prot as wire;".to_owned(),
            ),
        ]);
        let closure =
            compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
        assert!(closure.findings.is_empty(), "{target}: {closure:#?}");
        assert_reexported_seed_is_denied(
            &closure,
            "api::facade::wire",
            "use api::facade::wire::ClientProt;",
            (FindingKind::UsePath, Some("api::facade::wire::ClientProt")),
        );
    }
}

#[test]
fn reexport_r4_invalid_includes_fail_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::path::PathBuf;

    for (invocation, extra) in [
        ("include!(\"missing.rs\");", None),
        ("include!(\"extra.rs\");", Some("pub use")),
        ("include!(\"extra.rs\");", Some("include!(\"lib.rs\");")),
        ("include!(concat!(\"extra\", \".rs\"));", None),
    ] {
        let mut files = SourceMap::from([(PathBuf::from("lib.rs"), invocation.to_owned())]);
        if let Some(source) = extra {
            files.insert(PathBuf::from("extra.rs"), source.to_owned());
        }
        let closure = compute_deny_closure_sources(&[("api", &files)], &["api::interact"]);
        assert!(!closure.findings.is_empty(), "{invocation}: {closure:#?}");
    }
}

#[test]
fn reexport_r4_unknown_item_macros_fail_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    for invocation in [
        "macro_rules! reexport { () => { pub use client::io as wire; }; } reexport!();",
        "unknown!();",
        "other::thread_local!();",
        "legal_send!();",
    ] {
        let files = SourceMap::from([(PathBuf::from("lib.rs"), invocation.to_owned())]);
        let closure = compute_deny_closure_sources(&[("api", &files)], &["api::prot"]);
        assert_eq!(closure.findings.len(), 1, "{invocation}: {closure:#?}");
        assert_eq!(closure.findings[0].kind, FindingKind::UnsupportedReexport);
        assert_eq!(closure.findings[0].path.as_deref(), Some("api"));
    }
}

#[test]
fn reexport_r4_allowlisted_macros_and_test_scope_pass() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([(
        PathBuf::from("lib.rs"),
        "macro_rules! unused { () => { pub use client::io as wire; }; }
         thread_local! { static LOCAL: u8 = 0; }
         std::thread_local! { static QUALIFIED: u8 = 0; }
         pub mod prot {
             legal_send!();
             pub mod nested { another!(); }
         }
         #[cfg(test)] unknown!();
         #[cfg(test)] include!(\"missing.rs\");
         #[cfg(test)] mod tests { another!(); }"
            .to_owned(),
    )]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["api::prot"]);
    assert!(closure.findings.is_empty(), "{closure:#?}");
    assert_eq!(
        closure.denied,
        std::collections::BTreeSet::from(["api::prot".to_owned()])
    );
}

#[test]
fn reexport_r5_bare_type_alias_chain_is_denied() {
    assert_r5_alias_is_denied(
        "pub type A = client::io::client_prot::ClientProt; pub type B = A;",
        "api::B",
    );
}

#[test]
fn reexport_r5_bare_use_of_type_alias_is_denied() {
    assert_r5_alias_is_denied(
        "pub type A = client::io::client_prot::ClientProt; pub use A as C;",
        "api::C",
    );
}

#[test]
fn reexport_r5_box_argument_is_denied() {
    assert_r5_alias_is_denied(
        "use client::io::client_prot::ClientProt as D; pub type W = Box<D>;",
        "api::W",
    );
}

#[test]
fn reexport_r5_option_argument_is_denied() {
    assert_r5_alias_is_denied(
        "use client::io::client_prot::ClientProt as D; pub type W = Option<D>;",
        "api::W",
    );
}

#[test]
fn reexport_r5_parameter_default_is_denied() {
    assert_r5_alias_is_denied(
        "use client::io::client_prot::ClientProt as D; pub type W<T = D> = Vec<T>;",
        "api::W",
    );
}

#[test]
fn reexport_r5_tuple_component_is_denied() {
    assert_r5_alias_is_denied(
        "use client::io::client_prot::ClientProt as D; pub type W = (D, u8);",
        "api::W",
    );
}

fn assert_r5_alias_is_denied(source: &str, alias: &str) {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([(PathBuf::from("lib.rs"), source.to_owned())]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
    assert_reexported_seed_is_denied(
        &closure,
        alias,
        &format!("use {alias};"),
        (FindingKind::UsePath, Some(alias)),
    );
}

#[test]
fn reexport_r5_local_thread_local_definition_fails_closed() {
    assert_r5_macro_fails_closed(
        "macro_rules! thread_local { () => { pub use client::io as wire; }; }
         thread_local!();",
    );
}

#[test]
fn reexport_r5_thread_local_rename_fails_closed() {
    assert_r5_macro_fails_closed(
        "macro_rules! reexport { () => { pub use client::io as wire; }; }
         use reexport as thread_local;
         thread_local!();",
    );
}

#[test]
fn reexport_r5_local_include_definition_fails_closed() {
    assert_r5_macro_fails_closed(
        "macro_rules! include { ($file:literal) => { pub use client::io as wire; }; }
         include!(\"empty.rs\");",
    );
}

fn assert_r5_macro_fails_closed(source: &str) {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    let files = SourceMap::from([
        (PathBuf::from("lib.rs"), source.to_owned()),
        (PathBuf::from("empty.rs"), String::new()),
    ]);
    let closure = compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
    assert!(
        closure
            .findings
            .iter()
            .any(|finding| finding.kind == FindingKind::UnsupportedReexport),
        "{closure:#?}"
    );
}

#[test]
fn reexport_r5_unknown_bare_name_fails_closed() {
    use crate::test_support::source_scan::{compute_deny_closure_sources, FindingKind, SourceMap};
    use std::path::PathBuf;

    for source in ["pub use Unknown as Export;", "pub type Export = Unknown;"] {
        let files = SourceMap::from([(PathBuf::from("lib.rs"), source.to_owned())]);
        let closure =
            compute_deny_closure_sources(&[("api", &files)], &["client::io::client_prot"]);
        assert!(
            closure
                .findings
                .iter()
                .any(|finding| finding.kind == FindingKind::UnsupportedReexport),
            "{source}: {closure:#?}"
        );
    }
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
            (
                crate::test_support::source_scan::FindingKind::UsePath,
                Some("api::interact"),
            ),
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
        ("8", "use client::dash3d::CollisionFlag;", vec![]),
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
            vec![(FindingKind::QualifiedPath, Some("api::interact::walk"))],
        ),
        (
            "19",
            "extern crate client as c; fn f() -> c::client::Client { loop {} }",
            vec![(FindingKind::QualifiedPath, Some("client::client::Client"))],
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
    let report = scan_sources(
        &unattached,
        "nav",
        &["api", "client"],
        EXPECTED_ACTION_CLOSURE,
    );
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
    let report = scan_sources(
        &attached_test,
        "nav",
        &["api", "client"],
        EXPECTED_ACTION_CLOSURE,
    );
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
            (FindingKind::AttributePath, Some("api::interact::check")),
            (FindingKind::QualifiedPath, Some("api::interact::Bound")),
            (FindingKind::QualifiedPath, Some("api::interact::Value")),
            (FindingKind::QualifiedPath, Some("api::interact::Variant")),
        ],
    );
}

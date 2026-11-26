//! The panel's script picker through real frames: two files that show one
//! name are told apart by their path, the selected card shows its manifest
//! details, the filter reads name, description, category and tags, a refused
//! Load keeps its whole diagnostic on screen, and the strict parameter form
//! shows the shared dynamic-value rules.

use super::tests::drawn_text;
use crate::test_support::TestDir;
use frontend_core::scripts::{CardSource, ScriptChoice};

/// The migration text of a file without a manifest (design section 9.1), as
/// the operator must be able to read it whole.
const MIGRATION_TEXT: [&str; 5] = [
    "manifest-required at manifest: native JavaScript and TypeScript scripts require a manifest.",
    "Remove separate apiVersion, SETTINGS, and hostCapabilities exports.",
    "export const manifest = { apiVersion: 2, name: 'My script', version: '1', author: 'Your name', description: 'What it does', category: 'Examples', tags: [], settings: {}, capabilities: [] };",
    "Move your old SETTINGS object into manifest.settings. Keep export function tick(api) or export async function tick(api). Remove a native default export. Do not reference manifest at runtime.",
    "Add this named export:",
];

/// `text` without any whitespace, so a wrapped line compares equal to the
/// line it wrapped.
fn squeezed(text: &str) -> String {
    text.split_whitespace().collect()
}

fn manifest_file(dir: &TestDir, folder: &str, source: String) -> std::path::PathBuf {
    let folder = dir.join(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("miner.ts");
    std::fs::write(&path, source).unwrap();
    path
}

fn session_in(dir: &TestDir) -> crate::session::Session {
    let mut s = crate::session::Session::new();
    s.scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
    s
}

#[test]
fn equal_names_show_their_paths_and_the_selected_card_shows_its_details_and_the_filter_works() {
    let _guard = crate::test_support::imgui_context_guard();
    let _iso = frontend_core::test_support::IsolatedStores::enter("panel-picker-identity");
    let dir = TestDir::new("picker-identity");
    let mut ctx = crate::test_support::imgui_context();
    let mut s = session_in(&dir);
    let a = manifest_file(
        &dir,
        "one",
        frontend_core::test_support::scripts::native_fixture_with(
            "Miner",
            "{}",
            "export function tick(api) {}",
        ),
    );
    let b = manifest_file(
        &dir,
        "two",
        frontend_core::test_support::scripts::native_fixture_with(
            "Miner",
            "{}",
            "export function tick(api) { }",
        ),
    );
    s.load_js(&a);
    s.load_js(&b);
    assert_eq!(s.error, None, "{:?}", s.error);
    let id_a = s.scripts.cards().next().unwrap().identity_id();
    let id_b = s.scripts.cards().nth(1).unwrap().identity_id();
    assert_ne!(id_a, id_b);

    // The second file is selected: its details show, the first one shows
    // where it came from.
    assert_eq!(
        s.script_sel.as_ref().and_then(ScriptChoice::loaded_card),
        Some((CardSource::File, id_b.as_str()))
    );
    let shown = squeezed(&drawn_text(&mut ctx, &mut s, &|ui, s| {
        super::browse_window_body(ui, s)
    }));
    let a_text = a.to_string_lossy();
    let b_text = b.to_string_lossy();
    assert!(
        shown.contains(&squeezed(&format!("File: {a_text}"))),
        "the unselected card of a shared name shows its provenance: {shown}"
    );
    for row in [
        format!("Path: {b_text}"),
        "Version: test".to_owned(),
        "Author: test".to_owned(),
        "API: v2 (native)".to_owned(),
        "Declares: no host capabilities".to_owned(),
    ] {
        assert!(
            shown.contains(&squeezed(&row)),
            "the selected card shows {row}: {shown}"
        );
    }
    assert!(
        !shown.contains(&squeezed(&format!("Path: {a_text}"))),
        "only the selected card shows the detail rows"
    );

    // Selecting the first card by its identity swaps the roles; the same
    // name never selects both.
    s.select_script_selection(ScriptChoice::loaded(CardSource::File, &id_a));
    let shown = squeezed(&drawn_text(&mut ctx, &mut s, &|ui, s| {
        super::browse_window_body(ui, s)
    }));
    assert!(
        shown.contains(&squeezed(&format!("Path: {a_text}"))),
        "{shown}"
    );
    assert!(
        shown.contains(&squeezed(&format!("File: {b_text}"))),
        "{shown}"
    );

    // The filter reads the card's fields; a miss says so.
    s.browse_text_filter = "no-such-script-anywhere".into();
    let shown = drawn_text(&mut ctx, &mut s, &|ui, s| super::browse_window_body(ui, s));
    assert!(shown.contains("no scripts match the filter"), "{shown}");
    s.browse_text_filter = "Miner Tests".into();
    let shown = squeezed(&drawn_text(&mut ctx, &mut s, &|ui, s| {
        super::browse_window_body(ui, s)
    }));
    assert!(
        shown.contains(&squeezed(&format!("Path: {a_text}"))),
        "{shown}"
    );
}

#[test]
fn a_refused_unversioned_file_keeps_its_whole_diagnostic_on_screen() {
    let _guard = crate::test_support::imgui_context_guard();
    let _iso = frontend_core::test_support::IsolatedStores::enter("panel-picker-refusal");
    let dir = TestDir::new("picker-refusal");
    let mut ctx = crate::test_support::imgui_context();
    let mut s = session_in(&dir);
    let bare = dir.join("bare.js");
    std::fs::write(&bare, "export function tick(api) {}").unwrap();

    // The real route: stage the path, apply LoadScriptFile.
    s.load_js(&bare);
    let banner = s.error.clone().expect("the Load is refused");
    assert!(banner.starts_with("bare: "), "{banner}");
    assert!(s.scripts.cards().next().is_none(), "no card is registered");
    let refusal = s
        .scripts
        .latest_load_refusal()
        .expect("the refusal is kept");
    assert!(refusal
        .diagnostic()
        .contains("manifest-required at manifest"));
    assert_eq!(refusal.position(), (Some(1), Some(1)));

    // The banner holds one line; the Load dialog and the Browse list hold
    // the whole text, wrapped, never clipped.
    let shown = drawn_text(&mut ctx, &mut s, &|ui, s| super::banner(ui, s, None));
    assert!(shown.contains("bare.js:1:1"), "{shown}");
    assert!(shown.contains("(see log)"), "{shown}");
    assert!(
        !shown.contains("export const manifest"),
        "one line: {shown}"
    );

    let bare_text = bare.to_string_lossy().into_owned();
    for (surface, draw) in [
        (
            "load dialog",
            (|ui: &dear_imgui_rs::Ui, s: &mut crate::session::Session| {
                super::file_dialog_body(ui, s, super::DialogMode::File)
            }) as fn(&dear_imgui_rs::Ui, &mut crate::session::Session),
        ),
        (
            "browse list",
            (|ui: &dear_imgui_rs::Ui, s: &mut crate::session::Session| {
                super::browse_window_body(ui, s)
            }) as fn(&dear_imgui_rs::Ui, &mut crate::session::Session),
        ),
    ] {
        let shown = squeezed(&drawn_text(&mut ctx, &mut s, &draw));
        assert!(shown.contains("Thelast"), "{surface}: {shown}");
        assert!(
            shown.contains(&squeezed(&format!("{bare_text}:1:1"))),
            "{surface} names the path with line and column: {shown}"
        );
        for line in MIGRATION_TEXT {
            assert!(
                shown.contains(&squeezed(line)),
                "{surface} shows `{line}`: {shown}"
            );
        }
    }

    // Loading the file again once it carries a manifest clears the detail.
    std::fs::write(
        &bare,
        frontend_core::test_support::scripts::native_fixture("export function tick(api) {}"),
    )
    .unwrap();
    s.load_js(&bare);
    assert_eq!(s.error, None, "{:?}", s.error);
    assert!(s.scripts.latest_load_refusal().is_none());
}

#[test]
fn the_strict_parameter_form_shows_the_range_and_reports_blank_banks_by_key() {
    let _guard = crate::test_support::imgui_context_guard();
    let _iso = frontend_core::test_support::IsolatedStores::enter("panel-picker-params");
    let dir = TestDir::new("picker-params");
    let mut ctx = crate::test_support::imgui_context();
    let mut s = session_in(&dir);
    let path = manifest_file(
        &dir,
        "one",
        frontend_core::test_support::scripts::native_fixture_with(
            "Miner",
            "{ count: { type: 'number', default: 12.5, min: 2, max: 64, label: 'Ore count' }, \
             bank: { type: 'string', default: '', optionsFrom: 'named-banks', label: 'Bank' }, \
             banks: { type: 'string[]', default: [], optionsFrom: 'named-banks', label: 'Banks' } }",
            "export function tick(api) {}",
        ),
    );
    s.load_js(&path);
    assert_eq!(s.error, None, "{:?}", s.error);
    s.script_prefs_open = true;
    let shown = drawn_text(&mut ctx, &mut s, &|ui, s| {
        super::script_parameter_editors(ui, s)
    });
    assert!(shown.contains("Ore count"), "{shown}");
    assert!(shown.contains("Valid range: 2–64"), "{shown}");
    assert!(
        !shown.to_lowercase().contains("step"),
        "no step control: {shown}"
    );
    assert!(
        shown.contains("invalid saved value for setting `bank`"),
        "a blank named-banks value reports its key: {shown}"
    );
    assert!(
        !shown.contains("invalid saved value for setting `banks`")
            && !shown.contains("invalid saved value for setting `count`"),
        "an empty multiselection and 12.5 inside 2..=64 are valid: {shown}"
    );
}

#[test]
fn the_panel_renders_the_all_kinds_manifest_through_the_shared_form() {
    let _guard = crate::test_support::imgui_context_guard();
    let _iso = frontend_core::test_support::IsolatedStores::enter("panel-picker-all-kinds");
    let dir = TestDir::new("picker-all-kinds");
    let mut ctx = crate::test_support::imgui_context();
    let mut s = session_in(&dir);
    let path = manifest_file(
        &dir,
        "one",
        frontend_core::test_support::scripts::all_kinds_native_fixture(),
    );
    s.load_js(&path);
    assert_eq!(s.error, None, "{:?}", s.error);
    s.script_prefs_open = true;
    let shown = drawn_text(&mut ctx, &mut s, &|ui, s| {
        super::script_parameter_editors(ui, s)
    });
    for needle in [
        "General",
        "Mining",
        "Supplies",
        "Ore count",
        "Valid range: 2–64",
        "Copper ore",
        "Copper vein",
        "Tin vein",
        "Rune vein",
        "Spot",
        "Loadout (default: first loadout) · no loadouts available",
    ] {
        assert!(shown.contains(needle), "`{needle}` missing:\n{shown}");
    }
    assert!(
        !shown.contains("Rune note"),
        "showIf hides it while the ore is not rune:\n{shown}"
    );
    assert!(!shown.contains("None"), "{shown}");
    assert!(!shown.to_lowercase().contains("step"), "{shown}");
}

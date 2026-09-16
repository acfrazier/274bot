//! Unit/integration coverage for the author-facing native v2 example.
//! These tests use synthetic isolate observations; they are not LIVE tests.

use std::path::PathBuf;

use script::load::{ApiFamily, JsLibrary, LoadIsolate, LoadShape};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("274bot-bone-burier-v2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn authoritative_typescript_and_built_javascript_load_as_v2() {
    let dir = scratch();
    let mut library = JsLibrary::with_cache(dir.join("cards.json"), dir.join("cache"));
    let ts_path = example("bone_burier_v2.ts");
    let js_path = example("bone_burier_v2.js");
    let ts = library.load(&ts_path).unwrap();
    let js = library.load(&js_path).unwrap();

    assert_eq!(ts.shape, LoadShape::NativeTick);
    assert_eq!(ts.api_family, ApiFamily::V2);
    assert_eq!(js.shape, LoadShape::NativeTick);
    assert_eq!(js.api_family, ApiFamily::V2);
    assert!(ts.unloadable.is_none(), "unexpected TS import failure");
    assert!(js.unloadable.is_none(), "unexpected JS import failure");
    assert_eq!(ts.settings_schema.len(), 1);
    assert_eq!(ts.settings_schema[0].id, "boneName");
    assert!(js.js.contains("apiVersion"));
}

#[test]
fn built_example_runs_with_isolated_settings_and_bounded_failure() {
    let source = std::fs::read_to_string(example("bone_burier_v2.js")).unwrap();
    let left = LoadIsolate::spawn(source.clone(), LoadShape::NativeTick, vec![]).unwrap();
    let right = LoadIsolate::spawn(source, LoadShape::NativeTick, vec![]).unwrap();

    let mut left_settings = serde_json::Map::new();
    left_settings.insert("boneName".into(), serde_json::json!("Dragon bones"));
    left.post_settings_bag(&left_settings);
    let mut right_settings = serde_json::Map::new();
    right_settings.insert("boneName".into(), serde_json::json!("Bones"));
    right.post_settings_bag(&right_settings);

    // No posted snapshot is an unavailable/not-in-game observation, not a
    // bank-exhaustion observation. Both isolates stop cleanly with a useful
    // reason, and their public settings handles remain isolated.
    left.on_game_tick(1);
    right.on_game_tick(1);
    assert_eq!(
        left.probe("globalThis.__rs_api.settings.str('boneName')")
            .unwrap(),
        "Dragon bones"
    );
    assert_eq!(
        right
            .probe("globalThis.__rs_api.settings.str('boneName')")
            .unwrap(),
        "Bones"
    );
    assert!(std::fs::read_to_string(example("bone_burier_v2.ts"))
        .unwrap()
        .contains("stalled while"));
    assert!(std::fs::read_to_string(example("bone_burier_v2.ts"))
        .unwrap()
        .contains("confirmed loaded current-generation bank exhaustion"));
    left.join();
    right.join();
}

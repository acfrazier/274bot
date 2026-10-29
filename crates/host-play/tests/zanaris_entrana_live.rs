#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::WorldTile;
use quester_live::{Mode, PathCell};
use script::quester::registry;
use serde_json::json;
use std::path::PathBuf;

struct SourceGuard(Option<registry::FolderSource>);

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            registry::set_source(source);
        }
    }
}

#[test]
#[ignore = "requires LIVE=1 and the selected local 289 engine; uses a content-backed Path"]
fn live_zanaris_entrana_dungeon_entry() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let path_dir = PathBuf::from(std::env::var_os("QUESTER_PATH_DIR").expect("QUESTER_PATH_DIR"));
    let prior_source = registry::source();
    let _source_guard = SourceGuard(Some(prior_source));
    let source = scenario::quester::reload_quester_path_source("zanaris", Some(&path_dir)).unwrap();
    assert!(
        matches!(source, "folder" | "draft"),
        "expected a folder-backed Path, got {source}"
    );

    const EXTRA_ITEMS: [(&str, i32); 1] = [("knife", 1)];
    let cell = quester_live::path_cell(PathCell {
        quest: "zanaris",
        display: "Lost City",
        label: "dungeon-entry".to_owned(),
        stage: Some("zanaris:1"),
        loadout: None,
        extra_items: &EXTRA_ITEMS,
        seed_vars: &[],
        stand: WorldTile {
            x: 3145,
            z: 3202,
            level: 0,
        },
        mode: Mode::Stage {
            expect: vec!["zanaris:2".to_owned()],
        },
        before_relog: Vec::new(),
    })
    .unwrap();
    let start_settings = cell.start_settings.clone();
    let receipt = quester_live::run_family(
        cell,
        Box::new(move |handle, account, _| {
            handle.start_compiled(
                account,
                script::CompiledId("Quester"),
                start_settings.clone(),
            )
        }),
        Box::new(|snapshot, _, _| {
            if let Some((x, z, level)) = snapshot.tile() {
                if (2816..=2879).contains(&x) && (9728..=9791).contains(&z) && level == 0 {
                    return Ok(Some(json!({
                        "event": "entrana-dungeon-entry",
                        "tile": {"x": x, "z": z, "level": level},
                        "stop_condition": "first observed tile inside the dungeon region"
                    })));
                }
            }
            Ok(None)
        }),
    )
    .expect("Lost City Path must reach the Entrana dungeon on the selected nav pack");
    println!("LOST-CITY-DUNGEON-ENTRY {receipt}");
}

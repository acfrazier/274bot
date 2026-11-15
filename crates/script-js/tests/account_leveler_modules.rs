//! The rs2b0t modules the AccountLeveler catalog card imports beside the Dragon Slayer
//! journal: its quest records, its desktop storage and its stop error.

use std::sync::Arc;

use script_js::{LoadIsolate, LoadShape};

mod common;
#[path = "common/local_api_asset.rs"]
mod local_api_asset;
#[path = "common/local_game_data.rs"]
mod local_game_data;

const SCRIPT: &str = r#"
import { desktopStorage } from '../../runtime/desktopStorage.js';
import { ScriptAborted } from '../../runtime/ScriptContext.js';
import { ScriptAborted as ExecutionAborted } from '../../api/execution/Execution.js';
import { QUESTS } from '../../api/ai/quests/data/quests.js';
export default class Probe extends LoopingBot {
    loop() {}
}
globalThis.__probe = {
    stored: desktopStorage.getItem('rs2b0t:account-leveler:nobody'),
    refused: ['removeItem', 'keys', 'compareAndSet', 'subscribe'].map((member) => {
        try { desktopStorage[member](); return 'returned'; } catch (error) { return String(error.message); }
    }),
    sameAborted: ScriptAborted === ExecutionAborted,
    count: QUESTS.length,
    fortress: QUESTS.find((quest) => quest.id === 'blackknight'),
    squire: QUESTS.find((quest) => quest.id === 'squire'),
};
"#;

fn isolate(content: Option<Arc<api::game_data::SelectedGameData>>) -> LoadIsolate {
    let iso = LoadIsolate::spawn_with_content(
        SCRIPT.into(),
        LoadShape::CompatClass,
        vec![],
        content,
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    )
    .expect("spawn");
    iso.post_snapshot(script_js::wire::encode_snapshot(&common::ingame_snapshot()));
    iso.on_game_tick(1);
    iso.probe("true").expect("probe barrier");
    iso
}

#[test]
fn the_modules_load_and_serve_what_the_catalog_reads() {
    if !local_game_data::local_game_data_available(289) {
        return;
    }
    let selected =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected game data");
    let iso = isolate(Some(selected));
    let probe = iso.probe("globalThis.__probe").expect("probe");
    assert_eq!(probe["stored"], serde_json::Value::Null);
    assert_eq!(
        probe["refused"],
        serde_json::json!([
            "not impl: desktopStorage.removeItem",
            "not impl: desktopStorage.keys",
            "not impl: desktopStorage.compareAndSet",
            "not impl: desktopStorage.subscribe",
        ])
    );
    assert_eq!(probe["sameAborted"], true);
    assert_eq!(probe["count"], 68);
    assert_eq!(
        probe["fortress"],
        serde_json::json!({
            "id": "blackknight",
            "name": "Black Knight's Fortress",
            "questPoints": 3,
            "requirements": { "minQuestPoints": 12 },
        })
    );
    assert_eq!(
        probe["squire"]["requirements"],
        serde_json::json!({ "skills": [{ "skill": "cooking", "level": 10 }] })
    );
    iso.join();
}

#[test]
fn without_selected_content_the_quest_records_are_empty() {
    let iso = isolate(None);
    assert_eq!(
        iso.probe("globalThis.__probe.count").expect("probe"),
        serde_json::json!(0)
    );
    iso.join();
}

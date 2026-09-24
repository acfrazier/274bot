//! The ChickenKiller style announcement consumes Game's resolution object.

mod common;

use script::load::{LoadIsolate, LoadShape};

#[test]
fn matched_combat_style_result_can_be_described_by_the_catalog_helper() {
    let source = r#"
import { Game } from '../../api/game/Game.js';
import { describeCombatStyle } from '../../api/combat/CombatStyle.js';
export default class T extends LoopingBot {
    loop() {
        const resolution = Game.combatStyleResolution('strength');
        globalThis.result = { resolution, description: describeCombatStyle(resolution) };
    }
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![]).unwrap();
    // This checks the two JS API shapes together; the resolution reads the
    // decoded scene, so the combat-tab rows arrive as one FlatBuffer post.
    let styles = [script::isolate_fb::CombatStyleInput {
        mode: 1,
        label: "Aggressive",
        component_id: 77,
    }];
    let mut snapshot = common::ingame_snapshot();
    snapshot.combat_styles = &styles;
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let result = isolate.probe("result").unwrap();
    assert_eq!(result["resolution"]["requested"], "strength");
    assert_eq!(result["resolution"]["effective"], "strength");
    assert_eq!(result["resolution"]["mode"], 1);
    assert_eq!(result["description"], "strength");
    assert!(isolate
        .drain_logs()
        .iter()
        .all(|line| !line.contains("not impl")));
    isolate.join();
}

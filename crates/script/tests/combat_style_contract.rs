//! The ChickenKiller style announcement consumes Game's resolution object.

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
    // This checks the two JS API shapes together; packet publication is covered
    // by the existing posted-combat-style integration tests.
    isolate.probe("globalThis.__rs2b0t_host.snapshot = {combat_styles:[{mode:1,label:'Aggressive',component_id:77}]}").unwrap();
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

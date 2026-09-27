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
    assert_eq!(result["description"], "strength (training Strength)");
    assert!(isolate
        .drain_logs()
        .iter()
        .all(|line| !line.contains("not impl")));
    isolate.join();
}

/// The frozen `resolveCombatStyle` rules over the posted rows: the requested
/// style wins, a duplicate mode keeps its first label, and an unoffered style
/// falls back to the last defensive option, which `describeCombatStyle`
/// names as the effective style (`CombatStyle.ts:148-169`).
#[test]
fn resolution_falls_back_and_drops_duplicate_modes() {
    let source = r#"
import { Game } from '../../api/game/Game.js';
import { describeCombatStyle } from '../../api/combat/CombatStyle.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.result = {
            controlled: Game.combatStyleResolution('controlled'),
            defence: Game.combatStyleResolution('defence'),
            aggressive: Game.combatStyleResolution('aggressive'),
            unknown: Game.combatStyleResolution('no-such-style'),
            controlledLabel: describeCombatStyle(Game.combatStyleResolution('controlled')),
            attackLabel: describeCombatStyle(Game.combatStyleResolution('attack')),
        };
    }
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![]).unwrap();
    let styles = [
        script::isolate_fb::CombatStyleInput {
            mode: 0,
            label: "Accurate",
            component_id: 70,
        },
        script::isolate_fb::CombatStyleInput {
            mode: 1,
            label: "Aggressive",
            component_id: 71,
        },
        // Duplicate mode: the first label for a mode wins.
        script::isolate_fb::CombatStyleInput {
            mode: 1,
            label: "Defensive",
            component_id: 72,
        },
        script::isolate_fb::CombatStyleInput {
            mode: 2,
            label: "Defensive",
            component_id: 73,
        },
        // Unusable label: not a mode at all.
        script::isolate_fb::CombatStyleInput {
            mode: 3,
            label: "(Slam)",
            component_id: 74,
        },
    ];
    let mut snapshot = common::ingame_snapshot();
    snapshot.combat_styles = &styles;
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let result = isolate.probe("result").unwrap();
    // controlled is not offered: the last defensive row (mode 2) answers.
    assert_eq!(result["controlled"]["effective"], "defence");
    assert_eq!(result["controlled"]["mode"], 2);
    assert_eq!(result["defence"]["mode"], 2);
    assert_eq!(result["defence"]["effective"], "defence");
    // The frozen alias table resolves `aggressive` to the strength row.
    assert_eq!(result["aggressive"]["effective"], "strength");
    assert_eq!(result["aggressive"]["mode"], 1);
    // `(Slam)` parses to nothing, so it is not a defensive fallback either.
    assert_eq!(result["unknown"]["mode"], 2);
    assert_eq!(
        result["controlledLabel"],
        "defence (training Defence; controlled unavailable)"
    );
    assert_eq!(result["attackLabel"], "attack (training Attack)");
    isolate.join();
}

/// Frozen `parseCombatStyle` / `tryParseCombatStyle` / `parseRangeStyle`
/// (`CombatStyle.ts:39-49`, `:171-183`): aliases resolve, and an unknown
/// setting falls back to `strength` / `null` / mode 1 instead of throwing.
#[test]
fn style_parsers_answer_frozen_defaults_for_unknown_settings() {
    let source = r#"
import { parseCombatStyle, tryParseCombatStyle, parseRangeStyle, resolveSplitCombatSettings } from '../../api/combat/CombatStyle.js';
globalThis.result = {
    alias: parseCombatStyle(' Defensive '),
    unknown: parseCombatStyle('no-such-style'),
    tryAlias: tryParseCombatStyle('shared'),
    tryUnknown: tryParseCombatStyle('mage'),
    ranges: ['Accurate', 'rapid', 'long range', 'long-range', 'longrange', 'fast'].map(parseRangeStyle),
    split: resolveSplitCombatSettings('melee', 'bogus'),
};
export default class T extends LoopingBot { loop() {} }
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![]).unwrap();
    let result = isolate.probe("result").unwrap();
    assert_eq!(result["alias"], "defence");
    assert_eq!(result["unknown"], "strength");
    assert_eq!(result["tryAlias"], "controlled");
    assert_eq!(result["tryUnknown"], serde_json::Value::Null);
    assert_eq!(result["ranges"], serde_json::json!([0, 1, 2, 2, 2, 1]));
    assert_eq!(result["split"]["meleeStyle"], "strength");
    isolate.join();
}

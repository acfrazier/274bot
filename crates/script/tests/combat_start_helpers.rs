use script::{LoadIsolate, LoadShape};

fn probe_json(src: &str) -> serde_json::Value {
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn ranged_loadout_is_truthful_and_uses_dart_shape_only() {
    let value = probe_json(
        r#"
import { rangeLoadoutOf } from '../../api/combat/ranged.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs2b0t_host.content = { items: [
            { obj: 'bronze_dart', name: 'Bronze dart' },
            { obj: 'maple_shortbow', name: 'Maple shortbow' },
        ] };
        globalThis.__probe = JSON.stringify({
            empty: rangeLoadoutOf('', 'Iron arrow'),
            bow: rangeLoadoutOf('Maple shortbow', 'Iron arrow'),
            dart: rangeLoadoutOf('Bronze dart', 'Iron arrow'),
        });
    }
}
"#,
    );
    assert_eq!(
        value,
        serde_json::json!({
            "empty": {"weapon": "", "projectile": "Iron arrow", "thrown": false},
            "bow": {"weapon": "Maple shortbow", "projectile": "Iron arrow", "thrown": false},
            "dart": {"weapon": "Bronze dart", "projectile": "Bronze dart", "thrown": true},
        })
    );
}

#[test]
fn food_forms_are_generated_aliases_and_count_slots() {
    let value = probe_json(
        r#"
import { foodForms, foodCount, isFoodItem } from '../../api/combat/food.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs2b0t_host.content = { items: [
            { obj: 'cake', name: 'Cake' },
            { obj: 'partial_cake', name: '2/3 cake' },
            { obj: 'cake_slice', name: 'Slice of cake' },
            { obj: 'lobster', name: 'Lobster' },
        ] };
        const trout = Array.from({ length: 8 }, (_, slot) => ({ name: 'Trout', count: 20, slot }));
        globalThis.__probe = JSON.stringify({
            cake: foodForms('Cake'),
            partial: foodForms('2/3 cake'),
            shark: foodForms('Shark'),
            trout: foodCount(trout, 'Trout'),
            lobster: foodCount([{ name: 'Lobster', count: 10 }], 'Trout'),
            empty: foodCount(null, 'Trout'),
            exact: isFoodItem('trout', 'Trout'),
        });
    }
}
"#,
    );
    assert_eq!(
        value["cake"],
        serde_json::json!(["cake", "2/3 cake", "slice of cake"])
    );
    assert_eq!(value["partial"], serde_json::json!(["2/3 cake"]));
    assert_eq!(value["shark"], serde_json::json!(["shark"]));
    assert_eq!(value["trout"], 8);
    assert_eq!(value["lobster"], 0);
    assert_eq!(value["empty"], 0);
    assert_eq!(value["exact"], true);
}

#[test]
fn settings_display_and_saved_preserve_posted_values() {
    let value = probe_json(
        r#"
import { SettingsStore } from '../../runtime/Settings.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__rs2b0t_host.settingsBag = {
            combatStyle: 'defence',
            enabled: true,
            targets: ['A', 'B'],
            tile: { x: 3200, z: 3201, level: 2 },
        };
        globalThis.__probe = JSON.stringify({
            present: SettingsStore.displayString('T', 'combatStyle', { default: 'melee' }),
            missing: SettingsStore.displayString('T', 'missing', { default: 'Lobster' }),
            bool: SettingsStore.displayString('T', 'enabled', { default: false }),
            list: SettingsStore.displayString('T', 'targets', { default: '' }),
            tile: SettingsStore.displayString('T', 'tile', { default: '' }),
            saved: SettingsStore.saved('T', 'combatStyle'),
            absent: SettingsStore.saved('T', 'missing'),
            resolved: SettingsStore.resolve('T', { missing: { default: 'Lobster' } }).missing,
        });
    }
}
"#,
    );
    assert_eq!(value["present"], "defence");
    assert_eq!(value["missing"], "Lobster");
    assert_eq!(value["bool"], "true");
    assert_eq!(value["list"], "A, B");
    assert_eq!(value["tile"], "3200,3201,2");
    assert_eq!(value["saved"], "defence");
    assert!(value["absent"].is_null());
    assert!(value["resolved"].is_null());
}

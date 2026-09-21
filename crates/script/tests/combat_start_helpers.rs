use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};

fn probe_json(src: &str) -> serde_json::Value {
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn range_supply_empty_exports_all_three_sources_and_edges() {
    let value = probe_json(
        r#"
import { rangeSupplyEmpty } from '../../api/combat/ranged.js';
export default class T extends LoopingBot {
    loop() {
        const direct = globalThis.rustyscript.functions.__rs2b0t_range_supply_empty;
        // Root hold probe (exported f): [f(), f(NaN,0,0), f(Infinity,0,0),
        // f(-Infinity,0,0), f(undefined,0,0)] → [false,false,false,true,false].
        const rootProbe = [
            rangeSupplyEmpty(),
            rangeSupplyEmpty(NaN, 0, 0),
            rangeSupplyEmpty(Infinity, 0, 0),
            rangeSupplyEmpty(-Infinity, 0, 0),
            rangeSupplyEmpty(undefined, 0, 0),
        ];
        globalThis.__probe = JSON.stringify({
            allEmpty: rangeSupplyEmpty(0, 0, 0),
            equipped: rangeSupplyEmpty(1, 0, 0),
            carried: rangeSupplyEmpty(0, 1, 0),
            ground: rangeSupplyEmpty(0, 0, 1),
            allPresent: rangeSupplyEmpty(2, 3, 4),
            negatives: rangeSupplyEmpty(-1, -2, -3),
            fraction: rangeSupplyEmpty(0.5, 0, 0),
            large: rangeSupplyEmpty(2147483648, 0, 0),
            bareCall: rootProbe[0],
            nanEquipped: rootProbe[1],
            infEquipped: rootProbe[2],
            negInfEquipped: rootProbe[3],
            undefEquipped: rootProbe[4],
            rootProbe,
            exportedNulls: rangeSupplyEmpty(null, null, null),
            directEmpty: direct(0, 0, 0),
            directGround: direct(0, 0, 1),
            directNulls: direct(null, null, null),
            directMissing: direct(),
        });
    }
}
"#,
    );
    assert_eq!(value["allEmpty"], true);
    assert_eq!(value["equipped"], false);
    assert_eq!(value["carried"], false);
    assert_eq!(value["ground"], false);
    assert_eq!(value["allPresent"], false);
    assert_eq!(value["negatives"], true);
    assert_eq!(value["fraction"], false);
    assert_eq!(value["large"], false);
    assert_eq!(value["bareCall"], false);
    assert_eq!(value["nanEquipped"], false);
    assert_eq!(value["infEquipped"], false);
    assert_eq!(value["negInfEquipped"], true);
    assert_eq!(value["undefEquipped"], false);
    assert_eq!(
        value["rootProbe"],
        serde_json::json!([false, false, false, true, false])
    );
    assert_eq!(value["exportedNulls"], true);
    assert_eq!(value["directEmpty"], true);
    assert_eq!(value["directGround"], false);
    assert_eq!(value["directNulls"], true);
    assert_eq!(value["directMissing"], false);
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
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(
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
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
        data,
    )
    .unwrap();
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
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

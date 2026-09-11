use script::isolate_fb::{CombatStyleInput, ItemRowInput, SnapshotInput, StatInput, TileInput};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3222,
            z: 3222,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        widgets: &[],
    }
}

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

#[test]
fn posted_spell_db_and_staff_runes_construct_later_superheater_fire_staves() {
    let src = r#"
import { SPELL_DB, STAFF_RUNES } from '../../data/spelldb.js';
export default class T extends LoopingBot {
    loop() {
        const fireStaff = 'Staff of fire';
        const fireStaves = [
            fireStaff,
            ...Object.entries(STAFF_RUNES)
                .filter(([staff, runes]) =>
                    staff.toLowerCase() !== fireStaff.toLowerCase()
                    && runes.some(r => String(r).toLowerCase() === 'fire rune')
                )
                .map(([staff]) => staff)
        ];
        globalThis.__probe = {
            keys: Object.keys(SPELL_DB),
            wind: SPELL_DB['Wind Strike'],
            fireStaves,
            air: fireStaves.includes('Staff of air'),
        };
    }
}
"#;
    let iso =
        LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
            .unwrap();
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    let keys = probe["keys"].as_array().unwrap();
    assert_eq!(keys.first().unwrap(), "Wind Strike");
    assert_eq!(keys.last().unwrap(), "Fire Wave");
    assert_eq!(keys.len(), 16);
    assert_eq!(probe["wind"]["ssb"], 0);
    assert_eq!(probe["wind"]["level"], 1);
    let staves = probe["fireStaves"].as_array().unwrap();
    assert_eq!(staves[0], "Staff of fire");
    for name in [
        "Fire battlestaff",
        "Lava battlestaff",
        "Mystic fire staff",
        "Mystic lava staff",
    ] {
        assert!(
            staves.iter().any(|row| row == name),
            "later Superheater FIRE_STAVES missing {name}: {staves:?}"
        );
    }
    assert_eq!(probe["air"], false);
    iso.join();
}

#[test]
fn casts_available_and_withdraw_list_use_rust_staff_substitution() {
    let src = r#"
import { castsAvailable, runeWithdrawList, spellButtonCom } from '../../api/combat/CombatStyleLogic.js';
export default class T extends LoopingBot {
    loop() {
        const held = {
            'Mind rune': 10,
            'Air rune': 10,
            'Blood rune': 4,
            'Fire rune': 20,
        };
        const count = (name) => held[name] || 0;
        globalThis.__probe = {
            wind: castsAvailable('Wind Strike', ['Staff of air'], count),
            unknown: castsAvailable('Not a spell', [], count),
            fireWave: castsAvailable('Fire Wave', ['Mystic fire staff'], count),
            withdraw: runeWithdrawList('Wind Strike', ['Staff of air'], 5),
            com: spellButtonCom('Wind Strike'),
            missing: spellButtonCom('Nope'),
        };
    }
}
"#;
    let iso =
        LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
            .unwrap();
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["wind"], 10, "air is provided by the staff");
    assert_eq!(probe["unknown"], 0);
    assert_eq!(probe["fireWave"], 2, "mystic fire still needs blood+air");
    assert_eq!(
        probe["withdraw"],
        serde_json::json!([{"rune": "Mind rune", "count": 5}])
    );
    assert_eq!(probe["com"], 1830);
    assert_eq!(probe["missing"], -1);
    iso.join();
}

#[test]
fn cast_on_item_missing_spell_control_throws() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
export default class T extends LoopingBot {
    async loop() {
        try {
            await Game.castOnItem('Superheat Item', Inventory.first('Copper ore'));
            globalThis.__probe = 'no-throw';
        } catch (e) {
            globalThis.__probe = String(e.message || e);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    let inv = [ItemRowInput::nc(Some("Copper ore"), 1)];
    snap.inv = &inv;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing spell_buttons must throw, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn cast_on_item_rejects_stale_inventory_without_queue() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__probe = await Game.castOnItem('Superheat Item', { name: 'Copper ore' });
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    let spells = [CombatStyleInput {
        mode: 0,
        label: "Superheat Item",
        component_id: 1922,
    }];
    snap.spell_buttons = &spells;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__probe").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn cast_on_item_queues_then_observes_magic_xp_and_item_change() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Skills } from '../../api/skills/Skills.js';
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() {
        const before = Skills.xp('magic');
        const ore = Inventory.count('Copper ore');
        globalThis.__ok = await Game.castOnItem('Superheat Item', Inventory.first('Copper ore'));
        globalThis.__landed = null;
        globalThis.__landed = await Execution.delayUntil(
            () => Skills.xp('magic') > before && Inventory.count('Copper ore') < ore,
            5000,
        );
        globalThis.__xp = Skills.xp('magic');
        globalThis.__ore = Inventory.count('Copper ore');
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    let inv = [ItemRowInput::nc(Some("Copper ore"), 2)];
    let spells = [CombatStyleInput {
        mode: 0,
        label: "Superheat Item",
        component_id: 1922,
    }];
    let stats = [StatInput {
        index: 6,
        name: "magic",
        xp: 100,
        base: 55,
        effective: 55,
    }];
    snap.inv = &inv;
    snap.spell_buttons = &spells;
    snap.stats = &stats;
    snap.tick = 1;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::UseWidgetOn {
            component_id: 1922,
            kind: "held".into(),
            target_name: Some("Copper ore".into()),
            x: 0,
            z: 0,
            level: 0,
            index: None,
        }]
    );
    assert_eq!(iso.probe("__landed").unwrap(), serde_json::Value::Null);

    let inv2 = [ItemRowInput::nc(Some("Copper ore"), 1)];
    let stats2 = [StatInput {
        index: 6,
        name: "magic",
        xp: 630,
        base: 55,
        effective: 55,
    }];
    snap.inv = &inv2;
    snap.stats = &stats2;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__landed").unwrap(), true);
    assert_eq!(iso.probe("__xp").unwrap(), 630);
    assert_eq!(iso.probe("__ore").unwrap(), 1);
    iso.join();
}

#[test]
fn spell_helpers_are_unknown_without_selected_game_data() {
    let src = r#"
import { castsAvailable, spellButtonCom } from '../../api/combat/CombatStyleLogic.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            casts: castsAvailable('Wind Strike', ['Staff of air'], () => 99),
            com: spellButtonCom('Wind Strike'),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["casts"], 0);
    assert_eq!(probe["com"], -1);
    iso.join();
}

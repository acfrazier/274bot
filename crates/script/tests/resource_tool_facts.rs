//! Posted gather-tool facts, exclusive callbacks, and void Traversal.preload.

use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>(inv: &'a [ItemRowInput<'a>]) -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2582,
            z: 3481,
            level: 0,
        }),
        ingame: true,
        inv,
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
        side_tab: 0,
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
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        widgets: &[],
    }
}

const SRC: &str = r#"
import { AXES, PICKAXES, bestAxe, bestPickaxe, canWieldTool, exactTool, toolRestockPlan } from '../../api/acquisition/Tools.js';
import { Traversal } from '../../api/walking/Traversal.js';

function namesOf(list) {
    return list.map((t) => t.name);
}

const steelIdx = AXES.findIndex((a) => a.name === 'Steel axe');
globalThis.__axes = AXES.map((a) => ({ name: a.name, id: a.id }));
globalThis.__picks = PICKAXES.map((a) => ({ name: a.name, id: a.id }));
globalThis.__steelOrBetter = steelIdx < 0 ? [] : namesOf(AXES).filter((_, i) => i <= steelIdx);
globalThis.__belowSteel = steelIdx < 0 ? namesOf(AXES) : namesOf(AXES).filter((_, i) => i > steelIdx);

const axeCalls = [];
globalThis.__bestAxe = bestAxe(1, (name) => {
    axeCalls.push(name);
    return name === 'Rune axe' || name === 'Steel axe';
});
globalThis.__axeCalls = axeCalls;

const pickCalls = [];
globalThis.__bestPick30 = bestPickaxe(30, (name) => name === 'Steel pickaxe');
globalThis.__bestPickOrder = bestPickaxe(99, (name) => {
    pickCalls.push(name);
    return name === 'Steel pickaxe';
});
globalThis.__pickCalls = pickCalls;
globalThis.__bestPick5 = bestPickaxe(5, (name) => name === 'Steel pickaxe');
globalThis.__bestPick1 = bestPickaxe(1, (name) => name === 'Bronze pickaxe' || name === 'Steel pickaxe');

const held = new Set(['Steel axe']);
const bank = new Set(['Mithril axe']);
globalThis.__bankOnly = bestAxe(1, (name) => bank.has(name));
globalThis.__heldNotBank = bestAxe(1, (name) => held.has(name) && bank.has(name));

globalThis.__wieldSteelPick1 = canWieldTool('Steel pickaxe', 1);
globalThis.__wieldSteelPick5 = canWieldTool('Steel pickaxe', 5);
globalThis.__wieldSteelAxe1 = canWieldTool('Steel axe', 1);
globalThis.__wieldSteelAxe5 = canWieldTool('Steel axe', 5);
globalThis.__wieldBronze0 = canWieldTool('Bronze pickaxe', 0);
globalThis.__wieldUnknown = canWieldTool('Dragon pickaxe', 99);

try {
    toolRestockPlan();
    globalThis.__restock = 'called';
} catch (e) {
    globalThis.__restock = String(e && e.message ? e.message : e);
}
globalThis.__exact = exactTool('Hammer');

let preloadThrew = null;
let preloadValue;
try {
    preloadValue = Traversal.preload();
} catch (e) {
    preloadThrew = String(e && e.message ? e.message : e);
}
globalThis.__preload = {
    threw: preloadThrew,
    value: preloadValue === undefined ? 'undefined' : preloadValue,
    queued: Array.isArray(globalThis.__rs2b0t_host.interact) && globalThis.__rs2b0t_host.interact.length > 0,
};

export default class T extends LoopingBot {
    loop() {
        if (globalThis.__mode === 'inv') {
            const calls = [];
            globalThis.__invBest = bestPickaxe(30, (name) => {
                calls.push(name);
                return false;
            });
            globalThis.__invCalls = calls;
        }
        if (globalThis.__mode === 'absent') {
            const content = globalThis.__rs2b0t_host.content || {};
            delete content.gather_tools;
            globalThis.__rs2b0t_host.content = content;
            globalThis.__absent = {
                pick: bestPickaxe(99, () => true),
                axe: bestAxe(99, () => true),
                wield: canWieldTool('Steel pickaxe', 99),
            };
        }
    }
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(SRC.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

#[test]
fn posted_axes_are_best_first_with_real_ids_including_black() {
    let iso = spawn();
    let axes = iso.probe("__axes").unwrap();
    assert_eq!(
        axes,
        serde_json::json!([
            {"name": "Rune axe", "id": 1359},
            {"name": "Adamant axe", "id": 1357},
            {"name": "Mithril axe", "id": 1355},
            {"name": "Black axe", "id": 1361},
            {"name": "Steel axe", "id": 1353},
            {"name": "Iron axe", "id": 1349},
            {"name": "Bronze axe", "id": 1351},
        ])
    );
    let picks = iso.probe("__picks").unwrap();
    assert_eq!(
        picks,
        serde_json::json!([
            {"name": "Rune pickaxe", "id": 1275},
            {"name": "Adamant pickaxe", "id": 1271},
            {"name": "Mithril pickaxe", "id": 1273},
            {"name": "Steel pickaxe", "id": 1269},
            {"name": "Iron pickaxe", "id": 1267},
            {"name": "Bronze pickaxe", "id": 1265},
        ])
    );
    assert_eq!(
        iso.probe("__steelOrBetter").unwrap(),
        serde_json::json!([
            "Rune axe",
            "Adamant axe",
            "Mithril axe",
            "Black axe",
            "Steel axe",
        ])
    );
    assert_eq!(
        iso.probe("__belowSteel").unwrap(),
        serde_json::json!(["Iron axe", "Bronze axe"])
    );
    iso.join();
}

#[test]
fn best_tool_callback_is_exclusive_and_ordered() {
    let iso = spawn();
    assert_eq!(iso.probe("__bestAxe").unwrap(), "Rune axe");
    assert_eq!(
        iso.probe("__axeCalls").unwrap(),
        serde_json::json!(["Rune axe"])
    );
    assert_eq!(iso.probe("__bestPick30").unwrap(), "Steel pickaxe");
    assert_eq!(
        iso.probe("__pickCalls").unwrap(),
        serde_json::json!([
            "Rune pickaxe",
            "Adamant pickaxe",
            "Mithril pickaxe",
            "Steel pickaxe",
        ])
    );
    assert_eq!(iso.probe("__bestPick5").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.probe("__bestPick1").unwrap(), "Bronze pickaxe");
    assert_eq!(iso.probe("__bankOnly").unwrap(), "Mithril axe");
    assert_eq!(iso.probe("__heldNotBank").unwrap(), serde_json::Value::Null);

    iso.probe("globalThis.__mode = 'inv'").unwrap();
    let steel = [ItemRowInput {
        name: Some("Steel pickaxe"),
        count: 1,
        id: 1269,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    }];
    let snap = base_snapshot(&steel);
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__invBest").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__invCalls").unwrap(),
        serde_json::json!([
            "Mithril pickaxe",
            "Steel pickaxe",
            "Iron pickaxe",
            "Bronze pickaxe",
        ])
    );
    iso.join();
}

#[test]
fn can_wield_is_attack_not_held_and_unknown_is_false() {
    let iso = spawn();
    assert_eq!(iso.probe("__wieldSteelPick1").unwrap(), false);
    assert_eq!(iso.probe("__wieldSteelPick5").unwrap(), true);
    assert_eq!(iso.probe("__wieldSteelAxe1").unwrap(), false);
    assert_eq!(iso.probe("__wieldSteelAxe5").unwrap(), true);
    assert_eq!(iso.probe("__wieldBronze0").unwrap(), true);
    assert_eq!(iso.probe("__wieldUnknown").unwrap(), false);
    assert_eq!(
        iso.probe("__restock").unwrap(),
        "not impl: Tools.toolRestockPlan"
    );
    assert_eq!(
        iso.probe("__exact").unwrap(),
        serde_json::json!({"name": "Hammer"})
    );
    iso.join();
}

#[test]
fn preload_is_void_noop_and_absent_facts_fail_closed() {
    let iso = spawn();
    let preload = iso.probe("__preload").unwrap();
    assert_eq!(preload["threw"], serde_json::Value::Null);
    assert_eq!(preload["value"], "undefined");
    assert_eq!(preload["queued"], false);

    iso.probe("globalThis.__mode = 'absent'").unwrap();
    tick(&iso, 1);
    let absent = iso.probe("__absent").unwrap();
    assert_eq!(absent["pick"], serde_json::Value::Null);
    assert_eq!(absent["axe"], serde_json::Value::Null);
    assert_eq!(absent["wield"], false);
    iso.join();
}

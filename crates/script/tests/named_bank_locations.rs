//! Catalog ranking uses captured account facts, not scene-booth proximity.

use std::sync::Arc;

use api::named_banks::{NamedBank, NamedBankFacts, BANK_CATALOG};
use api::snapshot::WorldTile;
use nav::named_banks::resolve;
use script::isolate_fb::{NativeFactsInput, QuestStatusInput, ReachViewInput, SnapshotInput, StatInput, TileInput};
use script::{LoadIsolate, LoadShape};


fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3200,
            z: 3200,
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
        combat_level: 0,
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
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}


fn spawn(src: &str, facts: NamedBankFacts) -> LoadIsolate {
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        None,
        Arc::new(facts),
        Arc::new(api::run_policy::RunPolicyOverrideCell::new()),
    )
    .unwrap()
}

const PROBE: &str = r#"
import { nearestBank, nearestBanks, nearestUsableBank } from '../../api/bank/BankLocations.js';
globalThis.pick = (x, z, level = 0) => nearestBank({x, z, level})?.name ?? null;
globalThis.ranked = (x, z, level = 0) => nearestBanks({x, z, level}).map(bank => bank.name);
globalThis.custom = (x, z, name) => nearestUsableBank({x, z, level: 0}, bank => bank.name === name)?.name ?? null;
export default class T extends LoopingBot { loop() {} }
"#;

fn catalog() -> NamedBankFacts {
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    resolve(BANK_CATALOG, &data.bank_placements().unwrap().rows, |_| true)
}

fn post(iso: &LoadIsolate, fishing_base: i32, quests: &[QuestStatusInput<'_>]) {
    let stats = [StatInput { index: 10, name: "Fishing", xp: 0, base: fishing_base, effective: 99 }];
    let mut snapshot = base_snapshot();
    snapshot.stats = &stats;
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &snapshot, NativeFactsInput { quest_statuses: Some(quests), ..Default::default() },
    ));
    iso.probe("true").unwrap();
}

#[test]
fn account_gates_use_base_level_and_require_both_quest_and_opt_in() {
    let iso = spawn(PROBE, catalog());
    post(&iso, 67, &[]);
    assert_eq!(iso.probe("ranked(2586,3420).includes('Fishing Guild')").unwrap(), false);
    assert_eq!(iso.probe("ranked(2852,2954).includes('Shilo Village')").unwrap(), false);
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    let quests = [
        QuestStatusInput { name: "Shilo Village", status: "complete", component_id: None },
        QuestStatusInput { name: "Lost City", status: "complete", component_id: None },
    ];
    post(&iso, 68, &quests);
    assert_eq!(iso.probe("pick(2586,3420)").unwrap(), "Fishing Guild");
    assert_eq!(iso.probe("pick(2852,2954)").unwrap(), "Shilo Village");
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    iso.post_settings_bag(serde_json::json!({"useZanarisBank":true}).as_object().unwrap());
    assert_eq!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    post(&iso, 68, &[]);
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    iso.join();
}

#[test]
fn air_ranking_uses_mage_approach_and_caller_predicate_without_builtin_gates() {
    let iso = spawn(PROBE, catalog());
    post(&iso, 1, &[]);
    assert_ne!(iso.probe("pick(3091,3958)").unwrap(), "Mage Arena");
    assert_eq!(iso.probe("custom(3091,3958,'Mage Arena')").unwrap(), "Mage Arena");
    iso.post_settings_bag(serde_json::json!({"useMageBank":true}).as_object().unwrap());
    assert_eq!(iso.probe("pick(3091,3958)").unwrap(), "Mage Arena");
    iso.join();
}

#[test]
fn air_ties_keep_catalog_order_across_planes_without_isolate_leakage() {
    let a = spawn(PROBE, NamedBankFacts::from_banks(vec![
        NamedBank::new("Upstairs", WorldTile { x: 10, z: 11, level: 1 }),
        NamedBank::new("Ground", WorldTile { x: 11, z: 10, level: 0 }),
    ]));
    let b = spawn(PROBE, NamedBankFacts::from_banks(vec![
        NamedBank::new("Other world", WorldTile { x: 10, z: 10, level: 0 }),
    ]));
    assert_eq!(a.probe("pick(10,10)").unwrap(), "Upstairs");
    assert_eq!(a.probe("ranked(10,10)").unwrap(), serde_json::json!(["Upstairs", "Ground"]));
    assert_eq!(b.probe("pick(10,10)").unwrap(), "Other world");
    a.join();
    b.join();
}

/// The selected identity controls opening, not a later nearest-booth snapshot.
/// A selection (including an air fallback) is not a successful bank session.
#[test]
fn reachable_bank_continuations_open_npc_access_and_wait_for_loaded_stock() {
    use script::isolate_fb::{BankSelectionInput, SceneEntityInput};
    use script::shim::InteractReq;
    for (call, after) in [
        ("Banking.open()", 0),
        ("Bank.openNearestWorld()", 0),
        ("Banking.bankNearest({deposit: () => false, afterDeposit() { globalThis.__after++; }})", 1),
    ] {
        let source = format!(r#"
import {{ Bank }} from '../../api/bank/Bank.js';
import {{ Banking }} from '../../api/bank/Banking.js';
globalThis.__after = 0; globalThis.__ok = null;
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await {call};
    }}
}}"#);
        let facts = catalog();
        let index = facts.banks().iter().position(|bank| bank.name == "Shilo Village").unwrap();
        let stand = facts.banks()[index].tile;
        let iso = spawn(&source, facts);
        let quests = [QuestStatusInput { name: "Shilo Village", status: "complete", component_id: None }];
        let mut snapshot = base_snapshot();
        let mut native = NativeFactsInput { quest_statuses: Some(&quests), ..Default::default() };
        let tick = |snapshot: &SnapshotInput<'_>, native: NativeFactsInput<'_>| {
            iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(snapshot, native));
            iso.on_game_tick(snapshot.tick);
            iso.probe("true").unwrap();
        };
        tick(&snapshot, native);
        let requests = iso.drain_interacts();
        let [InteractReq::SelectBank { request_id, .. }] = requests.as_slice() else {
            panic!("{call}: expected an off-scene bank selection, got {requests:?}");
        };
        snapshot.tick = 2;
        native.bank_selection = BankSelectionInput { request_id: request_id + 1, generation: 1, bank_index: index as i32, kind: 2 };
        tick(&snapshot, native);
        assert!(iso.drain_interacts().is_empty(), "a stale identity must not start a walk");
        snapshot.tick = 3;
        native.bank_selection.request_id = *request_id;
        native.bank_selection.kind = if after == 1 { 3 } else { 2 };
        tick(&snapshot, native);
        assert!(matches!(iso.drain_interacts().as_slice(), [InteractReq::WalkNear { x, z, radius: 4, .. }]
            if (*x, *z) == (stand.x, stand.z)), "{call} must approach the selected stand");
        snapshot.tick = 4;
        snapshot.here = Some(TileInput { x: stand.x, z: stand.z, level: stand.level });
        snapshot.reach = ReachViewInput { available: true, base_x: stand.x, base_z: stand.z, level: stand.level,
            width: 1, height: 1, walkable: &[1], reachable: &[1], reachable_adj: &[1],
            exact_rank: &[0], adjacent_rank: &[0], step: &[0], canlight: &[0], stamp: 1 };
        let actions = ["Bank".to_string()];
        let npcs = [SceneEntityInput { index: 42, id: 499, name: Some("Banker"), x: stand.x + 1, z: stand.z,
            level: stand.level, distance: 1, health: 1, max_health: 1, in_combat: false, animating: false,
            actions: &actions, reachable: true, reachable_adj: true, combat_level: 0,
            target_kind: 0, target_index: -1, size: 1, nx: stand.x + 1, nz: stand.z }];
        snapshot.npcs = &npcs;
        tick(&snapshot, native);
        assert_eq!(iso.drain_interacts(), vec![InteractReq::Npc {
            name: "Banker".into(), action: "Bank".into(), index: Some(42),
        }], "{call}: NPC access must not wait for a booth or require a dialogue choice");
        snapshot.tick = 5;
        let choices = [script::isolate_fb::ChatOptionInput { text: "Tell me about this village." }];
        snapshot.chat_open = true;
        snapshot.chat_options = &choices;
        tick(&snapshot, native);
        assert!(iso.drain_interacts().is_empty(), "absent npcAccess.choose must not select an unrelated first option");
        assert_eq!(iso.probe("[__ok,__after]").unwrap(), serde_json::json!([null, 0]));
        snapshot.tick = 6;
        snapshot.chat_open = false;
        snapshot.chat_options = &[];
        snapshot.bank_open = true;
        snapshot.bank_generation = 1;
        tick(&snapshot, native);
        assert_eq!(iso.probe("[__ok,__after]").unwrap(), serde_json::json!([null, 0]));
        snapshot.bank_loaded = true;
        let side = [script::isolate_fb::ItemRowInput {
            name: Some("Coins"), count: 1, id: 995, ops: &[], noted: false,
            cert: -1, component_id: 1, slot: 0,
        }];
        snapshot.bank_side = &side;
        for n in 7..=9 { snapshot.tick = n; tick(&snapshot, native); }
        assert_eq!(iso.probe("[__ok,__after]").unwrap(), serde_json::json!([true, after]), "{call}");
        iso.join();
    }
}

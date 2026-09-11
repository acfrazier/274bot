//! Named BANK_LOCATIONS are import-time facts from bound-world resolve.

use std::sync::Arc;

use api::named_banks::{resolve, NamedBankFacts, CANDIDATES};
use api::snapshot::WorldTile;
use script::isolate_fb::{NearestBoothInput, ReachViewInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

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

fn packed_booths() -> Vec<WorldTile> {
    CANDIDATES
        .iter()
        .flat_map(|c| c.booths.iter().copied())
        .collect()
}

fn spawn(src: &str, facts: NamedBankFacts) -> LoadIsolate {
    LoadIsolate::spawn_with_content(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        None,
        Arc::new(facts),
    )
    .unwrap()
}

const PROBE: &str = r#"
import { BANK_LOCATIONS, nearestBank } from '../../api/bank/BankLocations.js';
import { COOK_LOCATIONS } from '../../api/cooking/CookLocations.js';

function bankTile(name) {
    const loc = BANK_LOCATIONS.find(b => b.name === name);
    if (!loc) throw new Error(`bankTile: unknown bank '${name}'`);
    return loc.tile;
}

const falador = BANK_LOCATIONS.find(b => b.name === 'Falador East');
globalThis.__falador = falador ? [falador.tile.x, falador.tile.z, falador.tile.level] : null;
globalThis.__missing = BANK_LOCATIONS.find(b => b.name === 'No Such Bank');
globalThis.__names = BANK_LOCATIONS.map(b => b.name);
globalThis.__cooks = COOK_LOCATIONS.map(c => c.name);
try {
    const tile = bankTile('Falador East');
    globalThis.__bankTile = [tile.x, tile.z, tile.level];
    globalThis.__bankTileErr = null;
} catch (e) {
    globalThis.__bankTile = null;
    globalThis.__bankTileErr = String(e && e.message ? e.message : e);
}
try {
    bankTile('No Such Bank');
    globalThis.__unknownErr = null;
} catch (e) {
    globalThis.__unknownErr = String(e && e.message ? e.message : e);
}

export default class T extends LoopingBot {
    loop() {
        const n = nearestBank();
        globalThis.__nearest = n ? [n.name, n.tile.x, n.tile.z, n.tile.level] : null;
    }
}
"#;

fn import_probe(iso: &LoadIsolate) -> serde_json::Value {
    iso.probe(
        r#"({
            falador: globalThis.__falador,
            missing: globalThis.__missing,
            names: globalThis.__names,
            cooks: globalThis.__cooks,
            bankTile: globalThis.__bankTile,
            bankTileErr: globalThis.__bankTileErr,
            unknownErr: globalThis.__unknownErr
        })"#,
    )
    .unwrap()
}

#[test]
fn import_time_find_sees_falador_without_a_snapshot() {
    let facts = resolve(&packed_booths(), |_| true);
    let iso = spawn(PROBE, facts);
    let probe = import_probe(&iso);
    assert_eq!(
        probe.get("falador"),
        Some(&serde_json::json!([3013, 3355, 0]))
    );
    assert_eq!(
        probe.get("bankTile"),
        Some(&serde_json::json!([3013, 3355, 0]))
    );
    assert_eq!(probe.get("bankTileErr"), Some(&serde_json::Value::Null));
    assert!(
        probe.get("missing").is_none_or(|v| v.is_null()),
        "unknown name must not be a BANK_LOCATIONS row, got {:?}",
        probe.get("missing")
    );
    assert_eq!(
        probe.get("unknownErr"),
        Some(&serde_json::json!("bankTile: unknown bank 'No Such Bank'"))
    );
    let names = probe
        .get("names")
        .and_then(|v| v.as_array())
        .expect("names");
    assert_eq!(
        names,
        &vec![
            serde_json::json!("Falador East"),
            serde_json::json!("Varrock East"),
            serde_json::json!("Edgeville"),
            serde_json::json!("Draynor"),
            serde_json::json!("Al Kharid"),
        ]
    );
    assert_eq!(probe.get("cooks"), Some(&serde_json::json!(["Catherby"])));
    iso.join();
}

#[test]
fn absent_world_facts_leave_bank_locations_empty() {
    let iso = spawn(PROBE, NamedBankFacts::empty());
    let probe = import_probe(&iso);
    assert_eq!(probe.get("falador"), Some(&serde_json::Value::Null));
    assert_eq!(probe.get("names"), Some(&serde_json::json!([])));
    assert_eq!(
        probe.get("unknownErr"),
        Some(&serde_json::json!("bankTile: unknown bank 'No Such Bank'"))
    );
    assert_eq!(
        probe.get("bankTileErr"),
        Some(&serde_json::json!("bankTile: unknown bank 'Falador East'"))
    );
    iso.join();
}

#[test]
fn independent_isolates_do_not_share_named_bank_facts() {
    let falador_only: Vec<WorldTile> = CANDIDATES
        .iter()
        .find(|c| c.name == "Falador East")
        .unwrap()
        .booths
        .to_vec();
    let varrock_only: Vec<WorldTile> = CANDIDATES
        .iter()
        .find(|c| c.name == "Varrock East")
        .unwrap()
        .booths
        .to_vec();
    let a = spawn(PROBE, resolve(&falador_only, |_| true));
    let b = spawn(PROBE, resolve(&varrock_only, |_| true));
    let pa = import_probe(&a);
    let pb = import_probe(&b);
    assert_eq!(pa.get("names"), Some(&serde_json::json!(["Falador East"])));
    assert_eq!(pb.get("names"), Some(&serde_json::json!(["Varrock East"])));
    a.join();
    let reset = spawn(PROBE, NamedBankFacts::empty());
    let pr = import_probe(&reset);
    assert_eq!(pr.get("names"), Some(&serde_json::json!([])));
    b.join();
    reset.join();
}

#[test]
fn nearest_bank_follows_posted_booth_not_the_alias_list() {
    let facts = resolve(&packed_booths(), |_| true);
    let iso = spawn(PROBE, facts);
    let booth = NearestBoothInput {
        x: 3222,
        z: 3218,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    };
    let mut snap = base_snapshot();
    snap.nearest_booth = Some(booth);
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let nearest = iso.probe("__nearest").unwrap();
    assert_eq!(
        nearest,
        serde_json::json!(["Bank booth", 3222, 3218, 0]),
        "nearestBank must follow live nearest_booth, not named aliases"
    );
    iso.join();
}

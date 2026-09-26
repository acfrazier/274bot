// reader.combatLevel and Player.combatLevel mirror posted native local/peer
// combat levels. Canonical foreign: raw?.localPlayer?.combatLevel ?? 0.

use script::isolate_fb::{ReachViewInput, SceneEntityInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

mod common;
use common::post_snapshot_input;

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3368,
            z: 3274,
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
        self_slot: 3,
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

fn player_row<'a>(
    index: i32,
    name: &'a str,
    combat_level: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: 0,
        name: Some(name),
        x: 3369,
        z: 3274,
        level: 0,
        distance: 1,
        health: 10,
        max_health: 10,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }
}

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
        .unwrap()
}

fn probe(iso: &LoadIsolate, snap: &SnapshotInput<'_>) -> serde_json::Value {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    iso.probe("__probe").unwrap()
}

#[test]
fn reader_combat_level_follows_posted_local_level_and_defaults_out_of_game() {
    let iso = spawn(
        r#"
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = reader.combatLevel(); }
}
"#,
    );
    let mut snap = base_snapshot();
    snap.combat_level = 126;
    assert_eq!(probe(&iso, &snap), 126);

    snap.tick = 2;
    snap.combat_level = 55;
    assert_eq!(probe(&iso, &snap), 55);

    snap.tick = 3;
    snap.ingame = false;
    snap.combat_level = 99;
    assert_eq!(probe(&iso, &snap), 0);
}

#[test]
fn player_combat_level_reads_peer_rows_and_delta_updates_without_stale_identity() {
    let iso = spawn(
        r#"
import { reader } from '../../adapter/ClientAdapter.js';
import { Players } from '../../api/players/Players.js';
export default class T extends LoopingBot {
    loop() {
        const peers = Players.all().map((p) => ({
            index: p.index,
            name: p.name,
            level: p.combatLevel,
        }));
        globalThis.__probe = { local: reader.combatLevel(), peers };
    }
}
"#,
    );
    let challenge = ["Challenge".to_string()];
    let p1 = player_row(1, "alpha", 100, &challenge);
    let p2 = player_row(2, "beta", 59, &challenge);
    let players = [p1, p2];
    let mut snap = base_snapshot();
    snap.combat_level = 55;
    snap.players = &players;
    let first = probe(&iso, &snap);
    assert_eq!(first["local"], 55);
    assert_eq!(first["peers"][0]["level"], 100);
    assert_eq!(first["peers"][1]["level"], 59);

    snap.tick = 2;
    let p2_updated = player_row(2, "beta", 61, &challenge);
    let players = [p1, p2_updated];
    snap.players = &players;
    let second = probe(&iso, &snap);
    assert_eq!(second["peers"][1]["level"], 61);
    assert_eq!(second["peers"][1]["index"], 2);
    assert_eq!(second["peers"][1]["name"], "beta");
}

#[test]
fn omitted_combat_level_delta_keeps_last_posted_local_level() {
    use script::isolate_fb::encode_snapshot_delta;
    let iso = spawn(
        r#"
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() { globalThis.__probe = reader.combatLevel(); }
}
"#,
    );
    let mut snap = base_snapshot();
    snap.combat_level = 126;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__probe").unwrap(), 126);

    snap.tick = 2;
    snap.run_energy = 50;
    let (_, fp) = encode_snapshot_delta(None, &snap, false);
    let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("__probe").unwrap(), 126);
}

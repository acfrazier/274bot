use script::isolate_fb::{ItemRowInput, ReachViewInput, SnapshotInput, StatInput, TileInput};
use script::shim::InteractReq;
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

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()
}

fn data_289() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
        .unwrap()
}

fn if_button(id: i32) -> InteractReq {
    InteractReq::IfButton { component_id: id }
}

fn rune(name: &'static str, count: i32, id: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    }
}

fn magic(xp: i32, level: i32) -> StatInput<'static> {
    StatInput {
        index: 6,
        name: "magic",
        xp,
        base: level,
        effective: level,
    }
}

const TELEPORT: &str = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Skills } from '../../api/skills/Skills.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Game.teleport(globalThis.__name || 'Varrock');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
        globalThis.__xp = Skills.xp('magic');
        globalThis.__tile = Game.tile();
        globalThis.__law = Inventory.count('Law rune');
        globalThis.__air = Inventory.count('Air rune');
        globalThis.__fire = Inventory.count('Fire rune');
    }
}
"#;

#[test]
fn missing_controls_throw_and_do_not_send() {
    let iso = LoadIsolate::spawn(TELEPORT.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = base_snapshot();
    let stats = [magic(100, 50)];
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing teleport facts must throw, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn unknown_and_target_names_return_false_without_a_click() {
    let iso = spawn(TELEPORT);
    let mut snap = base_snapshot();
    let stats = [magic(1000, 50)];
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    let _ = iso.probe("globalThis.__name = 'Wind Strike'");
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());

    let _ = iso.probe("globalThis.__name = 'Nowhere'");
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn low_magic_level_refuses_without_pressing() {
    let iso = spawn(TELEPORT);
    let mut snap = base_snapshot();
    let stats = [magic(100, 10)];
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn queued_button_is_not_arrival_until_xp_and_tile() {
    let iso = spawn(TELEPORT);
    let start_inv = [
        rune("Law rune", 10, 563),
        rune("Air rune", 30, 556),
        rune("Fire rune", 10, 554),
    ];
    let start_stats = [magic(1000, 50)];
    let mut snap = base_snapshot();
    snap.inv = &start_inv;
    snap.stats = &start_stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(1164)],
        "queued Varrock button is not arrival"
    );
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);

    let landed_inv = [
        rune("Law rune", 9, 563),
        rune("Air rune", 27, 556),
        rune("Fire rune", 9, 554),
    ];
    let landed_stats = [magic(1350, 50)];
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3213,
        z: 3424,
        level: 0,
    });
    snap.inv = &landed_inv;
    snap.stats = &landed_stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert_eq!(iso.probe("__xp").unwrap(), 1350);
    assert_eq!(iso.probe("__law").unwrap(), 9);
    assert_eq!(iso.probe("__air").unwrap(), 27);
    assert_eq!(iso.probe("__fire").unwrap(), 9);
    let tile = iso.probe("__tile").unwrap();
    assert_eq!(tile["x"], 3213);
    assert_eq!(tile["z"], 3424);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn falador_uses_selected_cache_button_on_both_revisions() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Game.teleport('Falador');
    }
}
"#;
    for selected in [data(), data_289()] {
        let iso = LoadIsolate::spawn_with_game_data(
            src.to_string(),
            LoadShape::CompatClass,
            vec![],
            selected,
        )
        .unwrap();
        let stats = [magic(2000, 50)];
        let mut snap = base_snapshot();
        snap.stats = &stats;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 1);
        assert_eq!(iso.drain_interacts(), vec![if_button(1170)]);
        let landed = [magic(2480, 50)];
        snap.tick = 2;
        snap.here = Some(TileInput {
            x: 2965,
            z: 3379,
            level: 0,
        });
        snap.stats = &landed;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 2);
        assert_eq!(iso.probe("__ok").unwrap(), true);
        iso.join();
    }
}

#[test]
fn timeout_without_xp_and_tile_returns_false() {
    let iso = spawn(TELEPORT);
    let stats = [magic(1000, 50)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(1164)]);
    for n in 2..=16 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pause_and_hold_freeze_mid_teleport_without_late_click() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Game.teleport('Varrock');
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;
    let iso = spawn(src);
    let stats = [magic(1000, 50)];
    let mut snap = base_snapshot();
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(1164)]);

    iso.pause();
    let landed = [magic(1350, 50)];
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3213,
        z: 3424,
        level: 0,
    });
    snap.stats = &landed;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "Pause must not complete the parked teleport"
    );
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "Guardian hold must freeze the parked teleport wait"
    );
    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    snap.here = Some(TileInput {
        x: 3222,
        z: 3222,
        level: 0,
    });
    snap.stats = &stats;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "session abort must not click again"
    );
    iso.join();
}

//! v1/v2 actor observation: packed size/network SW and local self target.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, encode_snapshot_with_native, CollisionViewInput,
    NativeFactsInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::load::{LoadIsolate, LoadShape};

fn empty_input(tick: u64) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: Some(TileInput {
            x: 3201,
            z: 3201,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
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
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
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

fn npc_row<'a>(
    name: Option<&'a str>,
    size: i32,
    nx: i32,
    nz: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 7,
        id: 9,
        name,
        x: 2833,
        z: 9823,
        level: 0,
        distance: 2,
        health: 5,
        max_health: 5,
        in_combat: false,
        animating: true,
        actions,
        reachable: false,
        reachable_adj: false,
        combat_level: 2,
        target_kind: 0,
        target_index: -1,
        size,
        nx,
        nz,
    }
}

#[test]
fn v1_size_and_route_head_centre_from_packed_row() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {
    loop() {
        const n = Npcs.all()[0];
        const origin = n.networkOrigin();
        const network = n.networkTile();
        globalThis.__probe = {
            size: n.size,
            nx: origin.x,
            nz: origin.z,
            networkX: network.x,
            networkZ: network.z,
            tileX: n.tile().x,
            tileZ: n.tile().z,
            same: origin.x === n.tile().x && origin.z === n.tile().z,
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let npcs = [npc_row(Some("Goblin"), 4, 2832, 9825, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["size"], 4);
    assert_eq!(probe["nx"], 2832);
    assert_eq!(probe["nz"], 9825);
    assert_eq!(probe["networkX"], 2834);
    assert_eq!(probe["networkZ"], 9827);
    assert_eq!(probe["tileX"], 2833);
    assert_eq!(probe["tileZ"], 9823);
    assert_eq!(probe["same"], false);
    iso.join();
}

#[test]
fn v1_size_not_impl_on_old_buffer() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {
    loop() {
        const n = Npcs.all()[0];
        try { n.size; globalThis.__size = 'ok'; }
        catch (e) { globalThis.__size = String(e); }
        try { n.networkOrigin(); globalThis.__origin = 'ok'; }
        catch (e) { globalThis.__origin = String(e); }
        const fallback = n.networkTile();
        globalThis.__network = { x: fallback.x, z: fallback.z };
        globalThis.__me = n.targetsMe();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let npcs = [npc_row(Some("Goblin"), 0, 0, 0, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let size = iso.probe("__size").unwrap();
    let origin = iso.probe("__origin").unwrap();
    assert!(
        size.as_str().unwrap_or("").contains("not impl: Npc.size"),
        "{size}"
    );
    assert!(
        origin
            .as_str()
            .unwrap_or("")
            .contains("not impl: Npc.networkOrigin"),
        "{origin}"
    );
    let network = iso.probe("__network").unwrap();
    assert_eq!(network["x"], 2833);
    assert_eq!(network["z"], 9823);
    assert_eq!(iso.probe("__me").unwrap(), false);
    iso.join();
}

#[test]
fn v1_self_target_npc_player_none() {
    let src = r#"
import { reader } from '../../adapter/ClientAdapter.js';
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            target: reader.selfTarget(),
            raw: reader.selfFaceEntity(),
            attacked: Game.attackedByPlayer(),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = empty_input(1);
    snap.self_target_kind = 1;
    snap.self_target_index = 42;
    snap.attacked_by_player = false;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["target"]["kind"], 1);
    assert_eq!(probe["target"]["index"], 42);
    assert_eq!(probe["raw"], 42);
    assert_eq!(probe["attacked"], false);

    snap.self_target_kind = 2;
    snap.self_target_index = 7;
    snap.attacked_by_player = true;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(2);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["target"]["kind"], 2);
    assert_eq!(probe["target"]["index"], 7);
    assert_eq!(probe["raw"], 32775);
    assert_eq!(probe["attacked"], true);

    snap.self_target_kind = 0;
    snap.self_target_index = -1;
    snap.attacked_by_player = false;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(3);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["target"]["kind"], 0);
    assert_eq!(probe["target"]["index"], -1);
    assert_eq!(probe["raw"], -1);
    assert_eq!(probe["attacked"], false);
    iso.join();
}

#[test]
fn retained_wrapper_keeps_construction_size_after_same_name_morph() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {
    loop() {
        if (!globalThis.__held) {
            globalThis.__held = Npcs.all()[0];
        }
        const live = Npcs.all()[0];
        globalThis.__probe = {
            valid: globalThis.__held.valid(),
            heldSize: globalThis.__held.size,
            liveSize: live.size,
            liveNx: live.networkOrigin().x,
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let first = [npc_row(Some("Goblin"), 1, 10, 10, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &first;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let second = [npc_row(Some("Goblin"), 4, 11, 12, &actions)];
    snap.npcs = &second;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(2);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["valid"], true);
    assert_eq!(probe["heldSize"], 1);
    assert_eq!(probe["liveSize"], 4);
    assert_eq!(probe["liveNx"], 11);
    iso.join();
}

#[test]
fn retained_wrapper_invalid_after_rename() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {
    loop() {
        if (!globalThis.__held) {
            globalThis.__held = Npcs.all()[0];
        }
        globalThis.__probe = globalThis.__held.valid();
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let first = [npc_row(Some("Goblin"), 1, 10, 10, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &first;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let second = [npc_row(Some("Guard"), 1, 10, 10, &actions)];
    snap.npcs = &second;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__probe").unwrap(), false);
    iso.join();
}

#[test]
fn v2_snapshot_exposes_npcs_and_self_target() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const n = api.snapshot.npcs[0];
  globalThis.__probe = {
    size: n.size,
    nx: n.nx,
    nz: n.nz,
    kind: api.snapshot.self_target_kind,
    index: api.snapshot.self_target_index,
    hidden: api.snapshot.locs,
  };
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let npcs = [npc_row(Some("Goblin"), 4, 2832, 9825, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    snap.self_target_kind = 1;
    snap.self_target_index = 42;
    iso.post_snapshot(encode_snapshot(&snap));
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["size"], 4);
    assert_eq!(probe["nx"], 2832);
    assert_eq!(probe["nz"], 9825);
    assert_eq!(probe["kind"], 1);
    assert_eq!(probe["index"], 42);
    assert!(probe["hidden"].is_null());
    iso.join();
}

#[test]
fn v2_line_of_sight_consumes_packed_network_sw() {
    let mut flags = [0i32; 256];
    flags[3 * 16 + 1] = 0x20000;
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const n = api.snapshot.npcs[0];
  const here = api.snapshot.here;
  globalThis.__probe = {
    network: api.lineOfSight({ from: here, to: { x: n.nx, z: n.nz, level: n.level }, size: n.size }),
    rendered: api.lineOfSight({ from: here, to: { x: n.x, z: n.z, level: n.level }, size: n.size }),
  };
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let npcs = [SceneEntityInput {
        index: 1,
        id: 1,
        name: Some("Goblin"),
        x: 3201,
        z: 3205,
        level: 0,
        distance: 4,
        health: 5,
        max_health: 5,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: false,
        reachable_adj: false,
        combat_level: 2,
        target_kind: 0,
        target_index: -1,
        size: 4,
        nx: 3205,
        nz: 3201,
    }];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    let native = NativeFactsInput {
        collision: Some(CollisionViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 16,
            height: 16,
            flags: &flags,
        }),
        ..NativeFactsInput::default()
    };
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &snap, native,
    ));
    iso.on_game_tick(1);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["network"]["ok"], true);
    assert_eq!(probe["rendered"]["ok"], true);
    assert_ne!(probe["network"]["value"], probe["rendered"]["value"]);
    iso.join();
}

#[test]
fn v2_omitted_delta_keeps_npcs_and_self_target() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const n = api.snapshot.npcs[0];
  globalThis.__probe = {
    size: n && n.size,
    nx: n && n.nx,
    kind: api.snapshot.self_target_kind,
    index: api.snapshot.self_target_index,
  };
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    let actions = ["Attack".to_string()];
    let npcs = [npc_row(Some("Goblin"), 4, 2832, 9825, &actions)];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    snap.self_target_kind = 1;
    snap.self_target_index = 42;
    let (kf, fp) = encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(kf);
    iso.on_game_tick(1);
    let (delta, _) = encode_snapshot_delta(Some(&fp), &snap, false);
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let probe = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(probe["size"], 4);
    assert_eq!(probe["nx"], 2832);
    assert_eq!(probe["kind"], 1);
    assert_eq!(probe["index"], 42);
    iso.join();
}

fn spawn_v2(source: &str) -> LoadIsolate {
    LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap()
}

fn open_collision<'a>(flags: &'a [i32]) -> NativeFactsInput<'a> {
    NativeFactsInput {
        collision: Some(CollisionViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 16,
            height: 16,
            flags,
        }),
        ..NativeFactsInput::default()
    }
}

fn scene_npc<'a>(actions: &'a [String]) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 7,
        id: 9,
        name: Some("Goblin"),
        x: 3201,
        z: 3205,
        level: 0,
        distance: 4,
        health: 5,
        max_health: 5,
        in_combat: false,
        animating: false,
        actions,
        reachable: false,
        reachable_adj: false,
        combat_level: 2,
        target_kind: 0,
        target_index: -1,
        size: 4,
        nx: 3205,
        nz: 3201,
    }
}

#[test]
fn qualification_example_imports_are_loadable() {
    let src = include_str!("../examples/actor_observation_v2.ts");
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn example_does_not_stop_without_size_ge_1_npc() {
    let js = script::transpile_ts(include_str!("../examples/actor_observation_v2.ts"))
        .expect("transpile actor_observation_v2.ts");
    let iso = spawn_v2(&js);
    let flags = [0i32; 256];
    let mut snap = empty_input(1);
    iso.post_snapshot(encode_snapshot_with_native(&snap, open_collision(&flags)));
    iso.on_game_tick(1);
    let logs = iso.drain_logs();
    assert!(!iso.stopped(), "no-NPC must not named-stop: logs={logs:?}");
    assert!(
        logs.iter().all(|line| !line.contains("actor-receipt:")),
        "no-NPC must not publish a receipt: {logs:?}"
    );
    iso.join();

    let iso = spawn_v2(&js);
    let actions = ["Attack".to_string()];
    let npcs = [npc_row(Some("Goblin"), 0, 3205, 3201, &actions)];
    snap.npcs = &npcs;
    iso.post_snapshot(encode_snapshot_with_native(&snap, open_collision(&flags)));
    iso.on_game_tick(1);
    let logs = iso.drain_logs();
    assert!(!iso.stopped(), "size<1 must not named-stop: logs={logs:?}");
    iso.join();
}

#[test]
fn example_executes_v1_and_v2_helpers_then_named_stop() {
    let js = script::transpile_ts(include_str!("../examples/actor_observation_v2.ts"))
        .expect("transpile actor_observation_v2.ts");
    let iso = spawn_v2(&js);
    let flags = [0i32; 256];
    let actions = ["Attack".to_string()];
    let npcs = [scene_npc(&actions)];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    snap.self_target_kind = 1;
    snap.self_target_index = 7;
    iso.post_snapshot(encode_snapshot_with_native(&snap, open_collision(&flags)));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let paint = iso.paint().expect("example paint receipt");
    let line = paint
        .lines
        .iter()
        .find(|row| row.starts_with("actor-receipt:"))
        .expect("compact actor-receipt paint line");
    let receipt: serde_json::Value =
        serde_json::from_str(line.strip_prefix("actor-receipt:").unwrap()).unwrap();
    assert_eq!(receipt["npc"]["index"], 7);
    assert_eq!(receipt["npc"]["name"], "Goblin");
    assert_eq!(receipt["npc"]["size"], 4);
    assert_eq!(receipt["npc"]["tile"]["x"], 3201);
    assert_eq!(receipt["npc"]["tile"]["z"], 3205);
    assert_eq!(receipt["npc"]["network"]["x"], 3205);
    assert_eq!(receipt["npc"]["network"]["z"], 3201);
    assert_eq!(receipt["packed"]["size"], 4);
    assert_eq!(receipt["packed"]["nx"], 3205);
    assert_eq!(receipt["packed"]["nz"], 3201);
    assert_eq!(receipt["rendered"]["x"], 3201);
    assert_eq!(receipt["rendered"]["z"], 3205);
    assert_ne!(receipt["packed"]["nx"], receipt["rendered"]["x"]);
    assert_eq!(receipt["self_target"]["kind"], 1);
    assert_eq!(receipt["self_target"]["index"], 7);
    assert_eq!(receipt["los"]["v2"], receipt["los"]["v1"]);
    assert_eq!(
        iso.script_stop_receipt().expect("named helper stop").reason,
        "actor observation qualification complete"
    );
    assert!(iso.stopped());
    iso.join();
}

#[test]
fn example_picks_nearest_size_ge_1_not_array_first() {
    let js = script::transpile_ts(include_str!("../examples/actor_observation_v2.ts"))
        .expect("transpile actor_observation_v2.ts");
    let iso = spawn_v2(&js);
    let flags = [0i32; 256];
    let actions = ["Attack".to_string()];
    let npcs = [
        SceneEntityInput {
            index: 1,
            id: 1,
            name: Some("Zero"),
            x: 3201,
            z: 3201,
            level: 0,
            distance: 0,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 1,
            target_kind: 0,
            target_index: -1,
            size: 0,
            nx: 3201,
            nz: 3201,
        },
        SceneEntityInput {
            index: 3,
            id: 2,
            name: Some("Far"),
            x: 3208,
            z: 3208,
            level: 0,
            distance: 8,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 2,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: 3208,
            nz: 3208,
        },
        SceneEntityInput {
            index: 11,
            id: 3,
            name: Some("Near"),
            x: 3202,
            z: 3202,
            level: 0,
            distance: 1,
            health: 5,
            max_health: 5,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: false,
            reachable_adj: false,
            combat_level: 2,
            target_kind: 0,
            target_index: -1,
            size: 2,
            nx: 3203,
            nz: 3201,
        },
    ];
    let mut snap = empty_input(1);
    snap.npcs = &npcs;
    snap.self_target_kind = 1;
    snap.self_target_index = 7;
    iso.post_snapshot(encode_snapshot_with_native(&snap, open_collision(&flags)));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let paint = iso.paint().expect("example paint receipt");
    let line = paint
        .lines
        .iter()
        .find(|row| row.starts_with("actor-receipt:"))
        .expect("compact actor-receipt paint line");
    let receipt: serde_json::Value =
        serde_json::from_str(line.strip_prefix("actor-receipt:").unwrap()).unwrap();
    assert_eq!(
        receipt["npc"]["index"], 11,
        "array-first size>=1 is index 3; File must pick nearest index 11: {receipt}"
    );
    assert_eq!(receipt["npc"]["name"], "Near");
    assert_eq!(receipt["npc"]["size"], 2);
    assert_eq!(
        iso.script_stop_receipt().expect("named helper stop").reason,
        "actor observation qualification complete"
    );
    iso.join();
}

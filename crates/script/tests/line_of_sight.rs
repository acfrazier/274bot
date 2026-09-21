//! Publication → FB → isolate Observation → public flags / v1 / v2.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta_with_native, CollisionViewInput, NativeFactsInput,
    ReachViewInput, SnapshotInput, TileInput,
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
        widgets: &[],
    }
}

fn collision_native<'a>(flags: &'a [i32]) -> NativeFactsInput<'a> {
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

fn spawn_v2(source: &str) -> LoadIsolate {
    LoadIsolate::spawn(source.into(), LoadShape::NativeTick, vec![]).unwrap()
}

const OPEN_FLAGS: [i32; 256] = [0; 256];

#[test]
fn first_post_without_collision_table_is_unavailable() {
    let iso = spawn_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const c = api.snapshot.collision;
  globalThis.__probe = {
    available: c && c.available,
    length: c && c.flags && c.flags.length,
    at0: c && c.flags && c.flags.at(0),
    v2: api.lineOfSight({ from: { x: 3201, z: 3201, level: 0 }, to: { x: 3202, z: 3201, level: 0 } }),
  };
}
"#,
    );
    iso.post_snapshot(encode_snapshot(&empty_input(1)));
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["available"], false);
    assert_eq!(value["length"], 0);
    assert!(value["at0"].is_null());
    assert_eq!(value["v2"]["ok"], false);
    assert_eq!(value["v2"]["error"], "missing-observation");
    iso.join();
}

#[test]
fn public_flags_and_v2_share_posted_identity() {
    let mut flags = OPEN_FLAGS;
    flags[5 * 16 + 1] = 0x20000;
    let iso = spawn_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  const c = api.snapshot.collision;
  const here = { x: 3201, z: 3201, level: 0 };
  globalThis.__probe = {
    available: c.available,
    length: c.flags.length,
    atSelf: c.flags.at(1 * c.height + 1),
    atVis: c.flags.at(5 * c.height + 1),
    atNeg: c.flags.at(-1),
    atOob: c.flags.at(c.flags.length),
    atNan: c.flags.at(Number.NaN),
    atFrac: c.flags.at(1.5),
    open: api.lineOfSight({ from: here, to: { x: 3203, z: 3201, level: 0 }, size: 1 }),
    blocked: api.lineOfSight({ from: here, to: { x: 3208, z: 3201, level: 0 }, size: 1 }),
    planes: api.lineOfSight({ from: here, to: { x: here.x, z: here.z, level: 1 }, size: 1 }),
  };
}
"#,
    );
    let bytes =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&flags), false).0;
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["available"], true);
    assert_eq!(value["length"], 256);
    assert_eq!(value["atSelf"], 0);
    assert_eq!(value["atVis"], 0x20000);
    assert!(value["atNeg"].is_null());
    assert!(value["atOob"].is_null());
    assert!(value["atNan"].is_null());
    assert!(value["atFrac"].is_null());
    assert_eq!(value["open"]["ok"], true);
    assert_eq!(value["open"]["value"], true);
    assert_eq!(value["blocked"]["ok"], true);
    assert_eq!(value["blocked"]["value"], false);
    assert_eq!(value["planes"]["ok"], true);
    assert_eq!(value["planes"]["value"], false);
    iso.join();
}

#[test]
fn retained_view_is_historical_after_refresh() {
    let mut first = OPEN_FLAGS;
    first[2 * 16 + 2] = 7;
    let mut second = OPEN_FLAGS;
    second[2 * 16 + 2] = 9;
    let iso = spawn_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  if (!globalThis.__old) {
    globalThis.__old = api.snapshot.collision;
    return;
  }
  const old = globalThis.__old;
  const cur = api.snapshot.collision;
  try { old.flags.length = 0; } catch (e) {}
  try { old.available = false; } catch (e) {}
  globalThis.__probe = {
    oldAt: old.flags.at(2 * 16 + 2),
    newAt: cur.flags.at(2 * 16 + 2),
    query: api.lineOfSight({ from: { x: 3202, z: 3202, level: 0 }, to: { x: 3202, z: 3202, level: 0 } }),
  };
}
"#,
    );
    let (a, fp) =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&first), false);
    iso.post_snapshot(a);
    iso.on_game_tick(1);
    let (b, _) =
        encode_snapshot_delta_with_native(Some(&fp), &empty_input(2), collision_native(&second), false);
    iso.post_snapshot(b);
    iso.on_game_tick(2);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["oldAt"], 7, "retained view stays old");
    assert_eq!(value["newAt"], 9);
    assert_eq!(value["query"]["ok"], true);
    assert_eq!(value["query"]["value"], true);
    iso.join();
}

#[test]
fn reset_clears_observation_pause_keeps() {
    let flags = OPEN_FLAGS;
    let iso = spawn_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = {
    available: api.snapshot.collision.available,
    v2: api.lineOfSight({ from: { x: 3201, z: 3201, level: 0 }, to: { x: 3202, z: 3201, level: 0 } }),
  };
}
"#,
    );
    let bytes =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&flags), false).0;
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["available"], true);
    assert_eq!(value["v2"]["ok"], true);
    iso.pause();
    let paused = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(paused["v2"]["ok"], true, "pause keeps last identity");
    iso.resume();
    iso.reset_session_work();
    iso.post_snapshot(encode_snapshot(&empty_input(3)));
    iso.on_game_tick(3);
    let after = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(after["v2"]["error"], "missing-observation");
    iso.join();
}

#[test]
fn example_calls_raw_at_and_true_false_los() {
    let mut flags = OPEN_FLAGS;
    // Adjacent east dest carries entering V_W. A dest VIS_SCENERY rock alone
    // is not a blocked pair.
    flags[2 * 16 + 1] = 0x10000;
    let js = script::transpile_ts(include_str!("../examples/line_of_sight_v2.ts"))
        .expect("transpile line_of_sight_v2.ts");
    let iso = spawn_v2(&js);
    let bytes =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&flags), false).0;
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let paint = iso.paint().expect("example paint receipt");
    let line = paint
        .lines
        .iter()
        .find(|row| row.starts_with("los-receipt:"))
        .expect("compact los-receipt paint line");
    let receipt: serde_json::Value =
        serde_json::from_str(line.strip_prefix("los-receipt:").unwrap()).unwrap();
    assert_eq!(receipt["open"]["v2"], true);
    assert_eq!(receipt["blocked"]["v2"], false);
    assert_eq!(receipt["open"]["v1"], true);
    assert_eq!(receipt["blocked"]["v1"], false);
    assert_eq!(receipt["here"]["flag"], 0);
    assert_eq!(receipt["open"]["from"]["x"], 3201);
    assert_eq!(receipt["blocked"]["to"]["x"], 3202);
    assert_eq!(receipt["blocked"]["mask"], 0x10000);
    assert_eq!(
        iso.script_stop_receipt().expect("named helper stop").reason,
        "line of sight qualification complete"
    );
    assert!(iso.stopped());
    iso.join();
}

#[test]
fn qualification_example_reachability_import_is_loadable() {
    let src = include_str!("../examples/line_of_sight_v2.ts");
    assert_eq!(script::first_unloadable_specifier(src), None);
}

#[test]
fn dest_vis_alone_is_fixture_failure_not_a_blocked_pair() {
    let mut flags = OPEN_FLAGS;
    flags[5 * 16 + 1] = 0x20000;
    let js = script::transpile_ts(include_str!("../examples/line_of_sight_v2.ts"))
        .expect("transpile line_of_sight_v2.ts");
    let iso = spawn_v2(&js);
    let bytes =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&flags), false).0;
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let logs = iso.drain_logs();
    iso.join();
    assert!(
        logs.iter().any(|line| line.contains("fixture failure")),
        "dest VIS_SCENERY alone must fail the pair fixture: {logs:?}"
    );
    assert!(
        logs.iter().all(|line| !line.contains("los-receipt:")),
        "dest VIS_SCENERY alone must not publish a receipt: {logs:?}"
    );
}

#[test]
fn v2_invalid_args_and_size_cap_when_width_zero() {
    let iso = spawn_v2(
        r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = {
    nonObject: api.lineOfSight(null),
    size0: api.lineOfSight({ from: { x: 1, z: 1, level: 0 }, to: { x: 2, z: 2, level: 0 }, size: 0 }),
    size105: api.lineOfSight({ from: { x: 1, z: 1, level: 0 }, to: { x: 2, z: 2, level: 0 }, size: 105 }),
    frac: api.lineOfSight({ from: { x: 1.5, z: 1, level: 0 }, to: { x: 2, z: 2, level: 0 } }),
    planes: api.lineOfSight({ from: { x: 1, z: 1, level: 0 }, to: { x: 1, z: 1, level: 1 } }),
  };
}
"#,
    );
    iso.post_snapshot(encode_snapshot(&empty_input(1)));
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    assert_eq!(value["nonObject"]["error"], "invalid-args");
    assert_eq!(value["size0"]["error"], "invalid-args");
    assert_eq!(value["size105"]["error"], "invalid-args");
    assert_eq!(value["frac"]["error"], "invalid-args");
    assert_eq!(value["planes"]["ok"], true);
    assert_eq!(value["planes"]["value"], false);
    iso.join();
}

#[test]
fn v1_marshal_through_reachability() {
    let mut flags = OPEN_FLAGS;
    flags[5 * 16 + 1] = 0x20000;
    let src = r#"
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
export default class T extends LoopingBot {
    loop() {
        const here = { x: 3201, z: 3201, level: 0 };
        globalThis.__probe = {
            open: Reachability.lineOfSight(here, { x: 3203, z: 3201, level: 0 }),
            blocked: Reachability.lineOfSight(here, { x: 3208, z: 3201, level: 0 }),
            omitted: Reachability.lineOfSight(here, { x: 3203, z: 3201, level: 0 }),
            zero: Reachability.lineOfSight(here, { x: 3203, z: 3201, level: 0 }, 0),
            big: Reachability.lineOfSight(here, { x: 3203, z: 3201, level: 0 }, 105),
            frac: Reachability.lineOfSight(here, { x: 3203, z: 3201, level: 0 }, 1.5),
            planes: Reachability.lineOfSight(here, { x: 3201, z: 3201, level: 1 }),
            badTile: Reachability.lineOfSight(null, here),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let bytes =
        encode_snapshot_delta_with_native(None, &empty_input(1), collision_native(&flags), false).0;
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["open"], true);
    assert_eq!(value["blocked"], false);
    assert_eq!(value["omitted"], true);
    assert_eq!(value["zero"], false);
    assert_eq!(value["big"], false);
    assert_eq!(value["frac"], false);
    assert_eq!(value["planes"], false);
    assert_eq!(value["badTile"], false);
    iso.join();
}

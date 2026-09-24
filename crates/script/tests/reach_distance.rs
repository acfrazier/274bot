//! F11: typed reach/distance helpers over the isolate reach cache.

use script::isolate_fb::{encode_snapshot_delta, ReachViewInput, SnapshotInput, TileInput};

use script::load::{LoadIsolate, LoadShape};

mod common;
use common::post_snapshot_input;

fn base_snapshot() -> SnapshotInput<'static> {
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
        hold: false,
        ours: false,
        npcs: &[],
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
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

fn reach_words(bits: &[usize]) -> Vec<u32> {
    let mut words = vec![0u32; 3];
    for &i in bits {
        words[i / 32] |= 1u32 << (i % 32);
    }
    words
}

fn reach_ranks(entries: &[(usize, u16)]) -> Vec<u16> {
    let mut ranks = vec![u16::MAX; 9 * 8];
    for &(i, rank) in entries {
        ranks[i] = rank;
    }
    ranks
}

#[test]
fn typed_reach_helpers_match_posted_view() {
    let src = r#"
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
export default class T extends LoopingBot {
    loop() {
        const t = (x, z, level) => ({ x, z, level });
        globalThis.__probe = {
            open: Reachability.walkable(t(3200, 3200, 0)),
            blocked: Reachability.walkable(t(3203, 3207, 0)),
            step: Reachability.canStep(t(3200, 3200, 0), t(3201, 3200, 0)),
            reach: Reachability.canReach(t(3204, 3200, 0), { maxSteps: 20 }),
            adj: Reachability.canReach(t(3200, 3201, 0), { adjacentOk: true, maxSteps: 0 }),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let walkable = reach_words(&[0, 32]);
    let reachable = reach_words(&[0, 32]);
    let adj = reach_words(&[0, 1, 32]);
    let exact_rank = reach_ranks(&[(0, 0), (32, 6)]);
    let adjacent_rank = reach_ranks(&[(0, 0), (1, 0), (32, 5)]);
    let mut step = vec![0u8; 9 * 8];
    step[0] = 1 << 1; // east from origin
    let mut snap = base_snapshot();
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 9,
        height: 8,
        walkable: &walkable,
        reachable: &reachable,
        reachable_adj: &adj,
        exact_rank: &exact_rank,
        adjacent_rank: &adjacent_rank,
        step: &step,
        canlight: &[],
        stamp: 0,
    };
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["open"], true);
    assert_eq!(value["blocked"], false);
    assert_eq!(value["step"], true);
    assert_eq!(value["reach"], true);
    assert_eq!(value["adj"], true);
    iso.join();
}

#[test]
fn tile_distance_to_frozen_cross_plane_and_extreme() {
    let src = r#"
import Tile from '../../geometry/Tile.js';
export default class T extends LoopingBot {
    loop() {
        const a = new Tile(3208, 3212, 0);
        globalThis.__probe = {
            same: a.distanceTo(new Tile(3211, 3215, 0)),
            cross: a.distanceTo(new Tile(3211, 3215, 1)),
            extreme: new Tile(-2147483648, 2147483647, 0).distanceTo(
                new Tile(2147483647, -2147483648, 3),
            ),
            nan: Number.isNaN(a.distanceTo(new Tile(NaN, 3215, 0))),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["same"], 3);
    assert_eq!(value["cross"], 1_000_003);
    let planar = i64::from(i32::MAX) - i64::from(i32::MIN);
    assert_eq!(value["extreme"].as_f64(), Some((1_000_000 + planar) as f64));
    assert_eq!(value["nan"], true);
    iso.join();
}

#[test]
fn v2_reach_helpers_match_posted_view() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
    const t = (x, z, level) => ({ x, z, level });
    globalThis.__probe = {
        walkable: api.walkable({ tile: t(3200, 3200, 0) }),
        blocked: api.walkable({ tile: t(3203, 3207, 0) }),
        step: api.canStep({ from: t(3200, 3200, 0), to: t(3201, 3200, 0) }),
        reach: api.canReach({ tile: t(3204, 3200, 0), maxSteps: 20 }),
        missing: api.walkable({ tile: t(3200, 3200, 1) }),
    };
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::NativeTick, vec![]).unwrap();
    let walkable = reach_words(&[0, 32]);
    let reachable = reach_words(&[0, 32]);
    let adj = reach_words(&[0, 1, 32]);
    let exact_rank = reach_ranks(&[(0, 0), (32, 6)]);
    let adjacent_rank = reach_ranks(&[(0, 0), (1, 0), (32, 5)]);
    let mut step = vec![0u8; 9 * 8];
    step[0] = 1 << 1;
    let mut snap = base_snapshot();
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 9,
        height: 8,
        walkable: &walkable,
        reachable: &reachable,
        reachable_adj: &adj,
        exact_rank: &exact_rank,
        adjacent_rank: &adjacent_rank,
        step: &step,
        canlight: &[],
        stamp: 11,
    };
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["walkable"]["ok"], true);
    assert_eq!(value["walkable"]["value"], true);
    assert_eq!(value["blocked"]["ok"], true);
    assert_eq!(value["blocked"]["value"], false);
    assert_eq!(value["step"]["ok"], true);
    assert_eq!(value["step"]["value"], true);
    assert_eq!(value["reach"]["ok"], true);
    assert_eq!(value["reach"]["value"], true);
    assert_eq!(value["missing"]["ok"], true);
    assert_eq!(value["missing"]["value"], false);
    iso.join();
}

#[test]
fn stamped_reach_omit_keeps_view_and_repost_replaces_it() {
    let src = r#"
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
export default class T extends LoopingBot {
    loop() {
        const t = (x, z, level) => ({ x, z, level });
        globalThis.__probe = {
            open: Reachability.walkable(t(3200, 3200, 0)),
            other: Reachability.walkable(t(3204, 3200, 0)),
            step: Reachability.canStep(t(3200, 3200, 0), t(3201, 3200, 0)),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let walkable = reach_words(&[0, 32]);
    let reachable = reach_words(&[0, 32]);
    let adj = reach_words(&[0, 1, 32]);
    let exact_rank = reach_ranks(&[(0, 0), (32, 6)]);
    let adjacent_rank = reach_ranks(&[(0, 0), (1, 0), (32, 5)]);
    let mut step = vec![0u8; 9 * 8];
    step[0] = 1 << 1;
    let mut snap = base_snapshot();
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 9,
        height: 8,
        walkable: &walkable,
        reachable: &reachable,
        reachable_adj: &adj,
        exact_rank: &exact_rank,
        adjacent_rank: &adjacent_rank,
        step: &step,
        canlight: &[],
        stamp: 11,
    };
    let (keyframe, fp) = encode_snapshot_delta(None, &snap, false);
    iso.post_snapshot(keyframe);
    iso.on_game_tick(1);
    let first = iso.probe("__probe").unwrap();
    assert_eq!(first["open"], true);
    assert_eq!(first["other"], true);
    assert_eq!(first["step"], true);

    snap.tick = 2;
    let (delta, fp2) = encode_snapshot_delta(Some(&fp), &snap, false);
    let omitted = script::isolate_fb::decode_snapshot(&delta).expect("delta");
    assert!(!omitted.has_reach(), "unchanged stamp must omit reach");
    iso.post_snapshot(delta);
    iso.on_game_tick(2);
    let kept = iso.probe("__probe").unwrap();
    assert_eq!(kept["open"], true, "omitted delta keeps last walkable bits");
    assert_eq!(kept["other"], true);
    assert_eq!(kept["step"], true, "omitted delta keeps last step bytes");

    let walkable2 = reach_words(&[0]);
    let reachable2 = reach_words(&[0]);
    let adj2 = reach_words(&[0, 1]);
    let exact2 = reach_ranks(&[(0, 0)]);
    let adj_rank2 = reach_ranks(&[(0, 0), (1, 0)]);
    let step2 = vec![0u8; 9 * 8];
    snap.tick = 3;
    snap.reach = ReachViewInput {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 9,
        height: 8,
        walkable: &walkable2,
        reachable: &reachable2,
        reachable_adj: &adj2,
        exact_rank: &exact2,
        adjacent_rank: &adj_rank2,
        step: &step2,
        canlight: &[],
        stamp: 12,
    };
    let (moved, _) = encode_snapshot_delta(Some(&fp2), &snap, false);
    let posted = script::isolate_fb::decode_snapshot(&moved).expect("repost");
    assert!(posted.has_reach(), "stamp move must re-post reach");
    iso.post_snapshot(moved);
    iso.on_game_tick(3);
    let replaced = iso.probe("__probe").unwrap();
    assert_eq!(replaced["open"], true);
    assert_eq!(
        replaced["other"], false,
        "re-post must replace walkable bits"
    );
    assert_eq!(replaced["step"], false, "re-post must replace step bytes");
    iso.join();
}

#[test]
fn distance_helper_accepts_js_numbers_and_same_plane_arrival() {
    let src = r#"
import { distanceTo } from '../../shim/_kernel.js';
import Tile from '../../geometry/Tile.js';
export default class T extends LoopingBot {
    loop() {
        const here = { x: 3208, z: 3212, level: 0 };
        const dest = { x: 3208, z: 3212, level: 1 };
        const radius = 2000000;
        const arrived = (a, b, r) => a.level === b.level && distanceTo(a, b) <= r;
        globalThis.__probe = {
            fractional: distanceTo({ x: 3208.5, z: 3212, level: 0 }, here),
            nullTile: distanceTo(null, here) === Infinity,

            hugeCross: arrived(here, dest, radius),
            hugeSame: arrived(here, { x: 3208, z: 3212, level: 0 }, radius),
            tileFractional: new Tile(3208, 3212, 0).distanceTo({
                x: 3208.25,
                z: 3212,
                level: 0,
            }),
        };
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").unwrap();
    assert_eq!(value["fractional"], 0.5);
    assert_eq!(value["nullTile"], true);

    assert_eq!(
        value["hugeCross"], false,
        "cross-plane huge radius is not arrived"
    );
    assert_eq!(value["hugeSame"], true);
    assert_eq!(value["tileFractional"], 0.25);
    iso.join();
}

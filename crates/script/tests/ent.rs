//! Ent quartet: native IDs/lifetime and supplied-array tile match.

use script::isolate_fb::{ReachViewInput, SceneEntityInput, SnapshotInput, TileInput};
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 2655,
            z: 3298,
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

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn probe_loop(iso: &LoadIsolate, snap: &SnapshotInput<'_>) -> serde_json::Value {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(snap.tick);
    iso.probe("__probe").expect("probe")
}

fn npc_at<'a>(id: i32, x: i32, z: i32, level: i32) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 1,
        id,
        name: Some("Tree"),
        x,
        z,
        level,
        distance: 0,
        health: 10,
        max_health: 10,
        in_combat: false,
        animating: false,
        actions: &[],
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: 0,
    }
}

#[test]
fn canonical_and_relative_imports_expose_set_and_helpers() {
    let src = r#"
import {
    ENT_NPC_IDS as apiIds,
    ENT_LIFE_TICKS as apiLife,
    isEntNpcId as apiIsEnt,
    entNpcOnTile as apiOnTile,
} from '@rs2b0t/api';
import {
    ENT_NPC_IDS,
    ENT_LIFE_TICKS,
    isEntNpcId,
    entNpcOnTile,
} from '../../data/woodcuttingLocations.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            apiSet: apiIds instanceof Set,
            relSet: ENT_NPC_IDS instanceof Set,
            size: ENT_NPC_IDS.size,
            apiSize: apiIds.size,
            has444: ENT_NPC_IDS.has(444) && apiIds.has(444),
            has453: ENT_NPC_IDS.has(453) || apiIds.has(453),
            life: ENT_LIFE_TICKS,
            apiLife,
            sameLife: ENT_LIFE_TICKS === apiLife,
            is444: isEntNpcId(444) && apiIsEnt(444),
            is453: isEntNpcId(453) || apiIsEnt(453),
            onTileSame: typeof entNpcOnTile === 'function' && typeof apiOnTile === 'function',
        };
    }
}
"#;
    let iso = spawn(src);
    let probe = probe_loop(&iso, &base_snapshot());
    assert_eq!(probe["apiSet"], true);
    assert_eq!(probe["relSet"], true);
    assert_eq!(probe["size"], 9);
    assert_eq!(probe["apiSize"], 9);
    assert_eq!(probe["has444"], true);
    assert_eq!(probe["has453"], false);
    assert_eq!(probe["life"], 60);
    assert_eq!(probe["apiLife"], 60);
    assert_eq!(probe["sameLife"], true);
    assert_eq!(probe["is444"], true);
    assert_eq!(probe["is453"], false);
    assert_eq!(probe["onTileSame"], true);
    iso.join();
}

#[test]
fn is_ent_npc_id_uses_supplied_id() {
    let src = r#"
import { isEntNpcId } from '@rs2b0t/api';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            low: isEntNpcId(443),
            first: isEntNpcId(444),
            last: isEntNpcId(452),
            armour: isEntNpcId(453),
            missing: isEntNpcId(undefined),
            text: isEntNpcId('444'),
        };
    }
}
"#;
    let iso = spawn(src);
    let probe = probe_loop(&iso, &base_snapshot());
    assert_eq!(probe["low"], false);
    assert_eq!(probe["first"], true);
    assert_eq!(probe["last"], true);
    assert_eq!(probe["armour"], false);
    assert_eq!(probe["missing"], false);
    assert_eq!(probe["text"], false);
    iso.join();
}

#[test]
fn ent_npc_on_tile_uses_supplied_array_not_posted_scene() {
    let src = r#"
import { entNpcOnTile } from '@rs2b0t/api';
export default class T extends LoopingBot {
    loop() {
        const tree = { x: 3087, z: 3234, level: 0 };
        const neighbour = { x: 3088, z: 3234, level: 0 };
        const upstairs = { x: 3087, z: 3234, level: 1 };
        globalThis.__probe = {
            hit: entNpcOnTile([{ id: 444, tile: tree }], tree),
            neighbour: entNpcOnTile([{ id: 444, tile: tree }], neighbour),
            plane: entNpcOnTile([{ id: 444, tile: tree }], upstairs),
            wrongId: entNpcOnTile([{ id: 443, tile: tree }], tree),
            armour: entNpcOnTile([{ id: 453, tile: tree }], tree),
            empty: entNpcOnTile([], tree),
            multiple: entNpcOnTile(
                [
                    { id: 443, tile: tree },
                    { id: 452, tile: tree },
                    { id: 444, tile: neighbour },
                ],
                tree,
            ),
            missing: entNpcOnTile(undefined, tree),
        };
    }
}
"#;
    let iso = spawn(src);
    let posted = npc_at(444, 3200, 3200, 0);
    let mut snap = base_snapshot();
    snap.npcs = std::slice::from_ref(&posted);
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["hit"], true, "supplied tree tile must match");
    assert_eq!(probe["neighbour"], false);
    assert_eq!(probe["plane"], false);
    assert_eq!(probe["wrongId"], false);
    assert_eq!(probe["armour"], false);
    assert_eq!(probe["empty"], false);
    assert_eq!(probe["multiple"], true);
    assert_eq!(probe["missing"], false);

    snap.tick = 2;
    snap.npcs = &[];
    let empty_scene = probe_loop(&iso, &snap);
    assert_eq!(
        empty_scene["hit"], true,
        "supplied array must not consult the posted scene"
    );
    iso.join();
}

#[test]
fn life_ticks_are_revision_independent_on_selected_caches() {
    let src = r#"
import { ENT_LIFE_TICKS, ENT_NPC_IDS, isEntNpcId } from '@rs2b0t/api';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            life: ENT_LIFE_TICKS,
            size: ENT_NPC_IDS.size,
            first: isEntNpcId(444),
            armour: isEntNpcId(453),
        };
    }
}
"#;
    let data_274 =
        api::game_data::for_revision(client::io::ClientRevision::R274).expect("274 game data");
    let data_289 =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("289 game data");
    for data in [data_274, data_289] {
        let iso = LoadIsolate::spawn_with_game_data(
            src.to_string(),
            LoadShape::CompatClass,
            vec![],
            data,
        )
        .unwrap();
        let probe = probe_loop(&iso, &base_snapshot());
        assert_eq!(probe["life"], 60);
        assert_eq!(probe["size"], 9);
        assert_eq!(probe["first"], true);
        assert_eq!(probe["armour"], false);
        iso.join();
    }
}

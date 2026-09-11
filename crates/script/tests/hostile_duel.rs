use script::isolate_fb::{
    ReachViewInput, SceneEntityInput, SnapshotInput, TileInput, WidgetTextInput,
};
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

fn npc_row<'a>(
    name: Option<&'a str>,
    in_combat: bool,
    target_kind: i32,
    target_index: i32,
    distance: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 1,
        id: 1,
        name,
        x: 2656,
        z: 3298,
        level: 0,
        distance,
        health: 10,
        max_health: 10,
        in_combat,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 21,
        target_kind,
        target_index,
    }
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

fn probe_loop(iso: &LoadIsolate, snap: &SnapshotInput<'_>) -> serde_json::Value {
    post_snapshot_input(iso, snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    iso.probe("__probe").unwrap()
}

#[test]
fn attacked_by_player_follows_posted_local_face_and_does_not_latch() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = Game.attackedByPlayer();
    }
}
"#;
    let iso = spawn(src);
    let mut snap = base_snapshot();
    snap.attacked_by_player = true;
    assert_eq!(probe_loop(&iso, &snap), true);
    snap.tick = 2;
    snap.attacked_by_player = false;
    assert_eq!(
        probe_loop(&iso, &snap),
        false,
        "a later NPC/none face must not keep the previous player-face true"
    );
    iso.join();
}

#[test]
fn is_hostile_attacker_uses_posted_npc_facts_not_name_table() {
    let src = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
import { isHostileAttacker, HOSTILE_NAMES } from '../../api/thieving/targets.js';
export default class T extends LoopingBot {
    loop() {
        const npc = Npcs.all()[0];
        globalThis.__probe = {
            hostile: npc ? isHostileAttacker(npc, 8) : false,
            names: HOSTILE_NAMES,
        };
    }
}
"#;
    let iso = spawn(src);
    let attack = ["Attack".to_string()];
    let talk = ["Talk-to".to_string()];
    let guard_self = npc_row(Some("Guard"), true, 2, 0, 1, &attack);
    let mut snap = base_snapshot();
    snap.npcs = std::slice::from_ref(&guard_self);
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["hostile"], true);
    assert_eq!(probe["names"], serde_json::json!([]));

    let other = npc_row(Some("Guard"), true, 2, 7, 1, &attack);
    snap.tick = 2;
    snap.npcs = std::slice::from_ref(&other);
    assert_eq!(probe_loop(&iso, &snap)["hostile"], false);

    let man = npc_row(Some("Man"), true, 2, 0, 1, &attack);
    snap.tick = 3;
    snap.npcs = std::slice::from_ref(&man);
    assert_eq!(probe_loop(&iso, &snap)["hostile"], false);

    let case = npc_row(Some("guard"), true, 2, 0, 1, &attack);
    snap.tick = 4;
    snap.npcs = std::slice::from_ref(&case);
    assert_eq!(probe_loop(&iso, &snap)["hostile"], false);

    let far = npc_row(Some("Guard"), true, 2, 0, 9, &attack);
    snap.tick = 5;
    snap.npcs = std::slice::from_ref(&far);
    assert_eq!(probe_loop(&iso, &snap)["hostile"], false);

    let talk_only = npc_row(Some("Guard"), true, 2, 0, 1, &talk);
    snap.tick = 6;
    snap.npcs = std::slice::from_ref(&talk_only);
    assert_eq!(probe_loop(&iso, &snap)["hostile"], false);

    snap.tick = 7;
    snap.npcs = &[];
    assert_eq!(
        probe_loop(&iso, &snap)["hostile"],
        false,
        "stale identity after NPC removal is not a hostile observation"
    );
    iso.join();
}

#[test]
fn if_text_reads_selected_posted_widgets_not_stale_labels() {
    let src = r#"
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            partner: reader.ifText(6671),
            waiting: reader.ifText(6684),
            confirm: reader.ifText(6571),
            missing: reader.ifText(1),
            bad: reader.ifText('nope'),
        };
    }
}
"#;
    let iso = spawn(src);
    let widgets = [
        WidgetTextInput {
            component_id: 6671,
            text: "Zezima",
        },
        WidgetTextInput {
            component_id: 6684,
            text: "Waiting for other player...",
        },
    ];
    let mut snap = base_snapshot();
    snap.main_modal_id = 6575;
    snap.widgets = &widgets;
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["partner"], "Zezima");
    assert_eq!(probe["waiting"], "Waiting for other player...");
    assert_eq!(probe["confirm"], serde_json::Value::Null);
    assert_eq!(probe["missing"], serde_json::Value::Null);
    assert_eq!(probe["bad"], serde_json::Value::Null);

    snap.tick = 2;
    snap.main_modal_id = -1;
    snap.widgets = &[];
    let closed = probe_loop(&iso, &snap);
    assert_eq!(
        closed["partner"],
        serde_json::Value::Null,
        "closed selected modal must not keep the previous partner label"
    );
    iso.join();
}

#[test]
fn hold_freezes_loop_but_keeps_posted_hostile_and_widget_facts() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
import { reader } from '../../adapter/ClientAdapter.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            attacked: Game.attackedByPlayer(),
            partner: reader.ifText(6671),
        };
    }
}
"#;
    let iso = spawn(src);
    let widgets = [WidgetTextInput {
        component_id: 6671,
        text: "Partner",
    }];
    let mut snap = base_snapshot();
    snap.hold = true;
    snap.attacked_by_player = true;
    snap.widgets = &widgets;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let looped = iso.probe("__probe");
    assert!(
        looped.is_err(),
        "Guardian hold must freeze loop(); got {looped:?}"
    );
    assert_eq!(
        iso.probe("globalThis.__rs2b0t_host.snapshot.attacked_by_player")
            .unwrap(),
        true
    );
    assert_eq!(
        iso.probe(
            "(globalThis.__rs2b0t_host.snapshot.widgets || []).find((w) => w.component_id === 6671).text"
        )
        .unwrap(),
        "Partner"
    );
    iso.join();
}

#[test]
fn duel_controls_match_both_selected_caches() {
    let src = r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = (globalThis.__rs2b0t_host.content || {}).duel || null;
    }
}
"#;
    for data in [data(), data_289()] {
        let iso = LoadIsolate::spawn_with_game_data(
            src.to_string(),
            LoadShape::CompatClass,
            vec![],
            data,
        )
        .unwrap();
        let snap = base_snapshot();
        let probe = probe_loop(&iso, &snap);
        assert_eq!(probe["select_modal"], 6575);
        assert_eq!(probe["confirm_modal"], 6412);
        assert_eq!(probe["win_modal"], 6733);
        assert_eq!(probe["select_accept"], 6674);
        assert_eq!(probe["confirm_accept"], 6520);
        assert_eq!(probe["select_partner"], 6671);
        assert_eq!(probe["select_status"], 6684);
        assert_eq!(probe["confirm_status"], 6571);
        iso.join();
    }
}

#[test]
fn missing_distance_or_max_refuses_hostile_true() {
    let src = r#"
import { isHostileAttacker } from '../../api/thieving/targets.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            noMax: isHostileAttacker({
                name: 'Guard',
                inCombat: true,
                targetsAnotherPlayer: false,
                distance: 1,
                actions: ['Attack'],
            }),
            noDistance: isHostileAttacker({
                name: 'Guard',
                inCombat: true,
                targetsAnotherPlayer: false,
                actions: ['Attack'],
            }, 8),
        };
    }
}
"#;
    let iso = spawn(src);
    let snap = base_snapshot();
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["noMax"], false);
    assert_eq!(probe["noDistance"], false);
    iso.join();
}

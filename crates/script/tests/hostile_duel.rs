use script::isolate_fb::{
    ChatOptionInput, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput, VarpInput,
    WidgetTextInput,
};
use script::{LoadIsolate, LoadShape};

mod common;
use common::post_snapshot_input;

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
            face: npc?.snap.faceEntity ?? null,
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
    assert_eq!(probe["face"], 32768);
    assert_eq!(probe["names"], serde_json::json!([]));

    let other = npc_row(Some("Guard"), true, 2, 7, 1, &attack);
    snap.tick = 2;
    snap.npcs = std::slice::from_ref(&other);
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["hostile"], false);
    assert_eq!(probe["face"], 32775);

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
            stake: reader.ifText(6669),
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
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6684,
            text: "Waiting for other player...",
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6669,
            text: "",
            item_count: 0,
        },
    ];
    let mut snap = base_snapshot();
    snap.main_modal_id = 6575;
    snap.widgets = &widgets;
    let probe = probe_loop(&iso, &snap);
    assert_eq!(probe["partner"], "Zezima");
    assert_eq!(probe["waiting"], "Waiting for other player...");
    assert_eq!(probe["confirm"], serde_json::Value::Null);
    assert_eq!(
        probe["stake"],
        serde_json::Value::Null,
        "an inventory component has no text"
    );
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
        item_count: -1,
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

fn handshake_source(partner: &str, initiator: bool) -> String {
    format!(
        r#"
import {{ ClueDuelHandshake }} from '../../api/duel/ClueDuel.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__done) return;
        this.handshake ||= new ClueDuelHandshake({partner:?}, {initiator}, () => {{}});
        globalThis.__result = await this.handshake.tick();
        globalThis.__done = true;
    }}
}}
"#
    )
}

fn run_handshake(
    partner: &str,
    initiator: bool,
    snapshot: &SnapshotInput<'_>,
) -> Vec<script::shim::InteractReq> {
    let iso = spawn(&handshake_source(partner, initiator));
    post_snapshot_input(&iso, snapshot);
    iso.on_game_tick(snapshot.tick);
    let _ = iso.probe("true");
    let requests = iso.drain_interacts();
    iso.join();
    requests
}

#[test]
fn clue_duel_both_roles_challenge_the_named_visible_partner() {
    let challenge = ["Challenge".to_string()];
    let player = npc_row(Some("Helper_Name"), false, 0, -1, 1, &challenge);
    let mut snapshot = base_snapshot();
    snapshot.here = Some(TileInput {
        x: 3368,
        z: 3274,
        level: 0,
    });
    snapshot.players = std::slice::from_ref(&player);
    for initiator in [false, true] {
        assert_eq!(
            run_handshake(" helper name ", initiator, &snapshot),
            vec![script::shim::InteractReq::Player {
                name: "Helper_Name".into(),
                action: "Challenge".into(),
            }]
        );
    }
}

#[test]
fn clue_duel_rejects_wrong_partner_and_any_stake() {
    let wrong = [
        WidgetTextInput {
            component_id: 6671,
            text: "Dueling with: Intruder",
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6669,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6670,
            text: "",
            item_count: 0,
        },
    ];
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6575;
    snapshot.widgets = &wrong;
    assert_eq!(
        run_handshake("Helper", true, &snapshot),
        vec![script::shim::InteractReq::CloseModal]
    );

    let staked = [
        WidgetTextInput {
            component_id: 6671,
            text: "Dueling with: Helper",
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6669,
            text: "",
            item_count: 1,
        },
        WidgetTextInput {
            component_id: 6670,
            text: "",
            item_count: 0,
        },
    ];
    snapshot.widgets = &staked;
    assert_eq!(
        run_handshake("Helper", false, &snapshot),
        vec![script::shim::InteractReq::CloseModal]
    );
}

#[test]
fn clue_duel_initiator_sets_obstacle_rules_when_zero_varp_is_not_transmitted() {
    let offer = [
        WidgetTextInput {
            component_id: 6671,
            text: "Dueling with: Helper",
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6669,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6670,
            text: "",
            item_count: 0,
        },
    ];
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6575;
    snapshot.widgets = &offer;
    snapshot.varps = &[];
    assert_eq!(
        run_handshake("Helper", true, &snapshot),
        vec![script::shim::InteractReq::IfButton { component_id: 6732 }]
    );
    assert!(
        run_handshake("Helper", false, &snapshot).is_empty(),
        "the helper waits for the solver to select obstacle rules"
    );
}
#[test]
fn clue_duel_helper_accepts_only_after_the_initiator_is_observed_accepted() {
    let options = [VarpInput {
        index: 286,
        value: 1024,
    }];
    let offer = |status: &'static str| {
        [
            WidgetTextInput {
                component_id: 6671,
                text: "Dueling with: Helper",
                item_count: -1,
            },
            WidgetTextInput {
                component_id: 6669,
                text: "",
                item_count: 0,
            },
            WidgetTextInput {
                component_id: 6670,
                text: "",
                item_count: 0,
            },
            WidgetTextInput {
                component_id: 6684,
                text: status,
                item_count: -1,
            },
        ]
    };
    let initial = offer("");
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6575;
    snapshot.varps = &options;
    snapshot.widgets = &initial;
    assert!(matches!(
        run_handshake("Helper", true, &snapshot).as_slice(),
        [script::shim::InteractReq::DuelAccept { screen, .. }] if screen == "offer"
    ));
    assert!(
        run_handshake("Helper", false, &snapshot).is_empty(),
        "the helper must not race the initiator's accept packet"
    );

    let accepted = offer("Other player has accepted.");
    snapshot.widgets = &accepted;
    assert!(matches!(
        run_handshake("Helper", false, &snapshot).as_slice(),
        [script::shim::InteractReq::DuelAccept { screen, .. }] if screen == "offer"
    ));
}

#[test]
fn clue_duel_offer_identity_does_not_authorize_a_staked_confirm() {
    let source = r#"
import { ClueDuelHandshake } from '../../api/duel/ClueDuel.js';
export default class T extends LoopingBot {
    async loop() {
        this.handshake ||= new ClueDuelHandshake('Helper', true, () => {});
        globalThis.__result = await this.handshake.tick();
    }
}
"#;
    let isolate = spawn(source);
    let options = [VarpInput {
        index: 286,
        value: 1024,
    }];
    let offer = [
        WidgetTextInput {
            component_id: 6671,
            text: "Dueling with: Helper",
            item_count: -1,
        },
        WidgetTextInput {
            component_id: 6669,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6670,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6684,
            text: "",
            item_count: -1,
        },
    ];
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6575;
    snapshot.varps = &options;
    snapshot.widgets = &offer;
    post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let _ = isolate.probe("globalThis.__result");
    assert_eq!(
        isolate.drain_interacts(),
        vec![script::shim::InteractReq::DuelAccept {
            screen: "offer".into(),
            partner: "Helper".into(),
            rules: 1024,
        }]
    );

    let confirm = [
        WidgetTextInput {
            component_id: 6507,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6508,
            text: "",
            item_count: 1,
        },
        WidgetTextInput {
            component_id: 6571,
            text: "",
            item_count: -1,
        },
    ];
    std::thread::sleep(std::time::Duration::from_millis(650));
    snapshot.tick = 2;
    snapshot.main_modal_id = 6412;
    snapshot.widgets = &confirm;
    post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(2);
    let _ = isolate.probe("globalThis.__result");
    assert_eq!(
        isolate.drain_interacts(),
        vec![script::shim::InteractReq::CloseModal],
        "a stake inserted after a valid offer must cancel instead of confirming"
    );
    isolate.join();
}

#[test]
fn clue_duel_confirm_without_a_validated_offer_is_rejected() {
    let widgets = [
        WidgetTextInput {
            component_id: 6507,
            text: "",
            item_count: 0,
        },
        WidgetTextInput {
            component_id: 6508,
            text: "",
            item_count: 0,
        },
    ];
    let options = [VarpInput {
        index: 286,
        value: 1024,
    }];
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6412;
    snapshot.widgets = &widgets;
    snapshot.varps = &options;
    assert_eq!(
        run_handshake("Helper", true, &snapshot),
        vec![script::shim::InteractReq::CloseModal]
    );
}

#[test]
fn clue_duel_leave_targets_the_nearest_forfeit_exit() {
    let actions = ["Forfeit".to_string()];
    let loc = |id, x, z| SceneEntityInput {
        index: id,
        id,
        name: Some("Trapdoor"),
        x,
        z,
        level: 0,
        distance: 0,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    };
    let locs = [loc(1, 3331, 3231), loc(2, 3365, 3245)];
    let mut snapshot = base_snapshot();
    snapshot.here = Some(TileInput {
        x: 3374,
        z: 3251,
        level: 0,
    });
    snapshot.locs = &locs;
    let iso = spawn(
        r#"
import { leaveClueDuel } from '../../api/duel/ClueDuel.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        await leaveClueDuel(() => {});
    }
}
"#,
    );
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(snapshot.tick);
    let _ = iso.probe("true");
    snapshot.tick += 1;
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(snapshot.tick);
    let _ = iso.probe("true");
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::Loc {
            x: 3365,
            z: 3245,
            level: 0,
            action: "Forfeit".into(),
            id: Some(2),
        }]
    );
    iso.join();
}

/// Frozen `walkAcrossClueDuel` from inside a pen to an outside tile: it
/// forfeits (logging it), answers 'Yes', then walks the destination leg as
/// a resilient walk whose wait settles on its own request.
#[test]
fn clue_duel_travel_walks_to_an_outside_destination_after_forfeit() {
    let actions = ["Forfeit".to_string()];
    let locs = [SceneEntityInput {
        index: 2,
        id: 3203,
        name: Some("Arena exit"),
        x: 3364,
        z: 3251,
        level: 0,
        distance: 10,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }];
    let mut snapshot = base_snapshot();
    snapshot.here = Some(TileInput {
        x: 3374,
        z: 3251,
        level: 0,
    });
    snapshot.locs = &locs;
    let iso = spawn(
        r#"
import { runMachine } from '../../shim/_kernel.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__log = [];
        await runMachine('clue_duel_travel', {
            x: 3382, z: 3269, level: 0, radius: 2,
        }, { log: (line) => globalThis.__log.push(line) });
    }
}
"#,
    );
    for tick in 1..=2 {
        snapshot.tick = tick;
        post_snapshot_input(&iso, &snapshot);
        iso.on_game_tick(tick);
        let _ = iso.probe("true");
    }
    let choices = [
        ChatOptionInput {
            text: "Yes",
            com_id: 4883,
        },
        ChatOptionInput {
            text: "No",
            com_id: 4884,
        },
    ];
    snapshot.tick = 3;
    snapshot.chat_options = &choices;
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(3);
    let _ = iso.probe("true");

    snapshot.tick = 4;
    snapshot.here = Some(TileInput {
        x: 3361,
        z: 3270,
        level: 0,
    });
    snapshot.chat_options = &[];
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(4);
    let _ = iso.probe("true");

    let requests = iso.drain_interacts();
    assert_eq!(
        requests[..2],
        [
            script::shim::InteractReq::Loc {
                x: 3364,
                z: 3251,
                level: 0,
                action: "Forfeit".into(),
                id: Some(3203),
            },
            script::shim::InteractReq::Answer { option: 1 },
        ]
    );
    assert!(
        matches!(
            &requests[2..],
            [script::shim::InteractReq::WalkNear {
                x: 3382,
                z: 3269,
                level: 0,
                radius: 2,
                request_id,
                ..
            }] if *request_id != 0
        ),
        "{requests:?}"
    );
    assert_eq!(
        iso.probe("globalThis.__log").unwrap(),
        serde_json::json!(["forfeiting the clue duel"])
    );
    iso.join();
}

/// A travel from the lobby into the packed clue's pen, logging to
/// `globalThis.__log` and settling into `globalThis.__crossed`.
const CROSS_TO_CLUE_PEN: &str = r#"
import { walkAcrossClueDuel } from '../../api/ai/clues/duelTravel.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__log = [];
        globalThis.__crossed = await walkAcrossClueDuel(
            { x: 3374, z: 3250, level: 0 }, 1, (line) => globalThis.__log.push(line));
    }
}
"#;

/// Frozen `walkAcrossClueDuel` stops on an event signal while it waits for
/// the helper, cancelling the duel screen only when one is open.
#[test]
fn clue_duel_travel_interrupt_cancels_only_an_open_duel() {
    let offer = [WidgetTextInput {
        component_id: 6671,
        text: "Dueling with: Helper",
        item_count: -1,
    }];
    for offer_open in [false, true] {
        let iso = spawn(CROSS_TO_CLUE_PEN);
        iso.post_settings_bag(
            serde_json::json!({ "clueDuelPartner": "Helper" })
                .as_object()
                .unwrap(),
        );
        let mut snapshot = base_snapshot();
        snapshot.here = Some(TileInput {
            x: 3368,
            z: 3274,
            level: 0,
        });
        snapshot.my_name = Some("Solver");
        post_snapshot_input(&iso, &snapshot);
        iso.on_game_tick(1);
        let _ = iso.probe("true");
        assert!(iso.drain_interacts().is_empty(), "waiting at the lobby");

        iso.probe("globalThis.__rs2b0t_event_interrupt = () => true, true")
            .unwrap();
        snapshot.tick = 2;
        if offer_open {
            snapshot.main_modal_id = 6575;
            snapshot.widgets = &offer;
        }
        post_snapshot_input(&iso, &snapshot);
        iso.on_game_tick(2);
        let _ = iso.probe("true");
        assert_eq!(
            iso.drain_interacts(),
            if offer_open {
                vec![script::shim::InteractReq::CloseModal]
            } else {
                Vec::new()
            },
            "offer open: {offer_open}"
        );
        assert_eq!(
            iso.probe("globalThis.__crossed").unwrap(),
            false,
            "offer open: {offer_open}"
        );
        assert_eq!(
            iso.probe("globalThis.__log").unwrap(),
            serde_json::json!(["waiting for clue helper Helper"])
        );
        iso.join();
    }
}

/// Frozen `Duel.cancel()` is `Modals.close()` on an open duel screen: it
/// resolves once the modal closed, and false with no duel screen open.
#[test]
fn duel_cancel_awaits_the_closed_duel_screen() {
    let iso = spawn(
        r#"
import { Duel } from '../../api/duel/Duel.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__cancelled = await Duel.cancel();
        globalThis.__again = await Duel.cancel();
    }
}
"#,
    );
    let mut snapshot = base_snapshot();
    snapshot.main_modal_id = 6575;
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    assert_eq!(
        iso.drain_interacts(),
        vec![script::shim::InteractReq::CloseModal]
    );
    assert_eq!(
        iso.probe("globalThis.__cancelled").unwrap(),
        serde_json::Value::Null,
        "Duel.cancel waits for the offer screen to close"
    );

    snapshot.tick = 2;
    snapshot.main_modal_id = -1;
    post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    assert_eq!(iso.probe("globalThis.__cancelled").unwrap(), true);
    assert_eq!(
        iso.probe("globalThis.__again").unwrap(),
        false,
        "no duel screen: nothing to cancel"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

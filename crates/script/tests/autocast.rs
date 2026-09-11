use script::isolate_fb::{ReachViewInput, SideTabIfaceInput, SnapshotInput, TileInput, VarpInput};
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

const ARM: &str = r#"
import { Autocast } from '../../api/magic/Autocast.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Autocast.arm('Wind Strike', (m) => this.log(m));
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

fn if_button(id: i32) -> InteractReq {
    InteractReq::IfButton { component_id: id }
}

#[test]
fn missing_controls_throw_and_do_not_send() {
    let src = ARM;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing autocast controls must throw, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn unknown_spell_refuses_without_send() {
    let src = r#"
import { Autocast } from '../../api/magic/Autocast.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = await Autocast.arm('Not a spell', (m) => this.log(m));
    }
}
"#;
    let iso = spawn(src);
    let tabs = [SideTabIfaceInput { index: 0, id: 328 }];
    let varps = [VarpInput {
        index: 108,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.side_tab_ifaces = &tabs;
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn staff_not_attached_refuses_without_send() {
    let iso = spawn(ARM);
    let tabs = [SideTabIfaceInput { index: 0, id: 192 }];
    let mut snap = base_snapshot();
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn initially_armed_reads_posted_varp_on_both_selected_caches() {
    let src = r#"
import { Autocast } from '../../api/magic/Autocast.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = { armed: Autocast.armed(), staff: Autocast.staffTabAttached() };
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
        let tabs = [SideTabIfaceInput { index: 0, id: 328 }];
        let varps = [VarpInput {
            index: 108,
            value: 3,
        }];
        let mut snap = base_snapshot();
        snap.side_tab_ifaces = &tabs;
        snap.varps = &varps;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 1);
        let probe = iso.probe("__probe").unwrap();
        assert_eq!(probe["armed"], true);
        assert_eq!(probe["staff"], true);
        iso.join();
    }
}

#[test]
fn arm_three_presses_then_posted_armed_state() {
    let iso = spawn(ARM);
    let staff = [SideTabIfaceInput { index: 0, id: 328 }];
    let idle = [VarpInput {
        index: 108,
        value: 0,
    }];
    let mut snap = base_snapshot();
    snap.side_tab_ifaces = &staff;
    snap.varps = &idle;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(353)],
        "choose is the first press; queued buttons are not armed"
    );
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);

    let panel = [SideTabIfaceInput { index: 0, id: 1829 }];
    snap.tick = 2;
    snap.side_tab_ifaces = &panel;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.drain_interacts(), vec![if_button(1830)]);

    let selected = [VarpInput {
        index: 108,
        value: 2,
    }];
    snap.tick = 3;
    snap.varps = &selected;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(iso.drain_interacts(), vec![if_button(349)]);

    let armed = [VarpInput {
        index: 108,
        value: 3,
    }];
    let staff_again = [SideTabIfaceInput { index: 0, id: 328 }];
    snap.tick = 4;
    snap.varps = &armed;
    snap.side_tab_ifaces = &staff_again;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn stale_chooser_times_out_after_choose() {
    let iso = spawn(ARM);
    let staff = [SideTabIfaceInput { index: 0, id: 328 }];
    let mut snap = base_snapshot();
    snap.side_tab_ifaces = &staff;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(353)]);
    for n in 2..=4 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
    }
    assert!(iso.probe("__ok").unwrap().is_null());
    assert!(
        iso.drain_interacts().is_empty(),
        "stale staff root must not press the grid"
    );
    iso.join();
}

#[test]
fn pause_and_hold_freeze_mid_arm_without_late_press() {
    let iso = spawn(ARM);
    let staff = [SideTabIfaceInput { index: 0, id: 328 }];
    let mut snap = base_snapshot();
    snap.side_tab_ifaces = &staff;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(353)]);

    iso.pause();
    let panel = [SideTabIfaceInput { index: 0, id: 1829 }];
    snap.tick = 2;
    snap.side_tab_ifaces = &panel;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "Pause must not press the spell grid"
    );
    iso.resume();

    snap.hold = true;
    snap.tick = 4;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    tick(&iso, 5);
    assert!(
        iso.drain_interacts().is_empty(),
        "Guardian hold must freeze the parked chooser wait"
    );
    iso.reset_session_work();
    snap.hold = false;
    snap.tick = 6;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(
        iso.drain_interacts()
            .iter()
            .all(|req| !matches!(req, InteractReq::IfButton { component_id: 1830 })),
        "session abort must drop the parked arm"
    );
    iso.join();
}

#[test]
fn melee_resolution_shape_is_unchanged() {
    let src = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = Game.combatStyleResolution('strength');
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    iso.probe("globalThis.__rs2b0t_host.snapshot = {combat_styles:[{mode:1,label:'Aggressive',component_id:77}]}").unwrap();
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["requested"], "strength");
    assert_eq!(probe["effective"], "strength");
    assert_eq!(probe["mode"], 1);
    iso.join();
}

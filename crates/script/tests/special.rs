use script::isolate_fb::{
    ItemRowInput, ReachViewInput, SideTabIfaceInput, SnapshotInput, TileInput, VarpInput,
};
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
import { Special } from '../../api/combat/Special.js';
export default class T extends LoopingBot {
    async loop() {
        globalThis.__ok = null;
        try {
            globalThis.__ok = await Special.arm();
        } catch (e) {
            globalThis.__ok = String(e.message || e);
        }
    }
}
"#;

fn if_button(id: i32) -> InteractReq {
    InteractReq::IfButton { component_id: id }
}

fn energy_armed(energy: i32, armed: i32) -> [VarpInput; 2] {
    [
        VarpInput {
            index: 300,
            value: energy,
        },
        VarpInput {
            index: 301,
            value: armed,
        },
    ]
}

#[test]
fn missing_controls_throw_and_do_not_send() {
    let iso = LoadIsolate::spawn(ARM.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let varps = energy_armed(1000, 0);
    let mut snap = base_snapshot();
    snap.varps = &varps;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let probe = iso.probe("__ok").unwrap();
    assert!(
        probe.as_str().unwrap_or("").contains("not impl"),
        "missing special controls must throw, got {probe:?}"
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn costs_ready_and_wielded_on_both_selected_caches() {
    let src = r#"
import { Special, SA_MAX_ENERGY } from '../../api/combat/Special.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            dagger: Special.cost('Dragon dagger'),
            poisoned: Special.cost('  dragon DAGGER(p) '),
            scim: Special.cost('Rune scimitar'),
            axe: Special.cost('Dragon battleaxe'),
            ready: Special.ready('Dragon dagger'),
            energy: Special.energy(),
            armed: Special.armed(),
            wielded: Special.wielded(),
            bar: Special.barComponent(),
            max: SA_MAX_ENERGY,
        };
    }
}
"#;
    let equipment = [ItemRowInput {
        name: Some("Dragon dagger"),
        count: 1,
        id: 1215,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 3,
    }];
    let tabs = [SideTabIfaceInput { index: 0, id: 2276 }];
    for (rev, selected) in [(274, data()), (289, data_289())] {
        let iso = LoadIsolate::spawn_with_game_data(
            src.to_string(),
            LoadShape::CompatClass,
            vec![],
            selected,
        )
        .unwrap();
        let varps = energy_armed(200, 0);
        let mut snap = base_snapshot();
        snap.varps = &varps;
        snap.equipment = &equipment;
        snap.side_tab_ifaces = &tabs;
        post_snapshot_input(&iso, &snap);
        tick(&iso, 1);
        let probe = iso.probe("__probe").unwrap();
        assert_eq!(probe["dagger"], 250, "rev {rev}");
        assert_eq!(probe["poisoned"], 250, "rev {rev}");
        assert_eq!(probe["scim"], serde_json::Value::Null, "rev {rev}");
        assert_eq!(probe["axe"], serde_json::Value::Null, "rev {rev}");
        assert_eq!(probe["ready"], false, "rev {rev} insufficient energy");
        assert_eq!(probe["energy"], 200, "rev {rev}");
        assert_eq!(probe["armed"], false, "rev {rev}");
        assert_eq!(probe["wielded"], "Dragon dagger", "rev {rev}");
        assert_eq!(probe["bar"], 7562, "rev {rev} stabsword bar");
        assert_eq!(probe["max"], 1000, "rev {rev}");
        iso.join();
    }
}

#[test]
fn already_armed_is_idempotent() {
    let iso = spawn(ARM);
    let varps = energy_armed(1000, 1);
    let tabs = [SideTabIfaceInput { index: 0, id: 2276 }];
    let mut snap = base_snapshot();
    snap.varps = &varps;
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert!(
        iso.drain_interacts().is_empty(),
        "already armed must not click the bar"
    );
    iso.join();
}

#[test]
fn no_bar_refuses_without_send() {
    let iso = spawn(ARM);
    let varps = energy_armed(1000, 0);
    let tabs = [SideTabIfaceInput { index: 0, id: 328 }];
    let mut snap = base_snapshot();
    snap.varps = &varps;
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(
        iso.drain_interacts().is_empty(),
        "staff combat tab has no spec bar"
    );
    iso.join();
}

#[test]
fn arm_clicks_bar_then_posted_armed_state() {
    let iso = spawn(ARM);
    let idle = energy_armed(1000, 0);
    let tabs = [SideTabIfaceInput { index: 0, id: 2276 }];
    let mut snap = base_snapshot();
    snap.varps = &idle;
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![if_button(7562)],
        "queued bar click is not an armed special"
    );
    assert_eq!(iso.probe("__ok").unwrap(), serde_json::Value::Null);

    let armed = energy_armed(1000, 1);
    snap.tick = 2;
    snap.varps = &armed;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(iso.probe("__ok").unwrap(), true);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn timeout_without_armed_returns_false() {
    let iso = spawn(ARM);
    let idle = energy_armed(1000, 0);
    let tabs = [SideTabIfaceInput { index: 0, id: 2276 }];
    let mut snap = base_snapshot();
    snap.varps = &idle;
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(7562)]);
    for n in 2..=3 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(iso.probe("__ok").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn weapon_change_selects_the_posted_combat_bar() {
    let src = r#"
import { Special } from '../../api/combat/Special.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = {
            wielded: Special.wielded(),
            bar: Special.barComponent(),
            cost: Special.cost(Special.wielded()),
        };
    }
}
"#;
    let iso = spawn(src);
    let dagger = [ItemRowInput {
        name: Some("Dragon dagger"),
        count: 1,
        id: 1215,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 3,
    }];
    let scim = [ItemRowInput {
        name: Some("Rune scimitar"),
        count: 1,
        id: 1333,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 3,
    }];
    let stab = [SideTabIfaceInput { index: 0, id: 2276 }];
    let hack = [SideTabIfaceInput { index: 0, id: 2423 }];
    let varps = energy_armed(1000, 0);
    let mut snap = base_snapshot();
    snap.varps = &varps;
    snap.equipment = &dagger;
    snap.side_tab_ifaces = &stab;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let first = iso.probe("__probe").unwrap();
    assert_eq!(first["wielded"], "Dragon dagger");
    assert_eq!(first["bar"], 7562);
    assert_eq!(first["cost"], 250);

    snap.tick = 2;
    snap.equipment = &scim;
    snap.side_tab_ifaces = &hack;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let second = iso.probe("__probe").unwrap();
    assert_eq!(second["wielded"], "Rune scimitar");
    assert_eq!(second["bar"], 7587);
    assert_eq!(second["cost"], serde_json::Value::Null);
    iso.join();
}

#[test]
fn pause_and_hold_freeze_mid_arm_without_late_click() {
    let iso = spawn(ARM);
    let idle = energy_armed(1000, 0);
    let tabs = [SideTabIfaceInput { index: 0, id: 2276 }];
    let mut snap = base_snapshot();
    snap.varps = &idle;
    snap.side_tab_ifaces = &tabs;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.drain_interacts(), vec![if_button(7562)]);

    iso.pause();
    let armed = energy_armed(1000, 1);
    snap.tick = 2;
    snap.varps = &armed;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__ok").unwrap(),
        serde_json::Value::Null,
        "Pause must not complete the parked arm"
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
        "Guardian hold must freeze the parked arm wait"
    );
    iso.reset_session_work();
    let staff = [SideTabIfaceInput { index: 0, id: 328 }];
    snap.hold = false;
    snap.tick = 6;
    snap.varps = &idle;
    snap.side_tab_ifaces = &staff;
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

#[test]
fn energy_without_posted_varp_still_throws() {
    let src = r#"
import { Special } from '../../api/combat/Special.js';
export default class T extends LoopingBot {
    loop() { Special.energy(); }
}
"#;
    let iso = spawn(src);
    post_snapshot_input(&iso, &base_snapshot());
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_bot");
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|l| l.contains("not impl") && l.contains("Special")),
        "Special.energy must not invent 0 when sa_energy was never posted: {logs:?}"
    );
    iso.join();
}

//! Native one-shot `Paint.buttons` seam. No packet opcode, no fabricated click.

use script::isolate_fb::{decode_paint, IsolateBuf, ReachViewInput, SnapshotInput, TileInput};
use script::shim::{ScriptPaint, ScriptPaintButton};
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

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn got(iso: &LoadIsolate) -> i64 {
    iso.probe("globalThis.__got || 0")
        .unwrap()
        .as_i64()
        .unwrap()
}

fn clicked(iso: &LoadIsolate) -> serde_json::Value {
    iso.probe("globalThis.__clicked").unwrap()
}

const PAINT: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__loops = (globalThis.__loops || 0) + 1;
    }
    onPaint() {
        const p = Paint.begin(null, {});
        p.title('NatureCrafter');
        p.row('status');
        const c = p.buttons([{id:'gobank', label: globalThis.__label || 'Go bank'}]);
        if (c) {
            globalThis.__clicked = c;
            globalThis.__got = (globalThis.__got || 0) + 1;
        } else if (globalThis.__clicked === undefined) {
            globalThis.__clicked = null;
        }
        p.end();
    }
}
"#;

#[test]
fn buttons_does_not_throw_and_headless_returns_null() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .all(|l| !l.contains("not impl") && !l.starts_with("tick ")),
        "Paint.buttons must not throw: {logs:?}"
    );
    assert_eq!(clicked(&iso), serde_json::Value::Null);
    assert_eq!(got(&iso), 0);
    let paint = iso.paint().expect("paint forwarded");
    assert_eq!(
        paint.buttons,
        vec![ScriptPaintButton {
            id: "gobank".into(),
            label: "Go bank".into(),
        }]
    );
    iso.join();
}

#[test]
fn paint_click_is_consumed_once() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    iso.paint_click("gobank");
    tick(&iso, 2);
    assert_eq!(clicked(&iso).as_str(), Some("gobank"));
    assert_eq!(got(&iso), 1, "one-shot even if onPaint runs twice");
    tick(&iso, 3);
    assert_eq!(got(&iso), 1);
    iso.join();
}

#[test]
fn unadvertised_id_returns_null_and_clears_pending() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    iso.paint_click("nope");
    tick(&iso, 2);
    assert_eq!(clicked(&iso), serde_json::Value::Null);
    assert_eq!(
        iso.probe("globalThis.__rs2b0t_host.paintClick").unwrap(),
        serde_json::Value::Null
    );
    iso.join();
}

#[test]
fn pause_drops_click_and_resume_does_not_replay() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    iso.pause();
    iso.paint_click("gobank");
    tick(&iso, 2);
    assert_eq!(clicked(&iso), serde_json::Value::Null);
    iso.resume();
    tick(&iso, 3);
    assert_eq!(clicked(&iso), serde_json::Value::Null);
    iso.join();
}

#[test]
fn hold_still_consumes_and_keeps_loop_frozen() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    assert_eq!(iso.probe("__loops").unwrap(), 1);
    let mut snap = base_snapshot();
    snap.hold = true;
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.paint_click("gobank");
    tick(&iso, 2);
    assert_eq!(iso.probe("__loops").unwrap(), 1, "hold freezes loop");
    assert_eq!(clicked(&iso).as_str(), Some("gobank"));
    iso.join();
}

#[test]
fn click_on_one_isolate_never_returns_from_another() {
    let a = spawn(PAINT);
    let b = spawn(PAINT);
    tick(&a, 1);
    tick(&b, 1);
    a.paint_click("gobank");
    tick(&a, 2);
    tick(&b, 2);
    assert_eq!(got(&a), 1);
    assert_eq!(got(&b), 0);
    a.join();
    b.join();
}

#[test]
fn generation_skip_drops_pending_click() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    iso.paint_click("gobank");
    iso.reset_session_work();
    tick(&iso, 2);
    assert_eq!(clicked(&iso), serde_json::Value::Null);
    iso.join();
}

#[test]
fn stale_frame_generation_rejects_same_id_on_later_script() {
    let iso = spawn(PAINT);
    tick(&iso, 1);
    let stale = iso.paint().expect("first frame");
    assert!(stale.buttons.iter().any(|b| b.id == "gobank"));
    assert_ne!(stale.generation, 0, "host stamps owned generation");
    iso.reset_session_work();
    tick(&iso, 2);
    let current = iso.paint().expect("later frame");
    assert!(
        current.buttons.iter().any(|b| b.id == "gobank"),
        "replacement script still advertises gobank"
    );
    assert_ne!(
        stale.generation, current.generation,
        "stale overlay generation must not match the later frame"
    );
    iso.join();
}

#[test]
fn two_buttons_calls_append_and_consume_this_call_only() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        p.title('two');
        globalThis.__a = p.buttons([{id:'gobank', label:'Go bank'}]) || globalThis.__a || null;
        globalThis.__b = p.buttons([{id:'other', label:'Other'}]) || globalThis.__b || null;
        p.buttons([]);
        p.end();
    }
}
"#;
    let iso = spawn(src);
    iso.paint_click("other");
    tick(&iso, 1);
    assert_eq!(iso.probe("__a").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.probe("__b").unwrap().as_str(), Some("other"));
    let paint = iso.paint().expect("forwarded");
    assert_eq!(
        paint.buttons,
        vec![
            ScriptPaintButton {
                id: "gobank".into(),
                label: "Go bank".into(),
            },
            ScriptPaintButton {
                id: "other".into(),
                label: "Other".into(),
            },
        ]
    );
    iso.join();
}

#[test]
fn empty_old_paint_still_decodes_and_label_toggle_is_a_change() {
    let quiet = ScriptPaint {
        title: Some("t".into()),
        accent: None,
        lines: vec!["line".into()],
        buttons: Vec::new(),
        generation: 0,
    };
    let bytes = IsolateBuf::new().encode_paint(&quiet);
    let decoded = decode_paint(&bytes).expect("empty buttons");
    assert!(decoded.buttons.is_empty());
    assert_eq!(decoded, quiet);

    let go = ScriptPaint {
        title: Some("t".into()),
        accent: None,
        lines: vec!["line".into()],
        buttons: vec![ScriptPaintButton {
            id: "gobank".into(),
            label: "Go bank".into(),
        }],
        generation: 0,
    };
    let resume = ScriptPaint {
        title: Some("t".into()),
        accent: None,
        lines: vec!["line".into()],
        buttons: vec![ScriptPaintButton {
            id: "gobank".into(),
            label: "Resume".into(),
        }],
        generation: 0,
    };
    assert_ne!(go, resume, "label toggle must be a real change");
    let round = decode_paint(&IsolateBuf::new().encode_paint(&go)).unwrap();
    assert_eq!(round, go);
}

#[test]
fn stepper_still_throws_not_impl() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        try { p.stepper(); } catch (e) { globalThis.__err = String(e.message || e); }
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    let err = iso.probe("__err").unwrap();
    assert!(
        err.as_str()
            .unwrap_or("")
            .contains("not impl: Paint.stepper"),
        "unused widgets stay missing, got {err:?}"
    );
    iso.join();
}

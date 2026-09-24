//! G1-chrome: persistent strip/rail/tabs selection, source-derived rowsLeft,
//! advertised chrome on ScriptPaint, and statGrid recording. No jive module.

use script::isolate_fb::{ReachViewInput, SnapshotInput, TileInput};
use script::shim::PaintChromeBand;
use script::{LoadIsolate, LoadShape};

mod common;

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
        stats: &common::FRESH_STATS,
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

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

const CHROME: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox', accent: '#e05be0' });
        const page = p.strip('k', ['Statistics', 'Options'], 'ok', 'JiveCrafting');
        const section = page === 'Statistics' ? p.rail('k', ['Overview', 'Supplies']) : '';
        p.footer('Jive scripts');
        globalThis.__page = page;
        globalThis.__section = section;
        globalThis.__rowsAfterStrip = p.rowsLeft();
        if (page === 'Statistics' && section === 'Overview') {
            p.statGrid([['Runtime: 1m', 'Made: 0']], 2);
        } else if (page === 'Options') {
            p.statGrid([['Product: ring']], 2);
        } else if (section === 'Supplies') {
            p.statGrid([['Bars: 0', 'Mould: 0']], 2);
        }
        p.end();
    }
}
"#;

#[test]
fn chrome_widgets_do_not_throw() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        try {
            globalThis.__page = p.strip('k', ['Statistics', 'Options'], 'ok', 'Brand');
            globalThis.__section = p.rail('k', ['Overview']);
            p.footer('Jive scripts');
            globalThis.__left = p.rowsLeft();
            p.statGrid([['A', 'B']], 2);
            p.end();
            globalThis.__err = null;
        } catch (e) {
            globalThis.__err = String(e.message || e);
        }
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    let err = iso.probe("__err").unwrap();
    assert_eq!(
        err,
        serde_json::Value::Null,
        "chrome must not throw: {err:?}"
    );
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Statistics"));
    iso.join();
}

#[test]
fn rows_left_after_strip_and_body_rows() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.strip('k', ['Statistics'], '', 'S');
        globalThis.__afterStrip = p.rowsLeft();
        p.row('a');
        p.row('b');
        globalThis.__afterTwo = p.rowsLeft();
        p.row('c');
        globalThis.__afterThree = p.rowsLeft();
        globalThis.__room = Math.max(0, Math.floor((globalThis.__afterThree - 2) / 2));
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(iso.probe("__afterStrip").unwrap(), 8);
    assert_eq!(iso.probe("__afterTwo").unwrap(), 6);
    assert_eq!(iso.probe("__afterThree").unwrap(), 5);
    assert_eq!(
        iso.probe("__room").unwrap(),
        1,
        "rowsLeft 5 and reserve 2 still only room for one two-row skill"
    );
    iso.join();
}

#[test]
fn gap_default_is_six_px_not_a_full_line() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.strip('k', ['Statistics'], '', 'S');
        p.gap();
        p.gap();
        globalThis.__afterTwoGaps = p.rowsLeft();
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__afterTwoGaps").unwrap(),
        7,
        "two 6px gaps stay inside the same leftover line as strip-only (8) minus one"
    );
    let paint = iso.paint().expect("forwarded");
    assert_eq!(
        paint.lines.iter().filter(|l| l.is_empty()).count(),
        2,
        "existing gap still records empty body lines"
    );
    iso.join();
}

#[test]
fn tabs_consume_tab_h_plus_inset() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.tabs('mg', ['Overview', 'Loot']);
        globalThis.__afterTabs = p.rowsLeft();
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    // cursorY += 3 + TAB_H + 2 = 23; floor((150-23)/16) = 7
    assert_eq!(iso.probe("__afterTabs").unwrap(), 7);
    iso.join();
}

#[test]
fn default_strip_and_stored_page_rail_tab_selection() {
    let iso = spawn(CHROME);
    tick(&iso, 1);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Statistics"));
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some("Overview"));
    let first = iso.paint().expect("default frame");
    assert_eq!(
        first.strip.as_ref().map(|b| b.selected.as_str()),
        Some("Statistics")
    );
    assert_eq!(
        first.rail.as_ref().map(|b| b.selected.as_str()),
        Some("Overview")
    );
    assert_eq!(first.footer.as_deref(), Some("Jive scripts"));
    assert!(first.lines.iter().any(|l| l.contains("Runtime")));
    assert!(first.lines.iter().any(|l| l.contains("Made")));
    assert_eq!(first.title.as_deref(), Some("JiveCrafting"));

    iso.paint_select("strip:k", "Options");
    tick(&iso, 2);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Options"));
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some(""));
    let options = iso.paint().expect("options frame");
    assert_eq!(
        options.strip.as_ref().map(|b| b.selected.as_str()),
        Some("Options")
    );
    assert!(
        options.rail.is_none(),
        "rail is not consumed on later pages: {:?}",
        options.rail
    );
    assert!(options.lines.iter().any(|l| l.contains("Product:")));
    assert!(!options.lines.iter().any(|l| l.contains("Made")));

    iso.paint_select("strip:k", "Statistics");
    iso.paint_select("rail:k", "Supplies");
    tick(&iso, 3);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Statistics"));
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some("Supplies"));
    let supplies = iso.paint().expect("supplies frame");
    assert!(supplies.lines.iter().any(|l| l.contains("Bars:")));
    assert!(supplies.lines.iter().any(|l| l.contains("Mould:")));
    assert!(!supplies.lines.iter().any(|l| l.contains("Made")));

    iso.paint_select("strip:k", "Nope");
    tick(&iso, 4);
    assert_eq!(
        iso.probe("__page").unwrap().as_str(),
        Some("Statistics"),
        "stale advertised name falls back to first"
    );
    iso.join();
}

#[test]
fn footer_is_not_a_body_line() {
    let iso = spawn(CHROME);
    tick(&iso, 1);
    let paint = iso.paint().expect("frame");
    assert_eq!(paint.footer.as_deref(), Some("Jive scripts"));
    assert!(
        !paint.lines.iter().any(|l| l == "Jive scripts"),
        "footer must not steal a rowsLeft line: {:?}",
        paint.lines
    );
    iso.join();
}

#[test]
fn tabs_store_is_shared_not_jive_only() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        p.title('MossGiant');
        globalThis.__tab = p.tabs('mg', ['Overview', 'Loot']);
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(iso.probe("__tab").unwrap().as_str(), Some("Overview"));
    iso.paint_select("tabs:mg", "Loot");
    tick(&iso, 2);
    assert_eq!(iso.probe("__tab").unwrap().as_str(), Some("Loot"));
    let paint = iso.paint().expect("tabs advertised");
    assert_eq!(
        paint.tabs,
        vec![PaintChromeBand {
            id: "mg".into(),
            names: vec!["Overview".into(), "Loot".into()],
            selected: "Loot".into(),
            status: None,
            brand: None,
        }]
    );
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__tab").unwrap().as_str(),
        Some("Loot"),
        "selection persists across paints"
    );
    iso.join();
}

#[test]
fn stat_grid_respects_columns() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.strip('k', ['Statistics'], '', 'S');
        p.statGrid([['A', 'B', 'C']], 2);
        p.statGrid([['X', 'Y', 'Z']], 1);
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    let paint = iso.paint().expect("statGrid");
    assert!(
        paint.lines.iter().any(|l| l == "A | B"),
        "columns=2 keeps first two cells: {:?}",
        paint.lines
    );
    assert!(
        !paint.lines.iter().any(|l| l.contains("C")),
        "extra cell beyond columns is dropped: {:?}",
        paint.lines
    );
    assert!(
        paint.lines.iter().any(|l| l == "X"),
        "columns=1 keeps first cell only: {:?}",
        paint.lines
    );
    iso.join();
}

#[test]
fn reset_clears_store_pause_and_stale_generation_reject() {
    let iso = spawn(CHROME);
    tick(&iso, 1);
    iso.paint_select("strip:k", "Options");
    tick(&iso, 2);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Options"));

    iso.reset_session_work();
    let mut snap = base_snapshot();
    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    assert_eq!(
        iso.probe("__page").unwrap().as_str(),
        Some("Statistics"),
        "work reset must not inherit a stored page"
    );

    iso.pause();
    iso.paint_select("strip:k", "Options");
    iso.resume();
    tick(&iso, 4);
    assert_eq!(
        iso.probe("__page").unwrap().as_str(),
        Some("Statistics"),
        "paused paint_select must not apply"
    );

    iso.paint_select("strip:k", "Options");
    iso.reset_session_work();
    snap.tick = 5;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 5);
    assert_eq!(
        iso.probe("__page").unwrap().as_str(),
        Some("Statistics"),
        "stale-generation select queued before reset must not apply"
    );
    iso.join();
}

#[test]
fn empty_select_is_ignored() {
    let iso = spawn(CHROME);
    tick(&iso, 1);
    iso.paint_select("strip:k", "");
    tick(&iso, 2);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Statistics"));
    iso.join();
}

#[test]
fn stepper_still_not_impl() {
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

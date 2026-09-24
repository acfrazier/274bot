//! G1-jive: Rust-owned XpTracker, levelRow, paintLevels, and frame defaults.
//! Selection/capacity stay on accepted chrome; this hop marshals only.

use script::isolate_fb::{encode_snapshot, ReachViewInput, SnapshotInput, StatInput, TileInput};
use script::{LoadIsolate, LoadShape};

mod common;

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3269,
            z: 3167,
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

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn crafting(xp: i32, base: i32) -> StatInput<'static> {
    StatInput {
        index: 12,
        name: "crafting",
        xp,
        base,
        effective: base,
    }
}

/// Every used stat loaded (so a compat card may paint), crafting as given.
fn loaded_with_crafting(xp: i32, base: i32) -> Vec<StatInput<'static>> {
    common::FRESH_STATS
        .iter()
        .map(|row| {
            if row.name == "crafting" {
                crafting(xp, base)
            } else {
                *row
            }
        })
        .collect()
}

const TRACKER: &str = r#"
import { XpTracker } from '../../paint/jive.js';
import { Skills } from '../../api/skills/Skills.js';
export default class T extends LoopingBot {
    constructor() {
        super();
        this.xp = new XpTracker(['crafting', 'magic'], Skills);
    }
    onStart() {
        if (globalThis.__doBegin) this.xp.begin();
    }
    loop() {
        globalThis.__progress = this.xp.progress();
        globalThis.__gains = this.xp.gains();
    }
}
"#;

#[test]
fn jive_import_specifier_is_not_unloadable() {
    let src = "import { jiveFrame, XpTracker, paintLevels } from '../../paint/jive.js';\n";
    assert_eq!(
        script::first_unloadable_specifier(src),
        None,
        "jive.js must remap"
    );
}

#[test]
fn combat_skills_are_the_seven_fight_names() {
    let src = r#"
import { COMBAT_SKILLS } from '../../paint/jive.js';
export default class T extends LoopingBot {
    onStart() { globalThis.__skills = COMBAT_SKILLS.slice(); }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__skills").unwrap(),
        serde_json::json!([
            "attack",
            "strength",
            "defence",
            "hitpoints",
            "ranged",
            "magic",
            "prayer"
        ])
    );
    iso.join();
}

#[test]
fn xp_tracker_zero_delta_negative_no_begin_and_gained_order() {
    let iso = spawn(TRACKER);
    let mut snap = base_snapshot();
    let stats0 = [
        crafting(0, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 1000,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats0;
    iso.probe("globalThis.__doBegin = true").unwrap();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__gains").unwrap(), serde_json::json!([]));
    let progress = iso.probe("__progress").unwrap();
    assert_eq!(progress[0]["skill"], "crafting");
    assert_eq!(progress[0]["gained"], 0);
    assert_eq!(progress[1]["skill"], "magic");
    assert_eq!(progress[1]["gained"], 0);

    let stats1 = [
        crafting(200, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 500,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats1;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    let progress = iso.probe("__progress").unwrap();
    assert_eq!(progress[0]["gained"], 200);
    assert_eq!(
        progress[1]["gained"], -500,
        "xp drop stays on progress as a negative delta"
    );
    let gains = iso.probe("__gains").unwrap();
    assert_eq!(gains.as_array().map(|a| a.len()), Some(1));
    assert_eq!(gains[0]["skill"], "crafting");
    assert_eq!(gains[0]["gained"], 200);

    let stats2 = [
        crafting(200, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 1800,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats2;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    let gains = iso.probe("__gains").unwrap();
    assert_eq!(
        gains.as_array().map(|rows| rows
            .iter()
            .map(|r| (
                r["skill"].as_str().unwrap_or("").to_string(),
                r["gained"].as_i64().unwrap_or(0)
            ))
            .collect::<Vec<_>>()),
        Some(vec![("magic".into(), 800), ("crafting".into(), 200)]),
        "gains filters moved skills and sorts biggest first"
    );
    iso.join();
}

#[test]
fn xp_tracker_no_begin_counts_nothing() {
    let iso = spawn(TRACKER);
    let mut snap = base_snapshot();
    let stats = [
        crafting(200, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 500,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats;
    iso.probe("globalThis.__doBegin = false").unwrap();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let progress = iso.probe("__progress").unwrap();
    assert_eq!(progress[0]["gained"], 0, "missing begin uses unwrap_or(xp)");
    assert_eq!(iso.probe("__gains").unwrap(), serde_json::json!([]));
    iso.join();
}

#[test]
fn xp_tracker_reset_and_isolate_isolation() {
    let left = spawn(TRACKER);
    let right = spawn(TRACKER);
    let mut snap = base_snapshot();
    let stats0 = [
        crafting(0, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 0,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats0;
    left.probe("globalThis.__doBegin = true").unwrap();
    right.probe("globalThis.__doBegin = true").unwrap();
    post_snapshot_input(&left, &snap);
    post_snapshot_input(&right, &snap);
    tick(&left, 1);
    tick(&right, 1);

    let stats1 = [
        crafting(200, 20),
        StatInput {
            index: 6,
            name: "magic",
            xp: 0,
            base: 15,
            effective: 15,
        },
    ];
    snap.stats = &stats1;
    post_snapshot_input(&left, &snap);
    post_snapshot_input(&right, &snap);
    tick(&left, 2);
    tick(&right, 2);
    assert_eq!(left.probe("__progress").unwrap()[0]["gained"], 200);
    assert_eq!(right.probe("__progress").unwrap()[0]["gained"], 200);

    left.reset_session_work();
    let _ = left.probe("true");
    left.probe("globalThis.__doBegin = false").unwrap();
    tick(&left, 3);
    assert_eq!(
        left.probe("__progress").unwrap()[0]["gained"],
        0,
        "reset clears baselines so a later script cannot inherit them"
    );
    assert_eq!(
        right.probe("__progress").unwrap()[0]["gained"],
        200,
        "other isolate keeps its own baselines"
    );
    left.join();
    right.join();
}

#[test]
fn script_frame_and_jive_frame_defaults() {
    let src = r#"
import { scriptFrame, jiveFrame, JIVE_ACCENT, JIVE_BYLINE } from '../../paint/jive.js';
export default class T extends LoopingBot {
    onPaint() {
        const a = scriptFrame(null, {
            script: 'GatheringBot',
            status: 'ok',
            pages: ['Statistics', 'Options'],
            sections: ['Overview', 'Supplies']
        });
        a.frame.end();
        const first = globalThis.__rs2b0t_host.paint;
        globalThis.__script = {
            page: a.page,
            section: a.section,
            accent: JIVE_ACCENT,
            byline: JIVE_BYLINE,
            key: first && first.strip && first.strip.id,
            footer: first && first.footer,
        };
        const b = jiveFrame(null, {
            script: 'JiveCrafting',
            status: 'starting',
            pages: ['Statistics', 'Options'],
            sections: ['Overview', 'Supplies']
        });
        globalThis.__jive = { page: b.page, section: b.section };
        b.frame.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(iso.probe("__script").unwrap()["page"], "Statistics");
    assert_eq!(iso.probe("__script").unwrap()["section"], "Overview");
    assert_eq!(iso.probe("__script").unwrap()["accent"], "#e05be0");
    assert_eq!(iso.probe("__script").unwrap()["byline"], "Jive scripts");
    assert_eq!(iso.probe("__script").unwrap()["key"], "GatheringBot");
    assert_eq!(iso.probe("__script").unwrap()["footer"], "Jive scripts");
    assert_eq!(iso.probe("__jive").unwrap()["page"], "Statistics");
    assert_eq!(iso.probe("__jive").unwrap()["section"], "Overview");
    let jive_paint = iso.paint().expect("jiveFrame overwrote last paint");
    assert_eq!(
        jive_paint.strip.as_ref().map(|b| b.id.as_str()),
        Some("jive:JiveCrafting")
    );
    assert_eq!(jive_paint.accent.as_deref(), Some("#e05be0"));
    iso.join();
}

const CRAFT_GOLD: &str = r#"
import { XpTracker, jiveFrame, paintLevels } from '../../paint/jive.js';
import { Skills } from '../../api/skills/Skills.js';
export default class T extends LoopingBot {
    constructor() {
        super();
        this.xp = new XpTracker(['crafting'], Skills);
        this.startedAt = Date.now();
    }
    onStart() { this.xp.begin(); }
    onPaint() {
        const { frame: p, page, section } = jiveFrame(null, {
            script: 'JiveCrafting',
            status: 'starting',
            pages: ['Statistics', 'Options'],
            sections: ['Overview', 'Supplies']
        });
        const mins = (Date.now() - this.startedAt) / 60_000;
        globalThis.__page = page;
        globalThis.__section = section;
        if (page === 'Options') {
            p.statGrid([[{ text: 'Product: Sapphire ring' }, { text: 'Needs: Crafting 20' }]]);
        } else if (section === 'Overview') {
            p.statGrid([
                [{ text: 'Runtime: 0s' }, { text: 'Made: 0' }],
                [{ text: 'Trips: 0' }, { text: 'Banked: 0' }]
            ]);
            p.bar('Pack', 0);
            paintLevels(p, this.xp.progress(), mins, 2);
        } else {
            p.statGrid([[{ text: 'Bars: 0' }, { text: 'Mould: missing' }]]);
        }
        p.end();
    }
}
"#;

#[test]
fn jive_crafting_chrome_selection_changes_recorded_lines() {
    let iso = spawn(CRAFT_GOLD);
    let mut snap = base_snapshot();
    let stats0 = loaded_with_crafting(0, 40);
    snap.stats = &stats0;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Statistics"));
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some("Overview"));
    let overview = iso.paint().expect("overview");
    assert!(overview.lines.iter().any(|l| l.contains("Runtime")));
    assert!(overview.lines.iter().any(|l| l.contains("Made")));
    assert!(
        overview.lines.iter().any(|l| l.contains("Craft")),
        "one level bar from progress(): {:?}",
        overview.lines
    );

    let stats1 = loaded_with_crafting(200, 40);
    snap.stats = &stats1;
    post_snapshot_input(&iso, &snap);
    iso.paint_select("strip:jive:JiveCrafting", "Options");
    tick(&iso, 2);
    assert_eq!(iso.probe("__page").unwrap().as_str(), Some("Options"));
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some(""));
    let options = iso.paint().expect("options");
    assert!(options.lines.iter().any(|l| l.contains("Product:")));
    assert!(!options.lines.iter().any(|l| l.contains("Made:")));
    assert!(
        options.rail.is_none(),
        "Options must not consume rail: {:?}",
        options.rail
    );

    iso.paint_select("strip:jive:JiveCrafting", "Statistics");
    iso.paint_select("rail:jive:JiveCrafting", "Supplies");
    tick(&iso, 3);
    assert_eq!(iso.probe("__section").unwrap().as_str(), Some("Supplies"));
    let supplies = iso.paint().expect("supplies");
    assert!(supplies.lines.iter().any(|l| l.contains("Bars:")));
    assert!(supplies.lines.iter().any(|l| l.contains("Mould:")));
    assert!(!supplies.lines.iter().any(|l| l.contains("Made:")));
    iso.join();
}

#[test]
fn paint_levels_respects_finite_room_and_empty() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
import { paintLevels, COMBAT_SKILLS } from '../../paint/jive.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.strip('k', ['Statistics'], '', 'Jive');
        p.statGrid([['Runtime: 1m', 'Made: 0']], 2);
        p.statGrid([['Trips: 0', 'Banked: 0']], 2);
        p.bar('Pack', 0);
        globalThis.__left = p.rowsLeft();
        const gains = COMBAT_SKILLS.map((skill, i) => ({
            skill, level: 70, xp: 737627, gained: 7000 - i
        }));
        paintLevels(p, gains, 10, 2);
        globalThis.__after = p.rowsLeft();
        p.end();
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    assert_eq!(
        iso.probe("__left").unwrap(),
        5,
        "two statGrid rows + pack bar after strip leaves 5"
    );
    let paint = iso.paint().expect("levels");
    let bars: Vec<_> = paint
        .lines
        .iter()
        .filter(|l| l.contains("70:") || l.contains(" 70:"))
        .cloned()
        .collect();
    assert_eq!(
        bars.len(),
        1,
        "room floor((5-2)/2)=1, not seven combat skills: {:?}",
        paint.lines
    );
    assert!(
        bars[0].contains("Att"),
        "constructor/gained order keeps first fitting skill: {:?}",
        bars
    );
    iso.join();

    let empty = r#"
import { jiveFrame, paintLevels } from '../../paint/jive.js';
export default class T extends LoopingBot {
    onPaint() {
        const { frame: p } = jiveFrame(null, {
            script: 'JiveDragons',
            status: '',
            pages: ['Statistics'],
            sections: ['Overview']
        });
        paintLevels(p, [], 10, 2);
        p.end();
    }
}
"#;
    let iso = spawn(empty);
    tick(&iso, 1);
    let paint = iso.paint().expect("empty");
    assert!(
        paint.lines.iter().any(|l| l == "no experience yet"),
        "empty gains records the default text: {:?}",
        paint.lines
    );
    iso.join();
}

#[test]
fn level_row_source_formats_curve_and_99_cap() {
    let src = r#"
import { levelRow } from '../../paint/levelProgress.js';
export default class T extends LoopingBot {
    onStart() {
        globalThis.__low = levelRow({ skill: 'crafting', level: 1, xp: 0, gained: 0 }, 0.4);
        globalThis.__rate = levelRow({ skill: 'magic', level: 1, xp: 0, gained: 4000 }, 10);
        globalThis.__max = levelRow({ skill: 'crafting', level: 99, xp: 13034431, gained: 1000 }, 10);
        globalThis.__combat = ['attack','strength','defence','hitpoints','ranged','magic','prayer']
            .map(skill => levelRow({ skill, level: 70, xp: 737627, gained: 1000 }, 10).label);
        globalThis.__gather = ['woodcutting','fishing','mining'].map(skill =>
            levelRow({ skill, level: 10, xp: 1154, gained: 100 }, 10).label);
    }
}
"#;
    let iso = spawn(src);
    tick(&iso, 1);
    let low = iso.probe("__low").unwrap();
    assert_eq!(low["label"], "Craft 1");
    assert_eq!(low["cells"][0], "xp/hr n/a");
    assert_eq!(low["cells"][1], "83 to go");
    assert_eq!(low["cells"][2], "eta n/a");
    assert!(low["fraction"].as_f64().unwrap() < 0.01);

    let rate = iso.probe("__rate").unwrap();
    assert_eq!(rate["label"], "Mage 1");
    assert_eq!(rate["cells"][0], "24.0k/hr");
    assert_eq!(rate["cells"][1], "83 to go");
    assert_eq!(rate["cells"][2], "eta 0:00:12");

    let maxed = iso.probe("__max").unwrap();
    assert_eq!(maxed["label"], "Craft 99");
    assert_eq!(maxed["fraction"], 1.0);
    assert_eq!(maxed["cells"][0], "6.0k/hr");
    assert_eq!(maxed["cells"][1], "maxed");
    assert_eq!(maxed["cells"][2], "");

    assert_eq!(
        iso.probe("__combat").unwrap(),
        serde_json::json!([
            "Att 70", "Str 70", "Def 70", "HP 70", "Range 70", "Mage 70", "Pray 70"
        ])
    );
    assert_eq!(
        iso.probe("__gather").unwrap(),
        serde_json::json!(["WC 10", "Fish 10", "Mine 10"])
    );
    iso.join();
}

//! Paint pipeline identity: the recorded `ScriptPaint` a script produces.
//!
//! The canvas op tape, the batched chrome rows, the frame plan and the shared
//! frame channel must not change what a script records. Each case below pins
//! the whole frame (title, lines, buttons, chrome, footer, tabs and every
//! recorded canvas op) as the pipeline produced it before the rework, so any
//! drift shows up here.

use std::path::PathBuf;

use script::shim::ScriptPaint;
use script::LoadIsolate;

fn spawn(src: &str) -> LoadIsolate {
    let shape = script::load::detect_shape(src);
    LoadIsolate::spawn(src.to_string(), shape, vec![]).expect("bot loads")
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("0");
}

/// One pass, read back as the frame the host shares.
fn pass(iso: &LoadIsolate, n: u64) -> ScriptPaint {
    tick(iso, n);
    iso.paint().expect("paint forwarded").as_ref().clone()
}

/// The whole frame as text: every field of every op, in order.
fn digest(paint: &ScriptPaint) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "title={:?} accent={:?}\n",
        paint.title, paint.accent
    ));
    for line in &paint.lines {
        out.push_str(&format!("line={line}\n"));
    }
    for button in &paint.buttons {
        out.push_str(&format!("button={}:{}\n", button.id, button.label));
    }
    out.push_str(&format!(
        "strip={:?}\nrail={:?}\nfooter={:?}\ntabs={:?}\n",
        paint.strip, paint.rail, paint.footer, paint.tabs
    ));
    for op in &paint.canvas {
        out.push_str(&format!("op={op:?}\n"));
    }
    out
}

fn examplebot_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ExampleBot.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let pin = script::js_cache::JsCache::origin_sha(&bytes);
    assert_eq!(
        pin, "8e8e27cd9c2e57cd6478132e4bfaf910678664898db44915acc08ffc501c0973",
        "tracked ExampleBot fixture drifted from rs2b0t 96410ec5 script-template"
    );
    let src = String::from_utf8(bytes).expect("ExampleBot utf-8");
    script::transpile_ts(&src).expect("transpile ExampleBot")
}

/// The pinned template's own `onPaint`: two style sets, a measure, a fillRect
/// and a fillText.
const EXAMPLEBOT_FRAME: &str = r##"title=None accent=None
strip=None
rail=None
footer=None
tabs=[]
op=FillRect { x: 6, y: 6, w: 209, h: 24, color: 153, extras: DrawExtras { clips: [], shadow: Shadow { color: 0, blur: 0.0, offset_x: 0.0, offset_y: 0.0 }, fill: Solid } }
op=FillText { text: "BoneBurier (external)  buried 0", x: 12, y: 22, color: 4289813503, font_px: 12, mono: true, align: Left, baseline: Alphabetic, extras: DrawExtras { clips: [], shadow: Shadow { color: 0, blur: 0.0, offset_x: 0.0, offset_y: 0.0 }, fill: Solid } }
"##;

#[test]
fn examplebot_frame_is_unchanged() {
    let iso = spawn(&examplebot_source());
    let paint = pass(&iso, 1);
    let got = digest(&paint);
    println!("{got}");
    assert_eq!(got, EXAMPLEBOT_FRAME, "ExampleBot recorded frame drifted");
    iso.join();
}

/// Every ctx member the declared surface has: style sets and gets, save,
/// restore, paths, an arc, a clip, a gradient and text.
const FULL_CTX: &str = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.font = '12px monospace';
        globalThis.__font = ctx.font;
        globalThis.__width = ctx.measureText('hi').width;
        ctx.fillStyle = '#101010';
        globalThis.__fill = ctx.fillStyle;
        ctx.fillRect(1, 2, 3, 4);
        ctx.save();
        ctx.fillStyle = 'rgba(0, 0, 0, 0.6)';
        ctx.shadowColor = '#ff0000';
        ctx.shadowBlur = 4;
        ctx.shadowOffsetX = 2;
        ctx.shadowOffsetY = -1;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        globalThis.__align = ctx.textAlign;
        ctx.fillText('hi', 10, 20);
        ctx.lineWidth = 3;
        globalThis.__lineWidth = ctx.lineWidth;
        ctx.lineJoin = 'round';
        ctx.strokeStyle = '#00ff00';
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.lineTo(10, 0);
        ctx.quadraticCurveTo(12, 2, 14, 0);
        ctx.closePath();
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(20, 20, 5, 0, 3.14159, true);
        ctx.clip();
        ctx.fillStyle = '#eeeeee';
        ctx.fill();
        ctx.restore();
        globalThis.__restored = ctx.fillStyle;
        globalThis.__restoredWidth = ctx.lineWidth;
        const g = ctx.createLinearGradient(0, 0, 10, 0);
        g.addColorStop(0, '#ff0000');
        g.addColorStop(1, '#0000ff');
        globalThis.__gradBack = false;
        ctx.fillStyle = g;
        globalThis.__gradBack = ctx.fillStyle === g;
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.lineTo(5, 5);
        ctx.lineTo(0, 5);
        ctx.closePath();
        ctx.fill();
        globalThis.__negThrew = false;
        try { ctx.arc(0, 0, -1, 0, 1); } catch (e) { globalThis.__negThrew = String(e.message).indexOf('IndexSizeError') >= 0; }
        ctx.fillText('after', 1, 2);
    }
}
"#;

const FULL_CTX_FRAME: &str = r##"title=None accent=None
strip=None
rail=None
footer=None
tabs=[]
op=FillRect { x: 1, y: 2, w: 3, h: 4, color: 269488383, extras: DrawExtras { clips: [], shadow: Shadow { color: 0, blur: 0.0, offset_x: 0.0, offset_y: 0.0 }, fill: Solid } }
op=FillText { text: "hi", x: 10, y: 20, color: 153, font_px: 12, mono: true, align: Center, baseline: Middle, extras: DrawExtras { clips: [], shadow: Shadow { color: 4278190335, blur: 4.0, offset_x: 2.0, offset_y: -1.0 }, fill: Solid } }
op=StrokePath { segs: [MoveTo { x: 0.0, y: 0.0 }, LineTo { x: 10.0, y: 0.0 }, QuadTo { cx: 12.0, cy: 2.0, x: 14.0, y: 0.0 }, Close], color: 16711935, line_width: 3.0, line_join: Round, extras: DrawExtras { clips: [], shadow: Shadow { color: 4278190335, blur: 4.0, offset_x: 2.0, offset_y: -1.0 }, fill: Solid } }
op=FillPath { segs: [MoveTo { x: 25.0, y: 20.0 }, CubicTo { c1x: 25.0, c1y: 17.238573, c2x: 22.76142, c2y: 14.999996, x: 19.999992, y: 15.0 }, CubicTo { c1x: 17.238565, c1y: 15.000004, c2x: 14.999992, c2y: 17.238586, x: 15.0, y: 20.000013 }], color: 4008636159, extras: DrawExtras { clips: [ClipPath { segs: [MoveTo { x: 25.0, y: 20.0 }, CubicTo { c1x: 25.0, c1y: 17.238573, c2x: 22.76142, c2y: 14.999996, x: 19.999992, y: 15.0 }, CubicTo { c1x: 17.238565, c1y: 15.000004, c2x: 14.999992, c2y: 17.238586, x: 15.0, y: 20.000013 }] }], shadow: Shadow { color: 4278190335, blur: 4.0, offset_x: 2.0, offset_y: -1.0 }, fill: Solid } }
op=FillPath { segs: [MoveTo { x: 0.0, y: 0.0 }, LineTo { x: 5.0, y: 5.0 }, LineTo { x: 0.0, y: 5.0 }, Close], color: 0, extras: DrawExtras { clips: [], shadow: Shadow { color: 0, blur: 0.0, offset_x: 0.0, offset_y: 0.0 }, fill: Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0, stops: [GradStop { offset: 0.0, color: 4278190335 }, GradStop { offset: 1.0, color: 65535 }] } } }
op=FillText { text: "after", x: 1, y: 2, color: 0, font_px: 12, mono: true, align: Left, baseline: Alphabetic, extras: DrawExtras { clips: [], shadow: Shadow { color: 0, blur: 0.0, offset_x: 0.0, offset_y: 0.0 }, fill: Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0, stops: [GradStop { offset: 0.0, color: 4278190335 }, GradStop { offset: 1.0, color: 65535 }] } } }
"##;

#[test]
fn full_ctx_frame_is_unchanged() {
    let iso = spawn(FULL_CTX);
    let paint = pass(&iso, 1);
    let got = digest(&paint);
    println!("{got}");
    assert_eq!(got, FULL_CTX_FRAME, "recorded canvas ops drifted");
    // The ctx's own style state answers the getters.
    assert_eq!(iso.probe("__font").unwrap(), "12px monospace");
    assert_eq!(iso.probe("__fill").unwrap(), "#101010");
    assert_eq!(iso.probe("__align").unwrap(), "center");
    assert_eq!(iso.probe("__lineWidth").unwrap(), 3);
    assert_eq!(iso.probe("__restored").unwrap(), "#101010");
    assert_eq!(iso.probe("__restoredWidth").unwrap(), 1);
    assert_eq!(iso.probe("__gradBack").unwrap(), true);
    assert_eq!(iso.probe("__negThrew").unwrap(), true);
    assert!(
        iso.probe("__width").unwrap().as_f64().unwrap() > 7.0,
        "measureText uses the set font"
    );
    iso.join();
}

/// The recorded widget frame: rows, gaps, chrome bands, buttons and the dock
/// budget, with the Jive frame plan.
const WIDGETS: &str = r#"
import { Paint } from '../../paint/Paint.js';
import { jiveFrame } from '../../paint/jive.js';

export default class T extends LoopingBot {
    onPaint() {
        const { frame: p, page, section } = jiveFrame(null, {
            script: 'JiveCrafting',
            status: 'starting',
            pages: ['Statistics', 'Options'],
            sections: ['Overview', 'Supplies']
        });
        globalThis.__page = page;
        globalThis.__section = section;
        p.title('JiveCrafting');
        p.row('Runtime: 0s', 'Made: 0');
        p.text('plain');
        p.bar('Pack', 0.25);
        p.gap();
        p.cells([{ text: 'Bars: 0' }, 'Mould: missing']);
        globalThis.__left = p.rowsLeft();
        globalThis.__clicked = p.buttons([
            { id: 'gobank', label: 'Go bank' },
            { id: 'stop', label: 'Stop' }
        ]);
        p.tabs('pages', ['Statistics', 'Options']);
        p.footer('done');
        p.end();
    }
}
"#;

const WIDGETS_FRAME: &str = r##"title=Some("JiveCrafting") accent=Some("#e05be0")
line=Runtime: 0s | Made: 0
line=plain
line=Pack: 25%
line=
line=Bars: 0 | Mould: missing
button=gobank:Go bank
button=stop:Stop
strip=Some(PaintChromeBand { id: "jive:JiveCrafting", names: ["Statistics", "Options"], selected: "Statistics", status: Some("starting"), brand: Some("JiveCrafting") })
rail=Some(PaintChromeBand { id: "jive:JiveCrafting", names: ["Overview", "Supplies"], selected: "Overview", status: None, brand: None })
footer=Some("done")
tabs=[PaintChromeBand { id: "pages", names: ["Statistics", "Options"], selected: "Statistics", status: None, brand: None }]
"##;

#[test]
fn widget_frame_is_unchanged() {
    let iso = spawn(WIDGETS);
    let paint = pass(&iso, 1);
    let got = digest(&paint);
    println!("{got}");
    assert_eq!(got, WIDGETS_FRAME, "recorded widget frame drifted");
    assert_eq!(iso.probe("__page").unwrap(), "Statistics");
    assert_eq!(iso.probe("__section").unwrap(), "Overview");
    assert_eq!(iso.probe("__clicked").unwrap(), serde_json::Value::Null);
    // 150px dock: strip 20 + title 20 + gap 6 + four 16px rows is 110 spent,
    // so 2 rows are left when the read lands (tabs come after it).
    assert_eq!(iso.probe("__left").unwrap(), 2, "{got}");
    iso.join();
}

/// `paintLevels` rows come from the Rust helper: room, order and formats.
const LEVELS: &str = r#"
import { Paint } from '../../paint/Paint.js';
import { paintLevels } from '../../paint/jive.js';

export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { dock: 'chatbox' });
        p.strip('k', ['Statistics'], '', 'Jive');
        p.statGrid([['Runtime: 1m', 'Made: 0']], 2);
        p.bar('Pack', 0);
        globalThis.__left = p.rowsLeft();
        const gains = [
            { skill: 'crafting', level: 1, xp: 0, gained: 0 },
            { skill: 'magic', level: 1, xp: 0, gained: 4000 },
            { skill: 'attack', level: 70, xp: 737627, gained: 7000 }
        ];
        paintLevels(p, gains, 10, 2);
        paintLevels(p, [], 10, 2);
        p.end();
    }
}
"#;

const LEVELS_FRAME: &str = r##"title=Some("Jive") accent=None
line=Runtime: 1m | Made: 0
line=Pack: 0%
line=Craft 1: 0%
line=0.0k/hr | 83 to go | eta n/a
line=Mage 1: 0%
line=24.0k/hr | 83 to go | eta 0:00:12
line=no experience yet
strip=Some(PaintChromeBand { id: "k", names: ["Statistics"], selected: "Statistics", status: Some(""), brand: Some("Jive") })
rail=None
footer=None
tabs=[]
"##;

#[test]
fn level_rows_are_unchanged() {
    let iso = spawn(LEVELS);
    let paint = pass(&iso, 1);
    let got = digest(&paint);
    println!("{got}");
    assert_eq!(got, LEVELS_FRAME, "recorded level rows drifted");
    // strip 20 + one statGrid row 16 + the pack bar 16 leaves 6 rows, so
    // paintLevels' floor((6-2)/2) = 2 fits two gains and the empty call still
    // appends its text.
    assert_eq!(iso.probe("__left").unwrap(), 6);
    iso.join();
}

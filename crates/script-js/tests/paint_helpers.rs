//! Contract checks for the catalog-used paint helpers and layout methods.

use script_js::{LoadIsolate, LoadShape};

const BOT: &str = r#"
import { Paint } from '../../paint/Paint.js';
import { PaintState, clipText, gatherPaintAccent, paintClip } from '../../paint/paintLogic.js';
export default class PaintHelpers extends LoopingBot {
    onPaint() {
        const ctx = { measureText(text) { return { width: String(text).length * 5 }; } };
        const p = Paint.begin(ctx, { dock: { y: 0, w: 106, h: 80 } });
        const cols = p.cols();
        p.row('status');
        p.wrap('alpha beta gamma delta');
        const list = p.list('items', ['zero', 'one', 'two', 'three', 'four'], 2, {
            focus: 3,
            footer: 'end',
        });
        const fill = p.fill('fill', ['zero', 'one', 'two', 'three', 'four'], { focus: 4 });
        const grid = p.grid('grid', ['zero', 'one', 'two', 'three', 'four'], 2, { focus: 4 });
        p.wrap('abcdefghijklmnopqrstuvwx');
        p.list(
            'lazy',
            ['shown', { get text() { throw new Error('offscreen row must remain lazy'); } }],
            1,
            { focus: 0 },
        );
        const own = new PaintState();
        const other = new PaintState();
        own.set('remember', 'value');
        globalThis.__paintFacts = {
            cols,
            list,
            fill,
            grid,
            lines: null,
            clipped: clipText('  abcdef', 5),
            unicodeClip: clipText('éclair', 4),
            emptyClip: clipText('abc', 0),
            paintClip: paintClip('\u00a0 abcdef \u00a0', 5),
            accentFish: gatherPaintAccent('fish'),
            accentMine: gatherPaintAccent('mine'),
            accentWc: gatherPaintAccent('wc'),
            accentOther: gatherPaintAccent('other'),
            accentFallback: gatherPaintAccent('unknown'),
            state: own.get('remember', 'fallback'),
            fallback: own.get('missing', 'fallback'),
            isolated: other.get('remember', 'other'),
        };
        p.end();
        globalThis.__paintFacts.lines = globalThis.__rs2b0t_host.paint.lines;
    }
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(BOT.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn paint helper contract isolate")
}

#[test]
fn catalog_paint_helpers_match_upstream_contract() {
    let iso = spawn();
    iso.on_game_tick(1);
    let facts = iso.probe("globalThis.__paintFacts").expect("paint facts");
    assert_eq!(
        facts["cols"], 18,
        "panel width and measured monospace width"
    );
    assert_eq!(
        facts["list"], 2,
        "focus scrolls list to its last visible rows"
    );
    assert_eq!(
        facts["fill"], 4,
        "fill follows focus within the available panel rows"
    );
    assert_eq!(facts["grid"], 2, "grid focus is converted to a row index");
    assert_eq!(
        facts["lines"],
        serde_json::json!([
            "status",
            "alpha beta gamma",
            "  delta",
            "two",
            "three",
            "3 to 4 of 5 · end",
            "four",
            "5 to 5 of 5",
            "four | ",
            "5 to 5 of 5",
            "abcdefghijklmnopqr",
            "  stuvwx",
            "shown",
            "1 to 1 of 2"
        ]),
        "paint rows are clipped and counters follow upstream scroll semantics",
    );
    assert_eq!(facts["clipped"], "  ab…");
    assert_eq!(facts["unicodeClip"], "écl…");
    assert_eq!(facts["emptyClip"], "");
    assert_eq!(facts["paintClip"], "abcd…");
    assert_eq!(facts["accentFish"], "#7ec8e3");
    assert_eq!(facts["accentMine"], "#e0c36a");
    assert_eq!(facts["accentWc"], "#9be05b");
    assert_eq!(facts["accentOther"], "#9be05b");
    assert_eq!(facts["accentFallback"], "#9be05b");
    assert_eq!(facts["state"], "value");
    assert_eq!(facts["fallback"], "fallback");
    assert_eq!(facts["isolated"], "other");
    iso.join();
}

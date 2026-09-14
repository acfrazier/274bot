//! Native Canvas recorder: unchanged template ops, transitions, wire, caps.

use std::path::PathBuf;

use script::canvas::{self, CanvasOp};
use script::isolate_fb::{decode_paint, IsolateBuf};
use script::shim::ScriptPaint;
use script::LoadIsolate;

fn tick_paint(iso: &LoadIsolate, n: u64) -> ScriptPaint {
    iso.on_game_tick(n);
    let _ = iso.probe("0");
    iso.paint().expect("paint forwarded")
}

fn spawn(src: &str) -> LoadIsolate {
    let shape = script::load::detect_shape(src);
    LoadIsolate::spawn(src.to_string(), shape, vec![]).expect("bot loads")
}

fn examplebot_source() -> String {
    let campaign = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/compat/p2-external/ExampleBot.ts");
    let frozen = PathBuf::from(
        "/Users/acfrazier/experiments/274bot/.superpowers/inputs/rs2b0t-96410ec5c779f3d8fe537268cae1a21c0174d16c/docs/script-template/src/ExampleBot.ts",
    );
    let path = if campaign.is_file() { campaign } else { frozen };
    let src =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    script::transpile_ts(&src).expect("transpile ExampleBot")
}

fn is_fallback(p: &ScriptPaint) -> bool {
    p.title.as_deref() == Some("onPaint")
        && p.lines
            .iter()
            .any(|l| l.contains("Paint.end") || l.contains("no frame"))
}

fn is_error(p: &ScriptPaint) -> bool {
    p.title.as_deref() == Some("onPaint") && p.lines.iter().any(|l| l.contains("not impl:"))
}

#[test]
fn unchanged_template_forwards_canvas_ops_not_fallback() {
    let iso = spawn(&examplebot_source());
    let paint = tick_paint(&iso, 1);
    assert!(
        !is_fallback(&paint),
        "canvas-only must not plant Paint.end fallback: {paint:?}"
    );
    assert!(
        paint.canvas.len() >= 2,
        "fillRect + fillText, got {:?}",
        paint.canvas
    );
    assert!(
        paint
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillRect { x: 6, y: 6, .. })),
        "banner rect at (6,6): {:?}",
        paint.canvas
    );
    let text = paint
        .canvas
        .iter()
        .find_map(|op| match op {
            CanvasOp::FillText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .expect("fillText");
    assert!(text.contains("BoneBurier"), "got {text:?}");
    let measured = canvas::measure_with(12, true, text);
    assert!(measured > 7.0, "real metrics, got {measured}");
    let rect_w = paint.canvas.iter().find_map(|op| match op {
        CanvasOp::FillRect { w, .. } => Some(*w),
        _ => None,
    });
    let expected = (measured + 12.0).round() as i32;
    assert_eq!(rect_w, Some(expected), "rect follows measureText + 12");
    iso.join();
}

#[test]
fn profileprobe_shaped_ctx_has_no_fallback() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.font = '12px monospace';
        ctx.fillStyle = 'rgba(0, 0, 0, 0.7)';
        ctx.fillRect(6, 6, 400, 50);
        ctx.fillStyle = '#ffd166';
        ctx.fillText('P2 alpha stride=1', 12, 23);
        ctx.fillText('loops=0 total=0', 12, 43);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(!is_fallback(&paint), "{paint:?}");
    assert_eq!(paint.lines.len(), 0);
    assert!(paint.title.is_none());
    assert_eq!(paint.canvas.len(), 3);
    iso.join();
}

#[test]
fn unsupported_verbs_throw_not_impl_and_canvas_is_undefined() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        globalThis.__canvasIs = ctx.canvas;
        ctx.beginPath();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(is_error(&paint), "{paint:?}");
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("not impl: Canvas.beginPath")),
        "{paint:?}"
    );
    assert!(paint.canvas.is_empty(), "throw discards this call's canvas");
    let canvas_is = iso.probe("__canvasIs").expect("probe");
    assert!(
        canvas_is.is_null(),
        "ctx.canvas must be undefined, got {canvas_is}"
    );
    iso.join();
}

#[test]
fn filltext_maxwidth_is_explicit_missing() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.fillText('hi', 1, 2, 10);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("not impl: Canvas.fillText.maxWidth")),
        "{paint:?}"
    );
    iso.join();
}

#[test]
fn drawimage_is_explicit_missing() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.drawImage(null, 0, 0);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("not impl: Canvas.drawImage")),
        "{paint:?}"
    );
    iso.join();
}

#[test]
fn canvas_to_empty_plants_fallback_not_stale_ops() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        if (globalThis.__rs2b0t_host.tick === 1) {
            ctx.fillStyle = '#ffb15b';
            ctx.fillRect(6, 6, 40, 10);
            ctx.fillText('banner', 8, 14);
        }
    }
}
"#;
    let iso = spawn(src);
    let first = tick_paint(&iso, 1);
    assert!(!first.canvas.is_empty(), "{first:?}");
    let second = tick_paint(&iso, 2);
    assert!(
        second.canvas.is_empty(),
        "empty onPaint must drop canvas: {second:?}"
    );
    assert!(is_fallback(&second), "{second:?}");
    iso.join();
}

#[test]
fn canvas_to_error_to_canvas_does_not_keep_banner() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        const t = globalThis.__rs2b0t_host.tick;
        if (t === 1) {
            ctx.fillRect(6, 6, 40, 10);
            ctx.fillText('ok', 8, 14);
            return;
        }
        if (t === 2) {
            ctx.beginPath();
            return;
        }
        ctx.fillRect(6, 6, 20, 8);
        ctx.fillText('back', 8, 14);
    }
}
"#;
    let iso = spawn(src);
    let first = tick_paint(&iso, 1);
    assert!(!first.canvas.is_empty());
    let err = tick_paint(&iso, 2);
    assert!(is_error(&err), "{err:?}");
    assert!(err.canvas.is_empty());
    let back = tick_paint(&iso, 3);
    assert!(!is_error(&back) && !is_fallback(&back), "{back:?}");
    assert!(
        back.canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillText { text, .. } if text == "back")),
        "{back:?}"
    );
    iso.join();
}

#[test]
fn structured_plus_canvas_coexist_and_empty_onpaint_keeps_loop_paint() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    loop() {
        const p = Paint.begin(null, { accent: '#f3e6a2' });
        p.title('BoneBurier — digging');
        p.row('Runtime: 1.2m');
        p.end();
    }
    onPaint(ctx) {
        if (globalThis.__rs2b0t_host.tick === 1) {
            ctx.fillRect(6, 6, 40, 10);
            ctx.fillText('banner', 8, 14);
        }
    }
}
"#;
    let iso = spawn(src);
    let both = tick_paint(&iso, 1);
    assert_eq!(both.title.as_deref(), Some("BoneBurier — digging"));
    assert!(!both.canvas.is_empty(), "{both:?}");
    let kept = tick_paint(&iso, 2);
    assert_eq!(kept.title.as_deref(), Some("BoneBurier — digging"));
    assert!(
        kept.canvas.is_empty(),
        "empty onPaint strips canvas, keeps loop paint: {kept:?}"
    );
    iso.join();
}

#[test]
fn two_isolates_do_not_share_canvas() {
    let a_src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) { ctx.fillText('AAA', 12, 22); }
}
"#;
    let b_src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) { ctx.fillText('BBB', 12, 22); }
}
"#;
    let a = spawn(a_src);
    let b = spawn(b_src);
    let pa = tick_paint(&a, 1);
    let pb = tick_paint(&b, 1);
    let ta = pa.canvas.iter().find_map(|op| match op {
        CanvasOp::FillText { text, .. } => Some(text.as_str()),
        _ => None,
    });
    let tb = pb.canvas.iter().find_map(|op| match op {
        CanvasOp::FillText { text, .. } => Some(text.as_str()),
        _ => None,
    });
    assert_eq!(ta, Some("AAA"));
    assert_eq!(tb, Some("BBB"));
    a.join();
    b.join();
}

#[test]
fn oversized_op_list_fails_closed_and_isolate_lives() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        for (let i = 0; i < 300; i++) ctx.fillRect(i % 50, 1, 1, 1);
        globalThis.__alive = true;
    }
}
"#;
    let iso = spawn(src);
    iso.on_game_tick(1);
    let _ = iso.probe("0");
    let alive = iso.probe("__alive").expect("isolate lives");
    assert_eq!(alive, true);
    if let Some(paint) = iso.paint() {
        assert!(
            paint.canvas.len() <= canvas::MAX_CANVAS_OPS,
            "capped, got {}",
            paint.canvas.len()
        );
    }
    iso.join();
}

#[test]
fn pause_retains_last_canvas_and_skips_new_ticks() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        globalThis.__paints = (globalThis.__paints || 0) + 1;
        ctx.fillText('tick-' + String(globalThis.__rs2b0t_host.tick), 12, 22);
    }
}
"#;
    let iso = spawn(src);
    let first = tick_paint(&iso, 1);
    let paints_after_first = iso.probe("__paints").unwrap().as_i64().unwrap();
    assert!(paints_after_first >= 1);
    iso.pause();
    iso.on_game_tick(2);
    let _ = iso.probe("0");
    assert_eq!(
        iso.probe("__paints").unwrap().as_i64().unwrap(),
        paints_after_first,
        "paused ticks must not run onPaint"
    );
    let paused = iso.paint().expect("last frame retained");
    assert_eq!(paused.canvas, first.canvas);
    iso.resume();
    let resumed = tick_paint(&iso, 3);
    assert_ne!(resumed.canvas, first.canvas, "resume paints a new tick");
    iso.join();
}

#[test]
fn encode_decode_preserves_ops_and_old_buffers_decode() {
    let paint = ScriptPaint {
        title: Some("t".into()),
        accent: None,
        lines: vec!["line".into()],
        buttons: Vec::new(),
        generation: 0,
        canvas: vec![
            CanvasOp::FillRect {
                x: 6,
                y: 6,
                w: 400,
                h: 50,
                color: canvas::pack_rgba(0, 0, 0, 178),
            },
            CanvasOp::FillText {
                text: "hi".into(),
                x: 12,
                y: 22,
                color: canvas::pack_rgba(0xff, 0xb1, 0x5b, 255),
                font_px: 12,
                mono: true,
            },
        ],
    };
    let bytes = IsolateBuf::new().encode_paint(&paint);
    let decoded = decode_paint(&bytes).expect("roundtrip");
    assert_eq!(decoded.canvas, paint.canvas);
    assert_eq!(decoded.title, paint.title);
    assert_eq!(decoded.lines, paint.lines);

    let old = ScriptPaint {
        title: Some("old".into()),
        accent: None,
        lines: vec!["x".into()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
    };
    let old_bytes = IsolateBuf::new().encode_paint(&old);
    let old_decoded = decode_paint(&old_bytes).expect("old buffer");
    assert!(old_decoded.canvas.is_empty());

    let mut huge = old.clone();
    huge.canvas = (0..=canvas::MAX_CANVAS_OPS)
        .map(|i| CanvasOp::FillRect {
            x: i as i32,
            y: 0,
            w: 1,
            h: 1,
            color: 0,
        })
        .collect();
    let over = IsolateBuf::new().encode_paint(&huge);
    assert!(
        decode_paint(&over).is_err(),
        "oversized canvas vector must fail closed"
    );
}

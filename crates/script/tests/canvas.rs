//! Native Canvas recorder: unchanged template ops, transitions, wire, caps.

use std::path::PathBuf;

use script::canvas::{self, CanvasOp, PathSeg};
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
        ctx.translate();
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
            .any(|l| l.contains("not impl: Canvas.translate")),
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
            ctx.translate();
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
            CanvasOp::fill_rect(6, 6, 400, 50, canvas::pack_rgba(0, 0, 0, 178)),
            CanvasOp::fill_text(
                "hi",
                12,
                22,
                canvas::pack_rgba(0xff, 0xb1, 0x5b, 255),
                12,
                true,
            ),
        ],
        ..Default::default()
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
        ..Default::default()
    };
    let old_bytes = IsolateBuf::new().encode_paint(&old);
    let old_decoded = decode_paint(&old_bytes).expect("old buffer");
    assert!(old_decoded.canvas.is_empty());

    let mut huge = old.clone();
    huge.canvas = (0..=canvas::MAX_CANVAS_OPS)
        .map(|i| CanvasOp::fill_rect(i as i32, 0, 1, 1, 0))
        .collect();
    let over = IsolateBuf::new().encode_paint(&huge);
    assert!(
        decode_paint(&over).is_err(),
        "oversized canvas vector must fail closed"
    );

    let mut huge_font = old.clone();
    huge_font.canvas = vec![CanvasOp::fill_text("x", 0, 10, 0, 65535, true)];
    let over_font = IsolateBuf::new().encode_paint(&huge_font);
    assert!(
        decode_paint(&over_font).is_err(),
        "decoded font_px must not exceed parser cap 256"
    );
}

#[test]
fn user_onpaint_title_and_paint_end_line_survive_with_canvas() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint(ctx) {
        const p = Paint.begin(null, { accent: '#ff5555' });
        p.title('onPaint');
        p.row('Paint.end was not called');
        p.end();
        ctx.fillRect(6, 6, 20, 8);
        ctx.fillText('banner', 8, 14);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(paint.title.as_deref(), Some("onPaint"));
    assert_eq!(paint.accent.as_deref(), Some("#ff5555"));
    assert!(
        paint.lines.iter().any(|l| l.contains("Paint.end")),
        "{paint:?}"
    );
    assert!(
        !paint.canvas.is_empty(),
        "user structured paint must keep canvas: {paint:?}"
    );
    iso.join();
}

#[test]
fn oversized_ops_plant_diagnostic_then_recover() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        const t = globalThis.__rs2b0t_host.tick;
        if (t === 1) {
            for (let i = 0; i < 300; i++) ctx.fillRect(i % 50, 1, 1, 1);
            globalThis.__alive = true;
            return;
        }
        ctx.fillRect(6, 6, 10, 10);
        ctx.fillText('ok', 8, 14);
    }
}
"#;
    let iso = spawn(src);
    let first = tick_paint(&iso, 1);
    assert_eq!(iso.probe("__alive").unwrap(), true);
    assert!(first.canvas.is_empty(), "{first:?}");
    assert!(
        first.lines.iter().any(|l| l.contains("canvas:")),
        "bounded diagnostic, not silent drop or Paint.end fallback: {first:?}"
    );
    let second = tick_paint(&iso, 2);
    assert!(!is_fallback(&second) && !is_error(&second), "{second:?}");
    assert!(!second.canvas.is_empty(), "{second:?}");
    iso.join();
}

#[test]
fn oversized_measure_fails_closed_then_recovers() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        const t = globalThis.__rs2b0t_host.tick;
        if (t === 1) {
            ctx.measureText('x'.repeat(600));
            return;
        }
        ctx.fillText('ok', 8, 14);
    }
}
"#;
    let iso = spawn(src);
    let first = tick_paint(&iso, 1);
    assert!(first.canvas.is_empty(), "{first:?}");
    assert!(
        first
            .lines
            .iter()
            .any(|l| l.contains("measureText") || l.contains("canvas:")),
        "{first:?}"
    );
    let second = tick_paint(&iso, 2);
    assert!(
        second
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillText { text, .. } if text == "ok")),
        "{second:?}"
    );
    iso.join();
}

#[test]
fn sherpa_required_verbs_record_without_not_impl() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.save();
        ctx.shadowColor = 'rgba(8, 4, 0, 0.55)';
        ctx.shadowBlur = 10;
        ctx.shadowOffsetY = 3;
        const leather = ctx.createLinearGradient(279, 211, 279, 333);
        leather.addColorStop(0, '#6d4a2e');
        leather.addColorStop(0.45, '#4a2e1c');
        leather.addColorStop(1, '#3a2416');
        ctx.beginPath();
        ctx.moveTo(287, 211);
        ctx.lineTo(439, 211);
        ctx.quadraticCurveTo(447, 211, 447, 219);
        ctx.lineTo(447, 325);
        ctx.quadraticCurveTo(447, 333, 439, 333);
        ctx.lineTo(287, 333);
        ctx.quadraticCurveTo(279, 333, 279, 325);
        ctx.lineTo(279, 219);
        ctx.quadraticCurveTo(279, 211, 287, 211);
        ctx.closePath();
        ctx.fillStyle = leather;
        ctx.fill();
        ctx.shadowColor = 'transparent';
        ctx.strokeStyle = '#d2b07a';
        ctx.lineWidth = 1.6;
        ctx.stroke();
        ctx.save();
        ctx.clip();
        ctx.fillStyle = 'rgba(20, 12, 6, 0.28)';
        ctx.fillRect(279, 231, 168, 6);
        ctx.restore();
        ctx.beginPath();
        ctx.arc(433, 295, 36, 0, Math.PI * 2);
        const ring = ctx.createRadialGradient(429, 291, 6, 433, 295, 39.5);
        ring.addColorStop(0, '#e8c888');
        ring.addColorStop(0.55, '#d2b07a');
        ring.addColorStop(1, '#5a3a16');
        ctx.fillStyle = ring;
        ctx.fill();
        ctx.textBaseline = 'top';
        ctx.textAlign = 'left';
        ctx.fillStyle = '#f4e6c8';
        ctx.font = 'bold 11px sans-serif';
        ctx.fillText("TENZING'S PASS", 313, 237);
        ctx.restore();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(
        !paint.lines.iter().any(|l| l.contains("not impl: Canvas.")),
        "{paint:?}"
    );
    assert!(
        paint
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillPath { .. })),
        "path fill: {paint:?}"
    );
    assert!(
        paint
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::StrokePath { .. })),
        "path stroke: {paint:?}"
    );
    let raster = canvas::rasterize(&paint.canvas).expect("raster");
    assert!(raster.w > 10 && raster.h > 10);
    let mut opaque = 0usize;
    for px in raster.rgba.chunks_exact(4) {
        if px[3] > 0 {
            opaque += 1;
        }
    }
    assert!(opaque > 50, "sherpa frame must paint, opaque={opaque}");
    iso.join();
}

#[test]
fn firegiant_outline_records_round_join_strokes() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.save();
        ctx.strokeStyle = 'rgba(255, 224, 64, 0.55)';
        ctx.lineWidth = 1.5;
        ctx.lineJoin = 'round';
        const box = [
            {x: 40, y: 40}, {x: 80, y: 40}, {x: 80, y: 90}, {x: 40, y: 90},
            {x: 50, y: 30}, {x: 90, y: 30}, {x: 90, y: 80}, {x: 50, y: 80}
        ];
        const edge = (a, b) => {
            ctx.beginPath();
            ctx.moveTo(box[a].x, box[a].y);
            ctx.lineTo(box[b].x, box[b].y);
            ctx.stroke();
        };
        for (let i = 0; i < 4; i++) {
            edge(i, (i + 1) % 4);
            edge(4 + i, 4 + ((i + 1) % 4));
            edge(i, 4 + i);
        }
        ctx.restore();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert!(
        !paint.lines.iter().any(|l| l.contains("not impl:")),
        "{paint:?}"
    );
    let strokes: Vec<_> = paint
        .canvas
        .iter()
        .filter(|op| matches!(op, CanvasOp::StrokePath { .. }))
        .collect();
    assert_eq!(strokes.len(), 12, "{paint:?}");
    match strokes[0] {
        CanvasOp::StrokePath {
            line_width,
            line_join,
            ..
        } => {
            assert!((line_width - 1.5).abs() < 1e-5);
            assert_eq!(*line_join, canvas::LineJoinKind::Round);
        }
        _ => unreachable!(),
    }
    assert!(canvas::rasterize(&paint.canvas).is_some());
    iso.join();
}

#[test]
fn clip_intersection_and_restore_roundtrip() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.beginPath();
        ctx.moveTo(10, 10);
        ctx.lineTo(60, 10);
        ctx.lineTo(60, 60);
        ctx.lineTo(10, 60);
        ctx.closePath();
        ctx.save();
        ctx.clip();
        ctx.fillStyle = '#ff0000';
        ctx.fillRect(0, 0, 80, 80);
        ctx.restore();
        ctx.fillStyle = '#00ff00';
        ctx.fillRect(70, 70, 10, 10);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(paint.canvas.len(), 2, "{paint:?}");
    match &paint.canvas[0] {
        CanvasOp::FillRect { extras, .. } => assert_eq!(extras.clips.len(), 1),
        other => panic!("{other:?}"),
    }
    match &paint.canvas[1] {
        CanvasOp::FillRect { extras, .. } => assert!(extras.clips.is_empty()),
        other => panic!("{other:?}"),
    }
    let bytes = IsolateBuf::new().encode_paint(&paint);
    let decoded = decode_paint(&bytes).expect("roundtrip");
    assert_eq!(decoded.canvas, paint.canvas);
    let raster = canvas::rasterize(&paint.canvas).expect("raster");
    let at = |x: i32, y: i32| -> [u8; 4] {
        let col = (x - raster.x) as usize;
        let row = (y - raster.y) as usize;
        let i = (row * raster.w as usize + col) * 4;
        [
            raster.rgba[i],
            raster.rgba[i + 1],
            raster.rgba[i + 2],
            raster.rgba[i + 3],
        ]
    };
    let inside = at(30, 30);
    assert!(
        inside[0] > 200 && inside[3] > 200,
        "clipped red inside {inside:?}"
    );
    iso.join();
}

#[test]
fn linear_and_radial_are_nonconstant() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        const g = ctx.createLinearGradient(10, 20, 80, 20);
        g.addColorStop(0, '#ff0000');
        g.addColorStop(1, '#0000ff');
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.moveTo(10, 10);
        ctx.lineTo(80, 10);
        ctx.lineTo(80, 30);
        ctx.lineTo(10, 30);
        ctx.closePath();
        ctx.fill();
        const ring = ctx.createRadialGradient(40, 70, 6, 44, 74, 24);
        ring.addColorStop(0, '#ffffff');
        ring.addColorStop(1, '#000000');
        ctx.fillStyle = ring;
        ctx.beginPath();
        ctx.arc(44, 74, 24, 0, Math.PI * 2);
        ctx.fill();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(paint.canvas.len(), 2, "{paint:?}");
    let raster = canvas::rasterize(&paint.canvas).expect("raster");
    let mut colors = std::collections::BTreeSet::new();
    for px in raster.rgba.chunks_exact(4) {
        if px[3] > 200 {
            colors.insert((px[0] / 16, px[1] / 16, px[2] / 16));
        }
    }
    assert!(
        colors.len() >= 2,
        "gradient must not collapse to one stop, got {colors:?}"
    );
    iso.join();
}

#[test]
fn fill_then_stroke_preserves_path_and_invalid_addcolorstop_throws() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.beginPath();
        ctx.moveTo(10, 10);
        ctx.lineTo(40, 10);
        ctx.lineTo(40, 40);
        ctx.closePath();
        ctx.fillStyle = '#ffffff';
        ctx.fill();
        ctx.strokeStyle = '#000000';
        ctx.lineWidth = 2;
        ctx.stroke();
        try {
            const g = ctx.createLinearGradient(0, 0, 1, 0);
            g.addColorStop(2, '#fff');
            globalThis.__stopOk = false;
        } catch (e) {
            globalThis.__stopOk = String(e.message || e).indexOf('IndexSizeError') >= 0;
        }
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(paint.canvas.len(), 2);
    assert!(matches!(paint.canvas[0], CanvasOp::FillPath { .. }));
    assert!(matches!(paint.canvas[1], CanvasOp::StrokePath { .. }));
    assert_eq!(iso.probe("__stopOk").unwrap(), true);
    iso.join();
}

fn path_cubics(segs: &[PathSeg]) -> usize {
    segs.iter()
        .filter(|s| matches!(s, PathSeg::CubicTo { .. }))
        .count()
}

fn path_end(segs: &[PathSeg]) -> (f32, f32) {
    match *segs.last().expect("segs") {
        PathSeg::MoveTo { x, y } | PathSeg::LineTo { x, y } => (x, y),
        PathSeg::CubicTo { x, y, .. } => (x, y),
        PathSeg::QuadTo { x, y, .. } => (x, y),
        PathSeg::Close => panic!("close"),
    }
}

#[test]
fn arc_ccw_equal_and_directed_isolate_geometry() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.fillStyle = '#ffffff';
        ctx.beginPath();
        ctx.arc(0, 0, 10, 0, Math.PI / 2, true);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(0, 0, 10, 0, -Math.PI / 2, true);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(4, 5, 8, 1.25, 1.25, false);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(4, 5, 8, 1.25, 1.25, true);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(0, 0, 10, 0, Math.PI * 2);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(0, 0, 10, 0, Math.PI * 3, true);
        ctx.fill();
        ctx.beginPath();
        ctx.arc(0, 0, 10, 0, -Math.PI * 2, false);
        ctx.fill();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(paint.canvas.len(), 7, "{paint:?}");
    let segs_of = |i: usize| -> &[PathSeg] {
        match &paint.canvas[i] {
            CanvasOp::FillPath { segs, .. } => segs,
            other => panic!("op {i}: {other:?}"),
        }
    };

    let q = segs_of(0);
    assert_eq!(path_cubics(q), 3, "{q:?}");
    let (x, y) = path_end(q);
    assert!(
        x.abs() < 1e-3 && (y - 10.0).abs() < 1e-3,
        "ccw quarter end {x},{y}"
    );

    let nq = segs_of(1);
    assert_eq!(path_cubics(nq), 1, "{nq:?}");
    let (x, y) = path_end(nq);
    assert!(
        x.abs() < 1e-3 && (y + 10.0).abs() < 1e-3,
        "ccw -quarter end {x},{y}"
    );

    for i in [2, 3] {
        let s = segs_of(i);
        assert_eq!(path_cubics(s), 0, "equal {i} {s:?}");
        assert_eq!(s.len(), 1, "equal {i}");
    }

    let full = segs_of(4);
    assert_eq!(path_cubics(full), 4, "{full:?}");
    let (x, y) = path_end(full);
    assert!(
        (x - 10.0).abs() < 1e-3 && y.abs() < 1e-3,
        "full cw end {x},{y}"
    );

    let half = segs_of(5);
    assert_eq!(path_cubics(half), 2, "{half:?}");
    let (x, y) = path_end(half);
    assert!(
        (x + 10.0).abs() < 1e-3 && y.abs() < 1e-3,
        "3pi ccw end {x},{y}"
    );

    let zero = segs_of(6);
    assert_eq!(path_cubics(zero), 0, "{zero:?}");
    iso.join();
}

#[test]
fn arc_zero_radius_connects_without_cubics_isolate() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        ctx.beginPath();
        ctx.moveTo(10, 10);
        ctx.arc(50, 60, 0, 0, Math.PI);
        ctx.fillStyle = '#ffffff';
        ctx.fill();
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    match &paint.canvas[0] {
        CanvasOp::FillPath { segs, .. } => {
            assert_eq!(path_cubics(segs), 0, "{segs:?}");
            assert!(
                segs.iter().any(
                    |s| matches!(s, PathSeg::LineTo { x, y } if (*x - 50.0).abs() < 1e-4 && (*y - 60.0).abs() < 1e-4)
                ),
                "{segs:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    iso.join();
}

#[test]
fn arc_negative_radius_throws_index_size_error_and_isolate_stays_alive() {
    let src = r#"
export default class T extends LoopingBot {
    onPaint(ctx) {
        try {
            ctx.arc(10, 10, -1, 0, 1);
            globalThis.__neg = false;
        } catch (e) {
            globalThis.__neg = String(e.message || e).indexOf('IndexSizeError') >= 0;
        }
        ctx.fillStyle = '#ffffff';
        ctx.fillRect(2, 2, 4, 4);
    }
}
"#;
    let iso = spawn(src);
    let paint = tick_paint(&iso, 1);
    assert_eq!(iso.probe("__neg").unwrap(), true);
    assert!(
        paint
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillRect { .. })),
        "{paint:?}"
    );
    assert!(!is_error(&paint), "{paint:?}");
    let second = tick_paint(&iso, 2);
    assert!(
        second
            .canvas
            .iter()
            .any(|op| matches!(op, CanvasOp::FillRect { .. })),
        "{second:?}"
    );
    iso.join();
}

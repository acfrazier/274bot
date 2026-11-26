//! The same paint program through the posted JS and lent Luau surfaces.

use super::PaintView;
use script::canvas::{CanvasOp, FillPaint, MAX_CANVAS_OPS};
use script::paint::ScriptPaint;
use script::surface::LentOutcome;
use script_js::LoadIsolate;

#[path = "../../../script-luau/tests/support/mod.rs"]
pub mod support;

const JS: &str = r#"
export default class PaintParity extends LoopingBot {
    onPaint(ctx) {
        this.frame = (this.frame || 0) + 1;
        if (this.frame === 2 || this.frame === 3) {
            for (let i = 0; i < 254 + this.frame; i++) ctx.fillRect(i, 2, 1, 1);
        } else if (this.frame === 4) {
            ctx.fillRect(3, 4, 1, 1);
            ctx.fillText('é'.repeat(257), 5, 6);
        } else if (this.frame === 5) {
            ctx.fillRect(7, 8, 1, 1);
            for (let i = 0; i < 32; i++) ctx.save();
        } else if (this.frame === 6) {
            ctx.fillRect(11, 12, 1, 1);
        } else {
            ctx.font = '14px monospace';
            ctx.fillStyle = '#345';
            ctx.fillStyle = 'invalid color';
            if (ctx.fillStyle !== '#345') throw new Error('bad color readback');
            ctx.fillRect(12, 8, ctx.measureText('Éclair').width + 8, 26);
            ctx.save();
            ctx.beginPath();
            ctx.moveTo(12, 8);
            ctx.lineTo(65, 8);
            ctx.quadraticCurveTo(65, 40, 12, 34);
            ctx.closePath();
            ctx.clip();
            const linear = ctx.createLinearGradient(12, 8, 65, 34);
            linear.addColorStop(0, '#f80');
            linear.addColorStop(1, '#0088ff');
            ctx.fillStyle = linear;
            if (ctx.fillStyle !== linear) throw new Error('bad gradient readback');
            ctx.fill();
            ctx.strokeStyle = '#0f0';
            ctx.lineWidth = 2.5;
            ctx.lineJoin = 'round';
            ctx.beginPath();
            ctx.arc(36, 20, 8, 0, 3, true);
            ctx.stroke();
            const radial = ctx.createRadialGradient(34, 18, 1, 34, 18, 20);
            radial.addColorStop(0.25, '#fff');
            radial.addColorStop(0.9, '#000');
            ctx.fillStyle = radial;
            ctx.shadowColor = '#40404080';
            ctx.shadowBlur = 1;
            ctx.shadowOffsetX = 2;
            ctx.shadowOffsetY = -1;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText('Éclair', 35, 19);
            ctx.restore();
            ctx.fillRect(70, 8, 2, 2);
        }
    }
}
"#;

const LUAU: &str = r#"
local frame = 0
return {
    tick = function() end,
    onPaint = function(ctx)
        frame += 1
        if frame == 2 or frame == 3 then
            for i = 0, 253 + frame do ctx:fillRect(i, 2, 1, 1) end
        elseif frame == 4 then
            ctx:fillRect(3, 4, 1, 1)
            ctx:fillText(string.rep('é', 257), 5, 6)
        elseif frame == 5 then
            ctx:fillRect(7, 8, 1, 1)
            for i = 1, 32 do ctx:save() end
        elseif frame == 6 then
            ctx:fillRect(11, 12, 1, 1)
        else
            ctx.font = '14px monospace'
            ctx.fillStyle = '#345'
            ctx.fillStyle = 'invalid color'
            assert(ctx.fillStyle == '#345', 'bad color readback')
            ctx:fillRect(12, 8, ctx:measureText('Éclair').width + 8, 26)
            ctx:save()
            ctx:beginPath()
            ctx:moveTo(12, 8)
            ctx:lineTo(65, 8)
            ctx:quadraticCurveTo(65, 40, 12, 34)
            ctx:closePath()
            ctx:clip()
            local linear = ctx:createLinearGradient(12, 8, 65, 34)
            linear:addColorStop(0, '#f80')
            linear:addColorStop(1, '#0088ff')
            ctx.fillStyle = linear
            assert(ctx.fillStyle == linear, 'bad gradient readback')
            ctx:fill()
            ctx.strokeStyle = '#0f0'
            ctx.lineWidth = 2.5
            ctx.lineJoin = 'round'
            ctx:beginPath()
            ctx:arc(36, 20, 8, 0, 3, true)
            ctx:stroke()
            local radial = ctx:createRadialGradient(34, 18, 1, 34, 18, 20)
            radial:addColorStop(0.25, '#fff')
            radial:addColorStop(0.9, '#000')
            ctx.fillStyle = radial
            ctx.shadowColor = '#40404080'
            ctx.shadowBlur = 1
            ctx.shadowOffsetX = 2
            ctx.shadowOffsetY = -1
            ctx.textAlign = 'center'
            ctx.textBaseline = 'middle'
            ctx:fillText('Éclair', 35, 19)
            ctx:restore()
            ctx:fillRect(70, 8, 2, 2)
        end
    end,
}
"#;

#[test]
fn luau_and_js_publish_the_same_canvas_ops_and_fail_closed_at_the_same_bounds() {
    let prepared =
        script_js::prepare_raw_origin_for_test("canvas.js", JS, &[]).expect("prepare JS");
    let js = LoadIsolate::spawn(JS.to_owned(), prepared.checked.shape(), vec![]).expect("spawn JS");
    let mut luau = support::spawn(LUAU);

    for tick in 1..=6 {
        js.on_game_tick(tick);
        js.probe("0").expect("wait until JS publishes paint");
        assert_eq!(
            support::tick(&mut *luau, &support::snapshot(tick)),
            LentOutcome::Accepted
        );
        let js_frame = js.paint().expect("JS paint frame");
        let luau_frame = luau.paint().expect("Luau paint frame");
        assert_eq!(
            luau_frame.canvas, js_frame.canvas,
            "canvas ops on tick {tick}"
        );
        assert_eq!(
            luau_frame.lines, js_frame.lines,
            "diagnostic on tick {tick}"
        );

        match tick {
            1 => assert_raster_parity(&luau_frame, &js_frame),
            2 => assert_eq!(luau_frame.canvas.len(), MAX_CANVAS_OPS),
            3 => assert_eq!(luau_frame.lines, ["canvas: exceeded 256 ops"]),
            4 => assert_eq!(luau_frame.lines, ["canvas: text exceeds 512 byte cap"]),
            5 => assert_eq!(luau_frame.lines, ["canvas: exceeded save depth"]),
            6 => assert!(matches!(
                &luau_frame.canvas[..],
                [CanvasOp::FillRect {
                    x: 11,
                    y: 12,
                    color: 0xff,
                    ..
                }]
            )),
            _ => unreachable!(),
        }
        if (3..=5).contains(&tick) {
            assert!(
                luau_frame.canvas.is_empty(),
                "drop partial canvas on tick {tick}"
            );
        }
    }
    assert!(support::stop(luau).hook_failure.is_none());
    js.join();
}

fn assert_raster_parity(luau: &ScriptPaint, js: &ScriptPaint) {
    let extent = |r: &super::CanvasRaster| (r.x(), r.y(), r.width(), r.height());
    let [CanvasOp::FillRect { w, .. }, CanvasOp::FillPath { extras: clip, .. }, CanvasOp::StrokePath { .. }, CanvasOp::FillText { extras: text, .. }, CanvasOp::FillRect {
        extras: restored, ..
    }] = &luau.canvas[..]
    else {
        panic!("expected all five draw operations, got {luau:?}");
    };
    assert!(*w > 8, "the banner uses measured glyph advances");
    assert!(matches!(clip.fill, FillPaint::Linear { .. }) && clip.clips.len() == 1);
    assert!(matches!(text.fill, FillPaint::Radial { .. }));
    assert!(matches!(restored.fill, FillPaint::Solid) && restored.clips.is_empty());

    let luau_raster = PaintView::new(luau)
        .canvas()
        .rasterize()
        .expect("Luau raster");
    let js_raster = PaintView::new(js).canvas().rasterize().expect("JS raster");
    assert_eq!(extent(&luau_raster), extent(&js_raster));
    assert_eq!(luau_raster.rgba(), js_raster.rgba());
    assert!(luau_raster
        .rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] != 0));
}

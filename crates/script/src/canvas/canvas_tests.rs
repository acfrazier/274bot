use super::*;
    fn reset() {
        begin();
    }

    #[test]
    fn parse_template_fill_styles() {
        assert_eq!(
            parse_color("#ffb15b"),
            Some(pack_rgba(0xff, 0xb1, 0x5b, 255))
        );
        assert_eq!(
            parse_color("#ffd166"),
            Some(pack_rgba(0xff, 0xd1, 0x66, 255))
        );
        let a = parse_color("rgba(0, 0, 0, 0.6)").unwrap();
        assert_eq!(unpack_rgba(a)[..3], [0, 0, 0]);
        assert_eq!(unpack_rgba(a)[3], (0.6_f32 * 255.0).round() as u8);
        assert_eq!(parse_color("#000"), Some(pack_rgba(0, 0, 0, 255)));
        assert_eq!(parse_color("transparent"), Some(pack_rgba(0, 0, 0, 0)));
        assert!(parse_color("red").is_none());
        assert!(parse_color("not-a-color").is_none());
    }

    #[test]
    fn unparseable_fill_style_keeps_previous() {
        reset();
        set_style("fillStyle", "#ffb15b");
        set_style("fillStyle", "red");
        set_style("fillStyle", "???");
        fill_rect(0.0, 0.0, 1.0, 1.0);
        match &take().ops[0] {
            CanvasOp::FillRect { color, .. } => {
                assert_eq!(*color, pack_rgba(0xff, 0xb1, 0x5b, 255));
            }
            _ => panic!("expected fillRect"),
        }
    }

    #[test]
    fn parse_template_fonts_and_unknown_keeps_previous() {
        assert_eq!(parse_font("12px monospace"), Some((12, true)));
        assert_eq!(parse_font("10px sans-serif"), Some((10, false)));
        assert!(parse_font("12px comic-sans-invented").is_none());
        reset();
        set_style("font", "12px monospace");
        set_style("font", "nope");
        fill_text("x", 0.0, 0.0);
        match &take().ops[0] {
            CanvasOp::FillText { font_px, mono, .. } => {
                assert_eq!((*font_px, *mono), (12, true), "nope keeps 12px monospace");
            }
            _ => panic!("expected fillText"),
        }
    }

    #[test]
    fn measure_text_is_not_width_seven() {
        reset();
        set_style("font", "12px monospace");
        let w = measure_text("BoneBurier (external)  buried 0").unwrap();
        assert!(w > 7.0, "real advance width, got {w}");
        let same = measure_with(12, true, "BoneBurier (external)  buried 0");
        assert!((w - same).abs() < 0.01);
        let short = measure_text("x").unwrap();
        assert!(w > short);
        let sans = measure_with(12, false, "BoneBurier (external)  buried 0");
        assert!(
            (sans - w).abs() > 0.5,
            "monospace must not be a relabeled sans face"
        );
    }

    #[test]
    fn raster_dirty_covers_measured_fillrect_and_glyphs() {
        reset();
        set_style("font", "12px monospace");
        let text = "BoneBurier (external)  buried 0";
        let width = measure_text(text).unwrap();
        set_style("fillStyle", "rgba(0, 0, 0, 0.6)");
        fill_rect(6.0, 6.0, width + 12.0, 24.0);
        set_style("fillStyle", "#ffb15b");
        fill_text(text, 12.0, 22.0);
        let taken = take();
        assert!(!taken.overflow);
        assert_eq!(taken.ops.len(), 2);
        let raster = rasterize(&taken.ops).expect("raster");
        assert!(raster.w > 0 && raster.h > 0);
        assert!(raster.byte_len() <= (APPLET_W as usize) * (APPLET_H as usize) * 4);
        assert!(raster.x <= 6 && raster.y <= 6);
        let mut opaque = 0usize;
        for px in raster.rgba.as_chunks::<4>().0 {
            if px[3] > 0 {
                opaque += 1;
            }
        }
        assert!(opaque > 50, "banner must actually paint, opaque={opaque}");
        let bg_w = (width + 12.0).round() as i32;
        assert!(
            raster.x + raster.w as i32 >= 6 + bg_w,
            "dirty width must cover the measured rect"
        );
    }

    #[test]
    fn fill_rect_clips_to_applet() {
        let ops = vec![CanvasOp::fill_rect(
            -10,
            -10,
            40,
            40,
            pack_rgba(255, 0, 0, 255),
        )];
        let raster = rasterize(&ops).unwrap();
        assert_eq!(raster.x, 0);
        assert_eq!(raster.y, 0);
        let ops = vec![CanvasOp::fill_rect(
            APPLET_W - 5,
            APPLET_H - 5,
            20,
            20,
            pack_rgba(0, 255, 0, 255),
        )];
        let raster = rasterize(&ops).unwrap();
        assert!(raster.x + raster.w as i32 <= APPLET_W);
        assert!(raster.y + raster.h as i32 <= APPLET_H);
        let ops = vec![CanvasOp::fill_rect(
            APPLET_W + 4,
            0,
            10,
            10,
            pack_rgba(0, 0, 255, 255),
        )];
        assert!(rasterize(&ops).is_none());
    }

    #[test]
    fn recorder_caps_ops_and_text() {
        reset();
        for _ in 0..(MAX_CANVAS_OPS + 4) {
            fill_rect(1.0, 1.0, 2.0, 2.0);
        }
        let taken = take();
        assert!(taken.overflow);
        assert!(taken.ops.len() <= MAX_CANVAS_OPS);
        reset();
        let huge = "x".repeat(MAX_PAINT_TEXT + 1);
        fill_text(&huge, 0.0, 10.0);
        let taken = take();
        assert!(taken.overflow);
        assert!(taken.ops.is_empty());
    }

    #[test]
    fn each_begin_replaces_ops() {
        reset();
        fill_rect(1.0, 1.0, 2.0, 2.0);
        begin();
        fill_text("hi", 3.0, 4.0);
        let taken = take();
        assert_eq!(taken.ops.len(), 1);
        assert!(matches!(taken.ops[0], CanvasOp::FillText { .. }));
    }

    #[test]
    fn applet_map_matches_chatbox_scale() {
        let native = map_applet_rect([10.0, 20.0], [765.0, 503.0], 6, 6, 400, 50);
        assert!((native[0] - 16.0).abs() < 0.01);
        assert!((native[1] - 26.0).abs() < 0.01);
        assert!((native[2] - 400.0).abs() < 0.01);
        assert!((native[3] - 50.0).abs() < 0.01);
        let half = map_applet_rect([0.0, 0.0], [382.5, 251.5], 6, 6, 400, 50);
        assert!((half[0] - 3.0).abs() < 0.01);
        assert!((half[1] - 3.0).abs() < 0.01);
        assert!((half[2] - 200.0).abs() < 0.01);
        assert!((half[3] - 25.0).abs() < 0.01);
    }

    #[test]
    fn extremes_do_not_panic_and_negative_dims_clip() {
        let max = vec![CanvasOp::fill_rect(
            i32::MAX,
            0,
            1,
            1,
            pack_rgba(255, 0, 0, 255),
        )];
        assert!(rasterize(&max).is_none());
        let min_dim = vec![CanvasOp::fill_rect(
            10,
            10,
            i32::MIN,
            i32::MIN,
            pack_rgba(0, 255, 0, 255),
        )];
        let _ = rasterize(&min_dim);
        let neg = vec![CanvasOp::fill_rect(
            20,
            20,
            -10,
            -10,
            pack_rgba(0, 0, 255, 255),
        )];
        let raster = rasterize(&neg).unwrap();
        assert_eq!(raster.x, 10);
        assert_eq!(raster.y, 10);
        let off_text = vec![CanvasOp::fill_text(
            "hi",
            i32::MAX,
            i32::MIN,
            pack_rgba(255, 255, 255, 255),
            12,
            true,
        )];
        assert!(rasterize(&off_text).is_none());
        reset();
        fill_rect(f64::NAN, 0.0, 10.0, 10.0);
        fill_rect(0.0, f64::INFINITY, 10.0, 10.0);
        fill_text("x", f64::NEG_INFINITY, 10.0);
        let taken = take();
        assert!(taken.ops.is_empty());
        reset();
        let err = measure_text(&"x".repeat(MAX_PAINT_TEXT + 1)).unwrap_err();
        assert!(err.contains("measureText"));
        onpaint_done(0, None);
        let composed = compose_paint(None);
        assert!(composed.canvas.is_empty());
        assert!(composed.lines.iter().any(|l| l.contains("canvas:")));
        reset();
        onpaint_done(0, None);
        fill_rect(6.0, 6.0, 10.0, 10.0);
        let recovered = compose_paint(None);
        assert_eq!(recovered.canvas.len(), 1);
    }

    #[test]
    fn compose_keeps_user_onpaint_title() {
        reset();
        onpaint_done(0, None);
        fill_rect(6.0, 6.0, 8.0, 8.0);
        let user = crate::shim::ScriptPaint {
            title: Some("onPaint".into()),
            accent: Some("#ff5555".into()),
            lines: vec!["Paint.end was not called".into()],
            buttons: Vec::new(),
            generation: 0,
            canvas: Vec::new(),
            ..Default::default()
        };
        let composed = compose_paint(Some(user));
        assert_eq!(composed.title.as_deref(), Some("onPaint"));
        assert_eq!(composed.accent.as_deref(), Some("#ff5555"));
        assert_eq!(composed.lines[0], "Paint.end was not called");
        assert_eq!(composed.canvas.len(), 1);
    }

    #[test]
    fn save_restore_style_not_path() {
        reset();
        set_style("fillStyle", "#ff0000");
        begin_path();
        move_to(1.0, 1.0);
        line_to(10.0, 1.0);
        save();
        set_style("fillStyle", "#00ff00");
        begin_path();
        move_to(2.0, 2.0);
        restore();
        fill();
        let taken = take();
        match &taken.ops[0] {
            CanvasOp::FillPath { color, segs, .. } => {
                assert_eq!(*color, pack_rgba(255, 0, 0, 255));
                assert!(
                    matches!(segs[0], PathSeg::MoveTo { x, y } if (x - 2.0).abs() < 1e-5 && (y - 2.0).abs() < 1e-5),
                    "restore must not reset the working path: {segs:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fill_leaves_path_for_stroke() {
        reset();
        set_style("fillStyle", "#ffffff");
        set_style("strokeStyle", "#000000");
        set_number("lineWidth", 1.0);
        begin_path();
        move_to(10.0, 10.0);
        line_to(20.0, 10.0);
        line_to(20.0, 20.0);
        close_path();
        fill();
        stroke();
        let taken = take();
        assert_eq!(taken.ops.len(), 2);
        assert!(matches!(taken.ops[0], CanvasOp::FillPath { .. }));
        assert!(matches!(taken.ops[1], CanvasOp::StrokePath { .. }));
    }

    #[test]
    fn invalid_line_width_and_shadow_blur_keep_prior() {
        reset();
        set_number("lineWidth", 1.5);
        set_number("lineWidth", 0.0);
        set_number("lineWidth", -2.0);
        set_number("lineWidth", f64::NAN);
        set_number("shadowBlur", 10.0);
        set_number("shadowBlur", -1.0);
        set_number("shadowBlur", f64::INFINITY);
        set_style("strokeStyle", "#000000");
        begin_path();
        move_to(0.0, 0.0);
        line_to(10.0, 0.0);
        stroke();
        fill_rect(0.0, 0.0, 1.0, 1.0);
        let taken = take();
        match &taken.ops[0] {
            CanvasOp::StrokePath { line_width, .. } => {
                assert!((line_width - 1.5).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        match &taken.ops[1] {
            CanvasOp::FillRect { extras, .. } => {
                assert!((extras.shadow.blur - 10.0).abs() < 1e-6, "{extras:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn gradient_snapshot_does_not_mutate_after_draw() {
        reset();
        let id = create_linear(0.0, 0.0, 10.0, 0.0).unwrap();
        add_color_stop(id, 0.0, "#ff0000").unwrap();
        add_color_stop(id, 1.0, "#0000ff").unwrap();
        set_fill_gradient(id);
        begin_path();
        move_to(0.0, 0.0);
        line_to(10.0, 0.0);
        line_to(10.0, 10.0);
        close_path();
        fill();
        add_color_stop(id, 0.5, "#00ff00").unwrap();
        let taken = take();
        match &taken.ops[0] {
            CanvasOp::FillPath { extras, .. } => match &extras.fill {
                FillPaint::Linear { stops, .. } => {
                    assert_eq!(
                        stops.len(),
                        2,
                        "later addColorStop must not mutate the draw"
                    );
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn restore_restores_clip() {
        reset();
        begin_path();
        move_to(0.0, 0.0);
        line_to(20.0, 0.0);
        line_to(20.0, 20.0);
        line_to(0.0, 20.0);
        close_path();
        save();
        clip();
        fill_rect(0.0, 0.0, 5.0, 5.0);
        restore();
        fill_rect(30.0, 30.0, 5.0, 5.0);
        let taken = take();
        match &taken.ops[0] {
            CanvasOp::FillRect { extras, .. } => assert_eq!(extras.clips.len(), 1),
            other => panic!("{other:?}"),
        }
        match &taken.ops[1] {
            CanvasOp::FillRect { extras, .. } => assert!(extras.clips.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn shadow_darkens_pixels_below_the_shape() {
        reset();
        set_style("fillStyle", "#ffffff");
        set_style("shadowColor", "rgba(0, 0, 0, 0.8)");
        set_number("shadowBlur", 0.0);
        set_number("shadowOffsetY", 4.0);
        fill_rect(20.0, 20.0, 8.0, 8.0);
        let taken = take();
        let raster = rasterize(&taken.ops).expect("raster");
        let at = |x: i32, y: i32| -> u8 {
            let col = (x - raster.x) as usize;
            let row = (y - raster.y) as usize;
            raster.rgba[(row * raster.w as usize + col) * 4 + 3]
        };
        assert!(at(24, 24) > 200, "shape itself is opaque");
        assert!(
            at(24, 28) > 80,
            "shadow offset below the rect must be visible"
        );
    }

    fn line_path(n: usize) {
        begin_path();
        move_to(0.0, 0.0);
        for i in 1..n {
            line_to(i as f64, 0.0);
        }
    }

    #[test]
    fn path_only_commands_cannot_grow_past_caps() {
        reset();
        line_path(MAX_PATH_SEGS_PER_OP);
        fill();
        let taken = take();
        assert!(!taken.overflow);
        match &taken.ops[0] {
            CanvasOp::FillPath { segs, .. } => assert_eq!(segs.len(), MAX_PATH_SEGS_PER_OP),
            other => panic!("{other:?}"),
        }

        reset();
        line_path(MAX_PATH_SEGS_PER_OP);
        line_to(400.0, 1.0);
        close_path();
        let taken = take();
        assert!(taken.overflow);
        assert_eq!(taken.fail, Some("canvas: exceeded path segments"));
        assert!(taken.ops.is_empty());

        reset();
        line_path(MAX_PATH_SEGS_PER_OP);
        arc(10.0, 10.0, 4.0, 0.0, std::f64::consts::TAU, false)
            .expect("finite non-negative radius");
        let taken = take();
        assert!(taken.overflow);
        assert_eq!(taken.fail, Some("canvas: exceeded path segments"));
        assert!(taken.ops.is_empty());
    }

    #[test]
    fn empty_clip_cannot_bypass_clip_cap() {
        reset();
        for _ in 0..MAX_CLIP_PATHS {
            begin_path();
            clip();
        }
        fill_rect(0.0, 0.0, 2.0, 2.0);
        let taken = take();
        assert!(!taken.overflow);
        match &taken.ops[0] {
            CanvasOp::FillRect { extras, .. } => assert_eq!(extras.clips.len(), MAX_CLIP_PATHS),
            other => panic!("{other:?}"),
        }

        reset();
        for _ in 0..(MAX_CLIP_PATHS + 3) {
            begin_path();
            clip();
        }
        fill_rect(0.0, 0.0, 2.0, 2.0);
        let taken = take();
        assert!(taken.overflow);
        assert_eq!(taken.fail, Some("canvas: exceeded clip paths"));
        assert!(taken.ops.is_empty());
    }

    #[test]
    fn clipped_fill_rects_charge_emitted_clip_segs() {
        reset();
        line_path(MAX_PATH_SEGS_PER_OP);
        clip();
        let per = MAX_PATH_SEGS_PER_OP;
        let ok = MAX_PATH_SEGS_PER_FRAME / per;
        for _ in 0..ok {
            fill_rect(0.0, 0.0, 1.0, 1.0);
        }
        let taken = take();
        assert!(!taken.overflow, "fail={:?}", taken.fail);
        assert_eq!(taken.ops.len(), ok);

        reset();
        line_path(MAX_PATH_SEGS_PER_OP);
        clip();
        for _ in 0..(ok + 1) {
            fill_rect(0.0, 0.0, 1.0, 1.0);
        }
        let taken = take();
        assert!(taken.overflow);
        assert_eq!(taken.fail, Some("canvas: exceeded path segments"));
        assert_eq!(taken.ops.len(), ok);
    }

    #[test]
    fn huge_finite_path_extent_does_not_panic() {
        let segs = vec![
            PathSeg::MoveTo {
                x: -f32::MAX,
                y: -f32::MAX,
            },
            PathSeg::LineTo {
                x: f32::MAX,
                y: f32::MAX,
            },
        ];
        let op = CanvasOp::FillPath {
            segs,
            color: pack_rgba(255, 0, 0, 255),
            extras: DrawExtras::default(),
        };
        let dirty = dirty_bounds(std::slice::from_ref(&op));
        assert!(dirty.is_some());
        let _ = rasterize(&[op]);

        let extras = DrawExtras {
            shadow: Shadow {
                color: pack_rgba(0, 0, 0, 200),
                blur: MAX_SHADOW_BLUR,
                offset_x: f32::MAX,
                offset_y: -f32::MAX,
            },
            ..Default::default()
        };
        let shadowed = CanvasOp::FillRect {
            x: 20,
            y: 20,
            w: 8,
            h: 8,
            color: pack_rgba(255, 255, 255, 255),
            extras,
        };
        let _ = dirty_bounds(std::slice::from_ref(&shadowed));
        let _ = rasterize(&[shadowed]);
    }

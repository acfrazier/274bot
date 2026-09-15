//! Dirty-rect raster: tiny-skia paths/clips/linear gradients, native two-circle
//! radial evaluation, bounded box-blur shadows, ab_glyph text.

use ab_glyph::{point, Font, ScaleFont};
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint,
    Pixmap, Point, PremultipliedColorU8, Rect, SpreadMode, Stroke, Transform,
};

use super::geom::{
    conical_t, path_bounds, sample_stops, segs_to_skia, ClipPath, DrawExtras, FillPaint,
    LineJoinKind, PathSeg, Shadow, TextAlign, TextBaseline, MAX_SHADOW_BLUR,
};
use super::{
    clip_to_applet, font_for, measure_with, normalize_rect, unpack_rgba, CanvasOp, DirtyRect,
    Raster, APPLET_H, APPLET_W, MAX_CANVAS_OPS, MAX_FONT_PX, MAX_PAINT_TEXT,
};

impl DirtyRect {
    pub(crate) fn union(self, other: DirtyRect) -> DirtyRect {
        let x1 = (self.x as i64).min(other.x as i64);
        let y1 = (self.y as i64).min(other.y as i64);
        let x2 = (self.x as i64 + self.w as i64).max(other.x as i64 + other.w as i64);
        let y2 = (self.y as i64 + self.h as i64).max(other.y as i64 + other.h as i64);
        DirtyRect {
            x: x1 as i32,
            y: y1 as i32,
            w: (x2 - x1) as i32,
            h: (y2 - y1) as i32,
        }
    }

    pub(crate) fn clip_applet(self) -> Option<DirtyRect> {
        clip_to_applet(self.x as i64, self.y as i64, self.w as i64, self.h as i64)
    }
}

fn fbounds_to_dirty(minx: f32, miny: f32, maxx: f32, maxy: f32) -> Option<DirtyRect> {
    if !minx.is_finite() || !miny.is_finite() || !maxx.is_finite() || !maxy.is_finite() {
        return None;
    }
    if maxx <= minx || maxy <= miny {
        return None;
    }
    // Clamp to applet before floor/ceil/i64 so huge finite extents cannot
    // overflow `ceil as i64 - floor as i64` (e.g. ±f32::MAX).
    let aw = APPLET_W as f32;
    let ah = APPLET_H as f32;
    let minx = minx.clamp(0.0, aw);
    let miny = miny.clamp(0.0, ah);
    let maxx = maxx.clamp(0.0, aw);
    let maxy = maxy.clamp(0.0, ah);
    if maxx <= minx || maxy <= miny {
        return None;
    }
    let x = minx.floor() as i64;
    let y = miny.floor() as i64;
    let w = (maxx.ceil() as i64).saturating_sub(x);
    let h = (maxy.ceil() as i64).saturating_sub(y);
    clip_to_applet(x, y, w, h)
}

fn expand_bounds(
    minx: f32,
    miny: f32,
    maxx: f32,
    maxy: f32,
    pad: f32,
    shadow: Shadow,
) -> Option<DirtyRect> {
    if !pad.is_finite() {
        return None;
    }
    let pad = pad.max(0.0);
    let mut x0 = minx - pad;
    let mut y0 = miny - pad;
    let mut x1 = maxx + pad;
    let mut y1 = maxy + pad;
    if shadow.active() {
        let r = 3.0 * shadow.blur.max(0.0).min(MAX_SHADOW_BLUR);
        let ox = shadow.offset_x;
        let oy = shadow.offset_y;
        if ox.is_finite() && oy.is_finite() && r.is_finite() {
            x0 = x0.min(x0 + ox) - r;
            y0 = y0.min(y0 + oy) - r;
            x1 = x1.max(x1 + ox) + r;
            y1 = y1.max(y1 + oy) + r;
        }
    }
    fbounds_to_dirty(x0, y0, x1, y1)
}

fn clip_intersect_dirty(dirty: DirtyRect, clips: &[ClipPath]) -> Option<DirtyRect> {
    let mut acc = dirty;
    for clip in clips {
        let (minx, miny, maxx, maxy) = path_bounds(&clip.segs)?;
        let c = fbounds_to_dirty(minx, miny, maxx, maxy)?;
        let x1 = acc.x.max(c.x);
        let y1 = acc.y.max(c.y);
        let x2 = (acc.x + acc.w).min(c.x + c.w);
        let y2 = (acc.y + acc.h).min(c.y + c.h);
        if x2 <= x1 || y2 <= y1 {
            return None;
        }
        acc = DirtyRect {
            x: x1,
            y: y1,
            w: x2 - x1,
            h: y2 - y1,
        };
    }
    acc.clip_applet()
}

fn stroke_pad(width: f32, join: LineJoinKind) -> f32 {
    let half = width.max(0.0) / 2.0;
    match join {
        LineJoinKind::Miter => (half * 10.0).max(half + 1.0),
        LineJoinKind::Round | LineJoinKind::Bevel => half + 1.0,
    }
}

pub(crate) fn text_pen(
    x: i32,
    y: i32,
    font_px: u16,
    mono: bool,
    text: &str,
    align: TextAlign,
    baseline: TextBaseline,
) -> (f32, f32) {
    let width = measure_with(font_px, mono, text) as f32;
    let mut px = x as f32;
    let mut py = y as f32;
    match align {
        TextAlign::Left => {}
        TextAlign::Center => px -= width / 2.0,
        TextAlign::Right => px -= width,
    }
    let font = font_for(mono);
    let scale = ab_glyph::PxScale::from(font_px.max(1) as f32);
    let scaled = font.as_scaled(scale);
    let ascent = scaled.ascent();
    let descent = scaled.descent();
    match baseline {
        TextBaseline::Alphabetic => {}
        TextBaseline::Top => py += ascent,
        TextBaseline::Middle => py += (ascent + descent) / 2.0,
        TextBaseline::Bottom => py += descent,
    }
    (px, py)
}

fn text_dirty(
    x: i32,
    y: i32,
    font_px: u16,
    mono: bool,
    text: &str,
    align: TextAlign,
    baseline: TextBaseline,
    extras: &DrawExtras,
) -> Option<DirtyRect> {
    if font_px == 0 || font_px > MAX_FONT_PX {
        return None;
    }
    let (pen_x, pen_y) = text_pen(x, y, font_px, mono, text, align, baseline);
    let font = font_for(mono);
    let scale = ab_glyph::PxScale::from(font_px as f32);
    let scaled = font.as_scaled(scale);
    let width = measure_with(font_px, mono, text).ceil() as f32;
    let ascent = scaled.ascent();
    let descent = scaled.descent().abs();
    let pad = font_px as f32;
    let dirty = expand_bounds(
        pen_x - pad,
        pen_y - ascent - pad,
        pen_x + width + pad,
        pen_y + descent + pad,
        0.0,
        extras.shadow,
    )?;
    clip_intersect_dirty(dirty, &extras.clips)
}

fn op_dirty(op: &CanvasOp) -> Option<DirtyRect> {
    match op {
        CanvasOp::FillRect {
            x, y, w, h, extras, ..
        } => {
            let (x, y, w, h) = normalize_rect(*x, *y, *w, *h)?;
            let dirty = expand_bounds(
                x as f32,
                y as f32,
                (x + w) as f32,
                (y + h) as f32,
                0.0,
                extras.shadow,
            )?;
            clip_intersect_dirty(dirty, &extras.clips)
        }
        CanvasOp::FillText {
            text,
            x,
            y,
            font_px,
            mono,
            align,
            baseline,
            extras,
            ..
        } => text_dirty(*x, *y, *font_px, *mono, text, *align, *baseline, extras),
        CanvasOp::FillPath { segs, extras, .. } => {
            let (minx, miny, maxx, maxy) = path_bounds(segs)?;
            let dirty = expand_bounds(minx, miny, maxx, maxy, 1.0, extras.shadow)?;
            clip_intersect_dirty(dirty, &extras.clips)
        }
        CanvasOp::StrokePath {
            segs,
            line_width,
            line_join,
            extras,
            ..
        } => {
            let (minx, miny, maxx, maxy) = path_bounds(segs)?;
            let pad = stroke_pad(*line_width, *line_join);
            let dirty = expand_bounds(minx, miny, maxx, maxy, pad, extras.shadow)?;
            clip_intersect_dirty(dirty, &extras.clips)
        }
    }
}

/// Union of op bounds, clipped to the applet. None if nothing is visible.
pub fn dirty_bounds(ops: &[CanvasOp]) -> Option<DirtyRect> {
    let mut acc: Option<DirtyRect> = None;
    for op in ops {
        if let Some(d) = op_dirty(op).and_then(DirtyRect::clip_applet) {
            acc = Some(match acc {
                Some(a) => a.union(d),
                None => d,
            });
        }
    }
    acc.and_then(DirtyRect::clip_applet)
}

fn pixmap_ts(dirty: DirtyRect) -> Transform {
    Transform::from_translate(-dirty.x as f32, -dirty.y as f32)
}

fn clip_mask(clips: &[ClipPath], dirty: DirtyRect) -> Option<Mask> {
    if clips.is_empty() {
        return None;
    }
    let mut mask = Mask::new(dirty.w as u32, dirty.h as u32)?;
    let ts = pixmap_ts(dirty);
    let mut first = true;
    for clip in clips {
        let Some(path) = segs_to_skia(&clip.segs) else {
            mask.clear();
            return Some(mask);
        };
        if first {
            mask.fill_path(&path, FillRule::Winding, true, ts);
            first = false;
        } else {
            mask.intersect_path(&path, FillRule::Winding, true, ts);
        }
    }
    Some(mask)
}

fn solid_paint(color: u32) -> Paint<'static> {
    let [r, g, b, a] = unpack_rgba(color);
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn linear_paint(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    stops: &[super::geom::GradStop],
) -> Option<Paint<'static>> {
    if stops.is_empty() {
        return None;
    }
    let gs: Vec<GradientStop> = stops
        .iter()
        .map(|s| {
            let [r, g, b, a] = unpack_rgba(s.color);
            GradientStop::new(s.offset.clamp(0.0, 1.0), Color::from_rgba8(r, g, b, a))
        })
        .collect();
    let shader = LinearGradient::new(
        Point::from_xy(x0, y0),
        Point::from_xy(x1, y1),
        gs,
        SpreadMode::Pad,
        Transform::identity(),
    )?;
    Some(Paint {
        shader,
        blend_mode: BlendMode::SourceOver,
        anti_alias: true,
        force_hq_pipeline: false,
    })
}

fn stroke_style(width: f32, join: LineJoinKind) -> Stroke {
    Stroke {
        width: width.max(0.0),
        miter_limit: 10.0,
        line_cap: LineCap::Butt,
        line_join: match join {
            LineJoinKind::Round => LineJoin::Round,
            LineJoinKind::Bevel => LineJoin::Bevel,
            LineJoinKind::Miter => LineJoin::Miter,
        },
        dash: None,
    }
}

fn blend_premul(pixmap: &mut Pixmap, i: usize, sr: f32, sg: f32, sb: f32, sa: f32) {
    if sa <= 0.0 {
        return;
    }
    let px = pixmap.pixels_mut()[i];
    let da = px.alpha() as f32 / 255.0;
    let inv = 1.0 - sa;
    let out_a = sa + da * inv;
    let out_r = sr * sa + (px.red() as f32 / 255.0) * inv;
    let out_g = sg * sa + (px.green() as f32 / 255.0) * inv;
    let out_b = sb * sa + (px.blue() as f32 / 255.0) * inv;
    let a8 = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    let r8 = (out_r * 255.0).round().clamp(0.0, 255.0) as u8;
    let g8 = (out_g * 255.0).round().clamp(0.0, 255.0) as u8;
    let b8 = (out_b * 255.0).round().clamp(0.0, 255.0) as u8;
    if let Some(p) = PremultipliedColorU8::from_rgba(r8.min(a8), g8.min(a8), b8.min(a8), a8) {
        pixmap.pixels_mut()[i] = p;
    }
}

fn fill_radial_coverage(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    coverage: &Mask,
    clip: Option<&Mask>,
    x0: f32,
    y0: f32,
    r0: f32,
    x1: f32,
    y1: f32,
    r1: f32,
    stops: &[super::geom::GradStop],
) {
    let w = dirty.w as usize;
    let h = dirty.h as usize;
    let cov = coverage.data();
    let clip_data = clip.map(Mask::data);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut a = cov[i];
            if let Some(c) = clip_data {
                a = ((a as u16 * c[i] as u16) / 255) as u8;
            }
            if a == 0 {
                continue;
            }
            let px = dirty.x as f32 + x as f32 + 0.5;
            let py = dirty.y as f32 + y as f32 + 0.5;
            let t = conical_t(px, py, x0, y0, r0, x1, y1, r1);
            let color = sample_stops(stops, t);
            let [sr, sg, sb, sa] = unpack_rgba(color);
            let src_a = (sa as f32 / 255.0) * (a as f32 / 255.0);
            blend_premul(
                pixmap,
                i,
                sr as f32 / 255.0,
                sg as f32 / 255.0,
                sb as f32 / 255.0,
                src_a,
            );
        }
    }
}

fn coverage_mask_for_path(path: &tiny_skia::Path, dirty: DirtyRect) -> Option<Mask> {
    let mut mask = Mask::new(dirty.w as u32, dirty.h as u32)?;
    mask.fill_path(path, FillRule::Winding, true, pixmap_ts(dirty));
    Some(mask)
}

fn coverage_mask_for_rect(x: f32, y: f32, w: f32, h: f32, dirty: DirtyRect) -> Option<Mask> {
    let rect = Rect::from_xywh(x, y, w, h)?;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.push_rect(rect);
    let path = pb.finish()?;
    coverage_mask_for_path(&path, dirty)
}

fn box_blur(buf: &mut [u8], w: usize, h: usize, radius: i32) {
    if radius <= 0 || w == 0 || h == 0 {
        return;
    }
    let r = radius as usize;
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        // horizontal
        for y in 0..h {
            let row = y * w;
            let mut sum: u32 = 0;
            let mut count: u32 = 0;
            for x in 0..=r.min(w.saturating_sub(1)) {
                sum += buf[row + x] as u32;
                count += 1;
            }
            for x in 0..w {
                tmp[row + x] = (sum / count) as u8;
                if x + r + 1 < w {
                    sum += buf[row + x + r + 1] as u32;
                    count += 1;
                }
                if x >= r {
                    sum -= buf[row + x - r] as u32;
                    count -= 1;
                }
            }
        }
        // vertical
        for x in 0..w {
            let mut sum: u32 = 0;
            let mut count: u32 = 0;
            for y in 0..=r.min(h.saturating_sub(1)) {
                sum += tmp[y * w + x] as u32;
                count += 1;
            }
            for y in 0..h {
                buf[y * w + x] = (sum / count) as u8;
                if y + r + 1 < h {
                    sum += tmp[(y + r + 1) * w + x] as u32;
                    count += 1;
                }
                if y >= r {
                    sum -= tmp[(y - r) * w + x] as u32;
                    count -= 1;
                }
            }
        }
    }
}

fn blit_shadow(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    alpha: &[u8],
    shadow: Shadow,
    clip: Option<&Mask>,
) {
    let [sr, sg, sb, sa] = unpack_rgba(shadow.color);
    if sa == 0 {
        return;
    }
    let w = dirty.w as usize;
    let h = dirty.h as usize;
    let ox = shadow.offset_x.round() as i64;
    let oy = shadow.offset_y.round() as i64;
    let clip_data = clip.map(Mask::data);
    for y in 0..h {
        for x in 0..w {
            let a = alpha[y * w + x];
            if a == 0 {
                continue;
            }
            let dx = (x as i64).saturating_add(ox);
            let dy = (y as i64).saturating_add(oy);
            if dx < 0 || dy < 0 || dx >= i64::from(dirty.w) || dy >= i64::from(dirty.h) {
                continue;
            }
            let di = dy as usize * w + dx as usize;
            let mut cov = a;
            if let Some(c) = clip_data {
                cov = ((cov as u16 * c[di] as u16) / 255) as u8;
            }
            if cov == 0 {
                continue;
            }
            let src_a = (sa as f32 / 255.0) * (cov as f32 / 255.0);
            blend_premul(
                pixmap,
                di,
                sr as f32 / 255.0,
                sg as f32 / 255.0,
                sb as f32 / 255.0,
                src_a,
            );
        }
    }
}

fn draw_coverage_shadow(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    coverage: &Mask,
    extras: &DrawExtras,
    clip: Option<&Mask>,
) {
    if !extras.shadow.active() {
        return;
    }
    let mut alpha = coverage.data().to_vec();
    let radius = extras.shadow.blur.round().clamp(0.0, MAX_SHADOW_BLUR) as i32;
    box_blur(&mut alpha, dirty.w as usize, dirty.h as usize, radius);
    blit_shadow(pixmap, dirty, &alpha, extras.shadow, clip);
}

fn paint_for_extras<'a>(
    color: u32,
    extras: &'a DrawExtras,
) -> Option<(Option<Paint<'static>>, Option<&'a FillPaint>)> {
    match &extras.fill {
        FillPaint::Solid => Some((Some(solid_paint(color)), None)),
        FillPaint::Linear {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => match linear_paint(*x0, *y0, *x1, *y1, stops) {
            Some(p) => Some((Some(p), None)),
            None => None, // zero-length linear: paint nothing
        },
        FillPaint::Radial { .. } => Some((None, Some(&extras.fill))),
    }
}

fn fill_path_op(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    segs: &[PathSeg],
    color: u32,
    extras: &DrawExtras,
    clip: Option<&Mask>,
) {
    let Some(path) = segs_to_skia(segs) else {
        return;
    };
    let Some(coverage) = coverage_mask_for_path(&path, dirty) else {
        return;
    };
    draw_coverage_shadow(pixmap, dirty, &coverage, extras, clip);
    match paint_for_extras(color, extras) {
        None => {}
        Some((Some(paint), _)) => {
            pixmap.fill_path(&path, &paint, FillRule::Winding, pixmap_ts(dirty), clip);
        }
        Some((
            None,
            Some(FillPaint::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            }),
        )) => {
            fill_radial_coverage(
                pixmap, dirty, &coverage, clip, *x0, *y0, *r0, *x1, *y1, *r1, stops,
            );
        }
        _ => {}
    }
}

fn stroke_path_op(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    segs: &[PathSeg],
    color: u32,
    width: f32,
    join: LineJoinKind,
    extras: &DrawExtras,
    clip: Option<&Mask>,
) {
    let Some(path) = segs_to_skia(segs) else {
        return;
    };
    let stroke = stroke_style(width, join);
    // Approximate stroke coverage with a filled stroke path for the shadow.
    if extras.shadow.active() {
        if let Some(stroked) = path.stroke(&stroke, 1.0) {
            if let Some(coverage) = coverage_mask_for_path(&stroked, dirty) {
                draw_coverage_shadow(pixmap, dirty, &coverage, extras, clip);
            }
        }
    }
    let paint = solid_paint(color);
    pixmap.stroke_path(&path, &paint, &stroke, pixmap_ts(dirty), clip);
}

fn fill_rect_op(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    color: u32,
    extras: &DrawExtras,
    clip: Option<&Mask>,
) {
    let Some(rect) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) else {
        return;
    };
    if extras.shadow.active() {
        if let Some(coverage) =
            coverage_mask_for_rect(x as f32, y as f32, w as f32, h as f32, dirty)
        {
            draw_coverage_shadow(pixmap, dirty, &coverage, extras, clip);
        }
    }
    match paint_for_extras(color, extras) {
        None => {}
        Some((Some(paint), _)) => {
            pixmap.fill_rect(rect, &paint, pixmap_ts(dirty), clip);
        }
        Some((
            None,
            Some(FillPaint::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            }),
        )) => {
            if let Some(coverage) =
                coverage_mask_for_rect(x as f32, y as f32, w as f32, h as f32, dirty)
            {
                fill_radial_coverage(
                    pixmap, dirty, &coverage, clip, *x0, *y0, *r0, *x1, *y1, *r1, stops,
                );
            }
        }
        _ => {}
    }
}

fn fill_text_op(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    text: &str,
    x: i32,
    y: i32,
    color: u32,
    font_px: u16,
    mono: bool,
    align: TextAlign,
    baseline: TextBaseline,
    extras: &DrawExtras,
    clip: Option<&Mask>,
) {
    if font_px == 0 || font_px > MAX_FONT_PX || text.len() > MAX_PAINT_TEXT {
        return;
    }
    let (pen_x, pen_y) = text_pen(x, y, font_px, mono, text, align, baseline);
    if extras.shadow.active() {
        // Glyph coverage into a mask, then blur.
        if let Some(mut coverage) = Mask::new(dirty.w as u32, dirty.h as u32) {
            draw_glyphs_coverage(&mut coverage, dirty, text, pen_x, pen_y, font_px, mono);
            draw_coverage_shadow(pixmap, dirty, &coverage, extras, clip);
        }
    }
    draw_glyphs_color(
        pixmap, dirty, text, pen_x, pen_y, color, font_px, mono, clip,
    );
}

fn draw_glyphs_coverage(
    mask: &mut Mask,
    dirty: DirtyRect,
    text: &str,
    pen_x: f32,
    pen_y: f32,
    font_px: u16,
    mono: bool,
) {
    let font = font_for(mono);
    let scale = ab_glyph::PxScale::from(font_px as f32);
    let scaled = font.as_scaled(scale);
    let mut pen = pen_x;
    let data = mask.data_mut();
    let w = dirty.w as usize;
    let h = dirty.h as usize;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(pen, pen_y));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                if cov <= 0.0 {
                    return;
                }
                let px = bounds.min.x.round() as i32 + gx as i32 - dirty.x;
                let py = bounds.min.y.round() as i32 + gy as i32 - dirty.y;
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    return;
                }
                let i = py as usize * w + px as usize;
                let add = (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
                data[i] = data[i].saturating_add(add);
            });
        }
        pen += scaled.h_advance(gid);
    }
}

fn draw_glyphs_color(
    pixmap: &mut Pixmap,
    dirty: DirtyRect,
    text: &str,
    pen_x: f32,
    pen_y: f32,
    color: u32,
    font_px: u16,
    mono: bool,
    clip: Option<&Mask>,
) {
    let font = font_for(mono);
    let scale = ab_glyph::PxScale::from(font_px as f32);
    let scaled = font.as_scaled(scale);
    let [sr, sg, sb, sa] = unpack_rgba(color);
    let w = dirty.w as usize;
    let h = dirty.h as usize;
    let clip_data = clip.map(Mask::data);
    let mut pen = pen_x;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(pen, pen_y));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                if cov <= 0.0 {
                    return;
                }
                let px = bounds.min.x.round() as i32 + gx as i32 - dirty.x;
                let py = bounds.min.y.round() as i32 + gy as i32 - dirty.y;
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    return;
                }
                let i = py as usize * w + px as usize;
                let mut src_a = (sa as f32 / 255.0) * cov.clamp(0.0, 1.0);
                if let Some(c) = clip_data {
                    src_a *= c[i] as f32 / 255.0;
                }
                blend_premul(
                    pixmap,
                    i,
                    sr as f32 / 255.0,
                    sg as f32 / 255.0,
                    sb as f32 / 255.0,
                    src_a,
                );
            });
        }
        pen += scaled.h_advance(gid);
    }
}

fn premul_to_straight(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = vec![0u8; pixmap.width() as usize * pixmap.height() as usize * 4];
    for (i, p) in pixmap.pixels().iter().enumerate() {
        let a = p.alpha();
        if a == 0 {
            continue;
        }
        let (r, g, b) = if a == 255 {
            (p.red(), p.green(), p.blue())
        } else {
            let aa = a as u16;
            (
                ((p.red() as u16 * 255) / aa) as u8,
                ((p.green() as u16 * 255) / aa) as u8,
                ((p.blue() as u16 * 255) / aa) as u8,
            )
        };
        let o = i * 4;
        out[o] = r;
        out[o + 1] = g;
        out[o + 2] = b;
        out[o + 3] = a;
    }
    out
}

/// Rasterize ops into a small transparent dirty pixmap. Same font as measure.
pub fn rasterize(ops: &[CanvasOp]) -> Option<Raster> {
    if ops.len() > MAX_CANVAS_OPS {
        return None;
    }
    let dirty = dirty_bounds(ops)?;
    let w = dirty.w as u32;
    let h = dirty.h as u32;
    if w == 0 || h == 0 {
        return None;
    }
    let max_bytes = (APPLET_W as usize) * (APPLET_H as usize) * 4;
    let nbytes = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if nbytes == 0 || nbytes > max_bytes {
        return None;
    }
    let mut pixmap = Pixmap::new(w, h)?;
    for op in ops {
        let clip = match op {
            CanvasOp::FillRect { extras, .. }
            | CanvasOp::FillText { extras, .. }
            | CanvasOp::FillPath { extras, .. }
            | CanvasOp::StrokePath { extras, .. } => clip_mask(&extras.clips, dirty),
        };
        match op {
            CanvasOp::FillRect {
                x,
                y,
                w,
                h,
                color,
                extras,
            } => {
                if let Some((x, y, w, h)) = normalize_rect(*x, *y, *w, *h) {
                    fill_rect_op(
                        &mut pixmap,
                        dirty,
                        x,
                        y,
                        w,
                        h,
                        *color,
                        extras,
                        clip.as_ref(),
                    );
                }
            }
            CanvasOp::FillText {
                text,
                x,
                y,
                color,
                font_px,
                mono,
                align,
                baseline,
                extras,
            } => {
                fill_text_op(
                    &mut pixmap,
                    dirty,
                    text,
                    *x,
                    *y,
                    *color,
                    *font_px,
                    *mono,
                    *align,
                    *baseline,
                    extras,
                    clip.as_ref(),
                );
            }
            CanvasOp::FillPath {
                segs,
                color,
                extras,
            } => {
                fill_path_op(&mut pixmap, dirty, segs, *color, extras, clip.as_ref());
            }
            CanvasOp::StrokePath {
                segs,
                color,
                line_width,
                line_join,
                extras,
            } => {
                stroke_path_op(
                    &mut pixmap,
                    dirty,
                    segs,
                    *color,
                    *line_width,
                    *line_join,
                    extras,
                    clip.as_ref(),
                );
            }
        }
    }
    Some(Raster {
        x: dirty.x,
        y: dirty.y,
        w,
        h,
        rgba: premul_to_straight(&pixmap),
    })
}

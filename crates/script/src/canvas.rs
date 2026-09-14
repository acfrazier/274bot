//! Isolate-thread Canvas recorder: parse, measure, rasterize, caps.
//!
//! No UI, no host observe-thread probes. The same font implementation is
//! used for `measureText` and `fillText` raster. JS only forwards
//! getters/setters/methods.

use std::cell::RefCell;
use std::sync::OnceLock;

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};

/// Client applet width in pixels (same as `panel::game_view::APPLET_W`).
pub const APPLET_W: i32 = 765;
/// Client applet height in pixels (same as `panel::game_view::APPLET_H`).
pub const APPLET_H: i32 = 503;

/// Max recorded canvas ops per onPaint call / paint frame.
pub const MAX_CANVAS_OPS: usize = 256;
/// Max UTF-8 bytes per fillText / measureText string.
pub const MAX_PAINT_TEXT: usize = 512;

const SANS_BYTES: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const MONO_BYTES: &[u8] = include_bytes!("../fonts/LiberationMono-Regular.ttf");

static SANS: OnceLock<FontRef<'static>> = OnceLock::new();
static MONO: OnceLock<FontRef<'static>> = OnceLock::new();

fn sans_font() -> &'static FontRef<'static> {
    SANS.get_or_init(|| FontRef::try_from_slice(SANS_BYTES).expect("Liberation Sans Regular"))
}

fn mono_font() -> &'static FontRef<'static> {
    MONO.get_or_init(|| FontRef::try_from_slice(MONO_BYTES).expect("Liberation Mono Regular"))
}

fn font_for(mono: bool) -> &'static FontRef<'static> {
    if mono {
        mono_font()
    } else {
        sans_font()
    }
}

/// One recorded draw op. Color is packed `0xRRGGBBAA`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op")]
pub enum CanvasOp {
    #[serde(rename = "fillRect")]
    FillRect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    #[serde(rename = "fillText")]
    FillText {
        text: String,
        x: i32,
        y: i32,
        color: u32,
        font_px: u16,
        mono: bool,
    },
}

/// Dirty-rect pixmap in applet space. Never a full 765×503 unless the ops
/// actually cover that much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Clone)]
struct Style {
    fill: u32,
    font_px: u16,
    mono: bool,
    font_css: String,
    fill_css: String,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: pack_rgba(0, 0, 0, 255),
            font_px: 10,
            mono: false,
            font_css: "10px sans-serif".into(),
            fill_css: "#000000".into(),
        }
    }
}

struct Recorder {
    style: Style,
    ops: Vec<CanvasOp>,
    overflow: bool,
}

impl Recorder {
    fn new() -> Self {
        Self {
            style: Style::default(),
            ops: Vec::new(),
            overflow: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn push(&mut self, op: CanvasOp) {
        if self.overflow {
            return;
        }
        if self.ops.len() >= MAX_CANVAS_OPS {
            self.overflow = true;
            return;
        }
        self.ops.push(op);
    }
}

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::new());
}

/// Start a new per-call recorder (HTML 2D defaults).
pub fn begin() {
    RECORDER.with(|r| r.borrow_mut().reset());
}

/// CSS `font` getter (last accepted string).
pub fn font() -> String {
    RECORDER.with(|r| r.borrow().style.font_css.clone())
}

/// CSS `fillStyle` getter (last accepted string).
pub fn fill_style() -> String {
    RECORDER.with(|r| r.borrow().style.fill_css.clone())
}

/// Set `font` or `fillStyle`. Unparseable assignments keep the previous
/// value (HTML). Unknown names are not invented.
pub fn set_style(prop: &str, value: &str) {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        match prop {
            "font" => {
                if let Some((px, mono)) = parse_font(value) {
                    rec.style.font_px = px;
                    rec.style.mono = mono;
                    rec.style.font_css = value.to_string();
                }
            }
            "fillStyle" => {
                if let Some(color) = parse_color(value) {
                    rec.style.fill = color;
                    rec.style.fill_css = value.to_string();
                }
            }
            _ => {}
        }
    });
}

pub fn fill_rect(x: f64, y: f64, w: f64, h: f64) {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        let color = rec.style.fill;
        rec.push(CanvasOp::FillRect {
            x: round_px(x),
            y: round_px(y),
            w: round_px(w),
            h: round_px(h),
            color,
        });
    });
}

pub fn fill_text(text: &str, x: f64, y: f64) {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if text.len() > MAX_PAINT_TEXT {
            rec.overflow = true;
            return;
        }
        let color = rec.style.fill;
        let font_px = rec.style.font_px;
        let mono = rec.style.mono;
        rec.push(CanvasOp::FillText {
            text: text.to_string(),
            x: round_px(x),
            y: round_px(y),
            color,
            font_px,
            mono,
        });
    });
}

/// Advance-sum width for the current font. Never a constant.
pub fn measure_text(text: &str) -> f64 {
    if text.len() > MAX_PAINT_TEXT {
        return 0.0;
    }
    RECORDER.with(|r| {
        let rec = r.borrow();
        measure_with(rec.style.font_px, rec.style.mono, text)
    })
}

/// Same metrics as [`measure_text`] for a concrete face/size (tests / panel).
pub fn measure_with(font_px: u16, mono: bool, text: &str) -> f64 {
    let font = font_for(mono);
    let scale = PxScale::from(font_px.max(1) as f32);
    let scaled = font.as_scaled(scale);
    let mut width = 0.0f32;
    for ch in text.chars() {
        width += scaled.h_advance(font.glyph_id(ch));
    }
    width as f64
}

pub struct Take {
    pub ops: Vec<CanvasOp>,
    pub overflow: bool,
}

/// Drain this call's ops. Overflow means the frame must drop canvas.
pub fn take() -> Take {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        Take {
            ops: std::mem::take(&mut rec.ops),
            overflow: rec.overflow,
        }
    })
}

fn round_px(v: f64) -> i32 {
    if !v.is_finite() {
        0
    } else {
        v.round() as i32
    }
}

pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a)
}

pub fn unpack_rgba(color: u32) -> [u8; 4] {
    [
        (color >> 24) as u8,
        (color >> 16) as u8,
        (color >> 8) as u8,
        color as u8,
    ]
}

pub fn parse_color(input: &str) -> Option<u32> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            3 => {
                let n = u32::from_str_radix(hex, 16).ok()?;
                let r = (((n >> 8) & 0xf) * 0x11) as u8;
                let g = (((n >> 4) & 0xf) * 0x11) as u8;
                let b = ((n & 0xf) * 0x11) as u8;
                Some(pack_rgba(r, g, b, 255))
            }
            6 => {
                let n = u32::from_str_radix(hex, 16).ok()?;
                Some((n << 8) | 0xff)
            }
            8 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        }
    } else if let Some(inner) = s.strip_prefix("rgba(").and_then(|t| t.strip_suffix(')')) {
        parse_rgb_inner(inner, true)
    } else if let Some(inner) = s.strip_prefix("rgb(").and_then(|t| t.strip_suffix(')')) {
        parse_rgb_inner(inner, false)
    } else {
        None
    }
}

fn parse_rgb_inner(inner: &str, with_alpha: bool) -> Option<u32> {
    let mut parts = inner.split(',').map(|p| p.trim());
    let r = parse_rgb_channel(parts.next()?)?;
    let g = parse_rgb_channel(parts.next()?)?;
    let b = parse_rgb_channel(parts.next()?)?;
    let a = if with_alpha {
        let a = parts.next()?.parse::<f64>().ok()?;
        if !a.is_finite() {
            return None;
        }
        (a.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        if parts.next().is_some() {
            return None;
        }
        255
    };
    if parts.next().is_some() {
        return None;
    }
    Some(pack_rgba(r, g, b, a))
}

fn parse_rgb_channel(s: &str) -> Option<u8> {
    let n: f64 = s.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some(n.round().clamp(0.0, 255.0) as u8)
}

/// Parse CSS font shorthand enough for `12px monospace` / `10px sans-serif`.
pub fn parse_font(input: &str) -> Option<(u16, bool)> {
    let s = input.trim();
    let px = s.find("px")?;
    let mut start = px;
    let bytes = s.as_bytes();
    while start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_digit() || c == b'.' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == px {
        return None;
    }
    let size: f64 = s[start..px].parse().ok()?;
    if !size.is_finite() || size <= 0.0 || size > 256.0 {
        return None;
    }
    let family = s[px + 2..]
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_ascii_lowercase();
    let mono = family == "monospace"
        || family == "courier"
        || family == "courier new"
        || family.contains("liberation mono");
    let sans = family == "sans-serif"
        || family == "sans"
        || family == "arial"
        || family == "helvetica"
        || family.contains("liberation sans");
    if !mono && !sans {
        return None;
    }
    Some((size.round() as u16, mono))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl DirtyRect {
    fn union(self, other: DirtyRect) -> DirtyRect {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.w).max(other.x + other.w);
        let y2 = (self.y + self.h).max(other.y + other.h);
        DirtyRect {
            x: x1,
            y: y1,
            w: x2 - x1,
            h: y2 - y1,
        }
    }

    fn clip_applet(self) -> Option<DirtyRect> {
        let x1 = self.x.max(0);
        let y1 = self.y.max(0);
        let x2 = (self.x + self.w).min(APPLET_W);
        let y2 = (self.y + self.h).min(APPLET_H);
        if x2 <= x1 || y2 <= y1 {
            None
        } else {
            Some(DirtyRect {
                x: x1,
                y: y1,
                w: x2 - x1,
                h: y2 - y1,
            })
        }
    }
}

fn normalize_rect(x: i32, y: i32, w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
    let (mut x, mut w) = (x, w);
    let (mut y, mut h) = (y, h);
    if w < 0 {
        x += w;
        w = -w;
    }
    if h < 0 {
        y += h;
        h = -h;
    }
    if w == 0 || h == 0 {
        None
    } else {
        Some((x, y, w, h))
    }
}

fn text_dirty(op_x: i32, op_y: i32, font_px: u16, mono: bool, text: &str) -> DirtyRect {
    let font = font_for(mono);
    let scale = PxScale::from(font_px.max(1) as f32);
    let scaled = font.as_scaled(scale);
    let width = measure_with(font_px, mono, text).ceil() as i32;
    let ascent = scaled.ascent().ceil() as i32;
    let descent = scaled.descent().abs().ceil() as i32;
    // Glyphs can overhang the advance box; pad by one em.
    let pad = font_px as i32;
    DirtyRect {
        x: op_x - pad,
        y: op_y - ascent - pad,
        w: width + pad * 2,
        h: ascent + descent + pad * 2,
    }
}

fn op_dirty(op: &CanvasOp) -> Option<DirtyRect> {
    match op {
        CanvasOp::FillRect { x, y, w, h, .. } => {
            let (x, y, w, h) = normalize_rect(*x, *y, *w, *h)?;
            Some(DirtyRect { x, y, w, h })
        }
        CanvasOp::FillText {
            text,
            x,
            y,
            font_px,
            mono,
            ..
        } => Some(text_dirty(*x, *y, *font_px, *mono, text)),
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
    let mut rgba = vec![0u8; nbytes];
    for op in ops {
        match op {
            CanvasOp::FillRect { x, y, w, h, color } => {
                if let Some((x, y, w, h)) = normalize_rect(*x, *y, *w, *h) {
                    fill_rect_into(&mut rgba, dirty, x, y, w, h, *color);
                }
            }
            CanvasOp::FillText {
                text,
                x,
                y,
                color,
                font_px,
                mono,
            } => {
                if text.len() > MAX_PAINT_TEXT {
                    continue;
                }
                fill_text_into(&mut rgba, dirty, text, *x, *y, *color, *font_px, *mono);
            }
        }
    }
    Some(Raster {
        x: dirty.x,
        y: dirty.y,
        w,
        h,
        rgba,
    })
}

fn fill_rect_into(rgba: &mut [u8], dirty: DirtyRect, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x1 = x.max(dirty.x).max(0);
    let y1 = y.max(dirty.y).max(0);
    let x2 = (x + w).min(dirty.x + dirty.w).min(APPLET_W);
    let y2 = (y + h).min(dirty.y + dirty.h).min(APPLET_H);
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    for py in y1..y2 {
        for px in x1..x2 {
            blend_pixel(rgba, dirty, px, py, color, 1.0);
        }
    }
}

fn fill_text_into(
    rgba: &mut [u8],
    dirty: DirtyRect,
    text: &str,
    x: i32,
    y: i32,
    color: u32,
    font_px: u16,
    mono: bool,
) {
    let font = font_for(mono);
    let scale = PxScale::from(font_px.max(1) as f32);
    let scaled = font.as_scaled(scale);
    let mut pen = x as f32;
    let baseline = y as f32;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(pen, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                if cov <= 0.0 {
                    return;
                }
                let px = bounds.min.x.round() as i32 + gx as i32;
                let py = bounds.min.y.round() as i32 + gy as i32;
                blend_pixel(rgba, dirty, px, py, color, cov);
            });
        }
        pen += scaled.h_advance(gid);
    }
}

fn blend_pixel(rgba: &mut [u8], dirty: DirtyRect, px: i32, py: i32, color: u32, cov: f32) {
    if px < dirty.x || py < dirty.y || px >= dirty.x + dirty.w || py >= dirty.y + dirty.h {
        return;
    }
    if px < 0 || py < 0 || px >= APPLET_W || py >= APPLET_H {
        return;
    }
    let [sr, sg, sb, sa] = unpack_rgba(color);
    let src_a = (sa as f32 / 255.0) * cov.clamp(0.0, 1.0);
    if src_a <= 0.0 {
        return;
    }
    let col = (px - dirty.x) as usize;
    let row = (py - dirty.y) as usize;
    let stride = dirty.w as usize;
    let i = (row * stride + col) * 4;
    if i + 3 >= rgba.len() {
        return;
    }
    let dr = rgba[i] as f32 / 255.0;
    let dg = rgba[i + 1] as f32 / 255.0;
    let db = rgba[i + 2] as f32 / 255.0;
    let da = rgba[i + 3] as f32 / 255.0;
    let out_a = src_a + da * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    let out_r = (sr as f32 / 255.0 * src_a + dr * da * (1.0 - src_a)) / out_a;
    let out_g = (sg as f32 / 255.0 * src_a + dg * da * (1.0 - src_a)) / out_a;
    let out_b = (sb as f32 / 255.0 * src_a + db * da * (1.0 - src_a)) / out_a;
    rgba[i] = (out_r * 255.0).round().clamp(0.0, 255.0) as u8;
    rgba[i + 1] = (out_g * 255.0).round().clamp(0.0, 255.0) as u8;
    rgba[i + 2] = (out_b * 255.0).round().clamp(0.0, 255.0) as u8;
    rgba[i + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Map an applet-space rect through the Game Image blit (`min`,`size`).
pub fn map_applet_rect(min: [f32; 2], size: [f32; 2], x: i32, y: i32, w: i32, h: i32) -> [f32; 4] {
    let sx = size[0] / APPLET_W as f32;
    let sy = size[1] / APPLET_H as f32;
    [
        min[0] + x as f32 * sx,
        min[1] + y as f32 * sy,
        w as f32 * sx,
        h as f32 * sy,
    ]
}

#[cfg(test)]
mod tests {
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
        assert!(parse_color("red").is_none());
        assert!(parse_color("not-a-color").is_none());
    }

    #[test]
    fn unparseable_fill_style_keeps_previous() {
        reset();
        set_style("fillStyle", "#ffb15b");
        set_style("fillStyle", "red");
        set_style("fillStyle", "???");
        assert_eq!(fill_style(), "#ffb15b");
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
        assert_eq!(font(), "12px monospace");
    }

    #[test]
    fn measure_text_is_not_width_seven() {
        reset();
        set_style("font", "12px monospace");
        let w = measure_text("BoneBurier (external)  buried 0");
        assert!(w > 7.0, "real advance width, got {w}");
        let same = measure_with(12, true, "BoneBurier (external)  buried 0");
        assert!((w - same).abs() < 0.01);
        let short = measure_text("x");
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
        let width = measure_text(text);
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
        // Background rect pixels have alpha.
        let mut opaque = 0usize;
        for px in raster.rgba.chunks_exact(4) {
            if px[3] > 0 {
                opaque += 1;
            }
        }
        assert!(opaque > 50, "banner must actually paint, opaque={opaque}");
        // Glyphs of the same string sit inside the measured background.
        let bg_w = (width + 12.0).round() as i32;
        assert!(
            raster.x + raster.w as i32 >= 6 + bg_w,
            "dirty width must cover the measured rect"
        );
    }

    #[test]
    fn fill_rect_clips_to_applet() {
        let ops = vec![CanvasOp::FillRect {
            x: -10,
            y: -10,
            w: 40,
            h: 40,
            color: pack_rgba(255, 0, 0, 255),
        }];
        let raster = rasterize(&ops).unwrap();
        assert_eq!(raster.x, 0);
        assert_eq!(raster.y, 0);
        let ops = vec![CanvasOp::FillRect {
            x: APPLET_W - 5,
            y: APPLET_H - 5,
            w: 20,
            h: 20,
            color: pack_rgba(0, 255, 0, 255),
        }];
        let raster = rasterize(&ops).unwrap();
        assert!(raster.x + raster.w as i32 <= APPLET_W);
        assert!(raster.y + raster.h as i32 <= APPLET_H);
        let ops = vec![CanvasOp::FillRect {
            x: APPLET_W + 4,
            y: 0,
            w: 10,
            h: 10,
            color: pack_rgba(0, 0, 255, 255),
        }];
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
}

//! Isolate-thread Canvas recorder: parse, measure, rasterize, caps.
//!
//! No UI, no host observe-thread probes. The same font implementation is
//! used for `measureText` and `fillText` raster. JS only forwards
//! getters/setters/methods.

mod compose;
mod geom;
mod raster;
mod style;

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;

use std::cell::RefCell;

pub use compose::compose_paint;
pub use geom::{
    ClipPath, ClipSet, DrawExtras, FillPaint, GradStop, LineJoinKind, PathSeg, Shadow, TextAlign,
    TextBaseline, MAX_CLIP_PATHS, MAX_GRADIENTS, MAX_GRADIENT_STOPS, MAX_LINE_WIDTH,
    MAX_PATH_SEGS_PER_FRAME, MAX_PATH_SEGS_PER_OP, MAX_SAVE_DEPTH, MAX_SHADOW_BLUR,
};
pub use raster::{dirty_bounds, rasterize};
pub(crate) use style::font_for;
pub use style::{font_px_allowed, measure_with, pack_rgba, parse_color, parse_font, unpack_rgba};

use geom::{append_arc, finite_f32};

/// Client applet width in pixels (bound to `api::native_input::APPLET_W`).
pub const APPLET_W: i32 = api::native_input::APPLET_W;
/// Client applet height in pixels (bound to `api::native_input::APPLET_H`).
pub const APPLET_H: i32 = api::native_input::APPLET_H;

/// Max recorded canvas ops per onPaint call / paint frame.
pub const MAX_CANVAS_OPS: usize = 256;
/// Max UTF-8 bytes per fillText / measureText string.
pub const MAX_PAINT_TEXT: usize = 512;
/// Font size cap (CSS parser and decode/raster share this).
pub const MAX_FONT_PX: u16 = 256;

/// One recorded draw op. Color is packed `0xRRGGBBAA`.
///
/// Additive `extras` / align / baseline default to the old fillRect/fillText
/// behaviour so previous buffers stay valid.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op")]
pub enum CanvasOp {
    #[serde(rename = "fillRect")]
    FillRect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
        #[serde(default)]
        extras: DrawExtras,
    },
    #[serde(rename = "fillText")]
    FillText {
        text: String,
        x: i32,
        y: i32,
        color: u32,
        font_px: u16,
        mono: bool,
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        baseline: TextBaseline,
        #[serde(default)]
        extras: DrawExtras,
    },
    #[serde(rename = "fillPath")]
    FillPath {
        segs: Vec<PathSeg>,
        color: u32,
        extras: DrawExtras,
    },
    #[serde(rename = "strokePath")]
    StrokePath {
        segs: Vec<PathSeg>,
        color: u32,
        line_width: f32,
        line_join: LineJoinKind,
        extras: DrawExtras,
    },
}

impl Eq for CanvasOp {}

impl CanvasOp {
    pub fn fill_rect(x: i32, y: i32, w: i32, h: i32, color: u32) -> Self {
        Self::FillRect {
            x,
            y,
            w,
            h,
            color,
            extras: DrawExtras::default(),
        }
    }

    pub fn fill_text(
        text: impl Into<String>,
        x: i32,
        y: i32,
        color: u32,
        font_px: u16,
        mono: bool,
    ) -> Self {
        Self::FillText {
            text: text.into(),
            x,
            y,
            color,
            font_px,
            mono,
            align: TextAlign::Left,
            baseline: TextBaseline::Alphabetic,
            extras: DrawExtras::default(),
        }
    }
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
enum FillKind {
    Solid(u32),
    Gradient(u32),
}

#[derive(Clone)]
struct Style {
    fill: FillKind,
    font_px: u16,
    mono: bool,
    font_css: String,
    fill_css: String,
    stroke: u32,
    stroke_css: String,
    line_width: f32,
    line_join: LineJoinKind,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    shadow_color: u32,
    shadow_css: String,
    shadow_blur: f32,
    shadow_offset_x: f32,
    shadow_offset_y: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: FillKind::Solid(pack_rgba(0, 0, 0, 255)),
            font_px: 10,
            mono: false,
            font_css: "10px sans-serif".into(),
            fill_css: "#000000".into(),
            stroke: pack_rgba(0, 0, 0, 255),
            stroke_css: "#000000".into(),
            line_width: 1.0,
            line_join: LineJoinKind::Miter,
            text_align: TextAlign::Left,
            text_baseline: TextBaseline::Alphabetic,
            shadow_color: 0,
            shadow_css: "transparent".into(),
            shadow_blur: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
        }
    }
}

#[derive(Clone)]
struct LiveGradient {
    paint: FillPaint,
}

#[derive(Clone)]
struct Saved {
    style: Style,
    clip: ClipSet,
}

/// Why this onPaint call failed closed (typed, not inferred from user text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundFail {
    Ops,
    Text,
    Save,
    PathSegs,
    Gradients,
    GradStops,
    Clips,
    LineWidth,
    ShadowBlur,
    Align,
}

/// Wrapper-reported onPaint result. Never inferred from title/accent/lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnPaintOutcome {
    Success,
    Missing,
    Error(String),
}

struct Recorder {
    style: Style,
    stack: Vec<Saved>,
    path: Vec<PathSeg>,
    clip: ClipSet,
    gradients: Vec<LiveGradient>,
    ops: Vec<CanvasOp>,
    overflow: bool,
    fail: Option<BoundFail>,
    path_segs_frame: usize,
}

impl Recorder {
    fn new() -> Self {
        Self {
            style: Style::default(),
            stack: Vec::new(),
            path: Vec::new(),
            clip: ClipSet::default(),
            gradients: Vec::new(),
            ops: Vec::new(),
            overflow: false,
            fail: None,
            path_segs_frame: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn fail(&mut self, kind: BoundFail) {
        self.overflow = true;
        if self.fail.is_none() {
            self.fail = Some(kind);
        }
    }

    fn push(&mut self, op: CanvasOp) {
        if self.overflow {
            return;
        }
        if self.ops.len() >= MAX_CANVAS_OPS {
            self.fail(BoundFail::Ops);
            return;
        }
        self.ops.push(op);
    }

    fn extras_clip_shadow(&self) -> DrawExtras {
        DrawExtras {
            clips: self.clip.clone(),
            shadow: Shadow {
                color: self.style.shadow_color,
                blur: self.style.shadow_blur,
                offset_x: self.style.shadow_offset_x,
                offset_y: self.style.shadow_offset_y,
            },
            fill: FillPaint::Solid,
        }
    }

    fn extras_for_fill(&self) -> DrawExtras {
        let mut extras = self.extras_clip_shadow();
        extras.fill = match self.style.fill {
            FillKind::Solid(_) => FillPaint::Solid,
            FillKind::Gradient(id) => self
                .gradients
                .get(id as usize)
                .map(|g| g.paint.clone())
                .unwrap_or(FillPaint::Solid),
        };
        extras
    }

    fn solid_fill(&self) -> u32 {
        match self.style.fill {
            FillKind::Solid(c) => c,
            FillKind::Gradient(_) => 0,
        }
    }

    fn clip_emitted_segs(&self) -> usize {
        self.clip.iter().map(|c| c.segs.len()).sum()
    }

    /// Charge path copy plus every clip snapshot that will be cloned onto the
    /// draw, matching decoder `MAX_PATH_SEGS_PER_FRAME` accounting.
    fn charge_emitted(&mut self, path_len: usize) -> bool {
        if self.overflow {
            return false;
        }
        let n = path_len.saturating_add(self.clip_emitted_segs());
        if self.path_segs_frame.saturating_add(n) > MAX_PATH_SEGS_PER_FRAME {
            self.fail(BoundFail::PathSegs);
            return false;
        }
        self.path_segs_frame += n;
        true
    }

    fn prepare_draw(&mut self, path_len: usize) -> bool {
        if self.overflow {
            return false;
        }
        if self.ops.len() >= MAX_CANVAS_OPS {
            self.fail(BoundFail::Ops);
            return false;
        }
        self.charge_emitted(path_len)
    }

    fn push_working_seg(&mut self, seg: PathSeg) -> bool {
        if self.overflow {
            return false;
        }
        if self.path.len() >= MAX_PATH_SEGS_PER_OP {
            self.fail(BoundFail::PathSegs);
            return false;
        }
        self.path.push(seg);
        true
    }

    fn push_path_copy(&mut self) -> Option<Vec<PathSeg>> {
        if self.path.is_empty() {
            return None;
        }
        if self.path.len() > MAX_PATH_SEGS_PER_OP {
            self.fail(BoundFail::PathSegs);
            return None;
        }
        if !self.prepare_draw(self.path.len()) {
            return None;
        }
        Some(self.path.clone())
    }
}

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::new());
    static OUTCOME: RefCell<Option<OnPaintOutcome>> = const { RefCell::new(None) };
}

/// Start a new per-call recorder (HTML 2D defaults).
pub fn begin() {
    RECORDER.with(|r| r.borrow_mut().reset());
    OUTCOME.with(|o| *o.borrow_mut() = None);
}

/// Record the wrapper's typed onPaint result (`0` ok, `1` missing, `2` error).
pub fn onpaint_done(kind: i64, message: Option<&str>) {
    let outcome = match kind {
        1 => OnPaintOutcome::Missing,
        2 => OnPaintOutcome::Error(message.unwrap_or("onPaint").to_string()),
        _ => OnPaintOutcome::Success,
    };
    OUTCOME.with(|o| *o.borrow_mut() = Some(outcome));
}

/// CSS `font` getter: the last accepted string.
pub fn font() -> String {
    RECORDER.with(|r| r.borrow().style.font_css.clone())
}

/// CSS `fillStyle` getter: the last accepted string. Empty once the fill is a
/// gradient handle, which `fill_gradient_id` reports instead.
pub fn fill_style() -> String {
    RECORDER.with(|r| r.borrow().style.fill_css.clone())
}

/// The declared style getter (`get_style` on the paint ctx). Reads the
/// recorder, so a rejected assignment reads back as the previous value.
pub fn get_style(prop: &str) -> String {
    RECORDER.with(|r| {
        let rec = r.borrow();
        match prop {
            "font" => rec.style.font_css.clone(),
            "fillStyle" => rec.style.fill_css.clone(),
            "strokeStyle" => rec.style.stroke_css.clone(),
            "shadowColor" => rec.style.shadow_css.clone(),
            "lineJoin" => match rec.style.line_join {
                LineJoinKind::Miter => "miter".into(),
                LineJoinKind::Round => "round".into(),
                LineJoinKind::Bevel => "bevel".into(),
            },
            "textAlign" => match rec.style.text_align {
                TextAlign::Left => "left".into(),
                TextAlign::Center => "center".into(),
                TextAlign::Right => "right".into(),
            },
            "textBaseline" => match rec.style.text_baseline {
                TextBaseline::Alphabetic => "alphabetic".into(),
                TextBaseline::Top => "top".into(),
                TextBaseline::Middle => "middle".into(),
                TextBaseline::Bottom => "bottom".into(),
            },
            _ => String::new(),
        }
    })
}

/// The declared numeric style getter.
pub fn get_number(prop: &str) -> f64 {
    RECORDER.with(|r| {
        let rec = r.borrow();
        match prop {
            "lineWidth" => rec.style.line_width as f64,
            "shadowBlur" => rec.style.shadow_blur as f64,
            "shadowOffsetX" => rec.style.shadow_offset_x as f64,
            "shadowOffsetY" => rec.style.shadow_offset_y as f64,
            _ => 0.0,
        }
    })
}

/// The gradient handle `fillStyle` currently holds, or -1 for a solid paint.
pub fn fill_gradient_id() -> i32 {
    RECORDER.with(|r| match r.borrow().style.fill {
        FillKind::Gradient(id) => id as i32,
        FillKind::Solid(_) => -1,
    })
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
                    rec.style.fill = FillKind::Solid(color);
                    rec.style.fill_css = value.to_string();
                }
            }
            "strokeStyle" => {
                if let Some(color) = parse_color(value) {
                    rec.style.stroke = color;
                    rec.style.stroke_css = value.to_string();
                }
            }
            "shadowColor" => {
                if let Some(color) = parse_color(value) {
                    rec.style.shadow_color = color;
                    rec.style.shadow_css = value.to_string();
                }
            }
            "lineJoin" => match value.trim().to_ascii_lowercase().as_str() {
                "miter" => rec.style.line_join = LineJoinKind::Miter,
                "round" => rec.style.line_join = LineJoinKind::Round,
                "bevel" => rec.style.line_join = LineJoinKind::Bevel,
                _ => {}
            },
            "textAlign" => match value.trim().to_ascii_lowercase().as_str() {
                "left" | "start" => rec.style.text_align = TextAlign::Left,
                "center" => rec.style.text_align = TextAlign::Center,
                "right" | "end" => rec.style.text_align = TextAlign::Right,
                _ => rec.fail(BoundFail::Align),
            },
            "textBaseline" => match value.trim().to_ascii_lowercase().as_str() {
                "alphabetic" => rec.style.text_baseline = TextBaseline::Alphabetic,
                "top" => rec.style.text_baseline = TextBaseline::Top,
                "middle" => rec.style.text_baseline = TextBaseline::Middle,
                "bottom" => rec.style.text_baseline = TextBaseline::Bottom,
                _ => rec.fail(BoundFail::Align),
            },
            _ => {}
        }
    });
}

pub fn set_number(prop: &str, value: f64) {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        match prop {
            "lineWidth" => {
                if value.is_finite() && value > 0.0 {
                    if value as f32 > MAX_LINE_WIDTH {
                        rec.fail(BoundFail::LineWidth);
                    } else {
                        rec.style.line_width = value as f32;
                    }
                }
            }
            "shadowBlur" => {
                if value.is_finite() && value >= 0.0 {
                    if value as f32 > MAX_SHADOW_BLUR {
                        rec.fail(BoundFail::ShadowBlur);
                    } else {
                        rec.style.shadow_blur = value as f32;
                    }
                }
            }
            "shadowOffsetX" if value.is_finite() => {
                rec.style.shadow_offset_x = value as f32;
            }
            "shadowOffsetY" if value.is_finite() => {
                rec.style.shadow_offset_y = value as f32;
            }
            _ => {}
        }
    });
}

pub fn set_fill_gradient(id: u32) {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if (id as usize) < rec.gradients.len() {
            rec.style.fill = FillKind::Gradient(id);
            rec.style.fill_css.clear();
        }
    });
}

pub fn fill_rect(x: f64, y: f64, w: f64, h: f64) {
    let Some(x) = round_px(x) else {
        return;
    };
    let Some(y) = round_px(y) else {
        return;
    };
    let Some(w) = round_px(w) else {
        return;
    };
    let Some(h) = round_px(h) else {
        return;
    };
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if !rec.prepare_draw(0) {
            return;
        }
        let color = rec.solid_fill();
        let extras = rec.extras_for_fill();
        rec.push(CanvasOp::FillRect {
            x,
            y,
            w,
            h,
            color,
            extras,
        });
    });
}

pub fn fill_text(text: &str, x: f64, y: f64) {
    let Some(x) = round_px(x) else {
        return;
    };
    let Some(y) = round_px(y) else {
        return;
    };
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if text.len() > MAX_PAINT_TEXT {
            rec.fail(BoundFail::Text);
            return;
        }
        if !rec.prepare_draw(0) {
            return;
        }
        let color = rec.solid_fill();
        let font_px = rec.style.font_px.clamp(1, MAX_FONT_PX);
        let mono = rec.style.mono;
        let align = rec.style.text_align;
        let baseline = rec.style.text_baseline;
        let extras = rec.extras_for_fill();
        rec.push(CanvasOp::FillText {
            text: text.to_string(),
            x,
            y,
            color,
            font_px,
            mono,
            align,
            baseline,
            extras,
        });
    });
}

pub fn save() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.overflow {
            return;
        }
        if rec.stack.len() >= MAX_SAVE_DEPTH {
            rec.fail(BoundFail::Save);
            return;
        }
        let saved = Saved {
            style: rec.style.clone(),
            clip: rec.clip.clone(),
        };
        rec.stack.push(saved);
    });
}

pub fn restore() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.overflow {
            return;
        }
        if let Some(saved) = rec.stack.pop() {
            rec.style = saved.style;
            rec.clip = saved.clip;
        }
    });
}

pub fn begin_path() {
    RECORDER.with(|r| r.borrow_mut().path.clear());
}

pub fn close_path() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.path.is_empty() {
            return;
        }
        rec.push_working_seg(PathSeg::Close);
    });
}

fn push_seg(seg: PathSeg) {
    RECORDER.with(|r| {
        r.borrow_mut().push_working_seg(seg);
    });
}

pub fn move_to(x: f64, y: f64) {
    let Some(x) = finite_f32(x) else {
        return;
    };
    let Some(y) = finite_f32(y) else {
        return;
    };
    push_seg(PathSeg::MoveTo { x, y });
}

pub fn line_to(x: f64, y: f64) {
    let Some(x) = finite_f32(x) else {
        return;
    };
    let Some(y) = finite_f32(y) else {
        return;
    };
    push_seg(PathSeg::LineTo { x, y });
}

pub fn quadratic_curve_to(cx: f64, cy: f64, x: f64, y: f64) {
    let Some(cx) = finite_f32(cx) else {
        return;
    };
    let Some(cy) = finite_f32(cy) else {
        return;
    };
    let Some(x) = finite_f32(x) else {
        return;
    };
    let Some(y) = finite_f32(y) else {
        return;
    };
    push_seg(PathSeg::QuadTo { cx, cy, x, y });
}

pub fn arc(x: f64, y: f64, r: f64, start: f64, end: f64, ccw: bool) -> Result<(), String> {
    let Some(x) = finite_f32(x) else {
        return Ok(());
    };
    let Some(y) = finite_f32(y) else {
        return Ok(());
    };
    let Some(r) = finite_f32(r) else {
        return Ok(());
    };
    let Some(start) = finite_f32(start) else {
        return Ok(());
    };
    let Some(end) = finite_f32(end) else {
        return Ok(());
    };
    if r < 0.0 {
        return Err("IndexSizeError: radius must be non-negative".into());
    }
    RECORDER.with(|rec| {
        let mut rec = rec.borrow_mut();
        if rec.overflow {
            return;
        }
        if !append_arc(
            &mut rec.path,
            x,
            y,
            r,
            start,
            end,
            ccw,
            MAX_PATH_SEGS_PER_OP,
        ) {
            rec.fail(BoundFail::PathSegs);
        }
    });
    Ok(())
}

pub fn fill() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        let Some(segs) = rec.push_path_copy() else {
            return;
        };
        let color = rec.solid_fill();
        let extras = rec.extras_for_fill();
        rec.push(CanvasOp::FillPath {
            segs,
            color,
            extras,
        });
    });
}

pub fn stroke() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        let Some(segs) = rec.push_path_copy() else {
            return;
        };
        let color = rec.style.stroke;
        let line_width = rec.style.line_width;
        let line_join = rec.style.line_join;
        let extras = rec.extras_clip_shadow();
        rec.push(CanvasOp::StrokePath {
            segs,
            color,
            line_width,
            line_join,
            extras,
        });
    });
}

pub fn clip() {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.overflow {
            return;
        }
        if rec.clip.len() >= MAX_CLIP_PATHS {
            rec.fail(BoundFail::Clips);
            return;
        }
        if rec.path.is_empty() {
            let clip = rec.clip.with_pushed(ClipPath { segs: Vec::new() });
            rec.clip = clip;
            return;
        }
        if rec.path.len() > MAX_PATH_SEGS_PER_OP {
            rec.fail(BoundFail::PathSegs);
            return;
        }
        let segs = rec.path.clone();
        let clip = rec.clip.with_pushed(ClipPath { segs });
        rec.clip = clip;
    });
}

pub fn create_linear(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<u32, String> {
    let x0 = finite_f32(x0).ok_or_else(|| "canvas: createLinearGradient not finite".to_string())?;
    let y0 = finite_f32(y0).ok_or_else(|| "canvas: createLinearGradient not finite".to_string())?;
    let x1 = finite_f32(x1).ok_or_else(|| "canvas: createLinearGradient not finite".to_string())?;
    let y1 = finite_f32(y1).ok_or_else(|| "canvas: createLinearGradient not finite".to_string())?;
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.overflow {
            return Err("canvas: exceeded gradients".into());
        }
        if rec.gradients.len() >= MAX_GRADIENTS {
            rec.fail(BoundFail::Gradients);
            return Err("canvas: exceeded gradients".into());
        }
        let id = rec.gradients.len() as u32;
        rec.gradients.push(LiveGradient {
            paint: FillPaint::Linear {
                x0,
                y0,
                x1,
                y1,
                stops: Vec::new(),
            },
        });
        Ok(id)
    })
}

pub fn create_radial(x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64) -> Result<u32, String> {
    let x0 = finite_f32(x0).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    let y0 = finite_f32(y0).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    let r0 = finite_f32(r0).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    let x1 = finite_f32(x1).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    let y1 = finite_f32(y1).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    let r1 = finite_f32(r1).ok_or_else(|| "canvas: createRadialGradient not finite".to_string())?;
    if r0 < 0.0 || r1 < 0.0 {
        return Err("IndexSizeError: radii must be non-negative".into());
    }
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        if rec.overflow {
            return Err("canvas: exceeded gradients".into());
        }
        if rec.gradients.len() >= MAX_GRADIENTS {
            rec.fail(BoundFail::Gradients);
            return Err("canvas: exceeded gradients".into());
        }
        let id = rec.gradients.len() as u32;
        rec.gradients.push(LiveGradient {
            paint: FillPaint::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops: Vec::new(),
            },
        });
        Ok(id)
    })
}

pub fn add_color_stop(id: u32, offset: f64, color: &str) -> Result<(), String> {
    if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
        return Err("IndexSizeError: offset must be in [0, 1]".into());
    }
    let color = parse_color(color).ok_or_else(|| "SyntaxError: invalid color".to_string())?;
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        let idx = id as usize;
        let len = match rec.gradients.get(idx).map(|g| &g.paint) {
            Some(FillPaint::Linear { stops, .. } | FillPaint::Radial { stops, .. }) => stops.len(),
            Some(FillPaint::Solid) => 0,
            None => return Err("canvas: unknown gradient".into()),
        };
        if len >= MAX_GRADIENT_STOPS {
            rec.fail(BoundFail::GradStops);
            return Err("canvas: exceeded gradient stops".into());
        }
        let g = rec
            .gradients
            .get_mut(idx)
            .ok_or_else(|| "canvas: unknown gradient".to_string())?;
        let stops = match &mut g.paint {
            FillPaint::Linear { stops, .. } | FillPaint::Radial { stops, .. } => stops,
            FillPaint::Solid => return Ok(()),
        };
        stops.push(GradStop {
            offset: offset as f32,
            color,
        });
        Ok(())
    })
}

/// Advance-sum width for the current font. Oversized input fails closed
/// instead of inventing width 0.
pub fn measure_text(text: &str) -> Result<f64, String> {
    if text.len() > MAX_PAINT_TEXT {
        RECORDER.with(|r| r.borrow_mut().fail(BoundFail::Text));
        return Err(format!(
            "canvas: measureText exceeds {MAX_PAINT_TEXT} byte cap"
        ));
    }
    RECORDER.with(|r| {
        let rec = r.borrow();
        Ok(measure_with(rec.style.font_px, rec.style.mono, text))
    })
}

pub struct Take {
    pub ops: Vec<CanvasOp>,
    pub overflow: bool,
    pub fail: Option<&'static str>,
}

impl Take {
    pub(super) fn fail_message(&self) -> Option<&'static str> {
        self.fail
    }
}

/// Drain this call's ops. Overflow means the frame must drop canvas.
pub fn take() -> Take {
    RECORDER.with(|r| {
        let mut rec = r.borrow_mut();
        let fail = match rec.fail {
            Some(BoundFail::Ops) => Some("canvas: exceeded 256 ops"),
            Some(BoundFail::Text) => Some("canvas: text exceeds 512 byte cap"),
            Some(BoundFail::Save) => Some("canvas: exceeded save depth"),
            Some(BoundFail::PathSegs) => Some("canvas: exceeded path segments"),
            Some(BoundFail::Gradients) => Some("canvas: exceeded gradients"),
            Some(BoundFail::GradStops) => Some("canvas: exceeded gradient stops"),
            Some(BoundFail::Clips) => Some("canvas: exceeded clip paths"),
            Some(BoundFail::LineWidth) => Some("canvas: exceeded line width"),
            Some(BoundFail::ShadowBlur) => Some("canvas: exceeded shadow blur"),
            Some(BoundFail::Align) => Some("canvas: invalid textAlign/textBaseline"),
            None => None,
        };
        Take {
            ops: std::mem::take(&mut rec.ops),
            overflow: rec.overflow,
            fail,
        }
    })
}

pub(super) fn take_outcome() -> OnPaintOutcome {
    OUTCOME.with(|o| o.borrow_mut().take().unwrap_or(OnPaintOutcome::Success))
}

fn round_px(v: f64) -> Option<i32> {
    if !v.is_finite() {
        return None;
    }
    let r = v.round();
    if r > i32::MAX as f64 {
        Some(i32::MAX)
    } else if r < i32::MIN as f64 {
        Some(i32::MIN)
    } else {
        Some(r as i32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub(crate) fn clip_to_applet(x: i64, y: i64, w: i64, h: i64) -> Option<DirtyRect> {
    let x1 = x.max(0);
    let y1 = y.max(0);
    let x2 = x.saturating_add(w).min(i64::from(APPLET_W));
    let y2 = y.saturating_add(h).min(i64::from(APPLET_H));
    if x2 <= x1 || y2 <= y1 {
        None
    } else {
        Some(DirtyRect {
            x: x1 as i32,
            y: y1 as i32,
            w: (x2 - x1) as i32,
            h: (y2 - y1) as i32,
        })
    }
}

pub(crate) fn normalize_rect(x: i32, y: i32, w: i32, h: i32) -> Option<(i64, i64, i64, i64)> {
    let mut x = i64::from(x);
    let mut y = i64::from(y);
    let mut w = i64::from(w);
    let mut h = i64::from(h);
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

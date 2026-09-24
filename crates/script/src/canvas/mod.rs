//! Isolate-thread Canvas recorder: parse, measure, rasterize, caps.
//!
//! No UI, no host observe-thread probes. The same font implementation is
//! used for `measureText` and `fillText` raster. JS only forwards
//! getters/setters/methods.

mod geom;
mod raster;

use std::cell::RefCell;
use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

pub use geom::{
    ClipPath, ClipSet, DrawExtras, FillPaint, GradStop, LineJoinKind, PathSeg, Shadow, TextAlign,
    TextBaseline, MAX_CLIP_PATHS, MAX_GRADIENTS, MAX_GRADIENT_STOPS, MAX_LINE_WIDTH,
    MAX_PATH_SEGS_PER_FRAME, MAX_PATH_SEGS_PER_OP, MAX_SAVE_DEPTH, MAX_SHADOW_BLUR,
};
pub use raster::{dirty_bounds, rasterize};

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

const SANS_BYTES: &[u8] = include_bytes!("../../fonts/LiberationSans-Regular.ttf");
const MONO_BYTES: &[u8] = include_bytes!("../../fonts/LiberationMono-Regular.ttf");

static SANS: OnceLock<FontRef<'static>> = OnceLock::new();
static MONO: OnceLock<FontRef<'static>> = OnceLock::new();

fn sans_font() -> &'static FontRef<'static> {
    SANS.get_or_init(|| FontRef::try_from_slice(SANS_BYTES).expect("Liberation Sans Regular"))
}

fn mono_font() -> &'static FontRef<'static> {
    MONO.get_or_init(|| FontRef::try_from_slice(MONO_BYTES).expect("Liberation Mono Regular"))
}

pub(crate) fn font_for(mono: bool) -> &'static FontRef<'static> {
    if mono {
        mono_font()
    } else {
        sans_font()
    }
}

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
    stroke: u32,
    line_width: f32,
    line_join: LineJoinKind,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    shadow_color: u32,
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
            stroke: pack_rgba(0, 0, 0, 255),
            line_width: 1.0,
            line_join: LineJoinKind::Miter,
            text_align: TextAlign::Left,
            text_baseline: TextBaseline::Alphabetic,
            shadow_color: 0,
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
                }
            }
            "fillStyle" => {
                if let Some(color) = parse_color(value) {
                    rec.style.fill = FillKind::Solid(color);
                }
            }
            "strokeStyle" => {
                if let Some(color) = parse_color(value) {
                    rec.style.stroke = color;
                }
            }
            "shadowColor" => {
                if let Some(color) = parse_color(value) {
                    rec.style.shadow_color = color;
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

/// Same metrics as [`measure_text`] for a concrete face/size (tests / panel).
pub fn measure_with(font_px: u16, mono: bool, text: &str) -> f64 {
    let font_px = font_px.clamp(1, MAX_FONT_PX);
    let font = font_for(mono);
    let scale = PxScale::from(font_px as f32);
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
    pub fail: Option<&'static str>,
}

impl Take {
    fn fail_message(&self) -> Option<&'static str> {
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

fn take_outcome() -> OnPaintOutcome {
    OUTCOME.with(|o| o.borrow_mut().take().unwrap_or(OnPaintOutcome::Success))
}

fn diagnostic_paint(accent: bool, line: &str) -> crate::shim::ScriptPaint {
    crate::shim::ScriptPaint {
        title: Some("onPaint".into()),
        accent: accent.then(|| "#ff5555".into()),
        lines: vec![line.to_string()],
        buttons: Vec::new(),
        generation: 0,
        canvas: Vec::new(),
        ..Default::default()
    }
}

fn canvas_only(ops: Vec<CanvasOp>) -> crate::shim::ScriptPaint {
    crate::shim::ScriptPaint {
        title: None,
        accent: None,
        lines: Vec::new(),
        buttons: Vec::new(),
        generation: 0,
        canvas: ops,
        ..Default::default()
    }
}

fn user_has_structured(paint: &crate::shim::ScriptPaint) -> bool {
    paint.title.is_some() || !paint.lines.is_empty() || !paint.buttons.is_empty()
}

/// Compose this call's native recorder with user `Paint.end` state.
/// Wrapper diagnostics are typed (`OnPaintOutcome`); user title/accent/lines
/// are never used as provenance.
pub fn compose_paint(user: Option<crate::shim::ScriptPaint>) -> crate::shim::ScriptPaint {
    let outcome = take_outcome();
    let taken = take();
    match outcome {
        OnPaintOutcome::Missing => diagnostic_paint(true, "no onPaint on bot"),
        OnPaintOutcome::Error(msg) => diagnostic_paint(true, &msg),
        OnPaintOutcome::Success => {
            if let Some(msg) = taken.fail_message() {
                return diagnostic_paint(true, msg);
            }
            if !taken.ops.is_empty() {
                return match user {
                    Some(mut paint) if user_has_structured(&paint) => {
                        paint.canvas = taken.ops;
                        paint
                    }
                    _ => canvas_only(taken.ops),
                };
            }
            match user {
                Some(mut paint) if user_has_structured(&paint) => {
                    paint.canvas = Vec::new();
                    paint
                }
                _ => diagnostic_paint(false, "onPaint ran but Paint.end was not called"),
            }
        }
    }
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
    if s.eq_ignore_ascii_case("transparent") {
        return Some(pack_rgba(0, 0, 0, 0));
    }
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
    if !size.is_finite() || size <= 0.0 || size > f64::from(MAX_FONT_PX) {
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

/// True when a decoded/recorded font size is inside the parser cap.
pub fn font_px_allowed(font_px: u16) -> bool {
    font_px > 0 && font_px <= MAX_FONT_PX
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
        for px in raster.rgba.chunks_exact(4) {
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
        let paint = crate::shim::ScriptPaint {
            title: None,
            accent: None,
            lines: Vec::new(),
            buttons: Vec::new(),
            generation: 0,
            canvas: taken.ops.clone(),
            ..Default::default()
        };
        let buf = crate::isolate_fb::IsolateBuf::new().encode_paint(&paint);
        let decoded =
            crate::isolate_fb::decode_paint(&buf).expect("decoder must accept recorder frame");
        assert_eq!(decoded.canvas.len(), ok);

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

        let mut extras = DrawExtras::default();
        extras.clips = vec![ClipPath {
            segs: (0..MAX_PATH_SEGS_PER_OP)
                .map(|i| {
                    if i == 0 {
                        PathSeg::MoveTo { x: 0.0, y: 0.0 }
                    } else {
                        PathSeg::LineTo {
                            x: i as f32,
                            y: 0.0,
                        }
                    }
                })
                .collect(),
        }]
        .into();
        let over: Vec<CanvasOp> = (0..(ok + 1))
            .map(|_| CanvasOp::FillRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                color: pack_rgba(255, 0, 0, 255),
                extras: extras.clone(),
            })
            .collect();
        let paint = crate::shim::ScriptPaint {
            title: None,
            accent: None,
            lines: Vec::new(),
            buttons: Vec::new(),
            generation: 0,
            canvas: over,
            ..Default::default()
        };
        let buf = crate::isolate_fb::IsolateBuf::new().encode_paint(&paint);
        let err = crate::isolate_fb::decode_paint(&buf).expect_err("decoder frame budget");
        assert!(err.contains("path segs"), "{err}");
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
        let dirty = dirty_bounds(&[op.clone()]);
        assert!(dirty.is_some());
        let _ = rasterize(&[op]);

        let mut extras = DrawExtras::default();
        extras.shadow = Shadow {
            color: pack_rgba(0, 0, 0, 200),
            blur: MAX_SHADOW_BLUR,
            offset_x: f32::MAX,
            offset_y: -f32::MAX,
        };
        let shadowed = CanvasOp::FillRect {
            x: 20,
            y: 20,
            w: 8,
            h: 8,
            color: pack_rgba(255, 255, 255, 255),
            extras,
        };
        let _ = dirty_bounds(&[shadowed.clone()]);
        let _ = rasterize(&[shadowed]);
    }
}

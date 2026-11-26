//! Shared canvas recorder and optional rasterizer.
//!
//! JavaScript records in its isolate thread. A synchronous surface can own
//! its recorder instead. Both use the same style, geometry, and bounds.

#[cfg(feature = "canvas-raster")]
mod compose;
mod geom;
#[cfg(feature = "canvas-raster")]
mod raster;
mod style;

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;

use std::cell::RefCell;
use std::sync::Arc;

#[cfg(feature = "canvas-raster")]
pub use compose::compose_paint;
pub use geom::{
    ClipPath, ClipSet, DrawExtras, FillPaint, GradStop, LineJoinKind, PathSeg, Shadow, TextAlign,
    TextBaseline, MAX_CLIP_PATHS, MAX_GRADIENTS, MAX_GRADIENT_STOPS, MAX_LINE_WIDTH,
    MAX_PATH_SEGS_PER_FRAME, MAX_PATH_SEGS_PER_OP, MAX_SAVE_DEPTH, MAX_SHADOW_BLUR,
};
#[cfg(feature = "canvas-raster")]
pub use raster::{dirty_bounds, rasterize};
#[cfg(feature = "canvas-raster")]
pub use style::font_for;
#[cfg(feature = "canvas-raster")]
pub use style::measure_with;
pub use style::{font_px_allowed, pack_rgba, parse_color, parse_font, unpack_rgba};

use geom::{append_arc, finite_f32};

/// Client applet width in pixels (bound to `api::native_input::APPLET_W`).
pub const APPLET_W: i32 = api::native_input::APPLET_W;
/// Client applet height in pixels (bound to `api::native_input::APPLET_H`).
pub const APPLET_H: i32 = api::native_input::APPLET_H;

/// A failure returned by a Canvas operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CanvasError {
    #[error("IndexSizeError: radius must be non-negative")]
    NegativeRadius,
    #[error("canvas: createLinearGradient not finite")]
    LinearGradientNotFinite,
    #[error("canvas: exceeded gradients")]
    ExceededGradients,
    #[error("canvas: createRadialGradient not finite")]
    RadialGradientNotFinite,
    #[error("IndexSizeError: radii must be non-negative")]
    NegativeRadii,
    #[error("IndexSizeError: offset must be in [0, 1]")]
    OffsetOutOfRange,
    #[error("SyntaxError: invalid color")]
    InvalidColor,
    #[error("canvas: unknown gradient")]
    UnknownGradient,
    #[error("canvas: exceeded gradient stops")]
    ExceededGradientStops,
    #[error("canvas: measureText exceeds {MAX_PAINT_TEXT} byte cap")]
    MeasureTextTooLong,
    #[error("canvas: measureText requires canvas raster support")]
    MeasureTextUnavailable,
}

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
    font_css: Option<Arc<str>>,
    fill_css: Option<Arc<str>>,
    stroke: u32,
    stroke_css: Option<Arc<str>>,
    line_width: f32,
    line_join: LineJoinKind,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    shadow_color: u32,
    shadow_css: Option<Arc<str>>,
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
            font_css: None,
            fill_css: None,
            stroke: pack_rgba(0, 0, 0, 255),
            stroke_css: None,
            line_width: 1.0,
            line_join: LineJoinKind::Miter,
            text_align: TextAlign::Left,
            text_baseline: TextBaseline::Alphabetic,
            shadow_color: 0,
            shadow_css: None,
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

/// Canvas state for one run. Reset it before each `onPaint` invocation.
#[derive(Default)]
pub struct CanvasRecorder {
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

impl CanvasRecorder {
    /// Start a new call with the default styles and no path or gradients.
    pub fn reset(&mut self) {
        self.style = Style::default();
        self.stack.clear();
        self.path.clear();
        self.clip = ClipSet::default();
        self.gradients.clear();
        self.ops.clear();
        self.overflow = false;
        self.fail = None;
        self.path_segs_frame = 0;
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

    /// The declared style getter (`get_style` on the paint ctx). Reads the
    /// recorder, so a rejected assignment reads back as the previous value.
    pub fn get_style(&self, prop: &str) -> &str {
        match prop {
            "font" => self.style.font_css.as_deref().unwrap_or("10px sans-serif"),
            "fillStyle" => match self.style.fill {
                FillKind::Solid(_) => self.style.fill_css.as_deref().unwrap_or("#000000"),
                FillKind::Gradient(_) => "",
            },
            "strokeStyle" => self.style.stroke_css.as_deref().unwrap_or("#000000"),
            "shadowColor" => self.style.shadow_css.as_deref().unwrap_or("transparent"),
            "lineJoin" => match self.style.line_join {
                LineJoinKind::Miter => "miter",
                LineJoinKind::Round => "round",
                LineJoinKind::Bevel => "bevel",
            },
            "textAlign" => match self.style.text_align {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
            },
            "textBaseline" => match self.style.text_baseline {
                TextBaseline::Alphabetic => "alphabetic",
                TextBaseline::Top => "top",
                TextBaseline::Middle => "middle",
                TextBaseline::Bottom => "bottom",
            },
            _ => "",
        }
    }

    /// The declared numeric style getter.
    pub fn get_number(&self, prop: &str) -> f64 {
        match prop {
            "lineWidth" => self.style.line_width as f64,
            "shadowBlur" => self.style.shadow_blur as f64,
            "shadowOffsetX" => self.style.shadow_offset_x as f64,
            "shadowOffsetY" => self.style.shadow_offset_y as f64,
            _ => 0.0,
        }
    }

    /// The gradient handle `fillStyle` holds, or -1 for a solid paint.
    pub fn fill_gradient_id(&self) -> i32 {
        match self.style.fill {
            FillKind::Gradient(id) => id as i32,
            FillKind::Solid(_) => -1,
        }
    }

    /// Set `font` or `fillStyle`. Unparseable assignments keep the previous
    /// value (HTML). Unknown names are not invented.
    pub fn set_style(&mut self, prop: &str, value: &str) {
        match prop {
            "font" => {
                if let Some((px, mono)) = parse_font(value) {
                    self.style.font_px = px;
                    self.style.mono = mono;
                    self.style.font_css = Some(Arc::from(value));
                }
            }
            "fillStyle" => {
                if let Some(color) = parse_color(value) {
                    self.style.fill = FillKind::Solid(color);
                    self.style.fill_css = Some(Arc::from(value));
                }
            }
            "strokeStyle" => {
                if let Some(color) = parse_color(value) {
                    self.style.stroke = color;
                    self.style.stroke_css = Some(Arc::from(value));
                }
            }
            "shadowColor" => {
                if let Some(color) = parse_color(value) {
                    self.style.shadow_color = color;
                    self.style.shadow_css = Some(Arc::from(value));
                }
            }
            "lineJoin" => match value.trim().to_ascii_lowercase().as_str() {
                "miter" => self.style.line_join = LineJoinKind::Miter,
                "round" => self.style.line_join = LineJoinKind::Round,
                "bevel" => self.style.line_join = LineJoinKind::Bevel,
                _ => {}
            },
            "textAlign" => match value.trim().to_ascii_lowercase().as_str() {
                "left" | "start" => self.style.text_align = TextAlign::Left,
                "center" => self.style.text_align = TextAlign::Center,
                "right" | "end" => self.style.text_align = TextAlign::Right,
                _ => self.fail(BoundFail::Align),
            },
            "textBaseline" => match value.trim().to_ascii_lowercase().as_str() {
                "alphabetic" => self.style.text_baseline = TextBaseline::Alphabetic,
                "top" => self.style.text_baseline = TextBaseline::Top,
                "middle" => self.style.text_baseline = TextBaseline::Middle,
                "bottom" => self.style.text_baseline = TextBaseline::Bottom,
                _ => self.fail(BoundFail::Align),
            },
            _ => {}
        }
    }

    pub fn set_number(&mut self, prop: &str, value: f64) {
        match prop {
            "lineWidth" => {
                if value.is_finite() && value > 0.0 {
                    if value as f32 > MAX_LINE_WIDTH {
                        self.fail(BoundFail::LineWidth);
                    } else {
                        self.style.line_width = value as f32;
                    }
                }
            }
            "shadowBlur" => {
                if value.is_finite() && value >= 0.0 {
                    if value as f32 > MAX_SHADOW_BLUR {
                        self.fail(BoundFail::ShadowBlur);
                    } else {
                        self.style.shadow_blur = value as f32;
                    }
                }
            }
            "shadowOffsetX" if value.is_finite() => {
                self.style.shadow_offset_x = value as f32;
            }
            "shadowOffsetY" if value.is_finite() => {
                self.style.shadow_offset_y = value as f32;
            }
            _ => {}
        }
    }

    pub fn set_fill_gradient(&mut self, id: u32) {
        if (id as usize) < self.gradients.len() {
            self.style.fill = FillKind::Gradient(id);
            self.style.fill_css = None;
        }
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
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
        if !self.prepare_draw(0) {
            return;
        }
        let color = self.solid_fill();
        let extras = self.extras_for_fill();
        self.push(CanvasOp::FillRect {
            x,
            y,
            w,
            h,
            color,
            extras,
        });
    }

    pub fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        let Some(x) = round_px(x) else {
            return;
        };
        let Some(y) = round_px(y) else {
            return;
        };
        if text.len() > MAX_PAINT_TEXT {
            self.fail(BoundFail::Text);
            return;
        }
        if !self.prepare_draw(0) {
            return;
        }
        let color = self.solid_fill();
        let font_px = self.style.font_px.clamp(1, MAX_FONT_PX);
        let mono = self.style.mono;
        let align = self.style.text_align;
        let baseline = self.style.text_baseline;
        let extras = self.extras_for_fill();
        self.push(CanvasOp::FillText {
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
    }

    pub fn save(&mut self) {
        if self.overflow {
            return;
        }
        if self.stack.len() >= MAX_SAVE_DEPTH {
            self.fail(BoundFail::Save);
            return;
        }
        let saved = Saved {
            style: self.style.clone(),
            clip: self.clip.clone(),
        };
        self.stack.push(saved);
    }

    pub fn restore(&mut self) {
        if self.overflow {
            return;
        }
        if let Some(saved) = self.stack.pop() {
            self.style = saved.style;
            self.clip = saved.clip;
        }
    }

    pub fn begin_path(&mut self) {
        self.path.clear();
    }

    pub fn close_path(&mut self) {
        if self.path.is_empty() {
            return;
        }
        self.push_working_seg(PathSeg::Close);
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        let Some(x) = finite_f32(x) else {
            return;
        };
        let Some(y) = finite_f32(y) else {
            return;
        };
        self.push_working_seg(PathSeg::MoveTo { x, y });
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        let Some(x) = finite_f32(x) else {
            return;
        };
        let Some(y) = finite_f32(y) else {
            return;
        };
        self.push_working_seg(PathSeg::LineTo { x, y });
    }

    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
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
        self.push_working_seg(PathSeg::QuadTo { cx, cy, x, y });
    }

    pub fn arc(
        &mut self,
        x: f64,
        y: f64,
        r: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> Result<(), CanvasError> {
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
            return Err(CanvasError::NegativeRadius);
        }
        if !self.overflow
            && !append_arc(
                &mut self.path,
                x,
                y,
                r,
                start,
                end,
                ccw,
                MAX_PATH_SEGS_PER_OP,
            )
        {
            self.fail(BoundFail::PathSegs);
        }
        Ok(())
    }

    pub fn fill(&mut self) {
        let Some(segs) = self.push_path_copy() else {
            return;
        };
        let color = self.solid_fill();
        let extras = self.extras_for_fill();
        self.push(CanvasOp::FillPath {
            segs,
            color,
            extras,
        });
    }

    pub fn stroke(&mut self) {
        let Some(segs) = self.push_path_copy() else {
            return;
        };
        let color = self.style.stroke;
        let line_width = self.style.line_width;
        let line_join = self.style.line_join;
        let extras = self.extras_clip_shadow();
        self.push(CanvasOp::StrokePath {
            segs,
            color,
            line_width,
            line_join,
            extras,
        });
    }

    pub fn clip(&mut self) {
        if self.overflow {
            return;
        }
        if self.clip.len() >= MAX_CLIP_PATHS {
            self.fail(BoundFail::Clips);
            return;
        }
        if self.path.is_empty() {
            let clip = self.clip.with_pushed(ClipPath { segs: Vec::new() });
            self.clip = clip;
            return;
        }
        if self.path.len() > MAX_PATH_SEGS_PER_OP {
            self.fail(BoundFail::PathSegs);
            return;
        }
        let segs = self.path.clone();
        let clip = self.clip.with_pushed(ClipPath { segs });
        self.clip = clip;
    }

    pub fn create_linear(
        &mut self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> Result<u32, CanvasError> {
        let x0 = finite_f32(x0).ok_or(CanvasError::LinearGradientNotFinite)?;
        let y0 = finite_f32(y0).ok_or(CanvasError::LinearGradientNotFinite)?;
        let x1 = finite_f32(x1).ok_or(CanvasError::LinearGradientNotFinite)?;
        let y1 = finite_f32(y1).ok_or(CanvasError::LinearGradientNotFinite)?;
        if self.overflow {
            return Err(CanvasError::ExceededGradients);
        }
        if self.gradients.len() >= MAX_GRADIENTS {
            self.fail(BoundFail::Gradients);
            return Err(CanvasError::ExceededGradients);
        }
        let id = self.gradients.len() as u32;
        self.gradients.push(LiveGradient {
            paint: FillPaint::Linear {
                x0,
                y0,
                x1,
                y1,
                stops: Vec::new(),
            },
        });
        Ok(id)
    }

    pub fn create_radial(
        &mut self,
        x0: f64,
        y0: f64,
        r0: f64,
        x1: f64,
        y1: f64,
        r1: f64,
    ) -> Result<u32, CanvasError> {
        let x0 = finite_f32(x0).ok_or(CanvasError::RadialGradientNotFinite)?;
        let y0 = finite_f32(y0).ok_or(CanvasError::RadialGradientNotFinite)?;
        let r0 = finite_f32(r0).ok_or(CanvasError::RadialGradientNotFinite)?;
        let x1 = finite_f32(x1).ok_or(CanvasError::RadialGradientNotFinite)?;
        let y1 = finite_f32(y1).ok_or(CanvasError::RadialGradientNotFinite)?;
        let r1 = finite_f32(r1).ok_or(CanvasError::RadialGradientNotFinite)?;
        if r0 < 0.0 || r1 < 0.0 {
            return Err(CanvasError::NegativeRadii);
        }
        if self.overflow {
            return Err(CanvasError::ExceededGradients);
        }
        if self.gradients.len() >= MAX_GRADIENTS {
            self.fail(BoundFail::Gradients);
            return Err(CanvasError::ExceededGradients);
        }
        let id = self.gradients.len() as u32;
        self.gradients.push(LiveGradient {
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
    }

    pub fn add_color_stop(&mut self, id: u32, offset: f64, color: &str) -> Result<(), CanvasError> {
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
            return Err(CanvasError::OffsetOutOfRange);
        }
        let color = parse_color(color).ok_or(CanvasError::InvalidColor)?;
        let idx = id as usize;
        let len = match self.gradients.get(idx).map(|g| &g.paint) {
            Some(FillPaint::Linear { stops, .. } | FillPaint::Radial { stops, .. }) => stops.len(),
            Some(FillPaint::Solid) => 0,
            None => return Err(CanvasError::UnknownGradient),
        };
        if len >= MAX_GRADIENT_STOPS {
            self.fail(BoundFail::GradStops);
            return Err(CanvasError::ExceededGradientStops);
        }
        let g = self
            .gradients
            .get_mut(idx)
            .ok_or(CanvasError::UnknownGradient)?;
        let stops = match &mut g.paint {
            FillPaint::Linear { stops, .. } | FillPaint::Radial { stops, .. } => stops,
            FillPaint::Solid => return Ok(()),
        };
        stops.push(GradStop {
            offset: offset as f32,
            color,
        });
        Ok(())
    }

    /// Advance-sum width for the current font. Oversized input fails closed.
    pub fn measure_text(&mut self, text: &str) -> Result<f64, CanvasError> {
        if text.len() > MAX_PAINT_TEXT {
            self.fail(BoundFail::Text);
            return Err(CanvasError::MeasureTextTooLong);
        }
        #[cfg(feature = "canvas-raster")]
        return Ok(measure_with(self.style.font_px, self.style.mono, text));
        #[cfg(not(feature = "canvas-raster"))]
        Err(CanvasError::MeasureTextUnavailable)
    }

    /// Drain this call's ops. Overflow means the frame must drop canvas.
    pub fn take(&mut self) -> Take {
        let fail = match self.fail {
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
            ops: std::mem::take(&mut self.ops),
            overflow: self.overflow,
            fail,
        }
    }
}

// JavaScript retains its isolate-local recorder and wrapper outcome.
thread_local! {
    static RECORDER: RefCell<CanvasRecorder> = RefCell::new(CanvasRecorder::default());
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
    RECORDER.with(|r| r.borrow().get_style("font").to_owned())
}

/// CSS `fillStyle` getter: the last accepted string. Empty once the fill is a
/// gradient handle, which `fill_gradient_id` reports instead.
pub fn fill_style() -> String {
    RECORDER.with(|r| r.borrow().get_style("fillStyle").to_owned())
}

/// The declared style getter (`get_style` on the paint ctx). Reads the
/// recorder, so a rejected assignment reads back as the previous value.
pub fn get_style(prop: &str) -> String {
    RECORDER.with(|r| r.borrow().get_style(prop).to_owned())
}

/// The declared numeric style getter.
pub fn get_number(prop: &str) -> f64 {
    RECORDER.with(|r| r.borrow().get_number(prop))
}

/// The gradient handle `fillStyle` currently holds, or -1 for a solid paint.
pub fn fill_gradient_id() -> i32 {
    RECORDER.with(|r| r.borrow().fill_gradient_id())
}

macro_rules! local_recorder {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) $(-> $result:ty)?;)*) => {
        $(pub fn $name($($arg: $ty),*) $(-> $result)? {
            RECORDER.with(|r| r.borrow_mut().$name($($arg),*))
        })*
    };
}

local_recorder! {
    fn set_style(prop: &str, value: &str);
    fn set_number(prop: &str, value: f64);
    fn set_fill_gradient(id: u32);
    fn fill_rect(x: f64, y: f64, w: f64, h: f64);
    fn fill_text(text: &str, x: f64, y: f64);
    fn save();
    fn restore();
    fn begin_path();
    fn close_path();
    fn move_to(x: f64, y: f64);
    fn line_to(x: f64, y: f64);
    fn quadratic_curve_to(cx: f64, cy: f64, x: f64, y: f64);
    fn arc(x: f64, y: f64, r: f64, start: f64, end: f64, ccw: bool) -> Result<(), CanvasError>;
    fn fill();
    fn stroke();
    fn clip();
    fn create_linear(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<u32, CanvasError>;
    fn create_radial(x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64) -> Result<u32, CanvasError>;
    fn add_color_stop(id: u32, offset: f64, color: &str) -> Result<(), CanvasError>;
    fn take() -> Take;
}

#[cfg(feature = "canvas-raster")]
local_recorder! {
    fn measure_text(text: &str) -> Result<f64, CanvasError>;
}

pub struct Take {
    pub ops: Vec<CanvasOp>,
    pub overflow: bool,
    pub fail: Option<&'static str>,
}

#[cfg(feature = "canvas-raster")]
impl Take {
    pub(super) fn fail_message(&self) -> Option<&'static str> {
        self.fail
    }
}

#[cfg(feature = "canvas-raster")]
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

#[cfg(feature = "canvas-raster")]
pub fn clip_to_applet(x: i64, y: i64, w: i64, h: i64) -> Option<DirtyRect> {
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

#[cfg(feature = "canvas-raster")]
pub fn normalize_rect(x: i32, y: i32, w: i32, h: i32) -> Option<(i64, i64, i64, i64)> {
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

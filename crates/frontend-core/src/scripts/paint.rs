//! A running script's paint frame as the front ends draw it (T3.3):
//! borrowed views over the frame the slot already holds, so a draw clones
//! nothing, plus the canvas rasterizer and a reusable cache of the last
//! drawn canvas.

use script::canvas::{self, CanvasOp};
use script::shim::{PaintChromeBand, ScriptPaint, ScriptPaintButton};

/// A script's paint frame held across UI frames (shared with the slot's
/// status row; never deep-copied).
#[derive(Debug)]
pub struct PaintFrame(std::sync::Arc<ScriptPaint>);

impl PaintFrame {
    pub(crate) fn new(paint: std::sync::Arc<ScriptPaint>) -> Self {
        Self(paint)
    }

    /// A paint frame over `paint` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_script(paint: ScriptPaint) -> Self {
        Self(std::sync::Arc::new(paint))
    }

    /// The frame as a borrowed view.
    pub fn view(&self) -> PaintView<'_> {
        PaintView(&self.0)
    }

    /// Another handle on the same frame.
    pub fn share(&self) -> Self {
        Self(std::sync::Arc::clone(&self.0))
    }
}

/// A script's paint frame.
#[derive(Debug, Clone, Copy)]
pub struct PaintView<'a>(&'a ScriptPaint);

impl<'a> PaintView<'a> {
    pub(crate) fn new(paint: &'a ScriptPaint) -> Self {
        Self(paint)
    }

    /// The panel title.
    pub fn title(&self) -> Option<&'a str> {
        self.0.title.as_deref()
    }

    /// The accent colour text.
    pub fn accent(&self) -> Option<&'a str> {
        self.0.accent.as_deref()
    }

    /// The text lines.
    pub fn lines(&self) -> &'a [String] {
        &self.0.lines
    }

    /// The buttons, in order.
    pub fn buttons(&self) -> impl ExactSizeIterator<Item = PaintButtonView<'a>> + 'a {
        self.0.buttons.iter().map(PaintButtonView)
    }

    /// The canvas operations.
    pub fn canvas(&self) -> CanvasOps<'a> {
        CanvasOps(&self.0.canvas)
    }

    /// The frame generation (a click names the generation it answers).
    pub fn generation(&self) -> u64 {
        self.0.generation
    }

    /// The chrome strip band.
    pub fn strip(&self) -> Option<ChromeBandView<'a>> {
        self.0.strip.as_ref().map(ChromeBandView)
    }

    /// The chrome rail band.
    pub fn rail(&self) -> Option<ChromeBandView<'a>> {
        self.0.rail.as_ref().map(ChromeBandView)
    }

    /// The footer text.
    pub fn footer(&self) -> Option<&'a str> {
        self.0.footer.as_deref()
    }

    /// The tab bands, in order.
    pub fn tabs(&self) -> impl ExactSizeIterator<Item = ChromeBandView<'a>> + 'a {
        self.0.tabs.iter().map(ChromeBandView)
    }
}

/// One paint button.
#[derive(Debug, Clone, Copy)]
pub struct PaintButtonView<'a>(&'a ScriptPaintButton);

impl<'a> PaintButtonView<'a> {
    /// The id a click sends back.
    pub fn id(&self) -> &'a str {
        &self.0.id
    }

    /// The label.
    pub fn label(&self) -> &'a str {
        &self.0.label
    }
}

/// One chrome band (strip, rail or tab): a named choice list.
#[derive(Debug, Clone, Copy)]
pub struct ChromeBandView<'a>(&'a PaintChromeBand);

impl<'a> ChromeBandView<'a> {
    /// The store key a selection sends back.
    pub fn id(&self) -> &'a str {
        &self.0.id
    }

    /// The choices.
    pub fn names(&self) -> &'a [String] {
        &self.0.names
    }

    /// The selected choice.
    pub fn selected(&self) -> &'a str {
        &self.0.selected
    }

    /// The status text.
    pub fn status(&self) -> Option<&'a str> {
        self.0.status.as_deref()
    }

    /// The brand text.
    pub fn brand(&self) -> Option<&'a str> {
        self.0.brand.as_deref()
    }
}

/// A paint frame's canvas operations.
#[derive(Debug, Clone, Copy)]
pub struct CanvasOps<'a>(&'a [CanvasOp]);

impl<'a> CanvasOps<'a> {
    /// No operations.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The applet-space rectangle `[x, y, w, h]` the operations touch.
    pub fn dirty_bounds(&self) -> Option<[i32; 4]> {
        canvas::dirty_bounds(self.0).map(|d| [d.x, d.y, d.w, d.h])
    }

    /// Rasterize the operations (bounded; `None` past the operation limit
    /// or with nothing to draw).
    pub fn rasterize(&self) -> Option<CanvasRaster> {
        canvas::rasterize(self.0).map(|raster| CanvasRaster {
            x: raster.x,
            y: raster.y,
            width: raster.w,
            height: raster.h,
            rgba: raster.rgba,
        })
    }
}

/// A rasterized canvas: an RGBA8 image placed in applet space.
#[derive(Debug)]
pub struct CanvasRaster {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl CanvasRaster {
    /// The applet-space left edge.
    pub fn x(&self) -> i32 {
        self.x
    }

    /// The applet-space top edge.
    pub fn y(&self) -> i32 {
        self.y
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA8 pixels, row-major.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// The canvas operations a front end drew last, kept to skip an unchanged
/// re-raster. Its buffer is reused, so a steady canvas allocates nothing.
#[derive(Debug, Default)]
pub struct CanvasCache(Vec<CanvasOp>);

impl CanvasCache {
    /// Whether `ops` are the operations drawn last.
    pub fn same_as(&self, ops: CanvasOps<'_>) -> bool {
        self.0.as_slice() == ops.0
    }

    /// Remember `ops` as drawn.
    pub fn store(&mut self, ops: CanvasOps<'_>) {
        super::refill(&mut self.0, ops.0);
    }

    /// Forget the last canvas.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Map an applet-space rectangle onto a game image drawn at `min` with
/// `size`: the screen rectangle `[x, y, w, h]`.
pub fn map_applet_rect(min: [f32; 2], size: [f32; 2], x: i32, y: i32, w: i32, h: i32) -> [f32; 4] {
    canvas::map_applet_rect(min, size, x, y, w, h)
}

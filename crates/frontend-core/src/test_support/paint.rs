//! Paint fixtures for front-end tests (see the parent module).
//!
//! Panel overlay tests and TUI chat tests build script-paint frames here,
//! so they never name the crates below the operator seam. Everything here
//! panics on failure: a fixture is a test's given, not its fallible
//! production path.

use crate::scripts::PaintFrame;

/// A script-paint frame a test draws: title, accent, lines, buttons, chrome
/// bands, footer and generation, with an empty canvas unless a canvas
/// fixture below adds ops. Panel and TUI tests never name the script crate.
pub struct TestPaint {
    paint: script::shim::ScriptPaint,
}

impl TestPaint {
    /// A paint frame with `title` and `lines` (tests).
    pub fn new(title: Option<&str>, lines: &[&str]) -> Self {
        Self {
            paint: script::shim::ScriptPaint {
                title: title.map(str::to_owned),
                accent: None,
                lines: lines.iter().map(|line| (*line).to_owned()).collect(),
                buttons: Vec::new(),
                generation: 0,
                canvas: Vec::new(),
                ..Default::default()
            },
        }
    }

    /// The accent colour text (tests).
    pub fn with_accent(mut self, accent: &str) -> Self {
        self.paint.accent = Some(accent.to_string());
        self
    }

    /// Append one more line after construction (tests).
    pub fn push_line(&mut self, line: &str) {
        self.paint.lines.push(line.to_string());
    }

    /// Advertise one paint button (tests).
    pub fn with_button(mut self, id: &str, label: &str) -> Self {
        self.paint.buttons.push(script::shim::ScriptPaintButton {
            id: id.to_string(),
            label: label.to_string(),
        });
        self
    }

    /// Advertise the host's Pause/Resume and Stop controls under the fixed
    /// ids the standard paint-controls helper writes (tests).
    pub fn with_host_controls(self) -> Self {
        self.with_button(script::shim::HOST_PAUSE_CONTROL_ID, "Pause")
            .with_button(script::shim::HOST_STOP_CONTROL_ID, "Stop")
    }

    /// Stamp the host-owned frame generation (tests).
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.paint.generation = generation;
        self
    }

    /// Advertise the brand strip band (tests).
    pub fn with_strip(
        mut self,
        id: &str,
        names: &[&str],
        selected: &str,
        status: Option<&str>,
        brand: Option<&str>,
    ) -> Self {
        self.paint.strip = Some(script::shim::PaintChromeBand {
            id: id.to_string(),
            names: names.iter().map(|name| (*name).to_string()).collect(),
            selected: selected.to_string(),
            status: status.map(str::to_owned),
            brand: brand.map(str::to_owned),
        });
        self
    }

    /// Advertise the vertical rail band (tests).
    pub fn with_rail(mut self, id: &str, names: &[&str], selected: &str) -> Self {
        self.paint.rail = Some(script::shim::PaintChromeBand {
            id: id.to_string(),
            names: names.iter().map(|name| (*name).to_string()).collect(),
            selected: selected.to_string(),
            status: None,
            brand: None,
        });
        self
    }

    /// The right-aligned byline (tests).
    pub fn with_footer(mut self, footer: &str) -> Self {
        self.paint.footer = Some(footer.to_string());
        self
    }

    /// Draw the canvas banner the overlay tests map: one translucent
    /// applet-space banner rect (tests).
    pub fn with_canvas_banner(mut self) -> Self {
        self.paint.canvas = vec![script::canvas::CanvasOp::fill_rect(
            6,
            6,
            400,
            50,
            script::canvas::pack_rgba(0, 0, 0, 178),
        )];
        self
    }

    /// Draw one stroked path, for the texture-cache tests (tests).
    pub fn with_path_stroke(mut self) -> Self {
        self.paint.canvas = vec![script::canvas::CanvasOp::StrokePath {
            segs: vec![
                script::canvas::PathSeg::MoveTo { x: 40.0, y: 40.0 },
                script::canvas::PathSeg::LineTo { x: 80.0, y: 90.0 },
            ],
            color: script::canvas::pack_rgba(255, 224, 64, 140),
            line_width: 1.5,
            line_join: script::canvas::LineJoinKind::Round,
            extras: script::canvas::DrawExtras::default(),
        }];
        self
    }

    /// The host-owned frame generation.
    pub fn generation(&self) -> u64 {
        self.paint.generation
    }

    /// This paint as the front ends draw it (tests).
    pub fn frame(&self) -> PaintFrame {
        PaintFrame::from_script(self.paint.clone())
    }

    /// This paint as read with a run of `profile` in `state`, so its host
    /// controls show and activate as they would for that run (tests).
    pub fn frame_for_run(
        &self,
        profile: &str,
        state: crate::scripts::ScriptRunState,
    ) -> PaintFrame {
        PaintFrame::with_run(self.paint.clone(), profile, state)
    }

    /// Publish this paint on `name`'s status row in `play`, as the slot's
    /// observe does after a script tick; the session reads it at its next
    /// poll (tests). Adds the row when it is missing.
    pub fn publish(&self, play: &host_play::Play, name: &str) {
        let paint = Some(std::sync::Arc::new(self.paint.clone()));
        let mut rows = play.statuses.lock().expect("status rows");
        match rows.iter_mut().find(|row| row.username == name) {
            Some(row) => row.script_paint = paint,
            None => rows.push(host_play::SlotStatus {
                username: name.to_owned(),
                script_paint: paint,
                ..host_play::SlotStatus::default()
            }),
        }
    }
}

/// One caption banner rasterized off-screen: the measured text width and the
/// RGBA8 image the canvas ops cover. Tests assert the fill covers the glyphs
/// and that paint reached pixels.
pub struct CaptionRaster {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    text_width: f64,
}

impl CaptionRaster {
    /// The applet-space left edge.
    pub fn x(&self) -> i32 {
        self.x
    }

    /// The applet-space top edge.
    pub fn y(&self) -> i32 {
        self.y
    }

    /// The image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA8 pixels, row-major.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The measured text width the fill rect covers.
    pub fn text_width(&self) -> f64 {
        self.text_width
    }
}

/// Rasterize one native-font caption banner exactly as the overlay's canvas
/// path draws it: monospace text over a translucent fill (tests).
pub fn caption_raster(text: &str) -> CaptionRaster {
    script::canvas::begin();
    script::canvas::set_style("font", "12px monospace");
    let text_width = script::canvas::measure_text(text).expect("caption text width");
    script::canvas::set_style("fillStyle", "rgba(0, 0, 0, 0.6)");
    script::canvas::fill_rect(6.0, 6.0, text_width + 12.0, 24.0);
    script::canvas::set_style("fillStyle", "#ffb15b");
    script::canvas::fill_text(text, 12.0, 22.0);
    let taken = script::canvas::take();
    let raster = script::canvas::rasterize(&taken.ops).expect("caption raster");
    CaptionRaster {
        x: raster.x,
        y: raster.y,
        width: raster.w,
        height: raster.h,
        rgba: raster.rgba,
        text_width,
    }
}
